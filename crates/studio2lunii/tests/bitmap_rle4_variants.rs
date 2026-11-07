//! BMP RLE4 de 320 x 240 « presque conformes » : Lunii.QT les recopie tels quels, donc les 16
//! niveaux sont les indices de palette. Ici ils sont décodés en indices (jamais passés par les
//! couleurs de la palette, ce qui ferait perdre un niveau) puis réencodés au format exact.

mod common;

use studio2lunii::bitmap::{to_lunii_bmp, BitmapError};

const W: usize = 320;
const H: usize = 240;

/// Niveau du pixel (x, y), y de haut en bas ; couvre les 16 niveaux sur chaque ligne.
fn level(x: usize, y: usize) -> u8 {
    ((x / 3 + y) % 16) as u8
}

fn expected_levels() -> Vec<u8> {
    (0..H)
        .flat_map(|y| (0..W).map(move |x| level(x, y)))
        .collect()
}

/// Données RLE4 en mélangeant mode absolu (lignes paires, rembourrage inclus) et séries codées
/// à deux demi-octets différents (lignes impaires). `end_of_file` : `00 01` final.
fn rle_data(end_of_file: bool) -> Vec<u8> {
    let mut d = Vec::new();
    for (n, y) in (0..H).rev().enumerate() {
        let row: Vec<u8> = (0..W).map(|x| level(x, y)).collect();
        if y % 2 == 0 {
            for (start, count) in [(0usize, 5usize), (5, 255), (260, 60)] {
                d.extend_from_slice(&[0, count as u8]);
                let mut bytes: Vec<u8> = row[start..start + count]
                    .chunks(2)
                    .map(|p| (p[0] << 4) | p.get(1).copied().unwrap_or(0))
                    .collect();
                if bytes.len() % 2 == 1 {
                    bytes.push(0);
                }
                d.extend(bytes);
            }
        } else {
            for p in row.chunks(2) {
                d.extend_from_slice(&[2, (p[0] << 4) | p[1]]);
            }
        }
        if n < H - 1 {
            d.extend_from_slice(&[0, 0]);
        }
    }
    if end_of_file {
        d.extend_from_slice(&[0, 1]);
    }
    d
}

/// En-tête BITMAPINFOHEADER « libre » : taille de fichier et de données à 0, résolution non
/// nulle, palette rampe 0..15 (et non la rampe de l'appareil), couleurs importantes à 16.
fn bmp_with(data: &[u8], palette: impl Fn(usize) -> [u8; 4]) -> Vec<u8> {
    let mut v = b"BM".to_vec();
    v.extend_from_slice(&[0; 4]);
    v.extend_from_slice(&[0; 4]);
    v.extend_from_slice(&118u32.to_le_bytes());
    v.extend_from_slice(&40u32.to_le_bytes());
    v.extend_from_slice(&(W as u32).to_le_bytes());
    v.extend_from_slice(&(H as u32).to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&4u16.to_le_bytes());
    v.extend_from_slice(&2u32.to_le_bytes());
    v.extend_from_slice(&0u32.to_le_bytes());
    v.extend_from_slice(&2835u32.to_le_bytes());
    v.extend_from_slice(&2835u32.to_le_bytes());
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&16u32.to_le_bytes());
    for i in 0..16 {
        v.extend_from_slice(&palette(i));
    }
    v.extend_from_slice(data);
    v
}

fn ramp(i: usize) -> [u8; 4] {
    [i as u8, i as u8, i as u8, 0]
}

#[test]
fn almost_conformant_rle4_keeps_the_16_levels_exactly() {
    let want = expected_levels();
    // Palettes : rampe 0..15, rampe de l'appareil, inversée, colorée : les indices comptent.
    let palettes: [Box<dyn Fn(usize) -> [u8; 4]>; 4] = [
        Box::new(ramp),
        Box::new(|i| {
            [(i * 255 / 16) as u8; 3]
                .into_iter()
                .chain([0])
                .collect::<Vec<_>>()
                .try_into()
                .unwrap()
        }),
        Box::new(|i| ramp(15 - i)),
        Box::new(|i| [(i * 16) as u8, 200, (255 - i * 16) as u8, 0]),
    ];
    for (k, pal) in palettes.iter().enumerate() {
        let src = bmp_with(&rle_data(true), pal);
        let out = to_lunii_bmp(&src).unwrap_or_else(|e| panic!("palette {k} : {e}"));
        assert_eq!(
            common::decode_device_bmp(&out).unwrap(),
            want,
            "palette {k}"
        );
    }
}

#[test]
fn trailing_bytes_after_end_of_rle_are_dropped() {
    let mut data = rle_data(true);
    data.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
    let out = to_lunii_bmp(&bmp_with(&data, ramp)).unwrap();
    assert_eq!(common::decode_device_bmp(&out).unwrap(), expected_levels());
    assert!(!out.windows(4).any(|w| w == [0xDE, 0xAD, 0xBE, 0xEF]));
}

#[test]
fn malformed_rle4_of_device_size_is_refused_not_degraded() {
    let good = rle_data(true);
    let mut cases: Vec<(&str, Vec<u8>)> = Vec::new();
    cases.push(("sans fin de fichier", rle_data(false)));
    let mut delta = good.clone();
    delta.splice(0..0, [0, 2, 1, 1]);
    cases.push(("saut (delta)", delta));
    let mut short_row = good.clone();
    short_row.splice(0..2, [1, 0x00]); // première série réduite à 1 pixel
    cases.push(("ligne incomplète", short_row));
    let mut long_row = good.clone();
    long_row.splice(0..0, [2, 0x11]);
    cases.push(("ligne trop longue", long_row));
    let cut = good.len() - 2 - 2 - 160 * 2; // une ligne de moins
    let mut fewer = good[..cut].to_vec();
    fewer.extend_from_slice(&[0, 1]);
    cases.push(("239 lignes", fewer));
    cases.push(("tronqué", good[..good.len() / 2].to_vec()));
    cases.push(("vide", Vec::new()));
    for (name, data) in cases {
        let r = to_lunii_bmp(&bmp_with(&data, ramp));
        assert!(matches!(r, Err(BitmapError::Decode(_))), "{name} : {r:?}");
    }
}

#[test]
fn rle4_data_offset_outside_the_file_is_refused() {
    let mut b = bmp_with(&rle_data(true), ramp);
    for off in [0u32, 20, 117, u32::MAX, b.len() as u32 + 1] {
        b[10..14].copy_from_slice(&off.to_le_bytes());
        assert!(
            matches!(to_lunii_bmp(&b), Err(BitmapError::Decode(_))),
            "offset {off}"
        );
    }
}
