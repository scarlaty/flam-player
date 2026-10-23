//! Conversion par `telmi2flam --progress-json` (issue #14) en processus enfant, avec l'un des
//! deux moteurs ([`Engine`], #136) : `telmi2flam.py` (Python embarqué, par défaut) ou le binaire
//! Rust `telmi2flam.exe` (même protocole, PORT-RUST.md §2.2). Seuls la ligne de commande et
//! l'annulation polie diffèrent ; protocole, Job Object, délai de grâce et nettoyage sont communs.
//!
//! - Lancement sans console visible : `CREATE_NO_WINDOW`, et `CREATE_NEW_PROCESS_GROUP` pour
//!   pouvoir viser le seul convertisseur avec `CTRL_BREAK_EVENT`.
//! - stdout : un événement JSON par ligne, lu ligne par ligne ; une ligne non JSON (ou un
//!   événement inconnu) est ignorée. Les messages affichés viennent de ces événements.
//! - stderr : page de code locale sous Windows, donc décodé en tolérant les erreurs
//!   (`PYTHONIOENCODING=utf-8` est aussi demandé) et gardé seulement comme repli (fin du flux).
//! - Arrêt forcé : le convertisseur est rattaché à un Job Object Windows
//!   (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`) ; l'arrêt forcé termine le job, donc aussi ses
//!   sous-processus. Pour qu'aucun sous-processus ne puisse échapper au job, le convertisseur
//!   est lancé **suspendu** (`CREATE_SUSPENDED`), rattaché au job, puis seulement son thread
//!   principal est repris : il n'exécute aucune instruction avant d'être dans le job. Si le job ne
//!   peut pas être créé (avertissement), la conversion continue sans lui : l'arrêt forcé se limite
//!   alors au processus direct.
//! - Annulation : demande polie (le convertisseur nettoie ses temporaires et sort avec 130), puis
//!   arrêt forcé après un délai de grâce, puis suppression de `<sortie>.<pid>.part`. La demande
//!   polie dépend du moteur :
//!   - Rust : `--cancel-on-stdin-eof`, stdin en tube gardé ouvert pendant toute la conversion
//!     et **fermé** pour annuler (aucun Python nécessaire) ;
//!   - Python : `CTRL_BREAK_EVENT` par l'assistant décrit ci-dessous.
//! - Aucune limite mémoire sur le job (décision D5) : seul `KILL_ON_JOB_CLOSE` est posé.
//!
//! Une application graphique n'a pas de console et le convertisseur, lancé avec
//! `CREATE_NO_WINDOW`, a la sienne (cachée) : `GenerateConsoleCtrlEvent` ne peut viser qu'un
//! processus de **sa** console. L'événement est donc envoyé par un court assistant Python
//! (même interpréteur) lancé sans fenêtre, qui quitte sa console, s'attache à celle du
//! convertisseur, s'immunise contre l'événement puis l'émet vers le groupe du convertisseur.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::watch;

use super::download::cancelled;
use crate::settings::ThumbFit;
use crate::{AppError, AppResult};

/// Ligne JSON la plus longue lue (au-delà : tronquée, donc ignorée).
const MAX_LINE: u64 = 1024 * 1024;
/// Fin de stderr conservée pour les messages de repli.
const STDERR_TAIL: usize = 4096;

#[cfg(windows)]
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
const CREATE_SUSPENDED: u32 = 0x0000_0004;

/// Assistant d'envoi de `CTRL_BREAK_EVENT` (voir l'en-tête du module). Code de sortie 0 si
/// l'événement a été émis.
#[cfg(windows)]
const CTRL_BREAK_HELPER: &str = "import ctypes,sys\n\
k=ctypes.windll.kernel32\n\
pid=int(sys.argv[1])\n\
k.FreeConsole()\n\
ok=k.AttachConsole(pid) and k.SetConsoleCtrlHandler(None,True) and k.GenerateConsoleCtrlEvent(1,pid)\n\
sys.exit(0 if ok else 1)\n";

/// Moteur de conversion lancé (#136).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine<'a> {
    /// `python <options durcies> telmi2flam.py …` ; annulation par `CTRL_BREAK_HELPER`.
    Python { python: &'a Path, script: &'a Path },
    /// `telmi2flam.exe … --cancel-on-stdin-eof` ; annulation par fermeture de stdin.
    Rust { exe: &'a Path },
}

impl Engine<'_> {
    /// Exécutable lancé.
    fn program(&self) -> &Path {
        match self {
            Self::Python { python, .. } => python,
            Self::Rust { exe } => exe,
        }
    }

    /// Désignation dans les messages d'erreur.
    fn label(&self) -> String {
        match self {
            Self::Python { python, .. } => format!("Python ({})", python.display()),
            Self::Rust { exe } => format!("telmi2flam.exe ({})", exe.display()),
        }
    }

    /// Fichier qui identifie le convertisseur (empreinte de repli de la version, voir
    /// `install::converter_fingerprint`) : le script pour Python, le binaire pour Rust.
    pub fn identity(&self) -> &Path {
        match self {
            Self::Python { script, .. } => script,
            Self::Rust { exe } => exe,
        }
    }
}

/// Une conversion à lancer.
pub struct Conversion<'a> {
    pub engine: Engine<'a>,
    pub input: &'a Path,
    /// Sortie `.plain.pk` (publication atomique par le convertisseur).
    pub output: &'a Path,
    /// ffmpeg à utiliser pour rééchantillonner l'audio à 44 100 Hz (#46), transmis par la
    /// variable `FLAM_FFMPEG`. `None` : variable vide, donc rééchantillonnage désactivé (le
    /// convertisseur ne cherche jamais un ffmpeg du `PATH`, dont la licence est inconnue).
    pub ffmpeg: Option<&'a Path>,
    /// Remplissage des vignettes, transmis par `--thumb-fit` (ensemble fermé, #279).
    pub thumb_fit: ThumbFit,
    /// Délai laissé au convertisseur après la demande d'arrêt polie avant l'arrêt forcé.
    pub grace: Duration,
}

