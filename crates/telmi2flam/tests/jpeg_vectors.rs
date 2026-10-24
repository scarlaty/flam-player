//! Parité du décodage JPEG avec `_open_rgba` (Pillow 12.3.0, libjpeg-turbo 3.1.4.1) : vecteurs
//! produits par `gen_jpeg_vectors.py` (supprimé) (#459, décision D7).
//!
//! Vecteurs lisibles : même taille, alpha identique, chaque composante à ±4 près (écart borné
//! de D7). Verdicts (lu, illisible, délégué) identiques à Python, sauf les écarts assumés
//! listés dans `alterations_divergences.tsv` et [`ASSUMED`].

use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;
use std::path::{Path, PathBuf};

use telmi2flam::archive::Archive;
use telmi2flam::png::{decode_rgba, PngError, Rgba};

/// Écart maximal admis par composante (D7).
const MAX_DEVIATION: u8 = 4;

/// Écarts de verdict assumés sur `vectors.tsv` (PORT-RUST.md §4.4) : nom, verdict du Rust.
const ASSUMED: &[(&str, &str)] = &[
    // codages arithmétique et sans perte : délégués dès que l'en-tête est valide (code 7)
    ("sof_arithmetique", "delegate"),
    ("sof_arithmetique_table_2", "delegate"),
    ("sof_sans_perte", "delegate"),
    // erreur de `_open` et `PCD_` à l'octet 2048 : autre format pour Pillow (PCD), délégué
    // (PORT-RUST.md §4.4), que l'image PCD soit lisible ou non
    ("pcd_polyglotte", "delegate"),
    ("pcd_polyglotte_court", "delegate"),
    ("pcd_libjpeg_puis_pillow", "delegate"),
    ("spider_polyglotte", "delegate"),
    // refus de zune-jpeg (DQT de précision 2, facteur 3, table inutilisée invalide), lus par
    // libjpeg-turbo : délégués
    ("dqt_pq_2", "delegate"),
    ("dqt_pq_2_apres_sos", "delegate"),
    ("sof_facteur_3", "delegate"),
    ("dht_inutilisee_invalide", "delegate"),
    // plus de `MAX_SCANS` balayages : délégué
    ("prog_balayages_trop", "delegate"),
    // flux de tables seules (EOI) suivi d'une image : non pris en charge, délégué
    ("tables_puis_image", "delegate"),
    // balayage unique sans EOI, données complètes : verdict de libjpeg incertain, délégué
    ("fin_seq_sans_eoi", "delegate"),
    ("fin_seq_ff_seul", "delegate"),
    // données utiles jusqu'au bout, à bits nuls (zune-jpeg complète par des 0) : délégué
    ("fin_seq_zeros_sans_eoi", "delegate"),
];

/// Données coupées puis terminées par un marqueur : lues des deux côtés, mais libjpeg laisse les
/// blocs manquants gris uniforme et zune-jpeg non (pixels non comparés, écart assumé de §4.4).
const CORRUPT: &[&str] = &[
    "fin_seq_moitie_eoi",
    "fin_seq_moitie_dht",
    "fin_prog_moitie_eoi",
];

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/vectors/jpeg")
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

fn cases() -> Archive<Cursor<Vec<u8>>> {
    let data = std::fs::read(dir().join("jpeg_cases.zip")).unwrap();
    Archive::new(Cursor::new(data), "jpeg_cases.zip".into()).unwrap()
}

fn never() -> bool {
    false
}

fn verdict(r: &Result<Rgba, PngError>) -> &'static str {
    match r {
        Ok(_) => "ok",
        Err(PngError::Unreadable(_)) => "unreadable",
        Err(PngError::Unsupported(_)) => "delegate",
        Err(PngError::Cancelled) => unreachable!(),
    }
}

/// Écart maximal par composante (`None` si les tailles ou l'alpha diffèrent).
fn deviation(img: &Rgba, want: &[u8]) -> Option<u8> {
    if img.rgba.len() != want.len() {
        return None;
    }
    let mut max = 0;
    for (a, b) in img
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .zip(want.as_chunks::<4>().0)
    {
        if a[3] != b[3] {
            return None;
        }
        for i in 0..3 {
            max = max.max(a[i].abs_diff(b[i]));
        }
    }
    Some(max)
}

