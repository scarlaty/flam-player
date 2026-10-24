//! Parité avec Python du paquet assemblé en mémoire ([`build_package`]) : les vecteurs
//! `tests/data/package_cases.zip` sont produits par `gen_package_cases.py` (supprimé)
//! (conversion complète par l'ancien convertisseur Python, Python 3.14.8 + Pillow 12.3.0,
//! vignette en `bandes` par défaut, `flou` ou `rogner` selon le champ `thumb_fit` du cas, sans
//! ffmpeg).
//!
//! Pour chaque cas : mêmes entrées dans le même ordre et mêmes octets (SHA-256), mêmes codes
//! d'avertissement dans le même ordre, ou même code d'erreur. Les écarts assumés du port
//! (PORT-RUST.md §3 et D4) sont vérifiés à part : le Rust échoue aussi, avec le code prévu.
//! Cas JPEG (étiquette `jpeg_bounded`, D7, #459) : les entrées issues d'un JPEG
//! (`jpeg_entries`) peuvent différer, de même taille d'image et à ±4 près avant quantification
//! par rapport au `.lif` de Python (`jpeg_lif/<id>/<entrée>` des vecteurs) ; le SHA-256 du
//! `.plain.pk` n'est alors pas comparé.
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
use telmi2flam::image::ThumbFit;
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
        // `thumb_fit` absent : `bandes`, le mode des vecteurs sans `--thumb-fit`
        // (gen_package_cases.py)
        thumb_fit: match case.get("thumb_fit").and_then(Value::as_str) {
            None | Some("bandes") => ThumbFit::Bandes,
            Some("flou") => ThumbFit::Flou,
            Some("rogner") => ThumbFit::Rogner,
            Some(other) => panic!("thumb_fit {other}"),
        },
    }
}

/// Vecteurs produits sans ffmpeg (`FLAM_FFMPEG=""`) : même réglage ici, quel que soit le
/// `PATH` de la machine de test (avec ffmpeg : `tests/ffmpeg.rs`).
///
/// `set_var` sous le harnais multi-fil (#369) : seul ce `Once` écrit l'environnement, une
/// fois, et tout code qui lit `FLAM_FFMPEG` (`find_ffmpeg`, appelé par `convert`) passe
/// d'abord par ici, donc attend la fin de l'écriture ; les autres tests de ce binaire ne
/// lisent pas l'environnement. Pas de `[env]` dans `.cargo/config.toml` : il s'appliquerait
/// aussi au launcher du même workspace, et un binaire de test lancé à la main n'en profiterait pas.
fn without_ffmpeg() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| std::env::set_var("FLAM_FFMPEG", ""));
}