/// Progression d'une phase (`images`, `audio`, `scripts`, `zip`).
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub phase: String,
    /// Rang de la phase (0..phases) et nombre de phases annoncées par `start`.
    pub index: usize,
    pub phases: usize,
    pub done: u64,
    pub total: u64,
}

impl Step {
    /// Avancement global de la conversion, en pourcentage.
    pub fn percent(&self) -> f64 {
        let within = if self.total == 0 {
            1.0
        } else {
            self.done.min(self.total) as f64 / self.total as f64
        };
        (self.index as f64 + within) * 100.0 / self.phases.max(1) as f64
    }
}

/// Conversion réussie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Converted {
    pub warnings: u32,
    /// Version annoncée par l'événement `start` (champ `version`), si présente et valide.
    pub version: Option<String>,
}

/// Version lue dans l'événement `start` : texte court, sans caractère de contrôle.
fn clean_version(raw: &str) -> Option<String> {
    let v = raw.trim();
    (!v.is_empty() && v.chars().count() <= 64 && !v.chars().any(char::is_control))
        .then(|| v.to_string())
}

/// Lit `reader` jusqu'à la fin par morceaux et ne garde que les `cap` derniers octets
/// (mémoire bornée quelle que soit la quantité écrite sur stderr).
async fn read_tail<R: AsyncRead + Unpin>(mut reader: R, cap: usize) -> String {
    let mut tail: Vec<u8> = Vec::with_capacity(cap.min(8192) * 2);
    let mut chunk = [0u8; 8192];
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                tail.extend_from_slice(&chunk[..n]);
                if tail.len() > cap * 2 {
                    tail.drain(..tail.len() - cap);
                }
            }
        }
    }
    let start = tail.len().saturating_sub(cap);
    String::from_utf8_lossy(&tail[start..]).into_owned()
}

/// Job Object Windows et reprise du convertisseur lancé suspendu.
#[cfg(windows)]
mod job {
    use tokio::process::Child;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};

    /// Job Object : fermer la dernière poignée (ou le terminer) tue tout le job.
    pub struct Job(HANDLE);

    // SAFETY : une poignée de Job Object peut être utilisée depuis n'importe quel thread.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    impl Job {
        /// Crée un job `KILL_ON_JOB_CLOSE` et y rattache `child`. `None` si l'une des étapes
        /// échoue (l'arrêt se limite alors au processus direct).
        pub fn attach(child: &Child) -> Option<Self> {
            let process = child.raw_handle()?;
            // SAFETY : appels Win32 sur des poignées valides ; la structure est initialisée à zéro.
            unsafe {
                let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if handle.is_null() {
                    return None;
                }
                let job = Job(handle);
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let ok = SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    std::ptr::addr_of!(info).cast(),
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
                if ok == 0 || AssignProcessToJobObject(handle, process as _) == 0 {
                    return None;
                }
                Some(job)
            }
        }

        pub fn terminate(&self) {
            // SAFETY : poignée de job valide tant que `self` existe.
            unsafe {
                TerminateJobObject(self.0, 1);
            }
        }

        /// Poignée brute (tests : `IsProcessInJob`).
        #[cfg(test)]
        pub fn raw(&self) -> HANDLE {
            self.0
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY : poignée possédée, fermée une seule fois.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    /// Reprend le thread principal d'un processus lancé avec `CREATE_SUSPENDED` (le seul thread
    /// du processus à ce stade). `tokio` ne donne pas la poignée du thread : on le retrouve par
    /// un instantané des threads du système, filtré sur le pid.
    pub fn resume(child: &Child) -> std::io::Result<()> {
        let pid = child
            .id()
            .ok_or_else(|| std::io::Error::other("processus déjà terminé"))?;
        // SAFETY : instantané et poignées de thread fermés avant le retour ; `THREADENTRY32`
        // est initialisée à zéro avec `dwSize` renseigné, comme l'exige l'API.
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                return Err(std::io::Error::last_os_error());
            }
            let mut entry: THREADENTRY32 = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
            let mut resumed = false;
            let mut failure = None;
            let mut more = Thread32First(snapshot, &mut entry) != 0;
            while more {
                if entry.th32OwnerProcessID == pid {
                    let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                    if thread.is_null() {
                        failure = Some(std::io::Error::last_os_error());
                    } else {
                        if ResumeThread(thread) == u32::MAX {
                            failure = Some(std::io::Error::last_os_error());
                        } else {
                            resumed = true;
                        }
                        CloseHandle(thread);
                    }
                }
                more = Thread32Next(snapshot, &mut entry) != 0;
            }
            CloseHandle(snapshot);
            match (resumed, failure) {
                (true, _) => Ok(()),
                (false, Some(err)) => Err(err),
                (false, None) => Err(std::io::Error::other("thread principal introuvable")),
            }
        }
    }
}

/// Hors Windows : pas de Job Object ni de lancement suspendu. `Job` est inhabité (aucun code
/// mort : `attach` ne renvoie jamais de valeur, `terminate` n'est donc jamais appelée).
#[cfg(not(windows))]
mod job {
    use tokio::process::Child;

    pub enum Job {}

    impl Job {
        pub fn attach(_child: &Child) -> Option<Self> {
            None
        }

        pub fn terminate(&self) {
            match *self {}
        }
    }

    pub fn resume(_child: &Child) -> std::io::Result<()> {
        Ok(())
    }
}

use job::Job;

