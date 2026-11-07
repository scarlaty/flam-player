//! Images d'une histoire STUdio converties au format que l'appareil lit : un BMP 4 bits compressé
//! en RLE4, de 320 x 240 pixels en 16 niveaux de gris. Comportement de `image_to_bitmap_rle4` de
//! Lunii.QT (`convert_image.py`, Pillow), réécrit en Rust pur ; l'entrée est PNG, JPEG ou BMP.
//!
//! # Chaîne de conversion
//!
//! 1. Un BMP déjà exactement au format de l'appareil (voir ci-dessous) est décodé en niveaux puis
//!    réencodé : jamais copié tel quel, un fichier hostile n'atteint donc pas l'appareil.
//! 2. Sinon l'image est décodée (dimensions et taille bornées avant décodage, voir [`MAX_SIDE`],
//!    [`MAX_PIXELS`] et [`MAX_INPUT_BYTES`]) puis, si elle n'a pas déjà 320 x 240, redimensionnée
//!    **sans conserver le rapport d'aspect** (déformation, comme `Image.resize((320, 240))`) :
//!    filtre bicubique de Pillow (a = -0,5, support élargi à la réduction, coefficients en virgule
//!    fixe sur 22 bits, passe horizontale puis verticale, arrondi à 8 bits entre les deux), refait
//!    à l'octet près ; les images à palette ou en noir et blanc 1 bit sont, comme dans Pillow,
//!    redimensionnées au plus proche voisin (`floor((x + 0,5) * entrée / sortie)`). Une image
//!    avec alpha est d'abord composée sur fond noir (couleur multipliée par l'opacité), à toute
//!    taille.
//! 3. Niveau de gris sur 4 bits (0 à 15) :
//!    - source en niveaux de gris : `v / 16` (division entière) ;
//!    - source en couleurs : `int((0.299 R + 0.587 G + 0.114 B) / 16)` en `f64`, dans cet ordre
//!      d'opérations.
//! 4. Lignes écrites **de bas en haut** (BMP à hauteur positive), RLE4 en mode codé uniquement.
//!
//! Écarts choisis avec Lunii.QT (note de conception `docs/studio-lunii.md`, § 3.5) : (a) il copie
//! tel quel un BMP RLE4 de 320 x 240 ; ici décodé puis réencodé. (b) Il écrit à l'envers une
//! source BMP de 320 x 240 non RLE4 (il ne la retourne pas) ; ici toutes les sources sortent à
//! l'endroit. (c) Il ignore l'alpha quand l'image n'est pas redimensionnée ; ici composition sur
//! noir dans tous les cas. Limites : (d) JPEG, le décodeur diffère de libjpeg, d'où des écarts de
//! quelques niveaux de gris possibles (la structure reste identique) ; (e) PNG 16 bits, octet de
//! poids fort.
//!
//! # Fichier produit
//!
//! | Offset | Taille | Valeur |
//! |---|---|---|
//! | 0 | 2 | `BM` |
//! | 2 | 4 | taille du fichier (`118 + n`), u32 LE |
//! | 6 | 4 | 0 |
//! | 10 | 4 | 118 : début des données (54 + palette de 64 octets) |
//! | 14 | 4 | 40 : taille de l'en-tête `BITMAPINFOHEADER` |
//! | 18 | 4 | largeur, 320 |
//! | 22 | 4 | hauteur, 240 (positive : lignes de bas en haut) |
//! | 26 | 2 | plans, 1 |
//! | 28 | 2 | bits par pixel, 4 |
//! | 30 | 4 | compression, 2 (`BI_RLE4`) |
//! | 34 | 4 | taille des données RLE `n` |
//! | 38 | 8 | résolutions horizontale et verticale, 0 |
//! | 46 | 4 | couleurs de la palette, 16 |
//! | 50 | 4 | couleurs importantes, 0 |
//! | 54 | 64 | palette : 16 entrées B, G, R, 0 avec B = G = R = `floor(255 * i / 16)`, soit 0, 15, 31, 47, ..., 239 |
//! | 118 | `n` | données RLE4 |
//!
//! Données RLE4 : pour chaque ligne, des paires `(nombre, valeur)` où `nombre` va de 1 à 255 et
//! `valeur` est l'octet `(v << 4) | v` (les deux demi-octets sont identiques, donc une série de
//! `nombre` pixels de niveau `v`) ; une ligne complète se termine par `00 00` sauf la dernière ;
//! le fichier se termine par `00 01`. Une série ne dépasse jamais la ligne.

