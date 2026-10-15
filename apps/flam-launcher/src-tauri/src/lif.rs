//! Décodeur LIF (Lunii Image File Format), porté de `src/formats/lif_decoder.c`.
//!
//! Format : magic `liff`/`LIFF`, largeur et hauteur (u32 big-endian), un octet de canaux
//! (`0xA2`), une charge utile d'opcodes (RGB565 + alpha, delta, cache de 64 couleurs, RLE),
//! puis 8 octets de fin. Les fichiers viennent d'archives non fiables : toutes les lectures
//! sont bornées et les dimensions limitées comme dans le décodeur de référence.
//!
//! Le résultat est un tampon RGBA8888 ligne par ligne (le décodeur C produit de l'ARGB pour
//! LVGL ; seul l'ordre des octets diffère).

use crate::thumbnail::RgbaImage;

const HEADER_SIZE: usize = 13;
const END_SIZE: usize = 8;
/// Dimension maximale (comme `LIF_MAX_DIM` du décodeur C).
const MAX_DIM: u32 = 2047;

/// Raison pour laquelle un fichier LIF est refusé.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LifError {
    #[error("fichier LIF trop court")]
    TooShort,
    #[error("signature LIF invalide")]
    BadMagic,
    #[error("dimensions LIF non prises en charge")]
    BadDimensions,
    #[error("fichier LIF sans données d'image")]
    NoPayload,
}

#[derive(Clone, Copy, Default)]
struct Color {
    r: i32,
    g: i32,
    b: i32,
    a: u8,
}

fn hash(c: &Color) -> usize {
    ((c.r * 7 + c.g * 5 + c.b * 3) & 0x3F) as usize
}

fn mod32(v: i32) -> i32 {
    v.rem_euclid(32)
}

fn mod64(v: i32) -> i32 {
    v.rem_euclid(64)
}

fn expand5(v: i32) -> u8 {
    ((v * 256) / 32) as u8
}

fn expand6(v: i32) -> u8 {
    ((v * 256) / 64) as u8
}

/// Somme des écarts RGB entre pixels voisins d'une ligne (détection ligne/colonne).
fn score_coherence(rgba: &[u8], w: usize, h: usize) -> u64 {
    let mut score = 0u64;
    for y in 0..h {
        let row = &rgba[y * w * 4..(y + 1) * w * 4];
        for x in 1..w {
            let (i, p) = (x * 4, (x - 1) * 4);
            for c in 0..3 {
                score += (i32::from(row[i + c]) - i32::from(row[p + c])).unsigned_abs() as u64;
            }
        }
    }
    score
}

/// Réinterprète un tampon stocké en colonnes en tampon par lignes.
fn transpose(src: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut dst = vec![0u8; src.len()];
    for y in 0..h {
        for x in 0..w {
            let s = (x * h + y) * 4;
            let d = (y * w + x) * 4;
            dst[d..d + 4].copy_from_slice(&src[s..s + 4]);
        }
    }
    dst
}

