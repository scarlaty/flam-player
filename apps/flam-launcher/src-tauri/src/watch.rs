//! Surveillance optionnelle d'un dossier de source locale (#37).
//!
//! `folder_watch(source_id, enabled)` démarre ou arrête la surveillance (non récursive, comme
//! `folder_scan` par défaut) du dossier d'une source. Chaque rafale de modifications du dossier
//! (ajout, suppression, renommage ou modification d'un fichier) produit **un seul** événement
//! `folder://changed` (`{ "source_id": … }`), émis après [`DEBOUNCE`] de calme (au plus
//! [`MAX_WAIT`] après le premier changement, même si le dossier ne se calme jamais). Le front
//! relance alors `folder_scan` : l'événement ne porte aucune donnée d'histoire.
//!
//! La surveillance est un confort : si elle ne peut pas démarrer (dossier absent, limite du
//! système), `folder_watch` renvoie l'erreur et le front continue avec le rafraîchissement
//! manuel. Une seule surveillance par source ; `enabled = false` ou la fin de l'application
//! libère le thread et l'observateur système. Les accès en lecture et les changements dans le
//! dossier de cache de l'application ne déclenchent rien.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::folder::resolve_folder;
use crate::settings::Environment;
use crate::storage::ConfigState;
use crate::{AppError, AppResult};

/// Nom de l'événement Tauri (miroir de `FOLDER_CHANGED_EVENT` dans `src/api/folder.ts`).
pub const EVENT_CHANGED: &str = "folder://changed";
/// Calme exigé avant d'émettre l'événement.
pub const DEBOUNCE: Duration = Duration::from_millis(400);
/// Délai maximal entre le premier changement d'une rafale et l'événement.
pub const MAX_WAIT: Duration = Duration::from_secs(3);

/// Charge de `folder://changed` (miroir de `FolderChanged` dans `src/api/folder.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FolderChanged {
    pub source_id: String,
}

/// Observateur d'un dossier : l'abandonner arrête la surveillance et le thread d'anti-rebond.
pub struct DirWatcher {
    _watcher: RecommendedWatcher,
}

/// Un événement du système change-t-il la liste des fichiers ? Les accès en lecture et les
/// événements dont tous les chemins sont sous `ignore` (dossier de cache) sont écartés.
fn is_relevant(event: &Event, ignore: Option<&Path>) -> bool {
    if matches!(event.kind, EventKind::Access(_)) {
        return false;
    }
    match ignore {
        Some(dir) if !event.paths.is_empty() => !event.paths.iter().all(|p| p.starts_with(dir)),
        _ => true,
    }
}

impl DirWatcher {
    /// Surveille `root` ; `on_change` est appelé (dans un thread dédié) une fois par rafale.
    pub fn start(
        root: &Path,
        ignore: Option<PathBuf>,
        debounce: Duration,
        max_wait: Duration,
        on_change: impl Fn() + Send + 'static,
    ) -> notify::Result<Self> {
        let (tx, rx) = mpsc::channel::<()>();
        let mut watcher =
            notify::recommended_watcher(move |res: notify::Result<Event>| match res {
                Ok(event) if is_relevant(&event, ignore.as_deref()) => {
                    let _ = tx.send(());
                }
                Ok(_) => {}
                Err(err) => tracing::warn!("surveillance du dossier : {err}"),
            })?;
        watcher.watch(root, RecursiveMode::NonRecursive)?;
        std::thread::Builder::new()
            .name("folder-watch".into())
            .spawn(move || {
                // `recv` échoue quand l'observateur (donc l'émetteur) est abandonné.
                while rx.recv().is_ok() {
                    let first = Instant::now();
                    loop {
                        let left = max_wait.saturating_sub(first.elapsed()).min(debounce);
                        if left.is_zero() {
                            break;
                        }
                        match rx.recv_timeout(left) {
                            Ok(()) => {}
                            Err(mpsc::RecvTimeoutError::Timeout) => break,
                            // Arrêt demandé : on ne notifie plus.
                            Err(mpsc::RecvTimeoutError::Disconnected) => return,
                        }
                    }
                    on_change();
                }
            })?;
        Ok(Self { _watcher: watcher })
    }
}