use std::io::Cursor;

use image::{DynamicImage, ImageFormat, ImageReader, Limits};

/// Largeur de l'image attendue par l'appareil.
pub const WIDTH: u32 = 320;
/// Hauteur de l'image attendue par l'appareil.
pub const HEIGHT: u32 = 240;
/// Taille maximale d'une image d'entrée, en octets.
pub const MAX_INPUT_BYTES: usize = 32 * 1024 * 1024;
/// Largeur ou hauteur maximale d'une image d'entrée, en pixels.
pub const MAX_SIDE: u32 = 8192;
/// Nombre maximal de pixels d'une image d'entrée (40 Mpx, environ 160 Mo en RGBA décodé).
pub const MAX_PIXELS: u64 = 40_000_000;

const HEADER_SIZE: usize = 54;
const PALETTE_SIZE: usize = 64;
const BI_RLE4: u32 = 2;
const PRECISION_BITS: u32 = 22;

/// Raison pour laquelle une image est refusée.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BitmapError {
    /// Fichier d'entrée plus gros que [`MAX_INPUT_BYTES`].
    #[error("image trop volumineuse : {len} octets (maximum {max})")]
    InputTooLarge {
        /// Taille reçue.
        len: usize,
        /// Taille maximale acceptée.
        max: usize,
    },
    /// Ni PNG, ni JPEG, ni BMP.
    #[error("format d'image non pris en charge (PNG, JPEG et BMP seulement)")]
    UnsupportedFormat,
    /// Dimensions nulles ou au-delà des bornes.
    #[error("dimensions d'image non acceptées : {width} x {height}")]
    BadDimensions {
        /// Largeur annoncée par l'en-tête.
        width: u32,
        /// Hauteur annoncée par l'en-tête.
        height: u32,
    },
    /// L'image est corrompue, tronquée ou d'un sous-format non géré.
    #[error("image illisible : {0}")]
    Decode(String),
}

/// Convertit une image STUdio (PNG, JPEG ou BMP) en BMP 4 bits RLE4 de 320 x 240, tel que
/// l'appareil l'attend. Voir l'en-tête du module pour le format exact.
pub fn to_lunii_bmp(bytes: &[u8]) -> Result<Vec<u8>, BitmapError> {
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(BitmapError::InputTooLarge {
            len: bytes.len(),
            max: MAX_INPUT_BYTES,
        });
    }
    let format = reader(bytes, true)?.format();
    if !matches!(
        format,
        Some(ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Bmp)
    ) {
        return Err(BitmapError::UnsupportedFormat);
    }
    let is_bmp = format == Some(ImageFormat::Bmp);

    // Dimensions lues dans l'en-tête, avant de décoder quoi que ce soit.
    let (w, h) = reader(bytes, false)?
        .into_dimensions()
        .map_err(|e| BitmapError::Decode(e.to_string()))?;
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE || u64::from(w) * u64::from(h) > MAX_PIXELS
    {
        return Err(BitmapError::BadDimensions {
            width: w,
            height: h,
        });
    }

    // Fichier déjà au format de l'appareil : décodé puis réencodé (donc identique), jamais copié.
    if is_bmp {
        if let Some(levels) = decode_device_levels(bytes) {
            return Ok(encode_rle4_bmp(&levels));
        }
    }

    let img = reader(bytes, true)?
        .decode()
        .map_err(|e| BitmapError::Decode(e.to_string()))?;
    let hints = if is_bmp {
        bmp_hints(bytes)
    } else {
        png_hints(bytes)
    };
    let mut raster = Raster::from_image(img);
    if hints.ignore_alpha {
        raster.drop_alpha();
    }
    if hints.gray {
        raster.keep_first_channel();
    }
    raster.composite_on_black();
    if (raster.w, raster.h) != (WIDTH as usize, HEIGHT as usize) {
        raster = raster.resize(WIDTH as usize, HEIGHT as usize, hints.nearest);
    }
    Ok(encode_rle4_bmp(&raster.to_levels()))
}

