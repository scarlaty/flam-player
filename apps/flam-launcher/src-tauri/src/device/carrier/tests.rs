//! Tests du mode carrier : lecture du couple, contrôle de cohérence, conservation chiffrée.
//! Clés inventées (aucune vraie clé), faux appareil dans un dossier temporaire.

use std::io::Write;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

use super::*;
use crate::device::crypto::encrypt_zero_padded;

pub(crate) const OFFICIAL: &str = "f0e1d2c3-b4a5-9687-7869-5a4b3c2d1e0f";
const OTHER: &str = "11111111-2222-3333-4444-555555555555";
pub(crate) const INFO_PLAIN: &[u8] =
    b"Histoire officielle\nSous-titre\n000000\nHistoire officielle\n";

/// `bt` inventé : clé puis IV.
pub(crate) fn test_bt() -> [u8; 32] {
    std::array::from_fn(|i| 0xE0 ^ (i as u8).wrapping_mul(7))
}

/// `key` inventé de l'histoire officielle.
pub(crate) fn test_key_file() -> [u8; 32] {
    std::array::from_fn(|i| 0x70 + i as u8)
}

/// Autre compte : `bt` et `key` différents.
pub(crate) fn foreign_bt() -> [u8; 32] {
    std::array::from_fn(|i| 0x0F ^ (i as u8).wrapping_mul(13))
}

pub(crate) fn write(path: &Path, data: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, data).unwrap();
}

/// `info` chiffré avec le `bt` donné.
pub(crate) fn encrypted_info(bt: &[u8; 32], plain: &[u8]) -> Vec<u8> {
    let key: [u8; 16] = bt[..16].try_into().unwrap();
    let iv: [u8; 16] = bt[16..].try_into().unwrap();
    encrypt_zero_padded(&key, &iv, plain)
}

/// Histoire officielle `uuid` sous `sub` (`str` ou `str.hidden`), avec ou sans `bt`.
pub(crate) fn official_story(root: &Path, sub: &str, uuid: &str, with_bt: bool) {
    let dir = root.join(sub).join(uuid);
    write(&dir.join(KEY_FILE), &test_key_file());
    write(
        &dir.join(INFO_FILE),
        &encrypted_info(&test_bt(), INFO_PLAIN),
    );
    if with_bt {
        write(&dir.join(BT_FILE), &test_bt());
    }
}

fn keys() -> CarrierKeys {
    CarrierKeys::from_parts(&test_bt(), &test_key_file()).unwrap()
}

/// Protecteur de test : XOR réversible lié à l'entropie, avec pannes à la demande.
pub(crate) struct FakeProtector {
    pub fail: bool,
}

impl Protector for FakeProtector {
    fn protect(&self, plain: &[u8], entropy: &[u8]) -> io::Result<Vec<u8>> {
        if self.fail {
            return Err(io::Error::other("panne injectée"));
        }
        let mut out = entropy.to_vec();
        out.extend(plain.iter().map(|b| b ^ 0x5A));
        Ok(out)
    }
    fn unprotect(&self, blob: &[u8], entropy: &[u8]) -> io::Result<Zeroizing<Vec<u8>>> {
        let rest = blob
            .strip_prefix(entropy)
            .ok_or_else(|| io::Error::other("entropie différente"))?;
        Ok(Zeroizing::new(rest.iter().map(|b| b ^ 0x5A).collect()))
    }
}

pub(crate) fn fake_store(dir: &Path) -> CarrierStore {
    CarrierStore::new(dir.to_path_buf(), Box::new(FakeProtector { fail: false }))
}

/// Toutes les formes sous lesquelles un secret pourrait fuir dans un texte.
pub(crate) fn leak_forms(secret: &[u8]) -> Vec<String> {
    let hex: String = secret.iter().map(|b| format!("{b:02x}")).collect();
    let mut forms = vec![
        hex.clone(),
        hex.to_uppercase(),
        format!("{secret:?}"),
        format!("{:?}", &secret[..8]),
        hex[..16].to_string(),
    ];
    forms.push(String::from_utf8_lossy(secret).into_owned());
    forms
}

