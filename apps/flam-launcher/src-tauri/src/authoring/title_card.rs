//! Image générée avec le titre (`docs/creation-dossier.md` § 7, « Image d'une histoire ou d'un
//! chapitre », troisième complément).
//!
//! Titre en blanc, centré et coupé sur plusieurs lignes, sur une couleur dérivée du titre.
//!
//! # Police
//!
//! La police du player : Nunito ExtraBold 20 px (`src/fonts/nunito_extrabold_20.c` à la racine
//! du dépôt, licence SIL OFL 1.1, voir `third-party/OFL-Nunito.txt`). Ce fichier C de LVGL est
//! **inclus tel quel** dans le binaire (`include_str!`) et lu au premier usage : aucune copie
//! des données, aucune dépendance de rendu de texte. Ce corps est le seul qu'ait le player :
//! le texte est composé à 20 px puis la couche de couverture est agrandie (Lanczos) à la taille
//! de l'image. Jeu de caractères : U+0020 à U+007F et U+00A0 à U+00FF. Pas de crénage.
//!
//! # Texte
//!
//! [`sanitize`] ramène le titre au jeu de la police : NFC, espaces réduites, ponctuation
//! typographique ramenée à l'ASCII, accents hors Latin-1 retirés (`Š` devient `S`), ligatures
//! développées (`œ` devient `oe`). Tout le reste (emoji, autres écritures, caractères de
//! contrôle, sélecteurs de variante) est **supprimé**. Un titre vide après nettoyage ne donne
//! pas d'image ([`render`] renvoie `Ok(None)`) : l'appelant passe au complément suivant.
//!
//! # Couleur
//!
//! [`title_color`] : teinte tirée de SHA-256 du titre, saturation fixe, clarté abaissée jusqu'à
//! un contraste d'au moins 4,5 : 1 avec le blanc (WCAG AA). Même titre, même couleur.

use std::sync::OnceLock;

use image::imageops::{self, FilterType};
use image::{GrayImage, Luma, Rgb, RgbImage};
use sha2::{Digest, Sha256};
use unicode_normalization::char::{canonical_combining_class, decompose_compatible};
use unicode_normalization::UnicodeNormalization;

use crate::podcast::artwork::{encode_png, Artwork, ArtworkError, ArtworkKind};

/// Source de la police du player (OFL).
const FONT_SOURCE: &str = include_str!("../../../../../src/fonts/nunito_extrabold_20.c");
/// Facteur d'agrandissement de la couche de texte (20 px devient environ 60 px).
const SCALE: u32 = 3;
/// Marge autour du texte, en pixels de police (avant agrandissement).
const MARGIN: i32 = 8;
/// Contraste minimal avec le blanc.
const MIN_CONTRAST: f64 = 4.5;
/// Nombre de caractères du titre pris en compte (borne de travail).
const MAX_TITLE_CHARS: usize = 400;

/// Glyphe de la police.
#[derive(Debug, Clone, Copy)]
struct Glyph {
    bitmap_index: usize,
    /// Avance, en seizièmes de pixel.
    adv_w: i32,
    box_w: i32,
    box_h: i32,
    ofs_x: i32,
    ofs_y: i32,
}

/// Plage Unicode contiguë de la police.
#[derive(Debug, Clone, Copy)]
struct Range {
    start: u32,
    len: u32,
    first_glyph: usize,
}

/// Police LVGL décodée.
struct Font {
    bitmap: Vec<u8>,
    glyphs: Vec<Glyph>,
    ranges: Vec<Range>,
    line_height: i32,
    base_line: i32,
}

/// Valeur entière après `key =` dans `line`.
fn field(line: &str, key: &str) -> Option<i32> {
    let rest = &line[line.find(key)? + key.len()..];
    let rest = rest.trim_start().strip_prefix('=')?.trim_start();
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '-'))
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

