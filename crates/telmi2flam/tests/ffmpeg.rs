//! Rééchantillonnage par ffmpeg externe (#354) avec un **faux ffmpeg déterministe** : ce
//! même exécutable de test (`harness = false`), relancé avec les arguments de ffmpeg, joue
//! ffmpeg. Il vérifie que ses arguments sont exactement ceux de [`ffmpeg_args`] et que son
//! dossier courant est celui de l'entrée, puis se comporte selon `T2F_FAKE_FFMPEG_MODE` :
//!
//! - `ok` (défaut) : écrit `3 + len % 5` trames MPEG-1 Layer III à 44 100 Hz au débit demandé
//!   (premier octet de données : somme des octets de l'entrée modulo 256) ;
//! - `rate48` : idem à 48 000 Hz ; `garbage` : octets non MPEG ; `empty` : fichier vide ;
//!   `none` : aucun fichier ; `fail` : code 1 et deux lignes sur la sortie d'erreur (UTF-8
//!   invalide comprise) ; `fail_silent` : code 3 sans message ; `flood` : 1 Mio sur la sortie
//!   d'erreur puis code 1 ; `sleep` : attend `T2F_FAKE_FFMPEG_SLEEP_MS` ms, crée le fichier
//!   `T2F_FAKE_FFMPEG_MARK`, puis `ok`.
//!
//! `T2F_FAKE_FFMPEG_LOG` : une ligne ajoutée par lancement (nombre d'appels).
//!
//! Le générateur Python `tests/data/gen_ffmpeg_cases.py` utilise le même exécutable : les
//! vecteurs `tests/data/ffmpeg_cases.zip` comparent les paquets de Python et de Rust avec
//! ffmpeg (sortie opaque, identique des deux côtés) et sans.
//!
//! Les tests modifient l'environnement et le dossier courant : ils s'exécutent en série dans
//! `main`, sans autre fil.

use std::ffi::OsString;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::Value;
use sha2::{Digest, Sha256};
use telmi2flam::archive::Archive;
use telmi2flam::error::ConvError;
use telmi2flam::ffmpeg::{
    ffmpeg_args, find_ffmpeg, resample_mp3, run_ffmpeg, MP3_BITRATES, NOT_FOUND, SAMPLERATE,
};
use telmi2flam::image::ThumbFit;
use telmi2flam::output::{write_package, write_package_from, Target};
use telmi2flam::package::{
    build_package, build_package_streamed, Data, Options, Package, Selector,
};

const MODE: &str = "T2F_FAKE_FFMPEG_MODE";
const LOG: &str = "T2F_FAKE_FFMPEG_LOG";
const MARK: &str = "T2F_FAKE_FFMPEG_MARK";
const SLEEP: &str = "T2F_FAKE_FFMPEG_SLEEP_MS";

const FFMPEG_VECTORS: &[u8] = include_bytes!("data/ffmpeg_cases.zip");
const PACKAGE_VECTORS: &[u8] = include_bytes!("data/package_cases.zip");

// ---------------------------------------------------------------------------------------
// Faux ffmpeg

/// Trames MPEG-1 Layer III stéréo à `rate` Hz, `kbps` kbit/s, sans remplissage.
fn fake_output(input: &[u8], kbps: u32, rate: u32) -> Vec<u8> {
    let table = [
        32u32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
    ];
    let idx = table.iter().position(|&b| b == kbps).unwrap() as u8 + 1;
    let sr = match rate {
        44_100 => 0u8,
        48_000 => 1,
        _ => 2,
    };
    let len = (144_000 * kbps / rate) as usize;
    let mut frame = vec![0u8; len];
    frame[..4].copy_from_slice(&[0xFF, 0xFB, (idx << 4) | (sr << 2), 0]);
    frame[4] = (input.iter().map(|&b| u32::from(b)).sum::<u32>() % 256) as u8;
    frame.repeat(3 + input.len() % 5)
}

fn strip_file(arg: &OsString) -> Option<PathBuf> {
    arg.to_str()?.strip_prefix("file:").map(PathBuf::from)
}

