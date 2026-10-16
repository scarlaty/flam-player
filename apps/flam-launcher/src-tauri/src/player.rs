//! Lancement et suivi de `flam-player.exe`.
//!
//! **Sécurité** : le front ne fournit jamais de chemin. L'histoire est désignée par son `uuid`
//! de bibliothèque ([`player_launch`]) ou par `source_id` + `entry_id` d'une source dossier déjà
//! analysée ([`player_launch_folder`]) ; le chemin du player vient des paramètres validés
//! (`settings::resolve`) et est revérifié juste avant l'exécution. Le processus est créé sans
//! shell, avec **un seul argument** (le chemin de l'histoire), passé en tableau.
//!
//! **Règle du player unique** : un seul player tourne à la fois. Un second lancement tant que
//! le premier n'est pas terminé est refusé (`player_busy`). Le créneau est libéré **avant**
//! l'émission de `player://exited` : l'interface peut relancer dès réception.
//!
//! **Fin du processus** : un thread attend le player, récupère la fin de sa sortie d'erreur
//! (dernières lignes, 8 Kio au plus, attente bornée à [`STDERR_GRACE`] après la fin du player)
//! et émet `player://exited` ; un code non nul, ou l'absence de code (processus tué), est un
//! échec dont `stderr` donne la raison (un player qui plante au démarrage échoue en quelques
//! millisecondes avec un message de chargement). Le créneau est tenu par une garde (`SlotGuard`)
//! libérée quoi qu'il arrive (échec ou panique d'un thread).
//!
//! **Copie de travail et sauvegardes** : voir [`Workspace`]. Le player n'est jamais lancé sur
//! l'original mais sur une copie dans `<cache>/play/`, et la progression (`saves/`) vit dans le
//! dossier de données `<config>/saves/<empreinte>/`, hors cache : restaurée avant le lancement,
//! rapatriée à la fin du player (même en cas d'échec). Le player est lancé avec son propre
//! dossier comme dossier courant. Les sources (fichier ordinaire pour un `.plain.pk`, dossier
//! pour un `.plain`, jamais un lien) sont vérifiées avant la copie.

use std::io::Read;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::State;

use crate::catalog::{blocking, CatalogService};
use crate::folder::{resolve_folder, FolderState};
use crate::library::Library;
use crate::settings::{self, Environment, SettingsStore};
use crate::storage::ConfigState;
use crate::story::{StoryEntry, StoryKind};
use crate::{AppError, AppResult};

/// Événement émis au démarrage du player (charge : [`PlayerSession`]).
pub const EVENT_STARTED: &str = "player://started";
/// Événement émis à la fin du player (charge : [`PlayerExited`]).
pub const EVENT_EXITED: &str = "player://exited";

/// Taille maximale de la sortie d'erreur conservée, et nombre de lignes renvoyées.
const STDERR_KEEP_BYTES: usize = 8 * 1024;
const STDERR_KEEP_LINES: usize = 20;
/// Attente maximale de la fin de stderr une fois le player terminé.
const STDERR_GRACE: Duration = Duration::from_millis(500);
/// Sous-dossier du cache des copies de travail (listé dans `cache::APP_CACHE_SUBDIRS`).
pub const PLAY_DIR: &str = "play";
/// Sauvegardes écrites par le player à côté de l'histoire.
const SAVES_DIR: &str = "saves";
/// Dossier de **données** (sous le dossier de configuration, hors cache) où vivent les
/// sauvegardes de chaque histoire : `<config>/saves/<empreinte>/`.
pub const SAVES_ROOT: &str = "saves";

/// Lancement en cours (miroir de `PlayerSession` dans `src/api/player.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlayerSession {
    /// `library` (histoire installée) ou `folder` (fichier d'une source dossier).
    pub origin: &'static str,
    /// Identifiant de l'histoire (celui de la bibliothèque, ou de l'entrée du dossier).
    pub uuid: String,
    pub title: String,
    pub pid: u32,
    /// Démarrage (secondes Unix).
    pub started_at: u64,
}

/// Charge de `player://exited` (miroir de `PlayerExited`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlayerExited {
    pub session: PlayerSession,
    /// Code de sortie ; `None` si le processus a été tué.
    pub code: Option<i32>,
    /// Vrai si le player s'est terminé avec le code 0.
    pub success: bool,
    pub duration_ms: u64,
    /// Fin de la sortie d'erreur du player (vide si rien).
    pub stderr: String,
}

/// Destination des événements (Tauri en production, enregistreur en test).
pub trait PlayerEvents: Send + Sync {
    fn started(&self, session: &PlayerSession);
    fn exited(&self, event: &PlayerExited);
}

/// Émission des événements Tauri.
pub struct TauriPlayerEvents(pub tauri::AppHandle);

