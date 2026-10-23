//! Parité avec Python du paquet assemblé en mémoire ([`build_package`]) : les vecteurs
//! `tests/data/package_cases.zip` sont produits par `tests/data/gen_package_cases.py`
//! (conversion complète par `tools/telmi2flam/telmi2flam.py`, Python 3.14.8 + Pillow 12.3.0,
//! vignette en mode `bandes`, sans ffmpeg).
//!
//! Pour chaque cas : mêmes entrées dans le même ordre et mêmes octets (SHA-256), mêmes codes
//! d'avertissement dans le même ordre, ou même code d'erreur. Les écarts assumés du port
//! (PORT-RUST.md §3 et D4) sont vérifiés à part : le Rust échoue aussi, avec le code prévu.
//!
//! Écriture (#130) : le `.plain.pk` écrit par [`write_package`] est identique à l'octet près à
//! celui de Python (`pk_sha256`, horloge de `zipfile` figée au 1980-01-01 par le générateur),
//! et [`write_zip`] reproduit `zipfile.writestr` sur des entrées synthétiques (`zip_cases.json` :
//! noms non ASCII, seuils ZIP64 réduits).
//!
//! Test d'injection de panne : annulation au *n*-ième appel du jeton, pour chaque *n*.

use std::cell::Cell;
use std::io::Cursor;

use serde_json::Value;
use sha2::{Digest, Sha256};
use telmi2flam::archive::Archive;
use telmi2flam::error::ConvError;
use telmi2flam::output::{write_package, write_zip, Target, ZipLimits};
use telmi2flam::package::{build_package, Options, Package, Selector};

const VECTORS: &[u8] = include_bytes!("data/package_cases.zip");

fn vectors() -> Archive<Cursor<&'static [u8]>> {
    Archive::new(Cursor::new(VECTORS), "package_cases.zip".into()).unwrap()
}

fn expected() -> Vec<Value> {
    let raw = vectors().read("expected.json").unwrap().unwrap();
    let v: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(v["python"], "3.14.8");
    assert_eq!(v["pillow"], "12.3.0");
    v["cases"].as_array().unwrap().clone()
}

fn options(case: &Value) -> Options {
    Options {
        keep_size: case["keep_size"].as_bool().unwrap(),
        allow_missing: case["allow_missing"].as_bool().unwrap(),
        selector: match case["selector"].as_str().unwrap() {
            "carousel" => Selector::Carousel,
            "image" => Selector::Image,
            other => panic!("sélecteur {other}"),
        },
    }
}

fn convert(
    data: &[u8],
    options: &Options,
    cancelled: &dyn Fn() -> bool,
) -> Result<Package, ConvError> {
    let mut archive = Archive::new(Cursor::new(data), "in.zip".into())?;
    build_package(&mut archive, options, cancelled)
}

/// Conversion dans un fil à pile de 8 Mo, comme le binaire (voir `lib.rs`).
fn convert_on_big_stack(data: Vec<u8>, options: Options) -> Result<Package, ConvError> {
    std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(move || convert(&data, &options, &|| false))
        .unwrap()
        .join()
        .unwrap()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Écarts assumés : le Rust échoue là où Python réussit ou échoue autrement.
fn assumed_divergence(tags: &[&str]) -> Option<&'static str> {
    for tag in tags {
        match *tag {
            // D4 : image non PNG, bzip2 / LZMA délégués à Python.
            "delegated" => return Some("unsupported_format"),
            // §3 / §4.1 : JSON hors limites, entrée chiffrée, nom UTF-8 invalide, position
            // d'en-tête négative : le Rust échoue (code propre au port).
            "json_limit" | "encrypted_entry" | "invalid_utf8_name" | "negative_offset" => {
                return Some("*")
            }
            _ => {}
        }
    }
    None
}

/// Écrit le paquet dans `dir` et compare le SHA-256 du `.plain.pk` à celui de Python.
fn check_pk(case: &Value, pkg: &Package, dir: &std::path::Path) -> Result<(), String> {
    let out = dir.join(format!("{}.plain.pk", case["id"].as_str().unwrap()));
    let target = Target::new(Some(&out), false, pkg);
    write_package(pkg, &target, &|| false).map_err(|e| format!("écriture : {e}"))?;
    let bytes = std::fs::read(&out).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&out);
    let want = case["pk_sha256"].as_str().unwrap();
    let have = hex(&Sha256::digest(&bytes));
    if want != have {
        return Err(format!(".plain.pk : SHA-256 Python {want}, Rust {have}"));
    }
    Ok(())
}