/// Convertisseur lancé depuis `cwd` (un dossier de l'application), **suspendu** sous Windows :
/// l'appelant le rattache au job puis le reprend ([`job::resume`]).
///
/// Rust : stdin en tube, que l'appelant garde ouvert jusqu'à la fin (sa fermeture annule, voir
/// [`converter_args`]). Python : stdin vide, interpréteur durci :
/// - `-E` : variables `PYTHON*` ignorées (`PYTHONPATH`, `PYTHONSTARTUP`…) ;
/// - `-s` : pas de `site-packages` utilisateur ;
/// - `-B` : aucun `__pycache__` écrit (le dossier d'une application installée doit rester
///   intact : le désinstallateur ne connaît pas ces fichiers) ;
/// - `-u` et `-X utf8` : flux non tamponnés et UTF-8 (remplacent `PYTHONUNBUFFERED` et
///   `PYTHONIOENCODING`, ignorés avec `-E`).
///
/// `-I` (isolement complet) n'est pas utilisable pour `telmi2flam.py` : il retire aussi le
/// dossier du script de `sys.path`, d'où viennent ses modules (`lif`, `mp3map`). Le dossier
/// courant, lui, n'est pas dans `sys.path` quand on lance un script.
fn command(engine: &Engine<'_>, cwd: &Path) -> Command {
    let mut cmd = Command::new(engine.program());
    match engine {
        Engine::Python { .. } => {
            cmd.args(["-E", "-s", "-B", "-u", "-X", "utf8"])
                .stdin(Stdio::null());
        }
        Engine::Rust { .. } => {
            cmd.stdin(Stdio::piped());
        }
    }
    cmd.current_dir(cwd).kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW | CREATE_SUSPENDED);
    cmd
}

/// Demande au convertisseur de s'arrêter proprement (Ctrl+Break sous Windows, SIGTERM
/// ailleurs). Renvoie faux si l'envoi a échoué.
async fn request_stop(python: &Path, pid: u32) -> bool {
    // `-I` : avec `-c`, le dossier courant serait en tête de `sys.path` ; l'isolement complet
    // évite d'importer un module déposé là (l'assistant n'importe que `ctypes` et `sys`).
    #[cfg(windows)]
    let mut cmd = {
        let mut cmd = Command::new(python);
        cmd.arg("-I")
            .arg("-c")
            .arg(CTRL_BREAK_HELPER)
            .arg(pid.to_string())
            .creation_flags(CREATE_NO_WINDOW);
        cmd
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let _ = python;
        let mut cmd = Command::new("kill");
        cmd.arg("-TERM").arg(pid.to_string());
        cmd
    };
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    matches!(
        tokio::time::timeout(Duration::from_secs(10), cmd.status()).await,
        Ok(Ok(status)) if status.success()
    )
}

/// `<sortie>.<pid>.part`, temporaire du convertisseur.
fn part_of(output: &Path, pid: u32) -> PathBuf {
    let mut name = output.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{pid}.part"));
    output.with_file_name(name)
}

/// Arrête le convertisseur : demande polie (fermeture de `stdin` pour Rust, `CTRL_BREAK_EVENT`
/// pour Python), délai de grâce, arrêt forcé, puis nettoyage du temporaire
/// `<sortie>.<pid>.part` (et d'une sortie éventuellement publiée entre-temps).
async fn stop(
    child: &mut Child,
    job: Option<&Job>,
    stdin: &mut Option<ChildStdin>,
    req: &Conversion<'_>,
) {
    if let Some(pid) = child.id() {
        let polite = match req.engine {
            Engine::Python { python, .. } => request_stop(python, pid).await,
            // `--cancel-on-stdin-eof` : la fin de stdin annule. Sans tube (jamais le cas en
            // pratique), pas de demande polie : arrêt forcé direct.
            Engine::Rust { .. } => stdin.take().is_some(),
        };
        let exited = polite
            && tokio::time::timeout(req.grace, child.wait())
                .await
                .is_ok_and(|r| r.is_ok());
        if !exited {
            tracing::warn!("convertisseur {pid} arrêté de force");
            if let Some(job) = job {
                // Tue aussi les sous-processus éventuels du convertisseur.
                job.terminate();
            }
            let _ = child.kill().await;
        }
        remove_any(&part_of(req.output, pid));
    }
    remove_any(req.output);
}

/// Supprime un **fichier** (jamais un dossier) laissé par le convertisseur.
fn remove_any(path: &Path) {
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file()) {
        let _ = std::fs::remove_file(path);
    }
}

/// Erreur rapportée par l'événement `error` du convertisseur.
struct Reported {
    code: String,
    message: String,
}

/// Arguments du convertisseur : jamais d'interpolation de texte libre (le remplissage vient d'un
/// ensemble fermé). Mêmes options pour les deux moteurs ; le script en tête pour Python,
/// `--cancel-on-stdin-eof` en plus pour Rust (option propre au port, passée seulement par le
/// launcher : sans elle, le binaire ne lit jamais stdin).
fn converter_args(req: &Conversion<'_>) -> Vec<OsString> {
    let mut args = Vec::with_capacity(9);
    if let Engine::Python { script, .. } = req.engine {
        args.push(script.as_os_str().to_owned());
    }
    args.extend([
        req.input.as_os_str().to_owned(),
        "-o".into(),
        req.output.as_os_str().to_owned(),
        "--progress-json".into(),
        "--thumb-fit".into(),
        req.thumb_fit.as_arg().into(),
    ]);
    if let Engine::Rust { .. } = req.engine {
        args.push("--cancel-on-stdin-eof".into());
    }
    args
}