#[test]
fn vecteurs_a_quatre_pres_de_pillow() {
    let rows = rows("vectors.tsv");
    let mut zip = cases();
    let mut errors = Vec::new();
    let (mut identical, mut read) = (0, 0);
    let mut worst = BTreeMap::new();
    for r in &rows {
        let (name, want) = (&r[0], &r[1]);
        let data = zip.read(&format!("{name}.jpg")).unwrap().unwrap();
        let got = decode_rgba(&data, &never);
        if let Some((_, v)) = ASSUMED.iter().find(|(n, _)| n == name) {
            if verdict(&got) != *v {
                errors.push(format!("{name} : écart assumé {v}, obtenu {got:?}"));
            }
            continue;
        }
        match (want.strip_prefix("rgba:"), &got) {
            (Some(_), Ok(_)) if CORRUPT.contains(&name.as_str()) => {}
            (Some(size), Ok(img)) => {
                read += 1;
                let have = format!("{}x{}", img.width, img.height);
                let ref_rgba = zip.read(&format!("{name}.rgba")).unwrap().unwrap();
                match deviation(img, &ref_rgba) {
                    _ if have != size => {
                        errors.push(format!("{name} : taille {have}, attendu {size}"))
                    }
                    None => errors.push(format!("{name} : alpha ou longueur différents")),
                    Some(d) if d > MAX_DEVIATION => {
                        errors.push(format!("{name} : écart {d} > {MAX_DEVIATION}"))
                    }
                    Some(d) => {
                        identical += usize::from(d == 0);
                        let kind = name.split('_').next().unwrap().to_owned();
                        let e = worst.entry(kind).or_insert(0);
                        *e = (*e).max(d);
                    }
                }
            }
            (None, got) if verdict(got) == want.as_str() => {}
            _ => errors.push(format!(
                "{name} : attendu {want}, obtenu {:?}",
                verdict(&got)
            )),
        }
    }
    eprintln!(
        "{read} vecteurs lus, {identical} identiques à Pillow, écart max par famille {worst:?}"
    );
    assert!(rows.len() > 200, "trop peu de vecteurs : {}", rows.len());
    assert!(
        errors.is_empty(),
        "{} écarts :\n{}",
        errors.len(),
        errors.join("\n")
    );
}

#[test]
fn troncatures_et_octets_alteres_comme_pillow() {
    let mut zip = cases();
    let rows = rows("alterations.tsv");
    let assumed: BTreeSet<String> = rows_or_empty("alterations_divergences.tsv")
        .into_iter()
        .map(|r| r.join("\t"))
        .collect();
    let mut bases = BTreeMap::new();
    let mut found = BTreeSet::new();
    for r in &rows {
        let base = bases
            .entry(r[0].clone())
            .or_insert_with(|| zip.read(&format!("{}.jpg", r[0])).unwrap().unwrap());
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
        let got = verdict(&decode_rgba(&data, &never));
        // signature JPEG détruite : autre chemin (délégation prudente de `identify`)
        if !data.starts_with(b"\xff\xd8\xff") && got == "delegate" {
            continue;
        }
        if got != r[3] {
            found.insert(format!("{}\t{}\t{}\t{got}", r[0], r[1], r[2]));
        }
    }
    assert!(rows.len() > 3000);
    let unexpected: Vec<&String> = found.difference(&assumed).collect();
    let fixed: Vec<&String> = assumed.difference(&found).collect();
    assert!(
        unexpected.is_empty() && fixed.is_empty(),
        "verdicts différents de Pillow hors liste ({}) :\n{}\nlistés mais conformes ({}) :\n{}",
        unexpected.len(),
        unexpected
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        fixed.len(),
        fixed
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
    );
}

fn rows_or_empty(name: &str) -> Vec<Vec<String>> {
    if dir().join(name).exists() {
        rows(name)
    } else {
        Vec::new()
    }
}

#[test]
fn annulation_avant_et_apres_le_decodage_puis_par_ligne() {
    use std::cell::Cell;
    let mut zip = cases();
    let data = zip.read("rgb_64x48_grad_s2_q95_p.jpg").unwrap().unwrap();
    let calls = Cell::new(0u32);
    decode_rgba(&data, &|| {
        calls.set(calls.get() + 1);
        false
    })
    .unwrap();
    // avant l'analyse, après le parcours, après le décodage, puis une fois par ligne
    assert!(calls.get() >= 3 + 48, "{}", calls.get());
    for n in 0..calls.get() {
        let seen = Cell::new(0u32);
        let got = decode_rgba(&data, &|| {
            seen.set(seen.get() + 1);
            seen.get() > n
        });
        assert_eq!(got, Err(PngError::Cancelled), "appel {n}");
    }
}
