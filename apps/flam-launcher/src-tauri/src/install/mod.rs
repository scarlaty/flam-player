//! Pipeline d'installation : file de tâches persistée, téléchargement, conversion
//! (`telmi2flam`), publication dans la bibliothèque (`library`).
//!
//! Une tâche = une `StoryEntry` d'une source, retrouvée **par le backend** (cache du catalogue
//! ou dernier scan du dossier) à partir de `(source_id, entry_id)` : le front ne fournit jamais
//! d'URL ni de chemin. Étapes :
//!
//! 1. `downloading` (archive distante seulement) : `<cache>/archives/<uuid>-<version>.<ext>`
//!    via un `.part` repris après redémarrage (`download`) ;
//! 2. `converting` (zip Telmi seulement) : `telmi2flam.py --progress-json` vers
//!    `<cache>/library/.staging/<tâche>.plain.pk` (`convert`) ; un `.plain.pk` est vérifié et
//!    copié, un dossier `.plain` est empaqueté (`pack`) ;
//! 3. `installing` : renommage vers `<cache>/library/<uuid>-<version>.plain.pk` et index
//!    `library.json` (`library`), puis suppression de l'archive téléchargée.
//!
//! Concurrence : `max_downloads` téléchargements (paramètre, 2 par défaut), **une** conversion
//! à la fois. Chaque tâche tient le portillon du cache (`CatalogService::cache_gate`) en
//! lecture : « Vider le cache » attend la fin des tâches en cours.
//!
//! File persistée dans `<config>/install_queue.json` (tâches non terminées) : au démarrage,
//! elles sont relancées et les téléchargements reprennent (`Range` + `If-Range`).
//!
//! Événements : `install://progress` ([`InstallProgress`]), `install://done`
//! ([`InstallDone`]), `install://error` ([`InstallFailed`]).

pub mod convert;
pub mod download;
pub mod pack;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::State;
use tokio::sync::{watch, Notify, RwLock, Semaphore};

use crate::catalog::{blocking, CatalogService};
use crate::folder::FolderState;
use crate::library::{remove_if_exists, Library, LibraryEntry};
use crate::settings::{self, Environment, SettingsStore};
use crate::sources::{Source, SourcesStore};
use crate::storage::{self, ConfigState, Loaded};
use crate::story::{validate_story_id, StoryEntry, StoryKind};
use crate::{AppError, AppResult};

use self::convert::{Conversion, Step};
use self::download::{cancelled, Download, Validator};

/// Sous-dossier du cache des archives téléchargées (listé dans `cache::APP_CACHE_SUBDIRS`).
pub const ARCHIVES_DIR: &str = "archives";
/// File persistée, dans le dossier de configuration.
pub const QUEUE_FILE: &str = "install_queue.json";
const QUEUE_SCHEMA: u32 = 1;
/// Taille maximale d'une archive téléchargée.
const MAX_DOWNLOAD_BYTES: u64 = 4 * 1024 * 1024 * 1024;
/// Délai laissé au convertisseur après Ctrl+Break avant l'arrêt forcé.
pub const GRACE: Duration = Duration::from_secs(5);
/// Intervalle minimal entre deux événements de progression d'une tâche.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(200);
/// Tâches terminées gardées en mémoire (les plus anciennes sont oubliées).
const MAX_FINISHED: usize = 100;

pub const EVENT_PROGRESS: &str = "install://progress";
pub const EVENT_DONE: &str = "install://done";
pub const EVENT_ERROR: &str = "install://error";

/// État d'une tâche (miroir de `TaskState` dans `src/api/install.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Queued,
    Downloading,
    Converting,
    Installing,
    Done,
    Failed,
    Cancelled,
}

impl TaskState {
    pub fn is_active(self) -> bool {
        !matches!(self, Self::Done | Self::Failed | Self::Cancelled)
    }
}

/// Erreur d'une tâche : `{ kind, message }` comme `AppError`, plus le code du convertisseur
/// (`missing_asset`…) pour `kind = "conversion"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TaskError {
    pub kind: String,
    pub message: String,
    pub code: Option<String>,
}

impl From<&AppError> for TaskError {
    fn from(err: &AppError) -> Self {
        Self {
            kind: err.kind().to_string(),
            message: err.to_string(),
            code: match err {
                AppError::Conversion { code, .. } => Some(code.clone()),
                _ => None,
            },
        }
    }
}

/// Tâche d'installation (miroir de `InstallTask` dans `src/api/install.ts`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InstallTask {
    pub id: String,
    pub entry: StoryEntry,
    pub state: TaskState,
    /// Avancement de l'étape en cours (0 à 100), si connu.
    pub percent: Option<f64>,
    pub error: Option<TaskError>,
    /// Création (secondes Unix).
    pub created_at: u64,
}

/// Charge de `install://progress` : changement d'état ou avancement.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InstallProgress {
    pub task_id: String,
    pub state: TaskState,
    /// Avancement de l'étape en cours (0 à 100), si connu.
    pub percent: Option<f64>,
    /// Téléchargement : octets reçus et taille annoncée.
    pub received: Option<u64>,
    pub total: Option<u64>,
    /// Conversion : phase de `telmi2flam` (`images`, `audio`, `scripts`, `zip`).
    pub phase: Option<String>,
}

