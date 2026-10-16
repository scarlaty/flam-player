//! Vignettes : décodage PNG, réduction, encodage PNG et cache disque.
//!
//! Le front reçoit des URL `data:image/png;base64,…` (autorisées par la CSP `img-src data:`) :
//! pas de protocole d'asset à ouvrir sur le disque. Les PNG réduits sont gardés dans
//! `<cache>/thumbs/` pour éviter de rouvrir les archives à chaque analyse.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use base64::Engine;

/// Dimension maximale acceptée pour un PNG source (archive non fiable).
const MAX_SOURCE_DIM: u32 = 8192;
/// Mémoire maximale allouée au décodage d'un PNG.
const MAX_DECODE_BYTES: usize = 64 * 1024 * 1024;

/// Image RGBA8888 par lignes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Échec de traitement d'une vignette.
#[derive(Debug, thiserror::Error)]
pub enum ThumbError {
    #[error("image PNG invalide : {0}")]
    Decode(String),
    #[error("dimensions d'image non prises en charge")]
    Dimensions,
    #[error("encodage PNG impossible : {0}")]
    Encode(String),
    #[error("image LIF invalide : {0}")]
    Lif(#[from] crate::lif::LifError),
}

/// Décode un PNG (tout type de couleur et de profondeur) en RGBA8888.
pub fn decode_png(bytes: &[u8]) -> Result<RgbaImage, ThumbError> {
    let mut decoder = png::Decoder::new_with_limits(
        Cursor::new(bytes),
        png::Limits {
            bytes: MAX_DECODE_BYTES,
        },
    );
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder
        .read_info()
        .map_err(|e| ThumbError::Decode(e.to_string()))?;
    let (w, h) = (reader.info().width, reader.info().height);
    if w == 0 || h == 0 || w > MAX_SOURCE_DIM || h > MAX_SOURCE_DIM {
        return Err(ThumbError::Dimensions);
    }
    let size = reader
        .output_buffer_size()
        .filter(|&n| n <= MAX_DECODE_BYTES)
        .ok_or(ThumbError::Dimensions)?;
    let mut buf = vec![0u8; size];
    let frame = reader
        .next_frame(&mut buf)
        .map_err(|e| ThumbError::Decode(e.to_string()))?;
    let data = &buf[..frame.buffer_size()];
    let rgba = match frame.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::GrayscaleAlpha => data
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Indexed => return Err(ThumbError::Decode("palette non étendue".into())),
    };
    if rgba.len() != (w as usize) * (h as usize) * 4 {
        return Err(ThumbError::Decode("taille de tampon incohérente".into()));
    }
    Ok(RgbaImage {
        width: w,
        height: h,
        rgba,
    })
}

/// Réduit l'image pour qu'elle tienne dans `max` × `max` (moyenne par blocs, alpha pondéré).
/// Une image déjà assez petite est renvoyée telle quelle.
pub fn fit(img: RgbaImage, max: u32) -> RgbaImage {
    if img.width <= max && img.height <= max {
        return img;
    }
    let scale = f64::from(max) / f64::from(img.width.max(img.height));
    let nw = ((f64::from(img.width) * scale).round() as u32).max(1);
    let nh = ((f64::from(img.height) * scale).round() as u32).max(1);
    let mut out = vec![0u8; (nw as usize) * (nh as usize) * 4];
    for dy in 0..nh {
        let y0 = (u64::from(dy) * u64::from(img.height) / u64::from(nh)) as usize;
        let y1 = (((u64::from(dy) + 1) * u64::from(img.height)).div_ceil(u64::from(nh)) as usize)
            .clamp(y0 + 1, img.height as usize);
        for dx in 0..nw {
            let x0 = (u64::from(dx) * u64::from(img.width) / u64::from(nw)) as usize;
            let x1 = (((u64::from(dx) + 1) * u64::from(img.width)).div_ceil(u64::from(nw))
                as usize)
                .clamp(x0 + 1, img.width as usize);
            let (mut r, mut g, mut b, mut a, mut n) = (0u64, 0u64, 0u64, 0u64, 0u64);
            for y in y0..y1 {
                for x in x0..x1 {
                    let o = (y * img.width as usize + x) * 4;
                    let alpha = u64::from(img.rgba[o + 3]);
                    r += u64::from(img.rgba[o]) * alpha;
                    g += u64::from(img.rgba[o + 1]) * alpha;
                    b += u64::from(img.rgba[o + 2]) * alpha;
                    a += alpha;
                    n += 1;
                }
            }
            let o = (dy as usize * nw as usize + dx as usize) * 4;
            if let (Some(r), Some(g), Some(b)) =
                (r.checked_div(a), g.checked_div(a), b.checked_div(a))
            {
                out[o] = r as u8;
                out[o + 1] = g as u8;
                out[o + 2] = b as u8;
            }
            out[o + 3] = (a / n) as u8;
        }
    }
    RgbaImage {
        width: nw,
        height: nh,
        rgba: out,
    }
}

