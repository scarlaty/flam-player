//! Pochettes de podcast : décodage JPEG (dont progressif), PNG et WebP, cadrage centré,
//! redimensionnement et écriture PNG.
//!
//! Logique pure (octets en entrée, octets en sortie) : ni réseau, ni Tauri, ni disque. Les
//! octets viennent d'Internet et ne sont pas fiables, d'où trois garde-fous appliqués dans
//! cet ordre :
//!
//! 1. taille des octets reçus ([`MAX_INPUT_BYTES`]) ;
//! 2. dimensions lues dans l'en-tête, **avant** tout décodage ([`MAX_DECODED_DIM`]) ;
//! 3. mémoire de l'image décodée bornée ([`MAX_DECODE_ALLOC`]), vérifiée sur la taille que
//!    le décodeur annonce avant d'allouer (`image::Limits` est aussi posée, mais `image`
//!    ne l'applique pas dans `from_decoder`).
//!
//! Le format est reconnu à son contenu (jamais à l'extension ni au `Content-Type`).
//!
//! # Isolation du décodage (#263)
//!
//! Le profil release a `panic = "abort"` : un panic d'un décodeur sur une pochette hostile
//! arrêterait le launcher. [`convert_isolated`] décode donc dans un processus enfant (le
//! launcher relancé avec [`DECODE_ARG`], voir `main.rs`) ; si l'enfant meurt ou dépasse son
//! délai, l'appelant reçoit [`ArtworkError::Isolated`] et peut prendre [`Artwork::fallback`].
//! Aucun appelant de production n'existe encore : le pipeline d'installation (#217) **doit** passer
//! par cette fonction et non par [`Artwork::convert`], qui décode dans le processus courant
//! (réservé aux tests et à l'enfant).
//!
//! # Choix de la crate
//!
//! `image` (MIT ou Apache-2.0), limitée aux fonctionnalités `jpeg`, `png` et `webp` : elle tire
//! `zune-jpeg` (JPEG de base et progressif), `png` (déjà utilisée par le launcher) et
//! `image-webp` (WebP avec et sans perte), toutes en Rust pur, sans code C à compiler ni DLL.
//! Elle fournit en plus le redimensionnement Lanczos, l'orientation EXIF et `Limits`. Trois
//! crates dédiées auraient obligé à réécrire ces trois derniers points.

mod isolated;

pub use isolated::{convert_isolated, convert_isolated_with, run_child, CHILD_TIMEOUT, DECODE_ARG};

use std::io::Cursor;

use image::imageops::{self, FilterType};
use image::metadata::Orientation;
use image::{ImageDecoder, ImageFormat, ImageReader, Limits, RgbImage};

/// Taille maximale des octets d'une pochette téléchargée (20 Mo).
pub const MAX_INPUT_BYTES: usize = 20 * 1024 * 1024;
/// Largeur et hauteur maximales d'une image à décoder, lues dans l'en-tête.
pub const MAX_DECODED_DIM: u32 = 8_000;
/// Taille maximale de l'image décodée (256 Mio : une image 8 000 × 8 000 en
/// RGBA 8 bits occupe 244 Mio).
pub const MAX_DECODE_ALLOC: u64 = 256 * 1024 * 1024;

/// Nombre d'octets de fin de fichier où chercher le marqueur de fin d'un JPEG.
const JPEG_EOI_WINDOW: usize = 1024;

/// Fond sous les pixels transparents : blanc (les pochettes à fond transparent sont des
/// logos pensés pour un fond clair).
pub const FLATTEN_BACKGROUND: [u8; 3] = [0xff, 0xff, 0xff];
/// Couleur de l'image de repli : `--color-surface` du thème sombre du design system
/// (`src/theme/tokens.css`), vert très foncé de l'écran de la Flam, neutre et sans rapport
/// avec le contenu. Le choix définitif relève de #208 : [`Artwork::fallback`] prend la
/// couleur en paramètre.
pub const FALLBACK_COLOR: [u8; 3] = [0x12, 0x2e, 0x2c];

/// Usage d'une pochette, qui fixe la taille de la sortie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtworkKind {
    /// Image d'épisode : 640 × 480.
    Episode,
    /// Couverture du podcast : 512 × 512.
    Cover,
}

impl ArtworkKind {
    /// Largeur et hauteur de la sortie, en pixels.
    pub fn size(self) -> (u32, u32) {
        match self {
            Self::Episode => (640, 480),
            Self::Cover => (512, 512),
        }
    }
}

