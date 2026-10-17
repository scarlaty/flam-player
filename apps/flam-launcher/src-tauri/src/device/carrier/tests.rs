//! Tests du mode carrier : lecture du couple, contrôle de cohérence, conservation chiffrée.
//! Clés inventées (aucune vraie clé), faux appareil dans un dossier temporaire.

use std::io::Write;
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
    for secret in [&test_bt()[..], &test_key_file()[..], &foreign_bt()[..]] {
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

    let found = list_candidates(root);
    assert_eq!(
        found,
        vec![
            CarrierCandidate {
                uuid: OFFICIAL.into(),
                title: "Histoire officielle".into(),
                hidden: false,
            },
            CarrierCandidate {
                uuid: "aaaaaaaa-0000-0000-0000-000000000001".into(),
                title: "Histoire officielle".into(),
                hidden: true,
            },
        ]
    );
    assert_no_leak(&serde_json::to_string(&found).unwrap(), "IPC");
    assert!(read_device_carrier(root, OFFICIAL).is_ok());
    assert!(matches!(
        read_device_carrier(root, OTHER),
        Err(CarrierError::InvalidSource(_))
    ));
    assert!(matches!(
        read_device_carrier(root, "../etc"),
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
        check_against_device(root, &keys()).unwrap().as_str(),
        OFFICIAL
    );
    // Autre compte : bt différent, ou key différent.
    let foreign = CarrierKeys::from_parts(&foreign_bt(), &test_key_file()).unwrap();
    assert!(matches!(
        check_against_device(root, &foreign),
        Err(CarrierError::Mismatch)
    ));
    let other_key = CarrierKeys::from_parts(&test_bt(), &[9u8; 32]).unwrap();
    assert!(matches!(
        check_against_device(root, &other_key),
        Err(CarrierError::Mismatch)
    ));
    // Flam vide.
    let empty = tempfile::tempdir().unwrap();
    assert!(matches!(
        check_against_device(empty.path(), &keys()),
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
