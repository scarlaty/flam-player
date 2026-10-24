//! Tests de la garde commune des écritures (#317) : conditions communes, disposition non
//! standard signalée sans refus, 2.x autorisée sans confirmation (#326), cohérence des clés 2.x
//! informative, état exposé. Fixtures partagées avec les tests du retrait, du réordonnancement et
//! du transfert.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use super::*;
use crate::device::crypto::tests::{TEST_IV, TEST_KEY};
use crate::device::crypto::{encrypt_zero_padded, key_file_from_serial, SERIAL_FIELD_LEN};

/// Numéro de série inventé des fixtures (14 caractères hexadécimaux).
pub(crate) const TEST_SERIAL_HEX: &[u8; 14] = b"00123456789abc";

/// `.mdf` synthétique : `header`, texte de version `text` (48 octets complétés de nuls),
/// numéro de série inventé, VID/PID attendus, clé et IV de test à 0x4E et 0x5E.
pub(crate) fn mdf_with(header: [u8; 2], text: &str) -> Vec<u8> {
    let mut m = header.to_vec();
    let mut field = text.as_bytes().to_vec();
    field.resize(48, 0);
    m.extend(field);
    let mut serial = TEST_SERIAL_HEX.to_vec();
    serial.resize(SERIAL_FIELD_LEN, 0);
    m.extend(serial);
    m.extend(EXPECTED_USB.0.to_le_bytes());
    m.extend(EXPECTED_USB.1.to_le_bytes());
    m.extend(TEST_KEY);
    m.extend(TEST_IV);
    m.extend([0u8; 16]);
    assert_eq!(m.len(), MDF_LEN);
    m
}

/// Disposition de la Flam 1.15.14 réelle (mesurée) : en-tête `01 00`, version nue.
pub(crate) fn mdf_v1_real() -> Vec<u8> {
    mdf_with(KNOWN_HEADER, "1.15.14-2d356995")
}

/// Flam 2.x de disposition standard, version préfixée « main: ».
pub(crate) fn mdf_v2_known() -> Vec<u8> {
    mdf_with(KNOWN_HEADER, "main: 2.4.2-test\ncomm: x")
}

/// Contexte de test : mémoire de session propre, aucun contrôle de racine, aucune histoire du
/// launcher.
pub(crate) fn test_ctx() -> WriteContext<'static> {
    ctx_with(HashSet::new())
}

fn ctx_with(launcher: HashSet<String>) -> WriteContext<'static> {
    let session: &'static KeySession = Box::leak(Box::default());
    WriteContext::new(session, None, launcher)
}

/// `info` chiffré avec les clés de test.
pub(crate) fn info_v2(title: &str) -> Vec<u8> {
    let plain = format!("{title}\nSous-titre\n000000\n{title}\n");
    encrypt_zero_padded(&TEST_KEY, &TEST_IV, plain.as_bytes())
}

/// Fichier `key` déduit du numéro de série de test.
pub(crate) fn key_v2() -> Vec<u8> {
    let mut serial = [0u8; 7];
    for (i, byte) in serial.iter_mut().enumerate() {
        let hex = std::str::from_utf8(&TEST_SERIAL_HEX[2 * i..2 * i + 2]).unwrap();
        *byte = u8::from_str_radix(hex, 16).unwrap();
    }
    key_file_from_serial(&serial).to_vec()
}

const STORY: &str = "11111111-2222-4333-8444-555555555555";
const OTHER: &str = "66666666-7777-4888-9999-aaaaaaaaaaaa";

fn write(path: &Path, data: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, data).unwrap();
}

/// Faux appareil de `.mdf` donné, sans histoire.
fn device(mdf: &[u8]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join(MDF_FILE), mdf);
    std::fs::create_dir_all(dir.path().join("str")).unwrap();
    dir
}

/// Ajoute une histoire au format Flam `uuid` (`main.lsf`, `key`, `info`).
pub(crate) fn story(root: &Path, uuid: &str, key: &[u8], info: &[u8]) {
    let dir = root.join("str").join(uuid);
    write(&dir.join("key"), key);
    write(&dir.join("info"), info);
    write(&dir.join("main.lsf"), &[0u8; 32]);
}