/// Faux ffmpeg ; rend le code de sortie.
fn fake(args: &[OsString]) -> i32 {
    let mut err = std::io::stderr();
    if args.len() != 29 {
        let _ = writeln!(err, "faux ffmpeg : {} arguments", args.len());
        return 9;
    }
    let (Some(src), Some(dst)) = (strip_file(&args[8]), strip_file(&args[28])) else {
        let _ = writeln!(err, "faux ffmpeg : entree ou sortie sans file:");
        return 9;
    };
    let kbps = args[21]
        .to_str()
        .and_then(|s| s.strip_suffix('k'))
        .and_then(|s| s.parse::<u32>().ok())
        .filter(|k| MP3_BITRATES.contains(k));
    let Some(kbps) = kbps else {
        let _ = writeln!(err, "faux ffmpeg : debit {:?}", args[21]);
        return 9;
    };
    if args != ffmpeg_args(&src, &dst, kbps).as_slice() {
        let _ = writeln!(err, "faux ffmpeg : arguments inattendus {args:?}");
        return 9;
    }
    let cwd = std::env::current_dir().and_then(|d| d.canonicalize()).ok();
    let parent = src.parent().and_then(|p| p.canonicalize().ok());
    if cwd.is_none() || cwd != parent {
        let _ = writeln!(err, "faux ffmpeg : dossier courant {cwd:?} != {parent:?}");
        return 8;
    }
    if let Some(log) = std::env::var_os(LOG) {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .unwrap();
        writeln!(f, "{kbps}").unwrap();
    }
    let input = std::fs::read(&src).unwrap();
    let mode = std::env::var(MODE).unwrap_or_else(|_| "ok".to_owned());
    let out = match mode.as_str() {
        "ok" => fake_output(&input, kbps, SAMPLERATE),
        "sleep" => {
            let ms = std::env::var(SLEEP).map_or(3000, |s| s.parse().unwrap());
            std::thread::sleep(Duration::from_millis(ms));
            if let Some(mark) = std::env::var_os(MARK) {
                std::fs::write(mark, b"survivant").unwrap();
            }
            fake_output(&input, kbps, SAMPLERATE)
        }
        "rate48" => fake_output(&input, kbps, 48_000),
        "garbage" => b"ceci n'est pas un mp3 ".repeat(20),
        "empty" => Vec::new(),
        "none" => return 0,
        "fail" => {
            let _ =
                err.write_all(b"premiere ligne\r\n  erreur du faux ffmpeg : \xc3\xa9\xff  \n\n");
            return 1;
        }
        "fail_silent" => return 3,
        "flood" => {
            let _ = err.write_all(&vec![b'x'; 1 << 20]);
            let _ = err.write_all(b"\nfin\n");
            return 1;
        }
        other => {
            let _ = writeln!(err, "faux ffmpeg : mode {other}");
            return 7;
        }
    };
    std::fs::write(&dst, out).unwrap();
    0
}

// ---------------------------------------------------------------------------------------
// Outils

fn exe() -> PathBuf {
    std::env::current_exe().unwrap()
}

/// Trames MPEG-1 Layer III à zéro (`mp3_frames` des tests Python).
fn frames(b2: u8, n: usize) -> Vec<u8> {
    let rates = [44_100u32, 48_000, 32_000];
    let kbps = [
        0u32, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
    ];
    let len = (144_000 * kbps[usize::from(b2 >> 4)] / rates[usize::from((b2 >> 2) & 3)]) as usize
        + usize::from((b2 >> 1) & 1);
    let mut f = vec![0u8; len];
    f[..4].copy_from_slice(&[0xFF, 0xFB, b2, 0]);
    f.repeat(n)
}

/// Dossier de test vide et propre à `name`.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("t2f-fftest-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn is_empty_dir(dir: &Path) -> bool {
    std::fs::read_dir(dir).unwrap().next().is_none()
}

fn never() -> bool {
    false
}

