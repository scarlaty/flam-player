//! Runtime du projet embarqué : lit le manifeste unique `tools/telmi2flam/runtime/manifest.txt`
//! (aussi lu par `telmi2flam.py` et `validate.py`) et génère la table `include_bytes!` de
//! `src/runtime.rs` (PORT-RUST.md §2.1). Un manifeste mal formé arrête la compilation ; le
//! test `manifeste_identique_aux_dossiers` vérifie qu'il liste exactement les dossiers.

use std::collections::HashSet;
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

fn main() {
    let crate_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let runtime = crate_dir.join("../../tools/telmi2flam/runtime");
    let manifest = runtime.join("manifest.txt");
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!(
        "cargo:rerun-if-changed={}",
        runtime.join("script").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        runtime.join("img/script").display()
    );
    let text = fs::read_to_string(&manifest)
        .unwrap_or_else(|e| panic!("{} illisible : {e}", manifest.display()));

    let mut seen = HashSet::new();
    let mut code = String::from("&[\n");
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (required, path) = match line.strip_prefix('!') {
            Some(p) => (true, p),
            None => (false, line),
        };
        let (dir, file, ext) = if let Some(f) = path.strip_prefix("script/") {
            ("script", f, ".lua")
        } else if let Some(f) = path.strip_prefix("img/script/") {
            ("img/script", f, ".lif")
        } else {
            panic!("manifest.txt:{} : dossier inattendu : {path}", n + 1);
        };
        // Nom de sortie du paquet tel quel : caractères sûrs seulement (garde zip-slip).
        let safe = !file.is_empty()
            && !file.starts_with('.')
            && file.ends_with(ext)
            && file
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'));
        assert!(safe, "manifest.txt:{} : nom refusé : {path}", n + 1);
        assert!(
            seen.insert(path.to_ascii_lowercase()),
            "manifest.txt:{} : doublon : {path}",
            n + 1
        );
        let src = runtime.join(dir).join(file);
        assert!(src.is_file(), "{} absent", src.display());
        println!("cargo:rerun-if-changed={}", src.display());
        let src = src.to_str().expect("chemin du runtime non UTF-8");
        writeln!(
            code,
            "    RuntimeFile {{ name: {path:?}, required: {required}, data: include_bytes!({src:?}) }},"
        )
        .expect("écriture en mémoire");
    }
    code.push_str("]\n");
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("runtime_files.rs");
    fs::write(&out, code).unwrap_or_else(|e| panic!("{} : {e}", out.display()));
}