fn reader(bytes: &[u8], limited: bool) -> Result<ImageReader<Cursor<&[u8]>>, BitmapError> {
    let mut r = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| BitmapError::Decode(e.to_string()))?;
    if limited {
        let mut limits = Limits::default();
        limits.max_image_width = Some(MAX_SIDE);
        limits.max_image_height = Some(MAX_SIDE);
        limits.max_alloc = Some(512 * 1024 * 1024);
        r.limits(limits);
    }
    Ok(r)
}

fn le16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn le32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// Décode un fichier exactement au format de l'appareil (celui que produit
/// [`encode_rle4_bmp`] : mêmes en-têtes, même palette, séries codées seulement) en niveaux 0 à 15,
/// lignes de haut en bas. `None` pour tout autre fichier, qui passe alors par le décodeur général.
/// Les niveaux sont les indices de palette : repasser par les couleurs de la palette ferait
/// perdre un niveau (le gris 15 de l'entrée 1 donnerait 0).
fn decode_device_levels(b: &[u8]) -> Option<Vec<u8>> {
    let (w, h) = (WIDTH as usize, HEIGHT as usize);
    let header_ok = b.starts_with(b"BM")
        && b.len() > HEADER_SIZE + PALETTE_SIZE
        && le32(b, 2) == Some(b.len() as u32)
        && le32(b, 6) == Some(0)
        && le32(b, 10) == Some((HEADER_SIZE + PALETTE_SIZE) as u32)
        && le32(b, 14) == Some(40)
        && le32(b, 18) == Some(WIDTH)
        && le32(b, 22) == Some(HEIGHT)
        && le16(b, 26) == Some(1)
        && le16(b, 28) == Some(4)
        && le32(b, 30) == Some(BI_RLE4)
        && le32(b, 34) == Some((b.len() - HEADER_SIZE - PALETTE_SIZE) as u32)
        && le32(b, 38) == Some(0)
        && le32(b, 42) == Some(0)
        && le32(b, 46) == Some(16)
        && le32(b, 50) == Some(0);
    if !header_ok {
        return None;
    }
    let palette_ok = (0..16usize).all(|i| {
        let g = (255 * i / 16) as u8;
        b[HEADER_SIZE + i * 4..HEADER_SIZE + i * 4 + 4] == [g, g, g, 0]
    });
    if !palette_ok {
        return None;
    }
    let mut rows: Vec<Vec<u8>> = Vec::with_capacity(h);
    let mut row: Vec<u8> = Vec::with_capacity(w);
    let data = &b[HEADER_SIZE + PALETTE_SIZE..];
    let mut at = 0;
    let mut done = false;
    while at + 2 <= data.len() {
        let (n, v) = (usize::from(data[at]), data[at + 1]);
        at += 2;
        if n == 0 {
            if row.len() != w {
                return None;
            }
            rows.push(std::mem::take(&mut row));
            match v {
                0 if rows.len() < h => {}
                1 if rows.len() == h => {
                    done = true;
                    break;
                }
                _ => return None,
            }
        } else {
            if v >> 4 != v & 15 || row.len() + n > w {
                return None;
            }
            row.extend(std::iter::repeat_n(v & 15, n));
        }
    }
    if !done || at != data.len() {
        return None;
    }
    Some(rows.into_iter().rev().flatten().collect())
}

