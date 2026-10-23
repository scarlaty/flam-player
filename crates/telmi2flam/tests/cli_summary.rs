//! Résumé humain du binaire (#133) : sur stdout sans `--progress-json`, sur stderr avec ; mêmes
//! lignes que `telmi2flam.py` (valeurs relevées sur l'oracle ; le reste vérifié à la main sur
//! les 125 archives de `package_cases.zip`).

use std::io::Cursor;
use std::process::Command;

use telmi2flam::archive::Archive;

const VECTORS: &[u8] = include_bytes!("data/package_cases.zip");

/// Convertit `cases/<case>.zip` vers `out` ; rend `(code, stdout, stderr)` (fins de ligne
/// normalisées).
fn run(case: &str, out: &str, extra: &[&str]) -> (i32, String, String) {
    let tag = format!("{case}-{}-{}", out.replace('.', "_"), extra.len());
    let dir = std::env::temp_dir().join(format!("t2f-summary-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut zip = Archive::new(Cursor::new(VECTORS), "package_cases.zip".into()).unwrap();
    let data = zip.read(&format!("cases/{case}.zip")).unwrap().unwrap();
    std::fs::write(dir.join("in.zip"), data).unwrap();
    let res = Command::new(env!("CARGO_BIN_EXE_telmi2flam"))
        .current_dir(&dir)
        .args(["in.zip", "-o", out, "--thumb-fit", "bandes"])
        .args(extra)
        .env("FLAM_FFMPEG", "")
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let text = |b: &[u8]| String::from_utf8_lossy(b).replace("\r\n", "\n");
    (
        res.status.code().unwrap(),
        text(&res.stdout),
        text(&res.stderr),
    )
}

fn sep() -> char {
    std::path::MAIN_SEPARATOR
}

fn full_summary() -> String {
    let sep = sep();
    format!(
        "OK -> o.plain.pk\nOK -> o.plain{sep}\n  titre   : T\n  stages  : 1 | actions : 1\n  \
         images  : 6 | audio : 1 | mp3map : 1\n  lib     : 34 scripts framework | 25 assets UI\n  \
         choix   : selecteur 'carousel'\n  taille  : 0.6 Mo\n"
    )
}

#[test]
fn resume_sur_stdout_sans_progression() {
    let (code, out, err) = run("ok_inventory", "o.plain.pk", &["--plain"]);
    assert_eq!((code, err.as_str()), (0, ""));
    assert_eq!(out, full_summary());
}

#[test]
fn resume_sur_stderr_avec_progression() {
    let (code, out, err) = run(
        "ok_inventory",
        "o.plain.pk",
        &["--plain", "--progress-json"],
    );
    assert_eq!(code, 0);
    assert!(out.lines().all(|l| l.starts_with("{\"event\":")), "{out}");
    assert_eq!(err, full_summary());
}

/// `-o x.plain` : seul le dossier, terminé par le séparateur, et pas de ligne `taille`.
#[test]
fn dossier_seul() {
    let (code, out, err) = run("ok_inventory", "o.plain", &[]);
    assert_eq!((code, err.as_str()), (0, ""));
    let sep = sep();
    assert_eq!(
        out,
        format!(
            "OK -> o.plain{sep}\n  titre   : T\n  stages  : 1 | actions : 1\n  \
             images  : 6 | audio : 1 | mp3map : 1\n  lib     : 34 scripts framework | 25 assets UI\n  \
             choix   : selecteur 'carousel'\n"
        )
    );
}

/// `--allow-missing` : ligne `alertes`, compteurs sans les assets absents.
#[test]
fn assets_manquants_autorises() {
    let (code, out, err) = run("missing_image_allowed", "o.plain.pk", &["--allow-missing"]);
    assert_eq!(code, 0);
    assert_eq!(
        out,
        "OK -> o.plain.pk\n  titre   : T\n  stages  : 1 | actions : 1\n  \
         images  : 3 | audio : 0 | mp3map : 0\n  lib     : 34 scripts framework | 25 assets UI\n  \
         choix   : selecteur 'carousel'\n  alertes : 2 manquant(s) | 0 illisible(s)\n  \
         taille  : 0.6 Mo\n"
    );
    assert_eq!(
        err,
        "  ! image manquante: absent.png\n  ! audio manquant: absent.mp3 -> silent.mp3\n  \
         ! 2 asset(s) manquant(s) : image absent.png, audio absent.mp3 (--allow-missing : \
         images omises, audios remplaces par silent.mp3)\n"
    );
}