impl Font {
    /// Décode le fichier C généré par `lv_font_conv` (4 bits par pixel, sans compression).
    fn parse(src: &str) -> Option<Self> {
        // Bitmaps : octets hexadécimaux entre `glyph_bitmap[] = {` et `};`, commentaires exclus.
        let start = src.find("glyph_bitmap[] = {")? + "glyph_bitmap[] = {".len();
        let end = start + src[start..].find("};")?;
        let mut bitmap = Vec::new();
        let mut in_comment = false;
        for token in src[start..end].split_whitespace() {
            if in_comment {
                in_comment = token != "*/";
                continue;
            }
            if token == "/*" {
                in_comment = true;
                continue;
            }
            if let Some(hex) = token.trim_end_matches(',').strip_prefix("0x") {
                bitmap.push(u8::from_str_radix(hex, 16).ok()?);
            }
        }
        // Glyphes, dans l'ordre (identifiant 0 réservé compris).
        let start = src.find("glyph_dsc[] = {")?;
        let end = start + src[start..].find("};")?;
        let mut glyphs = Vec::new();
        for line in src[start..end].lines().filter(|l| l.contains(".adv_w")) {
            glyphs.push(Glyph {
                bitmap_index: usize::try_from(field(line, ".bitmap_index")?).ok()?,
                adv_w: field(line, ".adv_w")?,
                box_w: field(line, ".box_w")?,
                box_h: field(line, ".box_h")?,
                ofs_x: field(line, ".ofs_x")?,
                ofs_y: field(line, ".ofs_y")?,
            });
        }
        // Plages (format « tiny » : code points et identifiants consécutifs).
        let mut ranges = Vec::new();
        for line in src.lines().filter(|l| l.contains(".range_start")) {
            ranges.push(Range {
                start: u32::try_from(field(line, ".range_start")?).ok()?,
                len: u32::try_from(field(line, ".range_length")?).ok()?,
                first_glyph: usize::try_from(field(line, ".glyph_id_start")?).ok()?,
            });
        }
        let font = Self {
            bitmap,
            glyphs,
            ranges,
            line_height: src.lines().find_map(|l| field(l, ".line_height"))?,
            base_line: src.lines().find_map(|l| field(l, ".base_line"))?,
        };
        // Cohérence : aucun glyphe ne déborde de la table des bitmaps.
        let sound = font.line_height > 0
            && !font.ranges.is_empty()
            && font.glyphs.iter().all(|g| {
                let pixels = usize::try_from(g.box_w * g.box_h).unwrap_or(usize::MAX);
                g.bitmap_index.saturating_add(pixels.div_ceil(2)) <= font.bitmap.len()
            });
        sound.then_some(font)
    }

    fn glyph(&self, c: char) -> Option<&Glyph> {
        let cp = u32::from(c);
        self.ranges
            .iter()
            .find(|r| cp >= r.start && cp - r.start < r.len)
            .and_then(|r| self.glyphs.get(r.first_glyph + (cp - r.start) as usize))
    }

    /// Avance de `c`, en seizièmes de pixel (0 si absent de la police).
    fn advance(&self, c: char) -> i32 {
        self.glyph(c).map_or(0, |g| g.adv_w)
    }

    fn width(&self, s: &str) -> i32 {
        s.chars().map(|c| self.advance(c)).sum()
    }

    /// Dessine `s` dans `mask` : `x16` est l'abscisse du début en seizièmes de pixel, `top` le
    /// haut de la ligne.
    fn draw(&self, mask: &mut GrayImage, s: &str, x16: i32, top: i32) {
        let baseline = top + self.line_height - self.base_line;
        let mut pen = x16;
        for c in s.chars() {
            let Some(g) = self.glyph(c) else { continue };
            let gx = (pen + 8).div_euclid(16) + g.ofs_x;
            let gy = baseline - g.ofs_y - g.box_h;
            for row in 0..g.box_h {
                for col in 0..g.box_w {
                    let i = (row * g.box_w + col) as usize;
                    let byte = self.bitmap[g.bitmap_index + i / 2];
                    let level = if i.is_multiple_of(2) {
                        byte >> 4
                    } else {
                        byte & 0xf
                    };
                    let (px, py) = (gx + col, gy + row);
                    if level == 0 || px < 0 || py < 0 {
                        continue;
                    }
                    let (px, py) = (px as u32, py as u32);
                    if px < mask.width() && py < mask.height() {
                        let cur = mask.get_pixel(px, py).0[0];
                        mask.put_pixel(px, py, Luma([cur.max(level * 17)]));
                    }
                }
            }
            pen += g.adv_w;
        }
    }
}

