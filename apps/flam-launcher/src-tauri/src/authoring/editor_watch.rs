//! Surveillance du dossier ouvert dans une fenêtre de l'éditeur (lot 9,
//! `docs/creation-dossier.md` § 4.5).
//!
//! - Tant que la fenêtre `editor-<n>` est ouverte, son dossier est surveillé **récursivement**
//!   (même mécanique d'anti-rebond que `watch.rs` : un seul signal par rafale, après
//!   [`DEBOUNCE`] de calme, au plus [`MAX_WAIT`] après le premier changement). Les liens ne sont
//!   pas suivis (`with_follow_symlinks(false)` ; sous Windows, `ReadDirectoryChangesW` ne
//!   traverse pas les jonctions).
//! - Ne déclenchent rien : les accès en lecture, et tout chemin dont un composant (relatif au
//!   dossier) commence par un point — `.flam/` (corbeille, fichiers de travail) et les
//!   temporaires des écritures atomiques du launcher (`.<nom>.<pid>-<n>.tmp`,
//!   `.<nom>.<pid>-<n>.flam-tmp`), que la lecture du dossier ignore aussi (`tree.rs`), ni la
//!   simple mise à jour de la date d'un dossier (que Windows signale à chaque ajout dedans).
//! - Les changements faits **par l'éditeur lui-même** sont ignorés : chaque opération tient un
//!   [`QuietGuard`] ([`EditorWatch::quiet`]) ; les événements reçus pendant l'opération et
//!   jusqu'à [`GRACE`] après sa fin sont écartés (le système les livre avec un léger retard).
//!   L'opération rend de toute façon la vue relue. Limite connue : un changement fait ailleurs
//!   dans cette fenêtre de temps n'est vu qu'au changement suivant ou à la prochaine opération.
//! - Dossier supprimé, renommé, ou clé USB retirée : la présence du dossier est revérifiée après
//!   chaque rafale, sur erreur de l'observateur et toutes les [`PRESENCE_POLL`] (Windows retire
//!   la surveillance d'un dossier supprimé sans rien dire). Signal [`Signal::Missing`] une seule
//!   fois ; l'observateur est libéré. Si le dossier revient (clé rebranchée), la surveillance
//!   reprend et [`Signal::Changed`] est émis.
//! - Arrêt : abandonner l'[`EditorWatch`] (fenêtre fermée, `EditorState::remove`) arrête le
//!   thread et libère l'observateur ; aucun signal n'est émis après la fin de `drop`.
//!
//! Côté application, les signaux deviennent les événements [`EVENT_CHANGED`] et
//! [`EVENT_MISSING`], envoyés à la seule fenêtre concernée ; ils ne portent aucune donnée : la
//! page relit la vue (`authoring_editor_view`).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use notify::event::ModifyKind;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tauri::{AppHandle, Emitter, EventTarget};

pub use crate::watch::{DEBOUNCE, MAX_WAIT};

/// Événement : le dossier a changé ailleurs (miroir de `FOLDER_CHANGED_EVENT`, `api/editor.ts`).
pub const EVENT_CHANGED: &str = "authoring:folder-changed";
/// Événement : le dossier a disparu (miroir de `FOLDER_MISSING_EVENT`, `api/editor.ts`).
pub const EVENT_MISSING: &str = "authoring:folder-missing";
/// Délai après une opération de l'éditeur pendant lequel les événements sont encore les siens.
pub const GRACE: Duration = Duration::from_millis(500);
/// Intervalle de vérification de la présence du dossier.
pub const PRESENCE_POLL: Duration = Duration::from_millis(1500);

/// Ce que la surveillance signale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// Le dossier a changé (ou est revenu après avoir disparu).
    Changed,
    /// Le dossier a disparu (supprimé, renommé, déplacé, clé retirée).
    Missing,
}

/// Délais de la surveillance (réduits dans les tests).
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    pub debounce: Duration,
    pub max_wait: Duration,
    pub grace: Duration,
    pub poll: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            debounce: DEBOUNCE,
            max_wait: MAX_WAIT,
            grace: GRACE,
            poll: PRESENCE_POLL,
        }
    }
}

// ---------------------------------------------------------------------------
// Silence pendant les opérations de l'éditeur
// ---------------------------------------------------------------------------

/// Opérations de l'éditeur en cours et fin du délai de grâce de la dernière.
#[derive(Debug)]
struct Quiet {
    busy: AtomicUsize,
    until: Mutex<Option<Instant>>,
    grace: Duration,
}