fn convert(
    data: &[u8],
    options: &Options,
    cancelled: &dyn Fn() -> bool,
) -> Result<Package, ConvError> {
    without_ffmpeg();
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
            // D4 : image d'un format non lu (ni PNG, ni GIF, ni JPEG pris en charge) ; bzip2/LZMA/Zstandard lus depuis #458.
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

/// Écart JPEG admis (D7, même règle que `parity.py`) avant quantification.
const JPEG_MAX_DEVIATION: u8 = 4;

/// Écart minimal avant quantification entre deux `.lif` (pas de 8, 4, 8 pour le RGB565
/// décodé : un écart décodé `d` vient au moins de `d - (pas - 1)`), ou `None` si les tailles
/// ou l'alpha (exact : JPEG opaque) diffèrent, ou qu'un fichier est illisible.
fn jpeg_deviation(python: &[u8], rust: &[u8]) -> Option<u8> {
    const STEPS: [u8; 4] = [8, 4, 8, 1];
    let (a, b) = (
        telmi2flam::lif::decode(python).ok()?,
        telmi2flam::lif::decode(rust).ok()?,
    );
    if (a.width, a.height) != (b.width, b.height) {
        return None;
    }
    let mut max = 0;
    for (i, (x, y)) in a.rgba.iter().zip(&b.rgba).enumerate() {
        let d = x.abs_diff(*y);
        if d > 0 && i % 4 == 3 {
            return None;
        }
        if d > 0 {
            max = max.max(d - (STEPS[i % 4] - 1).min(d));
        }
    }
    Some(max)
}

/// Écrit le paquet dans `dir`, le relit (mêmes entrées, dans le même ordre, que `pkg`) et, si
/// `sha` (conteneur attendu identique à celui de Python), compare son SHA-256.
fn check_pk(case: &Value, pkg: &Package, dir: &std::path::Path, sha: bool) -> Result<(), String> {
    let out = dir.join(format!("{}.plain.pk", case["id"].as_str().unwrap()));
    let target = Target::new(Some(&out), false, pkg);
    write_package(pkg, &target, &|| false).map_err(|e| format!("écriture : {e}"))?;
    let bytes = std::fs::read(&out).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&out);
    let mut back = Archive::new(Cursor::new(&bytes[..]), "relu".into())
        .map_err(|e| format!(".plain.pk relu : {e}"))?;
    let names = back.names().to_vec();
    let want_names: Vec<&String> = pkg.entries.iter().map(|(n, _)| n).collect();
    if names.iter().collect::<Vec<_>>() != want_names {
        return Err(format!(".plain.pk relu : entrées {names:?}"));
    }
    for (name, data) in &pkg.entries {
        if back.read(name).map_err(|e| e.to_string())?.as_ref() != Some(data) {
            return Err(format!(".plain.pk relu : {name} différent"));
        }
    }
    if !sha {
        return Ok(());
    }
    let want = case["pk_sha256"].as_str().unwrap();
    let have = hex(&Sha256::digest(&bytes));
    if want != have {
        return Err(format!(".plain.pk : SHA-256 Python {want}, Rust {have}"));
    }
    Ok(())
}