/// Encode une image RGBA en PNG.
pub fn encode_png(img: &RgbaImage) -> Result<Vec<u8>, ThumbError> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, img.width, img.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|e| ThumbError::Encode(e.to_string()))?;
    writer
        .write_image_data(&img.rgba)
        .map_err(|e| ThumbError::Encode(e.to_string()))?;
    writer
        .finish()
        .map_err(|e| ThumbError::Encode(e.to_string()))?;
    Ok(out)
}

/// URL `data:` d'un PNG.
pub fn data_uri(png: &[u8]) -> String {
    format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png)
    )
}

/// Hachage FNV-1a 64 bits (stable d'une version de Rust à l'autre, contrairement à
/// `DefaultHasher`) : sert à nommer les fichiers du cache.
pub fn fnv1a(parts: &[&[u8]]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        for &byte in *part {
            h ^= u64::from(byte);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h ^= 0xff;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Cache disque des vignettes réduites (`<cache>/thumbs/<clé>.png`).
pub struct ThumbCache {
    dir: PathBuf,
}

impl ThumbCache {
    /// `cache_dir` est le dossier de cache de l'application ; `thumbs/` est créé à la demande.
    pub fn new(cache_dir: &Path) -> Self {
        Self {
            dir: cache_dir.join("thumbs"),
        }
    }

    fn path(&self, key: u64) -> PathBuf {
        self.dir.join(format!("{key:016x}.png"))
    }

    /// Renvoie l'URL `data:` de la vignette `key`, en la produisant (`make` renvoie l'image
    /// source) si le cache ne la contient pas. Un cache illisible ou inscriptible n'est pas
    /// une erreur : la vignette est simplement recalculée.
    pub fn get_or_make<E>(
        &self,
        key: u64,
        max_dim: u32,
        make: impl FnOnce() -> Result<RgbaImage, E>,
    ) -> Result<String, E>
    where
        E: From<ThumbError>,
    {
        let path = self.path(key);
        if let Ok(bytes) = std::fs::read(&path) {
            if decode_png(&bytes).is_ok() {
                return Ok(data_uri(&bytes));
            }
        }
        let png = encode_png(&fit(make()?, max_dim))?;
        if let Err(e) = self.store(&path, &png) {
            tracing::warn!("cache de vignettes inscriptible impossible : {e}");
        }
        Ok(data_uri(&png))
    }

    fn store(&self, path: &Path, png: &[u8]) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
        std::fs::write(&tmp, png)?;
        if let Err(e) = std::fs::rename(&tmp, path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, px: [u8; 4]) -> RgbaImage {
        RgbaImage {
            width: w,
            height: h,
            rgba: px.repeat((w * h) as usize),
        }
    }

    #[test]
    fn aller_retour_png() {
        let img = solid(3, 2, [10, 20, 30, 255]);
        let png = encode_png(&img).unwrap();
        assert_eq!(decode_png(&png).unwrap(), img);
    }

    #[test]
    fn decode_les_png_gris_et_palette() {
        // Gris 8 bits 2x1.
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, 2, 1);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header()
            .unwrap()
            .write_image_data(&[0, 255])
            .unwrap();
        let img = decode_png(&out).unwrap();
        assert_eq!(img.rgba, vec![0, 0, 0, 255, 255, 255, 255, 255]);

        // Palette 1 couleur avec transparence.
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, 1, 1);
        enc.set_color(png::ColorType::Indexed);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_palette(vec![1, 2, 3]);
        enc.set_trns(vec![128]);
        enc.write_header().unwrap().write_image_data(&[0]).unwrap();
        assert_eq!(decode_png(&out).unwrap().rgba, vec![1, 2, 3, 128]);
    }

    #[test]
    fn png_invalide_refuse() {
        assert!(matches!(
            decode_png(b"pas un png"),
            Err(ThumbError::Decode(_))
        ));
    }

    #[test]
    fn png_trop_grand_refuse() {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, MAX_SOURCE_DIM + 1, 1);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_depth(png::BitDepth::One);
        enc.write_header()
            .unwrap()
            .write_image_data(&vec![0u8; (MAX_SOURCE_DIM as usize + 1).div_ceil(8)])
            .unwrap();
        assert!(matches!(decode_png(&out), Err(ThumbError::Dimensions)));
    }

    #[test]
    fn fit_garde_les_petites_images_et_reduit_les_grandes() {
        let small = solid(10, 5, [1, 2, 3, 255]);
        assert_eq!(fit(small.clone(), 192), small);
        let big = fit(solid(640, 480, [9, 8, 7, 255]), 192);
        assert_eq!((big.width, big.height), (192, 144));
        assert_eq!(&big.rgba[0..4], &[9, 8, 7, 255]);
        let thin = fit(solid(1000, 1, [9, 8, 7, 255]), 192);
        assert_eq!((thin.width, thin.height), (192, 1));
    }

    #[test]
    fn fit_ignore_la_couleur_des_pixels_transparents() {
        let mut img = solid(2, 1, [255, 0, 0, 0]);
        img.rgba[4..8].copy_from_slice(&[0, 0, 255, 255]);
        let out = fit(img, 1);
        assert_eq!(out.rgba, vec![0, 0, 255, 127]);
    }

    #[test]
    fn data_uri_et_hachage_stable() {
        assert_eq!(data_uri(b"abc"), "data:image/png;base64,YWJj");
        assert_eq!(fnv1a(&[b"a"]), fnv1a(&[b"a"]));
        assert_ne!(fnv1a(&[b"a", b"b"]), fnv1a(&[b"ab"]));
    }

    #[test]
    fn le_cache_est_ecrit_puis_relu() {
        let dir = tempfile::tempdir().unwrap();
        let cache = ThumbCache::new(dir.path());
        let first = cache
            .get_or_make::<ThumbError>(7, 192, || Ok(solid(4, 4, [1, 2, 3, 255])))
            .unwrap();
        assert!(first.starts_with("data:image/png;base64,"));
        assert!(dir
            .path()
            .join("thumbs")
            .join("0000000000000007.png")
            .exists());
        // Deuxième appel : la fabrique ne doit pas être utilisée.
        let second = cache
            .get_or_make::<ThumbError>(7, 192, || panic!("cache ignoré"))
            .unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn un_fichier_de_cache_corrompu_est_recalcule() {
        let dir = tempfile::tempdir().unwrap();
        let cache = ThumbCache::new(dir.path());
        std::fs::create_dir_all(dir.path().join("thumbs")).unwrap();
        std::fs::write(dir.path().join("thumbs/0000000000000001.png"), b"abc").unwrap();
        let uri = cache
            .get_or_make::<ThumbError>(1, 192, || Ok(solid(1, 1, [0, 0, 0, 255])))
            .unwrap();
        assert!(uri.starts_with("data:image/png"));
    }
}