/// Arborescence complète avec contenu (rien ne doit changer pendant une garde).
pub(crate) fn snapshot(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn walk(dir: &Path, out: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                out.insert(path.clone(), None);
                walk(&path, out);
            } else {
                out.insert(path.clone(), Some(std::fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, &mut out);
    out
}

fn refused(root: &Path, ctx: &WriteContext<'_>, kind: WriteKind) -> WriteRefused {
    let before = snapshot(root);
    let err = check_write(root, ctx, kind).unwrap_err();
    assert_eq!(
        snapshot(root),
        before,
        "la garde n'écrit jamais sur la Flam"
    );
    err
}

/// `info` chiffré avec d'autres clés que celles de test (illisible avec les clés de `.mdf`).
pub(in crate::device) fn wrong_info() -> Vec<u8> {
    encrypt_zero_padded(&[9u8; 16], &[8u8; 16], b"Autre\nclef\n000000\n")
}

#[test]
fn la_1_15_14_reelle_est_standard_et_autorisee() {
    let layout = analyze(&mdf_v1_real());
    assert_eq!(layout.family, Family::V1);
    assert_eq!(layout.form, VersionForm::Bare);
    assert_eq!(layout.header, Some([1, 0]));
    assert!(layout.standard);
    let dev = device(&mdf_v1_real());
    let permit = check_write(dev.path(), &test_ctx(), WriteKind::Send).unwrap();
    assert_eq!(permit.family(), FirmwareFamily::V1);
    assert!(permit.same_mdf(&mdf_v1_real()));
}

#[test]
fn une_disposition_non_standard_est_signalee_sans_refus() {
    let mut long = mdf_v1_real();
    long.extend([0u8; 74]);
    for mdf in [
        mdf_with([0, 0], "1.15.14-2d356995"),
        mdf_with([2, 0], "main: 1.15.14-x"),
        long,
        mdf_v1_real()[..MDF_MIN_LEN].to_vec(),
    ] {
        let layout = analyze(&mdf);
        assert!(!layout.standard);
        let dev = device(&mdf);
        check_write(dev.path(), &test_ctx(), WriteKind::Send).unwrap();
        let status = firmware_status(dev.path(), &mdf, &test_ctx(), false);
        assert!(!status.layout_standard);
        assert_eq!(status.write_block, None);
    }
}

#[test]
fn conditions_communes_refusees() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        check_write(dir.path(), &test_ctx(), WriteKind::Other).unwrap_err(),
        WriteRefused::DeviceNotFound
    );
    let mut zero_key = mdf_v2_known();
    zero_key[MDF_KEY_OFFSET..MDF_KEY_OFFSET + 16].fill(0);
    let mut ff_iv = mdf_v2_known();
    ff_iv[MDF_IV_OFFSET..MDF_IV_OFFSET + 16].fill(0xFF);
    let mut bad_serial = mdf_v2_known();
    bad_serial[0x32] = b'g';
    type Expect = fn(&WriteRefused) -> bool;
    let cases: [(Vec<u8>, Expect); 9] = [
        (mdf_v1_real()[..MDF_MIN_LEN - 1].to_vec(), |e| {
            matches!(e, WriteRefused::Truncated { .. })
        }),
        (mdf_with(KNOWN_HEADER, "pas de version"), |e| {
            *e == WriteRefused::VersionUnreadable
        }),
        (
            mdf_with(KNOWN_HEADER, "main: 3.0.0-x"),
            |e| matches!(e, WriteRefused::UnsupportedFirmware { version } if version == "3.0.0"),
        ),
        (
            mdf_with(KNOWN_HEADER, "main: 2..1-x"),
            |e| matches!(e, WriteRefused::UnsupportedFirmware { version } if version == "2..1"),
        ),
        (mdf_with(KNOWN_HEADER, "02.4-x"), |e| {
            matches!(e, WriteRefused::UnsupportedFirmware { .. })
        }),
        (zero_key, |e| matches!(e, WriteRefused::KeysTrivial { .. })),
        (ff_iv, |e| matches!(e, WriteRefused::KeysTrivial { .. })),
        (bad_serial, |e| {
            matches!(e, WriteRefused::KeysUnreadable { .. })
        }),
        (
            vec![0u8; crate::device::detect::MDF_MAX_SIZE as usize + 1],
            |e| matches!(e, WriteRefused::MdfUnreadable { .. }),
        ),
    ];
    for (mdf, expected) in cases {
        let dev = device(&mdf);
        for kind in [WriteKind::Send, WriteKind::Other] {
            let err = refused(dev.path(), &test_ctx(), kind);
            assert!(expected(&err), "{err}");
            assert!(err.to_string().contains("rien n'a été écrit"), "{err}");
        }
        // L'état affiché dit le même refus (un `.mdf` trop grand n'arrive pas jusqu'ici).
        if mdf.len() as u64 <= crate::device::detect::MDF_MAX_SIZE {
            let status = firmware_status(dev.path(), &mdf, &test_ctx(), false);
            assert!(
                status.write_block.as_ref().is_some_and(expected),
                "{status:?}"
            );
        }
    }
}

