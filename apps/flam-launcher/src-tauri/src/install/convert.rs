//! Conversion par `telmi2flam.py --progress-json` (issue #14) en processus enfant.
//!
//! - Lancement sans console visible : `CREATE_NO_WINDOW`, et `CREATE_NEW_PROCESS_GROUP` pour
//!   pouvoir viser le seul convertisseur avec `CTRL_BREAK_EVENT`.
//! - stdout : un événement JSON par ligne, lu ligne par ligne ; une ligne non JSON (ou un
//!   événement inconnu) est ignorée. Les messages affichés viennent de ces événements.
//! - stderr : page de code locale sous Windows, donc décodé en tolérant les erreurs
//!   (`PYTHONIOENCODING=utf-8` est aussi demandé) et gardé seulement comme repli (fin du flux).
//! - Arrêt forcé : le convertisseur est rattaché à un Job Object Windows
//!   (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`) ; l'arrêt forcé termine le job, donc aussi ses
//!   sous-processus.
//! - Annulation : `CTRL_BREAK_EVENT` (le convertisseur nettoie ses temporaires et sort avec
//!   130), puis arrêt forcé après un délai de grâce, puis suppression de `<sortie>.<pid>.part`.
//!
//! Une application graphique n'a pas de console et le convertisseur, lancé avec
//! `CREATE_NO_WINDOW`, a la sienne (cachée) : `GenerateConsoleCtrlEvent` ne peut viser qu'un
//! processus de **sa** console. L'événement est donc envoyé par un court assistant Python
//! (même interpréteur) lancé sans fenêtre, qui quitte sa console, s'attache à celle du
//! convertisseur, s'immunise contre l'événement puis l'émet vers le groupe du convertisseur.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::watch;

use super::download::cancelled;
use crate::{AppError, AppResult};

/// Ligne JSON la plus longue lue (au-delà : tronquée, donc ignorée).
const MAX_LINE: u64 = 1024 * 1024;
/// Fin de stderr conservée pour les messages de repli.
const STDERR_TAIL: usize = 4096;

#[cfg(windows)]
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Assistant d'envoi de `CTRL_BREAK_EVENT` (voir l'en-tête du module). Code de sortie 0 si
/// l'événement a été émis.
#[cfg(windows)]
const CTRL_BREAK_HELPER: &str = "import ctypes,sys\n\
k=ctypes.windll.kernel32\n\
pid=int(sys.argv[1])\n\
k.FreeConsole()\n\
ok=k.AttachConsole(pid) and k.SetConsoleCtrlHandler(None,True) and k.GenerateConsoleCtrlEvent(1,pid)\n\
sys.exit(0 if ok else 1)\n";

/// Une conversion à lancer.
pub struct Conversion<'a> {
    pub python: &'a Path,
    pub script: &'a Path,
    pub input: &'a Path,
    /// Sortie `.plain.pk` (publication atomique par le convertisseur).
    pub output: &'a Path,
    /// Délai laissé au convertisseur après `CTRL_BREAK_EVENT` avant l'arrêt forcé.
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

/// Job Object Windows : fermer la dernière poignée (ou le terminer) tue tout le job.
#[cfg(windows)]
struct Job(windows_sys::Win32::Foundation::HANDLE);

// SAFETY : une poignée de Job Object peut être utilisée depuis n'importe quel thread.
#[cfg(windows)]
unsafe impl Send for Job {}
#[cfg(windows)]
unsafe impl Sync for Job {}

#[cfg(windows)]
impl Job {
    /// Crée un job `KILL_ON_JOB_CLOSE` et y rattache `child`. `None` si l'une des étapes échoue
    /// (l'arrêt se limite alors au processus direct).
    fn attach(child: &Child) -> Option<Self> {
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };
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

    fn terminate(&self) {
        // SAFETY : poignée de job valide tant que `self` existe.
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.0, 1);
        }
    }
}

#[cfg(windows)]
impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY : poignée possédée, fermée une seule fois.
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(not(windows))]
struct Job;

#[cfg(not(windows))]
impl Job {
    fn attach(_child: &Child) -> Option<Self> {
        None
    }
    fn terminate(&self) {}
}