/// Charge de `install://done`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InstallDone {
    pub task_id: String,
    pub entry: LibraryEntry,
    /// Avertissements de la conversion.
    pub warnings: u32,
}

/// Charge de `install://error` : `state` vaut `failed` ou `cancelled`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InstallFailed {
    pub task_id: String,
    pub state: TaskState,
    pub error: TaskError,
}

/// Destination des événements (Tauri en production, enregistreur en test).
pub trait InstallEvents: Send + Sync {
    fn progress(&self, event: &InstallProgress);
    fn done(&self, event: &InstallDone);
    fn failed(&self, event: &InstallFailed);
}

/// Émission des événements Tauri.
pub struct TauriEvents(pub tauri::AppHandle);

impl InstallEvents for TauriEvents {
    fn progress(&self, event: &InstallProgress) {
        use tauri::Emitter;
        let _ = self.0.emit(EVENT_PROGRESS, event);
    }
    fn done(&self, event: &InstallDone) {
        use tauri::Emitter;
        let _ = self.0.emit(EVENT_DONE, event);
    }
    fn failed(&self, event: &InstallFailed) {
        use tauri::Emitter;
        let _ = self.0.emit(EVENT_ERROR, event);
    }
}

/// Outils et dossiers effectifs, relus au début de chaque tâche.
#[derive(Debug, Clone)]
pub struct Toolchain {
    pub cache_root: PathBuf,
    pub python: Option<PathBuf>,
    pub converter: Option<PathBuf>,
    pub max_downloads: usize,
}

pub type Resolver = Arc<dyn Fn() -> AppResult<Toolchain> + Send + Sync>;

/// `telmi2flam.py` : `<exe>/telmi2flam/telmi2flam.py` (application installée, #21), sinon en
/// développement `tools/telmi2flam/telmi2flam.py` d'un dossier ancêtre (racine du dépôt).
pub fn detect_converter(env: &Environment) -> Option<PathBuf> {
    let exe_dir = env.exe_dir.as_deref()?;
    let bundled = exe_dir.join("telmi2flam").join("telmi2flam.py");
    if bundled.is_file() {
        return Some(bundled);
    }
    if env.search_repo_build {
        return exe_dir
            .ancestors()
            .map(|dir| dir.join("tools").join("telmi2flam").join("telmi2flam.py"))
            .find(|candidate| candidate.is_file());
    }
    None
}

/// Résolution réelle : `settings.json` (cache, Python, téléchargements simultanés) et
/// détection du convertisseur.
pub fn settings_resolver(config_dir: PathBuf) -> Resolver {
    Arc::new(move || {
        let env = Environment::current();
        let saved = SettingsStore::new(&config_dir).load()?;
        let (resolved, _) = settings::resolve(&saved, &env);
        let cache_root = PathBuf::from(resolved.cache_dir);
        std::fs::create_dir_all(&cache_root)?;
        Ok(Toolchain {
            cache_root,
            python: resolved.python_path.map(PathBuf::from),
            converter: detect_converter(&env),
            max_downloads: saved.max_downloads as usize,
        })
    })
}

/// Repli pour la version du convertisseur : empreinte `sha256:<12 hex>` du script, utilisée
/// seulement si l'événement `start` ne porte pas de champ `version`.
pub fn converter_fingerprint(script: &Path) -> AppResult<String> {
    let bytes = std::fs::read(script)?;
    let digest = Sha256::digest(&bytes);
    let hex: String = digest[..6].iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!("sha256:{hex}"))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Identifiant de tâche : UUID v4 sous sa forme canonique (minuscules, avec tirets). Il sert
/// de nom de fichier dans `library/.staging/`, donc il est revérifié à la relecture de la file.
fn is_task_id(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok_and(|u| u.hyphenated().to_string() == id)
}

/// `dir/name`, refusé si le résultat ne reste pas **directement** sous `dir`.
fn child_of(dir: &Path, name: &str) -> AppResult<PathBuf> {
    let path = dir.join(name);
    let direct = path.parent() == Some(dir)
        && path.file_name().and_then(|n| n.to_str()) == Some(name)
        && !name.contains(['/', '\\', ':']);
    if !direct {
        return Err(AppError::Validation(format!(
            "chemin refusé hors de {} : {name:?}",
            dir.display()
        )));
    }
    Ok(path)
}

fn is_remote(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://")
}

