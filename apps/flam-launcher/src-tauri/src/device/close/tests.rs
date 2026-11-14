//! Tests de la retenue de la fermeture : décision, attente bornée, état interrogeable.

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
        // Deuxième demande pendant la même écriture : l'utilisateur insiste.
        assert_eq!(gate.decide(&lock), CloseDecision::Allow);
    }
}

#[test]
fn l_ejection_ne_retient_pas() {
    let gate = CloseGate::new();
    let lock = DeviceLock::new();
    let _guard = lock.try_begin(Operation::Eject).unwrap();
    assert_eq!(gate.decide(&lock), CloseDecision::Allow);
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
    let lock = DeviceLock::new();
    let guard = Cell::new(Some(lock.try_begin(Operation::Remove).unwrap()));
    let sleeps = Cell::new(0);
    let idle = wait_until_idle(&lock, GRACE, Duration::from_secs(1), |_| {
        sleeps.set(sleeps.get() + 1);
        if sleeps.get() == 3 {
            drop(guard.take());
        }
    });
    assert!(idle);
    assert_eq!(sleeps.get(), 3);
}

/// L'attente est bornée : une écriture qui ne finit pas ne bloque jamais la sortie.
#[test]
fn l_attente_est_bornee() {
    let lock = DeviceLock::new();
    let _guard = lock.try_begin(Operation::Transfer).unwrap();
    let sleeps = Cell::new(0u32);
    let idle = wait_until_idle(
        &lock,
        Duration::from_secs(10),
        Duration::from_secs(1),
        |_| sleeps.set(sleeps.get() + 1),
    );
    assert!(!idle);
    assert_eq!(sleeps.get(), 10);
}

#[test]
fn l_attente_ne_dort_pas_si_l_appareil_est_libre() {
    let lock = DeviceLock::new();
    assert!(wait_until_idle(&lock, GRACE, POLL, |_| panic!(
        "attente inutile"
    )));
}

#[test]
fn la_grace_est_bornee() {
    assert!(GRACE <= Duration::from_secs(300));
    assert!(GRACE >= Duration::from_secs(10));
}
