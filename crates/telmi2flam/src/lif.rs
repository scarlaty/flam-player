//! Codec LIF (Lunii Image File Format), port de `tools/telmi2flam/lif.py`.
//!
//! Format : magic `liff`, largeur et hauteur (u32 big-endian), un octet de canaux (`0xA2`),
//! une charge utile d'opcodes (RGB565 + alpha, deltas, cache de 64 couleurs, RLE), puis 8
//! octets de fin (`00 … 00 01`). Le décodeur de référence est `src/formats/lif_decoder.c`.
//!
//! - [`encode`] et [`quantize_rgba`] reproduisent `lif.encode` et `lif.quantize_rgba` à l'octet
//!   près (vecteurs produits par Python dans `tests/vectors/`).
//! - [`decode`] suit le décodeur C (et non `lif.decode`, simple miroir de test côté Python) :
//!   lectures bornées, dimensions limitées à [`MAX_DIM`], transposition des images hautes
//!   stockées en colonnes. Il lit des fichiers d'archives non fiables (vignettes du launcher).

use std::fmt;

const MAGIC: &[u8; 4] = b"liff";
const CHANNEL_RGBA: u8 = 0xA2;
const END_MARKER: [u8; 8] = [0, 0, 0, 0, 0, 0, 0, 1];
const HEADER_SIZE: usize = 13;
const END_SIZE: usize = END_MARKER.len();
/// Dimension maximale acceptée par le décodeur (comme `LIF_MAX_DIM` du décodeur C).
pub const MAX_DIM: u32 = 2047;

/// Image RGBA8888 par lignes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Raison pour laquelle un fichier LIF est refusé, ou un tampon RGBA rejeté.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifError {
    /// Fichier plus court que l'en-tête et le marqueur de fin.
    TooShort,
    /// Signature autre que `liff` / `LIFF`.
    BadMagic,
    /// Largeur ou hauteur nulle, ou au-delà de [`MAX_DIM`].
    BadDimensions,
    /// Aucun octet entre l'en-tête et le marqueur de fin.
    NoPayload,
    /// Tampon RGBA dont la taille ne correspond pas aux dimensions (`assert` de `lif.py`).
    BadBufferSize,
}

impl fmt::Display for LifError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TooShort => "fichier LIF trop court",
            Self::BadMagic => "signature LIF invalide",
            Self::BadDimensions => "dimensions LIF non prises en charge",
            Self::NoPayload => "fichier LIF sans données d'image",
            Self::BadBufferSize => "taille buffer RGBA incoherente",
        })
    }
}

impl std::error::Error for LifError {}

/// Couleur interne : RGB565 (r 0..=31, g 0..=63, b 0..=31) et alpha 8 bits.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Color {
    r: i32,
    g: i32,
    b: i32,
    a: u8,
}