/// Particularités de la source qui changent la conversion de Pillow.
#[derive(Default)]
struct Hints {
    /// Redimensionnement au plus proche voisin (modes `P` et `1` de Pillow).
    nearest: bool,
    /// Le canal alpha éventuel est ignoré (Pillow ne le charge pas : mode sans alpha).
    ignore_alpha: bool,
    /// Palette de gris : Pillow charge l'image en niveaux de gris (mode `L` ou `1`).
    gray: bool,
}

/// PNG : les types de couleur 0 (gris), 2 (RVB) et 3 (palette) n'ont jamais d'alpha chez Pillow
/// (la transparence `tRNS` n'est qu'une information) ; palette et gris 1 bit sont redimensionnés
/// au plus proche voisin.
fn png_hints(b: &[u8]) -> Hints {
    if b.len() < 26 || !b.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Hints::default();
    }
    let (depth, color) = (b[24], b[25]);
    Hints {
        nearest: color == 3 || (color == 0 && depth == 1),
        ignore_alpha: matches!(color, 0 | 2 | 3),
        gray: false,
    }
}

/// BMP : l'alpha est ignoré ; 1, 4 et 8 bits sont des images à palette (plus proche voisin), sauf
/// si la palette est la rampe de gris identité (`i, i, i`, ou noir et blanc pour 2 couleurs), que
/// Pillow charge en mode `L` (ou `1` pour 2 couleurs).
fn bmp_hints(b: &[u8]) -> Hints {
    let mut hints = Hints {
        ignore_alpha: true,
        ..Hints::default()
    };
    let (Some(hs), Some(bpp)) = (le32(b, 14), le16(b, 28)) else {
        return hints;
    };
    if bpp > 8 || hs < 40 {
        return hints;
    }
    let colors = match le32(b, 46) {
        Some(0) | None => 1usize << bpp,
        Some(n) => n as usize,
    };
    let pal = 14 + hs as usize;
    let gray = colors <= 256
        && (0..colors).all(|i| {
            let expect = if colors == 2 && i == 1 { 255 } else { i as u8 };
            b.get(pal + i * 4..pal + i * 4 + 3)
                .is_some_and(|e| e == [expect; 3])
        });
    hints.nearest = !gray || colors == 2;
    hints.gray = gray;
    hints
}

/// Image décodée, canaux entrelacés : 1 gris, 2 gris + alpha, 3 RVB, 4 RVBA.
struct Raster {
    w: usize,
    h: usize,
    ch: usize,
    data: Vec<u8>,
}

impl Raster {
    fn from_image(img: DynamicImage) -> Raster {
        let (w, h) = (img.width() as usize, img.height() as usize);
        let hi = |v: Vec<u16>| v.into_iter().map(|x| (x >> 8) as u8).collect::<Vec<u8>>();
        let (ch, data) = match img {
            DynamicImage::ImageLuma8(b) => (1, b.into_raw()),
            DynamicImage::ImageLumaA8(b) => (2, b.into_raw()),
            DynamicImage::ImageRgb8(b) => (3, b.into_raw()),
            DynamicImage::ImageRgba8(b) => (4, b.into_raw()),
            DynamicImage::ImageLuma16(b) => (1, hi(b.into_raw())),
            DynamicImage::ImageLumaA16(b) => (2, hi(b.into_raw())),
            DynamicImage::ImageRgb16(b) => (3, hi(b.into_raw())),
            DynamicImage::ImageRgba16(b) => (4, hi(b.into_raw())),
            other => (4, other.into_rgba8().into_raw()),
        };
        Raster { w, h, ch, data }
    }