pub(crate) fn assert_no_leak(text: &str, what: &str) {
    for secret in [
        &test_bt()[..],
        &test_key_file()[..],
        &foreign_bt()[..],
        &foreign_key_file()[..],
    ] {
        for form in leak_forms(secret) {
            assert!(!text.contains(&form), "clé visible dans {what} : {text}");
        }
        for half in [&secret[..16], &secret[16..]] {
            let hex: String = half.iter().map(|b| format!("{b:02x}")).collect();
            assert!(!text.to_lowercase().contains(&hex), "demi-clé dans {what}");
        }
    }
}

#[test]
fn couple_lu_et_masque() {
    let k = keys();
    assert_eq!(k.key_file(), &test_key_file());
    assert_eq!(&k.story_key()[..], &test_bt()[..16]);
    assert_eq!(&k.story_iv()[..], &test_bt()[16..]);
    assert_no_leak(&format!("{k:?} {k:#?}"), "Debug");
    // Fichiers plus longs : les 32 premiers octets servent ; trop courts : refusés.
    let mut long_bt = test_bt().to_vec();
    long_bt.extend([1, 2, 3]);
    assert!(CarrierKeys::from_parts(&long_bt, &test_key_file()).is_ok());
    let err = CarrierKeys::from_parts(&test_bt()[..31], &test_key_file()).unwrap_err();
    assert!(matches!(err, CarrierError::InvalidSource(_)));
    assert_no_leak(&err.to_string(), "erreur");
    assert!(CarrierKeys::from_parts(&test_bt(), &test_key_file()[..10]).is_err());
}

#[test]
fn texte_lisible_ou_bruit() {
    assert_eq!(readable_text(b"Titre\n\0\0\0").as_deref(), Some("Titre\n"));
    assert!(readable_text(b"abc\0").is_none());
    assert!(readable_text(&[0xff, 0xfe, 0x41, 0x42, 0x43, 0x44]).is_none());
    assert!(readable_text(b"\x01\x02\x03\x04\x05\x06abcd").is_none());
    // Caractère coupé par la limite de lecture : toléré à la fin.
    let mut cut = "Été long".as_bytes().to_vec();
    cut.push(0xc3);
    assert_eq!(readable_text(&cut).as_deref(), Some("Été long"));
    // Mauvaise clé : bruit refusé.
    let wrong = CarrierKeys::from_parts(&foreign_bt(), &test_key_file()).unwrap();
    assert!(wrong
        .decrypt_info(&encrypted_info(&test_bt(), INFO_PLAIN))
        .is_none());
    assert_eq!(
        title_of("  Titre\u{7}  \nSuite"),
        "Titre",
        "contrôles retirés"
    );
}

#[test]
fn histoires_officielles_utilisables() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    official_story(root, "str", OFFICIAL, true);
    official_story(
        root,
        "str.hidden",
        "aaaaaaaa-0000-0000-0000-000000000001",
        true,
    );
    // Sans bt, bt d'un autre compte, info absent : non proposées.
    official_story(root, "str", OTHER, false);
    let wrong = root.join("str/bbbbbbbb-0000-0000-0000-000000000002");
    write(&wrong.join(BT_FILE), &foreign_bt());
    write(&wrong.join(KEY_FILE), &test_key_file());
    write(
        &wrong.join(INFO_FILE),
        &encrypted_info(&test_bt(), INFO_PLAIN),
    );
    write(
        &root.join("str/cccccccc-0000-0000-0000-000000000003/bt"),
        &test_bt(),
    );
    write(&root.join("str/pas-un-uuid/bt"), &test_bt());

    let found = list_candidates(root, None);
    assert_eq!(
        found,
        vec![
            CarrierCandidate {
                uuid: OFFICIAL.into(),
                title: "Histoire officielle".into(),
                hidden: false,
                with_stored: false,
            },
            CarrierCandidate {
                uuid: "aaaaaaaa-0000-0000-0000-000000000001".into(),
                title: "Histoire officielle".into(),
                hidden: true,
                with_stored: false,
            },
        ]
    );
    assert_no_leak(&serde_json::to_string(&found).unwrap(), "IPC");
    assert!(read_device_carrier(root, OFFICIAL, || None).is_ok());
    assert!(matches!(
        read_device_carrier(root, OTHER, || None),
        Err(CarrierError::InvalidSource(_))
    ));
    assert!(matches!(
        read_device_carrier(root, "../etc", || None),
        Err(CarrierError::InvalidSource(_))
    ));
}