/// Revalide une entrée avant d'en faire une tâche : `uuid` utilisable comme nom de fichier,
/// lien http(s) valide ou chemin local absolu, dossier `.plain` jamais distant.
fn validate_entry(entry: &StoryEntry) -> AppResult<()> {
    if !validate_story_id(&entry.uuid) {
        return Err(AppError::Validation(format!(
            "identifiant d'histoire refusé : {:?}",
            entry.uuid
        )));
    }
    if is_remote(&entry.download_url) {
        url::Url::parse(&entry.download_url)
            .map_err(|_| AppError::Validation("lien de téléchargement invalide".into()))?;
        if entry.kind == StoryKind::FlamPlain {
            return Err(AppError::Validation(
                "un dossier .plain ne peut pas être téléchargé".into(),
            ));
        }
    } else if !Path::new(&entry.download_url).is_absolute() {
        return Err(AppError::Validation(
            "le chemin de l'histoire doit être absolu".into(),
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------
// Limite de téléchargements simultanés, modifiable à chaud
// ---------------------------------------------------------------------------------------

struct Limiter {
    /// (en cours, limite)
    state: Mutex<(usize, usize)>,
    notify: Notify,
}

struct LimiterGuard(Arc<Limiter>);

impl Limiter {
    fn new(limit: usize) -> Self {
        Self {
            state: Mutex::new((0, limit.max(1))),
            notify: Notify::new(),
        }
    }

    fn set_limit(&self, limit: usize) {
        lock(&self.state).1 = limit.max(1);
        self.notify.notify_waiters();
    }

    async fn acquire(self: &Arc<Self>) -> LimiterGuard {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let mut state = lock(&self.state);
                if state.0 < state.1 {
                    state.0 += 1;
                    return LimiterGuard(self.clone());
                }
            }
            notified.await;
        }
    }
}

impl Drop for LimiterGuard {
    fn drop(&mut self) {
        lock(&self.0.state).0 -= 1;
        self.0.notify.notify_waiters();
    }
}

// ---------------------------------------------------------------------------------------
// File persistée
// ---------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QueuedTask {
    id: String,
    entry: StoryEntry,
    created_at: u64,
    #[serde(default)]
    validator: Validator,
}

/// Contenu de la file à écrire, avec son numéro d'ordre.
struct QueueSnapshot {
    seq: u64,
    bytes: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize)]
struct QueueFile {
    schema_version: u32,
    tasks: Vec<QueuedTask>,
}

fn load_queue(path: &Path) -> Vec<QueuedTask> {
    match storage::load_versioned::<QueueFile>(path, QUEUE_SCHEMA, |_, _| {}) {
        Ok(Loaded::Found(file)) => file.tasks,
        Ok(_) => Vec::new(),
        Err(err) => {
            tracing::warn!("file d'installation ignorée : {err}");
            Vec::new()
        }
    }
}

// ---------------------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------------------

struct Slot {
    task: InstallTask,
    cancel: watch::Sender<bool>,
    validator: Validator,
}

/// Paramètres du service (injectés pour les tests).
pub struct InstallOptions {
    pub client: reqwest::Client,
    pub resolver: Resolver,
    pub events: Arc<dyn InstallEvents>,
    pub queue_path: PathBuf,
    pub gate: Arc<RwLock<()>>,
    pub grace: Duration,
}

struct Inner {
    opts: InstallOptions,
    tasks: Mutex<Vec<Slot>>,
    downloads: Arc<Limiter>,
    conversions: Semaphore,
    /// Sérialise les écritures de la bibliothèque (pipeline et `library_remove`).
    library_lock: Mutex<()>,
    /// Numéro de la dernière capture de la file demandée (attribué sous le verrou `tasks`).
    queue_requested: AtomicU64,
    /// Numéro de la dernière capture écrite ; sérialise aussi les écritures du fichier.
    queue_written: Mutex<u64>,
}

/// Service d'installation (état Tauri).
#[derive(Clone)]
pub struct InstallService(Arc<Inner>);

impl InstallService {
    pub fn new(opts: InstallOptions) -> Self {
        Self(Arc::new(Inner {
            opts,
            tasks: Mutex::default(),
            downloads: Arc::new(Limiter::new(settings::DEFAULT_MAX_DOWNLOADS as usize)),
            conversions: Semaphore::new(1),
            library_lock: Mutex::default(),
            queue_requested: AtomicU64::new(0),
            queue_written: Mutex::new(0),
        }))
    }

    /// Relance les tâches de la file persistée (au démarrage).
    ///
    /// Le fichier est relu comme une donnée non fiable : une tâche dont l'identifiant n'est
    /// pas un UUID canonique est ignorée, et l'entrée n'est **pas** reprise du fichier : elle
    /// est ré-résolue par `lookup(source_id, entry_id)` (cache du catalogue, scan du dossier).
    /// Une entrée introuvable ou invalide abandonne la tâche (journal, pas de reprise).
    pub async fn restore<F, Fut>(&self, lookup: F)
    where
        F: Fn(String, String) -> Fut,
        Fut: std::future::Future<Output = AppResult<StoryEntry>>,
    {
        let mut accepted = Vec::new();
        for q in load_queue(&self.0.opts.queue_path) {
            if !is_task_id(&q.id) {
                tracing::warn!(
                    "tâche d'installation ignorée : identifiant invalide {:?}",
                    q.id
                );
                continue;
            }
            let fresh = lookup(q.entry.source_id.clone(), q.entry.entry_id.clone())
                .await
                .and_then(|entry| validate_entry(&entry).map(|()| entry));
            match fresh {
                Ok(entry) => accepted.push((q, entry)),
                Err(err) => tracing::warn!(
                    "tâche d'installation {} abandonnée ({}) : {err}",
                    q.id,
                    q.entry.entry_id
                ),
            }
        }
        let (ids, snap): (Vec<String>, QueueSnapshot) = {
            let mut tasks = lock(&self.0.tasks);
            for (q, entry) in accepted {
                if tasks.iter().any(|s| s.task.id == q.id) {
                    continue;
                }
                // Le validateur ne vaut que pour la même archive (même lien, même version).
                let validator = if entry.download_url == q.entry.download_url
                    && entry.version == q.entry.version
                {
                    q.validator
                } else {
                    Validator::default()
                };
                tasks.push(Slot {
                    task: InstallTask {
                        id: q.id,
                        entry,
                        state: TaskState::Queued,
                        percent: None,
                        error: None,
                        created_at: q.created_at,
                    },
                    cancel: watch::channel(false).0,
                    validator,
                });
            }
            let snap = self.snapshot(&tasks);
            (tasks.iter().map(|s| s.task.id.clone()).collect(), snap)
        };
        self.write_queue(snap).await;
        for id in ids {
            self.spawn(id);
        }
    }