fn run_mode(
    mode: &str,
    root: &Path,
    timeout: Duration,
) -> Result<Result<Vec<u8>, String>, ConvError> {
    std::env::set_var(MODE, mode);
    let r = run_ffmpeg(&exe(), &frames(0x94, 50), 96, root, timeout, &never);
    std::env::remove_var(MODE);
    r
}

// ---------------------------------------------------------------------------------------
// Tests du lancement

fn sortie_et_echecs() {
    let root = scratch("modes");
    let input = frames(0x94, 50);
    let ok = run_mode("ok", &root, Duration::from_secs(30))
        .unwrap()
        .unwrap();
    assert_eq!(ok, fake_output(&input, 96, SAMPLERATE));
    assert!(is_empty_dir(&root), "dossier temporaire laissé");
    for (mode, want) in [
        // Dernière ligne non retaillée, comme `splitlines()[-1]` après `strip()` du tout.
        (
            "fail",
            "ffmpeg a echoue (  erreur du faux ffmpeg : \u{e9}\u{fffd})",
        ),
        ("fail_silent", "ffmpeg a echoue (3)"),
        ("flood", "ffmpeg a echoue (fin)"),
        ("empty", "ffmpeg n'a rien produit"),
    ] {
        let got = run_mode(mode, &root, Duration::from_secs(30)).unwrap();
        assert_eq!(got, Err(want.to_owned()), "{mode}");
        assert!(is_empty_dir(&root), "{mode} : dossier temporaire laissé");
    }
    let none = run_mode("none", &root, Duration::from_secs(30)).unwrap();
    assert!(none.unwrap_err().starts_with("ffmpeg inutilisable ("));
    // Sortie opaque rendue telle quelle : la validation est faite par assets.rs.
    let garbage = run_mode("garbage", &root, Duration::from_secs(30)).unwrap();
    assert_eq!(garbage.unwrap(), b"ceci n'est pas un mp3 ".repeat(20));
    assert!(is_empty_dir(&root));
    let _ = std::fs::remove_dir_all(&root);
}

/// Délai dépassé ou annulation : ffmpeg tué (le marqueur n'apparaît jamais), dossier
/// temporaire supprimé, retour rapide.
fn delai_et_annulation() {
    let root = scratch("kill");
    let mark = root.with_extension("mark");
    let _ = std::fs::remove_file(&mark);
    std::env::set_var(MARK, &mark);
    std::env::set_var(SLEEP, "6000");
    std::env::set_var(MODE, "sleep");
    let input = frames(0x94, 20);

    let t = Instant::now();
    let r = run_ffmpeg(
        &exe(),
        &input,
        64,
        &root,
        Duration::from_millis(300),
        &never,
    )
    .unwrap();
    assert_eq!(r, Err("ffmpeg trop long (> 0 s)".to_owned()));
    assert!(
        t.elapsed() < Duration::from_millis(5000),
        "{:?}",
        t.elapsed()
    );
    assert!(is_empty_dir(&root), "délai : dossier temporaire laissé");

    let t = Instant::now();
    let cancel = || t.elapsed() > Duration::from_millis(200);
    let e = run_ffmpeg(&exe(), &input, 64, &root, Duration::from_secs(60), &cancel).unwrap_err();
    assert_eq!(e.code, "cancelled");
    assert_eq!(e.exit_code, 130);
    assert!(
        t.elapsed() < Duration::from_millis(5000),
        "{:?}",
        t.elapsed()
    );
    assert!(
        is_empty_dir(&root),
        "annulation : dossier temporaire laissé"
    );

    // Marge au-delà du sommeil (6 s) du faux ffmpeg : il aurait créé le marqueur.
    std::thread::sleep(Duration::from_millis(6600));
    assert!(!mark.exists(), "ffmpeg a survécu");

    // Sans délai dépassé, le même faux ffmpeg finit et crée le marqueur.
    std::env::set_var(SLEEP, "10");
    let r = run_ffmpeg(&exe(), &input, 64, &root, Duration::from_secs(30), &never).unwrap();
    assert_eq!(r.unwrap(), fake_output(&input, 64, SAMPLERATE));
    assert!(mark.exists());
    for k in [MARK, SLEEP, MODE] {
        std::env::remove_var(k);
    }
    let _ = std::fs::remove_file(&mark);
    let _ = std::fs::remove_dir_all(&root);
}