#[test]
fn coherence_avec_la_flam() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    // L'histoire officielle n'a plus de bt : le couple fourni par fichier est reconnu.
    official_story(root, "str", OFFICIAL, false);
    assert_eq!(
        check_against_device(root, &keys(), &no_cancel())
            .unwrap()
            .as_str(),
        OFFICIAL
    );
    // Autre compte : bt différent, ou key différent.
    let foreign = CarrierKeys::from_parts(&foreign_bt(), &test_key_file()).unwrap();
    assert!(matches!(
        check_against_device(root, &foreign, &no_cancel()),
        Err(CarrierError::Mismatch)
    ));
    let other_key = CarrierKeys::from_parts(&test_bt(), &[9u8; 32]).unwrap();
    assert!(matches!(
        check_against_device(root, &other_key, &no_cancel()),
        Err(CarrierError::Mismatch)
    ));
    // Flam vide.
    let empty = tempfile::tempdir().unwrap();
    assert!(matches!(
        check_against_device(empty.path(), &keys(), &no_cancel()),
        Err(CarrierError::Mismatch)
    ));
    assert_no_leak(&CarrierError::Mismatch.to_string(), "erreur");
}

#[test]
fn source_fichier_bt_ou_archive() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("sauvegarde").join(OFFICIAL);
    write(&dir.join(BT_FILE), &test_bt());
    write(&dir.join(KEY_FILE), &test_key_file());
    let k = read_file_carrier(&dir.join(BT_FILE)).unwrap();
    assert_eq!(k.key_file(), &test_key_file());

    // `key` absent à côté du fichier choisi.
    write(&tmp.path().join("seul/bt"), &test_bt());
    assert!(matches!(
        read_file_carrier(&tmp.path().join("seul/bt")),
        Err(CarrierError::InvalidSource(_))
    ));
    // Fichier trop grand pour être un bt.
    write(&tmp.path().join("gros/bt"), &vec![1u8; 10_000]);
    write(&tmp.path().join("gros/key"), &test_key_file());
    assert!(read_file_carrier(&tmp.path().join("gros/bt")).is_err());

    // Archive zip : un seul bt, key du même dossier.
    let zip_path = tmp.path().join("sauvegarde.zip");
    let make_zip = |entries: &[(&str, &[u8])]| {
        let mut zip = zip::ZipWriter::new(File::create(&zip_path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        for (name, data) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
    };
    let bt = test_bt();
    let key = test_key_file();
    make_zip(&[
        (&format!("str/{OFFICIAL}/bt"), &bt),
        (&format!("str/{OFFICIAL}/key"), &key),
        ("str/autre/key", &[1u8; 32]),
    ]);
    assert_eq!(read_file_carrier(&zip_path).unwrap().key_file(), &key);
    make_zip(&[("a/bt", &bt), ("b/key", &key)]);
    assert!(
        read_file_carrier(&zip_path).is_err(),
        "key d'un autre dossier"
    );
    make_zip(&[
        ("a/bt", &bt),
        ("a/key", &key),
        ("b/bt", &bt),
        ("b/key", &key),
    ]);
    assert!(read_file_carrier(&zip_path).is_err(), "deux bt");
    assert!(read_file_carrier(&tmp.path().join("absent")).is_err());
}

#[test]
fn consentement_versionne() {
    assert!(!consent_is_current(None));
    let ok = CarrierConsent {
        text_version: WARNING_VERSION,
        accepted_at: "2026-10-17T10:00:00Z".into(),
    };
    assert!(consent_is_current(Some(&ok)));
    let old = CarrierConsent {
        text_version: WARNING_VERSION - 1,
        ..ok.clone()
    };
    assert!(!consent_is_current(Some(&old)));
    let undated = CarrierConsent {
        accepted_at: String::new(),
        ..ok
    };
    assert!(!consent_is_current(Some(&undated)));
}

#[test]
fn acceptation_horodatee_et_case_obligatoire() {
    let tmp = tempfile::tempdir().unwrap();
    let settings = SettingsStore::new(tmp.path());
    let now = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_791_000_000);
    assert!(accept(&settings, WARNING_VERSION, false, now).is_err());
    assert!(accept(&settings, WARNING_VERSION + 1, true, now).is_err());
    assert_eq!(settings.load().unwrap().carrier_consent, None);
    let consent = accept(&settings, WARNING_VERSION, true, now).unwrap();
    assert_eq!(consent.accepted_at, "2026-10-03T04:00:00Z");
    assert_eq!(
        settings.load().unwrap().carrier_consent,
        Some(consent.clone())
    );
    // `settings_set` ne peut ni le forger ni l'effacer.
    let mut forged = settings.load().unwrap();
    forged.carrier_consent = None;
    settings.update(forged).unwrap();
    assert_eq!(settings.load().unwrap().carrier_consent, Some(consent));
    let fresh = tempfile::tempdir().unwrap();
    let other = SettingsStore::new(fresh.path());
    let forged = crate::settings::Settings {
        carrier_consent: Some(CarrierConsent {
            text_version: WARNING_VERSION,
            accepted_at: "2026-01-01T00:00:00Z".into(),
        }),
        ..Default::default()
    };
    other.update(forged).unwrap();
    assert_eq!(other.load().unwrap().carrier_consent, None);
}

