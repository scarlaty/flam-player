//! Parité de `bitmap::to_lunii_bmp` avec Lunii.QT (`image_to_bitmap_rle4`, Pillow 12).
//!
//! Méthode de l'oracle : un script Python hors dépôt (Pillow, `convert_image.py` de Lunii.QT au
//! commit c8afe43) fabrique des images synthétiques (dégradé, aplats, cercle, diagonale ; petites,
//! grandes, paysage, portrait ; transparentes ; gris, palette, 1 bit ; PNG, JPEG, BMP) dans
//! `tests/data/bitmap/<nom>.in.<ext>` et écrit la sortie de Lunii.QT dans
//! `tests/data/bitmap/<nom>.expected.bmp`.
//!
//! - PNG, BMP et BMP RLE4 conforme : égalité **à l'octet** (le redimensionnement de Pillow est
//!   refait à l'identique).
//! - BMP : Lunii.QT écrit ces sources à l'envers (bogue : il oublie de retourner) ; l'attendu est
//!   sa sortie sur la même image au format PNG, donc à l'endroit.
//! - JPEG : le décodeur diffère de libjpeg ; on exige l'égalité exacte de l'en-tête et de la
//!   structure RLE, et des niveaux égaux à +-1 sur au moins 97 % des pixels.

mod common;

use std::fs;
use std::path::PathBuf;

use common::decode_device_bmp;
use studio2lunii::bitmap::to_lunii_bmp;

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/bitmap")
}

fn cases() -> Vec<(String, PathBuf, PathBuf)> {
    let mut v = Vec::new();
    for e in fs::read_dir(data_dir()).unwrap() {
        let p = e.unwrap().path();
        let name = p.file_name().unwrap().to_str().unwrap().to_string();
        if let Some((stem, _)) = name.split_once(".in.") {
            let exp = data_dir().join(format!("{stem}.expected.bmp"));
            assert!(exp.exists(), "attendu manquant pour {name}");
            v.push((stem.to_string(), p, exp));
        }
    }
    v.sort();
    v
}

#[test]
fn fixtures_are_present() {
    assert!(cases().len() >= 30, "jeu de données incomplet");
}

#[test]
fn exact_parity_for_non_jpeg() {
    let mut failures = Vec::new();
    for (stem, input, expected) in cases() {
        if stem.starts_with("jpeg_") {
            continue;
        }
        let got =
            to_lunii_bmp(&fs::read(&input).unwrap()).unwrap_or_else(|e| panic!("{stem} : {e}"));
        let want = fs::read(&expected).unwrap();
        if got != want {
            let diff = match (decode_device_bmp(&got), decode_device_bmp(&want)) {
                (Ok(a), Ok(b)) => a.iter().zip(&b).filter(|(x, y)| x != y).count(),
                _ => usize::MAX,
            };
            failures.push(format!("{stem} : {diff} pixels différents"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn jpeg_parity_within_tolerance() {
    for (stem, input, expected) in cases() {
        if !stem.starts_with("jpeg_") {
            continue;
        }
        let got = to_lunii_bmp(&fs::read(&input).unwrap()).unwrap();
        let want = fs::read(&expected).unwrap();
        // En-tête identique hors tailles (octets 2..6 et 34..38, qui suivent le contenu RLE).
        assert_eq!(got[..2], want[..2]);
        assert_eq!(got[6..34], want[6..34], "{stem}");
        assert_eq!(got[38..118], want[38..118], "{stem}");
        let g = decode_device_bmp(&got).unwrap();
        let w = decode_device_bmp(&want).unwrap();
        let close = g
            .iter()
            .zip(&w)
            .filter(|(a, b)| a.abs_diff(**b) <= 1)
            .count();
        let ratio = close as f64 / g.len() as f64;
        assert!(ratio >= 0.97, "{stem} : {:.2} % à +-1", ratio * 100.0);
        let max = g.iter().zip(&w).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
        assert!(max <= 3, "{stem} : écart maximal {max}");
    }
}

#[test]
fn every_output_has_the_exact_device_structure() {
    for (stem, input, _) in cases() {
        let out = to_lunii_bmp(&fs::read(&input).unwrap()).unwrap();
        decode_device_bmp(&out).unwrap_or_else(|e| panic!("{stem} : {e}"));
    }
}

#[test]
fn rle4_bmp_of_the_right_size_is_returned_unchanged() {
    let input = fs::read(data_dir().join("bmp_rle4_passthrough.in.bmp")).unwrap();
    assert_eq!(to_lunii_bmp(&input).unwrap(), input);
}

#[test]
fn conversion_is_idempotent() {
    // La sortie est un BMP RLE4 conforme : une seconde conversion la rend telle quelle.
    let input = fs::read(data_dir().join("rgb_640x480.in.png")).unwrap();
    let once = to_lunii_bmp(&input).unwrap();
    assert_eq!(to_lunii_bmp(&once).unwrap(), once);
}

#[test]
fn transparent_pixels_become_black_not_white() {
    // Blanc totalement transparent : Pillow prémultiplie, donc noir (le niveau 0, pas 15).
    let encode = |alpha: u8| {
        let img = image::RgbaImage::from_pixel(40, 30, image::Rgba([255, 255, 255, alpha]));
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        png
    };
    let clear = decode_device_bmp(&to_lunii_bmp(&encode(0)).unwrap()).unwrap();
    assert!(clear.iter().all(|&v| v == 0));
    let opaque = decode_device_bmp(&to_lunii_bmp(&encode(255)).unwrap()).unwrap();
    assert!(opaque.iter().all(|&v| v == 15));
}