impl PlayerEvents for TauriPlayerEvents {
    fn started(&self, session: &PlayerSession) {
        use tauri::Emitter;
        let _ = self.0.emit(EVENT_STARTED, session);
    }
    fn exited(&self, event: &PlayerExited) {
        use tauri::Emitter;
        let _ = self.0.emit(EVENT_EXITED, event);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Libère le créneau « player en cours » quand il est détruit : y compris si le thread d'attente
/// ne démarre pas, ou s'il paniquait.
struct SlotGuard(Arc<Mutex<Option<PlayerSession>>>);

impl Drop for SlotGuard {
    fn drop(&mut self) {
        *lock(&self.0) = None;
    }
}

/// Service de lancement (état Tauri).
#[derive(Clone)]
pub struct PlayerService {
    current: Arc<Mutex<Option<PlayerSession>>>,
    events: Arc<dyn PlayerEvents>,
}

impl PlayerService {
    pub fn new(events: Arc<dyn PlayerEvents>) -> Self {
        Self {
            current: Arc::default(),
            events,
        }
    }

    /// Player en cours d'exécution, s'il y en a un.
    pub fn status(&self) -> Option<PlayerSession> {
        lock(&self.current).clone()
    }

    /// Exécute `action` si aucun player n'est en cours, en tenant le créneau pendant toute la
    /// durée : aucun lancement ne peut démarrer en même temps. Sinon `player_busy`.
    pub fn while_idle<T>(&self, action: impl FnOnce() -> T) -> AppResult<T> {
        let slot = lock(&self.current);
        if let Some(running) = slot.as_ref() {
            return Err(AppError::PlayerBusy(format!(
                "flam-player est en cours d'exécution (« {} ») : fermez-le avant de continuer",
                running.title
            )));
        }
        let result = action();
        drop(slot);
        Ok(result)
    }

    /// Lance `exe` sur le fichier que renvoie `prepare` pour l'histoire `uuid`.
    ///
    /// `prepare` (copie de travail, voir [`stage`]) n'est appelée qu'une fois le player trouvé
    /// et le créneau libre, **sous le verrou du créneau** : elle ne peut pas nettoyer le dossier
    /// d'un player en cours. Appel bloquant (copie de fichiers) : à exécuter hors du runtime.
    ///
    /// `finish` (rapatriement des sauvegardes, voir [`Workspace::persist_saves`]) s'exécute dans
    /// le thread d'attente dès la fin du player, **avant** la libération du créneau : un nouveau
    /// lancement ne peut pas nettoyer la copie de travail pendant ce temps. Elle s'exécute aussi
    /// quand le player a échoué.
    pub fn launch(
        &self,
        exe: &Path,
        prepare: impl FnOnce() -> AppResult<PathBuf>,
        finish: impl FnOnce() + Send + 'static,
        origin: &'static str,
        uuid: &str,
        title: &str,
    ) -> AppResult<PlayerSession> {
        if !exe.is_file() {
            return Err(AppError::PlayerUnavailable(format!(
                "flam-player est introuvable : {}",
                exe.display()
            )));
        }
        let mut slot = lock(&self.current);
        if let Some(running) = slot.as_ref() {
            return Err(AppError::PlayerBusy(format!(
                "flam-player est déjà en cours d'exécution (« {} »)",
                running.title
            )));
        }
        let target = prepare()?;
        if !target.exists() {
            return Err(AppError::StoryFileMissing(format!(
                "le fichier de « {title} » n'existe plus : {}",
                target.display()
            )));
        }
        let began = Instant::now();
        let mut command = Command::new(exe);
        command
            .arg(&target)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        if let Some(dir) = exe.parent() {
            command.current_dir(dir);
        }
        let mut child = command.spawn().map_err(|err| {
            AppError::PlayerUnavailable(format!("impossible de lancer flam-player : {err}"))
        })?;
        let session = PlayerSession {
            origin,
            uuid: uuid.to_string(),
            title: title.to_string(),
            pid: child.id(),
            started_at: now_secs(),
        };
        *slot = Some(session.clone());
        let guard = SlotGuard(self.current.clone());
        drop(slot);

        // Lecteur de stderr : alimente un tampon partagé, signale sa fin en fermant `done_tx`.
        let kept: Arc<Mutex<Vec<u8>>> = Arc::default();
        let (done_tx, done) = channel::<()>();
        if let Some(stream) = child.stderr.take() {
            let kept = kept.clone();
            // Échec de création du thread : pas de stderr, sans autre conséquence.
            let _ = std::thread::Builder::new().spawn(move || {
                read_into(stream, &kept);
                drop(done_tx);
            });
        }

        // Thread d'attente ; le processus lui est confié une fois le lancement annoncé. S'il ne
        // démarre pas, le player est tué et le créneau (`guard`, détruit avec la fermeture)
        // libéré.
        let (hand_over, receive) = channel::<Child>();
        let service = self.clone();
        let waiting = session.clone();
        let waiter = std::thread::Builder::new().spawn(move || {
            let Ok(mut child) = receive.recv() else {
                return;
            };
            let status = child.wait();
            // Le player est terminé : la fin de stderr n'est attendue que brièvement (un
            // processus petit-fils peut garder le tube ouvert).
            let _ = done.recv_timeout(STDERR_GRACE);
            let (code, success, stderr) = match status {
                Ok(status) => (status.code(), status.success(), tail_text(&lock(&kept))),
                Err(err) => (None, false, format!("attente du player impossible : {err}")),
            };
            let event = PlayerExited {
                session: waiting,
                code,
                success,
                duration_ms: u64::try_from(began.elapsed().as_millis()).unwrap_or(u64::MAX),
                stderr,
            };
            tracing::info!("flam-player terminé : code {:?}", event.code);
            if std::panic::catch_unwind(AssertUnwindSafe(finish)).is_err() {
                tracing::error!("le rapatriement des sauvegardes a paniqué");
            }
            // Créneau libéré avant l'événement : le front peut relancer dès qu'il le reçoit.
            drop(guard);
            service.events.exited(&event);
        });
        if let Err(err) = waiter {
            let _ = child.kill();
            let _ = child.wait();
            return Err(AppError::PlayerUnavailable(format!(
                "impossible de surveiller flam-player : {err}"
            )));
        }
        tracing::info!("flam-player lancé (pid {}) : {title}", session.pid);
        // Un récepteur d'événement qui panique ne doit pas empêcher le suivi du processus.
        if std::panic::catch_unwind(AssertUnwindSafe(|| self.events.started(&session))).is_err() {
            tracing::error!("le gestionnaire de player://started a paniqué");
        }
        let _ = hand_over.send(child);
        Ok(session)
    }
}

/// Lit le flux jusqu'à sa fin dans `kept`, borné à [`STDERR_KEEP_BYTES`] octets (les derniers).
fn read_into(mut stream: impl Read, kept: &Mutex<Vec<u8>>) {
    let mut chunk = [0u8; 2048];
    while let Ok(n) = stream.read(&mut chunk) {
        if n == 0 {
            break;
        }
        let mut kept = lock(kept);
        kept.extend_from_slice(&chunk[..n]);
        if kept.len() > STDERR_KEEP_BYTES {
            let excess = kept.len() - STDERR_KEEP_BYTES;
            kept.drain(..excess);
        }
    }
}

/// Dernières lignes d'une sortie d'erreur.
fn tail_text(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text.lines().map(str::trim_end).collect();
    let start = lines.len().saturating_sub(STDERR_KEEP_LINES);
    lines[start..].join("\n").trim().to_string()
}

/// Vérifie, sans suivre de lien, que la source est du type attendu : fichier ordinaire pour un
/// `.plain.pk`, dossier pour un `.plain`. Un lien symbolique ou une jonction est refusé.
fn check_source(path: &Path, kind: StoryKind) -> AppResult<()> {
    let meta = std::fs::symlink_metadata(path).map_err(|_| {
        AppError::StoryFileMissing(format!(
            "le fichier de l'histoire n'existe plus : {}",
            path.display()
        ))
    })?;
    let file_type = meta.file_type();
    let (ok, expected) = match kind {
        StoryKind::FlamPlain => (file_type.is_dir(), "un dossier"),
        _ => (file_type.is_file(), "un fichier"),
    };
    if !ok {
        return Err(AppError::Validation(format!(
            "{} doit être {expected} ordinaire (ni lien, ni autre type)",
            path.display()
        )));
    }
    Ok(())
}

/// Profondeur maximale d'une copie d'arborescence.
const COPY_MAX_DEPTH: usize = 16;
/// Bornes de la copie des sauvegardes (nombre de fichiers, octets).
const SAVES_MAX_FILES: usize = 5000;
const SAVES_MAX_BYTES: u64 = 64 * 1024 * 1024;

/// Plafond d'une copie d'arborescence.
struct Budget {
    files: usize,
    bytes: u64,
}

impl Budget {
    fn unlimited() -> Budget {
        Budget {
            files: usize::MAX,
            bytes: u64::MAX,
        }
    }

    fn take(&mut self, len: u64) -> std::io::Result<()> {
        if self.files == 0 || self.bytes < len {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "arborescence trop volumineuse",
            ));
        }
        self.files -= 1;
        self.bytes -= len;
        Ok(())
    }
}