    fn drop_alpha(&mut self) {
        if self.ch == 2 || self.ch == 4 {
            let keep = self.ch - 1;
            self.data = self
                .data
                .chunks_exact(self.ch)
                .flat_map(|p| p[..keep].iter().copied())
                .collect();
            self.ch = keep;
        }
    }

    /// Image dont les canaux sont identiques (palette de gris) : ne garde que le premier.
    fn keep_first_channel(&mut self) {
        self.data = self.data.chunks_exact(self.ch).map(|p| p[0]).collect();
        self.ch = 1;
    }

    fn resize(self, ow: usize, oh: usize, nearest: bool) -> Raster {
        let ch = self.ch;
        if nearest {
            let (sx, sy) = (self.w as f64 / ow as f64, self.h as f64 / oh as f64);
            let mut out = Vec::with_capacity(ow * oh * ch);
            for y in 0..oh {
                let yi = (((y as f64 + 0.5) * sy) as usize).min(self.h - 1);
                for x in 0..ow {
                    let xi = (((x as f64 + 0.5) * sx) as usize).min(self.w - 1);
                    let i = (yi * self.w + xi) * ch;
                    out.extend_from_slice(&self.data[i..i + ch]);
                }
            }
            return Raster {
                w: ow,
                h: oh,
                ch,
                data: out,
            };
        }
        let mut data = self.data;
        let (mut w, mut h) = (self.w, self.h);
        if w != ow {
            data = resample_pass(&data, w, h, ch, ow, true);
            w = ow;
        }
        if h != oh {
            data = resample_pass(&data, w, h, ch, oh, false);
            h = oh;
        }
        Raster { w, h, ch, data }
    }

    /// Composition sur fond noir : chaque couleur est multipliée par l'opacité (arrondi de
    /// `MULDIV255` de Pillow), puis l'alpha disparaît. Le gris + alpha devient RVB, comme
    /// `convert("RGB")` de Pillow (il passe alors par la formule de luminance).
    fn composite_on_black(&mut self) {
        if self.ch != 2 && self.ch != 4 {
            return;
        }
        let ch = self.ch;
        let mul = |c: u8, a: u8| {
            let t = u32::from(c) * u32::from(a) + 128;
            (((t >> 8) + t) >> 8) as u8
        };
        self.data = self
            .data
            .chunks_exact(ch)
            .flat_map(|p| {
                let a = p[ch - 1];
                if ch == 2 {
                    let l = mul(p[0], a);
                    [l, l, l]
                } else {
                    [mul(p[0], a), mul(p[1], a), mul(p[2], a)]
                }
            })
            .collect();
        self.ch = 3;
    }

    /// Niveaux 0 à 15, lignes de haut en bas.
    fn to_levels(&self) -> Vec<u8> {
        self.data
            .chunks_exact(self.ch)
            .map(|p| match self.ch {
                1 => p[0] >> 4,
                _ => luma(p[0], p[1], p[2]),
            })
            .collect()
    }
}

/// Niveau 0 à 15 d'un pixel couleur, mêmes opérations flottantes que Lunii.QT.
fn luma(r: u8, g: u8, b: u8) -> u8 {
    ((f64::from(r) * 0.299 + f64::from(g) * 0.587 + f64::from(b) * 0.114) / 16.0) as u8
}

/// Filtre bicubique de Pillow (a = -0,5).
fn bicubic(x: f64) -> f64 {
    const A: f64 = -0.5;
    let x = x.abs();
    if x < 1.0 {
        ((A + 2.0) * x - (A + 3.0)) * x * x + 1.0
    } else if x < 2.0 {
        (((x - 5.0) * x + 8.0) * x - 4.0) * A
    } else {
        0.0
    }
}