    /// Ajoute une tâche pour `entry` (déjà résolue par le backend). Une tâche active pour le
    /// même `uuid` est renvoyée telle quelle au lieu d'en créer une seconde.
    pub fn start(&self, entry: StoryEntry) -> AppResult<InstallTask> {
        validate_entry(&entry)?;
        let (task, snap) = {
            let mut tasks = lock(&self.0.tasks);
            if let Some(slot) = tasks
                .iter()
                .find(|s| s.task.state.is_active() && s.task.entry.uuid == entry.uuid)
            {
                return Ok(slot.task.clone());
            }
            let task = InstallTask {
                id: uuid::Uuid::new_v4().to_string(),
                entry,
                state: TaskState::Queued,
                percent: None,
                error: None,
                created_at: now_secs(),
            };
            tasks.push(Slot {
                task: task.clone(),
                cancel: watch::channel(false).0,
                validator: Validator::default(),
            });
            let snap = self.snapshot(&tasks);
            (task, snap)
        };
        self.write_queue_detached(snap);
        self.emit_state(&task.id, TaskState::Queued, None);
        self.spawn(task.id.clone());
        Ok(task)
    }

    /// Demande l'annulation d'une tâche (sans effet si elle est terminée).
    pub fn cancel(&self, id: &str) -> AppResult<()> {
        let tasks = lock(&self.0.tasks);
        let slot = tasks
            .iter()
            .find(|s| s.task.id == id)
            .ok_or_else(|| AppError::NotFound(format!("tâche inconnue : {id}")))?;
        if slot.task.state.is_active() {
            slot.cancel.send_replace(true);
        }
        Ok(())
    }

    /// Tâches connues (en cours, en attente, puis terminées pendant cette session).
    pub fn list(&self) -> Vec<InstallTask> {
        lock(&self.0.tasks).iter().map(|s| s.task.clone()).collect()
    }

    /// Histoires installées.
    pub async fn library_list(&self) -> AppResult<Vec<LibraryEntry>> {
        let tc = (self.0.opts.resolver)()?;
        let inner = self.0.clone();
        blocking(move || {
            let _held = lock(&inner.library_lock);
            Library::new(&tc.cache_root).list()
        })
        .await?
    }

    /// Désinstalle l'histoire `uuid`.
    pub async fn library_remove(&self, uuid: String) -> AppResult<()> {
        let tc = (self.0.opts.resolver)()?;
        let inner = self.0.clone();
        let _gate = inner.opts.gate.clone().read_owned().await;
        blocking(move || {
            let _held = lock(&inner.library_lock);
            Library::new(&tc.cache_root).remove(&uuid)
        })
        .await?
    }

    /// Construit le contenu de la file (sous le verrou `tasks`, sans aucune E/S) et lui attribue
    /// un numéro d'ordre. L'écriture se fait ensuite hors du verrou ([`Self::write_queue`]).
    fn snapshot(&self, tasks: &[Slot]) -> QueueSnapshot {
        let file = QueueFile {
            schema_version: QUEUE_SCHEMA,
            tasks: tasks
                .iter()
                .filter(|s| s.task.state.is_active())
                .map(|s| QueuedTask {
                    id: s.task.id.clone(),
                    entry: s.task.entry.clone(),
                    created_at: s.task.created_at,
                    validator: s.validator.clone(),
                })
                .collect(),
        };
        let mut bytes = serde_json::to_vec_pretty(&file).unwrap_or_default();
        bytes.push(b'\n');
        QueueSnapshot {
            seq: self.0.queue_requested.fetch_add(1, Ordering::SeqCst) + 1,
            bytes,
        }
    }

    /// Écriture bloquante d'une capture : ignorée si une capture plus récente est déjà écrite.
    fn write_queue_blocking(inner: &Inner, snap: &QueueSnapshot) {
        let mut written = lock(&inner.queue_written);
        if snap.seq <= *written {
            return;
        }
        // Même en cas d'échec, la capture est « consommée » : une capture plus ancienne ne doit
        // jamais s'écrire après elle.
        *written = snap.seq;
        if let Err(err) = storage::write_atomic(&inner.opts.queue_path, &snap.bytes) {
            tracing::warn!("file d'installation non enregistrée : {err}");
        }
    }

    /// Écrit la file sans attendre (hors verrou, sur un thread bloquant).
    fn write_queue_detached(&self, snap: QueueSnapshot) {
        let inner = self.0.clone();
        tauri::async_runtime::spawn_blocking(move || Self::write_queue_blocking(&inner, &snap));
    }