#[test]
fn avertissement_versionne_dans_la_documentation() {
    let doc = include_str!("../../../../../../docs/flam-carrier.md");
    assert!(
        doc.contains(WARNING_TEXT),
        "texte absent ou modifié dans la doc"
    );
    assert!(doc.contains(&format!("version {WARNING_VERSION}")));
}

#[test]
fn conservation_aller_retour_et_oubli() {
    let tmp = tempfile::tempdir().unwrap();
    let store = fake_store(&tmp.path().join("cles"));
    let mdf_a = vec![1u8; 126];
    let mut mdf_b = mdf_a.clone();
    mdf_b[MDF_SERIAL_OFFSET] = 2;
    let a = DeviceId::from_mdf(&mdf_a).unwrap();
    let b = DeviceId::from_mdf(&mdf_b).unwrap();
    assert!(DeviceId::from_mdf(&[0u8; 60]).is_none());
    assert!(!store.has(&a));
    assert!(store.load(&a).unwrap().is_none());
    store.save(&a, &keys()).unwrap();
    assert!(store.has(&a) && !store.has(&b));
    let back = store.load(&a).unwrap().unwrap();
    assert_eq!(back.key_file(), &test_key_file());
    assert_eq!(&back.story_key()[..], &test_bt()[..16]);
    assert_eq!(&back.story_iv()[..], &test_bt()[16..]);
    // Fichier d'une autre Flam copié à la place : refusé (entropie et identifiant liés).
    std::fs::copy(store.path(&a), store.path(&b)).unwrap();
    assert!(matches!(store.load(&b), Err(CarrierError::Store(_))));
    // Fichier abîmé.
    std::fs::write(store.path(&a), b"abime").unwrap();
    let err = store.load(&a).err().unwrap();
    assert_no_leak(&err.to_string(), "erreur");
    // Oubli d'une Flam, puis de toutes.
    store.save(&a, &keys()).unwrap();
    assert_eq!(store.forget(Some(&a)).unwrap(), 1);
    assert!(!store.has(&a) && store.has(&b));
    assert_eq!(store.forget(Some(&a)).unwrap(), 0);
    assert_eq!(store.forget(None).unwrap(), 1);
    assert!(std::fs::read_dir(tmp.path().join("cles"))
        .unwrap()
        .next()
        .is_none());
    // Nom de fichier : empreinte, pas le numéro de série.
    assert_eq!(format!("{a:?}"), "DeviceId { <masqué> }");
}

#[test]
fn panne_de_chiffrement_rien_n_est_ecrit() {
    let tmp = tempfile::tempdir().unwrap();
    let store = CarrierStore::new(
        tmp.path().join("cles"),
        Box::new(FakeProtector { fail: true }),
    );
    let id = DeviceId::from_mdf(&[3u8; 126]).unwrap();
    assert!(matches!(
        store.save(&id, &keys()),
        Err(CarrierError::Store(_))
    ));
    assert!(!tmp.path().join("cles").exists());
}