impl Quiet {
    fn new(grace: Duration) -> Self {
        Self {
            busy: AtomicUsize::new(0),
            until: Mutex::new(None),
            grace,
        }
    }

    fn until(&self) -> std::sync::MutexGuard<'_, Option<Instant>> {
        self.until.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Un événement reçu maintenant vient-il (peut-être) de l'éditeur ?
    fn is_quiet(&self) -> bool {
        self.busy.load(Ordering::SeqCst) > 0 || self.until().is_some_and(|u| Instant::now() < u)
    }
}

/// Tenu pendant une opération de l'éditeur : ses changements ne sont pas signalés.
#[derive(Debug)]
pub struct QuietGuard(Arc<Quiet>);

impl QuietGuard {
    fn new(quiet: Arc<Quiet>) -> Self {
        quiet.busy.fetch_add(1, Ordering::SeqCst);
        Self(quiet)
    }
}

impl Drop for QuietGuard {
    fn drop(&mut self) {
        // Délai de grâce posé avant de libérer le compteur : aucun instant sans silence.
        {
            let mut until = self.0.until();
            let end = Instant::now() + self.0.grace;
            if until.is_none_or(|u| u < end) {
                *until = Some(end);
            }
        }
        self.0.busy.fetch_sub(1, Ordering::SeqCst);
    }
}

// ---------------------------------------------------------------------------
// Filtre
// ---------------------------------------------------------------------------

/// Le chemin `path` (sous `base`) compte-t-il pour la vue ? Un composant relatif commençant par
/// un point (`.flam/`, temporaires) l'écarte ; un chemin hors de `base` (ou `base` lui-même)
/// compte.
fn path_counts(path: &Path, base: &Path) -> bool {
    match path.strip_prefix(base) {
        Ok(rel) => !rel
            .components()
            .any(|c| c.as_os_str().to_string_lossy().starts_with('.')),
        Err(_) => true,
    }
}

/// Un événement du système change-t-il ce que montre l'éditeur ?
fn is_relevant(event: &Event, base: &Path) -> bool {
    if matches!(event.kind, EventKind::Access(_)) {
        return false;
    }
    // Sans chemin (débordement, « rescan ») : par prudence, on relit.
    event.paths.is_empty() || event.paths.iter().any(|p| path_counts(p, base))
}

/// Simple mise à jour de la date d'un dossier (Windows en signale une à chaque ajout ou retrait
/// dans le dossier, même d'un temporaire ignoré) : le contenu changé a son propre événement.
fn is_dir_touch(event: &Event) -> bool {
    matches!(
        event.kind,
        EventKind::Modify(ModifyKind::Any | ModifyKind::Data(_) | ModifyKind::Metadata(_))
    ) && !event.paths.is_empty()
        && event
            .paths
            .iter()
            .all(|p| std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir()))
}

/// Le dossier est-il encore là (un dossier, pas un lien) ?
fn present(base: &Path) -> bool {
    std::fs::symlink_metadata(base).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
}

// ---------------------------------------------------------------------------
// Observateur
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum Msg {
    /// Changement à signaler.
    Fs,
    /// Erreur de l'observateur : présence à revérifier.
    Error,
    /// Arrêt demandé.
    Stop,
}

fn start_notify(
    base: &Path,
    tx: mpsc::Sender<Msg>,
    quiet: Arc<Quiet>,
) -> notify::Result<RecommendedWatcher> {
    let root = base.to_path_buf();
    let mut watcher = RecommendedWatcher::new(
        move |res: notify::Result<Event>| match res {
            Ok(event)
                if is_relevant(&event, &root) && !quiet.is_quiet() && !is_dir_touch(&event) =>
            {
                let _ = tx.send(Msg::Fs);
            }
            Ok(_) => {}
            Err(err) => {
                tracing::warn!("surveillance du dossier de l'éditeur : {err}");
                let _ = tx.send(Msg::Error);
            }
        },
        Config::default().with_follow_symlinks(false),
    )?;
    watcher.watch(base, RecursiveMode::Recursive)?;
    Ok(watcher)
}

/// Surveillance du dossier d'un éditeur : l'abandonner l'arrête.
#[derive(Debug)]
pub struct EditorWatch {
    quiet: Arc<Quiet>,
    stop: mpsc::Sender<Msg>,
    thread: Option<JoinHandle<()>>,
}

