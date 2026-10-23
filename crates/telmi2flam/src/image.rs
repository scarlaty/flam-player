//! Conversions PNG vers LIF de `tools/telmi2flam/telmi2flam.py` : `png_to_lif`,
//! `png_to_scene_lif`, `png_to_fit_lif`, `png_to_thumb_lif`, avec `_bound` et
//! `_avoid_transpose`, et les trois modes de vignette (`flou`, `rogner`, `bandes`, voir
//! [`ThumbFit`]). Sortie identique à l'octet près à celle de Python 3.14.8 + Pillow 12.3.0
//! (décodage [`crate::png`], redimensionnement [`crate::resample`], flou et composition alpha
//! `crate::blur`, encodeur [`crate::lif`]).
//!
//! Erreurs : celles du décodage ([`PngError`]) ; un manque de mémoire pendant le
//! redimensionnement est [`PngError::Unreadable`], comme le `MemoryError` attrapé par Python
//! (avertissement `image_unreadable`, ou `cover_unreadable` pour la couverture). Le jeton d'annulation est consulté à chaque ligne décodée
//! ou redimensionnée.

use crate::blur::{alpha_composite, gaussian_blur};
use crate::lif;
use crate::png::{decode_rgba, PngError, Rgba};
use crate::resample::{resize_rgba, thumbnail};

type Result<T> = std::result::Result<T, PngError>;

/// Zone d'affichage des scènes sous le bandeau (`VISUAL_W`, `VISUAL_H`).
pub const VISUAL_W: u32 = 320;
pub const VISUAL_H: u32 = 212;
/// Vignette de l'histoire (`THUMB_W`, `THUMB_H`) : 104 × 136, taille des vignettes officielles.
pub const THUMB_W: u32 = 104;
pub const THUMB_H: u32 = 136;
/// Rayon du flou du fond des vignettes en mode `flou` (`_THUMB_BLUR_RADIUS`).
const THUMB_BLUR_RADIUS: f32 = 8.0;

/// Remplissage de la vignette 104 × 136 (`--thumb-fit`, `THUMB_FITS` de Python).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThumbFit {
    /// Image entière centrée ; fond : la même image agrandie pour couvrir puis floutée
    /// (`GaussianBlur(8)`). Défaut de Python (`DEFAULT_THUMB_FIT`).
    #[default]
    Flou,
    /// Image agrandie pour couvrir la vignette, rognée au centre.
    Rogner,
    /// Image entière centrée, bandes transparentes.
    Bandes,
}

impl ThumbFit {
    /// Nom de l'option (`flou`, `rogner`, `bandes`).
    pub fn as_str(self) -> &'static str {
        match self {
            ThumbFit::Flou => "flou",
            ThumbFit::Rogner => "rogner",
            ThumbFit::Bandes => "bandes",
        }
    }

    /// Mode nommé par l'option, ou `None` si le nom est inconnu.
    pub fn from_name(name: &str) -> Option<ThumbFit> {
        [ThumbFit::Flou, ThumbFit::Rogner, ThumbFit::Bandes]
            .into_iter()
            .find(|f| f.as_str() == name)
    }
}

/// Icônes d'inventaire (`INV_W`, `INV_H`).
pub const INV_W: u32 = 128;
pub const INV_H: u32 = 128;

/// Position horizontale d'une image portrait sur son canevas carré (`_avoid_transpose`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Center,
    Left,
}

