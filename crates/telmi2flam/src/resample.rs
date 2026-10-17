//! Redimensionnement Lanczos d'images RGBA, port scalaire de **Pillow 12.3.0** : même résultat
//! à l'octet près que `Image.resize(taille, Image.LANCZOS)` et `Image.thumbnail(taille,
//! Image.LANCZOS)` sur une image `RGBA` (décision D3 de `tools/telmi2flam/PORT-RUST.md`).
//!
//! Code porté (tag `12.3.0` de <https://github.com/python-pillow/Pillow>) :
//! - `src/libImaging/Resample.c` : `precompute_coeffs`, `normalize_coeffs_8bpc`,
//!   `_ImagingResample{Horizontal,Vertical}_8bpc` (4 bandes), `ImagingResampleInner`, filtre
//!   `lanczos_filter` et `clip8` (`PRECISION_BITS = 22`) ;
//! - `src/_imaging.c` : `_resize` (boîte en `float`, recadrage quand la boîte tombe juste) ;
//! - `src/libImaging/Convert.c` : `rgbA2rgba` (prémultiplication `MULDIV255`) et `rgba2rgbA` ;
//! - `src/PIL/Image.py` : `resize` (passage par `RGBa`, cas des images plus de 100 fois plus
//!   hautes que larges) et `thumbnail` (calcul de la taille conservant le rapport).
//!
//! `Reduce.c` n'est pas porté : `Image.resize` ne transmet pas `reducing_gap` à l'appel récursif
//! qu'il fait sur l'image `RGBa`, donc `thumbnail(…, reducing_gap=2.0)` ne réduit jamais une
//! image `RGBA` par `reduce` (vérifié avec Pillow 12.3.0 : `thumbnail` et `resize` sans
//! `reducing_gap` donnent les mêmes octets).
//!
//! Le flottant (`f64`, `sin` de la bibliothèque C) ne sert qu'aux coefficients, dans le même
//! ordre d'opérations que le C ; les pixels sont calculés en entiers 32 bits, sans SIMD.
//!
//! ---
//!
//! Ce module est un travail dérivé de Pillow, distribué sous la licence MIT-CMU (HPND) :
//!
//! The Python Imaging Library (PIL) is
//!
//! Copyright © 1997-2011 by Secret Labs AB
//! Copyright © 1995-2011 by Fredrik Lundh and contributors
//!
//! Pillow is the friendly PIL fork. It is
//!
//! Copyright © 2010 by Jeffrey 'Alex' Clark and contributors
//!
//! Like PIL, Pillow is licensed under the open source MIT-CMU License:
//!
//! By obtaining, using, and/or copying this software and/or its associated
//! documentation, you agree that you have read, understood, and will comply
//! with the following terms and conditions:
//!
//! Permission to use, copy, modify and distribute this software and its
//! documentation for any purpose and without fee is hereby granted,
//! provided that the above copyright notice appears in all copies, and that
//! both that copyright notice and this permission notice appear in supporting
//! documentation, and that the name of Secret Labs AB or the author not be
//! used in advertising or publicity pertaining to distribution of the software
//! without specific, written prior permission.
//!
//! SECRET LABS AB AND THE AUTHOR DISCLAIMS ALL WARRANTIES WITH REGARD TO THIS
//! SOFTWARE, INCLUDING ALL IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS.
//! IN NO EVENT SHALL SECRET LABS AB OR THE AUTHOR BE LIABLE FOR ANY SPECIAL,
//! INDIRECT OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES WHATSOEVER RESULTING FROM
//! LOSS OF USE, DATA OR PROFITS, WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE
//! OR OTHER TORTIOUS ACTION, ARISING OUT OF OR IN CONNECTION WITH THE USE OR
//! PERFORMANCE OF THIS SOFTWARE.
//!
//! (Texte aussi dans `crates/telmi2flam/LICENSE-Pillow.txt`, repris dans les notices.)