impl EditorWatch {
    /// Surveille `base` ; `on_signal` est appelé dans un thread dédié. Erreur si l'observateur
    /// ne peut pas démarrer (dossier absent, limite du système).
    pub fn start(
        base: &Path,
        timing: Timing,
        on_signal: impl Fn(Signal) + Send + 'static,
    ) -> notify::Result<Self> {
        let quiet = Arc::new(Quiet::new(timing.grace));
        let (tx, rx) = mpsc::channel::<Msg>();
        let watcher = start_notify(base, tx.clone(), quiet.clone())?;
        let base: PathBuf = base.to_path_buf();
        let (notify_tx, notify_quiet) = (tx.clone(), quiet.clone());
        let thread = std::thread::Builder::new()
            .name("editor-watch".into())
            .spawn(move || {
                run(
                    &base,
                    timing,
                    Some(watcher),
                    &rx,
                    || start_notify(&base, notify_tx.clone(), notify_quiet.clone()),
                    on_signal,
                )
            })
            .map_err(notify::Error::io)?;
        Ok(Self {
            quiet,
            stop: tx,
            thread: Some(thread),
        })
    }

    /// Silence pendant une opération de l'éditeur (jusqu'à [`GRACE`] après l'abandon du garde).
    pub fn quiet(&self) -> QuietGuard {
        QuietGuard::new(self.quiet.clone())
    }

    /// Le thread de surveillance est-il terminé ?
    #[cfg(test)]
    fn finished(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }
}

impl Drop for EditorWatch {
    fn drop(&mut self) {
        let _ = self.stop.send(Msg::Stop);
        if let Some(thread) = self.thread.take() {
            // Le thread ne prend aucun verrou de l'application : l'attente est brève.
            let _ = thread.join();
        }
    }
}

/// Boucle du thread de surveillance.
fn run(
    base: &Path,
    timing: Timing,
    watcher: Option<RecommendedWatcher>,
    rx: &mpsc::Receiver<Msg>,
    restart: impl Fn() -> notify::Result<RecommendedWatcher>,
    on_signal: impl Fn(Signal),
) {
    // Gardé vivant par ce thread ; libéré au retour (arrêt) ou quand le dossier disparaît.
    let mut _watcher = watcher;
    let mut missing = false;
    loop {
        let changed = match rx.recv_timeout(timing.poll) {
            Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => return,
            Ok(Msg::Fs) => {
                // Anti-rebond : on attend le calme (au plus `max_wait`).
                let first = Instant::now();
                loop {
                    let left = timing
                        .max_wait
                        .saturating_sub(first.elapsed())
                        .min(timing.debounce);
                    if left.is_zero() {
                        break;
                    }
                    match rx.recv_timeout(left) {
                        Ok(Msg::Fs) | Ok(Msg::Error) => {}
                        Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => return,
                        Err(RecvTimeoutError::Timeout) => break,
                    }
                }
                true
            }
            Ok(Msg::Error) | Err(RecvTimeoutError::Timeout) => false,
        };
        if missing {
            // Dossier revenu (clé rebranchée) : la surveillance reprend.
            if present(base) {
                if let Ok(w) = restart() {
                    _watcher = Some(w);
                    missing = false;
                    on_signal(Signal::Changed);
                }
            }
        } else if !present(base) {
            missing = true;
            // Observateur libéré : plus de poignée sur un dossier disparu.
            _watcher = None;
            on_signal(Signal::Missing);
        } else if changed {
            on_signal(Signal::Changed);
        }
    }
}

/// Démarre la surveillance du dossier `base` de la fenêtre `label` : les signaux deviennent des
/// événements envoyés à cette seule fenêtre.
pub fn start_for_window(app: &AppHandle, label: &str, base: &Path) -> notify::Result<EditorWatch> {
    let (app, label) = (app.clone(), label.to_owned());
    EditorWatch::start(base, Timing::default(), move |signal| {
        let event = match signal {
            Signal::Changed => EVENT_CHANGED,
            Signal::Missing => EVENT_MISSING,
        };
        // `labeled` : reçu par les écouteurs de la fenêtre `label` seulement (les écouteurs
        // globaux des autres fenêtres le recevraient aussi : la page écoute sur sa fenêtre).
        let _ = app.emit_to(EventTarget::labeled(label.clone()), event, ());
    })
}

#[cfg(test)]
#[path = "editor_watch_tests.rs"]
mod tests;
