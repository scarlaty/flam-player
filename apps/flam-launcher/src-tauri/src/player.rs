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
//! (dernières lignes, 8 Kio au plus) et émet `player://exited` ; un code non nul, ou l'absence
//! de code (processus tué), est un échec dont `stderr` donne la raison (un player qui plante au
//! démarrage échoue en quelques millisecondes avec un message de chargement).
//!
//! **Effets de bord du player** : pour une histoire `.plain.pk`, `flam-player` extrait
//! l'archive à côté d'elle (`<nom>.plain/`), donc dans `library/` ou dans le dossier de la
//! source. Le player est lancé avec son propre dossier comme dossier courant (ses sauvegardes,
//! `saves/<histoire>`, y sont écrites).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::State;

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

    /// Lance `exe` sur `target` pour l'histoire `uuid`. Vérifie juste avant l'exécution que le
    /// player et l'histoire existent, puis refuse un second lancement.
    pub fn launch(
        &self,
        exe: &Path,
        target: &Path,
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
        if !target.exists() {
            return Err(AppError::StoryFileMissing(format!(
                "le fichier de « {title} » n'existe plus : {}",
                target.display()
            )));
        }
        let mut slot = lock(&self.current);
        if let Some(running) = slot.as_ref() {
            return Err(AppError::PlayerBusy(format!(
                "flam-player est déjà en cours d'exécution (« {} »)",
                running.title
            )));
        }
        let mut command = Command::new(exe);
        command
            .arg(target)
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
        drop(slot);
        tracing::info!("flam-player lancé (pid {}) : {title}", session.pid);
        self.events.started(&session);

        let stderr = child.stderr.take();
        let reader = std::thread::spawn(move || stderr.map(read_tail).unwrap_or_default());
        let service = self.clone();
        let waiting = session.clone();
        let began = Instant::now();
        std::thread::spawn(move || {
            let status = child.wait();
            let stderr = reader.join().unwrap_or_default();
            let (code, success) = match &status {
                Ok(status) => (status.code(), status.success()),
                Err(_) => (None, false),
            };
            let stderr = match status {
                Err(err) => format!("attente du player impossible : {err}"),
                Ok(_) => stderr,
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
            *lock(&service.current) = None;
            service.events.exited(&event);
        });
        Ok(session)
    }
}

/// Lit le flux jusqu'à sa fin et renvoie ses dernières lignes (borné en mémoire).
fn read_tail(mut stream: impl Read) -> String {
    let mut kept: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 2048];
    while let Ok(n) = stream.read(&mut chunk) {
        if n == 0 {
            break;
        }
        kept.extend_from_slice(&chunk[..n]);
        if kept.len() > STDERR_KEEP_BYTES {
            kept.drain(..kept.len() - STDERR_KEEP_BYTES);
        }
    }
    let text = String::from_utf8_lossy(&kept);
    let lines: Vec<&str> = text.lines().map(str::trim_end).collect();
    let start = lines.len().saturating_sub(STDERR_KEEP_LINES);
    lines[start..].join("\n").trim().to_string()
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
/// dossier de la source. Un zip Telmi doit d'abord être installé (converti).
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
    Ok(path)
}

/// Lance l'histoire installée `uuid` dans `flam-player`.
#[tauri::command]
pub async fn player_launch(
    config: State<'_, ConfigState>,
    player: State<'_, PlayerService>,
    uuid: String,
) -> AppResult<PlayerSession> {
    let exe = player_exe(&config)?;
    let cache_root = {
        let _guard = config.guard();
        SettingsStore::new(&config.dir).resolved_cache_dir(&Environment::current())?
    };
    let (path, title) = Library::new(&cache_root).playable(&uuid)?;
    player.launch(&exe, &path, "library", &uuid, &title)
}

/// Lance directement une histoire `.plain` / `.plain.pk` du dernier scan d'une source dossier.
#[tauri::command]
pub async fn player_launch_folder(
    config: State<'_, ConfigState>,
    folder: State<'_, FolderState>,
    player: State<'_, PlayerService>,
    source_id: String,
    entry_id: String,
) -> AppResult<PlayerSession> {
    let exe = player_exe(&config)?;
    let root = {
        let _guard = config.guard();
        resolve_folder(&config.dir, &Environment::current(), &source_id)?.0
    };
    let entry = folder.entry(&source_id, &entry_id)?;
    let path = folder_target(&entry, &root)?;
    player.launch(&exe, &path, "folder", &entry_id, &entry.title)
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
    }

    impl PlayerEvents for Recorder {
        fn started(&self, session: &PlayerSession) {
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
        let (started_tx, started) = channel();
        let (exited_tx, exited) = channel();
        let service = PlayerService::new(Arc::new(Recorder {
            started: Mutex::new(started_tx),
            exited: Mutex::new(exited_tx),
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
            self.service
                .launch(&self.fake, target, "library", "u1", "Mon histoire")
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
            .launch(&c.dir.path().join("nope.exe"), &story, "library", "u", "t")
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
            .launch(&not_exe, &story, "library", "u", "t")
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
            .launch(&exe, &story, "library", "real", "Réel")
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
        let root = std::env::temp_dir().join("flam-root");
        let inside = root.join("a.plain.pk");
        let ok =
            folder_target(&entry(StoryKind::FlamPk, &inside.to_string_lossy()), &root).unwrap();
        assert_eq!(ok, inside);
        let plain = folder_target(
            &entry(StoryKind::FlamPlain, &inside.to_string_lossy()),
            &root,
        );
        assert!(plain.is_ok());
        let zip = folder_target(
            &entry(StoryKind::TelmiZip, &inside.to_string_lossy()),
            &root,
        );
        assert_eq!(zip.unwrap_err().kind(), "validation");
        let outside = std::env::temp_dir().join("autre").join("b.plain.pk");
        let err = folder_target(&entry(StoryKind::FlamPk, &outside.to_string_lossy()), &root)
            .unwrap_err();
        assert_eq!(err.kind(), "validation");
        let relative = folder_target(&entry(StoryKind::FlamPk, "a.plain.pk"), &root);
        assert_eq!(relative.unwrap_err().kind(), "validation");
    }
}