/// Échec de conversion d'une pochette.
#[derive(Debug, thiserror::Error)]
pub enum ArtworkError {
    #[error("image vide")]
    Empty,
    #[error("image trop volumineuse : {0} octets")]
    TooLarge(usize),
    #[error("format d'image non pris en charge (JPEG, PNG et WebP seulement)")]
    UnsupportedFormat,
    #[error("dimensions de l'image refusées : {0} × {1}")]
    Dimensions(u32, u32),
    #[error("image trop gourmande en mémoire au décodage : {0} octets")]
    Memory(u64),
    #[error("image illisible : {0}")]
    Decode(String),
    #[error("décodage isolé impossible : {0}")]
    Isolated(String),
    #[error("encodage PNG impossible : {0}")]
    Encode(String),
}

/// Pochette convertie : un PNG aux dimensions exactes de son usage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artwork {
    pub width: u32,
    pub height: u32,
    /// Fichier PNG complet.
    pub png: Vec<u8>,
}

impl Artwork {
    /// Convertit des octets JPEG, PNG ou WebP, avec [`FLATTEN_BACKGROUND`] sous la
    /// transparence. Voir [`Artwork::convert_on`].
    pub fn convert(bytes: &[u8], kind: ArtworkKind) -> Result<Self, ArtworkError> {
        Self::convert_on(bytes, kind, FLATTEN_BACKGROUND)
    }

    /// Décode `bytes`, aplatit la transparence sur `background`, cadre au centre (l'image
    /// remplit toute la sortie, l'excédent est rogné), redimensionne (Lanczos) à la taille de
    /// `kind` puis applique l'orientation EXIF.
    pub fn convert_on(
        bytes: &[u8],
        kind: ArtworkKind,
        background: [u8; 3],
    ) -> Result<Self, ArtworkError> {
        if bytes.is_empty() {
            return Err(ArtworkError::Empty);
        }
        if bytes.len() > MAX_INPUT_BYTES {
            return Err(ArtworkError::TooLarge(bytes.len()));
        }
        let format = image::guess_format(bytes).map_err(|_| ArtworkError::UnsupportedFormat)?;
        if !matches!(
            format,
            ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::WebP
        ) {
            return Err(ArtworkError::UnsupportedFormat);
        }

        // Le décodeur JPEG pardonne un fichier coupé (il complète en gris) : un JPEG dont les
        // derniers octets ne contiennent pas le marqueur de fin d'image (FF D9, jamais présent
        // tel quel dans les données compressées) est un téléchargement tronqué. La recherche
        // se limite à la fin du fichier : une miniature EXIF en contient aussi un, au début.
        let tail = &bytes[bytes.len().saturating_sub(JPEG_EOI_WINDOW)..];
        if format == ImageFormat::Jpeg && !tail.windows(2).any(|w| w == [0xff, 0xd9]) {
            return Err(ArtworkError::Decode("JPEG tronqué".into()));
        }

        let mut decoder = ImageReader::with_format(Cursor::new(bytes), format)
            .into_decoder()
            .map_err(|e| ArtworkError::Decode(e.to_string()))?;
        // Dimensions de l'en-tête : refus avant toute allocation de pixels.
        let (w, h) = decoder.dimensions();
        if w == 0 || h == 0 || w > MAX_DECODED_DIM || h > MAX_DECODED_DIM {
            return Err(ArtworkError::Dimensions(w, h));
        }
        // `from_decoder` n'applique pas `max_alloc` : le budget mémoire est vérifié ici, sur la
        // taille décodée (dimensions × canaux × profondeur) annoncée par le décodeur.
        let needed = decoder.total_bytes();
        if needed > MAX_DECODE_ALLOC {
            return Err(ArtworkError::Memory(needed));
        }
        let mut limits = Limits::default();
        limits.max_image_width = Some(MAX_DECODED_DIM);
        limits.max_image_height = Some(MAX_DECODED_DIM);
        limits.max_alloc = Some(MAX_DECODE_ALLOC);
        decoder
            .set_limits(limits)
            .map_err(|e| ArtworkError::Decode(e.to_string()))?;
        let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
        let decoded = image::DynamicImage::from_decoder(decoder)
            .map_err(|e| ArtworkError::Decode(e.to_string()))?;

        let rgb = flatten(decoded, background)?;
        let (tw, th) = kind.size();
        // L'orientation EXIF est appliquée à la petite image finale : le cadrage se fait donc
        // dans le repère non pivoté, avec une cible de proportions inversées si besoin.
        let (cw, ch) = if swaps_axes(orientation) {
            (th, tw)
        } else {
            (tw, th)
        };
        let mut out = cover(&rgb, cw, ch);
        drop(rgb);
        if orientation != Orientation::NoTransforms {
            let mut d = image::DynamicImage::ImageRgb8(out);
            d.apply_orientation(orientation);
            out = d.into_rgb8();
        }
        let png = encode_png(&out)?;
        Ok(Self {
            width: out.width(),
            height: out.height(),
            png,
        })
    }

