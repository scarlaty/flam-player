//! Verrou des opérations sur l'appareil : une seule opération à la fois (retrait, transfert,
//! éjection…). Les écritures d'index ne sont pas sûres en parallèle (voir
//! [`super::index::write_index`]) et l'éjection doit être refusée pendant une écriture.
//!
//! Le verrou ne bloque jamais : une opération qui le trouve pris est refusée aussitôt avec le
//! nom de l'opération en cours. Il est injectable ([`DeviceLock::new`]) pour les tests ;
//! l'application utilise l'instance globale [`DEVICE_LOCK`].

use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// Durée de vie du scellement pour la fermeture, comptée depuis le moment où il prend effet
/// (libération de l'opération retenue). La fenêtre se ferme normalement en quelques secondes
/// après le scellement : interrogation du verrou toutes les 200 ms, puis gestionnaire de
/// fermeture de l'interface (vidage du brouillon borné à 1 s, verdict borné à 2 s). Passé ce
/// délai, la fenêtre ne s'est pas fermée (fermeture acceptée mais jamais aboutie, gestionnaire
/// bloqué, fil d'attente mort) : le scellement est levé à la demande suivante au lieu de refuser
/// toutes les opérations sur la Flam jusqu'au redémarrage (#652). Le délai ne court pas pendant
/// l'opération retenue elle-même, si longue soit-elle.
pub const SEAL_GRACE: Duration = Duration::from_secs(30);

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
    /// Lecture seule de la Flam ou travail sur le PC : aperçu des clés du mode carrier, « Retirer
    /// du PC » (#577). Tient le verrou (aucune écriture en parallèle) mais n'écrit rien sur
    /// l'appareil : ne retient pas la fermeture de la fenêtre.
    Read,
    /// Fermeture de l'application en cours (#577) : le verrou est scellé après l'opération
    /// retenue ([`DeviceLock::seal_after`]), aucune autre ne démarre avant la sortie.
    Closing,
}

impl Operation {
    /// Vrai si l'opération écrit sur l'appareil ou peut le faire (la fermer en plein milieu
    /// laisserait un transfert ou un retrait inachevé, voir [`super::close`]). L'éjection n'écrit
    /// rien : elle est brève et la couper ne perd rien.
    pub fn writes(self) -> bool {
        !matches!(
            self,
            Operation::Eject | Operation::Read | Operation::Closing
        )
    }

    /// Nom stable transmis à l'interface (événement et commande de [`super::close`]).
    pub fn key(self) -> &'static str {
        match self {
            Operation::Remove => "remove",
            Operation::Transfer => "transfer",
            Operation::Reorder => "reorder",
            Operation::Eject => "eject",
            Operation::Read => "read",
            Operation::Closing => "closing",
        }
    }

    /// Libellé affiché dans les messages de refus.
    pub fn label(self) -> &'static str {
        match self {
            Operation::Remove => "un retrait d'histoire",
            Operation::Transfer => "un transfert",
            Operation::Reorder => "un réordonnancement",
            Operation::Eject => "une éjection",
            Operation::Read => "une lecture",
            Operation::Closing => "la fermeture de l'application",
        }
    }
}

/// Refus : une autre opération est en cours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("la Flam est occupée ({}) : réessayez quand l'opération est terminée", .0.label())]
pub struct Busy(pub Operation);

/// État du verrou : opération en cours (avec son numéro), scellement pour la fermeture.
#[derive(Debug, Default)]
struct State {
    current: Option<(Operation, u64)>,
    /// Numéro de la prochaine opération.
    next_id: u64,
    /// À la libération de l'opération de ce numéro, le verrou est scellé.
    seal_on: Option<u64>,
    /// Aucune opération ne démarre plus ([`Operation::Closing`]) depuis cet instant, pendant
    /// [`SEAL_GRACE`] au plus.
    sealed: Option<Instant>,
}

/// Verrou non bloquant des opérations sur l'appareil.
#[derive(Debug, Default)]
pub struct DeviceLock {
    state: Mutex<State>,
}

/// Instance de l'application.
pub static DEVICE_LOCK: DeviceLock = DeviceLock::new();

impl DeviceLock {
    pub const fn new() -> Self {
        Self {
            state: Mutex::new(State {
                current: None,
                next_id: 0,
                seal_on: None,
                sealed: None,
            }),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        // Un fil qui panique garde son `OpGuard`, libéré au déroulement : l'état reste juste.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Prend le verrou pour `op`, ou refuse si une opération est déjà en cours ou si le verrou est
    /// scellé pour la fermeture (scellement de moins de [`SEAL_GRACE`]).
    pub fn try_begin(&self, op: Operation) -> Result<OpGuard<'_>, Busy> {
        self.try_begin_at(op, Instant::now())
    }

    /// [`Self::try_begin`] à l'instant `now` (injectable pour les tests). Un scellement plus vieux
    /// que [`SEAL_GRACE`] est levé : la fermeture qu'il protégeait n'a pas eu lieu.
    pub fn try_begin_at(&self, op: Operation, now: Instant) -> Result<OpGuard<'_>, Busy> {
        let mut state = self.state();
        if let Some((running, _)) = state.current {
            return Err(Busy(running));
        }
        if let Some(since) = state.sealed {
            if now.saturating_duration_since(since) < SEAL_GRACE {
                return Err(Busy(Operation::Closing));
            }
            tracing::warn!(
                "scellement de fermeture levé : la fenêtre ne s'est pas fermée après {} s",
                SEAL_GRACE.as_secs()
            );
            state.sealed = None;
        }
        let id = state.next_id;
        state.next_id += 1;
        state.current = Some((op, id));
        Ok(OpGuard { lock: self, id })
    }