    /// Écrit la file et attend la fin de l'écriture.
    async fn write_queue(&self, snap: QueueSnapshot) {
        let inner = self.0.clone();
        let _ = blocking(move || Self::write_queue_blocking(&inner, &snap)).await;
    }

    /// Attend que toutes les captures demandées soient écrites (tests).
    #[cfg(test)]
    async fn flush_queue(&self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while *lock(&self.0.queue_written) < self.0.queue_requested.load(Ordering::SeqCst)
            && Instant::now() < deadline
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    fn spawn(&self, id: String) {
        let service = self.clone();
        tauri::async_runtime::spawn(async move { service.run(id).await });
    }

    fn emit(&self, event: InstallProgress) {
        {
            let mut tasks = lock(&self.0.tasks);
            if let Some(slot) = tasks.iter_mut().find(|s| s.task.id == event.task_id) {
                slot.task.state = event.state;
                slot.task.percent = event.percent;
            }
        }
        self.0.opts.events.progress(&event);
    }

    fn emit_state(&self, id: &str, state: TaskState, percent: Option<f64>) {
        self.emit(InstallProgress {
            task_id: id.to_string(),
            state,
            percent,
            received: None,
            total: None,
            phase: None,
        });
    }

    fn save_validator(&self, id: &str, validator: &Validator) {
        let snap = {
            let mut tasks = lock(&self.0.tasks);
            if let Some(slot) = tasks.iter_mut().find(|s| s.task.id == id) {
                slot.validator = validator.clone();
            }
            self.snapshot(&tasks)
        };
        self.write_queue_detached(snap);
    }

    async fn run(self, id: String) {
        let Some((entry, mut cancel, validator)) = lock(&self.0.tasks)
            .iter()
            .find(|s| s.task.id == id)
            .map(|s| {
                (
                    s.task.entry.clone(),
                    s.cancel.subscribe(),
                    s.validator.clone(),
                )
            })
        else {
            return;
        };
        let result = self.pipeline(&id, &entry, &mut cancel, validator).await;
        let (state, error) = match &result {
            Ok(_) => (TaskState::Done, None),
            Err(AppError::Cancelled) => (
                TaskState::Cancelled,
                Some(TaskError::from(&AppError::Cancelled)),
            ),
            Err(err) => (TaskState::Failed, Some(TaskError::from(err))),
        };
        let snap = {
            let mut tasks = lock(&self.0.tasks);
            if let Some(slot) = tasks.iter_mut().find(|s| s.task.id == id) {
                slot.task.state = state;
                slot.task.error = error.clone();
                slot.task.percent = (state == TaskState::Done).then_some(100.0);
            }
            let snap = self.snapshot(&tasks);
            let finished = tasks.iter().filter(|s| !s.task.state.is_active()).count();
            if finished > MAX_FINISHED {
                if let Some(pos) = tasks.iter().position(|s| !s.task.state.is_active()) {
                    tasks.remove(pos);
                }
            }
            snap
        };
        // Fichier de file à jour avant d'annoncer la fin (écriture hors verrou).
        self.write_queue(snap).await;
        match result {
            Ok((entry, warnings)) => self.0.opts.events.done(&InstallDone {
                task_id: id,
                entry,
                warnings,
            }),
            Err(err) => {
                if !matches!(err, AppError::Cancelled) {
                    tracing::warn!("installation {id} en échec : {err}");
                }
                self.0.opts.events.failed(&InstallFailed {
                    task_id: id,
                    state,
                    error: error.unwrap_or_else(|| TaskError::from(&err)),
                });
            }
        }
    }

    async fn pipeline(
        &self,
        id: &str,
        entry: &StoryEntry,
        cancel: &mut watch::Receiver<bool>,
        validator: Validator,
    ) -> AppResult<(LibraryEntry, u32)> {
        if !is_task_id(id) {
            return Err(AppError::Validation(format!(
                "identifiant de tâche invalide : {id:?}"
            )));
        }
        let tc = (self.0.opts.resolver)()?;
        self.0.downloads.set_limit(tc.max_downloads);
        let _gate = tokio::select! {
            g = self.0.opts.gate.clone().read_owned() => g,
            _ = cancelled(cancel) => return Err(AppError::Cancelled),
        };
        let library = Library::new(&tc.cache_root);
        let staging = {
            let library = Library::new(&tc.cache_root);
            blocking(move || library.staging_dir()).await??
        };
        let staged = child_of(&staging, &format!("{id}.plain.pk"))?;
        let result = self
            .produce(id, entry, &tc, &staged, cancel, validator)
            .await;
        let (downloaded, converter, warnings) = match result {
            Ok(produced) => produced,
            Err(err) => {
                discard(staged.clone()).await;
                return Err(err);
            }
        };
        if *cancel.borrow() {
            discard(staged.clone()).await;
            return Err(AppError::Cancelled);
        }
        self.emit_state(id, TaskState::Installing, None);
        let inner = self.0.clone();
        let source = entry.clone();
        let installed = blocking(move || {
            let _held = lock(&inner.library_lock);
            let result = library.install(&staged, &source, converter, now_secs());
            if result.is_err() {
                let _ = remove_if_exists(&staged);
            }
            result
        })
        .await??;
        if let Some(archive) = downloaded {
            let _ = blocking(move || remove_archive(&archive)).await;
        }
        Ok((installed, warnings))
    }

    /// Produit le `.plain.pk` à publier dans `staged`. Renvoie l'archive téléchargée (à
    /// supprimer après publication), la version du convertisseur et ses avertissements.
    async fn produce(
        &self,
        id: &str,
        entry: &StoryEntry,
        tc: &Toolchain,
        staged: &Path,
        cancel: &mut watch::Receiver<bool>,
        validator: Validator,
    ) -> AppResult<(Option<PathBuf>, Option<String>, u32)> {
        let downloaded = if is_remote(&entry.download_url) {
            Some(self.fetch(id, entry, tc, cancel, validator).await?)
        } else {
            None
        };
        let source = downloaded
            .clone()
            .unwrap_or_else(|| PathBuf::from(&entry.download_url));
        let result = self.transform(id, entry, tc, &source, staged, cancel).await;
        match result {
            Ok((converter, warnings)) => Ok((downloaded, converter, warnings)),
            Err(err) => {
                // Annulée (en attente du créneau de conversion comme pendant la conversion) :
                // l'archive téléchargée est abandonnée avec la tâche.
                if matches!(err, AppError::Cancelled) {
                    if let Some(archive) = downloaded {
                        let _ = blocking(move || remove_archive(&archive)).await;
                    }
                }
                Err(err)
            }
        }
    }

    /// Convertit, vérifie ou empaquette `source` vers `staged`. Renvoie la version du
    /// convertisseur et ses avertissements.
    async fn transform(
        &self,
        id: &str,
        entry: &StoryEntry,
        tc: &Toolchain,
        source: &Path,
        staged: &Path,
        cancel: &mut watch::Receiver<bool>,
    ) -> AppResult<(Option<String>, u32)> {
        let source = source.to_path_buf();
        match entry.kind {
            StoryKind::TelmiZip => {
                let python = tc.python.clone().ok_or_else(|| {
                    AppError::ConverterUnavailable(
                        "Python introuvable : indiquez le chemin de python.exe dans les paramètres"
                            .into(),
                    )
                })?;
                let script = tc.converter.clone().ok_or_else(|| {
                    AppError::ConverterUnavailable("convertisseur telmi2flam.py introuvable".into())
                })?;
                if !source.is_file() {
                    return Err(AppError::NotFound(format!(
                        "archive introuvable : {}",
                        source.display()
                    )));
                }
                let _slot = tokio::select! {
                    s = self.0.conversions.acquire() => s.map_err(|e| AppError::Io(std::io::Error::other(e.to_string())))?,
                    _ = cancelled(cancel) => return Err(AppError::Cancelled),
                };
                self.emit_state(id, TaskState::Converting, Some(0.0));
                let mut last = Instant::now();
                let mut on_step = |step: &Step| {
                    let end = step.done >= step.total;
                    if end || last.elapsed() >= PROGRESS_INTERVAL {
                        last = Instant::now();
                        self.emit(InstallProgress {
                            task_id: id.to_string(),
                            state: TaskState::Converting,
                            percent: Some(step.percent()),
                            received: None,
                            total: None,
                            phase: Some(step.phase.clone()),
                        });
                    }
                };
                let converted = convert::convert(
                    &Conversion {
                        python: &python,
                        script: &script,
                        input: &source,
                        output: staged,
                        grace: self.0.opts.grace,
                    },
                    cancel,
                    &mut on_step,
                )
                .await;
                let converted = converted?;
                // Version annoncée par l'événement `start` ; empreinte du script en repli
                // (convertisseur plus ancien, sans champ `version`).
                let converter = match converted.version {
                    Some(version) => version,
                    None => blocking(move || converter_fingerprint(&script)).await??,
                };
                Ok((Some(converter), converted.warnings))
            }
            StoryKind::FlamPk => {
                self.emit_state(id, TaskState::Installing, None);
                let (from, to) = (source.clone(), staged.to_path_buf());
                blocking(move || {
                    pack::check_plain_pk(&from)?;
                    std::fs::copy(&from, &to)?;
                    Ok::<_, AppError>(())
                })
                .await??;
                Ok((None, 0))
            }
            StoryKind::FlamPlain => {
                self.emit_state(id, TaskState::Installing, None);
                let (from, to) = (source.clone(), staged.to_path_buf());
                blocking(move || pack::pack_plain_dir(&from, &to)).await??;
                Ok((None, 0))
            }
        }
    }

    /// Télécharge l'archive distante (ou réutilise une archive complète déjà téléchargée).
    async fn fetch(
        &self,
        id: &str,
        entry: &StoryEntry,
        tc: &Toolchain,
        cancel: &mut watch::Receiver<bool>,
        validator: Validator,
    ) -> AppResult<PathBuf> {
        let ext = match entry.kind {
            StoryKind::TelmiZip => "zip",
            StoryKind::FlamPk => "plain.pk",
            StoryKind::FlamPlain => {
                return Err(AppError::Validation(
                    "un dossier .plain ne peut pas être téléchargé".into(),
                ))
            }
        };
        let dir = tc.cache_root.join(ARCHIVES_DIR);
        // `uuid` revalidé par `validate_entry` : nom de fichier sûr.
        let dest = child_of(
            &dir,
            &format!("{}-{}.{ext}", entry.uuid, entry.version.unwrap_or(0)),
        )?;
        let prepared = {
            let (dest, url) = (dest.clone(), entry.download_url.clone());
            let known = validator != Validator::default();
            blocking(move || prepare_archive(&dir, &dest, &url, known)).await??
        };
        // Validateur de la tâche, sinon celui mémorisé avec le `.part` d'un même lien.
        let mut validator = match prepared {
            Prepared::Reuse => return check_archive(dest).await,
            Prepared::Download(known) if validator == Validator::default() => known,
            Prepared::Download(_) => validator,
        };
        let _slot = tokio::select! {
            s = self.0.downloads.acquire() => s,
            _ = cancelled(cancel) => return Err(AppError::Cancelled),
        };
        self.emit_state(id, TaskState::Downloading, Some(0.0));
        let mut last: Option<Instant> = None;
        let mut on_progress = |received: u64, total: Option<u64>| {
            let end = total == Some(received);
            if end || last.is_none_or(|t| t.elapsed() >= PROGRESS_INTERVAL) {
                last = Some(Instant::now());
                self.emit(InstallProgress {
                    task_id: id.to_string(),
                    state: TaskState::Downloading,
                    percent: total
                        .filter(|t| *t > 0)
                        .map(|t| received as f64 * 100.0 / t as f64),
                    received: Some(received),
                    total,
                    phase: None,
                });
            }
        };
        let mut on_validator = |v: &Validator| self.save_validator(id, v);
        let result = download::download(
            &Download {
                client: &self.0.opts.client,
                url: &entry.download_url,
                dest: &dest,
                max_bytes: MAX_DOWNLOAD_BYTES,
            },
            &mut validator,
            cancel,
            &mut on_validator,
            &mut on_progress,
        )
        .await;
        if matches!(result, Err(AppError::Cancelled)) {
            discard(download::part_path(&dest)).await;
        }
        // Mémorise le lien et le validateur à côté de l'archive (ou du `.part` conservé après
        // une coupure) : une relance reprend le téléchargement, et l'archive n'est réutilisée
        // que pour le même lien.
        {
            let (dest, url, validator) = (dest.clone(), entry.download_url.clone(), validator);
            blocking(move || write_archive_meta(&dest, &url, &validator)).await?;
        }
        result?;
        check_archive(dest).await
    }
}

/// Vérifie qu'une archive téléchargée est un zip **complet** (répertoire central lisible) :
/// sans `Content-Length`, une connexion coupée ne se distingue pas d'une fin normale. Une
/// archive invalide est supprimée.
async fn check_archive(dest: PathBuf) -> AppResult<PathBuf> {
    let check = dest.clone();
    let valid = blocking(move || {
        std::fs::File::open(&check)
            .map_err(|e| e.to_string())
            .and_then(|f| {
                zip::ZipArchive::new(std::io::BufReader::new(f))
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            })
    })
    .await?;
    match valid {
        Ok(()) => Ok(dest),
        Err(reason) => {
            let _ = blocking(move || remove_archive(&dest)).await;
            Err(AppError::DownloadInvalid(format!(
                "archive téléchargée incomplète ou invalide : {reason}"
            )))
        }
    }
}

/// Mémoire d'une archive du cache, à côté d'elle (`<archive>.meta.json`).
#[derive(Debug, Serialize, Deserialize)]
struct ArchiveMeta {
    url: String,
    #[serde(default)]
    validator: Validator,
    /// Taille de l'archive complète (absente tant que le téléchargement n'est pas terminé).
    size: Option<u64>,
}

fn meta_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".meta.json");
    dest.with_file_name(name)
}

