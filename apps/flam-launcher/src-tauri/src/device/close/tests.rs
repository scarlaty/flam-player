//! Tests de la retenue de la fermeture : décision, attente sans délai, « Ne pas fermer »,
//! verdict pour l'interface.

use std::cell::Cell;

use super::*;

#[test]
fn sans_ecriture_la_fermeture_passe() {
    let gate = CloseGate::new();
    let lock = DeviceLock::new();
    assert_eq!(gate.decide(&lock), CloseDecision::Allow);
}

#[test]
fn une_ecriture_retient_la_premiere_demande_puis_cede_a_l_insistance() {
    for op in [Operation::Transfer, Operation::Remove, Operation::Reorder] {
        let gate = CloseGate::new();
        let lock = DeviceLock::new();
        let _guard = lock.try_begin(op).unwrap();
        assert_eq!(gate.decide(&lock), CloseDecision::Retain(op));
        // Deuxième demande pendant la même écriture : l'utilisateur choisit de couper. Elle
        // passe toujours, sans fil d'attente ni autre condition.
        assert_eq!(gate.decide(&lock), CloseDecision::Allow);
        assert_eq!(gate.decide(&lock), CloseDecision::Allow);
    }
}

/// L'éjection et les lectures (aperçu des clés, « Retirer du PC ») n'écrivent rien sur la Flam :
/// elles ne retiennent pas la fermeture.
#[test]
fn ejection_et_lectures_ne_retiennent_pas() {
    for op in [Operation::Eject, Operation::Read] {
        let gate = CloseGate::new();
        let lock = DeviceLock::new();
        let _guard = lock.try_begin(op).unwrap();
        assert_eq!(gate.decide(&lock), CloseDecision::Allow);
    }
}

#[test]
fn la_retenue_est_rendue_apres_l_ecriture() {
    let gate = CloseGate::new();
    let lock = DeviceLock::new();
    let guard = lock.try_begin(Operation::Remove).unwrap();
    assert_eq!(gate.decide(&lock), CloseDecision::Retain(Operation::Remove));
    drop(guard);
    assert_eq!(gate.decide(&lock), CloseDecision::Allow);
    // Une écriture suivante est retenue de nouveau.
    let _again = lock.try_begin(Operation::Transfer).unwrap();
    assert_eq!(
        gate.decide(&lock),
        CloseDecision::Retain(Operation::Transfer)
    );
}

#[test]
fn reset_rend_la_premiere_retenue() {
    let gate = CloseGate::new();
    let lock = DeviceLock::new();
    let _guard = lock.try_begin(Operation::Transfer).unwrap();
    assert_eq!(
        gate.decide(&lock),
        CloseDecision::Retain(Operation::Transfer)
    );
    gate.reset();
    assert_eq!(
        gate.decide(&lock),
        CloseDecision::Retain(Operation::Transfer)
    );
}

/// L'attente s'arrête dès que l'écriture est finie.
#[test]
fn l_attente_s_arrete_a_la_fin_de_l_ecriture() {
    let gate = CloseGate::new();
    let lock = DeviceLock::new();
    let guard = Cell::new(Some(lock.try_begin(Operation::Remove).unwrap()));
    let sleeps = Cell::new(0);
    let waited = wait_until_idle(&lock, &gate, gate.epoch(), POLL, |_| {
        sleeps.set(sleeps.get() + 1);
        if sleeps.get() == 3 {
            drop(guard.take());
        }
    });
    assert_eq!(waited, Waited::Idle);
    assert_eq!(sleeps.get(), 3);
}

/// Aucun délai : une écriture très longue (ici l'équivalent de plusieurs heures d'interrogation)
/// n'est jamais suivie d'une fermeture automatique tant qu'elle dure.
#[test]
fn pas_de_fermeture_apres_un_delai() {
    let gate = CloseGate::new();
    let lock = DeviceLock::new();
    let guard = Cell::new(Some(lock.try_begin(Operation::Transfer).unwrap()));
    let sleeps = Cell::new(0u32);
    // 200 ms par interrogation : 100 000 interrogations = plus de cinq heures.
    let waited = wait_until_idle(&lock, &gate, gate.epoch(), POLL, |_| {
        sleeps.set(sleeps.get() + 1);
        if sleeps.get() == 100_000 {
            drop(guard.take());
        }
    });
    assert_eq!(waited, Waited::Idle);
    assert_eq!(sleeps.get(), 100_000, "n'a pas abandonné avant la fin");
}