/// Interpréteur durci, lancé depuis `cwd` (un dossier de l'application) :
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
fn command(python: &Path, cwd: &Path) -> Command {
    let mut cmd = Command::new(python);
    cmd.args(["-E", "-s", "-B", "-u", "-X", "utf8"])
        .current_dir(cwd)
        .stdin(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
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

/// Arrête le convertisseur : demande polie, délai de grâce, arrêt forcé, puis nettoyage du
/// temporaire `<sortie>.<pid>.part` (et d'une sortie éventuellement publiée entre-temps).
async fn stop(child: &mut Child, job: Option<&Job>, req: &Conversion<'_>) {
    if let Some(pid) = child.id() {
        let polite = request_stop(req.python, pid).await;
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
    let mut child = command(req.python, cwd)
        .arg(req.script)
        .arg(req.input)
        .arg("-o")
        .arg(req.output)
        .arg("--progress-json")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            AppError::ConverterUnavailable(format!(
                "impossible de lancer Python ({}) : {e}",
                req.python.display()
            ))
        })?;
    let stdout = child.stdout.take().expect("stdout redirigé");
    let stderr = child.stderr.take().expect("stderr redirigé");
    let job = Job::attach(&child);
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
                stop(&mut child, job.as_ref(), req).await;
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
            stop(&mut child, job.as_ref(), req).await;
            stderr_task.abort();
            return Err(AppError::Cancelled);
        }
    };
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
    fn temporaire_du_convertisseur() {
        assert_eq!(
            part_of(Path::new("/l/.staging/t.plain.pk"), 42),
            PathBuf::from("/l/.staging/t.plain.pk.42.part")
        );
    }

    /// Vérification réelle avec le convertisseur embarqué (issue #71). Ignorée par défaut : elle
    /// exige l'artefact de `npm run build:converter` et une archive de test.
    /// `FLAM_TEST_CONVERTER_DIR` = dossier `telmi2flam/` (avec `python/python.exe`) et
    /// `FLAM_TEST_ARCHIVE` = histoire `.zip`. Pour prouver l'autonomie, lancer le binaire de test
    /// avec un `PATH` réduit à `C:\Windows\System32` (voir le README du launcher).
    #[cfg(windows)]
    mod embarque {
        use super::*;
        use std::sync::{Arc, Mutex};

        fn environnement() -> Option<(PathBuf, PathBuf)> {
            let dir = PathBuf::from(std::env::var_os("FLAM_TEST_CONVERTER_DIR")?);
            let archive = PathBuf::from(std::env::var_os("FLAM_TEST_ARCHIVE")?);
            Some((dir, archive))
        }

        #[tokio::test]
        #[ignore = "exige FLAM_TEST_CONVERTER_DIR et FLAM_TEST_ARCHIVE"]
        async fn conversion_avec_python_embarque() {
            let (dir, archive) = environnement().expect("variables d'environnement manquantes");
            let out = tempfile::tempdir().unwrap();
            let output = out.path().join("histoire.plain.pk");
            let python = dir.join("python").join("python.exe");
            let script = dir.join("telmi2flam.py");
            let (_tx, mut rx) = watch::channel(false);
            let mut steps = 0usize;
            let converted = convert(
                &Conversion {
                    python: &python,
                    script: &script,
                    input: &archive,
                    output: &output,
                    grace: Duration::from_secs(5),
                },
                &mut rx,
                &mut |_| steps += 1,
            )
            .await
            .expect("conversion");
            assert!(steps > 0, "aucune progression reçue");
            assert!(output.metadata().unwrap().len() > 0);
            println!(
                "conversion ok : {} avertissement(s), {steps} étapes",
                converted.warnings
            );
        }

        #[tokio::test]
        #[ignore = "exige FLAM_TEST_CONVERTER_DIR et FLAM_TEST_ARCHIVE"]
        async fn annulation_avec_python_embarque() {
            let (dir, archive) = environnement().expect("variables d'environnement manquantes");
            let out = tempfile::tempdir().unwrap();
            let output = out.path().join("histoire.plain.pk");
            let python = dir.join("python").join("python.exe");
            let script = dir.join("telmi2flam.py");
            let (tx, mut rx) = watch::channel(false);
            let tx = Arc::new(Mutex::new(tx));
            let result = convert(
                &Conversion {
                    python: &python,
                    script: &script,
                    input: &archive,
                    output: &output,
                    grace: Duration::from_secs(10),
                },
                &mut rx,
                &mut |_| {
                    let _ = tx.lock().unwrap().send(true);
                },
            )
            .await;
            assert!(matches!(result, Err(AppError::Cancelled)), "{result:?}");
            let restes: Vec<_> = std::fs::read_dir(out.path()).unwrap().collect();
            assert!(restes.is_empty(), "fichiers résiduels : {restes:?}");
        }
    }
}