/// Copie `from` dans `to` sans suivre aucun lien (ils sont ignorés), dans la limite de `budget`.
fn copy_tree(from: &Path, to: &Path, depth: usize, budget: &mut Budget) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    if depth >= COPY_MAX_DEPTH {
        return Ok(());
    }
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let (src, dst) = (entry.path(), to.join(entry.file_name()));
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            copy_tree(&src, &dst, depth + 1, budget)?;
        } else if file_type.is_file() {
            budget.take(entry.metadata()?.len())?;
            std::fs::copy(&src, &dst)?;
        } // liens et autres : ignorés
    }
    Ok(())
}

/// Vrai si `path` est un vrai dossier (pas un lien).
fn is_real_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_dir())
}

/// Copie de travail d'une histoire et dossier de ses sauvegardes.
///
/// Le player extrait un `.plain.pk` et écrit ses sauvegardes (`saves/<nom>`) **à côté** de
/// l'histoire : on ne le lance donc jamais sur l'original (dossier de l'utilisateur ou
/// `library/`) mais sur une copie, `<play_root>/<empreinte>/<nom>`, vidée avant chaque lancement.
/// Le cache étant jetable, la progression n'y vit pas : elle est conservée dans
/// `<saves_root>/<empreinte>/` (données de l'application, hors cache), recopiée vers
/// `<copie>/saves/` avant le lancement ([`Workspace::stage`]) et rapatriée à la fin du player
/// ([`Workspace::persist_saves`]). L'empreinte est stable pour une même clé
/// (`library:<uuid>` ou `folder:<source>:<entrée>`).
#[derive(Clone)]
pub struct Workspace {
    work: PathBuf,
    saved: PathBuf,
}

impl Workspace {
    pub fn new(play_root: &Path, saves_root: &Path, key: &str) -> Self {
        let digest = Sha256::digest(key.as_bytes());
        let hash: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
        Self {
            work: play_root.join(&hash),
            saved: saves_root.join(hash),
        }
    }

    fn sibling(&self, suffix: &str) -> PathBuf {
        let mut name = self.saved.file_name().unwrap_or_default().to_os_string();
        name.push(suffix);
        self.saved.with_file_name(name)
    }

