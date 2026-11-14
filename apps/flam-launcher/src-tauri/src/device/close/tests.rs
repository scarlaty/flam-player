//! Tests de la retenue de la fermeture : décision, double clic, attente sans délai mais bornée
//! en test, scellement du verrou, « Ne pas fermer », verdict pour l'interface.

use std::cell::Cell;

use super::*;
use crate::device::lock::Busy;

/// Attente simulée : panique au-delà de `max` appels, pour qu'une régression du contrôle d'époque
/// ou de verrou fasse échouer le test au lieu de le faire boucler.
fn bounded_sleep(count: &Cell<u32>, max: u32) -> impl Fn(Duration) + '_ {
    move |_| {
        count.set(count.get() + 1);
        assert!(
            count.get() <= max,
            "attente sans fin : {max} interrogations"
        );
    }
}

/// Instant `secs` secondes après `t0`.
fn at(t0: Instant, secs: u64) -> Instant {
    t0 + Duration::from_secs(secs)
}

fn id_of(lock: &DeviceLock) -> u64 {
    lock.current_id().unwrap().1
}

#[test]
fn sans_ecriture_la_fermeture_passe() {
    let gate = CloseGate::new();
    let lock = DeviceLock::new();
    assert_eq!(gate.decide(&lock, Instant::now()), CloseDecision::Allow);
}

#[test]
fn une_ecriture_retient_la_premiere_demande_puis_cede_a_l_insistance() {
    for op in [Operation::Transfer, Operation::Remove, Operation::Reorder] {
        let t0 = Instant::now();
        let gate = CloseGate::new();
        let lock = DeviceLock::new();
        let _guard = lock.try_begin(op).unwrap();
        assert_eq!(gate.decide(&lock, t0), CloseDecision::Retain(op));
        // Deuxième demande assez tard : l'utilisateur choisit de couper. Elle passe toujours,
        // sans fil d'attente ni autre condition.
        assert_eq!(gate.decide(&lock, at(t0, 3)), CloseDecision::Allow);
        assert_eq!(gate.decide(&lock, at(t0, 4)), CloseDecision::Allow);
    }
}

/// Un double clic (deuxième demande moins de `DEBOUNCE` après) ne coupe pas : retenue encore, sans
/// nouveau fil ; le verdict reste « retain » même avec l'éditeur de pack monté.
#[test]
fn un_double_clic_ne_coupe_pas() {
    let t0 = Instant::now();
    let gate = CloseGate::new();
    let lock = DeviceLock::new();
    let _guard = lock.try_begin(Operation::Transfer).unwrap();
    assert_eq!(
        gate.request(&lock, t0),
        CloseDecision::Retain(Operation::Transfer)
    );
    let held = gate.request(&lock, t0 + Duration::from_millis(200));
    assert_eq!(held, CloseDecision::Held(Operation::Transfer));
    assert_eq!(
        verdict_wait(&gate, VERDICT_WAIT, |_| panic!("verdict posé")),
        "retain"
    );
    // Juste avant la limite : encore retenue ; à la limite : insistance.
    assert_eq!(
        gate.decide(&lock, t0 + DEBOUNCE - Duration::from_millis(1)),
        CloseDecision::Held(Operation::Transfer)
    );
    assert_eq!(gate.decide(&lock, t0 + DEBOUNCE), CloseDecision::Allow);
}

/// L'éjection et les lectures (aperçu des clés, « Retirer du PC ») n'écrivent rien sur la Flam :
/// elles ne retiennent pas la fermeture.
#[test]
fn ejection_et_lectures_ne_retiennent_pas() {
    for op in [Operation::Eject, Operation::Read] {
        let gate = CloseGate::new();
        let lock = DeviceLock::new();
        let _guard = lock.try_begin(op).unwrap();
        assert_eq!(gate.decide(&lock, Instant::now()), CloseDecision::Allow);
    }
}

#[test]
fn la_retenue_est_rendue_apres_l_ecriture() {
    let t0 = Instant::now();
    let gate = CloseGate::new();
    let lock = DeviceLock::new();
    let guard = lock.try_begin(Operation::Remove).unwrap();
    assert_eq!(
        gate.decide(&lock, t0),
        CloseDecision::Retain(Operation::Remove)
    );
    drop(guard);
    lock.unseal();
    assert_eq!(gate.decide(&lock, at(t0, 1)), CloseDecision::Allow);
    assert_eq!(gate.retained(), None);
    // Une écriture suivante est retenue de nouveau.
    let _again = lock.try_begin(Operation::Transfer).unwrap();
    assert_eq!(
        gate.decide(&lock, at(t0, 2)),
        CloseDecision::Retain(Operation::Transfer)
    );
}