/// Lance la conversion et relaie les événements `step` à `on_step`. Annulation via `cancel`.
pub async fn convert(
    req: &Conversion<'_>,
    cancel: &mut watch::Receiver<bool>,
    on_step: &mut (dyn FnMut(&Step) + Send),
) -> AppResult<Converted> {
    if *cancel.borrow() {
        return Err(AppError::Cancelled);
    }
    let cwd = req
        .output
        .parent()
        .ok_or_else(|| AppError::Validation("sortie sans dossier parent".into()))?;
    let mut child = command(&req.engine, cwd)
        .env(
            "FLAM_FFMPEG",
            req.ffmpeg.map_or(OsStr::new(""), Path::as_os_str),
        )
        .args(converter_args(req))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            AppError::ConverterUnavailable(format!(
                "impossible de lancer {} : {e}",
                req.engine.label()
            ))
        })?;
    let stdout = child.stdout.take().expect("stdout redirigé");
    let stderr = child.stderr.take().expect("stderr redirigé");
    // Rust : tube de stdin tenu jusqu'à la fin de la conversion (le fermer annule). Python : `None`.
    let mut stdin = child.stdin.take();
    // Lancé suspendu : rattaché au job avant d'exécuter la moindre instruction. Sans job (échec de
    // l'attache), il est tout de même repris ; si la reprise échoue, il est tué (`kill_on_drop`).
    // Création du job et reprise (instantané des threads du système) sont des appels Win32
    // bloquants : ils s'exécutent hors du runtime (`spawn_blocking`), le `Child` y est déplacé
    // puis rendu. Si la tâche est abandonnée en route, le `Child` est détruit et tué avec elle.
    let attached = tokio::task::spawn_blocking(move || {
        let job = Job::attach(&child);
        let resumed = job::resume(&child);
        (child, job, resumed)
    })
    .await;
    let (mut child, job, resumed) = attached.map_err(|e| {
        AppError::ConverterUnavailable(format!(
            "impossible de préparer {} : {e}",
            req.engine.label()
        ))
    })?;
    if job.is_none() {
        tracing::warn!(
            "Job Object indisponible : l'arrêt forcé se limitera au convertisseur direct"
        );
    }
    if let Err(err) = resumed {
        let _ = child.start_kill();
        return Err(AppError::ConverterUnavailable(format!(
            "impossible de reprendre {} : {err}",
            req.engine.label()
        )));
    }
    let stderr_task = tokio::spawn(read_tail(stderr, STDERR_TAIL));

    let mut reader = BufReader::new(stdout);
    let mut phases: Vec<String> = Vec::new();
    let mut version: Option<String> = None;
    let mut done: Option<Converted> = None;
    let mut reported: Option<Reported> = None;
    let mut line = Vec::new();
    loop {
        line.clear();
        let mut limited = (&mut reader).take(MAX_LINE);
        let read = tokio::select! {
            r = limited.read_until(b'\n', &mut line) => r,
            _ = cancelled(cancel) => {
                stop(&mut child, job.as_ref(), &mut stdin, req).await;
                stderr_task.abort();
                return Err(AppError::Cancelled);
            }
        };
        match read {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let Ok(event) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        let text = |key: &str| event.get(key).and_then(Value::as_str).map(str::to_string);
        match event.get("event").and_then(Value::as_str) {
            Some("start") => {
                version = text("version").and_then(|v| clean_version(&v));
                phases = event
                    .get("phases")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|p| p.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
            }
            Some("step") => {
                let phase = text("phase").unwrap_or_default();
                let index = phases.iter().position(|p| *p == phase).unwrap_or(0);
                let num = |key: &str| event.get(key).and_then(Value::as_u64).unwrap_or(0);
                on_step(&Step {
                    phase,
                    index,
                    phases: phases.len().max(1),
                    done: num("done"),
                    total: num("total"),
                });
            }
            Some("done") => {
                let warnings = event.get("warnings").and_then(Value::as_u64).unwrap_or(0);
                done = Some(Converted {
                    warnings: u32::try_from(warnings).unwrap_or(u32::MAX),
                    version: version.clone(),
                });
            }
            Some("error") => {
                reported = Some(Reported {
                    code: text("code").unwrap_or_else(|| "unknown".into()),
                    message: text("message").unwrap_or_default(),
                });
            }
            _ => {}
        }
    }
    let status = tokio::select! {
        s = child.wait() => s?,
        _ = cancelled(cancel) => {
            stop(&mut child, job.as_ref(), &mut stdin, req).await;
            stderr_task.abort();
            return Err(AppError::Cancelled);
        }
    };
    // Processus terminé : le tube de stdin (Rust) n'annule plus rien.
    drop(stdin);
    let stderr_tail = stderr_task.await.unwrap_or_default();
    let exit = status.code();
    match (exit, done, reported) {
        (Some(0), Some(converted), _) if req.output.is_file() => Ok(converted),
        (Some(130), _, _) => {
            remove_any(req.output);
            Err(AppError::Cancelled)
        }
        (_, _, Some(r)) => {
            remove_any(req.output);
            Err(AppError::Conversion {
                code: r.code,
                message: r.message,
            })
        }
        (code, _, None) => {
            remove_any(req.output);
            let last = stderr_tail
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("aucun message")
                .trim()
                .to_string();
            Err(AppError::Conversion {
                code: code.map_or_else(|| "exit_unknown".into(), |c| format!("exit_{c}")),
                message: last,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pourcentage_global() {
        let step = |index, done, total| Step {
            phase: "x".into(),
            index,
            phases: 4,
            done,
            total,
        };
        assert_eq!(step(0, 0, 10).percent(), 0.0);
        assert_eq!(step(1, 5, 10).percent(), 37.5);
        assert_eq!(step(3, 0, 0).percent(), 100.0);
    }

    #[tokio::test]
    async fn stderr_borne_garde_la_fin() {
        let mut data = vec![b'a'; 3 * 1024 * 1024];
        data.extend_from_slice(
            b"
dernier message",
        );
        let tail = read_tail(std::io::Cursor::new(data), STDERR_TAIL).await;
        assert!(tail.len() <= STDERR_TAIL);
        assert!(tail.ends_with("dernier message"));
        assert_eq!(
            read_tail(std::io::Cursor::new(b"court".to_vec()), 4096).await,
            "court"
        );
    }

    #[test]
    fn version_du_convertisseur_nettoyee() {
        assert_eq!(clean_version(" 1.0.0 ").as_deref(), Some("1.0.0"));
        assert_eq!(clean_version(""), None);
        assert_eq!(
            clean_version(
                "a
b"
            ),
            None
        );
        assert_eq!(clean_version(&"x".repeat(65)), None);
    }

    #[test]
    fn arguments_des_deux_moteurs() {
        let python = Engine::Python {
            python: Path::new("python.exe"),
            script: Path::new("telmi2flam.py"),
        };
        let rust = Engine::Rust {
            exe: Path::new("telmi2flam.exe"),
        };
        let req = |engine, thumb_fit| Conversion {
            engine,
            input: Path::new("in.zip"),
            output: Path::new("out.plain.pk"),
            ffmpeg: None,
            thumb_fit,
            grace: Duration::from_secs(1),
        };
        let common = |fit: &str| -> Vec<OsString> {
            [
                "in.zip",
                "-o",
                "out.plain.pk",
                "--progress-json",
                "--thumb-fit",
                fit,
            ]
            .map(OsString::from)
            .to_vec()
        };
        for (fit, arg) in [
            (ThumbFit::Flou, "flou"),
            (ThumbFit::Rogner, "rogner"),
            (ThumbFit::Bandes, "bandes"),
        ] {
            // Python : le script en tête, puis les options communes.
            let mut expected = vec![OsString::from("telmi2flam.py")];
            expected.extend(common(arg));
            assert_eq!(converter_args(&req(python, fit)), expected);
            // Rust : les mêmes options, plus l'annulation par fin de stdin.
            let mut expected = common(arg);
            expected.push("--cancel-on-stdin-eof".into());
            assert_eq!(converter_args(&req(rust, fit)), expected);
        }
        assert_eq!(python.identity(), Path::new("telmi2flam.py"));
        assert_eq!(rust.identity(), Path::new("telmi2flam.exe"));
    }

    #[test]
    fn temporaire_du_convertisseur() {
        assert_eq!(
            part_of(Path::new("/l/.staging/t.plain.pk"), 42),
            PathBuf::from("/l/.staging/t.plain.pk.42.part")
        );
    }

    /// Panne injectée : exécutable absent (binaire Rust non livré, supprimé ou bloqué ;
    /// interpréteur Python disparu). Erreur `converter_unavailable` qui nomme le moteur, rien
    /// d'écrit.
    #[tokio::test]
    async fn executable_absent_pour_les_deux_moteurs() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("absent").join("telmi2flam.exe");
        let script = dir.path().join("telmi2flam.py");
        let output = dir.path().join("out.plain.pk");
        let input = dir.path().join("in.zip");
        std::fs::write(&input, b"PK").unwrap();
        for (engine, label) in [
            (Engine::Rust { exe: &missing }, "telmi2flam.exe"),
            (
                Engine::Python {
                    python: &missing,
                    script: &script,
                },
                "Python",
            ),
        ] {
            let (_tx, mut rx) = watch::channel(false);
            let err = convert(
                &Conversion {
                    engine,
                    input: &input,
                    output: &output,
                    ffmpeg: None,
                    thumb_fit: ThumbFit::default(),
                    grace: Duration::from_secs(1),
                },
                &mut rx,
                &mut |_| {},
            )
            .await
            .unwrap_err();
            assert_eq!(err.kind(), "converter_unavailable", "{err}");
            assert!(err.to_string().contains(label), "{err}");
        }
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, [OsString::from("in.zip")]);
    }

    /// Faux convertisseur ([`fake`]) lancé avec le moteur voulu. Rust : l'« exécutable » est
    /// l'interpréteur, qui exécute le `__main__.py` de l'archive d'entrée : la ligne de commande
    /// est exactement celle du moteur Rust (`<exe> <entrée> -o … --cancel-on-stdin-eof`).
    struct Banc {
        dir: tempfile::TempDir,
        python: PathBuf,
        script: PathBuf,
        input: PathBuf,
        output: PathBuf,
    }

    impl Banc {
        fn new(mode: &str) -> Option<Self> {
            let python =
                crate::settings::detect_python(&crate::settings::Environment::current()).path?;
            let dir = tempfile::tempdir().unwrap();
            let script = dir.path().join("fake_telmi2flam.py");
            std::fs::write(&script, fake::SCRIPT).unwrap();
            let input = dir.path().join("in.zip");
            std::fs::write(&input, fake::archive(mode, 0)).unwrap();
            // Le faux convertisseur signale qu'il est prêt deux dossiers au-dessus de la sortie.
            let staging = dir.path().join("library").join(".staging");
            std::fs::create_dir_all(&staging).unwrap();
            let output = staging.join("t.plain.pk");
            Some(Self {
                dir,
                python,
                script,
                input,
                output,
            })
        }

        fn engine(&self, rust: bool) -> Engine<'_> {
            if rust {
                Engine::Rust { exe: &self.python }
            } else {
                Engine::Python {
                    python: &self.python,
                    script: &self.script,
                }
            }
        }

        fn ready(&self) -> bool {
            std::fs::read_dir(self.dir.path()).is_ok_and(|d| {
                d.flatten()
                    .any(|e| e.file_name().to_string_lossy().starts_with("ready-"))
            })
        }

        fn staging_is_empty(&self) -> bool {
            std::fs::read_dir(self.output.parent().unwrap())
                .unwrap()
                .next()
                .is_none()
        }

        /// Lance la conversion, annule dès que le faux convertisseur est prêt (`.part` écrit,
        /// annulation armée), puis renvoie le résultat et la durée de l'arrêt.
        async fn cancel_when_ready(
            &self,
            rust: bool,
            grace: Duration,
        ) -> (AppResult<Converted>, Duration) {
            let (tx, mut rx) = watch::channel(false);
            let req = Conversion {
                engine: self.engine(rust),
                input: &self.input,
                output: &self.output,
                ffmpeg: None,
                thumb_fit: ThumbFit::default(),
                grace,
            };
            let canceller = async {
                let deadline = std::time::Instant::now() + Duration::from_secs(30);
                while !self.ready() {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "convertisseur jamais prêt"
                    );
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                let part = std::fs::read_dir(self.output.parent().unwrap())
                    .unwrap()
                    .flatten()
                    .any(|e| e.file_name().to_string_lossy().ends_with(".part"));
                assert!(part, "le .part existe avant le signal de disponibilité");
                tx.send(true).unwrap();
                std::time::Instant::now()
            };
            let mut ignore = |_: &Step| {};
            let (result, asked) = tokio::join!(convert(&req, &mut rx, &mut ignore), canceller);
            (result, asked.elapsed())
        }
    }

    #[tokio::test]
    async fn conversion_par_les_deux_moteurs() {
        for rust in [false, true] {
            let Some(banc) = Banc::new("versioned") else {
                eprintln!("Python introuvable : test ignoré");
                return;
            };
            let (_tx, mut rx) = watch::channel(false);
            let mut phases = Vec::new();
            let converted = convert(
                &Conversion {
                    engine: banc.engine(rust),
                    input: &banc.input,
                    output: &banc.output,
                    ffmpeg: None,
                    thumb_fit: ThumbFit::Bandes,
                    grace: Duration::from_secs(5),
                },
                &mut rx,
                &mut |step| phases.push(step.phase.clone()),
            )
            .await
            .unwrap_or_else(|e| panic!("rust={rust} : {e}"));
            assert_eq!(converted.warnings, 1, "rust={rust}");
            assert_eq!(
                converted.version.as_deref(),
                Some("9.9.9-test"),
                "rust={rust}"
            );
            assert!(
                phases.contains(&"zip".to_string()),
                "rust={rust} : {phases:?}"
            );
            assert!(banc.output.is_file(), "rust={rust}");
            // Le faux convertisseur rend les options reçues : même protocole pour les deux.
            let seen = std::fs::read_to_string(banc.dir.path().join("args.txt")).unwrap();
            assert!(seen.contains("--thumb-fit bandes"), "rust={rust} : {seen}");
            assert_eq!(seen.contains("--cancel-on-stdin-eof"), rust, "{seen}");
        }
    }

    /// Panne injectée : annulation polie. Rust : fermeture de stdin (le faux convertisseur ignore
    /// `CTRL_BREAK_EVENT` dans ce mode, l'arrêt avant le délai de grâce prouve que la fin de
    /// stdin a suffi) ; Python : `CTRL_BREAK_EVENT`.
    #[tokio::test]
    async fn annulation_polie_des_deux_moteurs() {
        for rust in [false, true] {
            let Some(banc) = Banc::new("slow") else {
                eprintln!("Python introuvable : test ignoré");
                return;
            };
            let grace = Duration::from_secs(20);
            let (result, took) = banc.cancel_when_ready(rust, grace).await;
            assert!(
                matches!(result, Err(AppError::Cancelled)),
                "rust={rust} : {result:?}"
            );
            assert!(
                took < Duration::from_secs(10),
                "rust={rust} : arrêt en {took:?}"
            );
            assert!(
                banc.staging_is_empty(),
                "rust={rust} : temporaires nettoyés"
            );
        }
    }

    /// Panne injectée : convertisseur sourd à l'annulation. Arrêt forcé par le job après le
    /// délai de grâce, puis suppression du `.part` par le launcher.
    #[tokio::test]
    async fn arret_force_des_deux_moteurs() {
        for rust in [false, true] {
            let Some(banc) = Banc::new("stubborn") else {
                eprintln!("Python introuvable : test ignoré");
                return;
            };
            let grace = Duration::from_millis(500);
            let (result, took) = banc.cancel_when_ready(rust, grace).await;
            assert!(
                matches!(result, Err(AppError::Cancelled)),
                "rust={rust} : {result:?}"
            );
            assert!(
                took >= grace,
                "rust={rust} : arrêt en {took:?}, avant le délai de grâce"
            );
            assert!(
                took < Duration::from_secs(15),
                "rust={rust} : arrêt en {took:?}"
            );
            assert!(
                banc.staging_is_empty(),
                "rust={rust} : .part supprimé par le launcher"
            );
        }
    }

    /// Vérification réelle avec les convertisseurs livrés (issues #71, #136). Ignorée par
    /// défaut : elle exige une archive de test (`FLAM_TEST_ARCHIVE` = histoire `.zip`) et l'un des
    /// deux moteurs :
    /// - `FLAM_TEST_RUST_CONVERTER` = binaire `telmi2flam.exe` (`cargo build -p telmi2flam`, ou
    ///   `build/telmi2flam/telmi2flam.exe` de `npm run build:converter`), lancé directement
    ///   comme par le launcher (`--cancel-on-stdin-eof`, stdin en tube) ;
    /// - sinon `FLAM_TEST_CONVERTER_DIR` = dossier `telmi2flam/` de `npm run build:converter`
    ///   (avec `python/python.exe`). Pour prouver l'autonomie, lancer le binaire de test avec un
    ///   `PATH` réduit à `C:\Windows\System32` (voir le README du launcher).
    ///
    /// Annulation avec le binaire Rust : demandée à la fin de la phase `zip`, où le binaire
    /// attend (`TELMI2FLAM_TEST_FAULT=wait:zip:end`, binaire de debug ou feature `test-hooks`)
    /// avec son `.part` présent. L'arrêt doit venir de la fermeture de stdin, avant le délai de
    /// grâce (sinon ce serait l'arrêt forcé).
    #[cfg(windows)]
    mod embarque {
        use super::*;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::{Arc, Mutex};

        /// La faute de test passe par l'environnement du processus : les deux tests ne
        /// tournent jamais en même temps.
        static ENV: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

        enum Convertisseur {
            Python { python: PathBuf, script: PathBuf },
            Rust(PathBuf),
        }

        impl Convertisseur {
            fn engine(&self) -> Engine<'_> {
                match self {
                    Self::Python { python, script } => Engine::Python { python, script },
                    Self::Rust(exe) => Engine::Rust { exe },
                }
            }
        }

        fn environnement() -> Option<(Convertisseur, PathBuf)> {
            let archive = PathBuf::from(std::env::var_os("FLAM_TEST_ARCHIVE")?);
            if let Some(exe) = std::env::var_os("FLAM_TEST_RUST_CONVERTER") {
                return Some((Convertisseur::Rust(PathBuf::from(exe)), archive));
            }
            let dir = PathBuf::from(std::env::var_os("FLAM_TEST_CONVERTER_DIR")?);
            let conv = Convertisseur::Python {
                python: dir.join("python").join("python.exe"),
                script: dir.join("telmi2flam.py"),
            };
            Some((conv, archive))
        }

        #[tokio::test]
        #[ignore = "exige FLAM_TEST_ARCHIVE et FLAM_TEST_RUST_CONVERTER ou FLAM_TEST_CONVERTER_DIR"]
        async fn conversion_avec_le_convertisseur_livre() {
            let _env = ENV.lock().await;
            let (conv, archive) = environnement().expect("variables d'environnement manquantes");
            let out = tempfile::tempdir().unwrap();
            let output = out.path().join("histoire.plain.pk");
            let (_tx, mut rx) = watch::channel(false);
            let mut steps = 0usize;
            let converted = convert(
                &Conversion {
                    engine: conv.engine(),
                    input: &archive,
                    output: &output,
                    ffmpeg: None,
                    thumb_fit: ThumbFit::default(),
                    grace: Duration::from_secs(5),
                },
                &mut rx,
                &mut |_| steps += 1,
            )
            .await
            .expect("conversion");
            assert!(steps > 0, "aucune progression reçue");
            assert!(output.metadata().unwrap().len() > 0);
            assert!(converted.version.is_some(), "version de l'événement start");
            println!(
                "conversion ok : {} avertissement(s), {steps} étapes, version {:?}",
                converted.warnings, converted.version
            );
        }

        #[tokio::test]
        #[ignore = "exige FLAM_TEST_ARCHIVE et FLAM_TEST_RUST_CONVERTER ou FLAM_TEST_CONVERTER_DIR"]
        async fn annulation_avec_le_convertisseur_livre() {
            let _env = ENV.lock().await;
            let (conv, archive) = environnement().expect("variables d'environnement manquantes");
            let rust = matches!(conv, Convertisseur::Rust(_));
            let out = tempfile::tempdir().unwrap();
            let output = out.path().join("histoire.plain.pk");
            if rust {
                // SAFETY (édition 2021 : fonction sûre) : lu seulement par le binaire lancé
                // ci-dessous ; l'autre test qui lance un convertisseur attend le verrou.
                std::env::set_var("TELMI2FLAM_TEST_FAULT", "wait:zip:end");
            }
            let (tx, mut rx) = watch::channel(false);
            let tx = Arc::new(Mutex::new(tx));
            let part_seen = AtomicBool::new(false);
            let asked = Mutex::new(None);
            let grace = Duration::from_secs(10);
            let dir_out = out.path().to_path_buf();
            let result = convert(
                &Conversion {
                    engine: conv.engine(),
                    input: &archive,
                    output: &output,
                    ffmpeg: None,
                    thumb_fit: ThumbFit::default(),
                    grace,
                },
                &mut rx,
                &mut |step| {
                    if rust && !(step.phase == "zip" && step.done == step.total) {
                        return;
                    }
                    if rust {
                        let part = std::fs::read_dir(&dir_out)
                            .unwrap()
                            .filter_map(|e| e.ok())
                            .any(|e| e.file_name().to_string_lossy().ends_with(".part"));
                        part_seen.store(part, Ordering::SeqCst);
                    }
                    asked
                        .lock()
                        .unwrap()
                        .get_or_insert_with(std::time::Instant::now);
                    let _ = tx.lock().unwrap().send(true);
                },
            )
            .await;
            std::env::remove_var("TELMI2FLAM_TEST_FAULT");
            assert!(matches!(result, Err(AppError::Cancelled)), "{result:?}");
            if rust {
                assert!(
                    part_seen.load(Ordering::SeqCst),
                    "aucun .part pendant l'attente : l'annulation ne teste pas le nettoyage \
                     (FLAM_TEST_RUST_CONVERTER : binaire de debug ou avec test-hooks)"
                );
                let took = asked
                    .lock()
                    .unwrap()
                    .expect("annulation demandée")
                    .elapsed();
                assert!(took < grace, "arrêt en {took:?} : stdin fermé sans effet");
            }
            let restes: Vec<_> = std::fs::read_dir(out.path()).unwrap().collect();
            assert!(restes.is_empty(), "fichiers résiduels : {restes:?}");
        }
    }
}

