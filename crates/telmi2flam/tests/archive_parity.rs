//! Parité de la lecture d'archive avec `zipfile` et `telmi2flam.py` : vecteurs produits par
//! `tests/vectors/make_archive_vectors.py` (Python 3.14.8 sous Windows, version de l'oracle).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use telmi2flam::archive::{find_root, Archive};
use telmi2flam::info::read_info;

fn vectors_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/vectors/archive")
}

fn expected() -> Vec<Value> {
    let raw = fs::read_to_string(vectors_dir().join("expected.json")).unwrap();
    serde_json::from_str::<Value>(&raw)
        .unwrap()
        .as_array()
        .unwrap()
        .clone()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Code d'erreur attendu côté Rust pour une catégorie d'échec Python.
fn rust_code(category: &str) -> &'static str {
    match category {
        "corrupt" | "encrypted" | "python_fails_too" => "archive_unreadable",
        "unsupported" => "unsupported_format",
        other => panic!("catégorie inconnue {other}"),
    }
}

/// Les noms avec `\` dépendent du système (`_sanitize_filename`) : vecteurs produits sous
/// Windows.
fn applicable(case: &Value) -> bool {
    cfg!(windows) || case["id"] != "names_backslash_nul"
}

#[test]
fn ouverture_noms_lectures_et_recherche_comme_python() {
    let cases = expected();
    assert!(cases.len() >= 40);
    for case in cases.iter().filter(|c| applicable(c)) {
        let id = case["id"].as_str().unwrap();
        let path = vectors_dir().join(format!("{id}.zip"));
        let opened = Archive::open(&path);
        let open = case["open"].as_str().unwrap();
        if open != "ok" {
            let e = opened
                .err()
                .unwrap_or_else(|| panic!("{id} : ouverture acceptée"));
            assert_eq!(e.code, rust_code(open), "{id}");
            assert_eq!(e.exit_code, 3, "{id}");
            continue;
        }
        let mut archive = opened.unwrap_or_else(|e| panic!("{id} : {e}"));
        let names: Vec<String> = case["namelist"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_str().unwrap().to_string())
            .collect();
        assert_eq!(archive.names(), names.as_slice(), "{id} : namelist");
        for read in case["reads"].as_array().unwrap() {
            let name = read["name"].as_str().unwrap();
            let got = archive.read(name);
            match read.get("hex") {
                Some(h) => {
                    let content = got
                        .unwrap_or_else(|e| panic!("{id} / {name:?} : {e}"))
                        .unwrap_or_else(|| panic!("{id} / {name:?} : absent"));
                    assert_eq!(hex(&content), h.as_str().unwrap(), "{id} / {name:?}");
                }
                None => {
                    let e = got.expect_err(&format!("{id} / {name:?} : lecture acceptée"));
                    assert_eq!(
                        e.code,
                        rust_code(read["error"].as_str().unwrap()),
                        "{id} / {name:?} : {e}"
                    );
                }
            }
        }
        for probe in case["lookup"].as_array().unwrap() {
            let p = probe[0].as_str().unwrap();
            let want = probe[1].as_str();
            assert_eq!(archive.lookup(p).as_deref(), want, "{id} : lookup {p:?}");
        }
        match find_root(archive.names()) {
            Ok(root) => assert_eq!(root, case["root"].as_str().unwrap(), "{id}"),
            Err(e) => {
                let want = case["root_error"].as_array().unwrap();
                assert_eq!(e.code, want[0].as_str().unwrap(), "{id}");
                assert_eq!(u64::from(e.exit_code), want[1].as_u64().unwrap(), "{id}");
            }
        }
    }
}

/// `read_info` (bibliothèque) et `telmi2flam <archive> --info` (binaire) contre Python.
fn check_info(case: &Value, path: &Path) {
    let id = case["id"].as_str().unwrap();
    let exit = case["info_exit"].as_u64().unwrap();
    let stdout = case["info_stdout"].as_str().unwrap().trim_end();
    match read_info(path) {
        Ok(info) => {
            assert_eq!(exit, 0, "{id} : --info accepté");
            assert_eq!(info.to_json(), stdout, "{id}");
        }
        Err(e) => assert_eq!(u64::from(e.exit_code), exit, "{id} : {e}"),
    }
    let out = Command::new(env!("CARGO_BIN_EXE_telmi2flam"))
        .arg(path)
        .arg("--info")
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(exit as i32),
        "{id} : code du binaire"
    );
    if exit == 0 {
        assert_eq!(
            String::from_utf8(out.stdout).unwrap().trim_end(),
            stdout,
            "{id}"
        );
    } else {
        assert!(out.stdout.is_empty(), "{id}");
    }
}

#[test]
fn info_identique_a_python() {
    for case in expected().iter().filter(|c| applicable(c)) {
        let id = case["id"].as_str().unwrap();
        check_info(case, &vectors_dir().join(format!("{id}.zip")));
    }
}

#[test]
fn chemins_windows_non_ascii_et_longs() {
    let cases = expected();
    let case = cases.iter().find(|c| c["id"] == "story_stored").unwrap();
    let src = vectors_dir().join("story_stored.zip");
    let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("archive_chemins");
    let _ = fs::remove_dir_all(&base);

    let unicode = base.join("été 漢字 😀");
    fs::create_dir_all(&unicode).unwrap();
    let target = unicode.join("histoire ÉTÉ 🐻.zip");
    fs::copy(&src, &target).unwrap();
    check_info(case, &target);

    // Plus de 260 caractères : la bibliothèque standard ajoute le préfixe \\?\ elle-même.
    let mut long = base.clone();
    for i in 0..12 {
        long = long.join(format!("dossier-très-long-{i:02}-ééééééééé"));
    }
    fs::create_dir_all(&long).unwrap();
    let target = long.join("histoire.zip");
    fs::copy(&src, &target).unwrap();
    assert!(target.as_os_str().len() > 260);
    check_info(case, &target);

    // Archive absente, dossier : archive_unreadable (code 3), comme Python.
    for missing in [unicode.join("absente.zip"), unicode.clone()] {
        let e = read_info(&missing).unwrap_err();
        assert_eq!((e.code, e.exit_code), ("archive_unreadable", 3));
    }
    let _ = fs::remove_dir_all(&base);
}

/// Entrées hostiles : troncatures et octets altérés de chaque vecteur. Aucune panique, et
/// chaque lecture se termine (succès ou erreur de conversion).
#[test]
fn aucune_panique_sur_archive_alteree() {
    let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for case in expected() {
        let id = case["id"].as_str().unwrap();
        let data = fs::read(vectors_dir().join(format!("{id}.zip"))).unwrap();
        let mut variants: Vec<Vec<u8>> = Vec::new();
        for cut in (0..data.len()).step_by(7) {
            variants.push(data[..cut].to_vec());
        }
        for _ in 0..300 {
            let mut v = data.clone();
            if v.is_empty() {
                break;
            }
            for _ in 0..1 + next() % 4 {
                let i = (next() % v.len() as u64) as usize;
                v[i] = (next() & 0xFF) as u8;
            }
            variants.push(v);
        }
        for v in variants {
            let Ok(mut archive) = Archive::new(std::io::Cursor::new(v), id.to_string()) else {
                continue;
            };
            let names = archive.names().to_vec();
            for n in &names {
                let _ = archive.read(n);
            }
            let _ = telmi2flam::info::read_info_from(&mut archive);
        }
    }
}
