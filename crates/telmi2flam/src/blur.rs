//! Flou gaussien et composition alpha d'images RGBA, port scalaire de **Pillow 12.3.0** : même
//! résultat à l'octet près que `Image.filter(ImageFilter.GaussianBlur(r))` et
//! `Image.alpha_composite(a, b)` sur des images `RGBA` (fond de la vignette en mode `flou`).
//!
//! Code porté (tag `12.3.0` de <https://github.com/python-pillow/Pillow>) :
//! - `src/libImaging/BoxBlur.c` : `_gaussian_blur_radius` (calcul en `float`, racine en
//!   `double`), `ImagingGaussianBlur`, `ImagingBoxBlur` (3 passes horizontales puis 3 passes
//!   sur l'image transposée), `ImagingHorizontalBoxBlur` et `ImagingLineBoxBlur32`
//!   (accumulateurs `UINT32` modulo 2³², poids `ww` et `fw` sur 24 bits, bords étendus par
//!   répétition du premier et du dernier pixel) ;
//! - `src/libImaging/AlphaComposite.c` : `ImagingAlphaComposite` (`PRECISION_BITS = 7`,
//!   `SHIFTFORDIV255` de `ImagingUtils.h`) ;
//! - `src/_imaging.c` : `_gaussian_blur` (rayon `float`, 3 passes) ; `src/PIL/ImageFilter.py` :
//!   `GaussianBlur.filter` (rayon nul : copie). `Image.filter` applique un `MultibandFilter`
//!   aux 4 bandes à la fois, sans prémultiplication par l'alpha.
//!
//! Le flottant ne sert qu'au rayon des boîtes et aux poids, dans le même ordre d'opérations que
//! le C (`float` 32 bits sans précision étendue ni contraction FMA, comme le wheel MSVC
//! x86-64) ; les pixels sont calculés en entiers.
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

/// Nombre de passes de `_gaussian_blur` (`passes = 3` par défaut).
const PASSES: u32 = 3;

const NO_MEMORY: PngError = PngError::Unreadable("mémoire insuffisante (flou)");

fn check(cancelled: &dyn Fn() -> bool) -> Result<()> {
    if cancelled() {
        Err(PngError::Cancelled)
    } else {
        Ok(())
    }
}

fn alloc(n: usize) -> Result<Vec<u8>> {
    let mut v = Vec::new();
    v.try_reserve_exact(n).map_err(|_| NO_MEMORY)?;
    v.resize(n, 0);
    Ok(v)
}

