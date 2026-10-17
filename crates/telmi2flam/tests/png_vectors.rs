//! Parité du décodage PNG avec `_open_rgba` (Python 3.14.8 + Pillow 12.3.0) : vecteurs produits
//! par `tests/vectors/png/gen_png_vectors.py`.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use telmi2flam::png::{decode_rgba, PngError};

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/vectors/png")
}

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("{} : {e}", path.display()))
}

fn rows(path: &Path) -> Vec<Vec<String>> {
    String::from_utf8(read(path))
        .unwrap()
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.split('\t').map(str::to_owned).collect())
        .collect()
}

fn never() -> bool {
    false
}

/// Résultat au format du manifeste (`rgba:<l>x<h>:<sha256>` ou `unreadable`), ou `delegate`.
fn outcome(data: &[u8]) -> String {
    match decode_rgba(data, &never) {
        Ok(img) => {
            let digest = Sha256::digest(&img.rgba);
            let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
            format!("rgba:{}x{}:{hex}", img.width, img.height)
        }
        Err(PngError::Unreadable(_)) => "unreadable".into(),
        Err(PngError::Unsupported(_)) => "delegate".into(),
        Err(PngError::Cancelled) => unreachable!(),
    }
}

/// Compare chaque ligne `nom  attendu  rust` d'un manifeste ; rend les écarts.
fn check_manifest(dir: &Path) -> (usize, Vec<String>) {
    let mut errors = Vec::new();
    let rows = rows(&dir.join("manifest.tsv"));
    for row in &rows {
        let (name, expected, rust) = (&row[0], &row[1], &row[2]);
        let got = outcome(&read(&dir.join(format!("{name}.png"))));
        let want = if rust == "delegate" {
            "delegate"
        } else {
            expected
        };
        if got != want {
            errors.push(format!("{name} : attendu {want}, obtenu {got}"));
        }
    }
    (rows.len(), errors)
}

#[test]
fn manifeste_identique_a_pillow() {
    let (n, errors) = check_manifest(&dir());
    assert!(n > 300, "manifeste trop court : {n}");
    assert!(
        errors.is_empty(),
        "{} écarts :\n{}",
        errors.len(),
        errors.join("\n")
    );
}

#[test]
fn troncatures_et_octets_alteres_identiques_a_pillow() {
    let mut errors = Vec::new();
    let mut delegated = 0;
    let rows = rows(&dir().join("alterations.tsv"));
    let mut base_name = String::new();
    let mut base = Vec::new();
    for row in &rows {
        if row[0] != base_name {
            base_name.clone_from(&row[0]);
            base = read(&dir().join(format!("{base_name}.png")));
        }
        let arg: usize = row[2].parse().unwrap();
        let data = match row[1].as_str() {
            "cut" => base[..arg].to_vec(),
            "xor" => {
                let mut d = base.clone();
                d[arg] ^= 0xFF;
                d
            }
            op => panic!("opération inconnue {op}"),
        };
        let got = outcome(&data);
        if !data.starts_with(&[0x89, b'P', b'N', b'G', 13, 10, 26, 10]) {
            // autre format : toujours délégué (Pillow pourrait le lire)
            if got != "delegate" {
                errors.push(format!(
                    "{} {} {} : non délégué ({got})",
                    row[0], row[1], arg
                ));
            }
        } else if got == "delegate" {
            // délégation à Python : jamais une divergence, mais doit rester rare
            delegated += 1;
        } else if got != row[3] {
            errors.push(format!(
                "{} {} {} : attendu {}, obtenu {got}",
                row[0], row[1], arg, row[3]
            ));
        }
    }
    assert!(rows.len() > 900);
    assert!(
        errors.is_empty(),
        "{} écarts :\n{}",
        errors.len(),
        errors.join("\n")
    );
    assert!(
        delegated * 50 < rows.len(),
        "trop de délégations : {delegated}"
    );
}

/// Matrice de `make_corpus.py` (#123), produite hors dépôt par
/// `gen_png_vectors.py --matrix <dossier>` : `TELMI2FLAM_PNG_MATRIX=<dossier> cargo test -p
/// telmi2flam --test png_vectors -- --ignored`.
#[test]
#[ignore = "matrice générée hors dépôt (TELMI2FLAM_PNG_MATRIX)"]
fn matrice_make_corpus() {
    let dir = PathBuf::from(std::env::var("TELMI2FLAM_PNG_MATRIX").expect("TELMI2FLAM_PNG_MATRIX"));
    let (n, errors) = check_manifest(&dir);
    eprintln!("{n} images de la matrice comparées");
    assert!(n > 0);
    assert!(
        errors.is_empty(),
        "{} écarts :\n{}",
        errors.len(),
        errors.join("\n")
    );
}