/// Décode un fichier LIF complet en image RGBA8888.
pub fn decode(data: &[u8]) -> Result<RgbaImage, LifError> {
    if data.len() < HEADER_SIZE + END_SIZE {
        return Err(LifError::TooShort);
    }
    if &data[0..4] != b"liff" && &data[0..4] != b"LIFF" {
        return Err(LifError::BadMagic);
    }
    let w = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
    let h = u32::from_be_bytes([data[8], data[9], data[10], data[11]]);
    if w == 0 || h == 0 || w > MAX_DIM || h > MAX_DIM {
        return Err(LifError::BadDimensions);
    }
    let payload = &data[HEADER_SIZE..data.len() - END_SIZE];
    if payload.is_empty() {
        return Err(LifError::NoPayload);
    }

    let (w, h) = (w as usize, h as usize);
    let total = w * h;
    let mut rgba = vec![0u8; total * 4];

    let mut cur = Color {
        a: 255,
        ..Color::default()
    };
    let mut run = 0u32;
    let mut cache = [Color::default(); 64];
    let mut p = 0usize;

    for px in 0..total {
        if run > 0 {
            run -= 1;
        } else if p < payload.len() {
            let b1 = payload[p];
            p += 1;
            match b1 {
                0xFE | 0xFF => {
                    let extra = if b1 == 0xFE { 2 } else { 3 };
                    if p + extra > payload.len() {
                        break;
                    }
                    let (lo, hi) = (payload[p], payload[p + 1]);
                    cur.b = i32::from(lo & 0x1F);
                    cur.g = i32::from(((lo >> 5) & 0x07) | ((hi << 3) & 0x38));
                    cur.r = i32::from((hi >> 3) & 0x1F);
                    if b1 == 0xFF {
                        cur.a = payload[p + 2];
                    }
                    p += extra;
                }
                _ => match b1 & 0xC0 {
                    0x00 => cur = cache[usize::from(b1 & 0x3F)],
                    0x40 => {
                        let db = i32::from((b1 >> 4) & 0x03) - 2;
                        let dg = i32::from((b1 >> 2) & 0x03) - 2;
                        let dr = i32::from(b1 & 0x03) - 2;
                        cur.r = mod32(cur.r + dr);
                        cur.g = mod64(cur.g + dg);
                        cur.b = mod32(cur.b + db);
                    }
                    0x80 => {
                        if p >= payload.len() {
                            break;
                        }
                        let b2 = payload[p];
                        p += 1;
                        let vg = i32::from(b1 & 0x3F) - 32;
                        cur.b = mod32(cur.b + vg - 8 + i32::from((b2 >> 4) & 0x0F));
                        cur.g = mod64(cur.g + vg);
                        cur.r = mod32(cur.r + vg - 8 + i32::from(b2 & 0x0F));
                    }
                    _ => run = u32::from(b1 & 0x3F),
                },
            }
        } else {
            break;
        }

        cache[hash(&cur)] = cur;
        let off = px * 4;
        rgba[off] = expand5(cur.r);
        rgba[off + 1] = expand6(cur.g);
        rgba[off + 2] = expand5(cur.b);
        rgba[off + 3] = cur.a;
    }

    // Images hautes stockées en colonnes : on garde la transposition si elle est au moins
    // 12 % plus cohérente (même heuristique que le décodeur C).
    if w.max(h) > 64 && h > w {
        let score_row = score_coherence(&rgba, w, h);
        if score_row > 0 {
            let transposed = transpose(&rgba, w, h);
            let score_col = score_coherence(&transposed, w, h);
            if score_col < score_row && 1.0 - score_col as f64 / score_row as f64 >= 0.12 {
                rgba = transposed;
            }
        }
    }

    Ok(RgbaImage {
        width: w as u32,
        height: h as u32,
        rgba,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Assemble un fichier LIF à partir d'une charge utile.
    fn lif(w: u32, h: u32, payload: &[u8]) -> Vec<u8> {
        let mut out = b"liff".to_vec();
        out.extend_from_slice(&w.to_be_bytes());
        out.extend_from_slice(&h.to_be_bytes());
        out.push(0xA2);
        out.extend_from_slice(payload);
        out.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 1]);
        out
    }

    /// Octets `(lo, hi)` d'une couleur RGB565.
    fn pack(r: u8, g: u8, b: u8) -> (u8, u8) {
        ((b & 0x1F) | ((g & 0x07) << 5), ((g >> 3) & 0x07) | (r << 3))
    }

    #[test]
    fn decode_un_pixel_rgb565_avec_alpha() {
        let (lo, hi) = pack(31, 63, 0);
        let img = decode(&lif(1, 1, &[0xFF, lo, hi, 200])).unwrap();
        assert_eq!((img.width, img.height), (1, 1));
        assert_eq!(img.rgba, vec![248, 252, 0, 200]);
    }

    #[test]
    fn rle_cache_et_deltas() {
        let (lo, hi) = pack(10, 20, 5);
        let payload = [
            0xFF,
            lo,
            hi,
            255,                            // pixel 0 : couleur + alpha
            0xC1,                           // run = 1 : pixel 1 + 1 répétition (pixel 2)
            0x40 | (2 << 4) | (2 << 2) | 3, // delta (0, 0, +1) sur r : pixel 3
            0x80 | 33,
            0x77, // vert étendu : g+1, r et b inchangés : pixel 4
        ];
        let img = decode(&lif(5, 1, &payload)).unwrap();
        let px = |i: usize| img.rgba[i * 4..i * 4 + 4].to_vec();
        let base = vec![expand5(10), expand6(20), expand5(5), 255];
        assert_eq!(px(0), base);
        assert_eq!(px(1), base);
        assert_eq!(px(2), base);
        assert_eq!(px(3), vec![expand5(11), expand6(20), expand5(5), 255]);
        assert_eq!(px(4), vec![expand5(11), expand6(21), expand5(5), 255]);
    }

    #[test]
    fn index_du_cache() {
        let (lo, hi) = pack(1, 2, 3);
        let idx = ((7 + 2 * 5 + 3 * 3) & 0x3F) as u8;
        let (lo2, hi2) = pack(30, 40, 20);
        // pixel 0 = A (mis en cache), pixel 1 = B (sans alpha), pixel 2 = A relu du cache.
        let payload = [0xFF, lo, hi, 255, 0xFE, lo2, hi2, idx];
        let img = decode(&lif(3, 1, &payload)).unwrap();
        assert_eq!(&img.rgba[0..4], &img.rgba[8..12]);
        assert_ne!(&img.rgba[0..4], &img.rgba[4..8]);
    }

    #[test]
    fn charge_utile_tronquee_laisse_des_pixels_transparents() {
        let (lo, hi) = pack(31, 63, 31);
        let img = decode(&lif(2, 1, &[0xFF, lo, hi, 255])).unwrap();
        assert_eq!(&img.rgba[0..4], &[248, 252, 248, 255]);
        assert_eq!(&img.rgba[4..8], &[0, 0, 0, 0]);
    }

    #[test]
    fn fichiers_invalides_refuses() {
        assert_eq!(decode(b"liff").unwrap_err(), LifError::TooShort);
        let mut bad = lif(1, 1, &[0xFE, 0, 0]);
        bad[0] = b'x';
        assert_eq!(decode(&bad).unwrap_err(), LifError::BadMagic);
        assert_eq!(
            decode(&lif(0, 1, &[0xFE, 0, 0])).unwrap_err(),
            LifError::BadDimensions
        );
        assert_eq!(
            decode(&lif(2048, 1, &[0xFE, 0, 0])).unwrap_err(),
            LifError::BadDimensions
        );
        assert_eq!(decode(&lif(1, 1, &[])).unwrap_err(), LifError::NoPayload);
    }

    #[test]
    fn la_signature_majuscule_est_acceptee() {
        let mut data = lif(1, 1, &[0xFE, 0, 0]);
        data[0..4].copy_from_slice(b"LIFF");
        assert!(decode(&data).is_ok());
    }

    #[test]
    fn colonnes_majeures_sont_retransposees() {
        // 70 x 80 stocké par colonnes : la couleur ne dépend que de y (= i % h).
        // Lue par lignes l'image est très incohérente ; transposée elle est constante
        // le long de chaque ligne.
        let (w, h) = (70usize, 80usize);
        let mut payload = Vec::new();
        for i in 0..w * h {
            let (lo, hi) = pack(((i % h) * 7 % 32) as u8, 0, 0);
            payload.extend_from_slice(&[0xFF, lo, hi, 255]);
        }
        let img = decode(&lif(w as u32, h as u32, &payload)).unwrap();
        let at = |x: usize, y: usize| img.rgba[(y * w + x) * 4];
        assert_eq!(at(0, 3), at(69, 3));
        assert_ne!(at(0, 0), at(0, 1));
    }

    /// Fichier produit par `tools/telmi2flam/lif.py` (encode) pour une image 8x4 ;
    /// l'image attendue est celle du décodeur Python de référence (`lif.decode`).
    #[test]
    fn fichier_produit_par_lif_py() {
        let data = hex(PYTHON_LIF);
        let img = decode(&data).unwrap();
        assert_eq!((img.width, img.height), (8, 4));
        assert_eq!(img.rgba, hex(PYTHON_RGBA));
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    const PYTHON_LIF: &str = "6c6966660000000800000004a2ff26cbffc1fe1858a06c9248c13ec19468a427ff440e80c13ec1fe5873fe5694b067c13ec1fed87cfe769eff440e80c10000000000000001";
    const PYTHON_RGBA: &str = "c86430ffc86430ffc86430ff5800c0ff7800b0ff08c820ff08c820ff08c820ffc86430ffc86430ffc86430ff6834c0ff8044b0ff08c8208008c8208008c82080c86430ffc86430ffc86430ff7068c0ff9088b0ff08c820ff08c820ff08c820ffc86430ffc86430ffc86430ff7898c0ff98ccb0ff08c8208008c8208008c82080";
}
