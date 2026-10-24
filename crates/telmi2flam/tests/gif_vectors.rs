//! Parité du décodage GIF avec `_open_rgba` (Python 3.14 + Pillow 12.3.0) : vecteurs produits
//! par `gen_gif_vectors.py` (supprimé) (#459).

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use telmi2flam::png::{decode_rgba, PngError};

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/vectors/gif")
}

fn rows(name: &str) -> Vec<Vec<String>> {
    let path = dir().join(name);
    String::from_utf8(std::fs::read(&path).unwrap_or_else(|e| panic!("{} : {e}", path.display())))
        .unwrap()
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.split('\t').map(str::to_owned).collect())
        .collect()
}

/// GIF en hexadécimal, ou `@<fichier>` pour les données de plus de 64 Kio.
fn unhex(s: &str) -> Vec<u8> {
    if let Some(name) = s.strip_prefix('@') {
        return std::fs::read(dir().join(name)).unwrap();
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn never() -> bool {
    false
}

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

#[test]
fn vecteurs_identiques_a_pillow() {
    let rows = rows("vectors.tsv");
    let errors: Vec<String> = rows
        .iter()
        .filter_map(|r| {
            let got = outcome(&unhex(&r[1]));
            (got != r[2]).then(|| format!("{} : attendu {}, obtenu {got}", r[0], r[2]))
        })
        .collect();
    assert!(rows.len() > 60, "trop peu de vecteurs : {}", rows.len());
    assert!(
        errors.is_empty(),
        "{} écarts :\n{}",
        errors.len(),
        errors.join("\n")
    );
}

#[test]
fn troncatures_et_octets_alteres_identiques_a_pillow() {
    let bases: std::collections::HashMap<String, Vec<u8>> = rows("vectors.tsv")
        .into_iter()
        .map(|r| (r[0].clone(), unhex(&r[1])))
        .collect();
    let rows = rows("alterations.tsv");
    let mut errors = Vec::new();
    for r in &rows {
        let base = &bases[&r[0]];
        let arg: usize = r[2].parse().unwrap();
        let data = match r[1].as_str() {
            "cut" => base[..arg].to_vec(),
            "xor" => {
                let mut d = base.clone();
                d[arg] ^= 0xFF;
                d
            }
            op => panic!("opération inconnue {op}"),
        };
        let got = outcome(&data);
        let gif = data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a");
        // préfixe trop court pour être reconnu : autre chemin (délégation ou illisible)
        if !gif && (got == "delegate" || got == r[3]) {
            continue;
        }
        if got != r[3] {
            errors.push(format!(
                "{} {} {} : attendu {}, obtenu {got}",
                r[0], r[1], arg, r[3]
            ));
        }
    }
    assert!(rows.len() > 100);
    assert!(
        errors.is_empty(),
        "{} écarts :\n{}",
        errors.len(),
        errors.join("\n")
    );
}

#[test]
fn annulation_avant_chaque_ligne_et_dans_les_codes() {
    use std::cell::Cell;
    let rows = rows("vectors.tsv");
    let data = unhex(&rows.iter().find(|r| r[0] == "grand_bruit_256").unwrap()[1]);
    let calls = Cell::new(0u32);
    let count = || {
        calls.set(calls.get() + 1);
        false
    };
    decode_rgba(&data, &count).unwrap();
    // au moins une vérification par ligne décodée et par ligne convertie
    assert!(calls.get() >= 2 * 256, "{}", calls.get());
    let stop = || true;
    assert_eq!(decode_rgba(&data, &stop), Err(PngError::Cancelled));
    // suite de codes de fin sans pixel (1 Mo) : la boucle de chargement reste annulable
    let mut flood =
        b"GIF89a\x04\x00\x05\x00\x00\x00\x00,\x00\x00\x00\x00\x04\x00\x05\x00\x00\x02".to_vec();
    // sous-bloc de 255 octets : codes de 3 bits tous égaux au code de fin (5)
    let mut block = vec![255u8];
    block.extend([0x6D, 0xDB, 0xB6].repeat(85));
    flood.extend(block.repeat(4096));
    let n = Cell::new(0u32);
    let later = || {
        n.set(n.get() + 1);
        n.get() > 3
    };
    assert_eq!(decode_rgba(&flood, &later), Err(PngError::Cancelled));
}