/// `Image.new("RGBA", (w, h), (0, 0, 0, 0))`.
fn transparent(w: u32, h: u32) -> Result<Rgba> {
    let n = (w as usize)
        .checked_mul(h as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or(PngError::Unreadable("mémoire insuffisante (canevas)"))?;
    let mut rgba = Vec::new();
    rgba.try_reserve_exact(n)
        .map_err(|_| PngError::Unreadable("mémoire insuffisante (canevas)"))?;
    rgba.resize(n, 0);
    Ok(Rgba {
        width: w,
        height: h,
        rgba,
    })
}

/// `canvas.paste(im, (x, y))` sans masque : copie brute des pixels, découpée au canevas.
fn paste(canvas: &mut Rgba, im: &Rgba, x: i64, y: i64) {
    let (cw, ch) = (i64::from(canvas.width), i64::from(canvas.height));
    let (iw, ih) = (i64::from(im.width), i64::from(im.height));
    let (x0, x1) = (x.max(0), (x + iw).min(cw));
    let (y0, y1) = (y.max(0), (y + ih).min(ch));
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let n = ((x1 - x0) * 4) as usize;
    for cy in y0..y1 {
        let dst = ((cy * cw + x0) * 4) as usize;
        let src = (((cy - y) * iw + (x0 - x)) * 4) as usize;
        canvas.rgba[dst..dst + n].copy_from_slice(&im.rgba[src..src + n]);
    }
}

/// `_bound` : au-delà de [`lif::MAX_DIM`] sur un côté, `thumbnail((2047, 2047))`.
fn bound(im: Rgba, cancelled: &dyn Fn() -> bool) -> Result<Rgba> {
    if im.width > lif::MAX_DIM || im.height > lif::MAX_DIM {
        thumbnail(im, lif::MAX_DIM, lif::MAX_DIM, cancelled)
    } else {
        Ok(im)
    }
}

/// `_avoid_transpose` : une image portrait de plus de 64 px de haut est posée sur un canevas
/// carré transparent, pour que le décodeur (`lif_decoder.c`) ne tente pas de transposition.
fn avoid_transpose(im: Rgba, align: Align) -> Result<Rgba> {
    let (w, h) = (im.width, im.height);
    if h > w && h > 64 {
        let mut canvas = transparent(h, h)?;
        let x = match align {
            Align::Left => 0,
            Align::Center => (i64::from(h) - i64::from(w)).div_euclid(2),
        };
        paste(&mut canvas, &im, x, 0);
        return Ok(canvas);
    }
    Ok(im)
}

/// `_encode` : `lif.encode(im.tobytes(), w, h)`. L'encodeur ne refuse qu'un tampon de taille
/// fausse (`assert` de `lif.py`, exception rangée dans `image_unreadable` côté Python).
fn encode(im: &Rgba) -> Result<Vec<u8>> {
    lif::encode(&im.rgba, im.width, im.height)
        .map_err(|_| PngError::Unreadable("encodage LIF impossible"))
}

/// `png_to_lif(png_bytes, None)` : taille native, bornée à 2047 px, sans transposition.
pub fn png_to_lif(png: &[u8], cancelled: &dyn Fn() -> bool) -> Result<Vec<u8>> {
    let im = decode_rgba(png, cancelled)?;
    encode(&avoid_transpose(bound(im, cancelled)?, Align::Center)?)
}

/// Image ajustée sans déformation dans `w × h` (agrandie ou réduite), centrée sur un canevas
/// transparent de exactement `w × h` (corps de `png_to_scene_lif`, et `_thumb_contain` de
/// `png_to_thumb_lif`).
fn fit_on_canvas(mut im: Rgba, w: u32, h: u32, cancelled: &dyn Fn() -> bool) -> Result<Rgba> {
    let (sw, sh) = (im.width, im.height);
    if sw == 0 || sh == 0 {
        return Err(PngError::Unreadable("image vide"));
    }
    // scale = min(w / sw, h / sh) ; round() de Python : arrondi au pair
    let scale = (f64::from(w) / f64::from(sw)).min(f64::from(h) / f64::from(sh));
    let nw = (f64::from(sw) * scale).round_ties_even().max(1.0) as u32;
    let nh = (f64::from(sh) * scale).round_ties_even().max(1.0) as u32;
    if (nw, nh) != (sw, sh) {
        im = resize_rgba(im, nw, nh, cancelled)?;
    }
    let mut canvas = transparent(w, h)?;
    let x = (i64::from(w) - i64::from(nw)).div_euclid(2);
    let y = (i64::from(h) - i64::from(nh)).div_euclid(2);
    paste(&mut canvas, &im, x, y);
    Ok(canvas)
}

/// `im.crop((left, top, right, bottom))` : copie de la fenêtre (dans l'image).
fn crop(im: &Rgba, left: u32, top: u32, w: u32, h: u32) -> Result<Rgba> {
    let mut out = transparent(w, h)?;
    paste(&mut out, im, -i64::from(left), -i64::from(top));
    Ok(out)
}

/// `_thumb_cover` : fenêtre centrale de la source au rapport de la vignette (`round()` de
/// Python, demi au pair, au moins 1 px), redimensionnée en 104 × 136.
fn thumb_cover(im: &Rgba, cancelled: &dyn Fn() -> bool) -> Result<Rgba> {
    let (sw, sh) = (im.width, im.height);
    if sw == 0 || sh == 0 {
        return Err(PngError::Unreadable("image vide"));
    }
    // sh * THUMB_W / THUMB_H : produit entier exact puis division vraie, comme Python
    let ratio = |n: u32, num: u32, den: u32| {
        ((f64::from(n) * f64::from(num)) / f64::from(den))
            .round_ties_even()
            .max(1.0) as u32
    };
    let (cw, ch) = if u64::from(sw) * u64::from(THUMB_H) > u64::from(sh) * u64::from(THUMB_W) {
        (ratio(sh, THUMB_W, THUMB_H), sh)
    } else {
        (sw, ratio(sw, THUMB_H, THUMB_W))
    };
    let (left, top) = ((sw - cw) / 2, (sh - ch) / 2);
    resize_rgba(crop(im, left, top, cw, ch)?, THUMB_W, THUMB_H, cancelled)
}

/// `png_to_scene_lif` : ajustée sans déformation dans 320 × 212 (agrandie ou réduite), centrée
/// sur un canevas transparent de exactement 320 × 212.
pub fn png_to_scene_lif(png: &[u8], cancelled: &dyn Fn() -> bool) -> Result<Vec<u8>> {
    let im = decode_rgba(png, cancelled)?;
    encode(&fit_on_canvas(im, VISUAL_W, VISUAL_H, cancelled)?)
}

/// `png_to_fit_lif(png_bytes, maxw, maxh, align)` : tient dans `max_w × max_h` en gardant le
/// rapport (`thumbnail`), puis `_avoid_transpose`.
pub fn png_to_fit_lif(
    png: &[u8],
    max_w: u32,
    max_h: u32,
    align: Align,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<u8>> {
    let im = thumbnail(decode_rgba(png, cancelled)?, max_w, max_h, cancelled)?;
    encode(&avoid_transpose(im, align)?)
}

/// `png_to_thumb_lif(png_bytes, fit)` : vignette 104 × 136 selon le mode `fit`.
pub fn png_to_thumb_lif(
    png: &[u8],
    fit: ThumbFit,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<u8>> {
    let im = decode_rgba(png, cancelled)?;
    let out = match fit {
        ThumbFit::Rogner => thumb_cover(&im, cancelled)?,
        ThumbFit::Bandes => fit_on_canvas(im, THUMB_W, THUMB_H, cancelled)?,
        ThumbFit::Flou => {
            // fond opaque : flouter du RGBA non prémultiplié laisserait des halos sombres
            let bg = thumb_cover(&im, cancelled)?;
            let mut base = transparent(THUMB_W, THUMB_H)?;
            for p in base.rgba.as_chunks_mut::<4>().0 {
                *p = [128, 128, 128, 255];
            }
            let bg = alpha_composite(&base, &bg, cancelled)?;
            let bg = gaussian_blur(bg, THUMB_BLUR_RADIUS, cancelled)?;
            alpha_composite(
                &bg,
                &fit_on_canvas(im, THUMB_W, THUMB_H, cancelled)?,
                cancelled,
            )?
        }
    };
    encode(&out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn im(w: u32, h: u32, v: u8) -> Rgba {
        Rgba {
            width: w,
            height: h,
            rgba: vec![v; (w * h * 4) as usize],
        }
    }

    #[test]
    fn paste_decoupe_au_canevas() {
        let mut c = im(4, 3, 0);
        paste(&mut c, &im(3, 2, 9), -1, 2);
        let set: Vec<usize> = (0..12).filter(|&i| c.rgba[i * 4] == 9).collect();
        assert_eq!(set, vec![8, 9]);
        let mut c = im(2, 2, 0);
        paste(&mut c, &im(1, 1, 9), 5, 5);
        assert!(c.rgba.iter().all(|&v| v == 0));
    }

    #[test]
    fn modes_de_vignette_comme_python() {
        assert_eq!(ThumbFit::default(), ThumbFit::Flou); // DEFAULT_THUMB_FIT
        for name in ["flou", "rogner", "bandes"] {
            assert_eq!(ThumbFit::from_name(name).unwrap().as_str(), name);
        }
        assert_eq!(ThumbFit::from_name("Flou"), None);
    }

    #[test]
    fn rognage_centre_au_rapport_de_la_vignette() {
        // 300 × 136 : fenêtre round(136 × 104 / 136) = 104 px de large, à partir de x = 98
        let mut src = im(300, 136, 0);
        for (i, p) in src.rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let x = (i % 300) as u8;
            *p = [x, 0, 0, 255];
        }
        let out = thumb_cover(&src, &|| false).unwrap();
        assert_eq!((out.width, out.height), (THUMB_W, THUMB_H));
        assert_eq!(out.rgba[0], 98);
        assert_eq!(out.rgba[(THUMB_W as usize - 1) * 4], 201);
    }

    #[test]
    fn transposition_evitee_pour_les_portraits_de_plus_de_64_px() {
        let a = avoid_transpose(im(10, 65, 7), Align::Center).unwrap();
        assert_eq!((a.width, a.height), (65, 65));
        assert_eq!(a.rgba[(27 * 4) as usize], 7); // colonne (65 - 10) // 2 = 27
        assert_eq!(a.rgba[(26 * 4) as usize], 0);
        let l = avoid_transpose(im(10, 65, 7), Align::Left).unwrap();
        assert_eq!(l.rgba[0], 7);
        let p = avoid_transpose(im(10, 64, 7), Align::Center).unwrap();
        assert_eq!((p.width, p.height), (10, 64));
    }
}