enum Prepared {
    /// Archive complète déjà présente et issue du même lien : à réutiliser.
    Reuse,
    /// À télécharger ; validateur connu pour reprendre un `.part` du même lien.
    Download(Validator),
}

/// Taille maximale d'une mémoire d'archive lue, et d'une valeur de validateur.
const MAX_META_BYTES: u64 = 4096;
const MAX_VALIDATOR_LEN: usize = 256;

/// Lit la mémoire d'une archive : lecture bornée, validateur revalidé (ASCII visible, longueur
/// bornée, sinon validateur vide pour ne pas faire échouer chaque relance).
fn read_archive_meta(path: &Path) -> Option<ArchiveMeta> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_META_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_META_BYTES {
        return None;
    }
    let mut meta: ArchiveMeta = serde_json::from_slice(&bytes).ok()?;
    let valid = |v: &Option<String>| {
        v.as_deref().is_none_or(|v| {
            v.len() <= MAX_VALIDATOR_LEN && v.bytes().all(|b| (0x20..=0x7e).contains(&b))
        })
    };
    if !valid(&meta.validator.etag) || !valid(&meta.validator.last_modified) {
        meta.validator = Validator::default();
    }
    Some(meta)
}

/// Décide de la réutilisation d'une archive ou d'un `.part` (E/S bloquantes). Une archive sans
/// mémoire, d'un autre lien ou d'une autre taille est supprimée ; un `.part` d'un autre lien
/// aussi.
fn prepare_archive(
    dir: &Path,
    dest: &Path,
    url: &str,
    task_has_validator: bool,
) -> AppResult<Prepared> {
    std::fs::create_dir_all(dir)?;
    let meta_file = meta_path(dest);
    let meta = read_archive_meta(&meta_file);
    let same = meta.as_ref().is_some_and(|m| m.url == url);
    if dest.is_file() {
        let size = std::fs::metadata(dest)?.len();
        if same && meta.as_ref().and_then(|m| m.size) == Some(size) {
            return Ok(Prepared::Reuse);
        }
        remove_if_exists(dest)?;
    }
    match meta {
        Some(m) if same => Ok(Prepared::Download(m.validator)),
        _ => {
            // Le validateur de la tâche (file persistée) vaut pour ce lien : son `.part` reste.
            if !task_has_validator {
                remove_if_exists(&download::part_path(dest))?;
            }
            remove_if_exists(&meta_file)?;
            Ok(Prepared::Download(Validator::default()))
        }
    }
}