#[cfg(windows)]
#[test]
fn dpapi_aller_retour() {
    let plain = test_bt();
    let blob = Dpapi.protect(&plain, b"entropie").unwrap();
    assert!(
        !blob.windows(16).any(|w| w == &plain[..16]),
        "clair visible"
    );
    assert_eq!(
        &Dpapi.unprotect(&blob, b"entropie").unwrap()[..],
        &plain[..]
    );
    assert!(Dpapi.unprotect(&blob, b"autre").is_err(), "entropie liée");
    let mut damaged = blob.clone();
    let last = damaged.len() - 1;
    damaged[last] ^= 1;
    assert!(Dpapi.unprotect(&damaged, b"entropie").is_err());

    // Conservation réelle : le fichier ne contient aucune clé en clair.
    let tmp = tempfile::tempdir().unwrap();
    let store = CarrierStore::new(tmp.path().to_path_buf(), Box::new(Dpapi));
    let id = DeviceId::from_mdf(&[7u8; 126]).unwrap();
    store.save(&id, &keys()).unwrap();
    let on_disk = std::fs::read(store.path(&id)).unwrap();
    for secret in [&test_bt()[..16], &test_bt()[16..], &test_key_file()[..16]] {
        assert!(!on_disk.windows(secret.len()).any(|w| w == secret));
    }
    let back = store.load(&id).unwrap().unwrap();
    assert_eq!(back.key_file(), &test_key_file());
}

/// Sortie des journaux capturée pendant `f`.
pub(crate) fn capture_logs(f: impl FnOnce()) -> String {
    #[derive(Clone, Default)]
    struct Buf(std::sync::Arc<Mutex<Vec<u8>>>);
    impl io::Write for Buf {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(data);
            Ok(data.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let buf = Buf::default();
    let writer = buf.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    tracing::subscriber::with_default(subscriber, f);
    let out = buf.0.lock().unwrap().clone();
    String::from_utf8_lossy(&out).into_owned()
}

#[test]
fn etat_sans_cle_et_sans_lecture_avant_consentement() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("flam");
    official_story(&root, "str", OFFICIAL, true);
    let mut mdf = vec![0u8; 2];
    let mut text = b"main: 1.15.14-test\ncomm: x".to_vec();
    text.resize(48, 0);
    mdf.extend(text);
    mdf.resize(126, 0);
    write(&root.join(".mdf"), &mdf);
    let store = fake_store(&tmp.path().join("cles"));

    let before = carrier_state(&root, None, &store).unwrap();
    assert!(before.carrier_needed);
    assert_eq!(before.firmware.as_deref(), Some("1.15.14"));
    assert!(before.consent.is_none());
    assert!(
        before.candidates.is_empty(),
        "rien n'est lu avant consentement"
    );
    assert_eq!(before.warning_text, WARNING_TEXT);

    let consent = CarrierConsent {
        text_version: WARNING_VERSION,
        accepted_at: "2026-10-17T10:00:00Z".into(),
    };
    let logs = capture_logs(|| {
        let after = carrier_state(&root, Some(consent.clone()), &store).unwrap();
        assert_eq!(after.candidates.len(), 1);
        assert!(!after.stored_keys);
        assert_no_leak(&serde_json::to_string(&after).unwrap(), "IPC");
    });
    assert_no_leak(&logs, "journaux");
}

// --- Saisie du bt (#271) ----------------------------------------------------------------------

pub(crate) fn no_cancel() -> AtomicBool {
    AtomicBool::new(false)
}

/// `bt` en hexadécimal, tel qu'un utilisateur le saisirait.
pub(crate) fn hex_of(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `key` inventé d'un autre compte.
pub(crate) fn foreign_key_file() -> [u8; 32] {
    std::array::from_fn(|i| 0x21 ^ (i as u8).wrapping_mul(3))
}

/// `.mdf` de la disposition observée sur une Flam 1.x : en-tête `01 00`, version nue sans
/// « main: » complétée d'octets nuls, numéro de série inventé.
pub(crate) fn real_mdf() -> Vec<u8> {
    let mut mdf = vec![1u8, 0];
    let mut text = b"1.15.14-0a1b2c3d".to_vec();
    text.resize(48, 0);
    mdf.extend(text);
    let mut serial = b"00fedcba987654".to_vec();
    serial.resize(24, 0);
    mdf.extend(serial);
    mdf.resize(crate::device::crypto::MDF_LEN, 0);
    mdf
}

/// Histoire officielle comme sur une Flam 1.x réelle : `info`, `key`, `main.lsf`, `version`,
/// **sans** `bt`. `info` et `main.lsf` sont chiffrés avec `bt`.
pub(crate) fn real_story(
    root: &Path,
    sub: &str,
    uuid: &str,
    bt: &[u8; 32],
    key: &[u8; 32],
    title: &str,
) {
    let dir = root.join(sub).join(uuid);
    let plain = format!("{title}\nSous-titre\n000000\n{title}\n");
    write(&dir.join(INFO_FILE), &encrypted_info(bt, plain.as_bytes()));
    write(&dir.join(KEY_FILE), key);
    write(
        &dir.join("main.lsf"),
        &encrypted_info(bt, b"function setup() end"),
    );
    write(&dir.join("version"), b"1");
}

/// Flam 1.x réelle (aucun `bt`) : une histoire d'un autre compte, deux du compte.
pub(crate) fn real_device(root: &Path) {
    write(&root.join(".mdf"), &real_mdf());
    real_story(
        root,
        "str",
        "22222222-0000-0000-0000-000000000002",
        &foreign_bt(),
        &foreign_key_file(),
        "Autre compte",
    );
    real_story(
        root,
        "str",
        OFFICIAL,
        &test_bt(),
        &test_key_file(),
        "Histoire du compte",
    );
    real_story(
        root,
        "str",
        OTHER,
        &test_bt(),
        &test_key_file(),
        "Deuxième histoire",
    );
}

/// Contenu complet d'une arborescence (chemins relatifs et octets).
pub(crate) fn tree(root: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    let mut out = std::collections::BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if path.is_dir() {
                out.insert(format!("{rel}/"), Vec::new());
                stack.push(path);
            } else {
                out.insert(rel, std::fs::read(&path).unwrap());
            }
        }
    }
    out
}