fn hash(r: i32, g: i32, b: i32) -> usize {
    ((r * 7 + g * 5 + b * 3) & 0x3F) as usize
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

/// 8 bits vers RGB565, aller-retour exact avec `expand5` / `expand6` (`lif._quantize`).
fn quantize(px: &[u8; 4]) -> Color {
    Color {
        r: ((i32::from(px[0]) + 4) / 8).min(31),
        g: ((i32::from(px[1]) + 2) / 4).min(63),
        b: ((i32::from(px[2]) + 4) / 8).min(31),
        a: px[3],
    }
}

/// Octets `(lo, hi)` d'une couleur RGB565 (`lif._pack565`).
fn pack565(c: Color) -> [u8; 2] {
    let lo = (c.b & 0x1F) | ((c.g & 0x07) << 5);
    let hi = ((c.g >> 3) & 0x07) | ((c.r & 0x1F) << 3);
    [lo as u8, hi as u8]
}

/// Nombre de pixels de `w × h`, si le tampon RGBA de `len` octets en contient au moins autant
/// (`exact` : exactement autant).
fn pixel_count(len: usize, w: u32, h: u32, exact: bool) -> Result<usize, LifError> {
    let n = usize::try_from(u64::from(w) * u64::from(h)).map_err(|_| LifError::BadBufferSize)?;
    let bytes = n.checked_mul(4).ok_or(LifError::BadBufferSize)?;
    if bytes == len || (!exact && bytes < len) {
        Ok(n)
    } else {
        Err(LifError::BadBufferSize)
    }
}

/// Encode une image RGBA8888 par lignes (`rgba.len() == w * h * 4`) en fichier LIF.
///
/// Sortie identique à `lif.encode` de Python.
pub fn encode(rgba: &[u8], w: u32, h: u32) -> Result<Vec<u8>, LifError> {
    pixel_count(rgba.len(), w, h, true)?;
    let px: Vec<Color> = rgba.as_chunks::<4>().0.iter().map(quantize).collect();
    let n = px.len();

    let mut out = Vec::with_capacity(HEADER_SIZE + END_SIZE + n);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&w.to_be_bytes());
    out.extend_from_slice(&h.to_be_bytes());
    out.push(CHANNEL_RGBA);

    // État initial du décodeur.
    let mut cur = Color {
        a: 255,
        ..Color::default()
    };
    let mut cache = [Color::default(); 64];
    let mut i = 0;

    // Premier pixel toujours en couleur + alpha explicite (0xFF) : ne suppose pas l'état initial
    // du décodeur (certains initialisent l'alpha à 0).
    if let Some(&first) = px.first() {
        out.push(0xFF);
        out.extend_from_slice(&pack565(first));
        out.push(first.a);
        cur = first;
        cache[hash(cur.r, cur.g, cur.b)] = cur;
        i = 1;
    }

    while i < n {
        let t = px[i];

        // RLE : pixels consécutifs identiques à la couleur courante. `run - 1` ne doit valoir ni
        // 62 (0xFE) ni 63 (0xFF), lus comme opcodes couleur : 62 pixels au plus.
        if t == cur {
            let mut run = 1;
            while i + run < n && px[i + run] == t && run < 62 {
                run += 1;
            }
            out.push(0xC0 | (run - 1) as u8);
            cache[hash(cur.r, cur.g, cur.b)] = cur;
            i += run;
            continue;
        }

        let idx = hash(t.r, t.g, t.b);
        if cache[idx] == t {
            out.push(idx as u8);
        } else if t.a == cur.a {
            let small = |c: i32, t: i32, m: fn(i32) -> i32| (-2..=1).find(|&d| m(c + d) == t);
            if let (Some(dr), Some(dg), Some(db)) = (
                small(cur.r, t.r, mod32),
                small(cur.g, t.g, mod64),
                small(cur.b, t.b, mod32),
            ) {
                // Petit delta (2 bits par canal, -2..=+1).
                out.push(0x40 | (((db + 2) << 4) | ((dg + 2) << 2) | (dr + 2)) as u8);
            } else {
                // Delta vert étendu : `vg` existe toujours (64 valeurs couvrent les 64 résidus).
                let vg = (-32..32)
                    .find(|&v| mod64(cur.g + v) == t.g)
                    .unwrap_or_default();
                let lo = (t.r - cur.r - vg + 8).rem_euclid(32);
                let hi = (t.b - cur.b - vg + 8).rem_euclid(32);
                if lo <= 15 && hi <= 15 {
                    out.push(0x80 | ((vg + 32) & 0x3F) as u8);
                    out.push(((hi << 4) | lo) as u8);
                } else {
                    // Couleur pleine sans alpha.
                    out.push(0xFE);
                    out.extend_from_slice(&pack565(t));
                }
            }
        } else {
            // Couleur pleine + alpha.
            out.push(0xFF);
            out.extend_from_slice(&pack565(t));
            out.push(t.a);
        }

        // Mise à jour de l'état et du cache, comme le décodeur à chaque pixel écrit.
        cur = t;
        cache[hash(cur.r, cur.g, cur.b)] = cur;
        i += 1;
    }

    out.extend_from_slice(&END_MARKER);
    Ok(out)
}

