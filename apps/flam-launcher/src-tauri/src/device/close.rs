//! Fermeture de la fenêtre pendant une écriture sur la Flam (#577).
//!
//! Fermer le launcher pendant un envoi, un retrait ou un changement d'ordre coupe l'opération en
//! plein milieu : au mieux une copie temporaire à nettoyer, au pire un retrait à reprendre (voir
//! les tableaux de reprise de `transfer`, `remove` et `reorder`). La demande de fermeture est donc
//! **retenue** tant qu'une écriture est en cours ([`DeviceLock`], [`Operation::writes`]) :
//!
//! 1. première demande pendant une écriture : fermeture empêchée, événement
//!    [`EVENT_CLOSE_RETAINED`] pour l'interface (bandeau non modal), et un fil attend la fin de
//!    **cette** écriture pour refermer la fenêtre de lui-même : c'est ce que l'utilisateur a
//!    demandé en cliquant. Aucun délai : un envoi peut durer plus de vingt minutes, rien ne le
//!    coupe sans geste explicite. Le moment est déterministe : le verrou de l'appareil est
//!    **scellé** à la libération de l'opération retenue ([`DeviceLock::seal_after`]), donc dans
//!    une file de plusieurs envois, l'envoi suivant ne démarre pas (refus « la Flam est occupée
//!    (la fermeture de l'application) ») et les envois en attente ne partent pas ;
//! 2. deuxième demande pendant l'écriture, au moins [`DEBOUNCE`] après la première : l'utilisateur
//!    choisit de couper, la fermeture est acceptée tout de suite (la reprise de la Flam, au
//!    prochain branchement, termine ou défait ce qui était en cours). Une demande arrivée moins de
//!    [`DEBOUNCE`] après la première (double clic) est retenue de la même façon, sans nouveau fil
//!    ni nouvel événement ;
//! 3. « Ne pas fermer » ([`device_cancel_close`]) : la retenue est levée, le verrou descellé, le
//!    fil s'arrête, la fenêtre ne se fermera pas à la fin de l'écriture ; événement
//!    [`EVENT_CLOSE_CLEARED`].
//!
//! Aucun chemin n'empêche définitivement de fermer : la décision ne dépend que de l'état du
//! verrou et d'un drapeau « déjà retenue » ; si le fil d'attente meurt ou si le verrou n'est
//! jamais relâché, la deuxième demande passe toujours.
//!
//! Inversement, aucun chemin ne laisse la Flam inutilisable si la fenêtre reste ouverte après une
//! fermeture acceptée (gestionnaire de l'interface bloqué ou en échec, `destroy()` refusé) : le
//! fil d'attente, après avoir demandé la fermeture, attend [`CLOSE_GRACE`] (bien au-delà d'une
//! fermeture normale, quelques secondes) ; si le processus vit encore, il lève la retenue comme
//! « Ne pas fermer » (verrou descellé, retenue oubliée, [`EVENT_CLOSE_CLEARED`] pour retirer le
//! bandeau), sauf si l'utilisateur a entre-temps levé ou relancé une retenue (#652). Si ce fil a
//! disparu, le scellement expire de lui-même [`super::lock::SEAL_GRACE`] après avoir pris effet.
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

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde::Serialize;

use super::lock::{DeviceLock, Operation, DEVICE_LOCK};
use crate::AppResult;

/// Événement Tauri émis quand une fermeture est retenue (charge : `{ operation }`).
pub const EVENT_CLOSE_RETAINED: &str = "device://close-retained";
/// Événement Tauri émis quand la retenue prend fin sans que l'application se ferme : écriture
/// terminée, ou « Ne pas fermer ».
pub const EVENT_CLOSE_CLEARED: &str = "device://close-cleared";
/// Une deuxième demande arrivée moins de ce délai après la retenue (double clic) ne coupe pas.
pub const DEBOUNCE: Duration = Duration::from_secs(2);
/// Intervalle d'interrogation du verrou pendant la retenue.
const POLL: Duration = Duration::from_millis(200);
/// Attente, après la demande de fermeture du fil d'attente, au-delà de laquelle une fenêtre
/// encore ouverte est tenue pour une fermeture qui n'aboutira pas (#652) : la retenue est levée.
/// Une fermeture normale prend quelques secondes (gestionnaire de l'interface : vidage du
/// brouillon borné à 1 s, verdict borné à 2 s). Plus court que [`super::lock::SEAL_GRACE`].
pub const CLOSE_GRACE: Duration = Duration::from_secs(30);
/// Attente maximale du verdict par [`device_close_verdict`] (le verdict est posé avant que
/// l'interface en entende parler ; cette borne ne sert qu'à une course improbable).
const VERDICT_WAIT: Duration = Duration::from_secs(2);