#[test]
fn le_message_d_un_firmware_refuse_garde_la_version() {
    let dev = device(&mdf_with(KNOWN_HEADER, "main: 3.1.4-x"));
    let err = refused(dev.path(), &test_ctx(), WriteKind::Send);
    assert!(err.to_string().contains("3.1.4"), "{err}");
}

#[test]
fn le_numero_de_serie_illisible_n_apparait_pas_dans_le_message() {
    let mut mdf = mdf_v2_known();
    mdf[0x32] = b'g';
    let dev = device(&mdf);
    let err = refused(dev.path(), &test_ctx(), WriteKind::Send);
    assert!(!err.to_string().contains("0123456789"), "{err}");
}

#[test]
fn une_racine_non_detectee_est_refusee() {
    let dev = device(&mdf_v1_real());
    let session = KeySession::default();
    let ctx = WriteContext::new(&session, Some(vec![PathBuf::from("Z:\\")]), HashSet::new());
    assert_eq!(
        refused(dev.path(), &ctx, WriteKind::Other),
        WriteRefused::InvalidRoot
    );
    let ctx = WriteContext::new(
        &session,
        Some(vec![dev.path().to_path_buf()]),
        HashSet::new(),
    );
    check_write(dev.path(), &ctx, WriteKind::Other).unwrap();
}

#[test]
fn v2_avec_ou_sans_main_taille_et_en_tete_quelconques_est_autorisee_sans_confirmation() {
    // #326 : aucune confirmation « j'ai sauvegardé ma Flam » (comme Lunii.QT).
    let mut long = mdf_v2_known();
    long.extend([0u8; 74]);
    for (mdf, standard) in [
        (mdf_v2_known(), true),
        (mdf_with(KNOWN_HEADER, "2.4.2-nue"), true),
        (mdf_with([0, 0], "main: 2.4.2-test"), false),
        (mdf_with([7, 9], "comm: 9.9\nmain: 2.4.2-x"), false),
        (long, false),
        (mdf_v2_known()[..MDF_MIN_LEN].to_vec(), false),
    ] {
        let layout = analyze(&mdf);
        assert_eq!(layout.family, Family::V2);
        assert_eq!(layout.version.as_deref(), Some("2.4.2"));
        assert_eq!(layout.standard, standard);
        let dev = device(&mdf);
        let ctx = test_ctx();
        for kind in [WriteKind::Send, WriteKind::Other] {
            let before = snapshot(dev.path());
            let permit = check_write(dev.path(), &ctx, kind).unwrap();
            assert_eq!(permit.family(), FirmwareFamily::V2);
            assert!(permit.same_mdf(&mdf));
            assert_eq!(snapshot(dev.path()), before, "la garde n'écrit jamais");
        }
        let status = firmware_status(dev.path(), &mdf, &ctx, false);
        assert_eq!(status.write_block, None);
        assert_eq!(status.layout_standard, standard);
    }
}

#[test]
fn comm_avant_main_donne_la_version_de_main() {
    let layout = analyze(&mdf_with(KNOWN_HEADER, "comm: 9.9.9\nmain: 2.4.2-x"));
    assert_eq!(layout.version.as_deref(), Some("2.4.2"));
    assert_eq!(layout.form, VersionForm::Main);
    assert_eq!(layout.family, Family::V2);
}

#[test]
fn coherence_histoire_tierce_au_format_flam() {
    let dev = device(&mdf_v2_known());
    story(dev.path(), STORY, &key_v2(), &info_v2("Histoire importée"));
    let ctx = test_ctx();
    let status = firmware_status(dev.path(), &mdf_v2_known(), &ctx, false);
    assert!(matches!(
        status.coherence,
        Coherence::Coherent { counts } if counts.decrypted == 1 && counts.examined == 1
    ));
}