/// `FLAM_FFMPEG` : vide, absent, dossier, relatif ; puis recherche dans le `PATH`.
fn recherche_de_ffmpeg() {
    let old_cwd = std::env::current_dir().unwrap();
    let old_path = std::env::var_os("PATH");
    let exe = exe();
    let abs = std::path::absolute(&exe).unwrap();

    std::env::set_var("FLAM_FFMPEG", "");
    assert_eq!(find_ffmpeg(), None);
    assert_eq!(
        resample_mp3(&frames(0x94, 10), 1.0, &never).unwrap(),
        Err(NOT_FOUND.to_owned())
    );
    std::env::set_var("FLAM_FFMPEG", exe.with_file_name("absent-ffmpeg"));
    assert_eq!(find_ffmpeg(), None);
    std::env::set_var("FLAM_FFMPEG", exe.parent().unwrap());
    assert_eq!(find_ffmpeg(), None, "dossier");
    // Chemin relatif au dossier courant, rendu absolu.
    std::env::set_current_dir(exe.parent().unwrap()).unwrap();
    std::env::set_var("FLAM_FFMPEG", exe.file_name().unwrap());
    assert_eq!(find_ffmpeg(), Some(abs.clone()));
    // Lancé par son chemin absolu, alors que ffmpeg tourne dans son dossier temporaire.
    let out = resample_mp3(&frames(0x94, 50), 1.0, &never).unwrap();
    assert!(out.is_ok(), "{out:?}");
    // Relu à chaque appel, comme Python : relatif à un autre dossier courant, introuvable.
    std::env::set_current_dir(&old_cwd).unwrap();
    assert_eq!(
        resample_mp3(&frames(0x94, 10), 1.0, &never).unwrap(),
        Err(NOT_FOUND.to_owned())
    );

    // PATH : copie nommée ffmpeg ; refusée si elle est dans le dossier courant.
    std::env::remove_var("FLAM_FFMPEG");
    let bin = scratch("path");
    let name = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    std::fs::copy(&exe, bin.join(name)).unwrap();
    let mut path = OsString::from(bin.as_os_str());
    path.push(if cfg!(windows) { ";" } else { ":" });
    path.push(bin.as_os_str());
    std::env::set_var("PATH", &path);
    let found = find_ffmpeg().expect("ffmpeg du PATH");
    // Sous Windows, nom construit avec l'extension de PATHEXT (`ffmpeg.EXE`), comme Python.
    let lower = |p: &Path| p.to_string_lossy().to_lowercase();
    assert_eq!(
        lower(&found),
        lower(&std::path::absolute(bin.join(name)).unwrap())
    );
    std::env::set_current_dir(&bin).unwrap();
    assert_eq!(find_ffmpeg(), None, "ffmpeg du dossier courant");
    std::env::set_current_dir(&old_cwd).unwrap();
    // Entrée vide du PATH = dossier courant (`shutil.which` la lit ainsi) : refusée aussi, y
    // compris quand le dossier courant n'est plus ajouté d'office (#369).
    let empty = scratch("path-vide");
    let mut with_empty = OsString::from(if cfg!(windows) { ";" } else { ":" });
    with_empty.push(empty.as_os_str());
    let old_nodefault = std::env::var_os("NoDefaultCurrentDirectoryInExePath");
    std::env::set_var("NoDefaultCurrentDirectoryInExePath", "1");
    std::env::set_var("PATH", &with_empty);
    std::env::set_current_dir(&bin).unwrap();
    assert_eq!(
        find_ffmpeg(),
        None,
        "entrée vide du PATH, ffmpeg du dossier courant"
    );
    std::env::set_current_dir(&old_cwd).unwrap();
    match old_nodefault {
        Some(v) => std::env::set_var("NoDefaultCurrentDirectoryInExePath", v),
        None => std::env::remove_var("NoDefaultCurrentDirectoryInExePath"),
    }
    let _ = std::fs::remove_dir_all(&empty);
    std::env::set_var("PATH", "");
    assert_eq!(find_ffmpeg(), None);

    match old_path {
        Some(p) => std::env::set_var("PATH", p),
        None => std::env::remove_var("PATH"),
    }
    std::env::set_var("FLAM_FFMPEG", "");
    let _ = std::fs::remove_dir_all(&bin);
}