/// Faux `telmi2flam` des tests (ce module et `install::tests`), commun aux deux moteurs. Le
/// comportement est lu dans l'archive d'entrée (`mode=<x>;`).
///
/// - Moteur Python : `python … fake_telmi2flam.py <entrée> -o <sortie> …` ;
/// - moteur Rust : l'« exécutable » est l'interpréteur, et l'archive d'entrée contient ce même
///   script en `__main__.py` : `python <entrée> -o <sortie> … --cancel-on-stdin-eof` l'exécute.
///   Dans ce mode, seule la fin de stdin annule (`CTRL_BREAK_EVENT` est ignoré), comme le
///   binaire avec `--cancel-on-stdin-eof`.
///
/// Modes : `ok` (par défaut), `noise` (lignes non JSON, stderr en cp1252), `versioned`
/// (`start.version`), `fail` (`missing_asset`, 5), `crash` (code 6 sans événement `error`),
/// `slow` (annulation polie), `stubborn` (sourd à l'annulation), `tree` (sourd, avec un
/// sous-processus). En mode `versioned`, les options reçues sont écrites dans `args.txt`, deux
/// dossiers au-dessus de la sortie.
#[cfg(test)]
pub(crate) mod fake {
    use std::io::Write as _;

    pub const SCRIPT: &str = r#"
import json, os, signal, sys, threading, time
args = sys.argv[1:]
rust = "--cancel-on-stdin-eof" in args
src = sys.argv[0] if rust else args[0]
out = args[args.index("-o") + 1]
mode = open(src, "rb").read().split(b"mode=")[1].split(b";")[0].decode()
def ev(**k):
    print(json.dumps(k), flush=True)
part = "%s.%d.part" % (out, os.getpid())
if mode == "noise":
    print("ceci n'est pas du JSON", flush=True)
    sys.stderr.buffer.write("caf\xe9 en cp1252\n".encode("cp1252")); sys.stderr.flush()
extra = {"version": "9.9.9-test"} if mode == "versioned" else {}
if mode == "tree":
    import subprocess
    g = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(120)"], stdin=subprocess.DEVNULL)
    open(out + ".gpid", "w").write(str(g.pid))
ev(event="start", total=4, phases=["images", "audio", "scripts", "zip"], input=src, **extra)
if mode == "fail":
    ev(event="error", code="missing_asset", message="image manquante : s1.png", exit=5)
    sys.exit(5)
if mode == "crash":
    sys.stderr.write("Traceback: boum\n"); sys.exit(6)
if mode in ("slow", "stubborn", "tree"):
    open(part, "wb").write(b"partiel")
    polite = mode == "slow"
    def stop(*a):
        os.remove(part)
        ev(event="error", code="cancelled", message="annule", exit=130)
        os._exit(130)
    brk = getattr(signal, "SIGBREAK", signal.SIGTERM)
    if rust:
        signal.signal(brk, signal.SIG_IGN)
        def watch():
            sys.stdin.buffer.read()
            if polite:
                stop()
        threading.Thread(target=watch, daemon=True).start()
    else:
        signal.signal(brk, stop if polite else signal.SIG_IGN)
    ev(event="step", phase="images", done=0, total=10)
    # Poignée de main : annulation armée et `.part` écrit (deux dossiers au-dessus de la
    # sortie, hors de `.staging`), le test peut annuler.
    open(os.path.join(os.path.dirname(out), "..", "..", "ready-%d" % os.getpid()), "wb").close()
    for _ in range(600):
        time.sleep(0.05)
    sys.exit(0)
if mode == "versioned":
    open(os.path.join(os.path.dirname(out), "..", "..", "args.txt"), "w").write(" ".join(args))
for phase in ["images", "audio", "scripts", "zip"]:
    ev(event="step", phase=phase, done=0, total=2)
    ev(event="unknown_event", x=1)
    ev(event="step", phase=phase, done=2, total=2)
if rust:
    # Le tube de stdin doit rester ouvert jusqu'à la fin : fermé trop tôt, c'est une annulation
    # (comme le binaire), donc un échec visible des tests de conversion réussie.
    eof = threading.Event()
    def early():
        sys.stdin.buffer.read()
        eof.set()
    threading.Thread(target=early, daemon=True).start()
    if eof.wait(0.3):
        ev(event="error", code="cancelled", message="stdin ferme avant la fin", exit=130)
        sys.exit(130)
open(part, "wb").write(b"PK\x03\x04plain.pk converti")
os.replace(part, out)
ev(event="done", output=out, size=os.path.getsize(out), warnings=1)
"#;