/// L'attente s'arrête dès que l'opération retenue est finie.
#[test]
fn l_attente_s_arrete_a_la_fin_de_l_ecriture() {
    let gate = CloseGate::new();
    let lock = DeviceLock::new();
    let guard = Cell::new(Some(lock.try_begin(Operation::Remove).unwrap()));
    let id = id_of(&lock);
    let sleeps = Cell::new(0);
    let counted = bounded_sleep(&sleeps, 10);
    let waited = wait_until_released(&lock, &gate, gate.epoch(), id, POLL, |d| {
        counted(d);
        if sleeps.get() == 3 {
            drop(guard.take());
        }
    });
    assert_eq!(waited, Waited::Released);
    assert_eq!(sleeps.get(), 3);
}

/// Aucun délai : une écriture très longue (ici l'équivalent de plusieurs heures d'interrogation)
/// n'est jamais suivie d'une fermeture automatique tant qu'elle dure.
#[test]
fn pas_de_fermeture_apres_un_delai() {
    let gate = CloseGate::new();
    let lock = DeviceLock::new();
    let guard = Cell::new(Some(lock.try_begin(Operation::Transfer).unwrap()));
    let id = id_of(&lock);
    let sleeps = Cell::new(0u32);
    let counted = bounded_sleep(&sleeps, 100_001);
    // 200 ms par interrogation : 100 000 interrogations = plus de cinq heures.
    let waited = wait_until_released(&lock, &gate, gate.epoch(), id, POLL, |d| {
        counted(d);
        if sleeps.get() == 100_000 {
            drop(guard.take());
        }
    });
    assert_eq!(waited, Waited::Released);
    assert_eq!(sleeps.get(), 100_000, "n'a pas abandonné avant la fin");
}

/// Déterminisme dans une file : l'opération suivante ne peut pas démarrer entre la fin de
/// l'opération retenue et la fermeture (verrou scellé), et l'attente rend la main même si elle
/// n'interroge qu'après.
#[test]
fn le_verrou_est_scelle_a_la_fin_de_l_operation_retenue() {
    let t0 = Instant::now();
    let gate = CloseGate::new();
    let lock = DeviceLock::new();
    let first = lock.try_begin(Operation::Transfer).unwrap();
    let id = id_of(&lock);
    assert_eq!(
        gate.decide(&lock, t0),
        CloseDecision::Retain(Operation::Transfer)
    );
    assert_eq!(gate.held_id(), Some(id));
    drop(first);
    // L'envoi suivant de la file est refusé : la fermeture a lieu à la fin de l'envoi en cours.
    assert_eq!(
        lock.try_begin(Operation::Transfer).unwrap_err(),
        Busy(Operation::Closing)
    );
    let waited = wait_until_released(&lock, &gate, gate.epoch(), id, POLL, |_| {
        panic!("attente inutile")
    });
    assert_eq!(waited, Waited::Released);
}

/// « Ne pas fermer » : l'attente s'arrête sans rien fermer, même si l'écriture continue, le
/// verrou est descellé et la demande suivante est de nouveau retenue la première fois.
#[test]
fn ne_pas_fermer_leve_la_retenue() {
    let t0 = Instant::now();
    let gate = CloseGate::new();
    let lock = DeviceLock::new();
    let guard = lock.try_begin(Operation::Transfer).unwrap();
    let id = id_of(&lock);
    assert_eq!(
        gate.decide(&lock, t0),
        CloseDecision::Retain(Operation::Transfer)
    );
    let epoch = gate.epoch();
    let sleeps = Cell::new(0);
    let counted = bounded_sleep(&sleeps, 10);
    let waited = wait_until_released(&lock, &gate, epoch, id, POLL, |d| {
        counted(d);
        if sleeps.get() == 2 {
            assert!(gate.cancel(&lock), "une retenue était en cours");
        }
    });
    assert_eq!(waited, Waited::Cancelled);
    assert_eq!(sleeps.get(), 2);
    assert!(!gate.cancel(&lock), "plus de retenue à lever");
    // Descellé : l'écriture terminée, une autre peut démarrer.
    drop(guard);
    assert!(!lock.is_sealed());
    assert!(lock.try_begin(Operation::Transfer).is_ok());
}