// ---------------------------------------------------------------------------------------
// Parité des paquets avec Python

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn options(run: &Value) -> Options {
    Options {
        keep_size: run["keep_size"].as_bool().unwrap(),
        allow_missing: run["allow_missing"].as_bool().unwrap(),
        selector: match run["selector"].as_str().unwrap() {
            "carousel" => Selector::Carousel,
            "image" => Selector::Image,
            other => panic!("sélecteur {other}"),
        },
        thumb_fit: ThumbFit::Bandes,
    }
}

fn convert(data: Vec<u8>, options: Options) -> Result<Package, ConvError> {
    std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(move || {
            let mut archive = Archive::new(Cursor::new(&data[..]), "in.zip".into())?;
            build_package(&mut archive, &options, &|| false)
        })
        .unwrap()
        .join()
        .unwrap()
}

/// Écarts assumés du port (voir `package_parity.rs`) : comparés là-bas, ignorés ici.
fn assumed_divergence(run: &Value) -> bool {
    run["tags"].as_array().unwrap().iter().any(|t| {
        matches!(
            t.as_str().unwrap(),
            "delegated"
                | "json_limit"
                | "encrypted_entry"
                | "invalid_utf8_name"
                | "negative_offset"
        )
    })
}

fn check_run(run: &Value, data: Vec<u8>, dir: &Path) -> Result<(), String> {
    let got = convert(data, options(run));
    if assumed_divergence(run) {
        return Ok(());
    }
    let pkg = match (run.get("error").and_then(Value::as_str), got) {
        (Some(py), Err(_)) if py.starts_with("python_crash:") => return Ok(()),
        (Some(py), Err(e)) if e.code == py => return Ok(()),
        (Some(py), Err(e)) => return Err(format!("erreur Python {py}, Rust {}", e.code)),
        (Some(py), Ok(_)) => return Err(format!("erreur Python {py}, le Rust réussit")),
        (None, Err(e)) => return Err(format!("Python réussit, Rust {e}")),
        (None, Ok(pkg)) => pkg,
    };
    let want: Vec<(String, String)> = run["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (e[0].as_str().unwrap().into(), e[1].as_str().unwrap().into()))
        .collect();
    let have: Vec<(String, String)> = pkg
        .entries
        .iter()
        .map(|(n, d)| (n.clone(), hex(&Sha256::digest(d))))
        .collect();
    if want != have {
        let diff: Vec<String> = want
            .iter()
            .filter(|w| !have.contains(w))
            .map(|w| w.0.clone())
            .collect();
        return Err(format!("entrées différentes : {diff:?}"));
    }
    let want_w: Vec<(&str, &str)> = run["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| (w[0].as_str().unwrap(), w[1].as_str().unwrap()))
        .collect();
    if want_w.len() != pkg.warnings.len() {
        return Err(format!(
            "avertissements : Python {want_w:?}, Rust {:?}",
            pkg.warnings
        ));
    }
    for ((code, msg), w) in want_w.iter().zip(&pkg.warnings) {
        if *code != w.code {
            return Err(format!("avertissement {code} / {}", w.code));
        }
        // Raisons de ffmpeg identiques au texte près ; pour une erreur du système (texte
        // propre à chaque langage), même début et même fin. Les messages des autres codes
        // ne viennent pas de ffmpeg : `package_parity.rs` en compare les codes.
        if *code == "audio_samplerate" {
            const OS: &str = "ffmpeg inutilisable (";
            const END: &str = ") ; fichier recopie tel quel, reechantillonner conseille)";
            let same = match msg.find(OS) {
                Some(i) => {
                    w.message.starts_with(&msg[..i + OS.len()])
                        && w.message.ends_with(END)
                        && msg.ends_with(END)
                }
                None => *msg == w.message,
            };
            if !same {
                return Err(format!("message Python {msg:?}\nRust {:?}", w.message));
            }
        }
    }
    let out = dir.join("out.plain.pk");
    let target = Target::new(Some(&out), false, &pkg);
    write_package(&pkg, &target, &|| false).map_err(|e| format!("écriture : {e}"))?;
    let bytes = std::fs::read(&out).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&out);
    let have_pk = hex(&Sha256::digest(&bytes));
    if run["pk_sha256"].as_str().unwrap() != have_pk {
        return Err(".plain.pk différent".to_owned());
    }
    Ok(())
}

/// Passe en flux (#402) : assemblage par `build_package_streamed`, écriture par
/// `write_package_from` (MP3 recopiés relus dans l'archive), même `.plain.pk` que Python à
/// l'octet. Rend le nombre d'entrées relues en flux.
fn check_streamed(run: &Value, data: &[u8], dir: &Path) -> Result<usize, String> {
    let options = options(run);
    let out = dir.join("flux.plain.pk");
    let data = data.to_vec();
    let target_out = out.clone();
    let got = std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(move || -> Result<usize, ConvError> {
            let mut archive = Archive::new(Cursor::new(data), "in.zip".into())?;
            let pkg = build_package_streamed(&mut archive, &options, &|| false, &mut ())?;
            let refs = pkg
                .entries
                .iter()
                .filter(|(_, d)| matches!(d, Data::Source { .. }))
                .count();
            let target = Target::new(Some(&target_out), false, &pkg);
            write_package_from(&pkg, archive, &target, &|| false, &mut ())?;
            Ok(refs)
        })
        .unwrap()
        .join()
        .unwrap();
    if assumed_divergence(run) {
        let _ = std::fs::remove_file(&out);
        return Ok(0);
    }
    let refs = match (run.get("error").and_then(Value::as_str), got) {
        (Some(py), Err(_)) if py.starts_with("python_crash:") => return Ok(0),
        (Some(py), Err(e)) if e.code == py => return Ok(0),
        (Some(py), Err(e)) => return Err(format!("erreur Python {py}, Rust {}", e.code)),
        (Some(py), Ok(_)) => return Err(format!("erreur Python {py}, le Rust réussit")),
        (None, Err(e)) => return Err(format!("Python réussit, Rust {e}")),
        (None, Ok(refs)) => refs,
    };
    let bytes = std::fs::read(&out).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&out);
    if run["pk_sha256"].as_str().unwrap() != hex(&Sha256::digest(&bytes)) {
        return Err(".plain.pk en flux différent".to_owned());
    }
    Ok(refs)
}