/// Taille du tampon d'une image `w × h` RGBA, vérifiée.
fn checked_len(im: &Rgba) -> Result<usize> {
    let n = (im.width as usize)
        .checked_mul(im.height as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or(NO_MEMORY)?;
    if n != im.rgba.len() {
        return Err(PngError::Unreadable("tampon RGBA incohérent"));
    }
    Ok(n)
}

// ---------------------------------------------------------------------------------------------
// AlphaComposite.c
// ---------------------------------------------------------------------------------------------

/// `SHIFTFORDIV255(a)` de `ImagingUtils.h`.
fn shift_for_div255(a: u32) -> u32 {
    ((a >> 8) + a) >> 8
}

/// `Image.alpha_composite(dst, src)` : `src` posée sur `dst` (mêmes tailles), nouvelle image.
pub fn alpha_composite(dst: &Rgba, src: &Rgba, cancelled: &dyn Fn() -> bool) -> Result<Rgba> {
    const PRECISION_BITS: u32 = 7;
    if (dst.width, dst.height) != (src.width, src.height) {
        return Err(PngError::Unreadable(
            "composition d'images de tailles différentes",
        ));
    }
    let n = checked_len(dst)?;
    checked_len(src)?;
    let mut out = alloc(n)?;
    let row = dst.width as usize * 4;
    if row == 0 {
        return Ok(Rgba {
            width: dst.width,
            height: dst.height,
            rgba: out,
        });
    }
    for ((o, d), s) in out
        .chunks_exact_mut(row)
        .zip(dst.rgba.chunks_exact(row))
        .zip(src.rgba.chunks_exact(row))
    {
        check(cancelled)?;
        for ((o, d), s) in o
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(d.as_chunks::<4>().0)
            .zip(s.as_chunks::<4>().0)
        {
            let sa = u32::from(s[3]);
            if sa == 0 {
                *o = *d;
                continue;
            }
            let blend = u32::from(d[3]) * (255 - sa);
            let outa255 = sa * 255 + blend;
            let coef1 = sa * 255 * 255 * (1 << PRECISION_BITS) / outa255;
            let coef2 = 255 * (1 << PRECISION_BITS) - coef1;
            for c in 0..3 {
                let tmp = u32::from(s[c]) * coef1 + u32::from(d[c]) * coef2;
                o[c] = (shift_for_div255(tmp + (0x80 << PRECISION_BITS)) >> PRECISION_BITS) as u8;
            }
            o[3] = shift_for_div255(outa255 + 0x80) as u8;
        }
    }
    Ok(Rgba {
        width: dst.width,
        height: dst.height,
        rgba: out,
    })
}

// ---------------------------------------------------------------------------------------------
// BoxBlur.c
// ---------------------------------------------------------------------------------------------

/// `_gaussian_blur_radius` : rayon de la boîte étendue équivalente (Gwosdek et al., 2011).
/// `sigma2`, `l` et `a` sont des `float` ; `sqrt`, `floor` et leurs constantes en `double`.
fn gaussian_blur_radius(radius: f32, passes: u32) -> f32 {
    let sigma2: f32 = radius * radius / passes as f32;
    let big_l: f32 = (12.0 * f64::from(sigma2) + 1.0).sqrt() as f32;
    let l: f32 = ((f64::from(big_l) - 1.0) / 2.0).floor() as f32;
    let mut a: f32 = (2.0 * l + 1.0) * (l * (l + 1.0) - 3.0 * sigma2);
    a /= 6.0 * (sigma2 - (l + 1.0) * (l + 1.0));
    l + a
}

/// Poids d'une boîte de rayon `float_radius` (`ImagingHorizontalBoxBlur`) : rayon entier,
/// poids `ww` des pixels entiers et `fw` des deux pixels partiels, sur 2²⁴.
struct BoxWeights {
    radius: i32,
    ww: u32,
    fw: u32,
}

impl BoxWeights {
    fn new(float_radius: f32) -> BoxWeights {
        let radius = float_radius as i32;
        // (UINT32)(1 << 24) / (floatRadius * 2 + 1) : division en float, troncature
        let ww = ((1u32 << 24) as f32 / (float_radius * 2.0 + 1.0)) as u32;
        let fw = (1u32 << 24).wrapping_sub(((radius * 2 + 1) as u32).wrapping_mul(ww)) / 2;
        BoxWeights { radius, ww, fw }
    }
}

/// `ImagingLineBoxBlur32` : une ligne de `n` pixels RGBA (`line_in`) floutée dans `line_out`.
/// Arithmétique `UINT32` modulo 2³² comme en C.
fn line_box_blur(line_out: &mut [[u8; 4]], line_in: &[[u8; 4]], b: &BoxWeights) {
    let n = line_in.len() as i32;
    let lastx = n - 1;
    let radius = b.radius;
    let edge_a = (radius + 1).min(n);
    let edge_b = (n - radius - 1).max(0);
    let (ww, fw) = (b.ww, b.fw);
    let px = |i: i32| -> &[u8; 4] { &line_in[i as usize] };

    let mut acc = [0u32; 4];
    for (c, v) in acc.iter_mut().enumerate() {
        *v = (i32::from(px(0)[c]) * (radius + 1)) as u32;
    }
    for x in 0..edge_a - 1 {
        for (c, v) in acc.iter_mut().enumerate() {
            *v = v.wrapping_add(u32::from(px(x)[c]));
        }
    }
    for (c, v) in acc.iter_mut().enumerate() {
        *v = v.wrapping_add((i32::from(px(lastx)[c]) * (radius - edge_a + 1)) as u32);
    }

    let mut step = |x: i32, subtract: i32, add: i32, left: i32, right: i32| {
        let out = &mut line_out[x as usize];
        for c in 0..4 {
            let delta = i32::from(px(add)[c]) - i32::from(px(subtract)[c]);
            acc[c] = acc[c].wrapping_add(delta as u32);
            let bulk = acc[c]
                .wrapping_mul(ww)
                .wrapping_add((u32::from(px(left)[c]) + u32::from(px(right)[c])).wrapping_mul(fw));
            out[c] = (bulk.wrapping_add(1 << 23) >> 24) as u8;
        }
    };

    if edge_a <= edge_b {
        for x in 0..edge_a {
            step(x, 0, x + radius, 0, x + radius + 1);
        }
        for x in edge_a..edge_b {
            step(
                x,
                x - radius - 1,
                x + radius,
                x - radius - 1,
                x + radius + 1,
            );
        }
        for x in edge_b..=lastx {
            step(x, x - radius - 1, lastx, x - radius - 1, lastx);
        }
    } else {
        for x in 0..edge_b {
            step(x, 0, x + radius, 0, x + radius + 1);
        }
        for x in edge_b..edge_a {
            step(x, 0, lastx, 0, lastx);
        }
        for x in edge_a..=lastx {
            step(x, x - radius - 1, lastx, x - radius - 1, lastx);
        }
    }
}

/// `n` passes de `ImagingHorizontalBoxBlur` sur chaque ligne de `px` (`w × h` pixels).
fn horizontal_passes(
    px: &mut [[u8; 4]],
    w: usize,
    b: &BoxWeights,
    passes: u32,
    cancelled: &dyn Fn() -> bool,
) -> Result<()> {
    let mut tmp = Vec::new();
    tmp.try_reserve_exact(w).map_err(|_| NO_MEMORY)?;
    tmp.resize(w, [0u8; 4]);
    for line in px.chunks_exact_mut(w) {
        check(cancelled)?;
        for _ in 0..passes {
            line_box_blur(&mut tmp, line, b);
            line.copy_from_slice(&tmp);
        }
    }
    Ok(())
}

/// `ImagingTranspose` : `w × h` vers `h × w`.
fn transpose(src: &[[u8; 4]], w: usize, h: usize, dst: &mut [[u8; 4]]) {
    for y in 0..h {
        for x in 0..w {
            dst[x * h + y] = src[y * w + x];
        }
    }
}

/// `Image.filter(ImageFilter.GaussianBlur(radius))` sur une image `RGBA` (rayon identique en
/// x et en y, 3 passes). Rayon nul : image inchangée (`image.copy()`).
pub fn gaussian_blur(img: Rgba, radius: f32, cancelled: &dyn Fn() -> bool) -> Result<Rgba> {
    let n = checked_len(&img)?;
    let (w, h) = (img.width as usize, img.height as usize);
    if radius == 0.0 || w == 0 || h == 0 {
        return Ok(img);
    }
    if radius < 0.0 || radius.is_nan() {
        return Err(PngError::Unreadable("rayon de flou négatif"));
    }
    let b = BoxWeights::new(gaussian_blur_radius(radius, PASSES));
    let mut rgba = img.rgba;
    let px = rgba.as_chunks_mut::<4>().0;
    // xradius != 0 : passes horizontales
    horizontal_passes(px, w, &b, PASSES, cancelled)?;
    // yradius != 0 : transposition, passes horizontales, transposition inverse
    let mut t = alloc(n)?;
    let tp = t.as_chunks_mut::<4>().0;
    transpose(px, w, h, tp);
    horizontal_passes(tp, h, &b, PASSES, cancelled)?;
    transpose(tp, h, w, px);
    Ok(Rgba {
        width: img.width,
        height: img.height,
        rgba,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn never() -> bool {
        false
    }

    #[test]
    fn rayon_de_boite_de_pillow() {
        // _gaussian_blur_radius(8, 3) : l = 7, a = 15 × (56 − 64) / (6 × (64/3 − 64))
        let r = gaussian_blur_radius(8.0, 3);
        assert!((r - 7.46875).abs() < 1e-4, "{r}");
        let b = BoxWeights::new(r);
        assert_eq!(b.radius, 7);
        assert_eq!(b.fw, ((1 << 24) - 15 * b.ww) / 2);
    }

    #[test]
    fn couleur_unie_reste_unie() {
        for (w, h) in [(1, 1), (3, 2), (104, 136), (20, 5)] {
            let img = Rgba {
                width: w,
                height: h,
                rgba: [200u8, 10, 99, 255].repeat((w * h) as usize),
            };
            let out = gaussian_blur(img.clone(), 8.0, &never).unwrap();
            assert_eq!(out, img);
        }
    }

    #[test]
    fn composition_sur_fond_opaque() {
        let dst = Rgba {
            width: 2,
            height: 1,
            rgba: vec![128, 128, 128, 255, 128, 128, 128, 255],
        };
        let src = Rgba {
            width: 2,
            height: 1,
            rgba: vec![9, 9, 9, 0, 255, 0, 0, 255],
        };
        let out = alpha_composite(&dst, &src, &never).unwrap();
        assert_eq!(out.rgba, vec![128, 128, 128, 255, 255, 0, 0, 255]);
    }
}
