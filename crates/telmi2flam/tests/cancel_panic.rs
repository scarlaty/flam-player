//! Annulation et panique du binaire (#132) : à chaque phase, l'annulation sort avec 130 et la
//! panique avec 6, sans `.part` ni nouvelle sortie, l'ancienne sortie intacte ; l'annulation
//! arrive par le jeton (faute injectée), par la fin de stdin (`--cancel-on-stdin-eof`,
//! protocole du launcher) ou par `CTRL_BREAK` / `SIGINT` / `SIGTERM`.
//!
//! Les fautes passent par `TELMI2FLAM_TEST_FAULT` (voir `src/main.rs`, actif en debug ou avec
//! la feature `test-hooks`) : sans elles, ces tests ne prouveraient rien.
#![cfg(any(debug_assertions, feature = "test-hooks"))]

use std::io::Cursor;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use serde_json::Value;
use telmi2flam::archive::Archive;

const VECTORS: &[u8] = include_bytes!("data/package_cases.zip");

/// Points de faute : (phase, début, `done` ou fin), dans l'ordre de la conversion. `zip:1` :
/// le `.part` du `.plain.pk` est ouvert, en cours d'écriture ; `zip:end` : le `.plain.pk` et
/// le dossier `.plain` sont écrits dans leurs `.part` (dernier fichier du dossier encore
/// ouvert), rien n'est publié.
const POINTS: &[&str] = &[
    "images:begin",
    "images:end",
    "audio:begin",
    "audio:end",
    "scripts:begin",
    "scripts:end",
    "zip:begin",
    "zip:1",
    "zip:end",
];

struct Work(PathBuf);

impl Drop for Work {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Dossier avec `in.zip` (histoire avec images, audio, inventaire) et une ancienne sortie.
fn work(name: &str) -> Work {
    let d = std::env::temp_dir().join(format!("t2f-cancel-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("out.plain")).unwrap();
    let mut zip = Archive::new(Cursor::new(VECTORS), "package_cases.zip".into()).unwrap();
    let data = zip.read("cases/ok_inventory.zip").unwrap().unwrap();
    std::fs::write(d.join("in.zip"), data).unwrap();
    std::fs::write(d.join("out.plain.pk"), b"ancien").unwrap();
    std::fs::write(d.join("out.plain/version"), b"ancien").unwrap();
    Work(d)
}

fn command(dir: &Path, fault: Option<&str>, extra: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_telmi2flam"));
    c.args([
        "in.zip",
        "-o",
        "out.plain.pk",
        "--plain",
        "--progress-json",
        "--thumb-fit",
        "flou",
    ])
    .args(extra)
    .current_dir(dir)
    .env_remove("TELMI2FLAM_TEST_FAULT");
    if let Some(f) = fault {
        c.env("TELMI2FLAM_TEST_FAULT", f);
    }
    c
}

/// Fichiers `.part` restants dans `dir`.
fn parts(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".part"))
        .collect()
}

/// Dernier événement JSON de `stdout`.
fn last_event(stdout: &[u8]) -> Value {
    let text = String::from_utf8_lossy(stdout);
    let line = text.lines().last().expect("aucun événement");
    serde_json::from_str(line).unwrap_or_else(|e| panic!("{line:?} : {e}"))
}

/// Aucun `.part`, ancienne sortie intacte.
fn assert_clean(dir: &Path, what: &str) {
    assert!(parts(dir).is_empty(), "{what} : {:?}", parts(dir));
    assert_eq!(
        std::fs::read(dir.join("out.plain.pk")).unwrap(),
        b"ancien",
        "{what}"
    );
    assert_eq!(
        std::fs::read(dir.join("out.plain/version")).unwrap(),
        b"ancien",
        "{what}"
    );
    assert_eq!(
        std::fs::read_dir(dir.join("out.plain")).unwrap().count(),
        1,
        "{what} : dossier .plain modifié"
    );
}

/// Code 130, événement `cancelled`, message humain de Python, rien de laissé.
fn assert_cancelled(dir: &Path, out: &std::process::Output, what: &str) {
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(130), "{what} : {stderr}");
    let ev = last_event(&out.stdout);
    assert_eq!(ev["event"], "error", "{what}");
    assert_eq!(ev["code"], "cancelled", "{what}");
    assert_eq!(ev["exit"], 130, "{what}");
    assert!(
        stderr.contains("Annule : aucune sortie partielle conservee."),
        "{what} : {stderr}"
    );
    assert_clean(dir, what);
}