fn paquets_identiques_a_python() {
    let mut ff = Archive::new(Cursor::new(FFMPEG_VECTORS), "ffmpeg_cases.zip".into()).unwrap();
    let mut pkg = Archive::new(Cursor::new(PACKAGE_VECTORS), "package_cases.zip".into()).unwrap();
    let raw = ff.read("expected.json").unwrap().unwrap();
    let expected: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(expected["python"], "3.14.8");
    let runs = expected["runs"].as_array().unwrap();
    assert!(runs.len() > 150, "{} exécutions", runs.len());
    let dir = scratch("parity");
    let log = dir.join("calls.log");
    let absent = dir.join("absent").join("ffmpeg.exe");
    let mut failures = Vec::new();
    let mut resampled_runs = 0;
    let mut mixed_runs = 0;
    for run in runs {
        let id = run["id"].as_str().unwrap();
        let mode = run["mode"].as_str().unwrap();
        let data = match run["source"].as_str().unwrap() {
            "package" => pkg.read(&format!("cases/{id}.zip")).unwrap().unwrap(),
            _ => ff.read(&format!("cases/{id}.zip")).unwrap().unwrap(),
        };
        assert_eq!(
            hex(&Sha256::digest(&data)),
            run["archive_sha256"].as_str().unwrap(),
            "{id} : archive différente de celle du générateur"
        );
        match run["ffmpeg"].as_str().unwrap() {
            "fake" => std::env::set_var("FLAM_FFMPEG", exe()),
            _ => std::env::set_var("FLAM_FFMPEG", &absent),
        }
        std::env::set_var(MODE, mode);
        // Deux passes (#402) : paquet en mémoire, puis en flux (MP3 recopiés relus dans
        // l'archive à l'écriture, à côté des MP3 rééchantillonnés par ffmpeg).
        for streamed in [false, true] {
            let pass = if streamed { "flux" } else { "memoire" };
            let _ = std::fs::remove_file(&log);
            std::env::set_var(LOG, &log);
            let r = if streamed {
                check_streamed(run, &data, &dir)
            } else {
                check_run(run, data.clone(), &dir).map(|()| 0)
            };
            match r {
                Ok(refs) if streamed && refs > 0 && run["resampled"].as_u64().unwrap_or(0) > 0 => {
                    mixed_runs += 1
                }
                Ok(_) => {}
                Err(why) => failures.push(format!("{id} [{mode}, {pass}] : {why}")),
            }
            let calls = std::fs::read_to_string(&log).map_or(0, |s| s.lines().count());
            if let Some(want) = run.get("ffmpeg_calls").and_then(Value::as_u64) {
                if calls as u64 != want {
                    failures.push(format!(
                        "{id} [{mode}, {pass}] : {calls} appel(s) de ffmpeg, Python {want}"
                    ));
                }
            }
        }
        if run["resampled"].as_u64().unwrap_or(0) > 0 {
            resampled_runs += 1;
        }
    }
    for k in [MODE, LOG] {
        std::env::remove_var(k);
    }
    std::env::set_var("FLAM_FFMPEG", "");
    let _ = std::fs::remove_dir_all(&dir);
    // Garde sur les vecteurs (Python a bien rééchantillonné) ; côté Rust, le
    // rééchantillonnage est prouvé par les SHA-256 des entrées et `ffmpeg_calls` (le compte
    // et les lignes `  i …` ne sont pas encore exposés par `Package`, #131).
    assert!(
        resampled_runs >= 5,
        "{resampled_runs} exécutions avec rééchantillonnage"
    );
    // Au moins une archive mêle MP3 rééchantillonnés et MP3 recopiés relus en flux.
    assert!(mixed_runs >= 1, "aucune exécution mixte en flux");
    assert!(
        failures.is_empty(),
        "{} écart(s) sur {} exécutions :\n{}",
        failures.len(),
        runs.len(),
        failures.join("\n")
    );
}

