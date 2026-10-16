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
//! **Copie de travail** : le player extrait un `.plain.pk` et écrit ses sauvegardes
//! (`saves/<nom>`) **à côté** de l'histoire. Il n'est donc jamais lancé sur le fichier d'origine
//! (dossier de l'utilisateur, ou `library/`) mais sur une copie dans `<cache>/play/<empreinte>/`
//! ([`stage`]), nettoyée avant chaque lancement sauf `saves/`. Les sauvegardes vivent donc dans
//! le cache (`play/`) : vider le cache les efface. Le player est lancé avec son propre dossier
//! comme dossier courant. Les sources (fichier ordinaire pour un `.plain.pk`, dossier pour un
//! `.plain`, jamais un lien) sont vérifiées avant la copie.

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
/// Sauvegardes du player, conservées d'un lancement à l'autre.
const SAVES_DIR: &str = "saves";

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

    /// Lance `exe` sur le fichier que renvoie `prepare` pour l'histoire `uuid`.
    ///
    /// `prepare` (copie de travail, voir [`stage`]) n'est appelée qu'une fois le player trouvé
    /// et le créneau libre, **sous le verrou du créneau** : elle ne peut pas nettoyer le dossier
    /// d'un player en cours. Appel bloquant (copie de fichiers) : à exécuter hors du runtime.
    pub fn launch(
        &self,
        exe: &Path,
        prepare: impl FnOnce() -> AppResult<PathBuf>,
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

/// Profondeur maximale de la copie d'un dossier `.plain`.
const COPY_MAX_DEPTH: usize = 16;

fn copy_tree(from: &Path, to: &Path, depth: usize) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    if depth >= COPY_MAX_DEPTH {
        return Ok(());
    }
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let (src, dst) = (entry.path(), to.join(entry.file_name()));
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            copy_tree(&src, &dst, depth + 1)?;
        } else if file_type.is_file() {
            std::fs::copy(&src, &dst)?;
        } // liens et autres : ignorés
    }
    Ok(())
}

/// Copie de travail de `source` (déjà vérifiée par [`check_source`]) dans
/// `<play_root>/<empreinte de key>/<nom>`, et renvoie ce chemin.
///
/// Le player extrait les `.plain.pk` et écrit ses sauvegardes **à côté** de l'histoire : on ne le
/// lance donc jamais dans un dossier que l'application ne possède pas (dossier d'une source
/// de l'utilisateur) ni dans `library/`. Le dossier de travail est nettoyé avant chaque copie,
/// **sauf `saves/`** (progression de l'enfant, conservée d'un lancement à l'autre).
pub fn stage(play_root: &Path, key: &str, source: &Path) -> AppResult<PathBuf> {
    let name = source.file_name().ok_or_else(|| {
        AppError::Validation(format!("nom d'histoire invalide : {}", source.display()))
    })?;
    let digest = Sha256::digest(key.as_bytes());
    let hash: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
    let dir = play_root.join(hash);
    std::fs::create_dir_all(&dir)?;
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        if entry.file_name() == SAVES_DIR {
            continue;
        }
        if entry.file_type()?.is_dir() {
            std::fs::remove_dir_all(entry.path())?;
        } else {
            std::fs::remove_file(entry.path())?;
        }
    }
    let dest = dir.join(name);
    if std::fs::symlink_metadata(source)?.is_dir() {
        copy_tree(source, &dest, 0)?;
    } else {
        std::fs::copy(source, &dest)?;
    }
    Ok(dest)
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
    // Le vidage du cache attend la fin de la copie.
    let _gate = catalog.cache_gate().read_owned().await;
    let service = player.inner().clone();
    blocking(move || {
        let (path, title) = Library::new(&cache_root).playable(&uuid)?;
        check_source(&path, StoryKind::FlamPk)?;
        let play = cache_root.join(PLAY_DIR);
        let key = format!("library:{uuid}");
        service.launch(&exe, || stage(&play, &key, &path), "library", &uuid, &title)
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
    let _gate = catalog.cache_gate().read_owned().await;
    let service = player.inner().clone();
    blocking(move || {
        let path = folder_target(&entry, &root)?;
        let play = cache_root.join(PLAY_DIR);
        let key = format!("folder:{source_id}:{entry_id}");
        service.launch(
            &exe,
            || stage(&play, &key, &path),
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
            .launch(&not_exe, || Ok(story), "library", "u", "t")
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

    #[test]
    fn stage_copie_un_pk_sans_toucher_la_source_et_conserve_les_saves() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("utilisateur");
        std::fs::create_dir(&src).unwrap();
        let pk = src.join("Histoire.plain.pk");
        std::fs::write(&pk, b"PK").unwrap();
        let play = dir.path().join("cache/play");

        let copy = stage(&play, "folder:s:1", &pk).unwrap();
        assert!(copy.starts_with(&play));
        assert_eq!(copy.file_name(), pk.file_name());
        assert_eq!(std::fs::read(&copy).unwrap(), b"PK");
        // Le player extrait et sauvegarde à côté de la copie.
        let work = copy.parent().unwrap().to_path_buf();
        std::fs::create_dir_all(work.join("Histoire.plain")).unwrap();
        std::fs::write(work.join("Histoire.plain/info.plain"), b"x").unwrap();
        std::fs::create_dir_all(work.join("saves/Histoire.plain")).unwrap();
        std::fs::write(work.join("saves/Histoire.plain/state"), b"s").unwrap();

        // Relance : extraction obsolète supprimée, sauvegardes conservées.
        let again = stage(&play, "folder:s:1", &pk).unwrap();
        assert_eq!(again, copy);
        assert!(!work.join("Histoire.plain").exists());
        assert!(work.join("saves/Histoire.plain/state").is_file());
        // Une autre entrée a son propre dossier.
        let other = stage(&play, "folder:s:2", &pk).unwrap();
        assert_ne!(other.parent(), copy.parent());
        // La source n'a reçu aucun fichier.
        assert_eq!(std::fs::read_dir(&src).unwrap().count(), 1);
    }

    #[test]
    fn stage_copie_un_dossier_plain_recursivement() {
        let dir = tempfile::tempdir().unwrap();
        let story = dir.path().join("H.plain");
        std::fs::create_dir_all(story.join("img")).unwrap();
        std::fs::write(story.join("info.plain"), b"t").unwrap();
        std::fs::write(story.join("img/a.lif"), b"i").unwrap();
        let copy = stage(&dir.path().join("play"), "k", &story).unwrap();
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
                || stage(&c.dir.path().join(PLAY_DIR), "real", &story),
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