/// Un fil d'une retenue levée puis reprise ne ferme pas la fenêtre de la retenue suivante.
#[test]
fn un_ancien_fil_s_arrete_apres_une_nouvelle_retenue() {
    let gate = CloseGate::new();
    let lock = DeviceLock::new();
    let guard = lock.try_begin(Operation::Remove).unwrap();
    let id = id_of(&lock);
    let old = gate.epoch();
    gate.cancel(&lock);
    assert_eq!(
        wait_until_released(&lock, &gate, old, id, POLL, |_| panic!("attente inutile")),
        Waited::Cancelled
    );
    drop(guard);
}

#[test]
fn l_attente_ne_dort_pas_si_l_operation_est_deja_finie() {
    let gate = CloseGate::new();
    let lock = DeviceLock::new();
    let guard = lock.try_begin(Operation::Remove).unwrap();
    let id = id_of(&lock);
    drop(guard);
    assert_eq!(
        wait_until_released(&lock, &gate, gate.epoch(), id, POLL, |_| panic!(
            "attente inutile"
        )),
        Waited::Released
    );
}

/// L'état « retenue active » est interrogeable (bandeau si l'événement a été manqué).
#[test]
fn la_retenue_est_interrogeable() {
    let t0 = Instant::now();
    let gate = CloseGate::new();
    let lock = DeviceLock::new();
    assert_eq!(gate.retained(), None);
    let _guard = lock.try_begin(Operation::Reorder).unwrap();
    gate.decide(&lock, t0);
    assert_eq!(gate.retained(), Some(Operation::Reorder));
    gate.cancel(&lock);
    assert_eq!(gate.retained(), None);
}

/// Le verdict mémorisé par la demande est rendu une seule fois à l'interface ; sans verdict après
/// l'attente, « retain » (jamais une écriture coupée).
#[test]
fn verdict_pour_l_interface() {
    let t0 = Instant::now();
    let gate = CloseGate::new();
    let lock = DeviceLock::new();
    let sleeps = Cell::new(0u32);
    let sleep = |_| sleeps.set(sleeps.get() + 1);

    assert_eq!(gate.request(&lock, t0), CloseDecision::Allow);
    assert_eq!(verdict_wait(&gate, VERDICT_WAIT, sleep), "allow");
    assert_eq!(sleeps.get(), 0);
    // Déjà rendu : sans nouvelle demande, retenue après l'attente bornée.
    assert_eq!(verdict_wait(&gate, VERDICT_WAIT, sleep), "retain");
    assert!(sleeps.get() > 0);

    let _guard = lock.try_begin(Operation::Reorder).unwrap();
    assert_eq!(
        gate.request(&lock, at(t0, 1)),
        CloseDecision::Retain(Operation::Reorder)
    );
    assert_eq!(verdict_wait(&gate, VERDICT_WAIT, sleep), "retain");
    // L'insistance donne « allow » : le gestionnaire de l'interface détruit la fenêtre.
    assert_eq!(gate.request(&lock, at(t0, 5)), CloseDecision::Allow);
    assert_eq!(verdict_wait(&gate, VERDICT_WAIT, sleep), "allow");
}

#[test]
fn les_cles_d_operation_sont_stables() {
    assert_eq!(Operation::Transfer.key(), "transfer");
    assert_eq!(Operation::Remove.key(), "remove");
    assert_eq!(Operation::Reorder.key(), "reorder");
    assert!(Operation::Transfer.writes() && Operation::Remove.writes());
    assert!(!Operation::Read.writes() && !Operation::Eject.writes());
    assert!(!Operation::Closing.writes());
}