/// #326 : une contradiction est un avertissement (faux positifs possibles : `info` en Latin-1 ou
/// de moins de 4 caractères) ; ni l'envoi ni le retrait ne sont refusés.
#[test]
fn contradiction_reelle_au_format_flam_avertit_sans_rien_refuser() {
    let dev = device(&mdf_v2_known());
    let ctx = test_ctx();
    story(dev.path(), STORY, &key_v2(), &wrong_info());
    // L'histoire en cause est désignée (#326).
    let status = firmware_status(dev.path(), &mdf_v2_known(), &ctx, false);
    assert!(
        matches!(
            &status.coherence,
            Coherence::Contradiction { counts, stories, total }
                if counts.key_matches == 1 && stories == &[STORY.to_string()] && *total == 1
        ),
        "{:?}",
        status.coherence
    );
    assert_eq!(status.write_block, None);
    for kind in [WriteKind::Send, WriteKind::Other] {
        check_write(dev.path(), &ctx, kind).unwrap();
    }
}

#[test]
fn les_histoires_en_contradiction_sont_bornees_et_triees() {
    let dev = device(&mdf_v2_known());
    for i in (0..MAX_CONTRADICTING + 3).rev() {
        let id = format!("00000000-0000-4000-8000-{i:012}");
        story(dev.path(), &id, &key_v2(), &wrong_info());
    }
    let coherence = coherence_v2(dev.path(), &mdf_v2_known(), &HashSet::new()).unwrap();
    let Coherence::Contradiction {
        counts,
        stories,
        total,
    } = coherence
    else {
        panic!("contradiction attendue : {coherence:?}");
    };
    assert_eq!(counts.key_matches, MAX_CONTRADICTING + 3);
    assert_eq!(stories.len(), MAX_CONTRADICTING);
    assert_eq!(total, MAX_CONTRADICTING + 3);
    assert_eq!(stories[0], "00000000-0000-4000-8000-000000000000");
    assert!(stories.windows(2).all(|w| w[0] < w[1]));
}

#[test]
fn histoire_au_format_lunii_ni_contradiction_ni_preuve() {
    let dev = device(&mdf_v2_known());
    let dir = dev.path().join("str").join(STORY);
    write(&dir.join("key"), &key_v2());
    write(&dir.join("info"), &wrong_info());
    for name in ["ni", "li", "ri", "si"] {
        write(&dir.join(name), b"x");
    }
    let ctx = test_ctx();
    let status = firmware_status(dev.path(), &mdf_v2_known(), &ctx, false);
    assert!(matches!(
        status.coherence,
        Coherence::NoEvidence { counts } if counts.lunii_format == 1 && counts.examined == 0
    ));
}

#[test]
fn info_en_clair_ecarte() {
    let dev = device(&mdf_v2_known());
    story(
        dev.path(),
        STORY,
        &key_v2(),
        b"Titre en clair\nSous-titre\n",
    );
    let ctx = test_ctx();
    let status = firmware_status(dev.path(), &mdf_v2_known(), &ctx, false);
    assert!(matches!(
        status.coherence,
        Coherence::NoEvidence { counts } if counts.clear_info == 1
    ));
}

#[test]
fn info_lisible_avec_un_autre_key_est_coherent() {
    let dev = device(&mdf_v2_known());
    story(dev.path(), STORY, &[b'z'; 32], &info_v2("Titre"));
    let ctx = test_ctx();
    let status = firmware_status(dev.path(), &mdf_v2_known(), &ctx, false);
    assert!(matches!(
        status.coherence,
        Coherence::Coherent { counts } if counts.decrypted_other_key == 1
    ));
}

#[test]
fn histoires_officielles_d_un_compte_ne_contredisent_pas() {
    // Comportement observé de la 2.x : `key` du compte, `info` chiffré avec la clé du compte.
    let dev = device(&mdf_v2_known());
    story(dev.path(), STORY, &[b'c'; 32], &wrong_info());
    let ctx = test_ctx();
    let status = firmware_status(dev.path(), &mdf_v2_known(), &ctx, false);
    assert!(
        matches!(
            status.coherence,
            Coherence::NoEvidence { counts } if counts.examined == 1 && counts.key_matches == 0
        ),
        "{:?}",
        status.coherence
    );
}

#[test]
fn une_histoire_coherente_l_emporte_sur_une_histoire_abimee() {
    let dev = device(&mdf_v2_known());
    story(dev.path(), STORY, &key_v2(), &info_v2("Titre"));
    story(dev.path(), OTHER, &key_v2(), &[0u8; 32]);
    let ctx = test_ctx();
    let status = firmware_status(dev.path(), &mdf_v2_known(), &ctx, false);
    assert!(
        matches!(
            status.coherence,
            Coherence::Coherent { counts } if counts.decrypted == 1 && counts.key_matches == 2
        ),
        "{:?}",
        status.coherence
    );
}

