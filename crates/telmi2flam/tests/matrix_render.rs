//! Rendu des `.lif` de la matrice de rééchantillonnage par le Rust, pour la CI de parité (#134).
//!
//! Test ignoré par défaut : il lit `matrix.json` (produit par `tests/parity/make_corpus.py`) dans
//! `TELMI2FLAM_MATRIX_DIR` et écrit un `<id>.lif` par entrée dans `TELMI2FLAM_MATRIX_OUT`. La
//! comparaison à l'oracle se fait ensuite par `parity.py matrix DIR --candidate-lif-dir OUT`.
//! Mêmes opérations que `make_corpus.python_lif`.

use std::path::PathBuf;

use telmi2flam::image::{
    png_to_fit_lif, png_to_lif, png_to_scene_lif, png_to_thumb_lif, Align, ThumbFit, INV_H, INV_W,
};

fn never() -> bool {
    false
}

#[test]
#[ignore = "lancé par la CI de parité avec TELMI2FLAM_MATRIX_DIR et TELMI2FLAM_MATRIX_OUT"]
fn rend_les_lif_de_la_matrice() {
    let dir = PathBuf::from(std::env::var("TELMI2FLAM_MATRIX_DIR").expect("TELMI2FLAM_MATRIX_DIR"));
    let out = PathBuf::from(std::env::var("TELMI2FLAM_MATRIX_OUT").expect("TELMI2FLAM_MATRIX_OUT"));
    std::fs::create_dir_all(&out).unwrap();
    let index: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("matrix.json")).unwrap()).unwrap();
    let entries = index.as_array().expect("matrix.json : tableau");
    assert!(!entries.is_empty(), "matrice vide");
    for e in entries {
        let id = e["id"].as_str().unwrap();
        let png = std::fs::read(dir.join(e["png"].as_str().unwrap())).unwrap();
        let lif = match e["op"].as_str().unwrap() {
            "scene" => png_to_scene_lif(&png, &never),
            "thumb" => png_to_thumb_lif(&png, ThumbFit::Bandes, &never),
            "inventory" => png_to_fit_lif(&png, INV_W, INV_H, Align::Center, &never),
            "title" => png_to_fit_lif(&png, 89, 120, Align::Left, &never),
            "bound" => png_to_lif(&png, &never),
            op => panic!("opération inconnue {op}"),
        }
        .unwrap_or_else(|err| panic!("{id} : {err:?}"));
        std::fs::write(out.join(format!("{id}.lif")), lif).unwrap();
    }
    println!("matrice : {} .lif écrits", entries.len());
}