/// Écrit la mémoire de l'archive (ou la supprime s'il ne reste ni archive ni `.part`).
fn write_archive_meta(dest: &Path, url: &str, validator: &Validator) {
    let size = std::fs::metadata(dest).ok().map(|m| m.len());
    let has_part = std::fs::metadata(download::part_path(dest)).is_ok();
    let file = meta_path(dest);
    if size.is_none() && !has_part {
        let _ = remove_if_exists(&file);
        return;
    }
    let meta = ArchiveMeta {
        url: url.to_string(),
        validator: validator.clone(),
        size,
    };
    if let Err(err) = storage::write_json(&file, &meta) {
        tracing::warn!("mémoire de l'archive non enregistrée : {err}");
    }
}

/// Supprime l'archive téléchargée, son `.part` et sa mémoire.
fn remove_archive(dest: &Path) {
    let _ = remove_if_exists(dest);
    let _ = remove_if_exists(&download::part_path(dest));
    let _ = remove_if_exists(&meta_path(dest));
}

/// Supprime un fichier hors du runtime (erreurs ignorées).
async fn discard(path: PathBuf) {
    let _ = tokio::fs::remove_file(path).await;
}

// ---------------------------------------------------------------------------------------
// Commandes Tauri
// ---------------------------------------------------------------------------------------

