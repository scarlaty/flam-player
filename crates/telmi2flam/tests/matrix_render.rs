//! Matrice de rééchantillonnage (PORT-RUST.md §4.3, point 2) comparée aux références figées.
//!
//! Test ignoré par défaut, lancé en release par la CI de parité (#134) : il lit `matrix.json`
//! (produit par `tools/telmi2flam/tests/parity/make_matrix.py`, sans dépendance) dans
//! `TELMI2FLAM_MATRIX_DIR`, vérifie que chaque PNG a l'empreinte figée (générateur inchangé),
//! rend chaque `.lif` et le compare, octet à octet par son SHA-256, à celui de l'ancien oracle
//! Python (Pillow 12.3.0) figé dans `tests/data/matrix_expected.json` (§4.5). Avec
//! `TELMI2FLAM_MATRIX_OUT`, les `.lif` du Rust et la liste des écarts (`ecarts.json`) y sont
//! écrits pour diagnostic.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use telmi2flam::image::{
    png_to_fit_lif, png_to_lif, png_to_scene_lif, png_to_thumb_lif, Align, ThumbFit, INV_H, INV_W,
};

fn never() -> bool {
    false
}

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn expected() -> serde_json::Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/matrix_expected.json");
    serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap()
}

fn table(v: &serde_json::Value, key: &str) -> BTreeMap<String, String> {
    v[key]
        .as_object()
        .unwrap_or_else(|| panic!("matrix_expected.json : `{key}` absent"))
        .iter()
        .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string()))
        .collect()
}

#[test]
fn references_figees_completes() {
    let want = expected();
    let pngs = table(&want, "pngs");
    let lifs = table(&want, "lifs");
    // Matrice complète de PORT-RUST.md §4.3 : 1465 cellules sur 353 PNG.
    assert_eq!((pngs.len(), lifs.len()), (353, 1465));
    for id in lifs.keys() {
        let (png, _op) = id.split_once("__").expect("identifiant <png>__<op>");
        assert!(pngs.contains_key(png), "{id} : PNG sans empreinte");
    }
}

#[test]
#[ignore = "lancé par la CI de parité avec TELMI2FLAM_MATRIX_DIR (make_matrix.py)"]
fn matrice_identique_aux_references_figees() {
    let dir = PathBuf::from(std::env::var("TELMI2FLAM_MATRIX_DIR").expect("TELMI2FLAM_MATRIX_DIR"));
    let out = std::env::var_os("TELMI2FLAM_MATRIX_OUT").map(PathBuf::from);
    if let Some(out) = &out {
        std::fs::create_dir_all(out).unwrap();
    }
    let want = expected();
    let pngs = table(&want, "pngs");
    let lifs = table(&want, "lifs");
    let index: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("matrix.json")).unwrap()).unwrap();
    let entries = index.as_array().expect("matrix.json : tableau");
    let mut seen = BTreeMap::new();
    let mut failures = Vec::new();
    let mut checked_png = BTreeMap::new();
    for e in entries {
        let id = e["id"].as_str().unwrap();
        let rel = e["png"].as_str().unwrap();
        let png = std::fs::read(dir.join(rel)).unwrap();
        let pid = Path::new(rel).file_stem().unwrap().to_str().unwrap();
        let png_ok = *checked_png
            .entry(pid.to_string())
            .or_insert_with(|| pngs.get(pid) == Some(&hex(&png)));
        if !png_ok {
            failures.push(format!("{id} : PNG d'entrée différent du PNG figé (générateur modifié ?)"));
            continue;
        }
        let lif = match e["op"].as_str().unwrap() {
            "scene" => png_to_scene_lif(&png, &never),
            "thumb" => png_to_thumb_lif(&png, ThumbFit::Bandes, &never),
            "inventory" => png_to_fit_lif(&png, INV_W, INV_H, Align::Center, &never),
            "title" => png_to_fit_lif(&png, 89, 120, Align::Left, &never),
            "bound" => png_to_lif(&png, &never),
            op => panic!("opération inconnue {op}"),
        }
        .unwrap_or_else(|err| panic!("{id} : {err:?}"));
        let have = hex(&lif);
        match lifs.get(id) {
            None => failures.push(format!("{id} : cellule sans référence figée")),
            Some(sha) if *sha != have => failures.push(format!("{id} : .lif différent")),
            Some(_) => {}
        }
        if let Some(out) = &out {
            std::fs::write(out.join(format!("{id}.lif")), &lif).unwrap();
        }
        seen.insert(id.to_string(), ());
    }
    for id in lifs.keys().filter(|id| !seen.contains_key(*id)) {
        failures.push(format!("{id} : cellule figée absente de matrix.json"));
    }
    if let Some(out) = &out {
        std::fs::write(
            out.join("ecarts.json"),
            serde_json::to_string_pretty(&failures).unwrap(),
        )
        .unwrap();
    }
    assert!(
        failures.is_empty(),
        "{} écart(s) sur {} cellules :\n{}",
        failures.len(),
        entries.len(),
        failures.join("\n")
    );
    println!("matrice : {} cellules identiques", entries.len());
}