fn consent() -> CarrierConsent {
    CarrierConsent {
        text_version: WARNING_VERSION,
        accepted_at: "2026-10-17T10:00:00Z".into(),
    }
}

fn unknown_bt() -> [u8; 32] {
    std::array::from_fn(|i| 0x99 ^ i as u8)
}

#[test]
fn saisie_hexadecimale_toleree_et_validee() {
    let bt = test_bt();
    let hex = hex_of(&bt);
    // Espaces, retours à la ligne, majuscules : tolérés.
    let spaced = hex
        .to_uppercase()
        .as_bytes()
        .chunks(8)
        .map(|c| std::str::from_utf8(c).unwrap())
        .collect::<Vec<_>>()
        .join(" \n\t");
    for text in [hex.clone(), spaced, format!("  {hex}  ")] {
        assert_eq!(&EnteredBt::new(text).parse().unwrap()[..], &bt[..]);
    }
    // Refus, sans reprendre le texte saisi.
    let mut invalid = hex.clone();
    invalid.replace_range(10..11, "g");
    let cases = [
        invalid,
        hex[..62].to_string(),
        format!("{hex}ab"),
        format!("{hex}a"),
        String::new(),
        format!("0x{}", &hex[2..]),
        "a".repeat(5000),
    ];
    for text in cases {
        let err = EnteredBt::new(text.clone()).parse().unwrap_err();
        assert!(matches!(err, CarrierError::InvalidSource(_)), "{err}");
        let shown = err.to_string();
        assert!(
            shown.starts_with("source des clés invalide : clé saisie"),
            "{shown}"
        );
        assert_no_leak(&shown, "erreur de saisie");
        assert!(!shown.contains(&hex[..12]), "texte saisi repris");
    }
    // `Debug` masqué ; désérialisation depuis l'IPC.
    let choice: CarrierChoice =
        serde_json::from_str(&format!(r#"{{"kind":"entered","bt":"{hex}"}}"#)).unwrap();
    assert_no_leak(&format!("{choice:?} {choice:#?}"), "Debug");
    assert_eq!(
        format!("{:?}", EnteredBt::new(hex.clone())),
        "EnteredBt { <masqué> }"
    );
    assert_eq!(
        choice,
        CarrierChoice::Entered {
            bt: EnteredBt::new(hex)
        }
    );
}

#[test]
fn bt_saisi_associe_a_une_histoire_de_la_flam() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    real_device(root);
    // Clé du compte : `key` de la première histoire dont elle déchiffre `info`.
    let keys = find_by_bt(root, &test_bt(), &no_cancel()).unwrap();
    assert_eq!(keys.key_file(), &test_key_file());
    assert_eq!(&keys.story_key()[..], &test_bt()[..16]);
    assert_eq!(&keys.story_iv()[..], &test_bt()[16..]);
    // Autre compte : le `key` de son histoire.
    let other = find_by_bt(root, &foreign_bt(), &no_cancel()).unwrap();
    assert_eq!(other.key_file(), &foreign_key_file());
    // Aucune histoire ne correspond.
    let err = find_by_bt(root, &unknown_bt(), &no_cancel()).err().unwrap();
    assert!(matches!(err, CarrierError::EnteredMismatch));
    assert_eq!(
        err.to_string(),
        "cette clé ne correspond à aucune histoire officielle de cette Flam : rien n'a été écrit"
    );
    // Annulation : vérifiée avant chaque histoire.
    assert!(matches!(
        find_by_bt(root, &test_bt(), &AtomicBool::new(true)),
        Err(CarrierError::Cancelled)
    ));
    // Flam vide.
    let empty = tempfile::tempdir().unwrap();
    assert!(matches!(
        find_by_bt(empty.path(), &test_bt(), &no_cancel()),
        Err(CarrierError::EnteredMismatch)
    ));
}

#[test]
fn bt_saisi_associe_a_une_histoire_masquee() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(&root.join(".mdf"), &real_mdf());
    real_story(
        root,
        "str",
        OTHER,
        &foreign_bt(),
        &foreign_key_file(),
        "Autre compte",
    );
    real_story(
        root,
        "str.hidden",
        OFFICIAL,
        &test_bt(),
        &test_key_file(),
        "Masquée",
    );
    let keys = find_by_bt(root, &test_bt(), &no_cancel()).unwrap();
    assert_eq!(keys.key_file(), &test_key_file());
    assert_eq!(
        check_against_device(root, &keys, &no_cancel())
            .unwrap()
            .as_str(),
        OFFICIAL
    );
}

