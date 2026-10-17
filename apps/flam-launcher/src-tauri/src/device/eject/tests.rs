//! Tests de l'éjection : logique sur faux appareil avec un système simulé ; éjection réelle
//! dans un test `#[ignore]`.

use std::cell::RefCell;

use super::*;

/// Système simulé : résultats programmés, appels enregistrés.
struct Fake {
    prepare: Result<(), PrepareError>,
    eject: Result<(), String>,
    calls: RefCell<Vec<&'static str>>,
}

impl Fake {
    fn new(prepare: Result<(), PrepareError>, eject: Result<(), String>) -> Self {
        Self {
            prepare,
            eject,
            calls: RefCell::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<&'static str> {
        self.calls.borrow().clone()
    }
}

impl EjectSystem for Fake {
    fn prepare(&self, _root: &Path) -> Result<(), PrepareError> {
        self.calls.borrow_mut().push("prepare");
        self.prepare.clone()
    }

    fn request_eject(&self, _root: &Path) -> Result<(), String> {
        self.calls.borrow_mut().push("eject");
        self.eject.clone()
    }
}

fn device() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join(MDF_FILE), [7u8; 126]).unwrap();
    dir
}

#[test]
fn ejection_reussie() {
    let dir = device();
    let lock = DeviceLock::new();
    let fake = Fake::new(Ok(()), Ok(()));
    let outcome = eject(dir.path(), &lock, &fake).unwrap();
    assert_eq!(outcome, EjectOutcome::ejected());
    assert_eq!(fake.calls(), ["prepare", "eject"]);
    assert_eq!(lock.current(), None);
}

#[test]
fn ejection_refusee_pendant_un_transfert() {
    let dir = device();
    let lock = DeviceLock::new();
    let _transfer = lock.try_begin(Operation::Transfer).unwrap();
    let fake = Fake::new(Ok(()), Ok(()));
    let err = eject(dir.path(), &lock, &fake).unwrap_err();
    assert!(matches!(err, EjectError::Busy(Busy(Operation::Transfer))));
    assert!(fake.calls().is_empty(), "aucun appel système");
    assert_eq!(lock.current(), Some(Operation::Transfer));
}

#[test]
fn ejection_refusee_pendant_un_retrait() {
    let dir = device();
    let lock = DeviceLock::new();
    let _remove = lock.try_begin(Operation::Remove).unwrap();
    let fake = Fake::new(Ok(()), Ok(()));
    assert!(eject(dir.path(), &lock, &fake).is_err());
    assert!(fake.calls().is_empty());
}

#[test]
fn le_verrou_bloque_un_transfert_pendant_l_ejection() {
    struct Probe<'a>(&'a DeviceLock);
    impl EjectSystem for Probe<'_> {
        fn prepare(&self, _root: &Path) -> Result<(), PrepareError> {
            assert_eq!(
                self.0.try_begin(Operation::Transfer).unwrap_err(),
                Busy(Operation::Eject)
            );
            Ok(())
        }
        fn request_eject(&self, _root: &Path) -> Result<(), String> {
            Ok(())
        }
    }
    let dir = device();
    let lock = DeviceLock::new();
    eject(dir.path(), &lock, &Probe(&lock)).unwrap();
    assert_eq!(lock.current(), None);
}

#[test]
fn volume_occupe_donne_le_repli_sans_demande_d_ejection() {
    let dir = device();
    let lock = DeviceLock::new();
    let fake = Fake::new(Err(PrepareError::InUse("accès refusé".into())), Ok(()));
    let outcome = eject(dir.path(), &lock, &fake).unwrap();
    let EjectOutcome::Fallback { reason, message } = outcome else {
        panic!("repli attendu");
    };
    assert!(reason.contains("ouvert par un autre programme"), "{reason}");
    assert_eq!(message, FALLBACK_MESSAGE);
    assert!(message.contains("Retirez la Flam en toute sécurité"));
    assert_eq!(fake.calls(), ["prepare"]);
    assert_eq!(lock.current(), None);
}

#[test]
fn echec_simule_de_l_ioctl_donne_le_repli() {
    let dir = device();
    let fake = Fake::new(
        Err(PrepareError::Failed("démontage : erreur".into())),
        Ok(()),
    );
    let outcome = eject(dir.path(), &DeviceLock::new(), &fake).unwrap();
    assert!(matches!(outcome, EjectOutcome::Fallback { .. }));
    assert_eq!(fake.calls(), ["prepare"]);
}