#[test]
fn reference_sans_faute() {
    let w = work("ref");
    let out = command(&w.0, None, &[]).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(last_event(&out.stdout)["event"], "done");
    // Chaque point de faute existe bien dans le flux (sinon les tests suivants ne prouvent
    // rien) : début et fin de chaque phase.
    let text = String::from_utf8_lossy(&out.stdout);
    for phase in ["images", "audio", "scripts", "zip"] {
        let steps: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .filter(|e| e["phase"] == phase)
            .collect();
        assert_eq!(steps.first().unwrap()["done"], 0, "{phase}");
        let last = steps.last().unwrap();
        assert_eq!(last["done"], last["total"], "{phase}");
        assert!(last["total"].as_u64().unwrap() > 0, "{phase}");
    }
    assert!(parts(&w.0).is_empty());
}

#[test]
fn annulation_a_chaque_phase() {
    for point in POINTS {
        let w = work(&format!("annule-{}", point.replace(':', "-")));
        let out = command(&w.0, Some(&format!("cancel:{point}")), &[])
            .output()
            .unwrap();
        assert_cancelled(&w.0, &out, point);
    }
}

#[test]
fn panique_a_chaque_phase() {
    for point in POINTS {
        let w = work(&format!("panique-{}", point.replace(':', "-")));
        let out = command(&w.0, Some(&format!("panic:{point}")), &[])
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(6), "{point} : {stderr}");
        let ev = last_event(&out.stdout);
        assert_eq!(ev["event"], "error", "{point}");
        assert_eq!(ev["code"], "internal_error", "{point}");
        assert_eq!(ev["exit"], 6, "{point}");
        assert!(
            ev["message"]
                .as_str()
                .unwrap()
                .starts_with("panic: faute injectee"),
            "{ev}"
        );
        assert!(stderr.contains("ERREUR inattendue (panic) : "), "{stderr}");
        assert_clean(&w.0, point);
    }
}

/// Panique sans `--progress-json` : code 6, stdout vide, rien de laissé.
#[test]
fn panique_sans_progress_json() {
    let w = work("panique-humain");
    let out = Command::new(env!("CARGO_BIN_EXE_telmi2flam"))
        .args(["in.zip", "-o", "out.plain.pk", "--plain"])
        .current_dir(&w.0)
        .env("TELMI2FLAM_TEST_FAULT", "panic:zip:end")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(6));
    assert!(out.stdout.is_empty());
    assert_clean(&w.0, "sans --progress-json");
}

/// Lance le binaire, bloqué (`wait`) à la fin de la phase `zip`, et rend la main quand
/// l'événement correspondant est lu : les `.part` existent alors.
fn spawn_waiting(dir: &Path, extra: &[&str], setup: impl FnOnce(&mut Command)) -> Child {
    let mut c = command(dir, Some("wait:zip:end"), extra);
    c.stdout(Stdio::piped()).stderr(Stdio::piped());
    setup(&mut c);
    let mut child = c.spawn().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    loop {
        let line = lines
            .next()
            .expect("fin du flux avant zip")
            .expect("lecture de stdout");
        let ev: Value = serde_json::from_str(&line).unwrap();
        if ev["phase"] == "zip" && ev["done"] == ev["total"] {
            break;
        }
    }
    // Le reste du flux est lu par un fil (le dernier événement est vérifié ensuite).
    let rest = std::thread::spawn(move || lines.map_while(Result::ok).collect::<Vec<_>>());
    let pid = child.id();
    let expected = [
        format!("out.plain.pk.{pid}.part"),
        format!("out.plain.{pid}.part"),
    ];
    let mut found = parts(dir);
    found.sort();
    let mut want = expected.to_vec();
    want.sort();
    assert_eq!(found, want, "temporaires attendus pendant l'attente");
    REST.with(|r| *r.borrow_mut() = Some(rest));
    child
}

thread_local! {
    static REST: std::cell::RefCell<Option<std::thread::JoinHandle<Vec<String>>>> =
        const { std::cell::RefCell::new(None) };
}

