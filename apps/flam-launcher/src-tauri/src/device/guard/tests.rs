//! Tests de la garde commune des écritures (#317) : conditions communes, disposition non
//! standard signalée sans refus, confirmation mémorisée par empreinte, vérification par
//! `library.cache`, cohérence des clés 2.x, état exposé. Fixtures partagées avec les tests du
//! retrait, du réordonnancement et du transfert.

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

/// Contexte de test : mémoires propres (en mémoire seulement), aucun contrôle de racine,
/// aucune histoire du launcher.
pub(crate) fn test_ctx() -> WriteContext<'static> {
    ctx_with(HashSet::new())
}

fn ctx_with(launcher: HashSet<String>) -> WriteContext<'static> {
    let session: &'static KeySession = Box::leak(Box::default());
    let trust: &'static TrustStore = Box::leak(Box::default());
    WriteContext::new(session, trust, None, launcher)
}

/// Contexte de test où la Flam de `root` est déjà confirmée (2.x).
pub(crate) fn confirmed_ctx(root: &Path) -> WriteContext<'static> {
    let ctx = test_ctx();
    confirm(root, &ctx).unwrap();
    ctx
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

fn wrong_info() -> Vec<u8> {
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
        // La confirmation ne lève aucun de ces refus.
        assert!(confirm(dev.path(), &test_ctx()).is_err());
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
    let trust = TrustStore::default();
    let ctx = WriteContext::new(
        &session,
        &trust,
        Some(vec![PathBuf::from("Z:\\")]),
        HashSet::new(),
    );
    assert_eq!(
        refused(dev.path(), &ctx, WriteKind::Other),
        WriteRefused::InvalidRoot
    );
    let ctx = WriteContext::new(
        &session,
        &trust,
        Some(vec![dev.path().to_path_buf()]),
        HashSet::new(),
    );
    check_write(dev.path(), &ctx, WriteKind::Other).unwrap();
}