/// Binaire (#132) : annulation (fin de stdin) puis panique pendant que le faux ffmpeg tourne.
/// Codes 130 et 6, aucun `t2f_ff_*` ni `.part` laissé, et ffmpeg tué : son marqueur, écrit à la
/// fin de son sommeil, n'apparaît jamais.
#[cfg(any(debug_assertions, feature = "test-hooks"))]
fn binaire_annulation_et_panique_pendant_ffmpeg() {
    use std::process::{Command, Stdio};
    let mut pkg = Archive::new(Cursor::new(PACKAGE_VECTORS), "package_cases.zip".into()).unwrap();
    let story = pkg.read("cases/mp3_48khz_warning.zip").unwrap().unwrap();
    for (name, want) in [("annulation", 130), ("panique", 6)] {
        let dir = scratch(&format!("bin-{name}"));
        let tmp = dir.join("tmp");
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(dir.join("in.zip"), &story).unwrap();
        let (mark, log) = (dir.join("mark"), dir.join("calls.log"));
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_telmi2flam"));
        cmd.args(["in.zip", "-o", "out.plain.pk", "--progress-json"])
            .current_dir(&dir)
            .env("FLAM_FFMPEG", exe())
            .env(MODE, "sleep")
            .env(SLEEP, "4000")
            .env(MARK, &mark)
            .env(LOG, &log)
            .env("TMP", &tmp)
            .env("TEMP", &tmp)
            .env("TMPDIR", &tmp)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if want == 130 {
            cmd.arg("--cancel-on-stdin-eof")
                .stdin(Stdio::piped())
                .env_remove("TELMI2FLAM_TEST_FAULT");
        } else {
            cmd.stdin(Stdio::null())
                .env("TELMI2FLAM_TEST_FAULT", "panic:ffmpeg");
        }
        let mut child = cmd.spawn().unwrap();
        // Le faux ffmpeg tourne dès qu'il a écrit dans le journal.
        let t = Instant::now();
        while !log.exists() && t.elapsed() < Duration::from_secs(30) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(log.exists(), "{name} : ffmpeg jamais lancé");
        let t = Instant::now();
        drop(child.stdin.take());
        let out = child.wait_with_output().unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(want), "{name} : {stderr}");
        assert!(
            t.elapsed() < Duration::from_millis(3000),
            "{name} : arrêt trop lent ({:?}), ffmpeg attendu",
            t.elapsed()
        );
        let last = String::from_utf8_lossy(&out.stdout)
            .lines()
            .last()
            .unwrap_or_default()
            .to_owned();
        let code = if want == 130 {
            "cancelled"
        } else {
            "internal_error"
        };
        assert!(
            last.contains(&format!("\"code\":\"{code}\"")),
            "{name} : {last}"
        );
        assert!(is_empty_dir(&tmp), "{name} : t2f_ff_* laissé");
        let parts: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".part"))
            .collect();
        assert!(parts.is_empty(), "{name} : {parts:?}");
        assert!(
            !dir.join("out.plain.pk").exists(),
            "{name} : sortie publiée"
        );
        // Marge au-delà du sommeil (4 s) du faux ffmpeg : il aurait créé le marqueur.
        std::thread::sleep(Duration::from_millis(5500));
        assert!(!mark.exists(), "{name} : ffmpeg a survécu");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