/// Une passe de rééchantillonnage (horizontale si `horizontal`) vers `out_size` pixels, comme
/// `ImagingResampleHorizontal_8bpc` / `Vertical_8bpc` de Pillow.
fn resample_pass(
    src: &[u8],
    w: usize,
    h: usize,
    ch: usize,
    out_size: usize,
    horizontal: bool,
) -> Vec<u8> {
    let in_size = if horizontal { w } else { h };
    let scale = in_size as f64 / out_size as f64;
    let filterscale = scale.max(1.0);
    let support = 2.0 * filterscale;
    // Fenêtre (début, coefficients en virgule fixe) de chaque pixel de sortie.
    let mut windows: Vec<(usize, Vec<i64>)> = Vec::with_capacity(out_size);
    for xx in 0..out_size {
        let center = (xx as f64 + 0.5) * scale;
        let xmin = ((center - support + 0.5) as i64).max(0) as usize;
        let xend = ((center + support + 0.5) as i64).clamp(0, in_size as i64) as usize;
        let xmax = xend.max(xmin) - xmin;
        let ss = 1.0 / filterscale;
        let mut k: Vec<f64> = (0..xmax)
            .map(|x| bicubic((x as f64 + xmin as f64 - center + 0.5) * ss))
            .collect();
        let ww: f64 = k.iter().sum();
        if ww != 0.0 {
            k.iter_mut().for_each(|v| *v /= ww);
        }
        let fixed = k
            .iter()
            .map(|&v| {
                let s = v * f64::from(1u32 << PRECISION_BITS);
                (if v < 0.0 { -0.5 + s } else { 0.5 + s }) as i64
            })
            .collect();
        windows.push((xmin, fixed));
    }
    let (ow, oh) = if horizontal {
        (out_size, h)
    } else {
        (w, out_size)
    };
    let mut out = vec![0u8; ow * oh * ch];
    for y in 0..oh {
        for x in 0..ow {
            let (start, coeffs) = if horizontal { &windows[x] } else { &windows[y] };
            for c in 0..ch {
                let mut acc = 1i64 << (PRECISION_BITS - 1);
                for (i, &k) in coeffs.iter().enumerate() {
                    let (sx, sy) = if horizontal {
                        (start + i, y)
                    } else {
                        (x, start + i)
                    };
                    acc += i64::from(src[(sy * w + sx) * ch + c]) * k;
                }
                out[(y * ow + x) * ch + c] = (acc >> PRECISION_BITS).clamp(0, 255) as u8;
            }
        }
    }
    out
}

/// Encode des niveaux 0 à 15 (320 x 240, lignes de haut en bas) en fichier BMP RLE4.
fn encode_rle4_bmp(levels: &[u8]) -> Vec<u8> {
    let (w, h) = (WIDTH as usize, HEIGHT as usize);
    debug_assert_eq!(levels.len(), w * h);
    let mut rle = Vec::new();
    // Bas de l'image d'abord.
    for (n, y) in (0..h).rev().enumerate() {
        let row = &levels[y * w..(y + 1) * w];
        let mut run = 1u8;
        let mut value = row[0];
        for &v in &row[1..] {
            if v == value && run < 255 {
                run += 1;
            } else {
                rle.extend_from_slice(&[run, (value << 4) | value]);
                run = 1;
                value = v;
            }
        }
        rle.extend_from_slice(&[run, (value << 4) | value]);
        if n < h - 1 {
            rle.extend_from_slice(&[0, 0]);
        }
    }
    rle.extend_from_slice(&[0, 1]);

    let data_offset = HEADER_SIZE + PALETTE_SIZE;
    let file_size = data_offset + rle.len();
    let mut out = Vec::with_capacity(file_size);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(file_size as u32).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(data_offset as u32).to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&WIDTH.to_le_bytes());
    out.extend_from_slice(&HEIGHT.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&BI_RLE4.to_le_bytes());
    out.extend_from_slice(&(rle.len() as u32).to_le_bytes());
    out.extend_from_slice(&[0; 8]);
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for i in 0..16u32 {
        let g = (255 * i / 16) as u8;
        out.extend_from_slice(&[g, g, g, 0]);
    }
    out.extend_from_slice(&rle);
    out
}