#[test]
fn v2_avec_ou_sans_main_taille_et_en_tete_quelconques_demande_une_confirmation() {
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
        let dev = device(&mdf);
        let ctx = test_ctx();
        for kind in [WriteKind::Send, WriteKind::Other] {
            let err = refused(dev.path(), &ctx, kind);
            assert_eq!(
                err,
                WriteRefused::ConfirmationRequired {
                    version: "2.4.2".into(),
                    standard
                }
            );
        }
        let status = confirm(dev.path(), &ctx).unwrap();
        assert_eq!(status.trust, Trust::Confirmed);
        assert_eq!(status.write_block, None);
        check_write(dev.path(), &ctx, WriteKind::Send).unwrap();
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
fn la_confirmation_est_memorisee_par_empreinte_d_une_session_a_l_autre() {
    let data = tempfile::tempdir().unwrap();
    let file = data.path().join(TRUST_FILE);
    let dev = device(&mdf_v2_known());
    {
        let session = KeySession::default();
        let trust = TrustStore::at(file.clone());
        let ctx = WriteContext::new(&session, &trust, None, HashSet::new());
        assert!(check_write(dev.path(), &ctx, WriteKind::Send).is_err());
        confirm(dev.path(), &ctx).unwrap();
    }
    // Nouvelle session (relance du launcher), même PC : la confirmation est relue.
    let session = KeySession::default();
    let trust = TrustStore::at(file.clone());
    let ctx = WriteContext::new(&session, &trust, None, HashSet::new());
    check_write(dev.path(), &ctx, WriteKind::Send).unwrap();
    // Le fichier ne contient ni clé, ni numéro de série, ni le `.mdf`.
    let saved = std::fs::read_to_string(&file).unwrap().to_lowercase();
    let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    for secret in [hex(&TEST_KEY), hex(&TEST_IV), "00123456789abc".to_string()] {
        assert!(!saved.contains(&secret), "{saved}");
    }
    assert!(!saved.contains(&hex(&Sha256::digest(mdf_v2_known()))));
    // Une autre Flam (autre `.mdf`) redemande la confirmation.
    let mut other = mdf_v2_known();
    other[0x32] = b'f';
    let dev2 = device(&other);
    assert!(matches!(
        check_write(dev2.path(), &ctx, WriteKind::Send),
        Err(WriteRefused::ConfirmationRequired { .. })
    ));
}

#[test]
fn une_memoire_illisible_redemande_la_confirmation() {
    let data = tempfile::tempdir().unwrap();
    let file = data.path().join(TRUST_FILE);
    std::fs::write(&file, b"pas du json").unwrap();
    let dev = device(&mdf_v2_known());
    let session = KeySession::default();
    let trust = TrustStore::at(file);
    let ctx = WriteContext::new(&session, &trust, None, HashSet::new());
    assert!(matches!(
        check_write(dev.path(), &ctx, WriteKind::Send),
        Err(WriteRefused::ConfirmationRequired { .. })
    ));
}

#[test]
fn coherence_histoire_tierce_au_format_flam() {
    let dev = device(&mdf_v2_known());
    story(dev.path(), STORY, &key_v2(), &info_v2("Histoire importée"));
    let ctx = confirmed_ctx(dev.path());
    check_write(dev.path(), &ctx, WriteKind::Send).unwrap();
    let status = firmware_status(dev.path(), &mdf_v2_known(), &ctx, false);
    assert!(matches!(
        status.coherence,
        Coherence::Coherent { counts } if counts.decrypted == 1 && counts.examined == 1
    ));
}

#[test]
fn contradiction_reelle_au_format_flam_refuse_l_envoi_pas_le_retrait() {
    let dev = device(&mdf_v2_known());
    let ctx = confirmed_ctx(dev.path());
    story(dev.path(), STORY, &key_v2(), &wrong_info());
    let err = refused(dev.path(), &ctx, WriteKind::Send);
    assert!(
        matches!(err, WriteRefused::KeysContradiction { .. }),
        "{err}"
    );
    assert!(err.to_string().contains("« Ma Flam »"), "{err}");
    // Le retrait reste possible (rien n'est chiffré).
    check_write(dev.path(), &ctx, WriteKind::Other).unwrap();
    // La confirmation reste possible sur une Flam non encore confirmée.
    let fresh = test_ctx();
    confirm(dev.path(), &fresh).unwrap();
    check_write(dev.path(), &fresh, WriteKind::Other).unwrap();
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
    let ctx = confirmed_ctx(dev.path());
    check_write(dev.path(), &ctx, WriteKind::Send).unwrap();
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
    let ctx = confirmed_ctx(dev.path());
    check_write(dev.path(), &ctx, WriteKind::Send).unwrap();
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
    let ctx = confirmed_ctx(dev.path());
    check_write(dev.path(), &ctx, WriteKind::Send).unwrap();
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
    let ctx = confirmed_ctx(dev.path());
    check_write(dev.path(), &ctx, WriteKind::Send).unwrap();
}

#[test]
fn une_histoire_coherente_l_emporte_sur_une_histoire_abimee() {
    let dev = device(&mdf_v2_known());
    story(dev.path(), STORY, &key_v2(), &info_v2("Titre"));
    story(dev.path(), OTHER, &key_v2(), &[0u8; 32]);
    let ctx = confirmed_ctx(dev.path());
    check_write(dev.path(), &ctx, WriteKind::Send).unwrap();
}

#[test]
fn les_histoires_du_launcher_n_entrent_pas_dans_la_coherence() {
    let dev = device(&mdf_v2_known());
    story(dev.path(), STORY, &key_v2(), &[0u8; 32]);
    let ctx = ctx_with([STORY.to_string()].into());
    confirm(dev.path(), &ctx).unwrap();
    check_write(dev.path(), &ctx, WriteKind::Send).unwrap();
}

fn put_cache(root: &Path, data: &[u8]) {
    write(&root.join(LIBRARY_CACHE), data);
}

#[test]
fn verification_par_la_flam_via_library_cache() {
    let data = tempfile::tempdir().unwrap();
    let file = data.path().join(TRUST_FILE);
    let dev = device(&mdf_v2_known());
    story(dev.path(), STORY, &key_v2(), &info_v2("Capi et la rivière"));
    let launcher: HashSet<String> = [STORY.to_string()].into();
    // Cache sans le titre (avant le redémarrage de la Flam) : rien n'est prouvé.
    put_cache(dev.path(), b"\x00\x01ancien cache sans le titre\x00");
    let session = KeySession::default();
    let trust = TrustStore::at(file.clone());
    let ctx = WriteContext::new(&session, &trust, None, launcher.clone());
    assert!(matches!(
        check_write(dev.path(), &ctx, WriteKind::Send),
        Err(WriteRefused::ConfirmationRequired { .. })
    ));
    // Cache reconstruit par la Flam avec le titre déchiffré (UTF-8) : vérifié, sans confirmation.
    let mut cache = b"\x00\x02".to_vec();
    cache.extend("Capi et la rivière".as_bytes());
    cache.extend(b"\x00\x00");
    put_cache(dev.path(), &cache);
    let before = snapshot(dev.path());
    check_write(dev.path(), &ctx, WriteKind::Send).unwrap();
    assert_eq!(
        snapshot(dev.path()),
        before,
        "le cache n'est pas supprimé par la garde"
    );
    let status = firmware_status(dev.path(), &mdf_v2_known(), &ctx, false);
    assert_eq!(status.trust, Trust::FlamVerified);
    // Mémorisé : une autre session, cache supprimé depuis, reste vérifiée.
    std::fs::remove_file(dev.path().join(LIBRARY_CACHE)).unwrap();
    let session = KeySession::default();
    let trust = TrustStore::at(file);
    let ctx = WriteContext::new(&session, &trust, None, launcher);
    check_write(dev.path(), &ctx, WriteKind::Send).unwrap();
}

#[test]
fn verification_par_la_flam_en_utf16() {
    let dev = device(&mdf_v2_known());
    story(dev.path(), STORY, &key_v2(), &info_v2("Titre long"));
    let cache: Vec<u8> = "xxTitre longyy"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    put_cache(dev.path(), &cache);
    assert!(flam_cache_proof(
        dev.path(),
        &mdf_v2_known(),
        &[STORY.to_string()].into()
    ));
}

#[test]
fn le_cache_ne_prouve_rien_pour_une_histoire_hors_launcher_ou_un_titre_court() {
    let dev = device(&mdf_v2_known());
    story(dev.path(), STORY, &key_v2(), &info_v2("Titre officiel"));
    put_cache(dev.path(), "Titre officiel".as_bytes());
    // Histoire non installée par le launcher.
    assert!(!flam_cache_proof(
        dev.path(),
        &mdf_v2_known(),
        &HashSet::new()
    ));
    assert!(!flam_cache_proof(
        dev.path(),
        &mdf_v2_known(),
        &[OTHER.to_string()].into()
    ));
    // Titre trop court pour être distinctif.
    let dev = device(&mdf_v2_known());
    story(dev.path(), STORY, &key_v2(), &info_v2("Abc"));
    put_cache(dev.path(), b"Abc");
    assert!(!flam_cache_proof(
        dev.path(),
        &mdf_v2_known(),
        &[STORY.to_string()].into()
    ));
}

#[test]
fn l_etat_expose_ne_contient_aucun_secret() {
    let dev = device(&mdf_v2_known());
    story(dev.path(), STORY, &key_v2(), &info_v2("Titre"));
    let ctx = confirmed_ctx(dev.path());
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
    assert!(json.contains("\"trust\":\"confirmed\""), "{json}");
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
    assert_eq!(status.trust, Trust::NotApplicable);
    assert!(status.layout_standard);
    assert_eq!(status.write_block, None);
    // 1.x : rien à confirmer, l'état est rendu tel quel.
    assert_eq!(confirm(dev.path(), &test_ctx()).unwrap(), status);
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
