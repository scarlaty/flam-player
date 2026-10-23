//! Résumé humain du binaire (#133) : sur stdout sans `--progress-json`, sur stderr avec ; mêmes
//! lignes que `telmi2flam.py` (vérifié à la main sur les 125 archives de `package_cases.zip`).

use std::io::Cursor;
use std::process::Command;

use telmi2flam::archive::Archive;

const VECTORS: &[u8] = include_bytes!("data/package_cases.zip");

fn run(extra: &[&str]) -> (i32, String, String) {
    let dir = std::env::temp_dir().join(format!(
        "t2f-summary-{}-{}",
        std::process::id(),
        extra.len()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut zip = Archive::new(Cursor::new(VECTORS), "package_cases.zip".into()).unwrap();
    let data = zip.read("cases/ok_inventory.zip").unwrap().unwrap();
    std::fs::write(dir.join("in.zip"), data).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_telmi2flam"))
        .current_dir(&dir)
        .args([
            "in.zip",
            "-o",
            "o.plain.pk",
            "--plain",
            "--thumb-fit",
            "bandes",
        ])
        .args(extra)
        .env("FLAM_FFMPEG", "")
        .output()
        .unwrap();
    let size = std::fs::metadata(dir.join("o.plain.pk")).unwrap().len();
    let _ = std::fs::remove_dir_all(&dir);
    let text = |b: &[u8]| String::from_utf8_lossy(b).replace("\r\n", "\n");
    let (so, se) = (text(&out.stdout), text(&out.stderr));
    assert!(size > 0);
    (out.status.code().unwrap(), so, se)
}

fn expected_summary() -> String {
    let sep = std::path::MAIN_SEPARATOR;
    format!(
        "OK -> o.plain.pk\nOK -> o.plain{sep}\n  titre   : T\n  stages  : 1 | actions : 1\n  \
         images  : 6 | audio : 1 | mp3map : 1\n  lib     : 34 scripts framework | 25 assets UI\n  \
         choix   : selecteur 'carousel'\n  taille  : 0.6 Mo\n"
    )
}

#[test]
fn resume_sur_stdout_sans_progression() {
    let (code, out, err) = run(&[]);
    assert_eq!((code, err.as_str()), (0, ""));
    assert_eq!(out, expected_summary());
}

#[test]
fn resume_sur_stderr_avec_progression() {
    let (code, out, err) = run(&["--progress-json"]);
    assert_eq!(code, 0);
    assert!(out.lines().all(|l| l.starts_with("{\"event\":")), "{out}");
    assert_eq!(err, expected_summary());
}
