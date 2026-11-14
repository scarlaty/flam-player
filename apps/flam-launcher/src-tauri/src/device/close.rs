//! Fermeture de la fenêtre pendant une écriture sur la Flam (#577).
//!
//! Fermer le launcher pendant un envoi, un retrait ou un changement d'ordre coupe l'opération en
//! plein milieu : au mieux une copie temporaire à nettoyer, au pire un retrait à reprendre (voir
//! les tableaux de reprise de `transfer`, `remove` et `reorder`). La demande de fermeture est donc
//! **retenue** tant qu'une écriture est en cours ([`DeviceLock`], [`Operation::writes`]) :
//!
//! 1. première demande pendant une écriture : fermeture empêchée, événement
//!    [`EVENT_CLOSE_RETAINED`] pour l'interface (bandeau non modal), et un fil attend la fin de
//!    l'écriture pour refermer la fenêtre **de lui-même** : c'est ce que l'utilisateur a demandé
//!    en cliquant. Aucun délai : un envoi peut durer plus de vingt minutes, rien ne le coupe sans
//!    geste explicite ;
//! 2. deuxième demande pendant l'écriture : l'utilisateur choisit de couper, la fermeture est
//!    acceptée tout de suite (la reprise de la Flam, au prochain branchement, termine ou défait ce
//!    qui était en cours) ;
//! 3. « Ne pas fermer » ([`device_cancel_close`]) : la retenue est levée, le fil s'arrête, la
//!    fenêtre ne se fermera pas à la fin de l'écriture ; événement [`EVENT_CLOSE_CLEARED`].
//!
//! Aucun chemin n'empêche définitivement de fermer : la décision ne dépend que de l'état du
//! verrou et d'un drapeau « déjà retenue » ; si le fil d'attente meurt ou si le verrou n'est
//! jamais relâché, la deuxième demande passe toujours.
//!
//! # Un seul point de décision
//!
//! La décision est prise **ici** ([`CloseGate::request`]), jamais ailleurs. Quand l'interface a un
//! gestionnaire de fermeture (l'éditeur de pack vide son brouillon avant de fermer), Tauri
//! empêche lui-même la fermeture et laisse le JavaScript la détruire sauf s'il appelle
//! `preventDefault()` : le gestionnaire JavaScript appelle donc toujours `preventDefault()`, fait
//! son travail, puis demande le verdict mémorisé par cette demande ([`device_close_verdict`]) et ne
//! détruit la fenêtre que si c'est « allow ». Sans gestionnaire JavaScript, le verdict est appliqué
//! directement par [`on_close_requested`].
//!
//! Arrêt de Windows ou fermeture de session : **non vérifié** ; Windows termine de toute façon les
//! processus qui tardent à répondre, la retenue ne peut pas l'empêcher.
//!
//! La logique ([`CloseGate`], [`wait_until_idle`]) ne dépend pas de Tauri ; seul
//! [`on_close_requested`] la relie à la fenêtre.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use serde::Serialize;

use super::lock::{DeviceLock, Operation, DEVICE_LOCK};
use crate::AppResult;

/// Événement Tauri émis quand une fermeture est retenue (charge : `{ operation }`).
pub const EVENT_CLOSE_RETAINED: &str = "device://close-retained";
/// Événement Tauri émis quand la retenue prend fin sans que l'application se ferme : écriture
/// terminée, ou « Ne pas fermer ».
pub const EVENT_CLOSE_CLEARED: &str = "device://close-cleared";
/// Intervalle d'interrogation du verrou pendant la retenue.
const POLL: Duration = Duration::from_millis(200);
/// Attente maximale du verdict par [`device_close_verdict`] (le verdict est posé avant que
/// l'interface en entende parler ; cette borne ne sert qu'à une course improbable).
const VERDICT_WAIT: Duration = Duration::from_secs(2);

/// Décision pour une demande de fermeture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseDecision {
    /// Fermer.
    Allow,
    /// Ne pas fermer maintenant : une écriture de cette nature est en cours.
    Retain(Operation),
}

/// Fin de l'attente de [`wait_until_idle`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waited {
    /// Plus d'écriture en cours.
    Idle,
    /// Retenue levée entre-temps (« Ne pas fermer ») : ne rien fermer.
    Cancelled,
}

/// Mémoire d'une fermeture déjà retenue (une seule retenue par écriture) et dernier verdict.
#[derive(Debug, Default)]
pub struct CloseGate {
    retained: AtomicBool,
    /// Change à chaque levée de la retenue : un fil d'attente d'une ancienne génération s'arrête.
    epoch: AtomicU64,
    verdict: Mutex<Option<CloseDecision>>,
}

/// Instance de l'application.
pub static CLOSE_GATE: CloseGate = CloseGate::new();

impl CloseGate {
    pub const fn new() -> Self {
        Self {
            retained: AtomicBool::new(false),
            epoch: AtomicU64::new(0),
            verdict: Mutex::new(None),
        }
    }

