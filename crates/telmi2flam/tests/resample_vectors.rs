//! Parité des conversions PNG vers LIF (redimensionnement porté de Pillow) avec
//! `png_to_*_lif` de Python 3.14.8 + Pillow 12.3.0 : vecteurs produits par
//! `gen_resample_vectors.py` (supprimé).

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use telmi2flam::image::{
    png_to_fit_lif, png_to_lif, png_to_scene_lif, png_to_thumb_lif, Align, ThumbFit, INV_H, INV_W,
};
use telmi2flam::png::PngError;

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/vectors/resample")
}

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("{} : {e}", path.display()))
}

fn never() -> bool {
    false
}

/// Opération de la matrice (mêmes appels que `matrix_render.rs`).
fn convert(png: &[u8], op: &str) -> Result<Vec<u8>, PngError> {
    match op {
        "scene" => png_to_scene_lif(png, &never),
        "thumb" => png_to_thumb_lif(png, ThumbFit::Bandes, &never),
        "thumb_flou" => png_to_thumb_lif(png, ThumbFit::Flou, &never),
        "thumb_rogner" => png_to_thumb_lif(png, ThumbFit::Rogner, &never),
        "inventory" => png_to_fit_lif(png, INV_W, INV_H, Align::Center, &never),
        "title" => png_to_fit_lif(png, 89, 120, Align::Left, &never),
        "bound" => png_to_lif(png, &never),
        _ => panic!("opération inconnue {op}"),
    }
}

/// Résultat au format du manifeste (`lif:<octets>:<sha256>` ou `unreadable`), ou `delegate`.
fn outcome(png: &[u8], op: &str) -> String {
    match convert(png, op) {
        Ok(lif) => {
            let hex: String = Sha256::digest(&lif)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            format!("lif:{}:{hex}", lif.len())
        }
        Err(PngError::Unreadable(_)) => "unreadable".into(),
        Err(PngError::Unsupported(_)) => "delegate".into(),
        Err(PngError::Cancelled) => unreachable!(),
    }
}

/// Compare chaque ligne `nom  opération  attendu` d'un manifeste : (lignes, écarts).
fn check_manifest(dir: &Path) -> (usize, Vec<String>) {
    let text = String::from_utf8(read(&dir.join("manifest.tsv"))).unwrap();
    let rows: Vec<Vec<&str>> = text
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.split('\t').collect())
        .collect();
    let mut errors = Vec::new();
    let (mut name, mut png) = ("", Vec::new());
    for row in &rows {
        if row[0] != name {
            name = row[0];
            png = read(&dir.join(format!("{name}.png")));
        }
        let got = outcome(&png, row[1]);
        if got != row[2] {
            errors.push(format!(
                "{name} {} : attendu {}, obtenu {got}",
                row[1], row[2]
            ));
        }
    }
    (rows.len(), errors)
}

#[test]
fn conversions_identiques_a_pillow() {
    let (n, errors) = check_manifest(&dir());
    assert!(n > 700, "manifeste trop court : {n}");
    assert!(
        errors.is_empty(),
        "{} écarts sur {n} :\n{}",
        errors.len(),
        errors.join("\n")
    );
}
