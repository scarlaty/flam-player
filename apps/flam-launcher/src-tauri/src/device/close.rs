//! Fermeture de la fenêtre pendant une écriture sur la Flam (#577).
//!
//! Fermer le launcher pendant un envoi, un retrait ou un changement d'ordre coupe l'opération en
//! plein milieu : au mieux une copie temporaire à nettoyer, au pire un retrait à reprendre (voir
//! les tableaux de reprise de `transfer`, `remove` et `reorder`). La demande de fermeture est donc
//! **retenue** tant qu'une écriture est en cours ([`DeviceLock`], [`Operation::writes`]) :
//!
//! 1. première demande pendant une écriture : fermeture empêchée, événement
//!    [`EVENT_CLOSE_RETAINED`] pour l'interface (message « fermeture dès que l'écriture est
//!    terminée, fermez à nouveau pour forcer »), et un fil attend la fin de l'écriture pour
//!    refermer la fenêtre de lui-même ;
//! 2. deuxième demande pendant la même attente : l'utilisateur insiste, la fermeture est
//!    acceptée tout de suite (la reprise de la Flam, au prochain branchement, termine ou défait
//!    ce qui était en cours) ;
//! 3. l'attente est bornée par [`GRACE`] : passé ce délai, la fenêtre se ferme quand même. La
//!    sortie n'est jamais bloquée indéfiniment.
//!
//! La logique ([`CloseGate`], [`wait_until_idle`]) ne dépend pas de Tauri ; seul
//! [`on_close_requested`] la relie à la fenêtre. L'état est interrogeable par la commande
//! [`device_write_in_progress`].

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;

use super::lock::{DeviceLock, Operation, DEVICE_LOCK};
use crate::AppResult;

/// Événement Tauri émis quand une fermeture est retenue.
pub const EVENT_CLOSE_RETAINED: &str = "device://close-retained";
/// Durée maximale de la retenue : au-delà, la fenêtre se ferme même si l'écriture continue.
pub const GRACE: Duration = Duration::from_secs(120);
/// Intervalle d'interrogation du verrou pendant la retenue.
const POLL: Duration = Duration::from_millis(200);

/// Décision pour une demande de fermeture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseDecision {
    /// Fermer.
    Allow,
    /// Ne pas fermer maintenant : une écriture de cette nature est en cours.
    Retain(Operation),
}

/// Mémoire d'une fermeture déjà retenue (une seule retenue par écriture).
#[derive(Debug, Default)]
pub struct CloseGate {
    retained: AtomicBool,
}

/// Instance de l'application.
pub static CLOSE_GATE: CloseGate = CloseGate::new();

impl CloseGate {
    pub const fn new() -> Self {
        Self {
            retained: AtomicBool::new(false),
        }
    }

    /// Rend sa première retenue à la prochaine écriture (fin d'attente réussie).
    pub fn reset(&self) {
        self.retained.store(false, Ordering::SeqCst);
    }

    /// Décide du sort d'une demande de fermeture. Sans écriture en cours : toujours `Allow`, et
    /// la mémoire est remise à zéro. Pendant une écriture : `Retain` la première fois, `Allow`
    /// ensuite (insistance de l'utilisateur, ou délai [`GRACE`] écoulé : le fil d'attente
    /// referme la fenêtre, qui repasse ici).
    pub fn decide(&self, lock: &DeviceLock) -> CloseDecision {
        match lock.current().filter(|op| op.writes()) {
            Some(op) => {
                if self.retained.swap(true, Ordering::SeqCst) {
                    CloseDecision::Allow
                } else {
                    CloseDecision::Retain(op)
                }
            }
            None => {
                self.retained.store(false, Ordering::SeqCst);
                CloseDecision::Allow
            }
        }
    }
}

/// Attend que `lock` n'ait plus d'écriture en cours, au plus `grace` (interrogation toutes les
/// `poll`, `sleep` injectable). Rend vrai si l'appareil est libre, faux si le délai est écoulé.
pub fn wait_until_idle(
    lock: &DeviceLock,
    grace: Duration,
    poll: Duration,
    mut sleep: impl FnMut(Duration),
) -> bool {
    let mut waited = Duration::ZERO;
    loop {
        if lock.current().is_none_or(|op| !op.writes()) {
            return true;
        }
        if waited >= grace {
            return false;
        }
        sleep(poll);
        waited += poll;
    }
}

/// Charge de [`EVENT_CLOSE_RETAINED`].
#[derive(Debug, Clone, Serialize)]
struct Retained {
    /// Écriture en cours (`transfer`, `remove` ou `reorder`).
    operation: &'static str,
    /// Délai maximal de la retenue, en secondes.
    grace_seconds: u64,
}

/// Relie la logique à la fenêtre : à appeler pour `WindowEvent::CloseRequested`.
pub fn on_close_requested(window: &tauri::Window, api: &tauri::CloseRequestApi) {
    use tauri::Emitter;
    let CloseDecision::Retain(operation) = CLOSE_GATE.decide(&DEVICE_LOCK) else {
        return;
    };
    api.prevent_close();
    tracing::info!(
        operation = operation.key(),
        "fermeture retenue : écriture en cours sur la Flam"
    );
    let _ = window.emit(
        EVENT_CLOSE_RETAINED,
        Retained {
            operation: operation.key(),
            grace_seconds: GRACE.as_secs(),
        },
    );
    let window = window.clone();
    std::thread::spawn(move || {
        if wait_until_idle(&DEVICE_LOCK, GRACE, POLL, std::thread::sleep) {
            // Appareil libre : une écriture démarrée avant la fermeture serait retenue à son tour.
            CLOSE_GATE.reset();
        } else {
            // Délai écoulé : la retenue reste épuisée, la fermeture passe (voir `decide`).
            tracing::warn!("fermeture après le délai de retenue, écriture toujours en cours");
        }
        let _ = window.close();
    });
}

/// Écriture en cours sur la Flam (`transfer`, `remove`, `reorder`), `None` sinon : permet à
/// l'interface de prévenir avant que l'utilisateur ne ferme la fenêtre.
#[tauri::command]
pub async fn device_write_in_progress() -> AppResult<Option<String>> {
    Ok(DEVICE_LOCK
        .current()
        .filter(|op| op.writes())
        .map(|op| op.key().to_string()))
}

#[cfg(test)]
mod tests;