/// Attend la fin du binaire et rend une sortie complète (stdout : le reste du flux).
fn finish(child: Child) -> std::process::Output {
    let mut out = child.wait_with_output().unwrap();
    let rest = REST
        .with(|r| r.borrow_mut().take())
        .unwrap()
        .join()
        .unwrap();
    out.stdout = rest.join("\n").into_bytes();
    out
}

/// Protocole du launcher : stdin en tube, fermé pour annuler pendant l'écriture.
#[test]
fn annulation_par_fin_de_stdin() {
    let w = work("stdin");
    let mut child = spawn_waiting(&w.0, &["--cancel-on-stdin-eof"], |c| {
        c.stdin(Stdio::piped());
    });
    drop(child.stdin.take());
    let out = finish(child);
    assert_cancelled(&w.0, &out, "fin de stdin");
}

/// Sans `--cancel-on-stdin-eof`, un stdin fermé ou vide n'annule rien (CI, `nohup`).
#[test]
fn stdin_ferme_sans_option_ignore() {
    let w = work("stdin-ignore");
    let out = command(&w.0, None, &[])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(parts(&w.0).is_empty());
    assert_ne!(std::fs::read(w.0.join("out.plain.pk")).unwrap(), b"ancien");
}

/// Avec l'option, un stdin déjà fermé annule : le binaire attend au début des images
/// (`wait`) jusqu'à ce que le fil de stdin lève le jeton, sans course avec la conversion.
#[test]
fn stdin_deja_ferme_avec_option_annule() {
    let w = work("stdin-null");
    let out = command(&w.0, Some("wait:images:begin"), &["--cancel-on-stdin-eof"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_cancelled(&w.0, &out, "stdin nul");
}

/// Des octets reçus sur stdin ne déclenchent rien : seule la fin annule.
#[test]
fn octets_sur_stdin_sans_effet() {
    use std::io::Write;
    let w = work("stdin-octets");
    let mut child = spawn_waiting(&w.0, &["--cancel-on-stdin-eof"], |c| {
        c.stdin(Stdio::piped());
    });
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(b"bonjour\nq\n").unwrap();
    stdin.flush().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert!(
        child.try_wait().unwrap().is_none(),
        "des octets sur stdin ont arrêté le binaire"
    );
    drop(stdin);
    let out = finish(child);
    assert_cancelled(&w.0, &out, "fin de stdin après des octets");
}
#[cfg(windows)]
mod console {
    use super::*;
    use std::os::windows::process::CommandExt as _;

    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CTRL_BREAK_EVENT: u32 = 1;

    #[link(name = "kernel32")]
    extern "system" {
        fn GenerateConsoleCtrlEvent(event: u32, group: u32) -> i32;
    }

    /// `CTRL_BREAK_EVENT` envoyé au groupe du binaire pendant l'écriture (ce que fait
    /// l'assistant Python du launcher) : `.part` supprimés, code 130. Sans console attachée
    /// (certains environnements de CI), l'envoi échoue et le test le signale sans échouer.
    #[test]
    fn annulation_par_ctrl_break() {
        let w = work("ctrl-break");
        let mut child = spawn_waiting(&w.0, &[], |c| {
            c.creation_flags(CREATE_NEW_PROCESS_GROUP);
        });
        // SAFETY : appel sans pointeur ; le groupe est celui du processus enfant.
        let sent = unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, child.id()) } != 0;
        if !sent {
            eprintln!(
                "CTRL_BREAK non envoyé (pas de console) : {}",
                std::io::Error::last_os_error()
            );
            let _ = child.kill();
            let _ = finish(child);
            return;
        }
        let out = finish(child);
        assert_cancelled(&w.0, &out, "CTRL_BREAK");
    }
}

#[cfg(unix)]
mod signals {
    use super::*;

    extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }

    #[test]
    fn annulation_par_sigint_et_sigterm() {
        for (sig, name) in [(2, "SIGINT"), (15, "SIGTERM")] {
            let w = work(name);
            let child = spawn_waiting(&w.0, &[], |_| {});
            // SAFETY : signal envoyé à notre enfant, encore vivant (il attend l'annulation).
            assert_eq!(unsafe { kill(child.id() as i32, sig) }, 0);
            let out = finish(child);
            assert_cancelled(&w.0, &out, name);
        }
    }
}