    /// Décide du sort d'une demande de fermeture. Sans écriture en cours : toujours `Allow`, et
    /// la mémoire est remise à zéro. Pendant une écriture : `Retain` la première fois, `Allow`
    /// ensuite (insistance de l'utilisateur).
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

    /// [`Self::decide`], et mémorise le verdict pour [`Self::take_verdict`] (gestionnaire de
    /// fermeture de l'interface).
    pub fn request(&self, lock: &DeviceLock) -> CloseDecision {
        let decision = self.decide(lock);
        *self.verdict.lock().unwrap_or_else(PoisonError::into_inner) = Some(decision);
        decision
    }

    /// Verdict de la dernière demande, rendu une seule fois.
    pub fn take_verdict(&self) -> Option<CloseDecision> {
        self.verdict
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    /// Génération courante de la retenue.
    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }

    /// Lève la retenue (« Ne pas fermer ») : le fil d'attente s'arrête sans fermer, et une
    /// nouvelle demande est de nouveau retenue la première fois. Rend vrai si une retenue était
    /// en cours.
    pub fn cancel(&self) -> bool {
        self.epoch.fetch_add(1, Ordering::SeqCst);
        self.retained.swap(false, Ordering::SeqCst)
    }

    /// Rend sa première retenue à la prochaine écriture (fin d'attente réussie).
    pub fn reset(&self) {
        self.retained.store(false, Ordering::SeqCst);
    }
}

/// Attend que `lock` n'ait plus d'écriture en cours, **sans délai maximal**, tant que la retenue
/// de la génération `epoch` n'est pas levée ([`CloseGate::cancel`]). Interrogation toutes les
/// `poll` (`sleep` injectable).
pub fn wait_until_idle(
    lock: &DeviceLock,
    gate: &CloseGate,
    epoch: u64,
    poll: Duration,
    mut sleep: impl FnMut(Duration),
) -> Waited {
    loop {
        if gate.epoch() != epoch {
            return Waited::Cancelled;
        }
        if lock.current().is_none_or(|op| !op.writes()) {
            return Waited::Idle;
        }
        sleep(poll);
    }
}

/// Charge de [`EVENT_CLOSE_RETAINED`].
#[derive(Debug, Clone, Serialize)]
struct Retained {
    /// Écriture en cours (`transfer`, `remove` ou `reorder`).
    operation: &'static str,
}

/// Relie la logique à la fenêtre : à appeler pour `WindowEvent::CloseRequested`.
pub fn on_close_requested(window: &tauri::Window, api: &tauri::CloseRequestApi) {
    use tauri::Emitter;
    let CloseDecision::Retain(operation) = CLOSE_GATE.request(&DEVICE_LOCK) else {
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
        },
    );
    let epoch = CLOSE_GATE.epoch();
    let window = window.clone();
    std::thread::spawn(move || {
        match wait_until_idle(&DEVICE_LOCK, &CLOSE_GATE, epoch, POLL, std::thread::sleep) {
            Waited::Cancelled => {}
            Waited::Idle => {
                // Écriture terminée : la fermeture demandée a lieu. Une écriture démarrée avant
                // elle serait retenue à son tour.
                CLOSE_GATE.reset();
                let _ = window.emit(EVENT_CLOSE_CLEARED, ());
                if window.close().is_err() {
                    tracing::warn!("fermeture de la fenêtre impossible après l'écriture");
                }
            }
        }
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

/// Verdict de la dernière demande de fermeture (`"allow"` ou `"retain"`), pour le gestionnaire de
/// fermeture de l'interface (voir le module). Sans verdict après une courte attente, `"retain"` :
/// la seule erreur possible est un clic à refaire, jamais une écriture coupée.
#[tauri::command]
pub async fn device_close_verdict() -> AppResult<String> {
    crate::catalog::blocking(|| verdict_wait(&CLOSE_GATE, VERDICT_WAIT, std::thread::sleep)).await
}

/// Corps de [`device_close_verdict`] (`sleep` injectable).
fn verdict_wait(gate: &CloseGate, max: Duration, mut sleep: impl FnMut(Duration)) -> String {
    let step = Duration::from_millis(20);
    let mut waited = Duration::ZERO;
    loop {
        match gate.take_verdict() {
            Some(CloseDecision::Allow) => return "allow".into(),
            Some(CloseDecision::Retain(_)) => return "retain".into(),
            None if waited >= max => return "retain".into(),
            None => {
                sleep(step);
                waited += step;
            }
        }
    }
}

/// « Ne pas fermer » : lève la retenue (voir le module). Rend vrai si une retenue était en cours.
#[tauri::command]
pub async fn device_cancel_close(app: tauri::AppHandle) -> AppResult<bool> {
    use tauri::Emitter;
    let was = CLOSE_GATE.cancel();
    if was {
        let _ = app.emit(EVENT_CLOSE_CLEARED, ());
    }
    Ok(was)
}

#[cfg(test)]
mod tests;
