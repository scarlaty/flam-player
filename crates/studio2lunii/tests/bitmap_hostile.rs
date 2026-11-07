//! Entrées hostiles pour `bitmap::to_lunii_bmp` : dimensions géantes annoncées par un en-tête
//! minuscule, fichiers tronqués, faux en-têtes. Aucune ne doit paniquer ni allouer.

use std::fs;
use std::path::PathBuf;

use studio2lunii::bitmap::{to_lunii_bmp, BitmapError, MAX_INPUT_BYTES};

fn fixture(name: &str) -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/bitmap")
        .join(name);
    fs::read(p).unwrap()
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn png_chunk(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut c = (body.len() as u32).to_be_bytes().to_vec();
    let mut tc = kind.to_vec();
    tc.extend_from_slice(body);
    c.extend_from_slice(&tc);
    c.extend_from_slice(&crc32(&tc).to_be_bytes());
    c
}

/// PNG valide en-tête, mais annonçant `w` x `h` et sans données d'image.
fn png_claiming(w: u32, h: u32) -> Vec<u8> {
    let mut ihdr = w.to_be_bytes().to_vec();
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
    v.extend(png_chunk(b"IHDR", &ihdr));
    v.extend(png_chunk(
        b"IDAT",
        &[0x78, 0x9c, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01],
    ));
    v.extend(png_chunk(b"IEND", &[]));
    v
}

fn bmp_claiming(w: i32, h: i32, bpp: u16, compression: u32) -> Vec<u8> {
    let mut v = b"BM".to_vec();
    v.extend_from_slice(&[0; 12]);
    v[10..14].copy_from_slice(&54u32.to_le_bytes());
    v.extend_from_slice(&40u32.to_le_bytes());
    v.extend_from_slice(&w.to_le_bytes());
    v.extend_from_slice(&h.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&bpp.to_le_bytes());
    v.extend_from_slice(&compression.to_le_bytes());
    v.extend_from_slice(&[0; 20]);
    v
}

#[test]
fn giant_png_dimensions_are_refused_before_decoding() {
    for (w, h) in [(100_000, 100_000), (8193, 10), (10, 8193), (7000, 7000)] {
        match to_lunii_bmp(&png_claiming(w, h)) {
            Err(BitmapError::BadDimensions { width, height }) => {
                assert_eq!((width, height), (w, h))
            }
            other => panic!("{w}x{h} : {other:?}"),
        }
    }
}

#[test]
fn giant_bmp_dimensions_are_refused() {
    for (w, h, bpp, comp) in [
        (100_000, 100_000, 24, 0),
        (i32::MAX, 10, 24, 0),
        (320, 240, 4, 2),
        (65_535, 65_535, 8, 1),
    ] {
        let r = to_lunii_bmp(&bmp_claiming(w, h, bpp, comp));
        assert!(r.is_err(), "{w}x{h} : {r:?}");
    }
}

#[test]
fn giant_jpeg_dimensions_are_refused() {
    // SOI, SOF0 65535 x 65535 en 3 composantes, puis plus rien.
    let jpeg = [
        0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11, 0x08, 0xFF, 0xFF, 0xFF, 0xFF, 0x03, 0x01, 0x22, 0x00,
        0x02, 0x11, 0x01, 0x03, 0x11, 0x01, 0xFF, 0xD9,
    ];
    assert!(to_lunii_bmp(&jpeg).is_err());
}

#[test]
fn oversized_input_is_refused_without_parsing() {
    let big = vec![0u8; MAX_INPUT_BYTES + 1];
    assert!(matches!(
        to_lunii_bmp(&big),
        Err(BitmapError::InputTooLarge { .. })
    ));
}

#[test]
fn unknown_formats_and_garbage_are_refused() {
    assert_eq!(to_lunii_bmp(&[]), Err(BitmapError::UnsupportedFormat));
    assert_eq!(
        to_lunii_bmp(b"GIF89a\x01\x00\x01\x00"),
        Err(BitmapError::UnsupportedFormat)
    );
    assert!(to_lunii_bmp(&[0xFF; 4096]).is_err());
    assert!(to_lunii_bmp(b"BM").is_err());
    assert!(to_lunii_bmp(b"\x89PNG\r\n\x1a\n").is_err());
    assert!(to_lunii_bmp(&[0xFF, 0xD8]).is_err());
    let mut junk = b"BM".to_vec();
    junk.extend((0..2000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8));
    assert!(to_lunii_bmp(&junk).is_err());
}

#[test]
fn false_rle4_header_without_valid_data_is_not_passed_through() {
    // En-tête « 320 x 240 RLE4 » complet mais données absentes ou incohérentes.
    let mut v = bmp_claiming(320, 240, 4, 2);
    v.extend_from_slice(&[0; 64]);
    v.extend_from_slice(&[5, 0x11, 0, 1]);
    // Jamais copié : refusé, ou réencodé en un fichier de structure exacte (le décodeur général
    // tolère un RLE4 court et complète en noir).
    if let Ok(out) = to_lunii_bmp(&v) {
        assert_ne!(out, v);
        common::decode_device_bmp(&out).unwrap();
    }
}

#[test]
fn truncated_files_never_panic() {
    for name in [
        "rgb_640x480.in.png",
        "palette_P_160x120.in.png",
        "jpeg_rgb_400x300.in.jpg",
        "bmp_rgb24_500x333.in.bmp",
        "bmp_rle4_passthrough.in.bmp",
    ] {
        let full = fixture(name);
        for cut in [
            0,
            1,
            7,
            8,
            20,
            33,
            54,
            100,
            full.len() / 4,
            full.len() / 2,
            full.len() - 1,
        ] {
            let r = to_lunii_bmp(&full[..cut.min(full.len())]);
            if !name.ends_with(".jpg") && cut + 16 < full.len() {
                assert!(r.is_err(), "{name} coupé à {cut} : accepté");
            }
        }
    }
}

#[test]
fn corrupted_bytes_never_panic() {
    for name in [
        "rgb_640x480.in.png",
        "bmp_rgb24_500x333.in.bmp",
        "jpeg_rgb_400x300.in.jpg",
    ] {
        let full = fixture(name);
        for seed in 0..40usize {
            let mut b = full.clone();
            for k in 0..8 {
                let at = (seed * 7919 + k * 104_729) % b.len();
                b[at] ^= 0xA5u8.wrapping_add((seed + k) as u8);
            }
            let _ = to_lunii_bmp(&b);
        }
    }
}

mod common;

#[test]
fn corrupted_device_bmp_is_never_copied() {
    // Un fichier presque conforme (un compteur RLE modifié) ne passe pas tel quel : il est refusé,
    // ou réencodé en un fichier de structure exacte.
    let good = fixture("bmp_rle4_passthrough.in.bmp");
    for at in [118usize, 119, 200, 400, good.len() - 2, good.len() - 1] {
        let mut b = good.clone();
        b[at] ^= 0x55;
        if let Ok(out) = to_lunii_bmp(&b) {
            common::decode_device_bmp(&out).unwrap();
        }
    }
}
