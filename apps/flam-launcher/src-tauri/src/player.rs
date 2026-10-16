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
//!
//! **Cas limites de la progression** (détail dans le README) : des sauvegardes hors limites sont
//! mises de côté en `.rejected` (une génération) sans bloquer les lancements, et signalées par
//! `PlayerExited::saves_error` ; un plantage du player laisse un `saves/` éventuellement partiel
//! qui remplace la version précédente ; une progression effacée à la main dans
//! `<config>/saves/` revient depuis `play/<empreinte>/saves` tant que la copie de travail existe ;
//! la désinstallation purge `play/<empreinte>/` et, sur demande seulement, la progression.

use std::io::Read;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, TryLockError};
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
    /// Raison pour laquelle la progression de l'enfant n'a pas pu être conservée telle quelle
    /// (sauvegardes au-delà des limites, mises de côté en `.rejected` ; rapatriement en échec),
    /// `None` si tout s'est bien passé. L'interface doit l'afficher : sans cela, l'enfant perd
    /// sa progression sans le savoir.
    pub saves_error: Option<String>,
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
    ///
    /// **Ne bloque jamais** : le créneau est aussi tenu pendant la préparation d'un lancement
    /// (copie de travail), pendant [`while_idle`](Self::while_idle) (vidage du cache, parfois
    /// long) et pendant [`while_not_playing`](Self::while_not_playing) (désinstallation, qui
    /// peut avoir lieu pendant qu'un player lit une **autre** histoire), et cette méthode est
    /// appelée depuis une commande `async` (un worker tokio ne doit pas attendre). Si le créneau
    /// est tenu, la réponse est `None` **même si un player tourne** : ce n'est pas une preuve
    /// d'absence. L'interface se fie à `player://started` / `player://exited` ; ce statut ne sert
    /// qu'à restaurer l'état après un rechargement, et la règle du player unique reste garantie
    /// par `launch` (`player_busy`), pas par ce statut.
    pub fn status(&self) -> Option<PlayerSession> {
        match self.current.try_lock() {
            Ok(slot) => slot.clone(),
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner().clone(),
            Err(TryLockError::WouldBlock) => None,
        }
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

    /// Exécute `action` sauf si le player est en train de lire l'histoire de bibliothèque
    /// `uuid` (`player_busy`) ; le créneau est tenu pendant l'action. Sert à la désinstallation :
    /// on ne supprime pas la copie de travail sous un player en cours.
    pub fn while_not_playing<T>(&self, uuid: &str, action: impl FnOnce() -> T) -> AppResult<T> {
        let slot = lock(&self.current);
        if let Some(running) = slot.as_ref() {
            if running.origin == "library" && running.uuid == uuid {
                return Err(AppError::PlayerBusy(format!(
                    "« {} » est en cours de lecture : fermez flam-player avant de la désinstaller",
                    running.title
                )));
            }
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
    /// `finish` (rapatriement des sauvegardes, voir [`Workspace::persist_saves_logged`])
    /// s'exécute dans le thread d'attente dès la fin du player, **avant** la libération du
    /// créneau : un nouveau lancement ne peut pas nettoyer la copie de travail pendant ce temps.
    /// Elle s'exécute aussi quand le player a échoué. Son résultat est le `saves_error` de
    /// `player://exited`.
    pub fn launch(
        &self,
        exe: &Path,
        prepare: impl FnOnce() -> AppResult<PathBuf>,
        finish: impl FnOnce() -> Option<String> + Send + 'static,
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
            let duration_ms = u64::try_from(began.elapsed().as_millis()).unwrap_or(u64::MAX);
            tracing::info!("flam-player terminé : code {code:?}");
            let saves_error =
                std::panic::catch_unwind(AssertUnwindSafe(finish)).unwrap_or_else(|_| {
                    tracing::error!("le rapatriement des sauvegardes a paniqué");
                    Some("le rapatriement des sauvegardes a échoué (erreur interne)".to_string())
                });
            let event = PlayerExited {
                session: waiting,
                code,
                success,
                duration_ms,
                stderr,
                saves_error,
            };
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
///
/// Vérification en CI : la **jonction** l'est (`check_source_refuse_une_jonction_vers_un_dossier`,
/// sans privilège) ; le **lien symbolique** ne l'est pas (test `#[ignore]`, la création d'un lien
/// symbolique demande le mode développeur ou l'élévation) : à lancer à la main avec
/// `cargo test check_source_refuse_les_liens_symboliques -- --ignored`.
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

/// Erreur de dépassement d'une limite de copie (profondeur, nombre de fichiers, taille) :
/// distincte d'un échec d'E/S, elle met les sauvegardes de côté au lieu de bloquer le lancement.
fn limit_error(message: String) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, LimitError(message))
}

/// Type dédié : une autre erreur `InvalidData` du système n'est pas prise pour une limite.
#[derive(Debug)]
struct LimitError(String);

impl std::fmt::Display for LimitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for LimitError {}

fn is_limit(err: &std::io::Error) -> bool {
    err.get_ref().is_some_and(|inner| inner.is::<LimitError>())
}

/// Supprime `path` (fichier, dossier, lien) sans jamais suivre un lien : un lien de dossier
/// (Windows : lien symbolique ou jonction, que `remove_file` refuse) est retiré lui-même, sans
/// toucher à sa cible. Absent : rien à faire.
fn remove_tree(path: &Path) -> std::io::Result<()> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    let file_type = meta.file_type();
    if file_type.is_symlink() {
        std::fs::remove_file(path).or_else(|_| std::fs::remove_dir(path))
    } else if file_type.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

/// Plafond d'une copie d'arborescence.
struct Budget {
    files: usize,
    bytes: u64,
    max_depth: usize,
}

impl Budget {
    /// Copie d'une histoire : ni nombre ni taille plafonnés, profondeur bornée.
    fn unlimited() -> Budget {
        Budget {
            files: usize::MAX,
            bytes: u64::MAX,
            max_depth: COPY_MAX_DEPTH,
        }
    }

    /// Déplacement par copie d'un dossier déjà refusé (mise de côté) : aucune limite, y compris
    /// la profondeur (la longueur des chemins du système la borne).
    fn unbounded() -> Budget {
        Budget {
            files: usize::MAX,
            bytes: u64::MAX,
            max_depth: usize::MAX,
        }
    }

    /// Sauvegardes de l'enfant : nombre, taille et profondeur plafonnés.
    fn saves() -> Budget {
        Budget {
            files: SAVES_MAX_FILES,
            bytes: SAVES_MAX_BYTES,
            max_depth: COPY_MAX_DEPTH,
        }
    }

    fn take(&mut self, len: u64) -> std::io::Result<()> {
        if self.files == 0 || self.bytes < len {
            return Err(limit_error(
                "arborescence trop volumineuse (nombre de fichiers ou taille)".to_string(),
            ));
        }
        self.files -= 1;
        self.bytes -= len;
        Ok(())
    }
}

/// Copie `from` dans `to` dans la limite de `budget`, **sans aucun lien suivi** : liens
/// symboliques, jonctions et autres types d'entrée sont **ignorés** (ni copiés, ni erreur).
/// Le parcours est **itératif** (pile explicite sur le tas) : une arborescence très profonde ne
/// peut pas faire déborder la pile d'un thread bloquant, même sans limite de profondeur
/// ([`Budget::unbounded`]).
fn copy_tree(from: &Path, to: &Path, depth: usize, budget: &mut Budget) -> std::io::Result<()> {
    let mut pending = vec![(from.to_path_buf(), to.to_path_buf(), depth)];
    while let Some((from, to, depth)) = pending.pop() {
        std::fs::create_dir_all(&to)?;
        for entry in std::fs::read_dir(&from)? {
            let entry = entry?;
            let (src, dst) = (entry.path(), to.join(entry.file_name()));
            let file_type = entry.file_type()?;
            if depth >= budget.max_depth && (file_type.is_dir() || file_type.is_file()) {
                // Jamais de troncature silencieuse : une copie incomplète passerait pour complète.
                return Err(limit_error(format!(
                    "arborescence trop profonde (plus de {} niveaux)",
                    budget.max_depth
                )));
            }
            if file_type.is_dir() {
                pending.push((src, dst, depth + 1));
            } else if file_type.is_file() {
                budget.take(entry.metadata()?.len())?;
                std::fs::copy(&src, &dst)?;
            } // liens et autres : ignorés
        }
    }
    Ok(())
}

/// Déplace `from` vers `to` : renommage, sinon (autre volume, verrou) copie sans aucune limite
/// puis suppression de la source **seulement après réussite de la copie**. En cas d'échec, la
/// source est intacte et la copie partielle supprimée.
///
/// **Liens** : le repli par copie les **ignore** (ils ne sont pas copiés), puis la suppression de
/// la source les retire eux-mêmes, sans toucher à leur cible. Un lien (symbolique ou jonction)
/// présent dans le dossier déplacé est donc perdu avec le repli, alors qu'un renommage le
/// conserverait : sans conséquence pour des sauvegardes, qui n'en contiennent pas.
fn move_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(err) => {
            tracing::warn!("renommage impossible ({err}) : copie");
            copy_then_remove(from, to, remove_tree)
        }
    }
}

