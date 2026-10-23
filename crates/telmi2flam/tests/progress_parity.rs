//! Parité du protocole du binaire avec Python (#131) : événements `--progress-json`, lignes
//! `  ! …` de stderr et codes de sortie. Les vecteurs `tests/data/progress_cases.json` sont
//! produits par `tests/data/gen_progress_cases.py` (CLI `telmi2flam.py`, Python 3.14.8 +
//! Pillow 12.3.0) sur les archives de `tests/data/package_cases.zip`.
//!
//! Normalisation commune (horodatages et espacements neutralisés) : `input`, `output`,
//! `version` remplacés, `message` de `error` retiré, étapes intermédiaires (ni `done == 0` ni
//! `done == total`) retirées.
//!
//! Comparé pour chaque exécution : le code de sortie, la suite exacte des événements (ordre
//! des avertissements par rapport aux étapes compris, taille du `.plain.pk` de `done`), le
//! texte des avertissements (sauf ceux qui citent le message d'erreur d'un décodeur ou
//! l'absence de ffmpeg) et les lignes `  !` de stderr. En plus : stdout ne contient que des
//! lignes JSON, et aucun `.part` ne reste.
//!
//! Écarts assumés (comme `package_parity.rs`) : délégation `unsupported_format` (D4), limites
//! propres au port, exceptions non rattrapées de Python. Le Rust doit alors échouer, avec un
//! flux bien formé (`start` … `error`, `exit` égal au code de sortie).

use std::io::Cursor;
use std::path::Path;
use std::process::Command;

use serde_json::{Map, Value};
use telmi2flam::archive::Archive;

const VECTORS: &[u8] = include_bytes!("data/package_cases.zip");
const RUNS: &str = include_str!("data/progress_cases.json");

/// Avertissements dont le texte cite un message propre au moteur (erreur du décodeur
/// d'image, de `mp3map` ou du JSON, ffmpeg) ou les noms Unicode que le port n'embarque pas
/// (`font_replaced`, `text::describe_replaced`) : seul le code est comparé.
const ENGINE_TEXT: &[&str] = &[
    "image_unreadable",
    "cover_unreadable",
    "audio_samplerate",
    "mp3map_failed",
    "notes_ignored",
    "font_replaced",
];

fn normalize(mut ev: Map<String, Value>) -> Option<Value> {
    match ev.get("event").and_then(Value::as_str) {
        Some("start") => {
            ev.insert("input".into(), "<input>".into());
            ev.insert("version".into(), "<version>".into());
        }
        Some("done") => {
            ev.insert("output".into(), "<output>".into());
        }
        Some("error") => {
            ev.remove("message");
        }
        Some("step") => {
            let (done, total) = (&ev["done"], &ev["total"]);
            if done.as_u64() != Some(0) && done != total {
                return None;
            }
        }
        _ => {}
    }
    Some(Value::Object(ev))
}

/// Texte comparable d'un avertissement : vide pour un texte propre au moteur.
fn comparable(code: &str, message: &str) -> String {
    if ENGINE_TEXT.contains(&code) {
        String::new()
    } else {
        message.to_owned()
    }
}

struct Outcome {
    exit: i32,
    events: Option<Vec<Value>>,
    /// Événements bruts (non normalisés) : texte des avertissements.
    raw_warnings: Vec<(String, String)>,
    bangs: Vec<String>,
    leftovers: Vec<String>,
}

fn run_binary(work: &Path, args: &[String], machine: bool) -> Result<Outcome, String> {
    let out = Command::new(env!("CARGO_BIN_EXE_telmi2flam"))
        .args(args)
        .current_dir(work)
        .output()
        .map_err(|e| e.to_string())?;
    let stdout = String::from_utf8(out.stdout).map_err(|_| "stdout non UTF-8".to_owned())?;
    let mut events = Vec::new();
    let mut raw_warnings = Vec::new();
    // Par phase : dernier `done` vu et `total`. Les étapes retirées par la normalisation sont
    // vérifiées ici : jamais `done > total`, `done` croissant, rien après `done == total`.
    let mut phases: std::collections::HashMap<String, (u64, u64)> = Default::default();
    for line in stdout.lines() {
        // #133 : sans --progress-json, Python écrit le résumé humain sur stdout (pas encore
        // porté) ; le Rust n'y écrit rien.
        if !machine {
            return Err(format!("stdout sans --progress-json : {line:?}"));
        }
        if !line.is_ascii() {
            return Err(format!("ligne non ASCII : {line:?}"));
        }
        let ev: Map<String, Value> =
            serde_json::from_str(line).map_err(|e| format!("ligne non JSON {line:?} : {e}"))?;
        if ev["event"] == "step" {
            let phase = ev["phase"].as_str().unwrap_or_default().to_owned();
            let (done, total) = (
                ev["done"].as_u64().unwrap_or(u64::MAX),
                ev["total"].as_u64().unwrap_or(0),
            );
            let bad = done > total
                || match phases.get(&phase) {
                    None => done != 0,
                    Some(&(last, t)) => t != total || last == total || done <= last,
                };
            if bad {
                return Err(format!(
                    "étape incohérente {line} (avant : {:?})",
                    phases.get(&phase)
                ));
            }
            phases.insert(phase, (done, total));
        }
        if ev["event"] == "warning" {
            raw_warnings.push((
                ev["code"].as_str().unwrap_or_default().to_owned(),
                ev["message"].as_str().unwrap_or_default().to_owned(),
            ));
        }
        events.extend(normalize(ev));
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let bangs = stderr
        .lines()
        .filter_map(|l| l.strip_prefix("  ! "))
        .map(str::to_owned)
        .collect();
    let leftovers = std::fs::read_dir(work)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".part"))
        .collect();
    Ok(Outcome {
        exit: out.status.code().unwrap_or(-1),
        events: machine.then_some(events),
        raw_warnings,
        bangs,
        leftovers,
    })
}