    /// Opération en cours, s'il y en a une.
    pub fn current(&self) -> Option<Operation> {
        self.state().current.map(|(op, _)| op)
    }

    /// Opération en cours et son numéro (unique dans le processus).
    pub fn current_id(&self) -> Option<(Operation, u64)> {
        self.state().current
    }

    /// Fait sceller le verrou à la libération de l'opération `id` : aucune autre opération ne peut
    /// démarrer entre cette libération et la fermeture, qui a lieu à un moment déterministe.
    /// Scellé aussitôt si `id` n'est plus en cours. Le scellement dure [`SEAL_GRACE`] au plus à
    /// partir du moment où il prend effet.
    pub fn seal_after(&self, id: u64) {
        let mut state = self.state();
        if state.current.is_some_and(|(_, running)| running == id) {
            state.seal_on = Some(id);
        } else {
            state.sealed = Some(Instant::now());
        }
    }

    /// Lève le scellement (« Ne pas fermer », fermeture impossible, insistance de l'utilisateur).
    pub fn unseal(&self) {
        let mut state = self.state();
        state.seal_on = None;
        state.sealed = None;
    }

    /// Vrai si le verrou est scellé pour la fermeture (scellement non encore expiré : il n'est
    /// levé qu'à la demande suivante, voir [`Self::try_begin_at`]).
    pub fn is_sealed(&self) -> bool {
        self.state().sealed.is_some()
    }
}

/// Verrou pris ; libéré à la destruction (y compris sur erreur ou panique).
#[derive(Debug)]
pub struct OpGuard<'a> {
    lock: &'a DeviceLock,
    id: u64,
}

impl Drop for OpGuard<'_> {
    fn drop(&mut self) {
        let mut state = self.lock.state();
        state.current = None;
        if state.seal_on == Some(self.id) {
            state.seal_on = None;
            state.sealed = Some(Instant::now());
        }
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
    fn le_verrou_scelle_a_la_liberation_de_l_operation_retenue() {
        let lock = DeviceLock::new();
        let guard = lock.try_begin(Operation::Transfer).unwrap();
        let (_, id) = lock.current_id().unwrap();
        lock.seal_after(id);
        assert!(!lock.is_sealed(), "pas avant la libération");
        drop(guard);
        // Aucune autre opération (envoi suivant d'une file) ne démarre avant la fermeture.
        assert!(lock.is_sealed());
        assert_eq!(
            lock.try_begin(Operation::Transfer).unwrap_err(),
            Busy(Operation::Closing)
        );
        lock.unseal();
        assert!(lock.try_begin(Operation::Transfer).is_ok());
    }

    /// Fermeture acceptée mais jamais aboutie (#652) : le scellement tient pendant
    /// [`SEAL_GRACE`] (aucune opération ne se glisse entre la fin de l'écriture et la fermeture),
    /// puis est levé à la demande suivante.
    #[test]
    fn le_scellement_est_leve_si_la_fenetre_ne_se_ferme_pas() {
        let lock = DeviceLock::new();
        let guard = lock.try_begin(Operation::Transfer).unwrap();
        let (_, id) = lock.current_id().unwrap();
        lock.seal_after(id);
        drop(guard);
        let sealed_at = Instant::now();
        assert_eq!(
            lock.try_begin_at(
                Operation::Transfer,
                sealed_at + SEAL_GRACE - Duration::from_secs(1)
            )
            .unwrap_err(),
            Busy(Operation::Closing),
            "encore scellé juste avant le délai"
        );
        assert!(lock.is_sealed());
        let late = sealed_at + SEAL_GRACE + Duration::from_millis(1);
        let again = lock.try_begin_at(Operation::Transfer, late).unwrap();
        assert!(!lock.is_sealed());
        drop(again);
        assert!(lock.try_begin(Operation::Remove).is_ok());
    }

    #[test]
    fn seal_after_une_operation_deja_finie_scelle_aussitot() {
        let lock = DeviceLock::new();
        let guard = lock.try_begin(Operation::Remove).unwrap();
        let (_, id) = lock.current_id().unwrap();
        drop(guard);
        lock.seal_after(id);
        assert!(lock.is_sealed());
    }

    #[test]
    fn les_numeros_d_operation_different() {
        let lock = DeviceLock::new();
        let first = lock.try_begin(Operation::Remove).unwrap();
        let (_, a) = lock.current_id().unwrap();
        drop(first);
        let _second = lock.try_begin(Operation::Remove).unwrap();
        let (_, b) = lock.current_id().unwrap();
        assert_ne!(a, b);
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