/// Retrouve l'entrée `entry_id` de la source `source_id` : cache du catalogue (sans réseau)
/// ou dernier scan du dossier. Avec `rescan`, un dossier pas encore analysé dans cette session
/// (reprise au démarrage) est analysé, sous-dossiers compris.
async fn resolve_entry(
    config: &ConfigState,
    catalog: &CatalogService,
    folder: &FolderState,
    source_id: &str,
    entry_id: &str,
    rescan: bool,
) -> AppResult<StoryEntry> {
    let (source, cache_root) = {
        let _guard = config.guard();
        let source = SourcesStore::new(&config.dir).get(source_id)?;
        let cache_root =
            SettingsStore::new(&config.dir).resolved_cache_dir(&Environment::current())?;
        (source, cache_root)
    };
    match source {
        Source::Catalog { url, .. } => catalog
            .entries(&cache_root, source_id, &url)
            .await?
            .entries
            .into_iter()
            .find(|e| e.entry_id == entry_id)
            .ok_or_else(|| {
                AppError::NotFound(format!("histoire inconnue dans le catalogue : {entry_id}"))
            }),
        Source::Folder { .. } => match folder.entry(source_id, entry_id) {
            Err(_) if rescan => {
                let (root, _) = {
                    let _guard = config.guard();
                    crate::folder::resolve_folder(&config.dir, &Environment::current(), source_id)?
                };
                let (state, sid) = (folder.clone(), source_id.to_string());
                blocking(move || state.scan(&sid, &root, true)).await??;
                folder.entry(source_id, entry_id)
            }
            found => found,
        },
        Source::Library { .. } => Err(AppError::Validation(
            "cette histoire est déjà dans la bibliothèque".into(),
        )),
    }
}

/// Ré-résolution d'une entrée de la file persistée au démarrage (voir [`InstallService::restore`]).
pub async fn resolve_for_restore(
    app: &tauri::AppHandle,
    source_id: &str,
    entry_id: &str,
) -> AppResult<StoryEntry> {
    use tauri::Manager;
    resolve_entry(
        &app.state::<ConfigState>(),
        &app.state::<CatalogService>(),
        &app.state::<FolderState>(),
        source_id,
        entry_id,
        true,
    )
    .await
}

/// Lance l'installation d'une histoire d'une source (catalogue ou dossier).
#[tauri::command]
pub async fn install_start(
    config: State<'_, ConfigState>,
    catalog: State<'_, CatalogService>,
    folder: State<'_, FolderState>,
    service: State<'_, InstallService>,
    source_id: String,
    entry_id: String,
) -> AppResult<InstallTask> {
    let entry = resolve_entry(&config, &catalog, &folder, &source_id, &entry_id, false).await?;
    service.start(entry)
}

/// Annule une tâche d'installation.
#[tauri::command]
pub async fn install_cancel(service: State<'_, InstallService>, task_id: String) -> AppResult<()> {
    service.cancel(&task_id)
}

/// Tâches d'installation de la session (en cours, en attente, terminées).
#[tauri::command]
pub async fn install_list(service: State<'_, InstallService>) -> AppResult<Vec<InstallTask>> {
    Ok(service.list())
}

#[cfg(test)]
mod tests;