    /// Archive zip « stored » réelle : `mode.txt` (lu par le faux convertisseur, en premier
    /// pour que `mode=` y soit trouvé avant le script), `pad.bin` (`pad` octets) et le script en
    /// `__main__.py` (moteur Rust).
    pub fn archive(mode: &str, pad: usize) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("mode.txt", options).unwrap();
        zip.write_all(format!("mode={mode};").as_bytes()).unwrap();
        zip.start_file("pad.bin", options).unwrap();
        zip.write_all(&vec![b'z'; pad]).unwrap();
        zip.start_file("__main__.py", options).unwrap();
        zip.write_all(SCRIPT.as_bytes()).unwrap();
        zip.finish().unwrap().into_inner()
    }
}

#[cfg(all(test, windows))]
mod job_tests {
    use super::*;
    use std::os::windows::process::CommandExt as _;

    /// Rattaché pendant la suspension, repris ensuite : le processus est dans le job avant sa
    /// première instruction, et s'exécute normalement une fois repris.
    #[tokio::test]
    async fn lance_suspendu_rattache_au_job_puis_repris() {
        use windows_sys::Win32::System::JobObjects::IsProcessInJob;
        let mut std_cmd = std::process::Command::new("cmd");
        std_cmd
            .args(["/C", "exit 3"])
            .creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW);
        let mut child = Command::from(std_cmd).kill_on_drop(true).spawn().unwrap();
        let job = Job::attach(&child).expect("job");
        let mut in_job = 0;
        // SAFETY : poignées valides (processus et job) tant que `child` et `job` existent.
        let ok =
            unsafe { IsProcessInJob(child.raw_handle().unwrap() as _, job.raw(), &mut in_job) };
        assert!(ok != 0 && in_job != 0, "dans le job avant la reprise");
        // Suspendu : il n'a pas pu sortir.
        let early = tokio::time::timeout(Duration::from_millis(300), child.wait()).await;
        assert!(early.is_err(), "suspendu : ne s'exécute pas");
        job::resume(&child).unwrap();
        let status = tokio::time::timeout(Duration::from_secs(20), child.wait())
            .await
            .expect("repris : il se termine")
            .unwrap();
        assert_eq!(status.code(), Some(3));
    }
}