/// Message d'avertissement privé de ce qui diffère par construction entre Python et le Rust
/// (appliqué aux deux côtés, texte restant comparé tel quel) :
/// - nom Unicode des caractères remplacés (`U+65E5 CJK UNIFIED IDEOGRAPH-65E5 -> '?'` côté
///   Python, `U+65E5 -> '?'` côté Rust : pas de `unicodedata.name`, voir `describe_replaced`) ;
/// - détail entre parenthèses après `illisible` (image, couverture) ou `JSON invalide` : texte
///   de l'exception de Python (Pillow, `json`, adresse mémoire de `BytesIO`), reformulé par le
///   Rust (documenté dans PORT-RUST.md, « messages »).
fn normalized(message: &str) -> String {
    let mut m = message.to_owned();
    if let Some(at) = m.find("hors police") {
        if let Some(colon) = m[at..].find(" : ") {
            let head = at + colon + 3;
            let items: Vec<String> = m[head..]
                .split(", ")
                .map(|item| match (item.find(' '), item.find(" -> ")) {
                    (Some(sp), Some(arrow)) if item.starts_with("U+") && sp < arrow => {
                        format!("{}{}", &item[..sp], &item[arrow..])
                    }
                    _ => item.to_owned(),
                })
                .collect();
            m = format!("{}{}", &m[..head], items.join(", "));
        }
    }
    // Motifs exacts (#421) : « sortie ffmpeg illisible ou inattendue » n'a pas de détail.
    for keyword in ["image illisible", "couverture illisible", "JSON invalide"] {
        let Some(k) = m.find(keyword) else { continue };
        let Some(open) = m[k..].find(" (").map(|i| k + i + 1) else {
            continue;
        };
        let mut depth = 0;
        for (i, c) in m[open..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        m.replace_range(open + 1..open + i, "...");
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    m
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
        // Exception non rattrapée de Python : le Rust refuse aussi, avec un code attendu
        // (archive corrompue : `archive_unreadable`) et jamais `internal_error`.
        (Some(py), Err(e)) if py.starts_with("python_crash:") => {
            let want = (py == "python_crash:BadZipFile").then_some("archive_unreadable");
            match want {
                Some(code) if e.code != code => Err(format!(
                    "erreur Python {py}, attendu {code}, Rust {} ({e})",
                    e.code
                )),
                None if e.code == "internal_error" => {
                    Err(format!("erreur Python {py}, Rust internal_error ({e})"))
                }
                _ => Ok(()),
            }
        }
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
            // D7 (#459) : entrées issues d'un JPEG, comparées à ±4 près avant quantification
            let jpeg: Vec<&str> = case
                .get("jpeg_entries")
                .and_then(Value::as_array)
                .map(|a| a.iter().map(|n| n.as_str().unwrap()).collect())
                .unwrap_or_default();
            let mut bounded = false;
            if want != have {
                let wn: Vec<&str> = want.iter().map(|e| e.0.as_str()).collect();
                let hn: Vec<&str> = have.iter().map(|e| e.0.as_str()).collect();
                if wn != hn {
                    return Err(format!("entrées : Python {wn:?}\nRust {hn:?}"));
                }
                let mut diff = Vec::new();
                for ((w, h), (name, data)) in want.iter().zip(&have).zip(&pkg.entries) {
                    if w == h {
                        continue;
                    }
                    if tags.contains(&"jpeg_bounded") && jpeg.contains(&name.as_str()) {
                        let id = case["id"].as_str().unwrap();
                        let python = vectors()
                            .read(&format!("jpeg_lif/{id}/{name}"))
                            .unwrap()
                            .unwrap_or_else(|| panic!("{id} : {name} absent des vecteurs"));
                        match jpeg_deviation(&python, data) {
                            Some(d) if d <= JPEG_MAX_DEVIATION => {
                                bounded = true;
                                continue;
                            }
                            d => diff.push(format!("{name} (écart JPEG {d:?})")),
                        }
                    } else {
                        diff.push(name.clone());
                    }
                }
                if !diff.is_empty() {
                    return Err(format!("octets différents : {diff:?}"));
                }
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
            // Messages : le texte de Python (#351), aux écarts de `normalized` près.
            let want_m: Vec<String> = case["warning_messages"]
                .as_array()
                .unwrap()
                .iter()
                .map(|w| normalized(w.as_str().unwrap()))
                .collect();
            let have_m: Vec<String> = pkg
                .warnings
                .iter()
                .map(|w| normalized(&w.message))
                .collect();
            if want_m != have_m {
                return Err(format!(
                    "messages d'avertissement : Python {want_m:?}, Rust {have_m:?}"
                ));
            }
            // conteneur différent par construction (contenus à ±4) : écrit et relu quand même,
            // sans comparer son SHA-256 à celui de Python
            check_pk(case, &pkg, dir, !bounded)
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

/// `Options.thumb_fit` atteint bien `png_to_thumb_lif` : pour une même couverture, les trois
/// modes donnent trois `img/thumbnail.lif` différents, et chacun est celui des vecteurs Python
/// (le SHA-256 du `.plain.pk` est vérifié par `paquet_identique_a_python`).
#[test]
fn thumb_fit_atteint_la_vignette() {
    let cases = expected();
    let mut zip = vectors();
    for shape in ["portrait", "paysage"] {
        let mut thumbs = Vec::new();
        for (mode, fit) in [
            ("flou", ThumbFit::Flou),
            ("rogner", ThumbFit::Rogner),
            ("bandes", ThumbFit::Bandes),
        ] {
            let id = format!("pkg_thumb_{mode}_{shape}");
            let case = cases
                .iter()
                .find(|c| c["id"] == id.as_str())
                .unwrap_or_else(|| panic!("cas {id}"));
            let data = zip.read(&format!("cases/{id}.zip")).unwrap().unwrap();
            let opts = options(case);
            assert_eq!(opts.thumb_fit, fit, "{id}");
            let pkg = convert_on_big_stack(data, opts).unwrap();
            let thumb = pkg
                .entries
                .iter()
                .find(|(name, _)| name == "img/thumbnail.lif")
                .unwrap_or_else(|| panic!("{id} : pas de vignette"))
                .1
                .clone();
            thumbs.push((mode, hex(&Sha256::digest(&thumb))));
        }
        for i in 0..thumbs.len() {
            for j in i + 1..thumbs.len() {
                assert_ne!(
                    thumbs[i].1, thumbs[j].1,
                    "{shape} : {} et {} donnent la même vignette",
                    thumbs[i].0, thumbs[j].0
                );
            }
        }
    }
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