fn font() -> Option<&'static Font> {
    static FONT: OnceLock<Option<Font>> = OnceLock::new();
    FONT.get_or_init(|| Font::parse(FONT_SOURCE)).as_ref()
}

/// Remplacement d'un caractère hors de la police : ASCII, ou rien.
fn substitute(c: char) -> Option<&'static str> {
    Some(match c {
        '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{2032}' => "'",
        '\u{201c}' | '\u{201d}' | '\u{201e}' | '\u{2033}' => "\"",
        '\u{2010}'..='\u{2015}' | '\u{2212}' => "-",
        '\u{2026}' => "...",
        '\u{0153}' => "oe",
        '\u{0152}' => "OE",
        '\u{0142}' => "l",
        '\u{0141}' => "L",
        '\u{0111}' => "d",
        '\u{0110}' => "D",
        '\u{1e9e}' => "SS",
        _ => return None,
    })
}

/// Ramène `title` au jeu de caractères de la police (voir l'en-tête du module). Les espaces
/// sont réduites et les extrémités rognées.
pub fn sanitize(title: &str) -> String {
    let Some(font) = font() else {
        return String::new();
    };
    // Ajoute `s` à `out` : espaces réduites, caractères hors police supprimés.
    fn push(font: &Font, s: &str, out: &mut String) {
        for c in s.chars() {
            if c.is_whitespace() || c.is_control() {
                if !out.is_empty() && !out.ends_with(' ') {
                    out.push(' ');
                }
            } else if font.glyph(c).is_some() {
                out.push(c);
            }
        }
    }
    let mut out = String::new();
    for c in title.chars().take(MAX_TITLE_CHARS).nfc() {
        if c.is_whitespace() || c.is_control() {
            push(font, " ", &mut out);
        } else if font.glyph(c).is_some() {
            out.push(c);
        } else if let Some(s) = substitute(c) {
            push(font, s, &mut out);
        } else {
            // Accent hors Latin-1 : décomposition de compatibilité, marques retirées.
            let mut base = String::new();
            decompose_compatible(c, |d| {
                if canonical_combining_class(d) == 0 {
                    base.push(d);
                }
            });
            push(font, &base, &mut out);
        }
    }
    out.trim().to_string()
}