/// Décision pour une demande de fermeture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseDecision {
    /// Fermer.
    Allow,
    /// Première retenue : ne pas fermer, annoncer, attendre la fin de l'écriture.
    Retain(Operation),
    /// Retenue déjà en cours, demande arrivée moins de [`DEBOUNCE`] après : ne pas fermer, rien
    /// d'autre.
    Held(Operation),
}

/// Fin de l'attente de [`wait_until_released`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waited {
    /// L'opération retenue est terminée.
    Released,
    /// Retenue levée entre-temps (« Ne pas fermer ») : ne rien fermer.
    Cancelled,
}

/// Retenue en cours : quand, quelle opération, quel numéro d'opération.
#[derive(Debug, Clone, Copy)]
struct Held {
    at: Instant,
    operation: Operation,
    id: u64,
}

/// Mémoire d'une fermeture déjà retenue (une seule retenue par écriture) et dernier verdict.
#[derive(Debug, Default)]
pub struct CloseGate {
    held: Mutex<Option<Held>>,
    /// Change à chaque levée de la retenue : un fil d'attente d'une ancienne génération s'arrête.
    epoch: AtomicU64,
    verdict: Mutex<Option<CloseDecision>>,
}

/// Instance de l'application.
pub static CLOSE_GATE: CloseGate = CloseGate::new();

impl CloseGate {
    pub const fn new() -> Self {
        Self {
            held: Mutex::new(None),
            epoch: AtomicU64::new(0),
            verdict: Mutex::new(None),
        }
    }

    fn held(&self) -> std::sync::MutexGuard<'_, Option<Held>> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Décide du sort d'une demande de fermeture faite à l'instant `now`. Sans écriture en cours :
    /// toujours `Allow`, et la mémoire est remise à zéro. Pendant une écriture : `Retain` la
    /// première fois (le verrou est scellé à la libération de cette opération), `Held` pour une
    /// demande de moins de [`DEBOUNCE`] après, `Allow` ensuite (insistance de l'utilisateur, le
    /// verrou est descellé).
    pub fn decide(&self, lock: &DeviceLock, now: Instant) -> CloseDecision {
        let mut held = self.held();
        let Some((operation, id)) = lock.current_id().filter(|(op, _)| op.writes()) else {
            *held = None;
            return CloseDecision::Allow;
        };
        match *held {
            None => {
                *held = Some(Held {
                    at: now,
                    operation,
                    id,
                });
                lock.seal_after(id);
                CloseDecision::Retain(operation)
            }
            Some(first) if now.saturating_duration_since(first.at) < DEBOUNCE => {
                CloseDecision::Held(first.operation)
            }
            Some(_) => {
                lock.unseal();
                CloseDecision::Allow
            }
        }
    }