use crate::png::{PngError, Rgba};

type Result<T> = std::result::Result<T, PngError>;

/// `PRECISION_BITS` de `Resample.c` : 32 bits moins 8 bits de résultat moins 2 de marge.
const PRECISION_BITS: u32 = 32 - 8 - 2;
/// Support du filtre Lanczos (`LANCZOS.support`).
const LANCZOS_SUPPORT: f64 = 3.0;

/// Échec d'allocation : Pillow lève `MemoryError`, que le convertisseur Python range dans
/// `image_unreadable` (`except Exception`).
const NO_MEMORY: PngError = PngError::Unreadable("mémoire insuffisante (redimensionnement)");

fn alloc<T: Clone>(n: usize, fill: T) -> Result<Vec<T>> {
    let mut v = Vec::new();
    v.try_reserve_exact(n).map_err(|_| NO_MEMORY)?;
    v.resize(n, fill);
    Ok(v)
}

fn check(cancelled: &dyn Fn() -> bool) -> Result<()> {
    if cancelled() {
        Err(PngError::Cancelled)
    } else {
        Ok(())
    }
}

/// Octets d'une image de `w × h` pixels RGBA, ou `MemoryError`.
fn bytes_of(w: usize, h: usize) -> Result<usize> {
    w.checked_mul(h)
        .and_then(|n| n.checked_mul(4))
        .ok_or(NO_MEMORY)
}

/// Image 4 bandes de 8 bits (mode `RGBa` pendant le redimensionnement).
struct Plane {
    w: usize,
    h: usize,
    px: Vec<u8>,
}

impl Plane {
    fn row(&self, y: usize) -> &[u8] {
        &self.px[y * self.w * 4..(y + 1) * self.w * 4]
    }
}

// ---------------------------------------------------------------------------------------------
// Convert.c : RGBA <-> RGBa
// ---------------------------------------------------------------------------------------------

/// `MULDIV255(a, b)` de `ImagingUtils.h` : `a * b / 255` arrondi, par décalages.
fn muldiv255(a: u8, b: u8) -> u8 {
    let tmp = u32::from(a) * u32::from(b) + 128;
    (((tmp >> 8) + tmp) >> 8) as u8
}

/// `rgbA2rgba` : prémultiplication par l'alpha, sur place, ligne par ligne.
fn premultiply(img: &mut Plane, cancelled: &dyn Fn() -> bool) -> Result<()> {
    let w = img.w * 4;
    if w == 0 {
        return Ok(());
    }
    for line in img.px.chunks_exact_mut(w) {
        check(cancelled)?;
        for p in line.as_chunks_mut::<4>().0 {
            let a = p[3];
            p[0] = muldiv255(p[0], a);
            p[1] = muldiv255(p[1], a);
            p[2] = muldiv255(p[2], a);
        }
    }
    Ok(())
}

