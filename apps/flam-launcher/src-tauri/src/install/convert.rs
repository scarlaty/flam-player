//! Conversion par `telmi2flam.py --progress-json` (issue #14) en processus enfant.
//!
//! - Lancement sans console visible : `CREATE_NO_WINDOW`, et `CREATE_NEW_PROCESS_GROUP` pour
//!   pouvoir viser le seul convertisseur avec `CTRL_BREAK_EVENT`.
//! - stdout : un événement JSON par ligne, lu ligne par ligne ; une ligne non JSON (ou un
//!   événement inconnu) est ignorée. Les messages affichés viennent de ces événements.
//! - stderr : page de code locale sous Windows, donc décodé en tolérant les erreurs
//!   (`PYTHONIOENCODING=utf-8` est aussi demandé) et gardé seulement comme repli (fin du flux).
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
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
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
}

/// Interpréteur durci, lancé depuis `cwd` (un dossier de l'application) :
/// - `-E` : variables `PYTHON*` ignorées (`PYTHONPATH`, `PYTHONSTARTUP`…) ;
/// - `-s` : pas de `site-packages` utilisateur ;
/// - `-u` et `-X utf8` : flux non tamponnés et UTF-8 (remplacent `PYTHONUNBUFFERED` et
///   `PYTHONIOENCODING`, ignorés avec `-E`).
///
/// `-I` (isolement complet) n'est pas utilisable pour `telmi2flam.py` : il retire aussi le
/// dossier du script de `sys.path`, d'où viennent ses modules (`lif`, `mp3map`). Le dossier
/// courant, lui, n'est pas dans `sys.path` quand on lance un script.
fn command(python: &Path, cwd: &Path) -> Command {
    let mut cmd = Command::new(python);
    cmd.args(["-E", "-s", "-u", "-X", "utf8"])
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
async fn stop(child: &mut Child, req: &Conversion<'_>) {
    if let Some(pid) = child.id() {
        let polite = request_stop(req.python, pid).await;
        let exited = polite
            && tokio::time::timeout(req.grace, child.wait())
                .await
                .is_ok_and(|r| r.is_ok());
        if !exited {
            tracing::warn!("convertisseur {pid} arrêté de force");
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
    let mut stderr = child.stderr.take().expect("stderr redirigé");
    let stderr_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes).await;
        let start = bytes.len().saturating_sub(STDERR_TAIL);
        String::from_utf8_lossy(&bytes[start..]).into_owned()
    });

    let mut reader = BufReader::new(stdout);
    let mut phases: Vec<String> = Vec::new();
    let mut done: Option<Converted> = None;
    let mut reported: Option<Reported> = None;
    let mut line = Vec::new();
    loop {
        line.clear();
        let mut limited = (&mut reader).take(MAX_LINE);
        let read = tokio::select! {
            r = limited.read_until(b'\n', &mut line) => r,
            _ = cancelled(cancel) => {
                stop(&mut child, req).await;
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
            stop(&mut child, req).await;
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

    #[test]
    fn temporaire_du_convertisseur() {
        assert_eq!(
            part_of(Path::new("/l/.staging/t.plain.pk"), 42),
            PathBuf::from("/l/.staging/t.plain.pk.42.part")
        );
    }
}