    /// [`Self::decide`], et mémorise le verdict pour [`Self::take_verdict`] (gestionnaire de
    /// fermeture de l'interface).
    pub fn request(&self, lock: &DeviceLock, now: Instant) -> CloseDecision {
        let decision = self.decide(lock, now);
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

    /// Numéro de l'opération retenue, s'il y en a une.
    pub fn held_id(&self) -> Option<u64> {
        self.held().map(|held| held.id)
    }

    /// Opération dont la fermeture est retenue, s'il y en a une (l'interface l'interroge au
    /// montage pour afficher le bandeau si l'événement a été manqué).
    pub fn retained(&self) -> Option<Operation> {
        self.held().map(|held| held.operation)
    }

    /// Lève la retenue de la génération `epoch` si la fermeture demandée pour l'opération `id`
    /// n'a pas abouti (#652) : rien si la génération a changé (« Ne pas fermer » déjà fait) ou si
    /// une autre retenue est en cours (nouvelle écriture retenue). Rend vrai si elle a été levée.
    pub fn cancel_unfinished_close(&self, lock: &DeviceLock, epoch: u64, id: u64) -> bool {
        let mut held = self.held();
        if self.epoch() != epoch || held.is_some_and(|h| h.id != id) {
            return false;
        }
        self.epoch.fetch_add(1, Ordering::SeqCst);
        lock.unseal();
        *held = None;
        true
    }

    /// Lève la retenue (« Ne pas fermer ») : le fil d'attente s'arrête sans fermer, le verrou est
    /// descellé, et une nouvelle demande est de nouveau retenue la première fois. Rend vrai si une
    /// retenue était en cours.
    pub fn cancel(&self, lock: &DeviceLock) -> bool {
        let mut held = self.held();
        self.epoch.fetch_add(1, Ordering::SeqCst);
        lock.unseal();
        held.take().is_some()
    }
}

/// Attend que l'opération `id` ne soit plus en cours sur `lock`, **sans délai maximal**, tant que
/// la retenue de la génération `epoch` n'est pas levée ([`CloseGate::cancel`]). Interrogation
/// toutes les `poll` (`sleep` injectable) ; le verrou étant scellé à la libération, la lenteur de
/// l'interrogation ne laisse démarrer aucune autre opération.
pub fn wait_until_released(
    lock: &DeviceLock,
    gate: &CloseGate,
    epoch: u64,
    id: u64,
    poll: Duration,
    mut sleep: impl FnMut(Duration),
) -> Waited {
    loop {
        if gate.epoch() != epoch {
            return Waited::Cancelled;
        }
        if lock.current_id().is_none_or(|(_, running)| running != id) {
            return Waited::Released;
        }
        sleep(poll);
    }
}

/// Après la demande de fermeture du fil d'attente : attend `grace` (`sleep` injectable) puis,
/// le processus vivant encore, lève la retenue si elle n'a pas changé
/// ([`CloseGate::cancel_unfinished_close`]). Rend vrai si elle a été levée (l'appelant prévient
/// l'interface).
pub fn release_unfinished_close(
    lock: &DeviceLock,
    gate: &CloseGate,
    epoch: u64,
    id: u64,
    grace: Duration,
    sleep: impl FnOnce(Duration),
) -> bool {
    sleep(grace);
    gate.cancel_unfinished_close(lock, epoch, id)
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
    let operation = match CLOSE_GATE.request(&DEVICE_LOCK, Instant::now()) {
        CloseDecision::Allow => return,
        CloseDecision::Held(_) => {
            api.prevent_close();
            return;
        }
        CloseDecision::Retain(operation) => operation,
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
    let (epoch, Some(id)) = (CLOSE_GATE.epoch(), CLOSE_GATE.held_id()) else {
        return;
    };
    let window = window.clone();
    std::thread::spawn(move || {
        let waited = wait_until_released(
            &DEVICE_LOCK,
            &CLOSE_GATE,
            epoch,
            id,
            POLL,
            std::thread::sleep,
        );
        if waited == Waited::Cancelled {
            return;
        }
        // Course avec « Ne pas fermer » : la retenue est revérifiée juste avant de fermer.
        if CLOSE_GATE.epoch() != epoch {
            return;
        }
        if window.close().is_err() {
            tracing::warn!("fermeture de la fenêtre impossible après l'écriture");
            CLOSE_GATE.cancel(&DEVICE_LOCK);
            let _ = window.emit(EVENT_CLOSE_CLEARED, ());
            return;
        }
        // Fermeture demandée : si le processus vit encore après `CLOSE_GRACE`, elle n'aboutira
        // pas (#652) ; la retenue est levée pour rendre la Flam utilisable.
        if release_unfinished_close(
            &DEVICE_LOCK,
            &CLOSE_GATE,
            epoch,
            id,
            CLOSE_GRACE,
            std::thread::sleep,
        ) {
            tracing::warn!("fenêtre toujours ouverte après la fermeture demandée : retenue levée");
            let _ = window.emit(EVENT_CLOSE_CLEARED, ());
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
            Some(CloseDecision::Retain(_) | CloseDecision::Held(_)) => return "retain".into(),
            None if waited >= max => return "retain".into(),
            None => {
                sleep(step);
                waited += step;
            }
        }
    }
}

/// Opération dont la fermeture est retenue (`transfer`, `remove`, `reorder`), `None` sinon :
/// l'interface l'interroge au montage pour afficher le bandeau si l'événement a été manqué
/// (écran de lecture, rechargement).
#[tauri::command]
pub async fn device_close_retained() -> AppResult<Option<String>> {
    Ok(CLOSE_GATE.retained().map(|op| op.key().to_string()))
}

/// « Ne pas fermer » : lève la retenue (voir le module). Rend vrai si une retenue était en cours.
#[tauri::command]
pub async fn device_cancel_close(app: tauri::AppHandle) -> AppResult<bool> {
    use tauri::Emitter;
    let was = CLOSE_GATE.cancel(&DEVICE_LOCK);
    if was {
        let _ = app.emit(EVENT_CLOSE_CLEARED, ());
    }
    Ok(was)
}

#[cfg(test)]
mod tests;