/// Compare un cas ; `Err` décrit l'écart.
fn check(case: &Value, data: Vec<u8>, dir: &std::path::Path) -> Result<(), String> {
    let tags: Vec<&str> = case["tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t.as_str().unwrap())
        .collect();
    let got = convert_on_big_stack(data, options(case));
    if let Some(code) = assumed_divergence(&tags) {
        return match got {
            Err(e) if code == "*" || e.code == code => Ok(()),
            Err(e) => Err(format!("écart assumé {tags:?} : attendu {code}, Rust {e}")),
            Ok(_) => Err(format!("écart assumé {tags:?} : le Rust réussit")),
        };
    }
    match (case.get("error").and_then(Value::as_str), got) {
        // Exception non rattrapée de Python (archive corrompue…) : le Rust refuse aussi.
        (Some(py), Err(_)) if py.starts_with("python_crash:") => Ok(()),
        (Some(py), Err(e)) if e.code == py => Ok(()),
        (Some(py), Err(e)) => Err(format!("erreur Python {py}, Rust {} ({e})", e.code)),
        (Some(py), Ok(_)) => Err(format!("erreur Python {py}, le Rust réussit")),
        (None, Err(e)) => Err(format!("Python réussit, Rust {e}")),
        (None, Ok(pkg)) => {
            let want: Vec<(String, String)> = case["entries"]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| {
                    (
                        e[0].as_str().unwrap().to_owned(),
                        e[1].as_str().unwrap().to_owned(),
                    )
                })
                .collect();
            let have: Vec<(String, String)> = pkg
                .entries
                .iter()
                .map(|(n, d)| (n.clone(), hex(&Sha256::digest(d))))
                .collect();
            if want != have {
                let wn: Vec<&str> = want.iter().map(|e| e.0.as_str()).collect();
                let hn: Vec<&str> = have.iter().map(|e| e.0.as_str()).collect();
                if wn != hn {
                    return Err(format!("entrées : Python {wn:?}\nRust {hn:?}"));
                }
                let diff: Vec<&str> = want
                    .iter()
                    .zip(&have)
                    .filter(|(w, h)| w != h)
                    .map(|(w, _)| w.0.as_str())
                    .collect();
                return Err(format!("octets différents : {diff:?}"));
            }
            let want_w: Vec<&str> = case["warnings"]
                .as_array()
                .unwrap()
                .iter()
                .map(|w| w.as_str().unwrap())
                .collect();
            let have_w: Vec<&str> = pkg.warnings.iter().map(|w| w.code).collect();
            if want_w != have_w {
                return Err(format!(
                    "avertissements : Python {want_w:?}, Rust {have_w:?}"
                ));
            }
            check_pk(case, &pkg, dir)
        }
    }
}

#[test]
fn paquet_identique_a_python() {
    let cases = expected();
    assert!(cases.len() > 100, "{} cas", cases.len());
    let written = cases
        .iter()
        .filter(|c| c.get("pk_sha256").is_some())
        .count();
    assert!(written > 80, "{written} .plain.pk de référence");
    let dir = std::env::temp_dir().join(format!("t2f-parity-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut zip = vectors();
    let mut failures = Vec::new();
    for case in &cases {
        let id = case["id"].as_str().unwrap();
        let data = zip.read(&format!("cases/{id}.zip")).unwrap().unwrap();
        if let Err(why) = check(case, data, &dir) {
            failures.push(format!("{id} : {why}"));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        failures.is_empty(),
        "{} écart(s) sur {} cas :\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}

/// Contenu synthétique de l'entrée `i` (`pattern` du générateur).
fn pattern(i: usize, size: usize) -> Vec<u8> {
    (0..size).map(|j| ((i * 7 + j) % 251) as u8).collect()
}

/// Octets de `zipfile.writestr` (horloge figée) pour des listes d'entrées synthétiques,
/// seuils ZIP64 de Python ou réduits (en-tête local ZIP64 dès `1,05 × taille`, répertoire
/// central ZIP64 par taille, position ou nombre d'entrées).
#[test]
fn zip_identique_a_zipfile() {
    let raw = vectors().read("zip_cases.json").unwrap().unwrap();
    let cases: Value = serde_json::from_slice(&raw).unwrap();
    let cases = cases.as_array().unwrap();
    assert_eq!(cases.len(), 21);
    for case in cases {
        let entries: Vec<(String, Vec<u8>)> = case["entries"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let size = e[1].as_u64().unwrap() as usize;
                (e[0].as_str().unwrap().to_owned(), pattern(i, size))
            })
            .collect();
        let limits = ZipLimits {
            zip64: case["zip64"].as_u64().unwrap(),
            file_count: case["file_count"].as_u64().unwrap() as usize,
        };
        let mut out = Vec::new();
        write_zip(&mut out, &entries, limits, &|| false, &mut |_| Ok(())).unwrap();
        assert_eq!(
            hex(&Sha256::digest(&out)),
            case["sha256"].as_str().unwrap(),
            "{case}"
        );
    }
}

/// Annulation au *n*-ième appel du jeton d'une conversion complète : toujours `cancelled` et
/// aucun paquet ; au-delà, paquet identique à une conversion normale. Chaque *n* pour
/// `missing_image_allowed` ; pour `pkg_name_collisions` (plus long), chaque *n* jusqu'à 100 puis
/// une soixantaine de points répartis sur les boucles de lignes des images, et le dernier appel.
#[test]
fn annulation_a_chaque_etape() {
    let mut zip = vectors();
    for id in ["pkg_name_collisions", "missing_image_allowed"] {
        let data = zip.read(&format!("cases/{id}.zip")).unwrap().unwrap();
        let case = expected().into_iter().find(|c| c["id"] == id).unwrap();
        let opts = options(&case);
        let calls = Cell::new(0usize);
        let reference = convert(&data, &opts, &|| {
            calls.set(calls.get() + 1);
            false
        })
        .unwrap();
        let total = calls.get();
        assert!(total > 10, "{id} : {total} appels du jeton");
        let stride = if id == "missing_image_allowed" {
            1
        } else {
            (total / 60).max(1)
        };
        let points = (0..total.min(100))
            .chain((100..total).step_by(stride))
            .chain([total - 1]);
        for n in points {
            let seen = Cell::new(0usize);
            let got = convert(&data, &opts, &|| {
                seen.set(seen.get() + 1);
                seen.get() > n
            });
            match got {
                Err(e) => assert_eq!(e.code, "cancelled", "{id}, appel {n} : {e}"),
                Ok(_) => panic!("{id} : annulation ignorée à l'appel {n}"),
            }
        }
        let again = convert(&data, &opts, &|| false).unwrap();
        assert_eq!(again.entries, reference.entries, "{id}");
    }
}
