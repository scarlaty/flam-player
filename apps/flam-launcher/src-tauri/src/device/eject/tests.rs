//! Tests de l'éjection : logique sur faux appareil avec un système simulé ; éjection réelle
//! dans un test `#[ignore]`.

use std::cell::RefCell;

use super::*;

/// Système simulé : résultats programmés, appels enregistrés.
struct Fake {
    prepare: Result<(), PrepareError>,
    eject: Result<(), Refusal>,
    /// Réponse de `volume_gone`.
    gone: bool,
    calls: RefCell<Vec<&'static str>>,
}

impl Fake {
    fn new(prepare: Result<(), PrepareError>, eject: Result<(), Refusal>) -> Self {
        Self {
            prepare,
            eject,
            gone: true,
            calls: RefCell::new(Vec::new()),
        }
    }

    fn still_present(self) -> Self {
        Self {
            gone: false,
            ..self
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

    fn request_eject(&self, _root: &Path) -> Result<(), Refusal> {
        self.calls.borrow_mut().push("eject");
        self.eject.clone()
    }

    fn volume_gone(&self, _root: &Path) -> bool {
        self.calls.borrow_mut().push("gone");
        self.gone
    }
}

/// Verrouillage refusé après vidage des caches (cas mesuré sur la Flam, #619).
fn in_use() -> PrepareError {
    PrepareError::InUse {
        reason: "Accès refusé. (os error 5)".into(),
        flushed: true,
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
    // Chemin nominal inchangé : volume démonté, aucune attente de disparition.
    let fake = Fake::new(Ok(()), Ok(())).still_present();
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
        fn request_eject(&self, _root: &Path) -> Result<(), Refusal> {
            Ok(())
        }
        fn volume_gone(&self, _root: &Path) -> bool {
            unreachable!("volume démonté : aucune attente")
        }
    }
    let dir = device();
    let lock = DeviceLock::new();
    eject(dir.path(), &lock, &Probe(&lock)).unwrap();
    assert_eq!(lock.current(), None);
}

/// #619 : verrouillage refusé (antivirus, Explorateur…) mais Windows accepte la demande de
/// retrait et le volume disparaît : éjectée.
#[test]
fn volume_occupe_puis_retrait_accepte_par_windows() {
    let dir = device();
    let lock = DeviceLock::new();
    let fake = Fake::new(Err(in_use()), Ok(()));
    let outcome = eject(dir.path(), &lock, &fake).unwrap();
    assert_eq!(outcome, EjectOutcome::ejected());
    assert_eq!(fake.calls(), ["prepare", "eject", "gone"]);
    assert_eq!(lock.current(), None);
}

/// #619 : verrouillage refusé puis refus (veto) de Windows : repli avec le message actuel ; le
/// nom donné par Windows (programme, chemin) n'est pas journalisé.
#[test]
fn volume_occupe_puis_refus_de_windows_donne_le_repli() {
    let dir = device();
    let lock = DeviceLock::new();
    let secret = dir.path().join("programme-qui-refuse.exe");
    let who = secret.to_string_lossy().into_owned();
    let fake = Fake::new(
        Err(in_use()),
        Err(Refusal {
            reason: format!("éjection refusée par Windows : {} ({who})", veto_label(5)),
            veto: Some(5),
        }),
    );
    let mut outcome = None;
    let logs = crate::device::carrier::tests::capture_logs(|| {
        outcome = Some(eject(dir.path(), &lock, &fake).unwrap());
    });
    // Aucun chemin propre à ce test dans le journal (journal partagé entre tests : seule
    // l'absence est vérifiable, voir `testlog`).
    assert!(!logs.contains(&*dir.path().to_string_lossy()), "{logs}");
    let EjectOutcome::Fallback { reason, message } = outcome.unwrap() else {
        panic!("repli attendu");
    };
    assert!(
        reason.starts_with(
            "un fichier de la Flam est ouvert par un autre programme (Accès refusé. (os error 5)) ; "
        ),
        "{reason}"
    );
    // Raison de Windows gardée dans `reason` (non affichée), jamais journalisée.
    assert!(reason.ends_with(&format!("({who})")), "{reason}");
    assert!(!logs.contains(&who), "{logs}");
    assert!(!reason.contains(DISMOUNTED_NOTE), "rien n'a été démonté");
    assert_eq!(message, FALLBACK_MESSAGE);
    assert_eq!(fake.calls(), ["prepare", "eject"]);
    assert_eq!(lock.current(), None);
}

/// Volume occupé puis échec de la demande sans veto (périphérique introuvable) : la raison de
/// Windows est rendue, sans affirmer qu'un fichier est ouvert.
#[test]
fn volume_occupe_puis_echec_sans_veto_garde_la_raison_de_windows() {
    let dir = device();
    let fake = Fake::new(
        Err(in_use()),
        Err("périphérique USB introuvable (code 13)".into()),
    );
    let outcome = eject(dir.path(), &DeviceLock::new(), &fake).unwrap();
    let EjectOutcome::Fallback { reason, message } = outcome else {
        panic!("repli attendu");
    };
    assert_eq!(
        reason,
        "périphérique USB introuvable (code 13) (volume occupé : Accès refusé. (os error 5))"
    );
    assert!(
        !reason.contains("ouvert par un autre programme"),
        "{reason}"
    );
    assert_eq!(message, FALLBACK_MESSAGE);
}

/// Ouverture du volume refusée (caches non vidés) : la demande de retrait est faite quand même,
/// Windows vidant les caches en verrouillant le volume.
#[test]
fn volume_ouvert_en_exclusif_demande_aussi_le_retrait() {
    let dir = device();
    let fake = Fake::new(
        Err(PrepareError::InUse {
            reason: "violation de partage".into(),
            flushed: false,
        }),
        Ok(()),
    );
    let outcome = eject(dir.path(), &DeviceLock::new(), &fake).unwrap();
    assert_eq!(outcome, EjectOutcome::ejected());
    assert_eq!(fake.calls(), ["prepare", "eject", "gone"]);
}

/// Le verrou de la Flam est tenu pendant l'attente de la disparition du volume.
#[test]
fn le_verrou_est_tenu_pendant_l_attente_de_disparition() {
    struct Probe<'a>(&'a DeviceLock, std::cell::Cell<bool>);
    impl EjectSystem for Probe<'_> {
        fn prepare(&self, _root: &Path) -> Result<(), PrepareError> {
            Err(in_use())
        }
        fn request_eject(&self, _root: &Path) -> Result<(), Refusal> {
            Ok(())
        }
        fn volume_gone(&self, _root: &Path) -> bool {
            assert_eq!(
                self.0.try_begin(Operation::Transfer).unwrap_err(),
                Busy(Operation::Eject)
            );
            self.1.set(true);
            true
        }
    }
    let dir = device();
    let lock = DeviceLock::new();
    let probe = Probe(&lock, std::cell::Cell::new(false));
    assert_eq!(
        eject(dir.path(), &lock, &probe).unwrap(),
        EjectOutcome::ejected()
    );
    assert!(probe.1.get(), "attente de disparition faite");
    assert_eq!(lock.current(), None);
}