/// Flux d'un échec : `start` (sauf `bad_arguments`) d'abord, `error` en dernier avec `exit`.
fn well_formed_failure(o: &Outcome) -> Result<(), String> {
    let Some(events) = &o.events else {
        return Ok(());
    };
    let last = events.last().ok_or("aucun événement")?;
    if last["event"] != "error" || last["exit"].as_i64() != Some(i64::from(o.exit)) {
        return Err(format!("dernier événement {last}, code {}", o.exit));
    }
    if last["code"] != "bad_arguments" && events[0]["event"] != "start" {
        return Err(format!("premier événement {}", events[0]));
    }
    Ok(())
}

fn check(run: &Value, cases: &[Value], zip: &mut Archive<Cursor<&[u8]>>) -> Result<(), String> {
    let id = run["id"].as_str().unwrap();
    let work = std::env::temp_dir().join(format!(
        "t2f-progress-{}-{}",
        std::process::id(),
        id.replace('+', "_")
    ));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).unwrap();
    let case = run["case"].as_str();
    if let Some(cid) = case {
        let data = zip.read(&format!("cases/{cid}.zip")).unwrap().unwrap();
        std::fs::write(work.join("in.zip"), data).unwrap();
    }
    let input = work.join("in.zip").display().to_string();
    let output = work.join("out.plain.pk").display().to_string();
    let args: Vec<String> = run["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            a.as_str()
                .unwrap()
                .replace("{in}", &input)
                .replace("{out}", &output)
        })
        .collect();
    let machine = args.iter().any(|a| a == "--progress-json");
    let got = run_binary(&work, &args, machine);
    let _ = std::fs::remove_dir_all(&work);
    let got = got?;
    if !got.leftovers.is_empty() {
        return Err(format!("temporaires restants : {:?}", got.leftovers));
    }
    // Lignes `  !` : une par avertissement, même texte que l'événement.
    if machine {
        let texts: Vec<&str> = got.raw_warnings.iter().map(|w| w.1.as_str()).collect();
        if got.bangs != texts {
            return Err(format!("stderr {:?} ≠ avertissements {texts:?}", got.bangs));
        }
    }

    let meta = case.and_then(|cid| cases.iter().find(|c| c["id"] == cid));
    let tags: Vec<&str> = meta
        .map(|c| {
            c["tags"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| t.as_str().unwrap())
                .collect()
        })
        .unwrap_or_default();
    let delegated = tags.contains(&"delegated");
    let assumed = delegated
        || tags.iter().any(|t| {
            matches!(
                *t,
                "json_limit" | "encrypted_entry" | "invalid_utf8_name" | "negative_offset"
            )
        });
    let python_crash = meta
        .and_then(|c| c.get("error"))
        .and_then(Value::as_str)
        .is_some_and(|e| e.starts_with("python_crash:"));
    if assumed || python_crash {
        if got.exit == 0 {
            return Err("écart assumé : le Rust réussit".into());
        }
        if delegated && got.exit != 7 {
            return Err(format!("délégation attendue (7), code {}", got.exit));
        }
        return well_formed_failure(&got);
    }

    let want_exit = run["exit"].as_i64().unwrap();
    if i64::from(got.exit) != want_exit {
        return Err(format!(
            "code de sortie Python {want_exit}, Rust {}",
            got.exit
        ));
    }
    let want_bangs: Vec<&str> = run["bangs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b.as_str().unwrap())
        .collect();
    if want_bangs.len() != got.bangs.len() {
        return Err(format!(
            "lignes `  !` : Python {want_bangs:?}, Rust {:?}",
            got.bangs
        ));
    }
    let Some(events) = &got.events else {
        // Sans --progress-json (cas choisis sans texte propre au moteur) : texte exact. Avec,
        // le texte est vérifié par les événements, égaux aux lignes `  !` (plus haut).
        if want_bangs != got.bangs {
            return Err(format!(
                "lignes `  !` : Python {want_bangs:?}, Rust {:?}",
                got.bangs
            ));
        }
        return Ok(());
    };
    // Texte des avertissements neutralisé là où il cite un message propre au moteur.
    let strip = |evs: &[Value]| -> Vec<Value> {
        evs.iter()
            .map(|e| {
                let mut e = e.clone();
                if e["event"] == "warning" {
                    let text = comparable(
                        e["code"].as_str().unwrap_or_default(),
                        e["message"].as_str().unwrap_or_default(),
                    );
                    e["message"] = text.into();
                }
                e
            })
            .collect()
    };
    let want = strip(run["events"].as_array().unwrap());
    let have = strip(events);
    if want != have {
        let show = |v: &[Value]| {
            v.iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n    ")
        };
        return Err(format!(
            "événements\n  Python :\n    {}\n  Rust :\n    {}",
            show(&want),
            show(&have)
        ));
    }
    Ok(())
}