#[test]
fn resolution_d_un_bt_saisi() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("flam");
    real_device(&root);
    let keys_dir = tmp.path().join("cles");
    let store = fake_store(&keys_dir);
    let before = tree(&root);
    let c = consent();
    let mdf = real_mdf();
    let run = |text: String, consent: Option<&CarrierConsent>, cancel: bool| {
        let request = CarrierRequest {
            consent,
            choice: CarrierChoice::Entered {
                bt: EnteredBt::new(text),
            },
            remember: true,
            store: &store,
        };
        resolve(&root, &mdf, &request, &AtomicBool::new(cancel))
    };
    let good = hex_of(&test_bt()).to_uppercase();
    let logs = capture_logs(|| {
        // Sans consentement : refus avant toute lecture.
        let err = run(good.clone(), None, false).err().unwrap();
        assert!(matches!(err, CarrierError::ConsentRequired));
        // Saisie invalide, clé ne correspondant à rien, annulation : rien n'est conservé.
        let err = run(good[..60].to_string(), Some(&c), false).err().unwrap();
        assert!(matches!(err, CarrierError::InvalidSource(_)));
        let err = run(hex_of(&unknown_bt()), Some(&c), false).err().unwrap();
        assert!(matches!(err, CarrierError::EnteredMismatch));
        assert_no_leak(&format!("{err} {err:?}"), "erreur");
        let err = run(good.clone(), Some(&c), true).err().unwrap();
        assert!(matches!(err, CarrierError::Cancelled));
        assert!(!keys_dir.exists(), "rien conservé sur le PC");

        // Clé du compte : couple complet, conservé (chiffré) pour cette Flam.
        let keys = run(good.clone(), Some(&c), false).unwrap();
        assert_eq!(keys.key_file(), &test_key_file());
        assert_no_leak(&format!("{keys:?}"), "Debug");
    });
    assert_no_leak(&logs, "journaux");
    assert_eq!(tree(&root), before, "Flam jamais modifiée");
    let id = DeviceId::from_mdf(&mdf).unwrap();
    let stored = store.load(&id).unwrap().unwrap();
    assert_eq!(stored.key_file(), &test_key_file());
    for name in tree(&keys_dir).keys() {
        assert_no_leak(name, "nom de fichier");
    }
}