    /// Prépare la copie de travail de `source` (déjà vérifiée par [`check_source`]) : rapatrie
    /// d'abord un `saves/` resté dans la copie (échec : erreur, rien n'est supprimé), vide le
    /// dossier de travail, y copie l'histoire, y restaure `saves/`, et renvoie le chemin à lancer.
    pub fn stage(&self, source: &Path) -> AppResult<PathBuf> {
        let name = source.file_name().ok_or_else(|| {
            AppError::Validation(format!("nom d'histoire invalide : {}", source.display()))
        })?;
        std::fs::create_dir_all(&self.work)?;
        // Partie précédente non rapatriée (launcher fermé ou tué, `persist_saves` en échec) :
        // la progression est mise à l'abri avant de vider. En cas d'échec, rien n'est détruit.
        self.persist_saves()?;
        for entry in std::fs::read_dir(&self.work)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                std::fs::remove_dir_all(entry.path())?;
            } else {
                std::fs::remove_file(entry.path())?;
            }
        }
        let dest = self.work.join(name);
        if is_real_dir(source) {
            copy_tree(source, &dest, 0, &mut Budget::unlimited())?;
        } else {
            std::fs::copy(source, &dest)?;
        }
        // Reprise après un arrêt entre les deux renommages de `persist_saves`.
        let old = self.sibling(".old");
        if !self.saved.exists() && is_real_dir(&old) {
            std::fs::rename(&old, &self.saved)?;
        }
        if is_real_dir(&self.saved) {
            let mut budget = Budget {
                files: SAVES_MAX_FILES,
                bytes: SAVES_MAX_BYTES,
            };
            copy_tree(&self.saved, &self.work.join(SAVES_DIR), 0, &mut budget)?;
        }
        Ok(dest)
    }

    /// Rapatrie `<copie>/saves/` vers le dossier de données : copie dans un dossier temporaire
    /// voisin, puis échange par renommage (l'ancien est conservé jusqu'au succès). Sans
    /// sauvegarde dans la copie, ne fait rien. Sauvegardes trop volumineuses : refusées.
    pub fn persist_saves(&self) -> AppResult<()> {
        let src = self.work.join(SAVES_DIR);
        if !is_real_dir(&src) {
            return Ok(());
        }
        let (tmp, old) = (self.sibling(".tmp"), self.sibling(".old"));
        for leftover in [&tmp, &old] {
            if is_real_dir(leftover) {
                std::fs::remove_dir_all(leftover)?;
            }
        }
        let mut budget = Budget {
            files: SAVES_MAX_FILES,
            bytes: SAVES_MAX_BYTES,
        };
        if let Err(err) = copy_tree(&src, &tmp, 0, &mut budget) {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(err.into());
        }
        let had_previous = is_real_dir(&self.saved);
        if had_previous {
            std::fs::rename(&self.saved, &old)?;
        }
        if let Err(err) = std::fs::rename(&tmp, &self.saved) {
            if had_previous {
                let _ = std::fs::rename(&old, &self.saved);
            }
            return Err(err.into());
        }
        if had_previous {
            let _ = std::fs::remove_dir_all(&old);
        }
        Ok(())
    }

    /// [`persist_saves`](Self::persist_saves), l'erreur étant tracée (appelée à la fin du player).
    pub fn persist_saves_logged(&self) {
        if let Err(err) = self.persist_saves() {
            tracing::error!("sauvegardes du player non conservées : {err}");
        }
    }
}

/// Chemin du player depuis les paramètres enregistrés (configuré valide, sinon détecté).
fn player_exe(config: &ConfigState) -> AppResult<PathBuf> {
    let _guard = config.guard();
    let saved = SettingsStore::new(&config.dir).load()?;
    let (resolved, _) = settings::resolve(&saved, &Environment::current());
    resolved.player_path.map(PathBuf::from).ok_or_else(|| {
        AppError::PlayerUnavailable(
            "flam-player.exe est introuvable : indiquez son chemin dans les paramètres".into(),
        )
    })
}

/// Fichier à lancer pour une entrée de source dossier : `.plain` ou `.plain.pk` situé sous le
/// dossier de la source, du type annoncé par `kind`. Un zip Telmi doit d'abord être installé.
fn folder_target(entry: &StoryEntry, root: &Path) -> AppResult<PathBuf> {
    if entry.kind == StoryKind::TelmiZip {
        return Err(AppError::Validation(
            "cette histoire est une archive Telmi : installez-la d'abord pour la convertir".into(),
        ));
    }
    let path = PathBuf::from(&entry.download_url);
    if !path.is_absolute() || !path.starts_with(root) {
        return Err(AppError::Validation(format!(
            "fichier hors du dossier de la source : {}",
            path.display()
        )));
    }
    check_source(&path, entry.kind)?;
    Ok(path)
}

/// Lance l'histoire installée `uuid` dans `flam-player` (depuis une copie de travail).
#[tauri::command]
pub async fn player_launch(
    config: State<'_, ConfigState>,
    catalog: State<'_, CatalogService>,
    player: State<'_, PlayerService>,
    uuid: String,
) -> AppResult<PlayerSession> {
    let exe = player_exe(&config)?;
    let cache_root = {
        let _guard = config.guard();
        SettingsStore::new(&config.dir).resolved_cache_dir(&Environment::current())?
    };
    let saves_root = config.dir.join(SAVES_ROOT);
    // Le vidage du cache attend la fin de la copie.
    let _gate = catalog.cache_gate().read_owned().await;
    let service = player.inner().clone();
    blocking(move || {
        let (path, title) = Library::new(&cache_root).playable(&uuid)?;
        check_source(&path, StoryKind::FlamPk)?;
        let ws = Workspace::new(
            &cache_root.join(PLAY_DIR),
            &saves_root,
            &format!("library:{uuid}"),
        );
        let (staging, saving) = (ws.clone(), ws);
        service.launch(
            &exe,
            || staging.stage(&path),
            move || saving.persist_saves_logged(),
            "library",
            &uuid,
            &title,
        )
    })
    .await?
}