#[test]
fn les_histoires_du_launcher_n_entrent_pas_dans_la_coherence() {
    let dev = device(&mdf_v2_known());
    story(dev.path(), STORY, &key_v2(), &[0u8; 32]);
    let ctx = ctx_with([STORY.to_string()].into());
    let status = firmware_status(dev.path(), &mdf_v2_known(), &ctx, false);
    assert!(matches!(
        status.coherence,
        Coherence::NoEvidence { counts } if counts.examined == 0
    ));
}

#[test]
fn l_etat_expose_ne_contient_aucun_secret() {
    let dev = device(&mdf_v2_known());
    story(dev.path(), STORY, &key_v2(), &info_v2("Titre"));
    let ctx = test_ctx();
    let mdf = mdf_v2_known();
    let status = firmware_status(dev.path(), &mdf, &ctx, false);
    let json = serde_json::to_string(&status).unwrap().to_lowercase();
    let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    for secret in [
        hex(&TEST_KEY),
        hex(&TEST_IV),
        String::from_utf8(TEST_SERIAL_HEX.to_vec()).unwrap(),
        hex(&Sha256::digest(&mdf)),
        fingerprint(&mdf),
        hex(&key_v2()),
    ] {
        assert!(!json.contains(&secret), "{json}");
    }
    assert!(json.contains("\"version\":\"2.4.2\""), "{json}");
    assert!(json.contains("\"header\":\"01 00\""), "{json}");
    assert!(json.contains("\"family\":\"v2\""), "{json}");
    assert!(json.contains("\"usb_ids\":\"303a:819e\""), "{json}");
    assert!(!json.contains("trust"), "{json}");
    assert!(json.contains("\"kind\":\"coherent\""), "{json}");
}

#[test]
fn l_etat_pendant_une_ecriture_ne_refait_pas_les_controles() {
    let dev = device(&mdf_v2_known());
    let ctx = test_ctx();
    let status = firmware_status(dev.path(), &mdf_v2_known(), &ctx, true);
    assert_eq!(status.coherence, Coherence::NotChecked);
}

#[test]
fn etat_v1_et_mdf_illisible() {
    let dev = device(&mdf_v1_real());
    let status = firmware_status(dev.path(), &mdf_v1_real(), &test_ctx(), false);
    assert_eq!(status.family, Family::V1);
    assert_eq!(status.coherence, Coherence::NotApplicable);
    assert!(status.layout_standard);
    assert_eq!(status.write_block, None);
    let status = unreadable_status(70_000, "trop grand".into());
    assert!(matches!(
        status.write_block,
        Some(WriteRefused::MdfUnreadable { .. })
    ));
}

#[test]
fn vid_pid_inattendu_signale_sans_refus() {
    let mut mdf = mdf_v1_real();
    mdf[0x4A] = 0x34;
    let layout = analyze(&mdf);
    assert_ne!(layout.usb, Some(EXPECTED_USB));
    let dev = device(&mdf);
    check_write(dev.path(), &test_ctx(), WriteKind::Send).unwrap();
}

#[test]
fn un_permis_ne_vaut_que_pour_sa_racine() {
    let dev = device(&mdf_v1_real());
    let other = device(&mdf_v1_real());
    let permit = check_write(dev.path(), &test_ctx(), WriteKind::Other).unwrap();
    assert!(permit.covers(dev.path()));
    assert!(!permit.covers(other.path()));
}

#[test]
fn apres_un_retrait_l_etat_ne_montre_plus_l_histoire_retiree() {
    // #345 : le contrôle fait par la garde d'un retrait n'est pas réutilisé par « Ma Flam ».
    let dev = device(&mdf_v2_known());
    story(dev.path(), STORY, &key_v2(), &wrong_info());
    let ctx = test_ctx();
    let mdf = mdf_v2_known();
    let status = firmware_status(dev.path(), &mdf, &ctx, false);
    assert!(matches!(status.coherence, Coherence::Contradiction { .. }));
    // Retrait : garde, puis suppression du dossier.
    check_write(dev.path(), &ctx, WriteKind::Other).unwrap();
    // Pendant l'écriture : aucun contrôle ancien n'est montré.
    let status = firmware_status(dev.path(), &mdf, &ctx, true);
    assert_eq!(status.coherence, Coherence::NotChecked);
    std::fs::remove_dir_all(dev.path().join("str").join(STORY)).unwrap();
    // Aussitôt après (bien avant `STATUS_REUSE`) : contrôle refait.
    let status = firmware_status(dev.path(), &mdf, &ctx, false);
    assert!(
        matches!(status.coherence, Coherence::NoEvidence { .. }),
        "{:?}",
        status.coherence
    );
    assert_eq!(status.write_block, None);
    check_write(dev.path(), &ctx, WriteKind::Send).unwrap();
}