/// Coupe `text` (déjà nettoyé) en lignes d'au plus `max_w16` seizièmes de pixel et `max_lines`
/// lignes : à l'espace, ou au caractère pour un mot plus large qu'une ligne. Si le titre ne
/// tient pas, la dernière ligne finit par `...`.
fn wrap(font: &Font, text: &str, max_w16: i32, max_lines: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut truncated = false;
    'words: for word in text.split(' ').filter(|w| !w.is_empty()) {
        let mut word = word.to_string();
        loop {
            let candidate = if cur.is_empty() {
                word.clone()
            } else {
                format!("{cur} {word}")
            };
            if font.width(&candidate) <= max_w16 {
                cur = candidate;
                continue 'words;
            }
            if lines.len() + 1 >= max_lines {
                // Dernière ligne disponible : le reste est abandonné.
                if cur.is_empty() {
                    cur = word;
                }
                truncated = true;
                break 'words;
            }
            if !cur.is_empty() {
                lines.push(std::mem::take(&mut cur));
                continue;
            }
            // Mot seul trop large : plus long préfixe qui tient (au moins un caractère).
            let mut head = String::new();
            for c in word.chars() {
                let mut next = head.clone();
                next.push(c);
                if !head.is_empty() && font.width(&next) > max_w16 {
                    break;
                }
                head = next;
            }
            word = word[head.len()..].to_string();
            lines.push(head);
            if word.is_empty() {
                continue 'words;
            }
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    if truncated {
        if let Some(last) = lines.last_mut() {
            while !last.is_empty() && font.width(&format!("{last}...")) > max_w16 {
                last.pop();
            }
            *last = format!("{}...", last.trim_end());
        }
    }
    lines
}

/// Lignes du titre telles que [`render`] les dessine pour `kind` (nettoyage, coupure). Vide si
/// le titre ne contient rien d'affichable.
pub fn layout(title: &str, kind: ArtworkKind) -> Vec<String> {
    let Some(font) = font() else {
        return Vec::new();
    };
    let text = sanitize(title);
    if text.is_empty() {
        return Vec::new();
    }
    let (cw, ch) = canvas_size(kind);
    let max_w16 = (cw - 2 * MARGIN) * 16;
    let max_lines = ((ch - 2 * MARGIN) / font.line_height).max(1) as usize;
    wrap(font, &text, max_w16, max_lines)
}

fn canvas_size(kind: ArtworkKind) -> (i32, i32) {
    let (w, h) = kind.size();
    (w.div_ceil(SCALE) as i32, h.div_ceil(SCALE) as i32)
}

/// Couleur de fond dérivée du titre : déterministe, contraste d'au moins 4,5 : 1 avec le blanc.
pub fn title_color(title: &str) -> [u8; 3] {
    let digest = Sha256::digest(title.trim().as_bytes());
    let hue = f64::from(u16::from_be_bytes([digest[0], digest[1]]) % 360);
    let mut light = 0.40;
    loop {
        let rgb = hsl_to_rgb(hue, 0.55, light);
        if contrast_with_white(rgb) >= MIN_CONTRAST || light <= 0.05 {
            return rgb;
        }
        light -= 0.01;
    }
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> [u8; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let q = |v: f64| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    [q(r), q(g), q(b)]
}

/// Contraste WCAG entre `rgb` et le blanc.
pub fn contrast_with_white(rgb: [u8; 3]) -> f64 {
    let lin = |v: u8| {
        let v = f64::from(v) / 255.0;
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let lum = 0.2126 * lin(rgb[0]) + 0.7152 * lin(rgb[1]) + 0.0722 * lin(rgb[2]);
    1.05 / (lum + 0.05)
}

/// Image du titre aux dimensions de `kind` (PNG). `Ok(None)` si le titre ne contient rien
/// d'affichable (vide, emoji seuls...) ou si la police est illisible : l'appelant passe au
/// complément suivant.
pub fn render(title: &str, kind: ArtworkKind) -> Result<Option<Artwork>, ArtworkError> {
    let Some(font) = font() else {
        return Ok(None);
    };
    let lines = layout(title, kind);
    if lines.is_empty() {
        return Ok(None);
    }
    let (w, h) = kind.size();
    let (cw, ch) = canvas_size(kind);
    let mut mask = GrayImage::new(cw as u32, ch as u32);
    let block = font.line_height * lines.len() as i32;
    let top = (ch - block).div_euclid(2);
    for (i, line) in lines.iter().enumerate() {
        let x16 = ((cw * 16 - font.width(line)) / 2).max(0);
        font.draw(&mut mask, line, x16, top + font.line_height * i as i32);
    }
    let mask = imageops::resize(&mask, w, h, FilterType::Lanczos3);
    let bg = title_color(title);
    let img = RgbImage::from_fn(w, h, |x, y| {
        let a = u32::from(mask.get_pixel(x, y).0[0]);
        let mix = |b: u8| ((u32::from(b) * (255 - a) + 255 * a + 127) / 255) as u8;
        Rgb([mix(bg[0]), mix(bg[1]), mix(bg[2])])
    });
    Ok(Some(Artwork {
        width: w,
        height: h,
        png: encode_png(&img)?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn police_du_player_decodee() {
        let f = font().expect("police lisible");
        assert_eq!(f.line_height, 26);
        assert!(f.glyph('A').is_some() && f.glyph('é').is_some());
        assert!(f.glyph('€').is_none() && f.glyph('😀').is_none());
        assert!(f.advance('W') > f.advance('i'));
    }

    #[test]
    fn couleur_deterministe_et_contrastee() {
        assert_eq!(
            title_color("Le petit prince"),
            title_color("  Le petit prince ")
        );
        assert_ne!(
            title_color("Le petit prince"),
            title_color("Le grand prince")
        );
        for i in 0..500 {
            let c = title_color(&format!("histoire {i}"));
            assert!(contrast_with_white(c) >= MIN_CONTRAST, "{i} {c:?}");
        }
    }

    #[test]
    fn nettoyage_du_titre() {
        assert_eq!(sanitize("  Été \t au\n bord  "), "Été au bord");
        // NFD recomposé, accents hors Latin-1 retirés, ligatures, typographie.
        assert_eq!(sanitize("E\u{301}te\u{301}"), "Été");
        assert_eq!(sanitize("Šárka et Łódź"), "Sárka et Lódz");
        assert_eq!(sanitize("Cœur d’or… — « oui »"), "Coeur d'or... - « oui »");
        // Emoji, autres écritures et contrôles supprimés.
        assert_eq!(sanitize("Fête 🎉🎂 des 👨‍👩‍👧 lutins\u{0}"), "Fête des lutins");
        assert_eq!(sanitize("🎉🎂"), "");
        assert_eq!(sanitize("日本語"), "");
    }

    #[test]
    fn decoupage_court_long_et_sans_espace() {
        let f = font().unwrap();
        let max = 184 * 16;
        assert_eq!(wrap(f, "Court", max, 5), ["Court"]);
        let long = "Les aventures extraordinaires du petit renard et de la grande chouette";
        let lines = wrap(f, long, max, 5);
        assert!(lines.len() >= 2);
        assert!(lines.iter().all(|l| f.width(l) <= max), "{lines:?}");
        assert_eq!(lines.join(" "), long, "aucun mot perdu");
        // Mot plus large qu'une ligne : coupé au caractère.
        let word = "Anticonstitutionnellement".repeat(3);
        let lines = wrap(f, &word, max, 8);
        assert!(lines.len() > 1 && lines.iter().all(|l| f.width(l) <= max));
        assert_eq!(lines.concat(), word);
        // Trop long : tronqué avec points de suspension, sans dépasser.
        let huge = "mot ".repeat(200);
        let lines = wrap(f, huge.trim(), max, 5);
        assert_eq!(lines.len(), 5);
        assert!(lines[4].ends_with("..."));
        assert!(lines.iter().all(|l| f.width(l) <= max), "{lines:?}");
    }

    #[test]
    fn mise_en_page_accents_et_emoji() {
        let lines = layout("Où est passée la sorcière 🧙‍♀️ ?", ArtworkKind::Episode);
        assert!(!lines.is_empty());
        let all = lines.join(" ");
        assert!(all.contains("Où") && all.contains("sorcière") && !all.contains('🧙'));
        assert!(layout("🎉", ArtworkKind::Cover).is_empty());
    }

    #[test]
    fn image_aux_dimensions_et_avec_du_texte() {
        for kind in [ArtworkKind::Episode, ArtworkKind::Cover] {
            let art = render("Le petit chaperon rouge", kind).unwrap().unwrap();
            assert_eq!((art.width, art.height), kind.size());
            let img = image::load_from_memory(&art.png).unwrap().to_rgb8();
            assert_eq!(img.dimensions(), kind.size());
            let bg = title_color("Le petit chaperon rouge");
            assert_eq!(img.get_pixel(1, 1).0, bg, "fond sans texte dans le coin");
            let light = img
                .pixels()
                .filter(|p| p.0[0] > 235 && p.0[2] > 235)
                .count();
            assert!(light > 200, "du texte blanc est dessiné ({light})");
            assert_eq!(
                art,
                render("Le petit chaperon rouge", kind).unwrap().unwrap()
            );
        }
        assert!(render("   ", ArtworkKind::Episode).unwrap().is_none());
    }
}