    /// Image de repli : une couleur unie aux dimensions de `kind`.
    pub fn fallback(kind: ArtworkKind, color: [u8; 3]) -> Result<Self, ArtworkError> {
        let (w, h) = kind.size();
        let img = RgbImage::from_pixel(w, h, image::Rgb(color));
        Ok(Self {
            width: w,
            height: h,
            png: encode_png(&img)?,
        })
    }
}

/// Vrai si l'orientation échange la largeur et la hauteur (rotations de 90° et 270°).
fn swaps_axes(o: Orientation) -> bool {
    matches!(
        o,
        Orientation::Rotate90
            | Orientation::Rotate270
            | Orientation::Rotate90FlipH
            | Orientation::Rotate270FlipH
    )
}

/// Aplatit la transparence sur `bg` et renvoie une image RGB8. Le tampon RGBA est compacté
/// sur place : pas de seconde allocation de la taille de l'image décodée.
fn flatten(img: image::DynamicImage, bg: [u8; 3]) -> Result<RgbImage, ArtworkError> {
    if !img.color().has_alpha() {
        return Ok(img.into_rgb8());
    }
    let rgba = img.into_rgba8();
    let (w, h) = rgba.dimensions();
    let mut buf = rgba.into_raw();
    let pixels = buf.len() / 4;
    for i in 0..pixels {
        let a = u32::from(buf[i * 4 + 3]);
        for c in 0..3 {
            // Mélange arrondi : (v·a + fond·(255 − a)) / 255. L'écriture en i·3 + c ne
            // dépasse jamais i·4 + c : aucun octet pas encore lu n'est écrasé (les trois
            // canaux du pixel i sont lus avant d'être écrits, canal par canal en ordre).
            let v = (u32::from(buf[i * 4 + c]) * a + u32::from(bg[c]) * (255 - a) + 127) / 255;
            buf[i * 3 + c] = v as u8;
        }
    }
    buf.truncate(pixels * 3);
    RgbImage::from_raw(w, h, buf)
        .ok_or_else(|| ArtworkError::Decode("tampon de pixels incohérent".into()))
}

/// Cadrage « cover » centré : rogne l'image aux proportions de la cible puis la redimensionne.
fn cover(src: &RgbImage, tw: u32, th: u32) -> RgbImage {
    let (sw, sh) = src.dimensions();
    // Comparaison par produits croisés (entiers) : sw/sh > tw/th.
    let (cw, ch) = if u64::from(sw) * u64::from(th) > u64::from(sh) * u64::from(tw) {
        let w = (u64::from(sh) * u64::from(tw)).div_ceil(u64::from(th)) as u32;
        (w.clamp(1, sw), sh)
    } else {
        let h = (u64::from(sw) * u64::from(th)).div_ceil(u64::from(tw)) as u32;
        (sw, h.clamp(1, sh))
    };
    let (x, y) = ((sw - cw) / 2, (sh - ch) / 2);
    let view = imageops::crop_imm(src, x, y, cw, ch);
    imageops::resize(&*view, tw, th, FilterType::Lanczos3)
}