#[test]
fn protocole_identique_a_python() {
    let runs: Vec<Value> = serde_json::from_str(RUNS).unwrap();
    assert!(runs.len() > 120, "{} exécutions", runs.len());
    let mut zip = Archive::new(Cursor::new(VECTORS), "package_cases.zip".into()).unwrap();
    let expected = zip.read("expected.json").unwrap().unwrap();
    let expected: Value = serde_json::from_slice(&expected).unwrap();
    let cases = expected["cases"].as_array().unwrap().clone();

    // Quatre fils : chaque exécution lance le binaire (debug) sur une histoire.
    let failures = std::sync::Mutex::new(Vec::new());
    let chunks: Vec<&[Value]> = runs.chunks(runs.len().div_ceil(4)).collect();
    std::thread::scope(|s| {
        for chunk in chunks {
            let (cases, failures) = (&cases, &failures);
            s.spawn(move || {
                let mut zip =
                    Archive::new(Cursor::new(VECTORS), "package_cases.zip".into()).unwrap();
                for run in chunk {
                    if let Err(why) = check(run, cases, &mut zip) {
                        failures
                            .lock()
                            .unwrap()
                            .push(format!("{} : {why}", run["id"].as_str().unwrap()));
                    }
                }
            });
        }
    });
    let failures = failures.into_inner().unwrap();
    assert!(
        failures.is_empty(),
        "{} écart(s) sur {} exécutions :\n{}",
        failures.len(),
        runs.len(),
        failures.join("\n")
    );
}

/// Les phases annoncées par `start` couvrent toutes les étapes, chacune de 0 à `total`, et
/// l'événement `done` compte les avertissements émis (vérifié sur les vecteurs Python, qui
/// fixent le format attendu du Rust).
#[test]
fn vecteurs_python_coherents() {
    let runs: Vec<Value> = serde_json::from_str(RUNS).unwrap();
    for run in &runs {
        let Some(events) = run["events"].as_array() else {
            continue;
        };
        let Some(done) = events.iter().find(|e| e["event"] == "done") else {
            continue;
        };
        let warnings = events.iter().filter(|e| e["event"] == "warning").count();
        assert_eq!(
            done["warnings"].as_u64(),
            Some(warnings as u64),
            "{}",
            run["id"]
        );
        for phase in ["images", "audio", "scripts", "zip"] {
            let steps: Vec<&Value> = events.iter().filter(|e| e["phase"] == phase).collect();
            assert_eq!(steps.first().unwrap()["done"], 0, "{} {phase}", run["id"]);
            let last = steps.last().unwrap();
            assert_eq!(last["done"], last["total"], "{} {phase}", run["id"]);
        }
    }
}

/// stdout fermé par le parent dès le départ : l'émission cesse sans erreur et la conversion
/// va jusqu'au bout (`_print_event`), code 0 et sortie publiée.
#[test]
fn stdout_ferme_la_conversion_continue() {
    use std::process::Stdio;
    let mut zip = Archive::new(Cursor::new(VECTORS), "package_cases.zip".into()).unwrap();
    let data = zip
        .read("cases/missing_image_allowed.zip")
        .unwrap()
        .unwrap();
    let work = std::env::temp_dir().join(format!("t2f-progress-{}-ferme", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(work.join("in.zip"), data).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_telmi2flam"))
        .args([
            "in.zip",
            "-o",
            "out.plain.pk",
            "--progress-json",
            "--thumb-fit",
            "bandes",
            "--allow-missing",
        ])
        .current_dir(&work)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let out = child.wait_with_output().unwrap();
    let produced = work.join("out.plain.pk").is_file();
    let _ = std::fs::remove_dir_all(&work);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(produced);
    // Les avertissements restent sur stderr.
    assert!(String::from_utf8_lossy(&out.stderr).contains("  ! image manquante: "));
}