/// #652 : fermeture acceptée après l'écriture, mais la fenêtre reste ouverte. Après
/// `CLOSE_GRACE`, la retenue est levée : verrou descellé, retenue oubliée (une écriture suivante
/// est de nouveau retenue à la première demande), nouvelle génération.
#[test]
fn fermeture_acceptee_sans_suite_la_retenue_est_levee() {
    let t0 = Instant::now();
    let gate = CloseGate::new();
    let lock = DeviceLock::new();
    let guard = lock.try_begin(Operation::Transfer).unwrap();
    let id = id_of(&lock);
    assert_eq!(
        gate.decide(&lock, t0),
        CloseDecision::Retain(Operation::Transfer)
    );
    let epoch = gate.epoch();
    drop(guard);
    assert!(lock.is_sealed());
    // Demande de fermeture du fil : plus d'écriture, acceptée ; la fenêtre ne se ferme pas.
    assert_eq!(gate.decide(&lock, at(t0, 1)), CloseDecision::Allow);
    let slept = Cell::new(None);
    assert!(release_unfinished_close(
        &lock,
        &gate,
        epoch,
        id,
        CLOSE_GRACE,
        |d| slept.set(Some(d))
    ));
    assert_eq!(slept.get(), Some(CLOSE_GRACE));
    assert!(!lock.is_sealed());
    assert_ne!(gate.epoch(), epoch);
    assert_eq!(gate.retained(), None);
    let _again = lock.try_begin(Operation::Remove).unwrap();
    assert_eq!(
        gate.decide(&lock, at(t0, 100)),
        CloseDecision::Retain(Operation::Remove),
        "une écriture suivante est de nouveau retenue"
    );
}

/// #652 : retenue restée en mémoire (insistance de l'utilisateur, fermeture sans suite) : elle est
/// oubliée aussi, sinon une fermeture pendant une écriture suivante passerait tout de suite.
#[test]
fn retenue_oubliee_meme_si_la_fermeture_insistante_n_aboutit_pas() {
    let t0 = Instant::now();
    let gate = CloseGate::new();
    let lock = DeviceLock::new();
    let guard = lock.try_begin(Operation::Transfer).unwrap();
    let id = id_of(&lock);
    gate.decide(&lock, t0);
    let epoch = gate.epoch();
    assert_eq!(gate.decide(&lock, at(t0, 5)), CloseDecision::Allow);
    drop(guard);
    assert!(release_unfinished_close(
        &lock,
        &gate,
        epoch,
        id,
        CLOSE_GRACE,
        |_| {}
    ));
    assert_eq!(gate.retained(), None);
    let _next = lock.try_begin(Operation::Transfer).unwrap();
    assert_eq!(
        gate.decide(&lock, at(t0, 200)),
        CloseDecision::Retain(Operation::Transfer)
    );
}

/// #652 : « Ne pas fermer » pendant l'attente, ou nouvelle retenue d'une autre écriture : le fil
/// ne touche à rien.
#[test]
fn la_levee_tardive_respecte_ne_pas_fermer_et_une_nouvelle_retenue() {
    let t0 = Instant::now();
    let gate = CloseGate::new();
    let lock = DeviceLock::new();
    let guard = lock.try_begin(Operation::Transfer).unwrap();
    let id = id_of(&lock);
    gate.decide(&lock, t0);
    let epoch = gate.epoch();
    drop(guard);
    assert!(!release_unfinished_close(
        &lock,
        &gate,
        epoch,
        id,
        CLOSE_GRACE,
        |_| {
            gate.cancel(&lock);
        }
    ));

    // Nouvelle retenue d'une autre écriture (même génération) : laissée intacte.
    let gate = CloseGate::new();
    let lock = DeviceLock::new();
    let first = lock.try_begin(Operation::Transfer).unwrap();
    let old = id_of(&lock);
    gate.decide(&lock, t0);
    let epoch = gate.epoch();
    drop(first);
    assert_eq!(gate.decide(&lock, at(t0, 1)), CloseDecision::Allow);
    lock.unseal();
    let _second = lock.try_begin(Operation::Remove).unwrap();
    assert_eq!(
        gate.decide(&lock, at(t0, 2)),
        CloseDecision::Retain(Operation::Remove)
    );
    assert!(!release_unfinished_close(
        &lock,
        &gate,
        epoch,
        old,
        CLOSE_GRACE,
        |_| {}
    ));
    assert_eq!(gate.retained(), Some(Operation::Remove));
}

#[test]
fn le_fil_agit_avant_l_expiration_du_scellement() {
    assert!(CLOSE_GRACE < crate::device::lock::SEAL_GRACE);
}