/// `rgba2rgbA` : division par l'alpha (`CLIP8(255 * c / a)`), sauf alpha 0 ou 255.
fn unpremultiply(img: &mut Plane, cancelled: &dyn Fn() -> bool) -> Result<()> {
    let w = img.w * 4;
    if w == 0 {
        return Ok(());
    }
    for line in img.px.chunks_exact_mut(w) {
        check(cancelled)?;
        for p in line.as_chunks_mut::<4>().0 {
            let a = u32::from(p[3]);
            if a != 0 && a != 255 {
                for c in &mut p[..3] {
                    *c = ((255 * u32::from(*c)) / a).min(255) as u8;
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Resample.c
// ---------------------------------------------------------------------------------------------

fn sinc(x: f64) -> f64 {
    if x == 0.0 {
        return 1.0;
    }
    let x = x * std::f64::consts::PI;
    x.sin() / x
}

/// `lanczos_filter` : sinus cardinal tronqué à 3 lobes.
fn lanczos(x: f64) -> f64 {
    if (-3.0..3.0).contains(&x) {
        sinc(x) * sinc(x / 3.0)
    } else {
        0.0
    }
}

/// `clip8` : `in >> PRECISION_BITS` (décalage arithmétique) borné à 0..255, comme la table
/// `_clip8_lookups` (indices -640..639, qui couvrent tout `i32 >> 22`).
fn clip8(v: i32) -> u8 {
    (v >> PRECISION_BITS).clamp(0, 255) as u8
}

/// Coefficients d'un axe : pour chaque pixel de sortie, premier pixel source et nombre de
/// pixels sources (`bounds`), puis `ksize` coefficients entiers (`kk`, déjà normalisés par
/// `normalize_coeffs_8bpc`).
struct Coeffs {
    ksize: usize,
    bounds: Vec<(usize, usize)>,
    kk: Vec<i32>,
}

/// `precompute_coeffs` suivi de `normalize_coeffs_8bpc`. `in0` et `in1` sont des `float` en C.
fn precompute_coeffs(in_size: usize, in0: f32, in1: f32, out_size: usize) -> Result<Coeffs> {
    // filterscale = scale = (double)(in1 - in0) / outSize : soustraction en float
    let scale = f64::from(in1 - in0) / out_size as f64;
    let filterscale = if scale < 1.0 { 1.0 } else { scale };
    let support = LANCZOS_SUPPORT * filterscale;
    // ksize = (int)ceil(support) * 2 + 1
    let ksize = (support.ceil() as i64) * 2 + 1;
    // `if (outSize > INT_MAX / (ksize * sizeof(double))) MemoryError` : même test en 64 bits
    // (identique tant que ksize * 8 tient dans un int, seul cas atteint par le convertisseur)
    if ksize <= 0 || (out_size as i64).saturating_mul(ksize.saturating_mul(8)) > i64::from(i32::MAX)
    {
        return Err(NO_MEMORY);
    }
    let ksize = ksize as usize;
    let mut kk = alloc(out_size * ksize, 0i32)?;
    let mut bounds = alloc(out_size, (0usize, 0usize))?;
    let mut k = alloc(ksize, 0f64)?;
    let inv_filterscale = 1.0 / filterscale;
    for xx in 0..out_size {
        let center = f64::from(in0) + (xx as f64 + 0.5) * scale;
        let mut ww = 0.0;
        // (int)(…) : troncature vers zéro, puis bornes
        let xmin = ((center - support + 0.5) as i64).max(0);
        let xmax = ((center + support + 0.5) as i64).min(in_size as i64) - xmin;
        // `center` reste dans [in0, in1] ⊂ [0, in_size] : xmin < in_size et xmax ≥ 1 dès que la
        // boîte n'est pas vide ; tout autre cas lirait hors de l'image en C.
        if xmin >= in_size as i64 || xmax <= 0 || xmax as usize > ksize {
            return Err(PngError::Unreadable(
                "boîte de redimensionnement hors de l'image",
            ));
        }
        let (xmin, xmax) = (xmin as usize, xmax as usize);
        for (x, kx) in k.iter_mut().enumerate().take(xmax) {
            let w = lanczos(((x + xmin) as f64 - center + 0.5) * inv_filterscale);
            *kx = w;
            ww += w;
        }
        if ww != 0.0 {
            for kx in k.iter_mut().take(xmax) {
                *kx /= ww;
            }
        }
        // normalize_coeffs_8bpc (les coefficients au-delà de xmax restent nuls)
        for (dst, &v) in kk[xx * ksize..xx * ksize + xmax].iter_mut().zip(&k) {
            let scaled = v * f64::from(1u32 << PRECISION_BITS);
            *dst = if v < 0.0 {
                (-0.5 + scaled) as i32
            } else {
                (0.5 + scaled) as i32
            };
        }
        bounds[xx] = (xmin, xmax);
    }
    Ok(Coeffs { ksize, bounds, kk })
}

/// `_ImagingResampleHorizontal_8bpc` (4 bandes) : lignes `offset..offset + out_h` de `src`.
fn horizontal(
    src: &Plane,
    out_h: usize,
    offset: usize,
    c: &Coeffs,
    cancelled: &dyn Fn() -> bool,
) -> Result<Plane> {
    let out_w = c.bounds.len();
    let mut out = alloc(bytes_of(out_w, out_h)?, 0u8)?;
    let half = 1i32 << (PRECISION_BITS - 1);
    for (yy, line_out) in out.chunks_exact_mut(out_w * 4).enumerate() {
        check(cancelled)?;
        let line_in = src.row(yy + offset);
        for (xx, px) in line_out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let (xmin, xmax) = c.bounds[xx];
            let k = &c.kk[xx * c.ksize..xx * c.ksize + xmax];
            let mut ss = [half; 4];
            for (p, &kx) in line_in[xmin * 4..(xmin + xmax) * 4]
                .as_chunks::<4>()
                .0
                .iter()
                .zip(k)
            {
                for (s, &v) in ss.iter_mut().zip(p) {
                    *s = s.wrapping_add(i32::from(v).wrapping_mul(kx));
                }
            }
            for (o, s) in px.iter_mut().zip(ss) {
                *o = clip8(s);
            }
        }
    }
    Ok(Plane {
        w: out_w,
        h: out_h,
        px: out,
    })
}

/// `_ImagingResampleVertical_8bpc` (4 bandes). Les sommes sont entières (et sans débordement :
/// |Σk| < 2^23, 255 × 2^23 < 2^31), donc l'ordre de parcours ne change pas le résultat ; les
/// lignes sources sont parcourues dans l'ordre de la mémoire.
fn vertical(src: &Plane, c: &Coeffs, cancelled: &dyn Fn() -> bool) -> Result<Plane> {
    let out_h = c.bounds.len();
    let w4 = src.w * 4;
    let mut out = alloc(bytes_of(src.w, out_h)?, 0u8)?;
    let mut acc = alloc(w4, 0i32)?;
    let half = 1i32 << (PRECISION_BITS - 1);
    if w4 == 0 {
        return Ok(Plane {
            w: src.w,
            h: out_h,
            px: out,
        });
    }
    for (yy, line_out) in out.chunks_exact_mut(w4).enumerate() {
        check(cancelled)?;
        let (ymin, ymax) = c.bounds[yy];
        let k = &c.kk[yy * c.ksize..yy * c.ksize + ymax];
        acc.fill(half);
        for (y, &ky) in k.iter().enumerate() {
            for (s, &v) in acc.iter_mut().zip(src.row(y + ymin)) {
                *s = s.wrapping_add(i32::from(v).wrapping_mul(ky));
            }
        }
        for (o, &s) in line_out.iter_mut().zip(&acc) {
            *o = clip8(s);
        }
    }
    Ok(Plane {
        w: src.w,
        h: out_h,
        px: out,
    })
}

/// `ImagingResampleInner` avec le filtre Lanczos : passage horizontal (sur les seules lignes
/// utiles au passage vertical), puis vertical.
fn resample_inner(
    src: &Plane,
    xsize: usize,
    ysize: usize,
    bx: [f32; 4],
    cancelled: &dyn Fn() -> bool,
) -> Result<Plane> {
    let need_horizontal = xsize != src.w || bx[0] != 0.0 || bx[2] != xsize as f32;
    let need_vertical = ysize != src.h || bx[1] != 0.0 || bx[3] != ysize as f32;

    let mut vert = precompute_coeffs(src.h, bx[1], bx[3], ysize)?;
    // premières et dernière lignes sources utilisées
    let ybox_first = vert.bounds[0].0;
    let (ymin_last, ymax_last) = vert.bounds[ysize - 1];
    let ybox_last = ymin_last + ymax_last;

    let mut temp = None;
    if need_horizontal {
        let horiz = precompute_coeffs(src.w, bx[0], bx[2], xsize)?;
        for b in &mut vert.bounds {
            // C : `bounds_vert[i * 2] -= ybox_first` ; une ligne antérieure à la première
            // n'existe pas (les débuts de fenêtre croissent avec la ligne de sortie)
            b.0 = b.0.checked_sub(ybox_first).ok_or(PngError::Unreadable(
                "boîte de redimensionnement hors de l'image",
            ))?;
        }
        let rows = ybox_last
            .checked_sub(ybox_first)
            .ok_or(PngError::Unreadable(
                "boîte de redimensionnement hors de l'image",
            ))?;
        temp = Some(horizontal(src, rows, ybox_first, &horiz, cancelled)?);
    }
    let input = temp.as_ref().unwrap_or(src);
    if need_vertical {
        if vert.bounds.iter().any(|&(lo, n)| lo + n > input.h) {
            return Err(PngError::Unreadable(
                "boîte de redimensionnement hors de l'image",
            ));
        }
        return vertical(input, &vert, cancelled);
    }
    match temp {
        Some(t) => Ok(t),
        None => copy(src),
    }
}

fn copy(src: &Plane) -> Result<Plane> {
    let mut px = alloc(0, 0u8)?;
    px.try_reserve_exact(src.px.len()).map_err(|_| NO_MEMORY)?;
    px.extend_from_slice(&src.px);
    Ok(Plane {
        w: src.w,
        h: src.h,
        px,
    })
}

/// `_resize` de `_imaging.c` (filtre Lanczos) : boîte convertie en `float`, recadrage
/// (`ImagingCrop`) quand la boîte est entière et de la taille demandée, sinon rééchantillonnage.
/// Les boîtes passées ici sont entières, positives et dans l'image (appels de [`resize_rgba`]).
fn imaging_resize(
    src: &Plane,
    xsize: usize,
    ysize: usize,
    bx: [f32; 4],
    cancelled: &dyn Fn() -> bool,
) -> Result<Plane> {
    let int = |v: f32| v - (v as i32) as f32 == 0.0;
    if int(bx[0]) && bx[2] - bx[0] == xsize as f32 && int(bx[1]) && bx[3] - bx[1] == ysize as f32 {
        // boîte entière : pixels (x0..x0 + xsize, y0..y0 + ysize)
        let (x0, y0) = (bx[0] as usize, bx[1] as usize);
        if x0 + xsize > src.w || y0 + ysize > src.h {
            return Err(PngError::Unreadable(
                "boîte de redimensionnement hors de l'image",
            ));
        }
        let mut px = alloc(0, 0u8)?;
        px.try_reserve_exact(bytes_of(xsize, ysize)?)
            .map_err(|_| NO_MEMORY)?;
        for y in y0..y0 + ysize {
            px.extend_from_slice(&src.row(y)[x0 * 4..(x0 + xsize) * 4]);
        }
        return Ok(Plane {
            w: xsize,
            h: ysize,
            px,
        });
    }
    resample_inner(src, xsize, ysize, bx, cancelled)
}

// ---------------------------------------------------------------------------------------------
// Image.py
// ---------------------------------------------------------------------------------------------

/// `Image.resize((w, h), Image.LANCZOS)` sur une image `RGBA` (sans `box` ni `reducing_gap`) :
/// `convert("RGBa")`, redimensionnement, `convert("RGBA")`.
///
/// `cancelled` est consulté à chaque ligne produite (conversions et passages) :
/// [`PngError::Cancelled`] dès qu'il rend `true`. Un manque de mémoire (y compris le refus de
/// `precompute_coeffs` pour un rapport de taille démesuré) rend [`PngError::Unreadable`],
/// comme le `MemoryError` de Pillow. Une taille nulle rend aussi `Unreadable` (`ValueError`).
pub fn resize_rgba(img: Rgba, w: u32, h: u32, cancelled: &dyn Fn() -> bool) -> Result<Rgba> {
    if (img.width, img.height) == (w, h) {
        return Ok(img);
    }
    if w == 0 || h == 0 || img.width == 0 || img.height == 0 {
        return Err(PngError::Unreadable("taille de redimensionnement nulle"));
    }
    let (sw, sh) = (img.width as usize, img.height as usize);
    if img.rgba.len() != bytes_of(sw, sh)? {
        return Err(PngError::Unreadable("tampon RGBA incohérent"));
    }
    let (w, h) = (w as usize, h as usize);
    let mut src = Plane {
        w: sw,
        h: sh,
        px: img.rgba,
    };
    premultiply(&mut src, cancelled)?;
    let full = [0.0, 0.0, sw as f32, sh as f32];
    let mut out = if sh as u64 > sw as u64 * 100 && h < sh {
        // image très haute : passage vertical seul vers (sw, h), puis horizontal seul
        let tall = imaging_resize(&src, sw, h, full, cancelled)?;
        drop(src);
        imaging_resize(&tall, w, h, [0.0, 0.0, sw as f32, h as f32], cancelled)?
    } else {
        imaging_resize(&src, w, h, full, cancelled)?
    };
    unpremultiply(&mut out, cancelled)?;
    Ok(Rgba {
        width: out.w as u32,
        height: out.h as u32,
        rgba: out.px,
    })
}

/// Taille calculée par `Image.thumbnail((max_w, max_h))` pour une image `w × h` : rapport
/// conservé au plus près (`round_aspect`), `None` si l'image tient déjà (aucun changement).
pub fn thumbnail_size(w: u32, h: u32, max_w: u32, max_h: u32) -> Option<(u32, u32)> {
    let (x, y) = (u64::from(max_w), u64::from(max_h));
    if x >= u64::from(w) && y >= u64::from(h) {
        return None;
    }
    if w == 0 || h == 0 || y == 0 {
        // ZeroDivisionError en Python : jamais atteint (images et cibles non vides)
        return None;
    }
    let aspect = f64::from(w) / f64::from(h);
    // round_aspect : min(floor, ceil, key=…) garde floor en cas d'égalité, puis max(…, 1)
    let round_aspect = |number: f64, key: &dyn Fn(f64) -> f64| -> u64 {
        let (lo, hi) = (number.floor(), number.ceil());
        let best = if key(hi) < key(lo) { hi } else { lo };
        (best as u64).max(1)
    };
    let (yf, xf) = (y as f64, x as f64);
    let (x, y) = if xf / yf >= aspect {
        (round_aspect(yf * aspect, &|n| (aspect - n / yf).abs()), y)
    } else {
        let key = |n: f64| {
            if n == 0.0 {
                0.0
            } else {
                (aspect - xf / n).abs()
            }
        };
        (x, round_aspect(xf / aspect, &key))
    };
    Some((x as u32, y as u32))
}

/// `Image.thumbnail((max_w, max_h), Image.LANCZOS)` sur une image `RGBA` : taille de
/// [`thumbnail_size`], puis [`resize_rgba`] (le `reducing_gap` de `thumbnail` est sans effet
/// sur une image `RGBA`, voir l'en-tête du module).
pub fn thumbnail(img: Rgba, max_w: u32, max_h: u32, cancelled: &dyn Fn() -> bool) -> Result<Rgba> {
    match thumbnail_size(img.width, img.height, max_w, max_h) {
        Some((w, h)) => resize_rgba(img, w, h, cancelled),
        None => Ok(img),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn never() -> bool {
        false
    }

    fn img(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Rgba {
        let mut rgba = Vec::new();
        for y in 0..h {
            for x in 0..w {
                rgba.extend_from_slice(&f(x, y));
            }
        }
        Rgba {
            width: w,
            height: h,
            rgba,
        }
    }

    #[test]
    fn muldiv255_est_l_arrondi_de_a_fois_b_sur_255() {
        for a in 0..=255u32 {
            for b in 0..=255u32 {
                let exact = (a * b * 2 + 255) / 510; // a*b/255 arrondi au plus proche
                assert_eq!(u32::from(muldiv255(a as u8, b as u8)), exact, "{a} {b}");
            }
        }
    }

    #[test]
    fn clip8_couvre_la_table_de_pillow() {
        assert_eq!(clip8(i32::MIN), 0);
        assert_eq!(clip8(-1), 0);
        assert_eq!(clip8((1 << 22) - 1), 0);
        assert_eq!(clip8(1 << 22), 1);
        assert_eq!(clip8(255 << 22), 255);
        assert_eq!(clip8(i32::MAX), 255);
    }

    #[test]
    fn taille_de_vignette_comme_pillow() {
        // valeurs relevées avec Pillow 12.3.0 (Image.new("RGBA", (w, h)).thumbnail(...))
        assert_eq!(thumbnail_size(640, 480, 128, 96), Some((128, 96)));
        assert_eq!(thumbnail_size(641, 481, 128, 96), Some((128, 96)));
        assert_eq!(thumbnail_size(3, 2049, 2047, 2047), Some((3, 2047)));
        assert_eq!(thumbnail_size(2049, 1, 128, 96), Some((128, 1)));
        assert_eq!(thumbnail_size(1, 2049, 89, 120), Some((1, 120)));
        assert_eq!(thumbnail_size(97, 72, 128, 96), None);
        assert_eq!(thumbnail_size(128, 96, 128, 96), None);
        assert_eq!(thumbnail_size(129, 96, 128, 96), Some((128, 95)));
    }

    #[test]
    fn meme_taille_ne_touche_pas_l_image() {
        let a = img(5, 3, |x, y| [x as u8, y as u8, 7, 100]);
        assert_eq!(resize_rgba(a.clone(), 5, 3, &never).unwrap(), a);
    }

    #[test]
    fn couleur_unie_opaque_reste_unie() {
        for (sw, sh, w, h) in [(1, 1, 212, 212), (97, 72, 128, 95), (3, 400, 1, 120)] {
            let a = img(sw, sh, |_, _| [200, 10, 99, 255]);
            let b = resize_rgba(a, w, h, &never).unwrap();
            assert_eq!((b.width, b.height), (w, h));
            assert!(b.rgba.chunks(4).all(|p| p == [200, 10, 99, 255]));
        }
    }

    #[test]
    fn annulation_consultee_pendant_le_redimensionnement() {
        use std::cell::Cell;
        let a = img(300, 300, |x, y| [x as u8, y as u8, 0, 255]);
        let calls = Cell::new(0);
        let cancel = || {
            calls.set(calls.get() + 1);
            calls.get() > 400
        };
        assert_eq!(resize_rgba(a, 128, 128, &cancel), Err(PngError::Cancelled));
    }

    #[test]
    fn rapport_demesure_rend_memory_error() {
        // out × ksize × 8 ≈ 48 × in : refus de Pillow dès qu'un côté réduit dépasse ~44,7 M px
        let n = 45_000_000usize;
        assert!(matches!(
            precompute_coeffs(n, 0.0, n as f32, 120),
            Err(PngError::Unreadable(_))
        ));
        let n = 100_000usize;
        assert!(precompute_coeffs(n, 0.0, n as f32, 1).is_ok());
    }

    #[test]
    fn tailles_nulles_ou_tampon_faux_sans_panique() {
        let a = img(2, 2, |_, _| [1, 2, 3, 4]);
        assert!(resize_rgba(a.clone(), 0, 3, &never).is_err());
        let mut b = a;
        b.rgba.pop();
        assert!(resize_rgba(b, 3, 3, &never).is_err());
    }
}