// ---------------------------------------------------------------------------------------

fn main() {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|a| a == "-nostdin") {
        std::process::exit(fake(&args));
    }
    if args.iter().any(|a| a == "--list") {
        return;
    }
    // Jamais le ffmpeg de la machine : chaque test choisit le sien.
    std::env::set_var("FLAM_FFMPEG", "");
    #[allow(unused_mut)]
    let mut tests: Vec<(&str, fn())> = vec![
        ("sortie_et_echecs", sortie_et_echecs),
        ("delai_et_annulation", delai_et_annulation),
        ("recherche_de_ffmpeg", recherche_de_ffmpeg),
        ("paquets_identiques_a_python", paquets_identiques_a_python),
    ];
    #[cfg(any(debug_assertions, feature = "test-hooks"))]
    tests.push((
        "binaire_annulation_et_panique_pendant_ffmpeg",
        binaire_annulation_et_panique_pendant_ffmpeg,
    ));
    // Filtres positionnels de libtest ; les options à valeur (`--skip X`…) sont lues avec
    // leur valeur, et `--skip` appliqué.
    let mut filters = Vec::new();
    let mut skips = Vec::new();
    let mut it = args.iter().filter_map(|a| a.to_str());
    while let Some(a) = it.next() {
        match a {
            "--skip" => skips.extend(it.next().map(str::to_owned)),
            "--test-threads" | "--format" | "--color" | "--logfile" | "-Z" => {
                it.next();
            }
            _ if a.starts_with('-') => {}
            _ => filters.push(a.to_owned()),
        }
    }
    let mut ran = 0;
    for (name, test) in tests {
        if !filters.is_empty() && !filters.iter().any(|f| name.contains(f.as_str())) {
            continue;
        }
        if skips.iter().any(|s| name.contains(s.as_str())) {
            continue;
        }
        println!("test {name} ...");
        test();
        println!("test {name} ... ok");
        ran += 1;
    }
    println!("\ntest result: ok. {ran} passed");
}