/// Tampon RGBA quantifié en RGB565 puis réétendu en 8 bits : l'image que le décodeur restitue
/// (`lif.quantize_rgba`). Comme en Python, les octets au-delà de `w * h * 4` sont ignorés.
pub fn quantize_rgba(rgba: &[u8], w: u32, h: u32) -> Result<Vec<u8>, LifError> {
    let n = pixel_count(rgba.len(), w, h, false)?;
    let mut out = Vec::with_capacity(n * 4);
    for p in rgba.as_chunks::<4>().0.iter().take(n) {
        let c = quantize(p);
        out.extend_from_slice(&[expand5(c.r), expand6(c.g), expand5(c.b), c.a]);
    }
    Ok(out)
}

/// Somme des écarts RGB entre pixels voisins d'une ligne (détection ligne/colonne).
fn score_coherence(rgba: &[u8], w: usize, h: usize) -> u64 {
    let mut score = 0u64;
    for y in 0..h {
        let row = &rgba[y * w * 4..(y + 1) * w * 4];
        for x in 1..w {
            let (i, p) = (x * 4, (x - 1) * 4);
            for c in 0..3 {
                score += u64::from((i32::from(row[i + c]) - i32::from(row[p + c])).unsigned_abs());
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

/// Décode un fichier LIF complet en image RGBA8888 (sémantique du décodeur C).
///
/// Une charge utile tronquée laisse les pixels restants transparents (zéro) ; un opcode coupé
/// arrête le décodage. Aucune lecture hors du tampon, allocation bornée par [`MAX_DIM`].
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

        cache[hash(cur.r, cur.g, cur.b)] = cur;
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

    #[test]
    fn opcode_tronque_et_dimensions_extremes() {
        // Opcode 0xFF coupé après un octet : pas de panique, pixels laissés à zéro.
        let img = decode(&lif(2, 1, &[0xFF, 0x1F])).unwrap();
        assert_eq!(img.rgba, vec![0; 8]);
        // Vert étendu sans second octet.
        assert!(decode(&lif(2, 1, &[0x80])).is_ok());
        // Dimensions maximales : alloue sans lire hors limites.
        let big = decode(&lif(2047, 2047, &[0xFE, 0, 0])).unwrap();
        assert_eq!(big.rgba.len(), 2047 * 2047 * 4);
        // Dimensions démesurées dans l'en-tête.
        assert_eq!(
            decode(&lif(u32::MAX, u32::MAX, &[0xFE, 0, 0])).unwrap_err(),
            LifError::BadDimensions
        );
    }

    #[test]
    fn encode_refuse_un_tampon_de_mauvaise_taille() {
        assert_eq!(encode(&[0; 15], 2, 2), Err(LifError::BadBufferSize));
        assert_eq!(encode(&[0; 17], 2, 2), Err(LifError::BadBufferSize));
        assert_eq!(
            encode(&[0; 4], u32::MAX, u32::MAX),
            Err(LifError::BadBufferSize)
        );
        assert_eq!(quantize_rgba(&[0; 15], 2, 2), Err(LifError::BadBufferSize));
        // Comme Python, quantize_rgba ignore les octets en trop.
        assert_eq!(quantize_rgba(&[0; 17], 2, 2).unwrap().len(), 16);
    }

    #[test]
    fn image_vide_donne_en_tete_et_fin_seulement() {
        let out = encode(&[], 0, 0).unwrap();
        assert_eq!(out.len(), HEADER_SIZE + END_SIZE);
        assert_eq!(&out[..4], b"liff");
        assert_eq!(&out[HEADER_SIZE..], &END_MARKER);
    }
}