/// Lance une histoire `.plain` / `.plain.pk` du dernier scan d'une source dossier, depuis une
/// copie de travail (le dossier de l'utilisateur n'est jamais modifié).
#[tauri::command]
pub async fn player_launch_folder(
    config: State<'_, ConfigState>,
    catalog: State<'_, CatalogService>,
    folder: State<'_, FolderState>,
    player: State<'_, PlayerService>,
    source_id: String,
    entry_id: String,
) -> AppResult<PlayerSession> {
    let exe = player_exe(&config)?;
    let (root, cache_root) = {
        let _guard = config.guard();
        resolve_folder(&config.dir, &Environment::current(), &source_id)?
    };
    let entry = folder.entry(&source_id, &entry_id)?;
    let saves_root = config.dir.join(SAVES_ROOT);
    let _gate = catalog.cache_gate().read_owned().await;
    let service = player.inner().clone();
    blocking(move || {
        let path = folder_target(&entry, &root)?;
        let key = format!("folder:{source_id}:{entry_id}");
        let ws = Workspace::new(&cache_root.join(PLAY_DIR), &saves_root, &key);
        let (staging, saving) = (ws.clone(), ws);
        service.launch(
            &exe,
            || staging.stage(&path),
            move || saving.persist_saves_logged(),
            "folder",
            &entry_id,
            &entry.title,
        )
    })
    .await?
}