#[test]
fn histoires_sans_bt_proposees_avec_la_cle_conservee() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("flam");
    real_device(&root);
    real_story(
        &root,
        "str.hidden",
        "aaaaaaaa-0000-0000-0000-000000000001",
        &test_bt(),
        &test_key_file(),
        "Masquée",
    );
    // Sans clé conservée : aucune histoire (aucun `bt` sur la Flam).
    assert!(list_candidates(&root, None).is_empty());
    let stored = keys();
    let found = list_candidates(&root, Some(&stored));
    let summary: Vec<(&str, &str, bool, bool)> = found
        .iter()
        .map(|c| (c.uuid.as_str(), c.title.as_str(), c.hidden, c.with_stored))
        .collect();
    assert_eq!(
        summary,
        [
            (OTHER, "Deuxième histoire", false, true),
            (OFFICIAL, "Histoire du compte", false, true),
            (
                "aaaaaaaa-0000-0000-0000-000000000001",
                "Masquée",
                true,
                true
            ),
        ]
    );
    assert_no_leak(&serde_json::to_string(&found).unwrap(), "IPC");
    // Choix d'une de ces histoires : `bt` conservé, `key` de l'histoire.
    let k = read_device_carrier(&root, OFFICIAL, || Some(keys())).unwrap();
    assert_eq!(k.key_file(), &test_key_file());
    // Sans clé conservée, ou avec celle d'un autre compte : refusé.
    assert!(matches!(
        read_device_carrier(&root, OFFICIAL, || None),
        Err(CarrierError::InvalidSource(_))
    ));
    let foreign = CarrierKeys::from_parts(&foreign_bt(), &foreign_key_file()).unwrap();
    assert!(read_device_carrier(&root, OFFICIAL, || Some(foreign)).is_err());
}

#[test]
fn etat_avec_cle_conservee_et_flam_sans_bt() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("flam");
    real_device(&root);
    // Version préfixée « main: » : la lecture de la version nue relève de #269.
    let mut mdf = real_mdf();
    let mut text = b"main: 1.15.14-test\ncomm: x".to_vec();
    text.resize(48, 0);
    mdf[2..50].copy_from_slice(&text);
    write(&root.join(".mdf"), &mdf);
    let id = DeviceId::from_mdf(&mdf).unwrap();
    let store = fake_store(&tmp.path().join("cles"));
    let empty = carrier_state(&root, Some(consent()), &store).unwrap();
    assert!(empty.carrier_needed && !empty.stored_keys);
    assert!(empty.candidates.is_empty());
    store.save(&id, &keys()).unwrap();
    let logs = capture_logs(|| {
        let state = carrier_state(&root, Some(consent()), &store).unwrap();
        assert!(state.stored_keys);
        assert_eq!(state.candidates.len(), 2);
        assert!(state.candidates.iter().all(|c| c.with_stored));
        assert_no_leak(&serde_json::to_string(&state).unwrap(), "IPC");
    });
    assert_no_leak(&logs, "journaux");
    // Clés conservées illisibles : liste sans elles, sans échec.
    std::fs::write(store.path(&id), b"abime").unwrap();
    let state = carrier_state(&root, Some(consent()), &store).unwrap();
    assert!(state.stored_keys && state.candidates.is_empty());
}