#[test]
fn seules_les_erreurs_d_absence_valent_disparition() {
    use std::io::{Error, ErrorKind};
    assert!(means_gone(&Error::from(ErrorKind::NotFound)));
    for kind in [
        ErrorKind::PermissionDenied,
        ErrorKind::Interrupted,
        ErrorKind::Other,
        ErrorKind::TimedOut,
    ] {
        assert!(!means_gone(&Error::from(kind)), "{kind:?}");
    }
    #[cfg(windows)]
    {
        for code in [2, 3, 15, 21, 1005, 1167] {
            assert!(means_gone(&Error::from_raw_os_error(code)), "{code}");
        }
        // Accès refusé, violation de partage, erreur d'E/S du périphérique, verrou.
        for code in [5, 32, 1117, 33] {
            assert!(!means_gone(&Error::from_raw_os_error(code)), "{code}");
        }
    }
}

#[test]
fn une_erreur_passagere_ne_vaut_pas_disparition() {
    use std::io::{Error, ErrorKind};
    let delay = std::time::Duration::from_millis(1);
    // Accès refusé à chaque essai : encore présent.
    let probe = || Err(Error::from(ErrorKind::PermissionDenied));
    assert!(!wait_until_gone_with(probe, 3, delay, |_| {}));
    // Accès refusé puis absence : disparu.
    let calls = std::cell::Cell::new(0);
    let probe = || {
        calls.set(calls.get() + 1);
        if calls.get() == 1 {
            Err(Error::from(ErrorKind::PermissionDenied))
        } else {
            Err(Error::from(ErrorKind::NotFound))
        }
    };
    assert!(wait_until_gone_with(probe, 3, delay, |_| {}));
    assert_eq!(calls.get(), 2);
}

/// Demande acceptée sans démontage préalable mais volume encore présent : pas de réussite
/// annoncée.
#[test]
fn retrait_accepte_mais_volume_present_n_annonce_pas_la_reussite() {
    for prepare in [Err(in_use()), Err(PrepareError::NoAccess)] {
        let dir = device();
        let fake = Fake::new(prepare, Ok(())).still_present();
        let outcome = eject(dir.path(), &DeviceLock::new(), &fake).unwrap();
        assert_eq!(outcome, EjectOutcome::fallback(STILL_PRESENT));
        assert_eq!(fake.calls(), ["prepare", "eject", "gone"]);
    }
}

