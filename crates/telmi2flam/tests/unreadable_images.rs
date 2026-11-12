//! Image que Pillow ne sait pas identifier (#574) : `image_unreadable` et `empty.lif`, la
//! conversion réussit (code 0). Avant le resserrement d'`identify.rs`, un fichier de zéros,
//! du texte, un SVG, une page HTML ou un octet `0x1C` suivi de déchets passaient pour un
//! format que Pillow lirait, et arrêtaient toute la conversion en `unsupported_format` (code 7).
//!
//! Le cas de départ est `img_bmp_delegated` de `package_cases.zip` (image de scène en BMP,
//! entrées ZIP stockées) : le BMP est remplacé, à longueur égale, par les données à essayer, et
//! la somme CRC-32 de l'entrée est recalculée.

use std::io::Cursor;
use std::process::Command;

use telmi2flam::archive::Archive;

const VECTORS: &[u8] = include_bytes!("data/package_cases.zip");
const ENTRY: &str = "images/s0.png";

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    hay[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|i| from + i)
}

/// Remplace le contenu de `ENTRY` (entrée stockée) par `payload` rempli de `fill` jusqu'à la
/// même longueur, et met à jour le CRC-32 des deux en-têtes.
fn replace_entry(zip: &mut [u8], payload: &[u8], fill: u8) {
    let name = ENTRY.as_bytes();
    // En-tête local : `PK\3\4`, nom en 30, longueur du nom en 26, des extras en 28, CRC en 14,
    // taille en 22.
    let mut local = None;
    let mut at = 0;
    while let Some(p) = find(zip, b"PK\x03\x04", at) {
        let nlen = usize::from(u16::from_le_bytes([zip[p + 26], zip[p + 27]]));
        if &zip[p + 30..p + 30 + nlen] == name {
            local = Some(p);
            break;
        }
        at = p + 4;
    }
    let p = local.expect("entrée de l'archive");
    let nlen = usize::from(u16::from_le_bytes([zip[p + 26], zip[p + 27]]));
    let elen = usize::from(u16::from_le_bytes([zip[p + 28], zip[p + 29]]));
    let size = u32::from_le_bytes(zip[p + 22..p + 26].try_into().unwrap()) as usize;
    let start = p + 30 + nlen + elen;
    assert!(payload.len() <= size, "charge utile trop longue");
    let mut data = payload.to_vec();
    data.resize(size, fill);
    zip[start..start + size].copy_from_slice(&data);
    let mut crc = flate2::Crc::new();
    crc.update(&data);
    let crc = crc.sum().to_le_bytes();
    zip[p + 14..p + 18].copy_from_slice(&crc);
    // Répertoire central : `PK\1\2`, nom en 46, longueur du nom en 28, CRC en 16.
    let mut at = 0;
    while let Some(c) = find(zip, b"PK\x01\x02", at) {
        let nlen = usize::from(u16::from_le_bytes([zip[c + 28], zip[c + 29]]));
        if &zip[c + 46..c + 46 + nlen] == name {
            zip[c + 16..c + 20].copy_from_slice(&crc);
            return;
        }
        at = c + 4;
    }
    panic!("répertoire central");
}

/// Convertit le cas avec l'entrée remplacée ; rend `(code, stdout, stderr)`.
fn run(tag: &str, payload: Option<(&[u8], u8)>) -> (i32, String, String) {
    let dir = std::env::temp_dir().join(format!("t2f-unreadable-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut archive = Archive::new(Cursor::new(VECTORS), "package_cases.zip".into()).unwrap();
    let mut data = archive
        .read("cases/img_bmp_delegated.zip")
        .unwrap()
        .unwrap();
    if let Some((payload, fill)) = payload {
        replace_entry(&mut data, payload, fill);
    }
    std::fs::write(dir.join("in.zip"), data).unwrap();
    let res = Command::new(env!("CARGO_BIN_EXE_telmi2flam"))
        .current_dir(&dir)
        .args([
            "in.zip",
            "-o",
            "o.plain.pk",
            "--plain",
            "--thumb-fit",
            "bandes",
        ])
        .env("FLAM_FFMPEG", "")
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let text = |b: &[u8]| String::from_utf8_lossy(b).replace("\r\n", "\n");
    (
        res.status.code().unwrap(),
        text(&res.stdout),
        text(&res.stderr),
    )
}

/// Un vrai format que Pillow lit mais que le Rust ne décode pas (BMP) reste délégué : le
/// resserrement ne change pas D4 pour les formats reconnus.
#[test]
fn le_bmp_reste_delegue() {
    let (code, _, err) = run("bmp", None);
    assert_eq!(code, 7, "{err}");
}

#[test]
fn les_dechets_sont_illisibles_sans_echec() {
    let svg =
        b"<?xml version=\"1.0\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\">\n<rect/>\n</svg>\n";
    let html = b"<!DOCTYPE html>\n<html>\n<body>404 Not Found</body>\n</html>\n";
    let text = b"premiere ligne\ndeuxieme ligne\ntroisieme ligne\n";
    let cases: [(&str, &[u8], u8); 6] = [
        ("zeros", b"", 0),
        ("texte", text, b' '),
        ("svg", svg, b' '),
        ("html", html, b' '),
        ("iptc", b"\x1c", 0xAA),
        (
            "tga_dimensions_nulles",
            &[5, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 24, 0],
            0,
        ),
    ];
    for (tag, payload, fill) in cases {
        let (code, out, err) = run(tag, Some((payload, fill)));
        assert_eq!(code, 0, "{tag} : {out}{err}");
        assert!(
            err.contains("image illisible") && err.contains("empty.lif"),
            "{tag} : {err}"
        );
    }
}