/// Surveillances actives, par source.
#[derive(Clone, Default)]
pub struct WatchState(Arc<Mutex<HashMap<String, DirWatcher>>>);

impl WatchState {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, DirWatcher>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Remplace la surveillance de `source_id` (l'ancienne est arrêtée).
    pub fn set(&self, source_id: &str, watcher: DirWatcher) {
        self.lock().insert(source_id.to_string(), watcher);
    }

    /// Arrête la surveillance de `source_id` (sans effet si elle n'existe pas).
    pub fn stop(&self, source_id: &str) {
        self.lock().remove(source_id);
    }

    /// Nombre de surveillances actives.
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Démarre (`enabled = true`) ou arrête la surveillance du dossier de la source `source_id`.
/// Erreur `folder_unavailable` si le dossier a disparu.
#[tauri::command]
pub async fn folder_watch(
    app: AppHandle,
    config: State<'_, ConfigState>,
    watch: State<'_, WatchState>,
    source_id: String,
    enabled: bool,
) -> AppResult<()> {
    if !enabled {
        watch.stop(&source_id);
        return Ok(());
    }
    let (path, cache) = {
        let _guard = config.guard();
        resolve_folder(&config.dir, &Environment::current(), &source_id)?
    };
    if !std::fs::metadata(&path).is_ok_and(|m| m.is_dir()) {
        return Err(AppError::FolderUnavailable(format!(
            "dossier introuvable ou inaccessible : {}",
            path.display()
        )));
    }
    let id = source_id.clone();
    let watcher = DirWatcher::start(&path, Some(cache), DEBOUNCE, MAX_WAIT, move || {
        let _ = app.emit(
            EVENT_CHANGED,
            FolderChanged {
                source_id: id.clone(),
            },
        );
    })
    .map_err(|e| AppError::Io(std::io::Error::other(e.to_string())))?;
    watch.set(&source_id, watcher);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const QUIET: Duration = Duration::from_millis(80);

    fn counter() -> (Arc<AtomicUsize>, impl Fn() + Send + 'static) {
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        (count, move || {
            c.fetch_add(1, Ordering::SeqCst);
        })
    }

    /// Attend (au plus 10 s) que `cond` soit vraie.
    fn wait_for(cond: impl Fn() -> bool) -> bool {
        let end = Instant::now() + Duration::from_secs(10);
        while Instant::now() < end {
            if cond() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        cond()
    }

    #[test]
    fn ajout_modification_suppression_sont_signales() {
        let dir = tempfile::tempdir().unwrap();
        let (count, on_change) = counter();
        let _w = DirWatcher::start(dir.path(), None, QUIET, MAX_WAIT, on_change).unwrap();
        let file = dir.path().join("a.zip");

        std::fs::write(&file, b"1").unwrap();
        assert!(wait_for(|| count.load(Ordering::SeqCst) >= 1), "ajout");
        let after_add = count.load(Ordering::SeqCst);

        std::thread::sleep(QUIET * 4);
        std::fs::write(&file, b"22").unwrap();
        assert!(
            wait_for(|| count.load(Ordering::SeqCst) > after_add),
            "modification"
        );
        let after_edit = count.load(Ordering::SeqCst);

        std::thread::sleep(QUIET * 4);
        std::fs::remove_file(&file).unwrap();
        assert!(
            wait_for(|| count.load(Ordering::SeqCst) > after_edit),
            "suppression"
        );
    }

    #[test]
    fn une_rafale_ne_produit_qu_un_evenement() {
        let dir = tempfile::tempdir().unwrap();
        let (count, on_change) = counter();
        // Anti-rebond long : un runner chargé peut suspendre le thread plusieurs centaines de
        // ms entre deux écritures et scinder la rafale. On vérifie donc la coalescence (bien
        // moins d'événements que d'écritures), pas un compte exact.
        let debounce = Duration::from_millis(1500);
        let _w = DirWatcher::start(dir.path(), None, debounce, MAX_WAIT, on_change).unwrap();
        for i in 0..10 {
            std::fs::write(dir.path().join(format!("{i}.zip")), b"x").unwrap();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(wait_for(|| count.load(Ordering::SeqCst) >= 1));
        std::thread::sleep(debounce * 2);
        let events = count.load(Ordering::SeqCst);
        assert!(
            (1..=3).contains(&events),
            "{events} événements pour 10 écritures"
        );
    }

    #[test]
    fn un_flux_continu_est_notifie_apres_max_wait() {
        let dir = tempfile::tempdir().unwrap();
        let (count, on_change) = counter();
        let _w = DirWatcher::start(
            dir.path(),
            None,
            Duration::from_millis(300),
            Duration::from_millis(600),
            on_change,
        )
        .unwrap();
        let end = Instant::now() + Duration::from_millis(1500);
        let mut i = 0;
        while Instant::now() < end {
            std::fs::write(dir.path().join("f.zip"), i.to_string()).unwrap();
            i += 1;
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            count.load(Ordering::SeqCst) >= 1,
            "aucun événement pendant le flux"
        );
    }

    #[test]
    fn le_dossier_de_cache_est_ignore() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("cache");
        std::fs::create_dir(&cache).unwrap();
        let (count, on_change) = counter();
        let _w =
            DirWatcher::start(dir.path(), Some(cache.clone()), QUIET, MAX_WAIT, on_change).unwrap();
        std::fs::write(cache.join("t.png"), b"x").unwrap();
        std::thread::sleep(QUIET * 8);
        assert_eq!(count.load(Ordering::SeqCst), 0);
        // Un fichier voisin est bien signalé : la surveillance fonctionne.
        std::fs::write(dir.path().join("a.zip"), b"x").unwrap();
        assert!(wait_for(|| count.load(Ordering::SeqCst) >= 1));
    }

    #[test]
    fn abandonner_l_observateur_arrete_les_notifications() {
        let dir = tempfile::tempdir().unwrap();
        let (count, on_change) = counter();
        let w = DirWatcher::start(dir.path(), None, QUIET, MAX_WAIT, on_change).unwrap();
        drop(w);
        std::fs::write(dir.path().join("a.zip"), b"x").unwrap();
        std::thread::sleep(QUIET * 8);
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn dossier_absent_refuse() {
        let dir = tempfile::tempdir().unwrap();
        let (_count, on_change) = counter();
        let gone = dir.path().join("absent");
        assert!(DirWatcher::start(&gone, None, QUIET, MAX_WAIT, on_change).is_err());
    }

    #[test]
    fn l_etat_remplace_et_arrete_par_source() {
        let dir = tempfile::tempdir().unwrap();
        let state = WatchState::default();
        let mk = || {
            let (_c, f) = counter();
            DirWatcher::start(dir.path(), None, QUIET, MAX_WAIT, f).unwrap()
        };
        state.set("s", mk());
        state.set("s", mk());
        state.set("t", mk());
        assert_eq!(state.len(), 2);
        state.stop("s");
        state.stop("inconnue");
        assert_eq!(state.len(), 1);
        state.stop("t");
        assert!(state.is_empty());
    }

    #[test]
    fn filtre_des_evenements() {
        use notify::event::{AccessKind, CreateKind};
        let access = Event::new(EventKind::Access(AccessKind::Any)).add_path("/a/x".into());
        assert!(!is_relevant(&access, None));
        let create = Event::new(EventKind::Create(CreateKind::File)).add_path("/a/x".into());
        assert!(is_relevant(&create, None));
        assert!(!is_relevant(&create, Some(Path::new("/a"))));
        assert!(is_relevant(&create, Some(Path::new("/b"))));
        let mixed = create.clone().add_path("/b/y".into());
        assert!(is_relevant(&mixed, Some(Path::new("/a"))));
    }
}