/// Player en cours d'exécution (`null` si aucun) : pour restaurer l'état après un rechargement.
#[tauri::command]
pub async fn player_status(player: State<'_, PlayerService>) -> AppResult<Option<PlayerSession>> {
    Ok(player.status())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{channel, Receiver, Sender};
    use std::time::Duration;

    struct Recorder {
        started: Mutex<Sender<PlayerSession>>,
        exited: Mutex<Sender<PlayerExited>>,
        panic_on_started: bool,
    }

    impl PlayerEvents for Recorder {
        fn started(&self, session: &PlayerSession) {
            assert!(!self.panic_on_started, "gestionnaire défaillant");
            let _ = lock(&self.started).send(session.clone());
        }
        fn exited(&self, event: &PlayerExited) {
            let _ = lock(&self.exited).send(event.clone());
        }
    }

    struct Ctx {
        service: PlayerService,
        started: Receiver<PlayerSession>,
        exited: Receiver<PlayerExited>,
        dir: tempfile::TempDir,
        fake: PathBuf,
    }

    /// Le faux player est l'exemple `fake_player`, construit par `cargo test`.
    fn fake_player() -> PathBuf {
        let exe = std::env::current_exe().unwrap();
        let path = exe
            .parent()
            .and_then(Path::parent)
            .unwrap()
            .join("examples")
            .join(format!("fake_player{}", std::env::consts::EXE_SUFFIX));
        assert!(
            path.is_file(),
            "faux player absent ({}) : lancer `cargo test` sans filtre de cible",
            path.display()
        );
        path
    }

    fn ctx() -> Ctx {
        ctx_with(false)
    }

    fn ctx_with(panic_on_started: bool) -> Ctx {
        let (started_tx, started) = channel();
        let (exited_tx, exited) = channel();
        let service = PlayerService::new(Arc::new(Recorder {
            started: Mutex::new(started_tx),
            exited: Mutex::new(exited_tx),
            panic_on_started,
        }));
        Ctx {
            service,
            started,
            exited,
            dir: tempfile::tempdir().unwrap(),
            fake: fake_player(),
        }
    }

    impl Ctx {
        fn story(&self, name: &str, script: &str) -> PathBuf {
            let path = self.dir.path().join(name);
            std::fs::write(&path, script).unwrap();
            path
        }

        fn launch(&self, target: &Path) -> AppResult<PlayerSession> {
            self.service.launch(
                &self.fake,
                || Ok(target.to_path_buf()),
                || {},
                "library",
                "u1",
                "Mon histoire",
            )
        }

        fn wait_exit(&self) -> PlayerExited {
            self.exited
                .recv_timeout(Duration::from_secs(20))
                .expect("player:// exited attendu")
        }
    }

    #[test]
    fn succes_code_zero_avec_evenements() {
        let c = ctx();
        let story = c.story("ok.plain.pk", "exit 0\n");
        let session = c.launch(&story).unwrap();
        assert_eq!(session.uuid, "u1");
        assert_eq!(
            c.started.recv_timeout(Duration::from_secs(5)).unwrap(),
            session
        );
        let done = c.wait_exit();
        assert_eq!(done.session, session);
        assert_eq!(done.code, Some(0));
        assert!(done.success);
        assert_eq!(done.stderr, "");
        assert_eq!(c.service.status(), None);
    }

    #[test]
    fn code_d_erreur_et_stderr_remontes() {
        let c = ctx();
        let script = "stderr première ligne\nstderr Erreur de chargement : x\nexit 7\n";
        c.launch(&c.story("ko.plain.pk", script)).unwrap();
        let done = c.wait_exit();
        assert_eq!(done.code, Some(7));
        assert!(!done.success);
        assert_eq!(done.stderr, "première ligne\nErreur de chargement : x");
    }

    #[test]
    fn stderr_est_borne_aux_dernieres_lignes() {
        let c = ctx();
        let mut script: String = (0..100).map(|i| format!("stderr ligne {i}\n")).collect();
        script.push_str("exit 1\n");
        c.launch(&c.story("long.plain.pk", &script)).unwrap();
        let done = c.wait_exit();
        let lines: Vec<&str> = done.stderr.lines().collect();
        assert_eq!(lines.len(), STDERR_KEEP_LINES);
        assert_eq!(lines.last(), Some(&"ligne 99"));
    }

    #[test]
    fn fichier_manquant_est_refuse_sans_lancer() {
        let c = ctx();
        let err = c.launch(&c.dir.path().join("absent.plain.pk")).unwrap_err();
        assert_eq!(err.kind(), "story_file_missing");
        assert_eq!(c.service.status(), None);
        assert!(c.started.try_recv().is_err());
    }

    #[test]
    fn player_manquant_est_refuse() {
        let c = ctx();
        let story = c.story("a.plain.pk", "exit 0\n");
        let err = c
            .service
            .launch(
                &c.dir.path().join("nope.exe"),
                || Ok(story),
                || {},
                "library",
                "u",
                "t",
            )
            .unwrap_err();
        assert_eq!(err.kind(), "player_unavailable");
    }

    #[test]
    fn executable_invalide_est_une_erreur_typee() {
        let c = ctx();
        let story = c.story("a.plain.pk", "exit 0\n");
        let not_exe = c.story("faux.exe", "pas un exécutable");
        let err = c
            .service
            .launch(&not_exe, || Ok(story), || {}, "library", "u", "t")
            .unwrap_err();
        assert_eq!(err.kind(), "player_unavailable");
        assert_eq!(c.service.status(), None);
    }

    #[test]
    fn double_lancement_refuse_puis_relance_possible() {
        let c = ctx();
        let slow = c.story("lent.plain.pk", "sleep 1500\nexit 0\n");
        let quick = c.story("vite.plain.pk", "exit 0\n");
        let first = c.launch(&slow).unwrap();
        assert_eq!(c.service.status(), Some(first));
        let err = c.launch(&quick).unwrap_err();
        assert_eq!(err.kind(), "player_busy");
        assert!(err.to_string().contains("Mon histoire"));
        c.wait_exit();
        // Le créneau est libre dès la réception de l'événement.
        c.launch(&quick).unwrap();
        c.wait_exit();
    }

    #[test]
    fn le_chemin_est_un_seul_argument_sans_shell() {
        let c = ctx();
        // Espaces, esperluette, parenthèses, apostrophe : jamais interprétés ni découpés.
        let story = c.story("Mon héros & (ses) amis d'ici.plain.pk", "exit 5\n");
        c.launch(&story).unwrap();
        assert_eq!(c.wait_exit().code, Some(5));
    }

    #[test]
    fn dossier_plain_lance_directement() {
        let c = ctx();
        // Un dossier existe : le faux player ne peut pas le lire (code 3), mais il est lancé.
        let dir = c.dir.path().join("Histoire.plain");
        std::fs::create_dir(&dir).unwrap();
        c.launch(&dir).unwrap();
        assert_eq!(c.wait_exit().code, Some(3));
    }

    #[test]
    fn un_gestionnaire_started_defaillant_ne_bloque_pas_le_creneau() {
        let c = ctx_with(true);
        let story = c.story("ok.plain.pk", "exit 0\n");
        // Le gestionnaire de `started` panique : le lancement réussit quand même, la fin est
        // suivie et le créneau libéré.
        c.launch(&story).unwrap();
        assert_eq!(c.wait_exit().code, Some(0));
        assert_eq!(c.service.status(), None);
    }

    #[test]
    fn la_garde_libere_le_creneau_au_drop() {
        let current: Arc<Mutex<Option<PlayerSession>>> = Arc::default();
        *lock(&current) = Some(PlayerSession {
            origin: "library",
            uuid: "u".into(),
            title: "t".into(),
            pid: 1,
            started_at: 0,
        });
        let guard = SlotGuard(current.clone());
        let panicked = std::thread::spawn(move || {
            let _guard = guard;
            panic!("thread d'attente défaillant");
        })
        .join();
        assert!(panicked.is_err());
        assert!(lock(&current).is_none());
    }

    #[test]
    fn la_fin_n_attend_pas_un_petit_fils_qui_garde_stderr() {
        let c = ctx();
        // Le player se termine, mais un processus fils garde le tube stderr ouvert 8 s.
        let story = c.story("fils.plain.pk", "stderr avant\nchild_sleep 8000\nexit 0\n");
        let began = Instant::now();
        c.launch(&story).unwrap();
        let done = c.exited.recv_timeout(Duration::from_secs(6)).unwrap();
        assert!(began.elapsed() < Duration::from_secs(6));
        assert_eq!(done.code, Some(0));
        assert_eq!(c.service.status(), None);
        assert!(done.stderr.contains("avant"), "{:?}", done.stderr);
    }

    fn workspace(dir: &Path, key: &str) -> Workspace {
        Workspace::new(&dir.join("cache/play"), &dir.join("config/saves"), key)
    }

    /// Ce que fait le player : extrait l'archive et écrit une sauvegarde à côté de la copie.
    fn play_and_save(copy: &Path, content: &[u8]) {
        let work = copy.parent().unwrap();
        std::fs::create_dir_all(work.join("Histoire.plain")).unwrap();
        std::fs::write(work.join("Histoire.plain/info.plain"), b"x").unwrap();
        std::fs::create_dir_all(work.join("saves/Histoire.plain")).unwrap();
        std::fs::write(work.join("saves/Histoire.plain/state"), content).unwrap();
    }

    #[test]
    fn stage_copie_un_pk_sans_toucher_la_source() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("utilisateur");
        std::fs::create_dir(&src).unwrap();
        let pk = src.join("Histoire.plain.pk");
        std::fs::write(&pk, b"PK").unwrap();

        let ws = workspace(dir.path(), "folder:s:1");
        let copy = ws.stage(&pk).unwrap();
        assert!(copy.starts_with(dir.path().join("cache/play")));
        assert_eq!(copy.file_name(), pk.file_name());
        assert_eq!(std::fs::read(&copy).unwrap(), b"PK");
        play_and_save(&copy, b"s");

        // Relance : extraction obsolète supprimée.
        let again = ws.stage(&pk).unwrap();
        assert_eq!(again, copy);
        assert!(!copy.parent().unwrap().join("Histoire.plain").exists());
        // Une autre entrée a son propre dossier.
        let other = workspace(dir.path(), "folder:s:2").stage(&pk).unwrap();
        assert_ne!(other.parent(), copy.parent());
        // La source n'a reçu aucun fichier.
        assert_eq!(std::fs::read_dir(&src).unwrap().count(), 1);
    }

    #[test]
    fn stage_apres_un_player_non_termine_conserve_la_progression() {
        let dir = tempfile::tempdir().unwrap();
        let pk = dir.path().join("Histoire.plain.pk");
        std::fs::write(&pk, b"PK").unwrap();
        let ws = workspace(dir.path(), "library:u1");

        let copy = ws.stage(&pk).unwrap();
        play_and_save(&copy, b"niveau 2");
        ws.persist_saves().unwrap();
        // Nouvelle partie, puis launcher tué : `persist_saves` n'est jamais appelé.
        play_and_save(&copy, b"niveau 5");

        let copy = ws.stage(&pk).unwrap();
        let restored = copy.parent().unwrap().join("saves/Histoire.plain/state");
        assert_eq!(std::fs::read(restored).unwrap(), b"niveau 5");
        let saved = dir.path().join("config/saves");
        let kept = std::fs::read_dir(&saved).unwrap().next().unwrap().unwrap();
        assert_eq!(
            std::fs::read(kept.path().join("Histoire.plain/state")).unwrap(),
            b"niveau 5"
        );
    }

    #[test]
    fn stage_ne_detruit_rien_si_le_rapatriement_echoue() {
        let dir = tempfile::tempdir().unwrap();
        let pk = dir.path().join("Histoire.plain.pk");
        std::fs::write(&pk, b"PK").unwrap();
        let ws = workspace(dir.path(), "library:u1");
        let copy = ws.stage(&pk).unwrap();
        let saves = copy.parent().unwrap().join("saves");
        std::fs::create_dir_all(&saves).unwrap();
        for i in 0..=SAVES_MAX_FILES {
            std::fs::write(saves.join(format!("{i}")), b"").unwrap();
        }
        assert!(ws.stage(&pk).is_err());
        assert!(saves.join("0").is_file(), "la copie de travail est intacte");
        assert!(copy.is_file());
    }

    #[test]
    fn while_idle_est_refuse_pendant_une_partie_et_execute_sinon() {
        let c = ctx();
        let story = c.story(
            "long.plain.pk",
            "sleep 1500
exit 0
",
        );
        assert_eq!(c.service.while_idle(|| 7).unwrap(), 7);
        c.launch(&story).unwrap();
        let mut ran = false;
        let err = c.service.while_idle(|| ran = true).unwrap_err();
        assert_eq!(err.kind(), "player_busy");
        assert!(!ran, "l'action n'est pas exécutée pendant une partie");
        c.wait_exit();
        assert_eq!(c.service.while_idle(|| 1).unwrap(), 1);
    }

    #[test]
    fn la_sauvegarde_survit_au_vidage_du_cache_et_revient_au_lancement_suivant() {
        let dir = tempfile::tempdir().unwrap();
        let pk = dir.path().join("Histoire.plain.pk");
        std::fs::write(&pk, b"PK").unwrap();
        let ws = workspace(dir.path(), "library:u1");

        let copy = ws.stage(&pk).unwrap();
        play_and_save(&copy, b"niveau 3");
        ws.persist_saves().unwrap();

        // « Vider le cache » : le dossier `play` disparaît.
        std::fs::remove_dir_all(dir.path().join("cache")).unwrap();
        assert!(dir
            .path()
            .join("config/saves")
            .read_dir()
            .unwrap()
            .next()
            .is_some());

        let copy = ws.stage(&pk).unwrap();
        let restored = copy.parent().unwrap().join("saves/Histoire.plain/state");
        assert_eq!(std::fs::read(restored).unwrap(), b"niveau 3");

        // Une partie suivante remplace la sauvegarde ; l'ancienne copie `.old` disparaît.
        play_and_save(&copy, b"niveau 4");
        ws.persist_saves().unwrap();
        let copy = ws.stage(&pk).unwrap();
        let again = copy.parent().unwrap().join("saves/Histoire.plain/state");
        assert_eq!(std::fs::read(again).unwrap(), b"niveau 4");
        let names: Vec<_> = std::fs::read_dir(dir.path().join("config/saves"))
            .unwrap()
            .collect();
        assert_eq!(names.len(), 1, "ni .tmp ni .old");
    }

    #[test]
    fn sans_sauvegarde_rien_n_est_cree_et_les_saves_trop_gros_sont_refuses() {
        let dir = tempfile::tempdir().unwrap();
        let pk = dir.path().join("Histoire.plain.pk");
        std::fs::write(&pk, b"PK").unwrap();
        let ws = workspace(dir.path(), "k");
        ws.persist_saves().unwrap(); // pas encore de copie de travail
        let copy = ws.stage(&pk).unwrap();
        ws.persist_saves().unwrap(); // pas de saves/ écrit par le player
        assert!(!dir.path().join("config").exists());

        let saves = copy.parent().unwrap().join("saves");
        std::fs::create_dir_all(&saves).unwrap();
        for i in 0..=SAVES_MAX_FILES {
            std::fs::write(saves.join(format!("{i}")), b"").unwrap();
        }
        assert!(ws.persist_saves().is_err());
        let left: Vec<_> = std::fs::read_dir(dir.path().join("config/saves"))
            .unwrap()
            .collect();
        assert!(left.is_empty(), "le dossier temporaire est nettoyé");
    }

    #[test]
    fn le_rapatriement_s_execute_a_la_fin_du_player_meme_en_echec() {
        let c = ctx();
        let story = c.story("ko.plain.pk", "exit 9\n");
        let (tx, rx) = channel::<()>();
        let tx = Mutex::new(tx);
        c.service
            .launch(
                &c.fake,
                || Ok(story),
                move || {
                    let _ = lock(&tx).send(());
                },
                "library",
                "u",
                "t",
            )
            .unwrap();
        let done = c.wait_exit();
        assert_eq!(done.code, Some(9));
        assert!(rx.try_recv().is_ok(), "finish exécuté avant l'événement");
    }

    #[test]
    fn stage_copie_un_dossier_plain_recursivement() {
        let dir = tempfile::tempdir().unwrap();
        let story = dir.path().join("H.plain");
        std::fs::create_dir_all(story.join("img")).unwrap();
        std::fs::write(story.join("info.plain"), b"t").unwrap();
        std::fs::write(story.join("img/a.lif"), b"i").unwrap();
        let copy = workspace(dir.path(), "k").stage(&story).unwrap();
        assert_eq!(std::fs::read(copy.join("info.plain")).unwrap(), b"t");
        assert_eq!(std::fs::read(copy.join("img/a.lif")).unwrap(), b"i");
    }

    #[test]
    fn check_source_exige_le_bon_type() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.plain.pk");
        std::fs::write(&file, b"x").unwrap();
        let folder = dir.path().join("b.plain");
        std::fs::create_dir(&folder).unwrap();
        assert!(check_source(&file, StoryKind::FlamPk).is_ok());
        assert!(check_source(&folder, StoryKind::FlamPlain).is_ok());
        let wrong = check_source(&folder, StoryKind::FlamPk).unwrap_err();
        assert_eq!(wrong.kind(), "validation");
        let wrong = check_source(&file, StoryKind::FlamPlain).unwrap_err();
        assert_eq!(wrong.kind(), "validation");
        let absent = check_source(&dir.path().join("x"), StoryKind::FlamPk).unwrap_err();
        assert_eq!(absent.kind(), "story_file_missing");
    }

    #[test]
    fn check_source_refuse_les_liens_symboliques() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("vrai.plain.pk");
        std::fs::write(&target, b"x").unwrap();
        let link = dir.path().join("lien.plain.pk");
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_file(&target, &link);
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&target, &link);
        if made.is_err() {
            return; // création de lien non autorisée sur ce poste (Windows sans privilège)
        }
        let err = check_source(&link, StoryKind::FlamPk).unwrap_err();
        assert_eq!(err.kind(), "validation");
    }

    /// Vérification manuelle avec le vrai player (fenêtre SDL) :
    /// `FLAM_PLAYER_EXE=... FLAM_STORY=...\x.plain.pk cargo test real_player -- --ignored`.
    /// Le player est tué (`taskkill`) après 3 s ; copier l'histoire dans un dossier jetable
    /// (le player extrait l'archive à côté d'elle).
    #[test]
    #[ignore = "lance le vrai flam-player"]
    fn real_player() {
        let exe = PathBuf::from(std::env::var("FLAM_PLAYER_EXE").unwrap());
        let story = PathBuf::from(std::env::var("FLAM_STORY").unwrap());
        let c = ctx();
        let session = c
            .service
            .launch(
                &exe,
                || {
                    Workspace::new(
                        &c.dir.path().join(PLAY_DIR),
                        &c.dir.path().join("s"),
                        "real",
                    )
                    .stage(&story)
                },
                || {},
                "library",
                "real",
                "Réel",
            )
            .unwrap();
        std::thread::sleep(Duration::from_secs(3));
        assert_eq!(
            c.service.status(),
            Some(session.clone()),
            "le player tourne"
        );
        let kill = Command::new("taskkill")
            .args(["/PID", &session.pid.to_string(), "/F"])
            .output()
            .unwrap();
        assert!(kill.status.success());
        let done = c.wait_exit();
        println!("{done:?}");
        assert!(!done.success);
        assert!(done.duration_ms >= 2500);
        assert_eq!(c.service.status(), None);
    }

    fn entry(kind: StoryKind, url: &str) -> StoryEntry {
        StoryEntry {
            source_id: "s".into(),
            entry_id: "s:1".into(),
            uuid: "u1".into(),
            version: None,
            title: "T".into(),
            description: None,
            age: None,
            category: None,
            author: None,
            voice: None,
            license: None,
            thumb_url: None,
            download_url: url.to_string(),
            size: None,
            updated_at: None,
            kind,
        }
    }

    #[test]
    fn cible_dossier_filtrage() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("src");
        std::fs::create_dir(&root).unwrap();
        let pk = root.join("a.plain.pk");
        std::fs::write(&pk, b"x").unwrap();
        let plain = root.join("b.plain");
        std::fs::create_dir(&plain).unwrap();
        let url = |p: &Path| p.to_string_lossy().into_owned();

        assert_eq!(
            folder_target(&entry(StoryKind::FlamPk, &url(&pk)), &root).unwrap(),
            pk
        );
        assert!(folder_target(&entry(StoryKind::FlamPlain, &url(&plain)), &root).is_ok());
        // Type incohérent avec l'entrée, zip Telmi, hors source, relatif.
        for (kind, path) in [
            (StoryKind::FlamPlain, url(&pk)),
            (StoryKind::FlamPk, url(&plain)),
            (StoryKind::TelmiZip, url(&pk)),
            (StoryKind::FlamPk, url(&dir.path().join("hors.plain.pk"))),
            (StoryKind::FlamPk, "a.plain.pk".to_string()),
        ] {
            let err = folder_target(&entry(kind, &path), &root).unwrap_err();
            assert_eq!(err.kind(), "validation", "{path}");
        }
        let missing = url(&root.join("z.plain.pk"));
        let gone = folder_target(&entry(StoryKind::FlamPk, &missing), &root);
        assert_eq!(gone.unwrap_err().kind(), "story_file_missing");
    }
}
