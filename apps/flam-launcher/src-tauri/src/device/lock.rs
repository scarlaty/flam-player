//! Verrou des opérations sur l'appareil : une seule opération à la fois (retrait, transfert,
//! éjection…). Les écritures d'index ne sont pas sûres en parallèle (voir
//! [`super::index::write_index`]) et l'éjection doit être refusée pendant une écriture.
//!
//! Le verrou ne bloque jamais : une opération qui le trouve pris est refusée aussitôt avec le
//! nom de l'opération en cours. Il est injectable ([`DeviceLock::new`]) pour les tests ;
//! l'application utilise l'instance globale [`DEVICE_LOCK`].

use std::sync::{Mutex, MutexGuard, PoisonError};

/// Opération en cours sur l'appareil.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    /// Retrait d'une histoire (#231).
    Remove,
    /// Transfert d'une histoire (#229).
    Transfer,
    /// Réécriture de l'index (#233).
    Reorder,
    /// Éjection du volume (#232).
    Eject,
}

impl Operation {
    /// Libellé affiché dans les messages de refus.
    pub fn label(self) -> &'static str {
        match self {
            Operation::Remove => "un retrait d'histoire",
            Operation::Transfer => "un transfert",
            Operation::Reorder => "un réordonnancement",
            Operation::Eject => "une éjection",
        }
    }
}

/// Refus : une autre opération est en cours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("la Flam est occupée ({}) : réessayez quand l'opération est terminée", .0.label())]
pub struct Busy(pub Operation);

/// Verrou non bloquant des opérations sur l'appareil.
#[derive(Debug, Default)]
pub struct DeviceLock {
    current: Mutex<Option<Operation>>,
}

/// Instance de l'application.
pub static DEVICE_LOCK: DeviceLock = DeviceLock::new();

impl DeviceLock {
    pub const fn new() -> Self {
        Self {
            current: Mutex::new(None),
        }
    }

    fn state(&self) -> MutexGuard<'_, Option<Operation>> {
        // Un fil qui panique garde son `OpGuard`, libéré au déroulement : l'état reste juste.
        self.current.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Prend le verrou pour `op`, ou refuse si une opération est déjà en cours.
    pub fn try_begin(&self, op: Operation) -> Result<OpGuard<'_>, Busy> {
        let mut state = self.state();
        if let Some(running) = *state {
            return Err(Busy(running));
        }
        *state = Some(op);
        Ok(OpGuard { lock: self })
    }

    /// Opération en cours, s'il y en a une.
    pub fn current(&self) -> Option<Operation> {
        *self.state()
    }
}

/// Verrou pris ; libéré à la destruction (y compris sur erreur ou panique).
#[derive(Debug)]
pub struct OpGuard<'a> {
    lock: &'a DeviceLock,
}

impl Drop for OpGuard<'_> {
    fn drop(&mut self) {
        *self.lock.state() = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn une_seule_operation_a_la_fois() {
        let lock = DeviceLock::new();
        let guard = lock.try_begin(Operation::Remove).unwrap();
        assert_eq!(lock.current(), Some(Operation::Remove));
        assert_eq!(
            lock.try_begin(Operation::Eject).unwrap_err(),
            Busy(Operation::Remove)
        );
        drop(guard);
        assert_eq!(lock.current(), None);
        assert!(lock.try_begin(Operation::Eject).is_ok());
    }

    #[test]
    fn le_verrou_est_libere_apres_une_panique() {
        let lock = DeviceLock::new();
        let result = std::panic::catch_unwind(|| {
            let _guard = lock.try_begin(Operation::Transfer).unwrap();
            panic!("échec simulé");
        });
        assert!(result.is_err());
        assert_eq!(lock.current(), None);
    }
}