#[test]
fn attente_de_la_disparition_du_volume() {
    let dir = device();
    let slept = std::cell::Cell::new(0);
    let sleep = |_| slept.set(slept.get() + 1);
    let delay = std::time::Duration::from_millis(1);
    // `.mdf` toujours là : faux après tous les essais.
    assert!(!wait_until_gone(dir.path(), 3, delay, sleep));
    assert_eq!(slept.get(), 2);
    // `.mdf` disparu au deuxième essai.
    slept.set(0);
    let gone_after_first = |_| {
        slept.set(slept.get() + 1);
        let _ = fs::remove_file(dir.path().join(MDF_FILE));
    };
    assert!(wait_until_gone(dir.path(), 3, delay, gone_after_first));
    assert_eq!(slept.get(), 1);
    // Racine absente : disparue aussitôt.
    assert!(wait_until_gone(
        &dir.path().join("absente"),
        3,
        delay,
        |_| unreachable!()
    ));
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
        EjectOutcome::fallback(format!(
            "éjection refusée par Windows (motif 6){DISMOUNTED_NOTE}"
        ))
    );
}

#[test]
fn sans_droit_sur_le_volume_la_demande_est_faite_quand_meme() {
    let dir = device();
    let fake = Fake::new(Err(PrepareError::NoAccess), Ok(()));
    let outcome = eject(dir.path(), &DeviceLock::new(), &fake).unwrap();
    assert_eq!(outcome, EjectOutcome::ejected());
    // Rien de démonté : la disparition du volume est vérifiée avant d'annoncer la réussite.
    assert_eq!(fake.calls(), ["prepare", "eject", "gone"]);
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
fn refus_apres_demontage_le_precise() {
    let dir = device();
    let outcome = eject(
        dir.path(),
        &DeviceLock::new(),
        &Fake::new(Ok(()), Err("refusé".into())),
    )
    .unwrap();
    let EjectOutcome::Fallback { reason, .. } = outcome else {
        panic!("repli attendu");
    };
    assert_eq!(reason, format!("refusé{DISMOUNTED_NOTE}"));
    // Étape de préparation sautée (pas de droit) : rien n'a été démonté.
    let outcome = eject(
        dir.path(),
        &DeviceLock::new(),
        &Fake::new(Err(PrepareError::NoAccess), Err("refusé".into())),
    )
    .unwrap();
    let EjectOutcome::Fallback { reason, .. } = outcome else {
        panic!("repli attendu");
    };
    assert_eq!(reason, "refusé");
}

#[test]
fn libelles_des_motifs_de_refus() {
    assert_eq!(veto_label(5), "un fichier de l'appareil est encore ouvert");
    assert_eq!(veto_label(0), "motif inconnu");
    assert_eq!(veto_label(99), "motif inconnu");
    assert_eq!(veto_label(-1), "motif inconnu");
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
fn chemin_sur_le_lecteur_de_la_flam() {
    let root = Path::new(r"E:\");
    for on in [r"E:\", r"e:\str\abc", "E:", r"\\?\E:\str", "e:/str"] {
        assert_eq!(on_volume(root, Path::new(on)), Some(true), "{on}");
    }
    for off in [
        r"C:\Temp",
        r"F:\str",
        "",
        r"\\serveur\E:",
        r"\\?\UNC\E:\x",
        "/mnt/e",
        "E",
    ] {
        assert_eq!(on_volume(root, Path::new(off)), Some(false), "{off}");
    }
    // Racine sans lettre de lecteur : indéterminable.
    assert_eq!(on_volume(Path::new(r"E:\str"), Path::new(r"E:\str")), None);
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
    // Raison journalisée : sans le chemin.
    assert!(!reason.contains(&*dir.path().to_string_lossy()), "{reason}");
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

/// Lecteur système : repli sans aucun appel au volume (pas de verrouillage ni de démontage).
#[cfg(windows)]
#[test]
fn systeme_reel_refuse_le_lecteur_systeme() {
    let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
    let root = format!("{drive}\\");
    let outcome = CurrentSystem.prepare(Path::new(&root)).unwrap_err();
    assert!(
        matches!(&outcome, PrepareError::Failed(reason) if reason.contains("lecteur système")),
        "{outcome:?}"
    );
    let refusal = CurrentSystem.request_eject(Path::new(&root)).unwrap_err();
    assert!(refusal.reason.contains("lecteur système"), "{refusal:?}");
    assert_eq!(refusal.veto, None);
}