/// #326 : la cohérence ne sert qu'à l'affichage ; l'écriture ne la contrôle pas, et l'envoi
/// garde le contrôle de « Ma Flam » (il n'écrit que des histoires du launcher, exclues).
#[test]
fn l_ecriture_ne_controle_pas_la_coherence_et_l_envoi_garde_l_affichage() {
    let dev = device(&mdf_v2_known());
    let ctx = test_ctx();
    story(dev.path(), STORY, &key_v2(), &info_v2("Histoire importée"));
    check_write(dev.path(), &ctx, WriteKind::Send).unwrap();
    // Aucun contrôle fait par l'envoi : rien à réutiliser pendant l'écriture.
    let status = firmware_status(dev.path(), &mdf_v2_known(), &ctx, true);
    assert_eq!(status.coherence, Coherence::NotChecked);
    // Contrôle de l'affichage, puis envoi : réutilisé pendant l'écriture (Flam occupée).
    let status = firmware_status(dev.path(), &mdf_v2_known(), &ctx, false);
    assert!(matches!(status.coherence, Coherence::Coherent { .. }));
    check_write(dev.path(), &ctx, WriteKind::Send).unwrap();
    let status = firmware_status(dev.path(), &mdf_v2_known(), &ctx, true);
    assert!(matches!(status.coherence, Coherence::Coherent { .. }));
}

#[test]
fn un_info_dechiffre_sans_titre_n_est_pas_une_contradiction() {
    // #326 : le déchiffrement est jugé sur la structure du clair, pas sur le titre.
    let dev = device(&mdf_v2_known());
    let blank = encrypt_zero_padded(&TEST_KEY, &TEST_IV, b"\n   \n\n");
    story(dev.path(), STORY, &key_v2(), &blank);
    let ctx = test_ctx();
    let status = firmware_status(dev.path(), &mdf_v2_known(), &ctx, false);
    assert!(
        matches!(
            status.coherence,
            Coherence::Coherent { counts } if counts.decrypted == 1
        ),
        "{:?}",
        status.coherence
    );
    check_write(dev.path(), &ctx, WriteKind::Send).unwrap();
}

#[test]
fn une_version_a_point_final_est_lue() {
    for (text, family, version) in [
        ("1.15.14.", Family::V1, Some("1.15.14")),
        ("main: 2.4.2. comm: x", Family::V2, Some("2.4.2")),
        ("main: 2..-x", Family::Unsupported, Some("2.")),
        ("main: 2..1-x", Family::Unsupported, Some("2..1")),
        ("main: 2.4.x", Family::Unsupported, Some("2.4.")),
        ("1.x-", Family::Unsupported, Some("1.")),
        ("main: 2.4.2.-x", Family::V2, Some("2.4.2")),
    ] {
        let layout = analyze(&mdf_with(KNOWN_HEADER, text));
        assert_eq!(layout.family, family, "{text}");
        assert_eq!(layout.version.as_deref(), version, "{text}");
    }
}

#[test]
fn l_affichage_n_enumere_pas_les_lecteurs() {
    // #326 : `device_status` (scruté) n'a pas besoin des racines détectées.
    let dev = device(&mdf_v1_real());
    let ctx = WriteContext::current(None);
    let status = firmware_status(dev.path(), &mdf_v1_real(), &ctx, false);
    assert_eq!(status.family, Family::V1);
    assert!(ctx.detected_roots.get().is_none());
}

#[test]
fn chaque_ecriture_other_change_la_generation() {
    // #345 : une scrutation commencée avant un retrait ne garde pas son contrôle (génération
    // relue au stockage) ; ici, la génération avance à chaque garde `Other`, pas à l'envoi.
    let dev = device(&mdf_v2_known());
    let ctx = test_ctx();
    let key = session_key(dev.path(), &mdf_v2_known());
    let before = ctx.session.with(key.clone(), |e| e.generation);
    check_write(dev.path(), &ctx, WriteKind::Other).unwrap();
    assert_eq!(ctx.session.with(key.clone(), |e| e.generation), before + 1);
    check_write(dev.path(), &ctx, WriteKind::Send).unwrap();
    assert_eq!(ctx.session.with(key, |e| e.generation), before + 1);
}