fn encode_png(img: &RgbImage) -> Result<Vec<u8>, ArtworkError> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, img.width(), img.height());
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc
        .write_header()
        .map_err(|e| ArtworkError::Encode(e.to_string()))?;
    w.write_image_data(img.as_raw())
        .map_err(|e| ArtworkError::Encode(e.to_string()))?;
    w.finish()
        .map_err(|e| ArtworkError::Encode(e.to_string()))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROGRESSIVE_JPEG: &[u8] = include_bytes!("../../tests/fixtures/progressif.jpg");
    const LOSSY_WEBP: &[u8] = include_bytes!("../../tests/fixtures/avec-perte.webp");
    const LOSSLESS_WEBP: &[u8] = include_bytes!("../../tests/fixtures/sans-perte.webp");

    /// Relit un PNG produit par le module : (largeur, hauteur, RGB8).
    fn read_png(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
        let mut dec = png::Decoder::new(Cursor::new(bytes));
        dec.set_transformations(png::Transformations::EXPAND);
        let mut reader = dec.read_info().expect("PNG valide");
        let mut buf = vec![0u8; reader.output_buffer_size().expect("taille")];
        let frame = reader.next_frame(&mut buf).expect("image PNG complète");
        assert_eq!(frame.color_type, png::ColorType::Rgb);
        assert_eq!(frame.bit_depth, png::BitDepth::Eight);
        buf.truncate(frame.buffer_size());
        (frame.width, frame.height, buf)
    }

    fn pixel(art: &Artwork, x: u32, y: u32) -> [u8; 3] {
        let (w, h, data) = read_png(&art.png);
        assert!(x < w && y < h);
        let o = ((y * w + x) * 3) as usize;
        [data[o], data[o + 1], data[o + 2]]
    }

    fn near(p: [u8; 3], expected: [u8; 3], tol: i32) -> bool {
        p.iter()
            .zip(expected)
            .all(|(&a, b)| (i32::from(a) - i32::from(b)).abs() <= tol)
    }

    fn encode_rgba_png(w: u32, h: u32, px: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        let mut data = Vec::new();
        for y in 0..h {
            for x in 0..w {
                data.extend_from_slice(&px(x, y));
            }
        }
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().unwrap();
        writer.write_image_data(&data).unwrap();
        writer.finish().unwrap();
        out
    }

    /// JPEG de base : moitié gauche rouge, moitié droite bleue.
    fn two_tone_jpeg(w: u32, h: u32, exif: Option<&[u8]>) -> Vec<u8> {
        let img = RgbImage::from_fn(w, h, |x, _| {
            if x < w / 2 {
                image::Rgb([255, 0, 0])
            } else {
                image::Rgb([0, 0, 255])
            }
        });
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90)
            .encode_image(&img)
            .unwrap();
        if let Some(seg) = exif {
            // Insère un segment APP1 juste après le marqueur SOI.
            let mut with = out[..2].to_vec();
            with.extend_from_slice(&[0xff, 0xe1]);
            with.extend_from_slice(&((seg.len() + 2) as u16).to_be_bytes());
            with.extend_from_slice(seg);
            with.extend_from_slice(&out[2..]);
            return with;
        }
        out
    }

    /// Segment EXIF minimal (TIFF big-endian) ne portant que l'orientation.
    fn exif_orientation(value: u16) -> Vec<u8> {
        let mut e = b"Exif\0\0MM\0\x2a\0\0\0\x08".to_vec();
        e.extend_from_slice(&[0, 1, 0x01, 0x12, 0, 3, 0, 0, 0, 1]);
        e.extend_from_slice(&value.to_be_bytes());
        e.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        e
    }

    #[test]
    fn jpeg_progressif_vers_episode_et_couverture() {
        // Garde-fou du test : la fixture est bien un JPEG progressif (marqueur SOF2).
        assert!(PROGRESSIVE_JPEG.windows(2).any(|w| w == [0xff, 0xc2]));
        let ep = Artwork::convert(PROGRESSIVE_JPEG, ArtworkKind::Episode).unwrap();
        assert_eq!((ep.width, ep.height), (640, 480));
        let (w, h, _) = read_png(&ep.png);
        assert_eq!((w, h), (640, 480));
        // Image source 200 × 100, rognée en 133 × 100 : bande rouge à gauche, jaune au
        // centre, bleue à droite.
        assert!(near(pixel(&ep, 2, 240), [255, 0, 0], 40));
        assert!(near(pixel(&ep, 320, 240), [255, 255, 0], 40));
        assert!(near(pixel(&ep, 637, 240), [0, 0, 255], 40));

        let cv = Artwork::convert(PROGRESSIVE_JPEG, ArtworkKind::Cover).unwrap();
        assert_eq!((cv.width, cv.height), (512, 512));
        assert_eq!(read_png(&cv.png).0, 512);
        assert!(near(pixel(&cv, 256, 256), [255, 255, 0], 40));
    }

    #[test]
    fn webp_avec_et_sans_perte() {
        for (name, bytes) in [("avec perte", LOSSY_WEBP), ("sans perte", LOSSLESS_WEBP)] {
            let ep = Artwork::convert(bytes, ArtworkKind::Episode).unwrap_or_else(|e| {
                panic!("{name} : {e}");
            });
            assert_eq!((ep.width, ep.height), (640, 480), "{name}");
            assert!(near(pixel(&ep, 320, 240), [255, 255, 0], 40), "{name}");
            assert!(near(pixel(&ep, 2, 240), [255, 0, 0], 40), "{name}");
            let cv = Artwork::convert(bytes, ArtworkKind::Cover).unwrap();
            assert_eq!((cv.width, cv.height), (512, 512), "{name}");
        }
    }

    #[test]
    fn png_transparent_aplati_sur_le_fond() {
        // 64 × 64 : centre rouge opaque, reste transparent.
        let src = encode_rgba_png(64, 64, |x, y| {
            if (24..40).contains(&x) && (24..40).contains(&y) {
                [255, 0, 0, 255]
            } else {
                [0, 255, 0, 0]
            }
        });
        let white = Artwork::convert(&src, ArtworkKind::Cover).unwrap();
        assert_eq!((white.width, white.height), (512, 512));
        assert_eq!(pixel(&white, 2, 2), [255, 255, 255]);
        assert!(near(pixel(&white, 256, 256), [255, 0, 0], 3));

        let custom = Artwork::convert_on(&src, ArtworkKind::Cover, [10, 20, 30]).unwrap();
        assert_eq!(pixel(&custom, 2, 2), [10, 20, 30]);
    }

    #[test]
    fn alpha_partiel_melange_au_fond() {
        let src = encode_rgba_png(8, 8, |_, _| [200, 100, 0, 128]);
        let art = Artwork::convert_on(&src, ArtworkKind::Cover, [0, 0, 0]).unwrap();
        // 128/255 de la couleur sur un fond noir.
        assert!(near(pixel(&art, 256, 256), [100, 50, 0], 2));
    }

    #[test]
    fn image_minuscule_agrandie() {
        let src = encode_rgba_png(1, 1, |_, _| [9, 8, 7, 255]);
        let art = Artwork::convert(&src, ArtworkKind::Episode).unwrap();
        assert_eq!((art.width, art.height), (640, 480));
        assert_eq!(pixel(&art, 10, 10), [9, 8, 7]);
    }

    #[test]
    fn cadrage_centre_cover() {
        // 400 × 100 : tiers gauche rouge, tiers du milieu vert, tiers droit bleu. Une
        // couverture carrée garde le centre (100 × 100 vert) et jette les côtés.
        let src = encode_rgba_png(400, 100, |x, _| match x {
            0..=132 => [255, 0, 0, 255],
            133..=266 => [0, 255, 0, 255],
            _ => [0, 0, 255, 255],
        });
        let art = Artwork::convert(&src, ArtworkKind::Cover).unwrap();
        for (x, y) in [(5, 5), (256, 256), (506, 506), (5, 506)] {
            assert!(near(pixel(&art, x, y), [0, 255, 0], 3), "({x}, {y})");
        }
    }

    #[test]
    fn orientation_exif_appliquee() {
        // 400 × 200, rouge à gauche et bleu à droite. Orientation 6 (rotation de 90° dans le
        // sens horaire) : à l'affichage, le rouge est en haut et le bleu en bas.
        let src = two_tone_jpeg(400, 200, Some(&exif_orientation(6)));
        let art = Artwork::convert(&src, ArtworkKind::Episode).unwrap();
        assert_eq!((art.width, art.height), (640, 480));
        assert!(near(pixel(&art, 320, 20), [255, 0, 0], 40));
        assert!(near(pixel(&art, 320, 460), [0, 0, 255], 40));
        // Sans EXIF : rouge à gauche, bleu à droite.
        let plain = Artwork::convert(&two_tone_jpeg(400, 200, None), ArtworkKind::Episode).unwrap();
        assert!(near(pixel(&plain, 20, 240), [255, 0, 0], 40));
        assert!(near(pixel(&plain, 620, 240), [0, 0, 255], 40));
    }

    #[test]
    fn fallback_uni_aux_bonnes_dimensions() {
        let ep = Artwork::fallback(ArtworkKind::Episode, FALLBACK_COLOR).unwrap();
        let cv = Artwork::fallback(ArtworkKind::Cover, [1, 2, 3]).unwrap();
        assert_eq!((ep.width, ep.height), (640, 480));
        assert_eq!((cv.width, cv.height), (512, 512));
        let (w, h, data) = read_png(&ep.png);
        assert_eq!((w, h), (640, 480));
        assert!(data.chunks(3).all(|p| p == FALLBACK_COLOR));
        assert_eq!(pixel(&cv, 511, 511), [1, 2, 3]);
    }

    #[test]
    fn octets_vides_ou_trop_gros_refuses() {
        assert!(matches!(
            Artwork::convert(&[], ArtworkKind::Cover),
            Err(ArtworkError::Empty)
        ));
        let big = vec![0u8; MAX_INPUT_BYTES + 1];
        assert!(matches!(
            Artwork::convert(&big, ArtworkKind::Cover),
            Err(ArtworkError::TooLarge(_))
        ));
    }

    #[test]
    fn formats_inconnus_refuses() {
        assert!(matches!(
            Artwork::convert(b"GIF89a\x01\0\x01\0\0\0\0;", ArtworkKind::Cover),
            Err(ArtworkError::UnsupportedFormat)
        ));
        assert!(matches!(
            Artwork::convert(
                b"<svg xmlns='http://www.w3.org/2000/svg'/>",
                ArtworkKind::Cover
            ),
            Err(ArtworkError::UnsupportedFormat)
        ));
        assert!(matches!(
            Artwork::convert(&[0x5a; 4096], ArtworkKind::Cover),
            Err(ArtworkError::UnsupportedFormat)
        ));
    }

    #[test]
    fn images_corrompues_refusees() {
        let png = encode_rgba_png(32, 32, |x, y| [x as u8 * 8, y as u8 * 8, 0, 255]);
        let mut cases: Vec<(&str, Vec<u8>)> = Vec::new();
        for (name, good) in [
            ("png", png),
            ("jpeg", PROGRESSIVE_JPEG.to_vec()),
            ("webp avec perte", LOSSY_WEBP.to_vec()),
            ("webp sans perte", LOSSLESS_WEBP.to_vec()),
        ] {
            // Signature valide puis des octets quelconques.
            let mut garbage = good[..12.min(good.len())].to_vec();
            garbage.extend((0..2048u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8));
            cases.push((name, garbage));
            // Corps abîmé : on écrase le milieu du fichier.
            let mut broken = good.clone();
            let mid = broken.len() / 2;
            let end = (mid + 8).min(broken.len());
            for b in &mut broken[mid..end] {
                *b = !*b;
            }
            // Seul un PNG abîmé échoue toujours (CRC) : JPEG et WebP n'ont pas de somme de
            // contrôle et décodent alors une image fausse, sans erreur.
            if name == "png" {
                cases.push((name, broken));
            }
        }
        for (name, bytes) in cases {
            let r = Artwork::convert(&bytes, ArtworkKind::Cover);
            assert!(r.is_err(), "{name} corrompu accepté");
        }
    }

    #[test]
    fn images_tronquees_refusees() {
        let png = encode_rgba_png(64, 64, |x, y| [x as u8 * 4, y as u8 * 4, 128, 255]);
        for (name, good) in [
            ("png", png),
            ("jpeg", PROGRESSIVE_JPEG.to_vec()),
            ("webp avec perte", LOSSY_WEBP.to_vec()),
            ("webp sans perte", LOSSLESS_WEBP.to_vec()),
        ] {
            for cut in [good.len() / 2, good.len() / 4] {
                let r = Artwork::convert(&good[..cut], ArtworkKind::Cover);
                assert!(r.is_err(), "{name} tronqué à {cut} octets accepté");
            }
        }
    }

    #[test]
    fn bombe_png_refusee_avant_decodage() {
        // En-tête valide annonçant 60 000 × 60 000 (14 Gpx), sans aucune donnée d'image.
        let out = png_header_only(60_000, 60_000);
        assert!(matches!(
            Artwork::convert(&out, ArtworkKind::Cover),
            Err(ArtworkError::Dimensions(60_000, 60_000))
        ));
    }

    #[test]
    fn bombe_jpeg_refusee_avant_decodage() {
        let mut bytes = PROGRESSIVE_JPEG.to_vec();
        // SOF2 : FF C2, longueur (2), précision (1), hauteur (2), largeur (2).
        let sof = bytes.windows(2).position(|w| w == [0xff, 0xc2]).unwrap();
        bytes[sof + 5..sof + 7].copy_from_slice(&60_000u16.to_be_bytes());
        bytes[sof + 7..sof + 9].copy_from_slice(&60_000u16.to_be_bytes());
        assert!(matches!(
            Artwork::convert(&bytes, ArtworkKind::Cover),
            Err(ArtworkError::Dimensions(60_000, 60_000))
        ));
    }

    #[test]
    fn bombe_webp_refusee_avant_decodage() {
        // WebP avec perte : le code de début de trame 9D 01 2A est suivi de la largeur puis
        // de la hauteur (14 bits chacune, petit-boutiste), ici 16 000 × 16 000.
        let mut lossy = LOSSY_WEBP.to_vec();
        let at = lossy
            .windows(3)
            .position(|w| w == [0x9d, 0x01, 0x2a])
            .unwrap()
            + 3;
        lossy[at..at + 2].copy_from_slice(&16_000u16.to_le_bytes());
        lossy[at + 2..at + 4].copy_from_slice(&16_000u16.to_le_bytes());
        assert!(matches!(
            Artwork::convert(&lossy, ArtworkKind::Cover),
            Err(ArtworkError::Dimensions(16_000, 16_000))
        ));

        // WebP sans perte : signature 0x2F puis (largeur − 1) et (hauteur − 1) sur 14 bits.
        let mut lossless = LOSSLESS_WEBP.to_vec();
        let at = lossless.iter().position(|&b| b == 0x2f).unwrap() + 1;
        let bits: u32 = (16_000 - 1) | ((16_000 - 1) << 14);
        let old = u32::from_le_bytes(lossless[at..at + 4].try_into().unwrap());
        let new = (old & !0x0fff_ffff) | bits;
        lossless[at..at + 4].copy_from_slice(&new.to_le_bytes());
        assert!(matches!(
            Artwork::convert(&lossless, ArtworkKind::Cover),
            Err(ArtworkError::Dimensions(16_000, 16_000))
        ));
    }

    #[test]
    fn dimension_limite_acceptee_et_au_dessus_refusee() {
        let at_limit = png_header_only(MAX_DECODED_DIM, 1);
        // Largeur permise : l'échec éventuel vient des données absentes, pas des dimensions.
        assert!(!matches!(
            Artwork::convert(&at_limit, ArtworkKind::Cover),
            Err(ArtworkError::Dimensions(..))
        ));
        let over = png_header_only(MAX_DECODED_DIM + 1, 1);
        assert!(matches!(
            Artwork::convert(&over, ArtworkKind::Cover),
            Err(ArtworkError::Dimensions(..))
        ));
    }

    #[test]
    fn memoire_du_decodeur_bornee() {
        // 8 000 × 8 000 en RGBA 16 bits : 488 Mio décodés, au-dessus de MAX_DECODE_ALLOC. Les
        // dimensions sont admises, seul le budget mémoire peut refuser, avant tout décodage.
        let out = png_header_only_with(MAX_DECODED_DIM, MAX_DECODED_DIM, png::BitDepth::Sixteen);
        assert!(matches!(
            Artwork::convert(&out, ArtworkKind::Cover),
            Err(ArtworkError::Memory(_))
        ));
        // 8 000 × 8 000 en RGBA 8 bits (244 Mio) reste dans le budget : l'échec vient des
        // données absentes.
        let ok = png_header_only(MAX_DECODED_DIM, MAX_DECODED_DIM);
        assert!(!matches!(
            Artwork::convert(&ok, ArtworkKind::Cover),
            Err(ArtworkError::Memory(_))
        ));
    }

    /// PNG RGBA 8 bits dont l'en-tête annonce `w` × `h` et dont les données d'image sont vides.
    fn png_header_only(w: u32, h: u32) -> Vec<u8> {
        png_header_only_with(w, h, png::BitDepth::Eight)
    }

    /// Un PNG sans rangée de pixels, mais avec un bloc IDAT (flux zlib vide) : le décodeur
    /// lit l'en-tête sans erreur, seul le contrôle des dimensions peut le refuser.
    fn png_header_only_with(w: u32, h: u32, depth: png::BitDepth) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, w, h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(depth);
            let mut writer = enc.write_header().unwrap();
            writer
                .write_chunk(png::chunk::IDAT, &[0x78, 0x9c, 0x03, 0, 0, 0, 0, 1])
                .unwrap();
        }
        out
    }
}