/// « Ne pas fermer » : l'attente s'arrête sans rien fermer, même si l'écriture continue, et la
/// demande suivante est de nouveau retenue la première fois.
#[test]
fn ne_pas_fermer_leve_la_retenue() {
    let gate = CloseGate::new();
    let lock = DeviceLock::new();
    let _guard = lock.try_begin(Operation::Transfer).unwrap();
    assert_eq!(
        gate.decide(&lock),
        CloseDecision::Retain(Operation::Transfer)
    );
    let epoch = gate.epoch();
    let sleeps = Cell::new(0);
    let waited = wait_until_idle(&lock, &gate, epoch, POLL, |_| {
        sleeps.set(sleeps.get() + 1);
        if sleeps.get() == 2 {
            assert!(gate.cancel(), "une retenue était en cours");
        }
    });
    assert_eq!(waited, Waited::Cancelled);
    assert_eq!(sleeps.get(), 2);
    assert!(!gate.cancel(), "plus de retenue à lever");
    assert_eq!(
        gate.decide(&lock),
        CloseDecision::Retain(Operation::Transfer)
    );
}

/// Un fil d'une retenue levée puis reprise ne ferme pas la fenêtre de la retenue suivante.
#[test]
fn un_ancien_fil_s_arrete_apres_une_nouvelle_retenue() {
    let gate = CloseGate::new();
    let lock = DeviceLock::new();
    let guard = lock.try_begin(Operation::Remove).unwrap();
    let old = gate.epoch();
    gate.cancel();
    assert_eq!(
        wait_until_idle(&lock, &gate, old, POLL, |_| panic!("attente inutile")),
        Waited::Cancelled
    );
    drop(guard);
}

#[test]
fn l_attente_ne_dort_pas_si_l_appareil_est_libre() {
    let gate = CloseGate::new();
    let lock = DeviceLock::new();
    assert_eq!(
        wait_until_idle(&lock, &gate, gate.epoch(), POLL, |_| panic!(
            "attente inutile"
        )),
        Waited::Idle
    );
}

/// Le verdict mémorisé par la demande est rendu une seule fois à l'interface ; sans verdict après
/// l'attente, « retain » (jamais une écriture coupée).
#[test]
fn verdict_pour_l_interface() {
    let gate = CloseGate::new();
    let lock = DeviceLock::new();
    let sleeps = Cell::new(0u32);
    let sleep = |_| sleeps.set(sleeps.get() + 1);

    assert_eq!(gate.request(&lock), CloseDecision::Allow);
    assert_eq!(verdict_wait(&gate, VERDICT_WAIT, sleep), "allow");
    assert_eq!(sleeps.get(), 0);
    // Déjà rendu : sans nouvelle demande, retenue après l'attente bornée.
    assert_eq!(verdict_wait(&gate, VERDICT_WAIT, sleep), "retain");
    assert!(sleeps.get() > 0);

    let _guard = lock.try_begin(Operation::Reorder).unwrap();
    assert_eq!(
        gate.request(&lock),
        CloseDecision::Retain(Operation::Reorder)
    );
    assert_eq!(verdict_wait(&gate, VERDICT_WAIT, sleep), "retain");
    // L'insistance donne « allow » : le gestionnaire de l'interface détruit la fenêtre.
    assert_eq!(gate.request(&lock), CloseDecision::Allow);
    assert_eq!(verdict_wait(&gate, VERDICT_WAIT, sleep), "allow");
}

#[test]
fn les_cles_d_operation_sont_stables() {
    assert_eq!(Operation::Transfer.key(), "transfer");
    assert_eq!(Operation::Remove.key(), "remove");
    assert_eq!(Operation::Reorder.key(), "reorder");
    assert!(Operation::Transfer.writes() && Operation::Remove.writes());
    assert!(!Operation::Read.writes() && !Operation::Eject.writes());
}
