//! Tests de la garde commune des écritures (#317) : disposition de `.mdf`, famille de firmware,
//! preuve des clés 2.x, confirmation de session, état exposé. Fixtures partagées avec les tests
//! du retrait, du réordonnancement et du transfert.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use super::*;
use crate::device::crypto::tests::{TEST_IV, TEST_KEY};
use crate::device::crypto::{encrypt_zero_padded, key_file_from_serial, SERIAL_FIELD_LEN};

/// Numéro de série inventé des fixtures (14 caractères hexadécimaux).
pub(crate) const TEST_SERIAL_HEX: &[u8; 14] = b"00123456789abc";

/// `.mdf` synthétique : `header`, texte de version `text` (48 octets complétés de nuls),
/// numéro de série inventé, clé et IV de test à 0x4E et 0x5E.
pub(crate) fn mdf_with(header: [u8; 2], text: &str) -> Vec<u8> {
    let mut m = header.to_vec();
    let mut field = text.as_bytes().to_vec();
    field.resize(48, 0);
    m.extend(field);
    let mut serial = TEST_SERIAL_HEX.to_vec();
    serial.resize(SERIAL_FIELD_LEN, 0);
    m.extend(serial);
    m.extend([0u8; 4]);
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

/// Disposition 2.x reconnue (supposée) : en-tête `01 00`, « main: ».
pub(crate) fn mdf_v2_known() -> Vec<u8> {
    mdf_with(KNOWN_HEADER, "main: 2.4.2-test\ncomm: x")
}

/// Contexte de test : mémoire de session propre, aucun contrôle de racine, aucune histoire du
/// launcher.
pub(crate) fn test_ctx() -> WriteContext<'static> {
    let session: &'static KeySession = Box::leak(Box::default());
    WriteContext::new(session, None, HashSet::new())
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

const OFFICIAL: &str = "11111111-2222-4333-8444-555555555555";
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

/// Ajoute une histoire `uuid` (fichiers `key` et `info`).
pub(crate) fn story(root: &Path, uuid: &str, key: &[u8], info: &[u8]) {
    write(&root.join("str").join(uuid).join("key"), key);
    write(&root.join("str").join(uuid).join("info"), info);
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

fn refused(root: &Path, ctx: &WriteContext<'_>) -> WriteRefused {
    let before = snapshot(root);
    let err = check_write(root, ctx).unwrap_err();
    assert_eq!(snapshot(root), before, "la garde n'écrit jamais");
    err
}

#[test]
fn la_1_15_14_reelle_est_reconnue_et_autorisee() {
    let layout = analyze(&mdf_v1_real());
    assert_eq!(layout.family, Family::V1);
    assert_eq!(layout.form, VersionForm::Bare);
    assert_eq!(layout.header, Some([1, 0]));
    assert!(layout.recognized);
    let dev = device(&mdf_v1_real());
    let permit = check_write(dev.path(), &test_ctx()).unwrap();
    assert_eq!(permit.family(), FirmwareFamily::V1);
    assert!(permit.same_mdf(&mdf_v1_real()));
}

#[test]
fn une_1x_de_disposition_inconnue_est_refusee() {
    let mut long = mdf_v1_real();
    long.extend([0u8; 2]);
    let cases = [
        mdf_with([0, 0], "1.15.14-2d356995"),
        mdf_with([2, 0], "1.15.14-2d356995"),
        mdf_with(KNOWN_HEADER, "main: 1.15.14-x"),
        long,
    ];
    for mdf in cases {
        let dev = device(&mdf);
        let err = refused(dev.path(), &test_ctx());
        assert!(matches!(err, WriteRefused::UnknownLayout { .. }), "{err}");
    }
}

#[test]
fn mdf_absent_tronque_illisible_ou_firmware_non_pris_en_charge_refuse() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        check_write(dir.path(), &test_ctx()).unwrap_err(),
        WriteRefused::DeviceNotFound
    );
    type Expect = fn(&WriteRefused) -> bool;
    let cases: [(Vec<u8>, Expect); 6] = [
        (mdf_v1_real()[..100].to_vec(), |e| {
            matches!(e, WriteRefused::Truncated { size: 100 })
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
        (
            vec![0u8; crate::device::detect::MDF_MAX_SIZE as usize + 1],
            |e| matches!(e, WriteRefused::MdfUnreadable { .. }),
        ),
    ];
    for (mdf, expected) in cases {
        let dev = device(&mdf);
        let err = refused(dev.path(), &test_ctx());
        assert!(expected(&err), "{err}");
        assert!(err.to_string().contains("rien n'a été écrit"), "{err}");
    }
}

#[test]
fn le_message_d_un_firmware_refuse_garde_la_version() {
    let dev = device(&mdf_with(KNOWN_HEADER, "main: 3.1.4-x"));
    let err = refused(dev.path(), &test_ctx());
    assert!(err.to_string().contains("3.1.4"), "{err}");
}

#[test]
fn une_racine_non_detectee_est_refusee() {
    let dev = device(&mdf_v1_real());
    let session = KeySession::default();
    let ctx = WriteContext::new(&session, Some(vec![PathBuf::from("Z:\\")]), HashSet::new());
    assert_eq!(refused(dev.path(), &ctx), WriteRefused::InvalidRoot);
    let ctx = WriteContext::new(
        &session,
        Some(vec![dev.path().to_path_buf()]),
        HashSet::new(),
    );
    check_write(dev.path(), &ctx).unwrap();
}

#[test]
fn v2_verifiee_par_une_histoire_officielle() {
    let dev = device(&mdf_v2_known());
    story(
        dev.path(),
        OFFICIAL,
        &key_v2(),
        &info_v2("Histoire officielle"),
    );
    let ctx = test_ctx();
    let permit = check_write(dev.path(), &ctx).unwrap();
    assert_eq!(permit.family(), FirmwareFamily::V2);
    let status = firmware_status(dev.path(), &mdf_v2_known(), &ctx, false);
    assert!(matches!(
        status.keys,
        KeysState::Verified { counts } if counts.decrypted == 1 && counts.examined == 1
    ));
    assert_eq!(status.write_block, None);
}

#[test]
fn v2_disposition_inconnue_autorisee_seulement_par_la_preuve() {
    // En-tête `00 00` ou inconnu, `.mdf` long, version nue, « comm: » avant « main: ».
    let mut long = mdf_v2_known();
    long.extend([0u8; 74]);
    for mdf in [
        mdf_with([0, 0], "main: 2.4.2-test"),
        mdf_with([7, 9], "main: 2.4.2-test"),
        long,
        mdf_with(KNOWN_HEADER, "2.4.2-nue"),
        mdf_with([0, 0], "comm: 9.9\nmain: 2.4.2-x"),
    ] {
        assert!(!analyze(&mdf).recognized);
        let dev = device(&mdf);
        let ctx = test_ctx();
        // Sans histoire officielle : refus « disposition inconnue », et la confirmation est
        // refusée elle aussi.
        let err = refused(dev.path(), &ctx);
        assert!(matches!(err, WriteRefused::UnknownLayout { .. }), "{err}");
        assert!(matches!(
            confirm_unverified(dev.path(), &ctx),
            Err(WriteRefused::UnknownLayout { .. })
        ));
        assert!(check_write(dev.path(), &ctx).is_err());
        // Avec une histoire officielle déchiffrée : autorisé.
        story(dev.path(), OFFICIAL, &key_v2(), &info_v2("Titre"));
        check_write(dev.path(), &ctx).unwrap();
    }
}

#[test]
fn comm_avant_main_donne_la_version_de_main() {
    let layout = analyze(&mdf_with(KNOWN_HEADER, "comm: 9.9.9\nmain: 2.4.2-x"));
    assert_eq!(layout.version.as_deref(), Some("2.4.2"));
    assert_eq!(layout.form, VersionForm::Main);
    assert_eq!(layout.family, Family::V2);
    // Reconnue : 126 octets, en-tête connu, « main: » présent.
    assert!(layout.recognized);
}

#[test]
fn v2_contradiction_refusee_meme_apres_confirmation() {
    // Même fichier `key`, `info` illisible avec les clés de `.mdf` : clés fausses.
    let dev = device(&mdf_v2_known());
    let ctx = test_ctx();
    confirm_unverified(dev.path(), &ctx).unwrap();
    check_write(dev.path(), &ctx).unwrap();
    let wrong = encrypt_zero_padded(&[9u8; 16], &[8u8; 16], b"Autre\nclef\n000000\n");
    story(dev.path(), OFFICIAL, &key_v2(), &wrong);
    let err = refused(dev.path(), &ctx);
    assert!(
        matches!(err, WriteRefused::KeysContradiction { .. }),
        "{err}"
    );
    assert!(err.to_string().contains("« Ma Flam »"), "{err}");
}

#[test]
fn v2_contradiction_sur_le_fichier_key() {
    // `info` lisible avec les clés de `.mdf`, mais fichier `key` différent du déduit.
    let dev = device(&mdf_v2_known());
    story(dev.path(), OFFICIAL, &[b'z'; 32], &info_v2("Titre"));
    let err = refused(dev.path(), &test_ctx());
    assert!(
        matches!(err, WriteRefused::KeysContradiction { .. }),
        "{err}"
    );
}

#[test]
fn une_histoire_verifiee_l_emporte_sur_une_histoire_abimee() {
    let dev = device(&mdf_v2_known());
    story(dev.path(), OFFICIAL, &key_v2(), &info_v2("Titre"));
    story(dev.path(), OTHER, &key_v2(), &[0u8; 32]);
    check_write(dev.path(), &test_ctx()).unwrap();
}

#[test]
fn les_histoires_d_un_autre_compte_ne_prouvent_ni_ne_contredisent() {
    let dev = device(&mdf_v2_known());
    let wrong = encrypt_zero_padded(&[9u8; 16], &[8u8; 16], b"Autre\nclef\n000000\n");
    story(dev.path(), OTHER, &[b'z'; 32], &wrong);
    let err = refused(dev.path(), &test_ctx());
    assert!(matches!(err, WriteRefused::KeysUnverified { .. }), "{err}");
}

#[test]
fn les_histoires_du_launcher_ne_prouvent_rien() {
    let dev = device(&mdf_v2_known());
    story(
        dev.path(),
        OFFICIAL,
        &key_v2(),
        &info_v2("Envoyée par le launcher"),
    );
    let session = KeySession::default();
    let launcher: HashSet<String> = [OFFICIAL.to_string()].into();
    let ctx = WriteContext::new(&session, None, launcher);
    let err = refused(dev.path(), &ctx);
    assert!(matches!(err, WriteRefused::KeysUnverified { .. }), "{err}");
    let status = firmware_status(dev.path(), &mdf_v2_known(), &ctx, false);
    assert!(matches!(
        status.keys,
        KeysState::Unverifiable { counts } if counts.examined == 0
    ));
}

#[test]
fn v2_non_verifiable_autorisee_apres_confirmation_de_session() {
    let dev = device(&mdf_v2_known());
    let ctx = test_ctx();
    let err = refused(dev.path(), &ctx);
    assert!(matches!(err, WriteRefused::KeysUnverified { ref version } if version == "2.4.2"));
    let status = confirm_unverified(dev.path(), &ctx).unwrap();
    assert!(matches!(status.keys, KeysState::Confirmed { .. }));
    assert_eq!(status.write_block, None);
    check_write(dev.path(), &ctx).unwrap();
    // Une autre session (relance du launcher) redemande la confirmation.
    assert!(matches!(
        check_write(dev.path(), &test_ctx()),
        Err(WriteRefused::KeysUnverified { .. })
    ));
    // Une autre Flam (autre racine) aussi.
    let other = device(&mdf_v2_known());
    assert!(check_write(other.path(), &ctx).is_err());
}

#[test]
fn la_confirmation_est_refusee_hors_etat_non_verifiable() {
    // 1.x : rien à confirmer, l'état est rendu sans changement.
    let dev = device(&mdf_v1_real());
    let status = confirm_unverified(dev.path(), &test_ctx()).unwrap();
    assert_eq!(status.keys, KeysState::NotApplicable);
    // Contradiction : refus.
    let dev = device(&mdf_v2_known());
    story(dev.path(), OFFICIAL, &key_v2(), &[0u8; 64]);
    assert!(matches!(
        confirm_unverified(dev.path(), &test_ctx()),
        Err(WriteRefused::KeysContradiction { .. })
    ));
}

#[test]
fn numero_de_serie_illisible_en_2x_refuse() {
    let mut mdf = mdf_v2_known();
    mdf[0x32] = b'g';
    let dev = device(&mdf);
    let err = refused(dev.path(), &test_ctx());
    assert!(matches!(err, WriteRefused::KeysUnreadable { .. }), "{err}");
    assert!(!err.to_string().contains("0123456789"), "{err}");
}

#[test]
fn l_etat_expose_ne_contient_aucun_secret() {
    let dev = device(&mdf_v2_known());
    story(dev.path(), OFFICIAL, &key_v2(), &info_v2("Titre"));
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
        hex(&key_v2()),
    ] {
        assert!(!json.contains(&secret), "{json}");
    }
    assert!(json.contains("\"version\":\"2.4.2\""), "{json}");
    assert!(json.contains("\"header\":\"01 00\""), "{json}");
    assert!(json.contains("\"family\":\"v2\""), "{json}");
    assert!(json.contains("\"kind\":\"verified\""), "{json}");
}

#[test]
fn l_etat_pendant_une_ecriture_ne_refait_pas_la_preuve() {
    let dev = device(&mdf_v2_known());
    let ctx = test_ctx();
    let status = firmware_status(dev.path(), &mdf_v2_known(), &ctx, true);
    assert_eq!(status.keys, KeysState::NotChecked);
    // Une preuve déjà faite est réutilisée.
    story(dev.path(), OFFICIAL, &key_v2(), &info_v2("Titre"));
    check_write(dev.path(), &ctx).unwrap();
    let status = firmware_status(dev.path(), &mdf_v2_known(), &ctx, true);
    assert!(matches!(status.keys, KeysState::Verified { .. }));
}

#[test]
fn une_preuve_verifiee_est_memorisee_pour_la_session() {
    let dev = device(&mdf_v2_known());
    story(dev.path(), OFFICIAL, &key_v2(), &info_v2("Titre"));
    let ctx = test_ctx();
    check_write(dev.path(), &ctx).unwrap();
    // L'histoire officielle retirée ensuite : les clés restent vérifiées pour ce `.mdf`.
    std::fs::remove_dir_all(dev.path().join("str").join(OFFICIAL)).unwrap();
    check_write(dev.path(), &ctx).unwrap();
    // Un `.mdf` différent (autre Flam au même emplacement) est une autre entrée.
    let mut other = mdf_v2_known();
    other[0x32] = b'f';
    write(&dev.path().join(MDF_FILE), &other);
    assert!(check_write(dev.path(), &ctx).is_err());
}

#[test]
fn etat_v1_et_mdf_illisible() {
    let dev = device(&mdf_v1_real());
    let status = firmware_status(dev.path(), &mdf_v1_real(), &test_ctx(), false);
    assert_eq!(status.family, Family::V1);
    assert_eq!(status.keys, KeysState::NotApplicable);
    assert!(status.layout_recognized);
    assert_eq!(status.write_block, None);
    let status = unreadable_status(70_000, "trop grand".into());
    assert!(matches!(
        status.write_block,
        Some(WriteRefused::MdfUnreadable { .. })
    ));
}