#[test]
fn refus_de_windows_donne_le_repli() {
    let dir = device();
    let fake = Fake::new(Ok(()), Err("éjection refusée par Windows (motif 6)".into()));
    let outcome = eject(dir.path(), &DeviceLock::new(), &fake).unwrap();
    assert_eq!(
        outcome,
        EjectOutcome::fallback("éjection refusée par Windows (motif 6)")
    );
}

#[test]
fn sans_droit_sur_le_volume_la_demande_est_faite_quand_meme() {
    let dir = device();
    let fake = Fake::new(Err(PrepareError::NoAccess), Ok(()));
    let outcome = eject(dir.path(), &DeviceLock::new(), &fake).unwrap();
    assert_eq!(outcome, EjectOutcome::ejected());
    assert_eq!(fake.calls(), ["prepare", "eject"]);
}

#[test]
fn racine_sans_mdf_refusee() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Fake::new(Ok(()), Ok(()));
    let err = eject(dir.path(), &DeviceLock::new(), &fake).unwrap_err();
    assert!(matches!(err, EjectError::NotADevice));
    assert!(fake.calls().is_empty());
}

#[test]
fn resultat_serialise_pour_l_interface() {
    let json = serde_json::to_value(EjectOutcome::fallback("x")).unwrap();
    assert_eq!(json["status"], "fallback");
    assert_eq!(json["reason"], "x");
    assert_eq!(json["message"], FALLBACK_MESSAGE);
    let json = serde_json::to_value(EjectOutcome::ejected()).unwrap();
    assert_eq!(json["status"], "ejected");
    assert_eq!(json["message"], EJECTED_MESSAGE);
}

#[test]
fn lettre_de_lecteur() {
    assert_eq!(drive_letter(Path::new(r"E:\")), Some('E'));
    assert_eq!(drive_letter(Path::new("e:/")), Some('E'));
    assert_eq!(drive_letter(Path::new("F:")), Some('F'));
    assert_eq!(drive_letter(Path::new(r"\\?\G:\")), Some('G'));
    for bad in [
        r"E:\str",
        r"C:\Users",
        "E",
        "",
        r"\\serveur\partage",
        "1:",
        "/mnt/flam",
    ] {
        assert_eq!(drive_letter(Path::new(bad)), None, "{bad}");
    }
}

#[test]
fn liste_multi_sz() {
    let mut buf: Vec<u16> = "a\0bc\0\0".encode_utf16().collect();
    assert_eq!(split_multi_sz(&buf), ["a", "bc"]);
    buf = vec![0];
    assert!(split_multi_sz(&buf).is_empty());
    assert!(split_multi_sz(&[]).is_empty());
}

/// Racine qui n'est pas un lecteur : repli, aucun appel au volume.
#[cfg(windows)]
#[test]
fn systeme_reel_refuse_une_racine_qui_n_est_pas_un_lecteur() {
    let dir = device();
    let outcome = eject(dir.path(), &DeviceLock::new(), &CurrentSystem).unwrap();
    let EjectOutcome::Fallback { reason, .. } = outcome else {
        panic!("repli attendu");
    };
    assert!(
        reason.contains("n'est pas la racine d'un lecteur"),
        "{reason}"
    );
}

/// Éjection réelle d'une Flam branchée (test manuel, jamais lancé par la CI).
///
/// 1. Brancher la Flam en mode connexion USB, noter sa lettre (par exemple `E:\`).
/// 2. `set FLAM_EJECT_ROOT=E:\` puis
///    `cargo test --lib device::eject::tests::ejection_reelle -- --ignored --nocapture`.
/// 3. Attendu : `Ejected`, le lecteur disparaît de l'Explorateur. Recommencer avec un
///    fichier de la Flam ouvert dans un autre programme : `Fallback`, lecteur toujours là.
/// 4. Comportement pendant une mise à jour du firmware : à vérifier sur appareil (#236).
#[test]
#[ignore = "éjecte un vrai volume : FLAM_EJECT_ROOT requis, lancement manuel"]
fn ejection_reelle() {
    let root = std::env::var("FLAM_EJECT_ROOT")
        .expect("définir FLAM_EJECT_ROOT (racine de la Flam, par exemple E:\\)");
    let outcome = eject(Path::new(&root), &DeviceLock::new(), &CurrentSystem).unwrap();
    println!("{outcome:?}");
}
