//! L'archive d'entrée n'est jamais remplacée (#580, invariant 3 de PORT-RUST.md) : une sortie
//! (`-o`, dossier `.plain`, nom par défaut) qui désigne l'entrée est refusée avant toute
//! écriture (`output_error`, code 6), l'entrée gardant ses octets.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;

use telmi2flam::archive::Archive;

const VECTORS: &[u8] = include_bytes!("data/package_cases.zip");

fn case_bytes() -> Vec<u8> {
    let mut zip = Archive::new(Cursor::new(VECTORS), "package_cases.zip".into()).unwrap();
    zip.read("cases/ok_inventory.zip").unwrap().unwrap()
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("t2f-noover-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Lance le binaire dans `dir` ; rend `(code, stderr)`.
fn run(dir: &Path, args: &[&str]) -> (i32, String) {
    let res = Command::new(env!("CARGO_BIN_EXE_telmi2flam"))
        .current_dir(dir)
        .args(args)
        .args(["--thumb-fit", "bandes"])
        .env("FLAM_FFMPEG", "")
        .output()
        .unwrap();
    (
        res.status.code().unwrap(),
        String::from_utf8_lossy(&res.stderr).into_owned(),
    )
}

/// `-o` identique à l'entrée, écrit de plusieurs façons (relatif, avec `.`, `.plain.pk`) :
/// refus, entrée intacte, aucun `.part`.
#[test]
fn sortie_identique_a_l_entree() {
    let data = case_bytes();
    for (input, out) in [
        ("in.zip", "in.zip"),
        ("in.zip", "./in.zip"),
        ("in.zip", "sub/../in.zip"),
        ("x.plain.pk", "x.plain.pk"),
    ] {
        let dir = scratch(&format!("o-{}", out.replace(['/', '.'], "_")));
        std::fs::create_dir(dir.join("sub")).unwrap();
        std::fs::write(dir.join(input), &data).unwrap();
        let (code, err) = run(&dir, &[input, "-o", out]);
        assert_eq!(code, 6, "{input} -> {out} : {err}");
        assert!(err.contains("archive d'entree"), "{err}");
        assert_eq!(std::fs::read(dir.join(input)).unwrap(), data, "{out}");
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert!(names.iter().all(|n| !n.ends_with(".part")), "{names:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Windows : la casse ne distingue pas deux chemins du même fichier.
#[cfg(windows)]
#[test]
fn sortie_identique_a_la_casse_pres() {
    let data = case_bytes();
    let dir = scratch("casse");
    std::fs::write(dir.join("in.zip"), &data).unwrap();
    let (code, err) = run(&dir, &["in.zip", "-o", "IN.ZIP"]);
    assert_eq!(code, 6, "{err}");
    assert_eq!(std::fs::read(dir.join("in.zip")).unwrap(), data);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Entrée nommée `x.plain` : `-o x.plain` désigne le dossier `.plain`, donc l'entrée.
#[test]
fn dossier_plain_identique_a_l_entree() {
    let data = case_bytes();
    let dir = scratch("plain");
    std::fs::write(dir.join("x.plain"), &data).unwrap();
    let (code, err) = run(&dir, &["x.plain", "-o", "x.plain"]);
    assert_eq!(code, 6, "{err}");
    assert_eq!(std::fs::read(dir.join("x.plain")).unwrap(), data);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Sans `-o`, le nom par défaut (`<titre>.<UUID>.plain.pk`) peut être celui de l'entrée : cas
/// d'un `.plain.pk` déjà produit, reconverti dans son dossier.
#[test]
fn nom_par_defaut_identique_a_l_entree() {
    let data = case_bytes();
    let dir = scratch("defaut");
    std::fs::write(dir.join("in.zip"), &data).unwrap();
    let (code, err) = run(&dir, &["in.zip"]);
    assert_eq!(code, 0, "{err}");
    let produced: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.ends_with(".plain.pk"))
        .collect();
    assert_eq!(produced.len(), 1, "{produced:?}");
    let default = &produced[0];
    std::fs::write(dir.join(default), &data).unwrap();
    let (code, err) = run(&dir, &[default]);
    assert_eq!(code, 6, "{err}");
    assert_eq!(std::fs::read(dir.join(default)).unwrap(), data);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Lien symbolique (POSIX) vers l'entrée : même fichier.
#[cfg(unix)]
#[test]
fn lien_vers_l_entree() {
    let data = case_bytes();
    let dir = scratch("lien");
    std::fs::write(dir.join("in.zip"), &data).unwrap();
    std::os::unix::fs::symlink(dir.join("in.zip"), dir.join("lien.zip")).unwrap();
    let (code, err) = run(&dir, &["in.zip", "-o", "lien.zip"]);
    assert_eq!(code, 6, "{err}");
    assert_eq!(std::fs::read(dir.join("in.zip")).unwrap(), data);
    let _ = std::fs::remove_dir_all(&dir);
}

/// `-o` donné : le refus précède la construction du paquet. Une archive ZIP valide mais qui
/// n'est pas une histoire (ici vide) donnerait `not_telmi` (code 3) si le paquet était
/// construit d'abord ; le refus (code 6) prouve qu'il ne l'est pas.
#[test]
fn refus_avant_la_construction_du_paquet() {
    let dir = scratch("avant");
    // Archive ZIP vide : seulement l'enregistrement de fin.
    let mut empty = b"PK\x05\x06".to_vec();
    empty.extend_from_slice(&[0u8; 18]);
    std::fs::write(dir.join("in.zip"), &empty).unwrap();
    let (code, _) = run(&dir, &["in.zip", "-o", "other.plain.pk"]);
    assert_eq!(
        code, 3,
        "contrôle : archive vide non refusée comme histoire"
    );
    let (code, err) = run(&dir, &["in.zip", "-o", "in.zip"]);
    assert_eq!(code, 6, "{err}");
    assert_eq!(std::fs::read(dir.join("in.zip")).unwrap(), empty);
    let _ = std::fs::remove_dir_all(&dir);
}