/// Repli de [`move_dir`], avec l'effacement de la source injecté (tests de panne).
///
/// Copie dans `<to>.part` (partiel si l'on s'arrête en route), puis renommage atomique : `to`
/// n'existe que complet. La source n'est effacée qu'ensuite, **sous le marqueur voisin
/// `<from>.moved`** (écrit avant le premier effacement, retiré après le dernier) : si
/// l'effacement s'arrête en route, la source partielle restante est reconnue comme un reste
/// ([`finish_moved`]) et ne peut plus être lue comme des sauvegardes (#91). Erreur renvoyée si
/// l'effacement de la source échoue ; la copie complète reste à destination. Seul l'effacement
/// final du marqueur en échec n'est pas une erreur (journalisé, repris par [`finish_moved`]).
fn copy_then_remove(
    from: &Path,
    to: &Path,
    remove_source: impl FnOnce(&Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    copy_then_remove_with(from, to, remove_source, remove_tree)
}

/// [`copy_then_remove`] avec l'effacement du marqueur injecté (tests de panne).
fn copy_then_remove_with(
    from: &Path,
    to: &Path,
    remove_source: impl FnOnce(&Path) -> std::io::Result<()>,
    remove_marker: impl FnOnce(&Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let mut name = to.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    let part = to.with_file_name(name);
    remove_tree(&part)?;
    if let Err(copy_err) = copy_tree(from, &part, 0, &mut Budget::unbounded()) {
        let _ = remove_tree(&part);
        return Err(copy_err);
    }
    if let Err(rename_err) = std::fs::rename(&part, to) {
        let _ = remove_tree(&part);
        return Err(rename_err);
    }
    let marker = moved_marker(from);
    std::fs::write(&marker, b"")?;
    remove_source(from)?;
    // Tout est complet (copie publiée, source effacée) : un marqueur resté n'est qu'un reste,
    // effacé par le prochain [`finish_moved`] (#100). Pas d'erreur.
    if let Err(err) = remove_marker(&marker) {
        tracing::warn!(
            "marqueur {} non effacé ({err}) : repris au prochain lancement",
            marker.display()
        );
    }
    Ok(())
}

/// Marqueur d'une source dont la copie complète est publiée : `<from>.moved` (voisin, hors de
/// `from`, pour que l'ordre d'effacement de `from` ne puisse pas le retirer avant la fin).
fn moved_marker(from: &Path) -> PathBuf {
    let mut name = from.file_name().unwrap_or_default().to_os_string();
    name.push(".moved");
    from.with_file_name(name)
}

/// Finit un déplacement interrompu pendant l'effacement de sa source : si `<dir>.moved` existe,
/// `dir` (complet, partiel, vide ou absent) n'est qu'un reste dont la copie complète est déjà
/// publiée ; il est effacé, puis le marqueur. Échec : erreur d'une ligne nommant le chemin en
/// cause (le marqueur illisible, ou `dir` non effacé avec la marche à suivre), et rien n'est lu
/// de `dir`.
fn finish_moved(dir: &Path) -> std::io::Result<()> {
    finish_moved_with(dir, |p: &Path| std::fs::symlink_metadata(p), remove_tree)
}

/// [`finish_moved`] avec la lecture du marqueur et l'effacement injectés (tests de panne).
fn finish_moved_with(
    dir: &Path,
    probe: impl Fn(&Path) -> std::io::Result<std::fs::Metadata>,
    remove: impl Fn(&Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let marker = moved_marker(dir);
    match probe(&marker) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(std::io::Error::new(
                err.kind(),
                format!(
                    "impossible de lire le marqueur {} : {err}",
                    marker.display()
                ),
            ))
        }
        Ok(_) => {}
    }
    let finish = || {
        remove(dir)?;
        tracing::warn!(
            "reste d'une mise de côté interrompue effacé : {} (copie complète déjà publiée)",
            dir.display()
        );
        remove(&marker)
    };
    finish().map_err(|err| {
        std::io::Error::new(
            err.kind(),
            format!(
                "reste d'une mise de côté interrompue non effacé ({}) : {err} ; fermer ce qui \
                 utilise le dossier, puis réessayer",
                dir.display()
            ),
        )
    })
}

/// Remplace `rejected` (ancienne génération, supprimée ici) par `fresh` (complet) : renommage.
fn promote_rejected(fresh: &Path, rejected: &Path) -> std::io::Result<()> {
    remove_tree(rejected)?;
    std::fs::rename(fresh, rejected)
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
    /// Informations sur la progression (sauvegardes mises de côté, etc.), partagées entre les
    /// clones : recueillies par [`persist_saves_logged`](Self::persist_saves_logged) pour le
    /// `saves_error` de `player://exited`.
    notes: Arc<Mutex<Vec<String>>>,
}

impl Workspace {
    pub fn new(play_root: &Path, saves_root: &Path, key: &str) -> Self {
        let digest = Sha256::digest(key.as_bytes());
        let hash: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
        Self {
            work: play_root.join(&hash),
            saved: saves_root.join(hash),
            notes: Arc::default(),
        }
    }

    fn sibling(&self, suffix: &str) -> PathBuf {
        let mut name = self.saved.file_name().unwrap_or_default().to_os_string();
        name.push(suffix);
        self.saved.with_file_name(name)
    }

    fn note(&self, text: String) {
        lock(&self.notes).push(text);
    }

    fn take_notes(&self) -> Vec<String> {
        std::mem::take(&mut *lock(&self.notes))
    }

    /// Erreur d'un lancement qui a échoué après [`stage`](Self::stage) (spawn en échec, fichier
    /// disparu) : les informations recueillies sur la progression (sauvegardes mises de côté)
    /// n'iront pas à `player://exited` ; elles sont ajoutées au message de l'erreur.
    fn launch_failure(&self, err: AppError) -> AppError {
        let notes = self.take_notes();
        if notes.is_empty() {
            err
        } else {
            err.with_note(&notes.join(" ; "))
        }
    }

    /// Met de côté un `saves/` hors limites : `<saved>.rejected`, **une seule génération** (la
    /// précédente est remplacée). Le dossier est déplacé (renommage) ; si le renommage échoue
    /// (autre volume, verrou), il est copié (sans plafond) puis la source n'est supprimée
    /// qu'après réussite de la copie : **jamais de suppression sans copie**.
    ///
    /// Étapes : déplacement vers `.rejected.new` (renommage ; ou copie dans
    /// `.rejected.new.part`, partiel si le processus s'arrête en route, puis renommage atomique :
    /// `.rejected.new` est toujours **complet**), puis remplacement de `.rejected` par
    /// `.rejected.new`.
    /// - Échec **avant** la fin du déplacement : erreur d'E/S, la source reste intacte, l'ancien
    ///   `.rejected` aussi.
    /// - Échec **après** (la source a déjà quitté sa place, ses données sont dans
    ///   `.rejected.new`) : **pas d'erreur**, une note l'indique à l'utilisateur ; la réparation
    ///   se fait au prochain lancement ou rapatriement ([`recover_rejected`](Self::recover_rejected)).
    /// - Échec **pendant l'effacement de la source** (repli par copie) : erreur ; la source
    ///   partielle est marquée `<source>.moved` et seulement effacée ensuite ([`finish_moved`],
    ///   au début de [`persist_saves`](Self::persist_saves)), jamais rapatriée ni remise de côté.
    ///
    /// Renvoie le message destiné à l'utilisateur.
    fn set_aside(&self, dir: &Path, why: &std::io::Error, what: &str) -> AppResult<String> {
        self.set_aside_with(dir, why, what, move_dir, promote_rejected)
    }

    /// [`set_aside`](Self::set_aside) avec le déplacement et l'étape de remplacement injectés
    /// (tests de panne).
    fn set_aside_with(
        &self,
        dir: &Path,
        why: &std::io::Error,
        what: &str,
        move_to: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
        promote: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
    ) -> AppResult<String> {
        let rejected = self.sibling(".rejected");
        let fresh = self.sibling(".rejected.new");
        if let Some(parent) = rejected.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Un `.rejected.new` resté d'un échec précédent est d'abord réparé : il ne doit jamais
        // être écrasé par une nouvelle mise de côté.
        self.recover_rejected()?;
        // Après ce déplacement, `.rejected.new` est complet (renommage, ou copie terminée puis
        // renommée) : c'est un point de non-retour, sans suppression possible de ce dossier.
        move_to(dir, &fresh)?;
        let note = match promote(&fresh, &rejected) {
            Ok(()) => format!(
                "{what} refusées ({why}) : mises de côté dans {}",
                rejected.display()
            ),
            Err(err) => format!(
                "{what} refusées ({why}) : mises de côté dans {} (le dossier {} n'a pas pu être \
                 remplacé : {err} ; repris au prochain lancement)",
                fresh.display(),
                rejected.display()
            ),
        };
        tracing::warn!("sauvegardes du player : {note}");
        Ok(note)
    }

    /// Répare une mise de côté interrompue : un `.rejected.new` (toujours complet) est promu en
    /// `.rejected`, l'ancienne génération étant remplacée ; un `.rejected.new.part` (copie partielle)
    /// est supprimé, sa source n'ayant jamais été retirée avant la fin de la copie.
    fn recover_rejected(&self) -> AppResult<()> {
        let (rejected, fresh) = (self.sibling(".rejected"), self.sibling(".rejected.new"));
        remove_tree(&self.sibling(".rejected.new.part"))?;
        if is_real_dir(&fresh) {
            promote_rejected(&fresh, &rejected)?;
            tracing::warn!(
                "sauvegardes mises de côté réparées : {} promu en {}",
                fresh.display(),
                rejected.display()
            );
        }
        Ok(())
    }

    /// Prépare la copie de travail de `source` (déjà vérifiée par [`check_source`]) : rapatrie
    /// d'abord un `saves/` resté dans la copie (échec d'E/S : erreur, rien n'est supprimé), vide
    /// le dossier de travail, y copie l'histoire, y restaure `saves/`, et renvoie le chemin à
    /// lancer.
    ///
    /// Des sauvegardes hors limites (nombre, taille, profondeur) ne bloquent **pas** le
    /// lancement : elles sont mises de côté en `.rejected` ([`set_aside`](Self::set_aside)) et
    /// l'information est transmise à `player://exited` (`saves_error`).
    pub fn stage(&self, source: &Path) -> AppResult<PathBuf> {
        let name = source.file_name().ok_or_else(|| {
            AppError::Validation(format!("nom d'histoire invalide : {}", source.display()))
        })?;
        std::fs::create_dir_all(&self.work)?;
        // Partie précédente non rapatriée (launcher fermé ou tué, `persist_saves` en échec) :
        // la progression est mise à l'abri avant de vider. En cas d'échec, rien n'est détruit.
        if let Some(note) = self.persist_saves()? {
            self.note(note);
        }
        for entry in std::fs::read_dir(&self.work)? {
            remove_tree(&entry?.path())?;
        }
        let dest = self.work.join(name);
        if is_real_dir(source) {
            copy_tree(source, &dest, 0, &mut Budget::unlimited())?;
        } else {
            std::fs::copy(source, &dest)?;
        }
        if is_real_dir(&self.saved) {
            // Restauration dans un dossier temporaire, renommé seulement en cas de succès : un
            // `saves/` partiel ne doit jamais pouvoir être pris pour une partie à rapatrier.
            let mut budget = Budget::saves();
            let tmp = self.work.join(format!("{SAVES_DIR}.tmp"));
            let restored = copy_tree(&self.saved, &tmp, 0, &mut budget)
                .and_then(|()| std::fs::rename(&tmp, self.work.join(SAVES_DIR)));
            if let Err(err) = restored {
                let _ = std::fs::remove_dir_all(&tmp);
                if !is_limit(&err) {
                    return Err(err.into());
                }
                // Sauvegarde conservée trop grosse (limites changées, dossier modifié à la main) :
                // mise de côté, partie sans progression, sans bloquer le lancement.
                let note = self.set_aside(&self.saved, &err, "sauvegardes conservées")?;
                self.note(note);
            }
        }
        Ok(dest)
    }

    /// Rapatrie `<copie>/saves/` vers le dossier de données : copie dans un dossier temporaire
    /// voisin, puis échange par renommage (l'ancien est conservé jusqu'au succès). Sans
    /// sauvegarde dans la copie, ne fait rien (`Ok(None)`).
    ///
    /// Sauvegardes hors limites : la **version précédente est conservée**, le nouveau `saves/`
    /// est mis de côté en `.rejected` et le message est renvoyé (`Ok(Some(..))`). Les autres
    /// échecs (E/S) sont des erreurs, la copie de travail restant intacte.
    ///
    /// **Compromis (player qui plante)** : le rapatriement a lieu aussi après un échec du
    /// player ; un `saves/` partiel (écriture interrompue par le plantage) remplace donc la
    /// version précédente, qui n'est pas gardée au-delà de l'échange.
    pub fn persist_saves(&self) -> AppResult<Option<String>> {
        // Reste d'une mise de côté interrompue pendant l'effacement de sa source (#91) : effacé
        // avant toute lecture ; échec : erreur, rien n'est lu (ni rapatrié, ni restauré).
        finish_moved(&self.saved)?;
        finish_moved(&self.work.join(SAVES_DIR))?;
        // Mise de côté interrompue (lancement ou rapatriement précédent) : réparée sans bloquer ;
        // la prochaine mise de côté la répare de toute façon, ou échoue proprement.
        if let Err(err) = self.recover_rejected() {
            tracing::warn!("réparation de .rejected impossible : {err}");
        }
        // Reprise après un arrêt entre les deux renommages d'un rapatriement précédent.
        let old = self.sibling(".old");
        if !self.saved.exists() && is_real_dir(&old) {
            std::fs::rename(&old, &self.saved)?;
        }
        let src = self.work.join(SAVES_DIR);
        if !is_real_dir(&src) {
            return Ok(None);
        }
        let (tmp, old) = (self.sibling(".tmp"), self.sibling(".old"));
        for leftover in [&tmp, &old] {
            if is_real_dir(leftover) {
                std::fs::remove_dir_all(leftover)?;
            }
        }
        let mut budget = Budget::saves();
        if let Err(err) = copy_tree(&src, &tmp, 0, &mut budget) {
            let _ = std::fs::remove_dir_all(&tmp);
            if is_limit(&err) {
                let note = self.set_aside(&src, &err, "nouvelles sauvegardes")?;
                return Ok(Some(note));
            }
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
        Ok(None)
    }

    /// Rapatriement de fin de partie : les erreurs sont tracées et, avec les informations
    /// recueillies (sauvegardes mises de côté au lancement ou ici), renvoyées pour
    /// `PlayerExited::saves_error`.
    pub fn persist_saves_logged(&self) -> Option<String> {
        let mut notes = self.take_notes();
        match self.persist_saves() {
            Ok(note) => notes.extend(note),
            Err(err) => {
                tracing::error!("sauvegardes du player non conservées : {err}");
                notes.push(format!("sauvegardes du player non conservées : {err}"));
            }
        }
        (!notes.is_empty()).then(|| notes.join(" ; "))
    }

    /// Désinstallation de l'histoire : supprime la copie de travail et, si `purge_saves`, la
    /// progression de l'enfant (`<config>/saves/<empreinte>` et ses `.tmp`, `.old`,
    /// `.rejected`). **Sans `purge_saves`, la progression est conservée** (rapatriée d'abord si
    /// la copie de travail en porte une non conservée : en cas d'échec, rien n'est supprimé).
    pub fn purge(&self, purge_saves: bool) -> AppResult<()> {
        if !purge_saves {
            self.persist_saves()?;
        }
        remove_tree(&self.work)?;
        if purge_saves {
            for suffix in [
                "",
                ".tmp",
                ".old",
                ".rejected",
                ".rejected.new",
                ".rejected.new.part",
                ".moved",
            ] {
                remove_tree(&self.sibling(suffix))?;
            }
        }
        Ok(())
    }
}

/// Désinstallation de l'histoire de bibliothèque `uuid` : purge sa copie de travail
/// `<cache>/play/<empreinte>/` et, si `purge_saves`, sa progression (voir [`Workspace::purge`]).
pub fn purge_library_story(
    cache_root: &Path,
    saves_root: &Path,
    uuid: &str,
    purge_saves: bool,
) -> AppResult<()> {
    Workspace::new(&cache_root.join(PLAY_DIR), saves_root, &library_key(uuid)).purge(purge_saves)
}

/// Clé de la copie de travail et des sauvegardes d'une histoire de bibliothèque.
fn library_key(uuid: &str) -> String {
    format!("library:{uuid}")
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
        let ws = Workspace::new(&cache_root.join(PLAY_DIR), &saves_root, &library_key(&uuid));
        let (staging, saving, failed) = (ws.clone(), ws.clone(), ws);
        let library = Library::new(&cache_root);
        service
            .launch(
                &exe,
                || {
                    // Relu sous le créneau : une désinstallation qui vient de passer (elle tient le
                    // même créneau) fait échouer le lancement (`not_found`) au lieu de lancer un
                    // fichier supprimé ou de recréer une copie de travail.
                    let (current, _) = library.playable(&uuid)?;
                    check_source(&current, StoryKind::FlamPk)?;
                    staging.stage(&current)
                },
                move || saving.persist_saves_logged(),
                "library",
                &uuid,
                &title,
            )
            .map_err(|err| failed.launch_failure(err))
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
        let (staging, saving, failed) = (ws.clone(), ws.clone(), ws);
        service
            .launch(
                &exe,
                || staging.stage(&path),
                move || saving.persist_saves_logged(),
                "folder",
                &entry_id,
                &entry.title,
            )
            .map_err(|err| failed.launch_failure(err))
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
                || None,
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
        assert_eq!(done.saves_error, None);
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
                || None,
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
            .launch(&not_exe, || Ok(story), || None, "library", "u", "t")
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
    fn stage_ne_detruit_rien_si_le_rapatriement_echoue_pour_une_raison_d_e_s() {
        let dir = tempfile::tempdir().unwrap();
        let pk = dir.path().join("Histoire.plain.pk");
        std::fs::write(&pk, b"PK").unwrap();
        let ws = workspace(dir.path(), "library:u1");
        let copy = ws.stage(&pk).unwrap();
        play_and_save(&copy, b"niveau 2");
        // `<config>/saves` est un fichier : le rapatriement échoue (E/S, pas une limite).
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        std::fs::write(dir.path().join("config/saves"), b"x").unwrap();
        assert!(ws.stage(&pk).is_err());
        let kept = copy.parent().unwrap().join("saves/Histoire.plain/state");
        assert_eq!(std::fs::read(kept).unwrap(), b"niveau 2");
        assert!(copy.is_file());
    }

    #[test]
    fn des_saves_hors_limites_ne_bloquent_pas_le_lancement_et_sont_mis_de_cote() {
        let dir = tempfile::tempdir().unwrap();
        let pk = dir.path().join("Histoire.plain.pk");
        std::fs::write(&pk, b"PK").unwrap();
        let ws = workspace(dir.path(), "library:u1");
        let copy = ws.stage(&pk).unwrap();
        play_and_save(&copy, b"niveau 2");
        assert_eq!(ws.persist_saves().unwrap(), None);

        // Partie suivante : sauvegardes au-delà des limites.
        let saves = copy.parent().unwrap().join("saves");
        for i in 0..=SAVES_MAX_FILES {
            std::fs::write(saves.join(format!("{i}")), b"").unwrap();
        }
        let note = ws.persist_saves().unwrap().expect("information renvoyée");
        assert!(note.contains("refusées"), "{note}");
        // La version précédente est conservée, le nouveau saves/ est mis de côté.
        assert_eq!(
            std::fs::read(ws.saved.join("Histoire.plain/state")).unwrap(),
            b"niveau 2"
        );
        assert!(ws.sibling(".rejected").join("0").is_file());
        assert!(
            !saves.exists(),
            "plus de saves/ à rapatrier (pas de boucle)"
        );
        assert_eq!(ws.persist_saves().unwrap(), None);

        // Le lancement suivant n'est pas bloqué et restaure la version précédente.
        let copy = ws.stage(&pk).unwrap();
        let restored = copy.parent().unwrap().join("saves/Histoire.plain/state");
        assert_eq!(std::fs::read(restored).unwrap(), b"niveau 2");

        // Une seule génération : un second rejet remplace le premier.
        let saves = copy.parent().unwrap().join("saves");
        for i in 0..=SAVES_MAX_FILES {
            std::fs::write(saves.join(format!("n{i}")), b"").unwrap();
        }
        ws.persist_saves().unwrap().unwrap();
        assert!(ws.sibling(".rejected").join("n0").is_file());
        assert!(!ws.sibling(".rejected").join("0").exists());
    }

    #[test]
    fn un_lancement_en_echec_apres_stage_renvoie_les_notes_dans_l_erreur() {
        let dir = tempfile::tempdir().unwrap();
        let pk = dir.path().join("Histoire.plain.pk");
        std::fs::write(&pk, b"PK").unwrap();
        let ws = workspace(dir.path(), "library:u1");
        let copy = ws.stage(&pk).unwrap();
        play_and_save(&copy, b"niveau 2");
        // Partie terminée avec trop de sauvegardes, non rapatriées : le lancement suivant les
        // met de côté (note) avant de copier l'histoire.
        let saves = copy.parent().unwrap().join("saves");
        for i in 0..=SAVES_MAX_FILES {
            std::fs::write(saves.join(format!("{i}")), b"").unwrap();
        }

        // Lancement réel qui échoue après `stage` : le fichier de l'histoire disparaît.
        let c = ctx();
        let (staging, failed) = (ws.clone(), ws);
        let err = c
            .service
            .launch(
                &c.fake,
                || {
                    let copy = staging.stage(&pk)?;
                    std::fs::remove_file(&copy)?;
                    Ok(copy)
                },
                || None,
                "library",
                "u1",
                "Mon histoire",
            )
            .map_err(|err| failed.launch_failure(err))
            .unwrap_err();
        assert_eq!(err.kind(), "story_file_missing");
        assert!(
            err.to_string().contains("nouvelles sauvegardes refusées"),
            "{err}"
        );
        // Les notes sont consommées : une seconde erreur n'est pas complétée.
        let again = failed.launch_failure(AppError::StoryFileMissing("absent.".into()));
        assert_eq!(again.to_string(), "absent.");
    }

    #[test]
    fn un_saves_trop_profond_est_une_erreur_explicite_pas_une_troncature() {
        let dir = tempfile::tempdir().unwrap();
        let pk = dir.path().join("Histoire.plain.pk");
        std::fs::write(&pk, b"PK").unwrap();
        let ws = workspace(dir.path(), "library:u1");
        let copy = ws.stage(&pk).unwrap();
        let mut deep = copy.parent().unwrap().join("saves");
        for _ in 0..=COPY_MAX_DEPTH {
            deep.push("n");
        }
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("state"), b"profond").unwrap();
        let note = ws.persist_saves().unwrap().expect("refus explicite");
        assert!(note.contains("trop profonde"), "{note}");
        assert!(!ws.saved.exists(), "rien de tronqué n'est conservé");
        // copy_tree : erreur de limite, jamais Ok tronqué.
        let target = dir.path().join("copie");
        let mut budget = Budget::unlimited();
        std::fs::create_dir_all(dir.path().join("a")).unwrap();
        let mut nested = dir.path().join("a");
        for _ in 0..=COPY_MAX_DEPTH {
            nested.push("d");
        }
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("f"), b"x").unwrap();
        let err = copy_tree(&dir.path().join("a"), &target, 0, &mut budget).unwrap_err();
        assert!(is_limit(&err));
    }

    #[test]
    fn le_repli_par_copie_deplace_meme_un_arbre_trop_profond() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("saves");
        let mut deep = from.clone();
        for _ in 0..(COPY_MAX_DEPTH + 4) {
            deep.push("n");
        }
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("state"), b"profond").unwrap();
        // Le repli de `move_dir` (sans renommage) copie sans aucune limite, puis supprime.
        let to = dir.path().join("rejected");
        copy_tree(&from, &to, 0, &mut Budget::unbounded()).unwrap();
        assert_eq!(
            std::fs::read(to.join("n/".repeat(COPY_MAX_DEPTH + 4)).join("state")).unwrap(),
            b"profond"
        );
        // Avec la limite de profondeur, la même copie est refusée.
        let err = copy_tree(&from, &dir.path().join("x"), 0, &mut Budget::saves()).unwrap_err();
        assert!(is_limit(&err));
        // Déplacement complet : source disparue, destination complète.
        let moved = dir.path().join("moved");
        move_dir(&from, &moved).unwrap();
        assert!(!from.exists());
        assert!(moved.exists());
    }

    #[test]
    fn l_ancien_rejected_survit_si_la_mise_de_cote_echoue() {
        let dir = tempfile::tempdir().unwrap();
        let pk = dir.path().join("Histoire.plain.pk");
        std::fs::write(&pk, b"PK").unwrap();
        let ws = workspace(dir.path(), "library:u1");
        let copy = ws.stage(&pk).unwrap();
        std::fs::create_dir_all(ws.sibling(".rejected")).unwrap();
        std::fs::write(ws.sibling(".rejected").join("ancien"), b"x").unwrap();
        // Ici l'échec survient **avant** que le nouveau contenu soit en sécurité : la source à
        // mettre de côté n'existe pas, donc le déplacement vers `.rejected.new` échoue. Rien ne
        // doit alors toucher à l'ancien `.rejected` (il n'est remplacé qu'une fois le nouveau
        // contenu complet dans `.rejected.new`, voir `set_aside_apres_deplacement_*`).
        let missing = copy.parent().unwrap().join("saves-absent");
        let why = limit_error("x".into());
        assert!(ws.set_aside(&missing, &why, "test").is_err());
        assert!(ws.sibling(".rejected").join("ancien").is_file());
        assert!(!ws.sibling(".rejected.new").exists());
    }

    #[test]
    fn set_aside_apres_deplacement_renvoie_une_note_puis_se_repare() {
        let dir = tempfile::tempdir().unwrap();
        let pk = dir.path().join("Histoire.plain.pk");
        std::fs::write(&pk, b"PK").unwrap();
        let ws = workspace(dir.path(), "library:u1");
        let copy = ws.stage(&pk).unwrap();
        let saves = copy.parent().unwrap().join("saves");
        std::fs::create_dir_all(&saves).unwrap();
        std::fs::write(saves.join("0"), b"progression").unwrap();
        std::fs::create_dir_all(ws.sibling(".rejected")).unwrap();
        std::fs::write(ws.sibling(".rejected").join("ancien"), b"x").unwrap();
        // Le remplacement de `.rejected` échoue APRÈS le déplacement : pas d'erreur, une note, et
        // la progression est intacte dans `.rejected.new`.
        let why = limit_error("trop gros".into());
        let note = ws
            .set_aside_with(&saves, &why, "nouvelles sauvegardes", move_dir, |_, _| {
                Err(std::io::Error::other("verrou"))
            })
            .expect("une note, pas une erreur");
        // #91 : texte exact, une seule ligne, sans espaces doublés.
        assert_eq!(
            note,
            format!(
                "nouvelles sauvegardes refusées (trop gros) : mises de côté dans {} (le dossier {} \
                 n'a pas pu être remplacé : verrou ; repris au prochain lancement)",
                ws.sibling(".rejected.new").display(),
                ws.sibling(".rejected").display()
            )
        );
        assert!(!note.contains('\n') && !note.contains("  "), "{note:?}");
        assert!(!saves.exists(), "la source a bien été déplacée");
        assert_eq!(
            std::fs::read(ws.sibling(".rejected.new").join("0")).unwrap(),
            b"progression"
        );
        assert!(ws.sibling(".rejected").join("ancien").is_file());
        // Au rapatriement ou lancement suivant : `.rejected.new` est promu, l'ancien remplacé.
        assert!(ws.persist_saves().unwrap().is_none());
        assert_eq!(
            std::fs::read(ws.sibling(".rejected").join("0")).unwrap(),
            b"progression"
        );
        assert!(!ws.sibling(".rejected").join("ancien").exists());
        assert!(!ws.sibling(".rejected.new").exists());
    }

    /// Effacement de source injecté : efface `removed` fichiers de la source, puis échoue
    /// (panne à mi-chemin de `remove_tree`, #91).
    fn half_remove(removed: usize) -> impl FnOnce(&Path) -> std::io::Result<()> {
        move |from: &Path| {
            assert!(
                moved_marker(from).is_file(),
                "marqueur écrit avant l'effacement"
            );
            let mut names: Vec<_> = std::fs::read_dir(from)
                .unwrap()
                .map(|e| e.unwrap().path())
                .collect();
            names.sort();
            for path in names.into_iter().take(removed) {
                std::fs::remove_file(path).unwrap();
            }
            Err(std::io::Error::other("panne pendant l'effacement"))
        }
    }

    fn write_files(dir: &Path, names: &[&str]) {
        std::fs::create_dir_all(dir).unwrap();
        for name in names {
            std::fs::write(dir.join(name), name.as_bytes()).unwrap();
        }
    }

    fn assert_files(dir: &Path, names: &[&str]) {
        let mut found: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        found.sort();
        assert_eq!(found, names, "{}", dir.display());
        for name in names {
            assert_eq!(std::fs::read(dir.join(name)).unwrap(), name.as_bytes());
        }
    }

    /// #91, double panne au rapatriement : la copie complète est publiée en `.rejected.new`, puis
    /// l'effacement de `<copie>/saves` échoue à mi-chemin (après 0, 1 ou tous les fichiers). La
    /// source restante ne doit être ni rapatriée (elle tiendrait désormais dans les limites et
    /// écraserait la progression précédente) ni remise de côté (elle remplacerait la copie
    /// complète).
    #[test]
    fn double_panne_au_rapatriement_la_source_partielle_n_ecrase_rien() {
        for removed in [0, 1, 3] {
            let dir = tempfile::tempdir().unwrap();
            let pk = dir.path().join("Histoire.plain.pk");
            std::fs::write(&pk, b"PK").unwrap();
            let ws = workspace(dir.path(), "library:u1");
            let copy = ws.stage(&pk).unwrap();
            write_files(&ws.saved, &["precedente"]);
            let src = copy.parent().unwrap().join(SAVES_DIR);
            write_files(&src, &["a", "b", "c"]);
            let why = limit_error("trop gros".into());
            let moved = |f: &Path, t: &Path| copy_then_remove(f, t, half_remove(removed));
            assert!(ws
                .set_aside_with(&src, &why, "nouvelles sauvegardes", moved, promote_rejected)
                .is_err());
            assert_files(&ws.sibling(".rejected.new"), &["a", "b", "c"]);
            assert!(moved_marker(&src).is_file(), "reste marqué ({removed})");
            // Reprise (rapatriement, ou lancement qui commence par lui) : reste effacé, copie
            // complète promue, progression précédente intacte.
            assert!(ws.persist_saves().unwrap().is_none());
            assert_files(&ws.sibling(".rejected"), &["a", "b", "c"]);
            assert!(!ws.sibling(".rejected.new").exists());
            assert!(!src.exists() && !moved_marker(&src).exists());
            assert_files(&ws.saved, &["precedente"]);
            // Lancement suivant : rien ne change, la progression précédente est restaurée.
            let copy = ws.stage(&pk).unwrap();
            assert_files(&copy.parent().unwrap().join(SAVES_DIR), &["precedente"]);
            assert_files(&ws.sibling(".rejected"), &["a", "b", "c"]);
        }
    }

    /// #91, double panne au lancement : la mise de côté des sauvegardes conservées
    /// (`<config>/saves/<empreinte>`) échoue pendant l'effacement de la source. Le lancement
    /// suivant ne restaure pas la source partielle.
    #[test]
    fn double_panne_au_lancement_la_source_partielle_n_est_pas_restauree() {
        let dir = tempfile::tempdir().unwrap();
        let pk = dir.path().join("Histoire.plain.pk");
        std::fs::write(&pk, b"PK").unwrap();
        let ws = workspace(dir.path(), "library:u1");
        write_files(&ws.saved, &["a", "b", "c"]);
        let why = limit_error("trop gros".into());
        let moved = |f: &Path, t: &Path| copy_then_remove(f, t, half_remove(1));
        assert!(ws
            .set_aside_with(
                &ws.saved,
                &why,
                "sauvegardes conservées",
                moved,
                promote_rejected
            )
            .is_err());
        assert!(ws.saved.exists() && moved_marker(&ws.saved).is_file());
        let copy = ws.stage(&pk).unwrap();
        assert!(
            !copy.parent().unwrap().join(SAVES_DIR).exists(),
            "partie sans progression, la source partielle n'est pas restaurée"
        );
        assert!(!ws.saved.exists() && !moved_marker(&ws.saved).exists());
        assert_files(&ws.sibling(".rejected"), &["a", "b", "c"]);
    }

    /// Arrêt après l'effacement complet de la source, avant celui du marqueur : le marqueur seul
    /// est effacé, sans effet sur une progression écrite ensuite.
    #[test]
    fn un_marqueur_seul_est_efface_sans_rien_toucher_d_autre() {
        let dir = tempfile::tempdir().unwrap();
        let pk = dir.path().join("Histoire.plain.pk");
        std::fs::write(&pk, b"PK").unwrap();
        let ws = workspace(dir.path(), "library:u1");
        std::fs::create_dir_all(ws.saved.parent().unwrap()).unwrap();
        std::fs::write(moved_marker(&ws.saved), b"").unwrap();
        let copy = ws.stage(&pk).unwrap();
        assert!(!moved_marker(&ws.saved).exists());
        play_and_save(&copy, b"nouvelle");
        assert!(ws.persist_saves().unwrap().is_none());
        assert_eq!(
            std::fs::read(ws.saved.join("Histoire.plain/state")).unwrap(),
            b"nouvelle"
        );
    }

    /// #100 : l'effacement final du marqueur échoue alors que tout est complet (copie publiée,
    /// source effacée) : pas d'erreur ; le lancement suivant efface le marqueur resté et restaure
    /// la progression précédente.
    #[test]
    fn l_effacement_du_marqueur_en_echec_n_est_pas_une_erreur_et_se_reprend() {
        let dir = tempfile::tempdir().unwrap();
        let pk = dir.path().join("Histoire.plain.pk");
        std::fs::write(&pk, b"PK").unwrap();
        let ws = workspace(dir.path(), "library:u1");
        let copy = ws.stage(&pk).unwrap();
        write_files(&ws.saved, &["precedente"]);
        let src = copy.parent().unwrap().join(SAVES_DIR);
        write_files(&src, &["a", "b", "c"]);
        let why = limit_error("trop gros".into());
        let moved = |f: &Path, t: &Path| {
            copy_then_remove_with(f, t, remove_tree, |_: &Path| {
                Err(std::io::Error::other("marqueur verrouillé"))
            })
        };
        let note = ws
            .set_aside_with(&src, &why, "nouvelles sauvegardes", moved, promote_rejected)
            .unwrap();
        assert!(note.contains(".rejected"), "{note}");
        assert_files(&ws.sibling(".rejected"), &["a", "b", "c"]);
        assert!(!src.exists() && moved_marker(&src).is_file());
        let copy = ws.stage(&pk).unwrap();
        assert!(!moved_marker(&src).exists());
        assert_files(&copy.parent().unwrap().join(SAVES_DIR), &["precedente"]);
        assert_files(&ws.sibling(".rejected"), &["a", "b", "c"]);
        assert_files(&ws.saved, &["precedente"]);
    }

    /// #100 : un reste impossible à effacer (fichier verrouillé durablement) est signalé sur une
    /// ligne, chemin et consigne compris ; rien n'est effacé ni lu, et un nouvel essai le finit.
    #[test]
    fn un_reste_impossible_a_effacer_est_signale_et_reessaye() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join(SAVES_DIR);
        write_files(&src, &["a"]);
        let marker = moved_marker(&src);
        std::fs::write(&marker, b"").unwrap();
        let err = finish_moved_with(
            &src,
            |p: &Path| std::fs::symlink_metadata(p),
            |_: &Path| Err(std::io::Error::other("fichier verrouillé")),
        )
        .unwrap_err();
        let text = err.to_string();
        assert_eq!(
            text,
            format!(
                "reste d'une mise de côté interrompue non effacé ({}) : fichier verrouillé ; \
                 fermer ce qui utilise le dossier, puis réessayer",
                src.display()
            )
        );
        assert!(!text.contains('\n') && !text.contains("  "), "{text}");
        assert_files(&src, &["a"]);
        assert!(marker.is_file());
        // Reste effacé, mais marqueur verrouillé : erreur, marqueur conservé.
        let err = finish_moved_with(
            &src,
            |p: &Path| std::fs::symlink_metadata(p),
            |p: &Path| {
                if p == marker {
                    Err(std::io::Error::other("marqueur verrouillé"))
                } else {
                    remove_tree(p)
                }
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("puis réessayer"));
        assert!(!src.exists() && marker.is_file());
        finish_moved(&src).unwrap();
        assert!(!src.exists() && !marker.exists());
    }

    /// #110 : le marqueur illisible (autre que absent) n'est pas présenté comme un reste non
    /// effacé : message exact, une ligne, chemin du marqueur compris ; rien n'est effacé ni lu.
    #[test]
    fn un_marqueur_illisible_est_signale_tel_quel_sans_rien_effacer() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join(SAVES_DIR);
        write_files(&src, &["a"]);
        let marker = moved_marker(&src);
        std::fs::write(&marker, b"").unwrap();
        let err = finish_moved_with(
            &src,
            |_: &Path| Err(std::io::Error::other("accès refusé")),
            |_: &Path| -> std::io::Result<()> { panic!("effacement sans lecture du marqueur") },
        )
        .unwrap_err();
        let text = err.to_string();
        assert_eq!(
            text,
            format!(
                "impossible de lire le marqueur {} : accès refusé",
                marker.display()
            )
        );
        assert!(
            !text.contains('\n') && !text.contains("non effacé"),
            "{text}"
        );
        assert_files(&src, &["a"]);
        assert!(marker.is_file());
        // Marqueur absent : rien à faire, le dossier est une vraie donnée.
        std::fs::remove_file(&marker).unwrap();
        finish_moved_with(
            &src,
            |p: &Path| std::fs::symlink_metadata(p),
            |_: &Path| panic!("effacement sans marqueur"),
        )
        .unwrap();
        assert_files(&src, &["a"]);
    }

    /// #100 : l'écriture du marqueur échoue (copie déjà publiée en `.rejected.new`) : erreur,
    /// source intacte et jamais effacée ; la reprise promeut `.rejected.new`.
    #[test]
    fn l_ecriture_du_marqueur_en_echec_laisse_la_source_intacte() {
        let dir = tempfile::tempdir().unwrap();
        let pk = dir.path().join("Histoire.plain.pk");
        std::fs::write(&pk, b"PK").unwrap();
        let ws = workspace(dir.path(), "library:u1");
        let copy = ws.stage(&pk).unwrap();
        let src = copy.parent().unwrap().join(SAVES_DIR);
        write_files(&src, &["a", "b", "c"]);
        // Un dossier non vide à la place du marqueur : son écriture échoue.
        std::fs::create_dir_all(moved_marker(&src).join("bloque")).unwrap();
        let why = limit_error("trop gros".into());
        let moved = |f: &Path, t: &Path| {
            copy_then_remove(f, t, |_: &Path| -> std::io::Result<()> {
                panic!("source effacée sans marqueur")
            })
        };
        assert!(ws
            .set_aside_with(&src, &why, "nouvelles sauvegardes", moved, promote_rejected)
            .is_err());
        assert_files(&src, &["a", "b", "c"]);
        assert_files(&ws.sibling(".rejected.new"), &["a", "b", "c"]);
        std::fs::remove_dir_all(moved_marker(&src)).unwrap();
        ws.recover_rejected().unwrap();
        assert_files(&ws.sibling(".rejected"), &["a", "b", "c"]);
        assert_files(&src, &["a", "b", "c"]);
    }

    /// #100 : marqueur `<copie>/saves.moved` resté seul (arrêt après l'effacement de la source) :
    /// effacé au rapatriement, sans toucher à la progression conservée. #110 : avec un reste
    /// encore présent sous le marqueur, l'effacement précède toute lecture : le reste n'est
    /// jamais rapatrié.
    #[test]
    fn un_marqueur_saves_moved_seul_est_efface_sans_toucher_la_progression() {
        for remnant in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let pk = dir.path().join("Histoire.plain.pk");
            std::fs::write(&pk, b"PK").unwrap();
            let ws = workspace(dir.path(), "library:u1");
            let copy = ws.stage(&pk).unwrap();
            write_files(&ws.saved, &["precedente"]);
            let src = copy.parent().unwrap().join(SAVES_DIR);
            if remnant {
                play_and_save(&copy, b"reste");
            }
            std::fs::write(moved_marker(&src), b"").unwrap();
            assert!(ws.persist_saves().unwrap().is_none(), "{remnant}");
            assert!(!moved_marker(&src).exists() && !src.exists(), "{remnant}");
            assert_files(&ws.saved, &["precedente"]);
            assert!(!ws.sibling(".rejected").exists(), "{remnant}");
            let copy = ws.stage(&pk).unwrap();
            play_and_save(&copy, b"nouvelle");
            assert!(ws.persist_saves().unwrap().is_none());
            assert_eq!(
                std::fs::read(ws.saved.join("Histoire.plain/state")).unwrap(),
                b"nouvelle"
            );
        }
    }

    /// #100 : désinstallation avec un reste marqué. `purge(true)` efface tout, marqueurs compris ;
    /// `purge(false)` efface le reste sans le rapatrier et conserve la progression.
    #[test]
    fn purge_avec_un_reste_marque() {
        for purge_saves in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let pk = dir.path().join("Histoire.plain.pk");
            std::fs::write(&pk, b"PK").unwrap();
            let ws = workspace(dir.path(), "library:u1");
            let copy = ws.stage(&pk).unwrap();
            write_files(&ws.saved, &["precedente"]);
            let src = copy.parent().unwrap().join(SAVES_DIR);
            write_files(&src, &["c"]);
            std::fs::write(moved_marker(&src), b"").unwrap();
            write_files(&ws.sibling(".rejected.new"), &["a", "b", "c"]);
            ws.purge(purge_saves).unwrap();
            assert!(!ws.work.exists(), "{purge_saves}");
            if purge_saves {
                let left: Vec<_> = std::fs::read_dir(ws.saved.parent().unwrap())
                    .unwrap()
                    .map(|e| e.unwrap().file_name())
                    .collect();
                assert!(left.is_empty(), "{left:?}");
            } else {
                assert_files(&ws.saved, &["precedente"]);
                assert_files(&ws.sibling(".rejected"), &["a", "b", "c"]);
            }
        }
        // Reste de `<config>/saves/<empreinte>` (mise de côté au lancement interrompue).
        for purge_saves in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let ws = workspace(dir.path(), "library:u1");
            write_files(&ws.saved, &["c"]);
            std::fs::write(moved_marker(&ws.saved), b"").unwrap();
            write_files(&ws.sibling(".rejected.new"), &["a", "b", "c"]);
            ws.purge(purge_saves).unwrap();
            assert!(!ws.saved.exists() && !moved_marker(&ws.saved).exists());
            if purge_saves {
                assert!(!ws.sibling(".rejected").exists());
                assert!(!ws.sibling(".rejected.new").exists());
            } else {
                assert_files(&ws.sibling(".rejected"), &["a", "b", "c"]);
            }
        }
    }

    #[test]
    fn un_rejected_new_sans_rejected_est_promu_au_lancement_et_un_part_partiel_supprime() {
        let dir = tempfile::tempdir().unwrap();
        let pk = dir.path().join("Histoire.plain.pk");
        std::fs::write(&pk, b"PK").unwrap();
        let ws = workspace(dir.path(), "library:u1");
        std::fs::create_dir_all(ws.sibling(".rejected.new")).unwrap();
        std::fs::write(ws.sibling(".rejected.new").join("0"), b"gardee").unwrap();
        std::fs::create_dir_all(ws.sibling(".rejected.new.part")).unwrap();
        std::fs::write(ws.sibling(".rejected.new.part").join("partiel"), b"?").unwrap();
        ws.stage(&pk).unwrap();
        assert_eq!(
            std::fs::read(ws.sibling(".rejected").join("0")).unwrap(),
            b"gardee"
        );
        assert!(!ws.sibling(".rejected.new").exists());
        assert!(!ws.sibling(".rejected.new.part").exists());
    }

    #[test]
    fn la_copie_sans_limite_ne_deborde_pas_la_pile_sur_une_arborescence_tres_profonde() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("profond");
        let mut deep = from.clone();
        for _ in 0..1500 {
            deep.push("n");
        }
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("state"), b"fond").unwrap();
        let to = dir.path().join("copie");
        // Pile minuscule : une récursion par niveau y déborderait largement.
        let copied = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(move || copy_tree(&from, &to, 0, &mut Budget::unbounded()).map(|()| to))
            .unwrap()
            .join()
            .expect("pas de débordement de pile")
            .unwrap();
        let mut leaf = copied;
        for _ in 0..1500 {
            leaf.push("n");
        }
        assert_eq!(std::fs::read(leaf.join("state")).unwrap(), b"fond");
    }

    #[test]
    fn un_lancement_pendant_une_desinstallation_est_refuse_sans_lancer() {
        use crate::library::{tests::story, Library};
        let c = ctx();
        let cache = c.dir.path().join("cache");
        let saves = c.dir.path().join("config/saves");
        let lib = Library::new(&cache);
        let staged = lib.staging_dir().unwrap().join("t.plain.pk");
        std::fs::write(&staged, b"PK").unwrap();
        lib.install(&staged, &story("u1", Some(1)), None, 0)
            .unwrap();

        let (entered_tx, entered) = channel::<()>();
        let (release, released) = channel::<()>();
        let remover = {
            let (service, cache, saves) = (c.service.clone(), cache.clone(), saves.clone());
            std::thread::spawn(move || {
                // Même séquence que `library_remove` : tout sous le créneau.
                service
                    .while_not_playing("u1", || {
                        entered_tx.send(()).unwrap();
                        released.recv().unwrap();
                        purge_library_story(&cache, &saves, "u1", true).unwrap();
                        Library::new(&cache).remove("u1").unwrap();
                    })
                    .unwrap();
            })
        };
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        // Lancement pendant le retrait : il attend le créneau, puis ne trouve plus l'histoire.
        let launcher = {
            let (service, fake, cache, saves) = (
                c.service.clone(),
                c.fake.clone(),
                cache.clone(),
                saves.clone(),
            );
            std::thread::spawn(move || {
                let lib = Library::new(&cache);
                let ws = Workspace::new(&cache.join(PLAY_DIR), &saves, &library_key("u1"));
                service.launch(
                    &fake,
                    || {
                        let (path, _) = lib.playable("u1")?;
                        ws.stage(&path)
                    },
                    || None,
                    "library",
                    "u1",
                    "t",
                )
            })
        };
        std::thread::sleep(Duration::from_millis(300));
        assert!(!launcher.is_finished(), "le lancement attend le créneau");
        release.send(()).unwrap();
        remover.join().unwrap();
        let err = launcher.join().unwrap().unwrap_err();
        assert_eq!(err.kind(), "not_found");
        assert!(c.started.try_recv().is_err(), "aucun player lancé");
        assert!(!cache.join(PLAY_DIR).exists(), "rien recréé dans play/");
        assert!(!saves.exists(), "la progression purgée ne revient pas");
    }

    #[test]
    fn seule_une_erreur_de_limite_est_une_limite() {
        let other = std::io::Error::new(std::io::ErrorKind::InvalidData, "autre");
        assert!(!is_limit(&other));
        assert!(is_limit(&limit_error("x".into())));
    }

    #[test]
    fn les_notes_de_lancement_remontent_dans_saves_error() {
        let dir = tempfile::tempdir().unwrap();
        let pk = dir.path().join("Histoire.plain.pk");
        std::fs::write(&pk, b"PK").unwrap();
        let ws = workspace(dir.path(), "library:u1");
        std::fs::create_dir_all(&ws.saved).unwrap();
        for i in 0..=SAVES_MAX_FILES {
            std::fs::write(ws.saved.join(format!("{i}")), b"x").unwrap();
        }
        // Sauvegarde conservée hors limites : le lancement n'est pas bloqué.
        let copy = ws.stage(&pk).unwrap();
        assert!(copy.is_file());
        assert!(!copy.parent().unwrap().join("saves").exists());
        assert!(!ws.saved.exists());
        assert!(ws.sibling(".rejected").join("0").is_file());
        let info = ws
            .persist_saves_logged()
            .expect("information pour l'interface");
        assert!(info.contains("sauvegardes conservées refusées"), "{info}");
        // Les notes ne sont renvoyées qu'une fois.
        assert_eq!(ws.persist_saves_logged(), None);
    }

    #[test]
    fn un_echec_de_rapatriement_remonte_dans_saves_error() {
        let dir = tempfile::tempdir().unwrap();
        let pk = dir.path().join("Histoire.plain.pk");
        std::fs::write(&pk, b"PK").unwrap();
        let ws = workspace(dir.path(), "library:u1");
        let copy = ws.stage(&pk).unwrap();
        play_and_save(&copy, b"x");
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        std::fs::write(dir.path().join("config/saves"), b"x").unwrap();
        let info = ws.persist_saves_logged().expect("échec signalé");
        assert!(info.contains("non conservées"), "{info}");
    }

    #[test]
    fn purge_supprime_la_copie_de_travail_et_garde_la_progression_par_defaut() {
        let dir = tempfile::tempdir().unwrap();
        let pk = dir.path().join("Histoire.plain.pk");
        std::fs::write(&pk, b"PK").unwrap();
        let (cache, config) = (dir.path().join("cache"), dir.path().join("config/saves"));
        let ws = Workspace::new(&cache.join(PLAY_DIR), &config, &library_key("u1"));
        let copy = ws.stage(&pk).unwrap();
        // Partie non rapatriée : la purge sans `purge_saves` la met d'abord à l'abri.
        play_and_save(&copy, b"niveau 9");

        purge_library_story(&cache, &config, "u1", false).unwrap();
        assert!(!ws.work.exists(), "copie de travail supprimée");
        assert_eq!(
            std::fs::read(ws.saved.join("Histoire.plain/state")).unwrap(),
            b"niveau 9"
        );
        // Une autre histoire n'est pas touchée.
        let other = Workspace::new(&cache.join(PLAY_DIR), &config, &library_key("u2"));
        other.stage(&pk).unwrap();
        purge_library_story(&cache, &config, "u1", false).unwrap();
        assert!(other.work.is_dir());
        // Idempotent sur une histoire jamais lancée.
        purge_library_story(&cache, &config, "jamais", true).unwrap();
    }

    #[test]
    fn purge_saves_efface_aussi_la_progression_et_ses_reliquats() {
        let dir = tempfile::tempdir().unwrap();
        let pk = dir.path().join("Histoire.plain.pk");
        std::fs::write(&pk, b"PK").unwrap();
        let (cache, config) = (dir.path().join("cache"), dir.path().join("config/saves"));
        let ws = Workspace::new(&cache.join(PLAY_DIR), &config, &library_key("u1"));
        let copy = ws.stage(&pk).unwrap();
        play_and_save(&copy, b"niveau 9");
        ws.persist_saves().unwrap();
        for suffix in [".tmp", ".old", ".rejected"] {
            std::fs::create_dir_all(ws.sibling(suffix)).unwrap();
        }
        let autre = Workspace::new(&cache.join(PLAY_DIR), &config, &library_key("u2"));
        autre.stage(&pk).unwrap();
        play_and_save(&autre.work.join("Histoire.plain.pk"), b"autre");
        autre.persist_saves().unwrap();

        purge_library_story(&cache, &config, "u1", true).unwrap();
        assert!(!ws.work.exists());
        for suffix in ["", ".tmp", ".old", ".rejected"] {
            assert!(!ws.sibling(suffix).exists(), "{suffix:?}");
        }
        assert!(
            autre.saved.is_dir(),
            "la progression d'une autre histoire reste"
        );
    }

    #[test]
    fn purge_echoue_sans_rien_supprimer_si_la_progression_ne_peut_etre_mise_a_l_abri() {
        let dir = tempfile::tempdir().unwrap();
        let pk = dir.path().join("Histoire.plain.pk");
        std::fs::write(&pk, b"PK").unwrap();
        let (cache, config) = (dir.path().join("cache"), dir.path().join("config/saves"));
        let ws = Workspace::new(&cache.join(PLAY_DIR), &config, &library_key("u1"));
        let copy = ws.stage(&pk).unwrap();
        play_and_save(&copy, b"x");
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        std::fs::write(dir.path().join("config/saves"), b"fichier").unwrap();
        assert!(purge_library_story(&cache, &config, "u1", false).is_err());
        assert!(ws.work.join("saves").is_dir());
    }

    /// Lien de dossier sans privilège : jonction (`mklink /J`) sous Windows, lien symbolique
    /// ailleurs.
    fn link_dir(link: &Path, target: &Path) {
        #[cfg(windows)]
        {
            let out = Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                // `mklink` prend `/` pour une option : chemins en `\` uniquement.
                .arg(link.to_string_lossy().replace('/', "\\"))
                .arg(target.to_string_lossy().replace('/', "\\"))
                .output()
                .unwrap();
            assert!(out.status.success(), "mklink /J : {out:?}");
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
    }

    #[test]
    fn remove_tree_retire_un_lien_de_dossier_sans_toucher_sa_cible() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("cible");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("precieux"), b"x").unwrap();
        let link = dir.path().join("lien");
        link_dir(&link, &target);
        remove_tree(&link).unwrap();
        assert!(!link.exists());
        assert!(target.join("precieux").is_file(), "la cible est intacte");
        remove_tree(&dir.path().join("absent")).unwrap();
    }

    #[test]
    fn stage_vide_un_lien_de_dossier_de_la_copie_de_travail() {
        let dir = tempfile::tempdir().unwrap();
        let pk = dir.path().join("Histoire.plain.pk");
        std::fs::write(&pk, b"PK").unwrap();
        let outside = dir.path().join("dehors");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("precieux"), b"x").unwrap();
        let ws = workspace(dir.path(), "library:u1");
        ws.stage(&pk).unwrap();
        let link = ws.work.join("lien");
        link_dir(&link, &outside);
        ws.stage(&pk).unwrap();
        assert!(!link.exists());
        assert!(outside.join("precieux").is_file());
    }

    #[test]
    fn status_ne_bloque_pas_pendant_un_vidage_long() {
        let c = ctx();
        let (entered_tx, entered) = channel::<()>();
        let (release, released) = channel::<()>();
        let service = c.service.clone();
        let busy = std::thread::spawn(move || {
            service
                .while_idle(|| {
                    entered_tx.send(()).unwrap();
                    released.recv().unwrap();
                })
                .unwrap();
        });
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        let began = Instant::now();
        // Le créneau est tenu par le vidage : pas de player en cours, et on ne l'attend pas.
        assert_eq!(c.service.status(), None);
        assert!(began.elapsed() < Duration::from_millis(500));
        release.send(()).unwrap();
        busy.join().unwrap();
    }

    #[test]
    fn while_not_playing_refuse_seulement_l_histoire_en_cours() {
        let c = ctx();
        let story = c.story("long.plain.pk", "sleep 30000\nexit 0\n");
        let session = c.launch(&story).unwrap(); // uuid « u1 », origine bibliothèque
        c.started.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(
            c.service.while_not_playing("u1", || ()).unwrap_err().kind(),
            "player_busy"
        );
        assert_eq!(c.service.while_not_playing("autre", || 5).unwrap(), 5);
        let kill = Command::new("taskkill")
            .args(["/PID", &session.pid.to_string(), "/F"])
            .output()
            .unwrap();
        assert!(kill.status.success());
        c.wait_exit();
        assert!(c.service.while_not_playing("u1", || ()).is_ok());
    }

    #[test]
    fn while_idle_est_refuse_pendant_une_partie_et_execute_sinon() {
        let c = ctx();
        // Le player dort assez longtemps ; il est tué à la fin du test.
        let story = c.story(
            "long.plain.pk",
            "sleep 30000
exit 0
",
        );
        assert_eq!(c.service.while_idle(|| 7).unwrap(), 7);
        let session = c.launch(&story).unwrap();
        c.started.recv_timeout(Duration::from_secs(10)).unwrap();
        let mut ran = false;
        let err = c.service.while_idle(|| ran = true).unwrap_err();
        assert_eq!(err.kind(), "player_busy");
        assert!(!ran, "l'action n'est pas exécutée pendant une partie");
        let kill = Command::new("taskkill")
            .args(["/PID", &session.pid.to_string(), "/F"])
            .output()
            .unwrap();
        assert!(kill.status.success());
        c.wait_exit();
        assert_eq!(c.service.while_idle(|| 1).unwrap(), 1);
    }

    #[test]
    fn une_restauration_hors_limites_met_la_sauvegarde_de_cote_sans_bloquer() {
        let dir = tempfile::tempdir().unwrap();
        let pk = dir.path().join("Histoire.plain.pk");
        std::fs::write(&pk, b"PK").unwrap();
        let ws = workspace(dir.path(), "library:u1");
        let saved = ws.saved.clone();
        std::fs::create_dir_all(&saved).unwrap();
        // Sauvegarde au-delà de la limite : la restauration échoue en cours de route.
        for i in 0..=SAVES_MAX_FILES {
            std::fs::write(saved.join(format!("{i}")), b"bonne").unwrap();
        }
        let copy = ws.stage(&pk).unwrap();
        assert!(copy.is_file(), "lancement possible");
        // Pas de `saves/` partiel à rapatrier, rien de laissé en route.
        assert!(!ws.work.join("saves").exists());
        assert!(!ws.work.join("saves.tmp").exists());
        // La sauvegarde est mise de côté (une génération), pas détruite.
        assert!(!saved.exists());
        let rejected = ws.sibling(".rejected");
        assert_eq!(std::fs::read(rejected.join("0")).unwrap(), b"bonne");
        assert!(!ws.sibling(".old").exists());
        // Le lancement suivant n'a plus rien à restaurer ni à mettre de côté.
        ws.stage(&pk).unwrap();
        assert!(rejected.join("0").is_file());
    }

    #[test]
    fn la_reprise_de_old_se_fait_avant_le_nettoyage_de_persist_saves() {
        let dir = tempfile::tempdir().unwrap();
        let pk = dir.path().join("Histoire.plain.pk");
        std::fs::write(&pk, b"PK").unwrap();
        let ws = workspace(dir.path(), "library:u1");
        let copy = ws.stage(&pk).unwrap();
        play_and_save(&copy, b"niveau 7");
        ws.persist_saves().unwrap();
        // Arrêt entre les deux renommages : la sauvegarde n'est plus que dans `.old`.
        std::fs::rename(&ws.saved, ws.sibling(".old")).unwrap();
        ws.persist_saves().unwrap();
        assert!(!ws.sibling(".old").exists());
        assert_eq!(
            std::fs::read(ws.saved.join("Histoire.plain/state")).unwrap(),
            b"niveau 7"
        );
        // Sans copie de travail à rapatrier, la reprise a aussi lieu.
        std::fs::remove_dir_all(&ws.work).unwrap();
        std::fs::rename(&ws.saved, ws.sibling(".old")).unwrap();
        ws.persist_saves().unwrap();
        assert!(ws.saved.join("Histoire.plain/state").is_file());
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
    fn sans_sauvegarde_rien_n_est_cree() {
        let dir = tempfile::tempdir().unwrap();
        let pk = dir.path().join("Histoire.plain.pk");
        std::fs::write(&pk, b"PK").unwrap();
        let ws = workspace(dir.path(), "k");
        assert_eq!(ws.persist_saves().unwrap(), None); // pas encore de copie de travail
        ws.stage(&pk).unwrap();
        assert_eq!(ws.persist_saves().unwrap(), None); // pas de saves/ écrit par le player
        assert!(!dir.path().join("config").exists());
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
                    Some("sauvegardes écartées".to_string())
                },
                "library",
                "u",
                "t",
            )
            .unwrap();
        let done = c.wait_exit();
        assert_eq!(done.code, Some(9));
        assert!(rx.try_recv().is_ok(), "finish exécuté avant l'événement");
        assert_eq!(done.saves_error.as_deref(), Some("sauvegardes écartées"));
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

    /// Compense, en CI, le test de lien symbolique ci-dessous (ignoré : il demande un privilège) :
    /// une jonction (`mklink /J`, sans privilège) vers un vrai dossier est refusée de même.
    #[test]
    fn check_source_refuse_une_jonction_vers_un_dossier() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("vrai.plain");
        std::fs::create_dir(&target).unwrap();
        let link = dir.path().join("lien.plain");
        link_dir(&link, &target);
        let err = check_source(&link, StoryKind::FlamPlain).unwrap_err();
        assert_eq!(err.kind(), "validation");
        assert!(check_source(&target, StoryKind::FlamPlain).is_ok());
    }

    #[test]
    fn la_copie_ignore_les_liens_de_la_source_sans_les_suivre() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("dehors");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("secret"), b"x").unwrap();
        let from = dir.path().join("saves");
        std::fs::create_dir_all(from.join("d")).unwrap();
        std::fs::write(from.join("d/f"), b"ok").unwrap();
        link_dir(&from.join("lien"), &outside);
        let to = dir.path().join("copie");
        copy_tree(&from, &to, 0, &mut Budget::unbounded()).unwrap();
        assert_eq!(std::fs::read(to.join("d/f")).unwrap(), b"ok");
        assert!(!to.join("lien").exists(), "lien ignoré, pas copié");
        // Le repli de `move_dir` supprime ensuite la source, lien compris, sans toucher la cible.
        remove_tree(&from).unwrap();
        assert!(outside.join("secret").is_file());
    }

    #[test]
    #[ignore = "demande le privilège de création de liens symboliques (mode développeur Windows)"]
    fn check_source_refuse_les_liens_symboliques() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("vrai.plain.pk");
        std::fs::write(&target, b"x").unwrap();
        let link = dir.path().join("lien.plain.pk");
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(&target, &link).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).unwrap();
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
                || None,
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
            download_count: None,
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
