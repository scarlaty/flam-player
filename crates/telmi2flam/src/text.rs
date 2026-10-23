//! Textes affichés par le player (titre, sous-titre, labels de choix, inventaire) : `_flat`,
//! `_font_safe`, `_text`, `_choice_label`, `_describe_replaced` et `LABEL_MAX` de
//! `tools/telmi2flam/telmi2flam.py`.
//!
//! Les données Unicode sont celles du Python de l'installateur (3.14, Unicode 16.0.0) :
//! - espaces de `str.split()` / `str.isspace()` (dont `\x1c`-`\x1f` et U+0085, absents de
//!   `char::is_whitespace`) et catégories générales L* / N* : tables générées depuis Python
//!   (`text_tables.rs`) ;
//! - NFC, NFKD et classe combinatoire (`unicodedata.combining`) : `unicode-normalization`,
//!   épinglée sur la version qui porte la même version d'Unicode (vérifié par un test).
//!
//! Divergence de texte assumée : `_describe_replaced` cite `unicodedata.name` ; le Rust omet
//! le nom (sa table pèse plusieurs centaines de Ko). Seul le message de l'avertissement
//! `font_replaced` change : son code, que compare la parité (§4.1), est le même.

use std::collections::BTreeMap;

use unicode_normalization::char::canonical_combining_class;
use unicode_normalization::UnicodeNormalization;

use crate::json::Value;
use crate::text_tables::{LETTER_OR_NUMBER, NON_PRINTABLE, PY_WHITESPACE};

pub use crate::text_tables::UNIDATA_VERSION;

/// Longueur maximale (en points de code) du label d'un choix (carrousel).
pub const LABEL_MAX: usize = 40;

/// Caractères remplacés par [`font_safe`] : `{caractère: remplacement}`, triés par point de
/// code comme `sorted(replaced)` de Python.
pub type Replaced = BTreeMap<char, String>;

/// `c.isspace()` de Python.
pub fn is_py_space(c: char) -> bool {
    PY_WHITESPACE.binary_search(&c).is_ok()
}

/// `unicodedata.category(c)[0] in "LN"`.
fn is_letter_or_number(c: char) -> bool {
    let cp = c as u32;
    LETTER_OR_NUMBER
        .binary_search_by(|&(lo, hi)| {
            if hi < cp {
                std::cmp::Ordering::Less
            } else if lo > cp {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// `" ".join(s.split())` : aplatit une chaîne sur une ligne.
pub fn flat(s: &str) -> String {
    s.split(is_py_space)
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// `s.strip()` de Python.
pub fn py_strip(s: &str) -> &str {
    s.trim_matches(is_py_space)
}

/// Caractères couverts par les polices du player (ASCII imprimable et Latin-1).
fn in_font(c: char) -> bool {
    matches!(c as u32, 0x20..=0x7E | 0xA0..=0xFF)
}

/// Remplacements typographiques usuels (`_FONT_MAP`).
fn font_map(c: char) -> Option<&'static str> {
    Some(match c {
        '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{201b}' | '\u{2032}' => "'",
        '\u{2039}' => "<",
        '\u{203a}' => ">",
        '\u{201c}' | '\u{201d}' | '\u{201e}' | '\u{201f}' | '\u{2033}' => "\"",
        '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2015}'
        | '\u{2212}' | '\u{2022}' => "-",
        '\u{2026}' => "...",
        '\u{0153}' => "oe",
        '\u{0152}' => "OE",
        '\u{20ac}' => "EUR",
        '\u{2122}' => "TM",
        _ => return None,
    })
}

/// Remplacement d'un caractère hors police (branche `rep is None` de `_font_safe`).
fn fallback(ch: char) -> String {
    let base: String = std::iter::once(ch)
        .nfkd()
        .filter(|&c| canonical_combining_class(c) == 0)
        .collect();
    if !base.is_empty() && base.chars().all(in_font) {
        base // ex. "o" double aigu -> "o", ligature fi -> "fi"
    } else if is_letter_or_number(ch) {
        "?".to_owned() // lettre ou chiffre sans équivalent
    } else {
        String::new() // emoji, symbole, contrôle, invisible : supprimé
    }
}

/// Chaîne affichable par les polices du player (`_font_safe`) : NFC, puis chaque caractère
/// hors police est remplacé ; les espaces Unicode restent (ramenés à " " par [`flat`]).
/// `replaced` reçoit `{caractère: remplacement}` pour l'avertissement `font_replaced`.
pub fn font_safe(s: &str, mut replaced: Option<&mut Replaced>) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.nfc() {
        if in_font(ch) || is_py_space(ch) {
            out.push(ch);
            continue;
        }
        let rep = match font_map(ch) {
            Some(r) => r.to_owned(),
            None => fallback(ch),
        };
        out.push_str(&rep);
        if let Some(map) = replaced.as_deref_mut() {
            map.insert(ch, rep);
        }
    }
    out
}

/// Texte affiché (`_text`) : compatible avec la police et sur une ligne.
pub fn text(s: &str, replaced: Option<&mut Replaced>) -> String {
    flat(&font_safe(s, replaced))
}

/// Label court d'un choix (`_choice_label`) : `title`, sinon `text`, sinon `notes` ; tronqué
/// à [`LABEL_MAX`] points de code. Un texte égal à l'id du nœud est ignoré (titre par défaut
/// de l'éditeur TELMI).
pub fn choice_label(
    note: &Value,
    node_id: Option<&str>,
    mut replaced: Option<&mut Replaced>,
) -> String {
    let Value::Object(map) = note else {
        return String::new();
    };
    for key in ["title", "text", "notes"] {
        let Some(Value::Str(v)) = map.get(key) else {
            continue;
        };
        if py_strip(v).is_empty() {
            continue;
        }
        let label = flat(v);
        if node_id.is_some_and(|id| label == flat(id)) {
            continue;
        }
        let label = text(&label, replaced.as_deref_mut());
        if label.is_empty() {
            continue;
        }
        if label.chars().count() > LABEL_MAX {
            let cut: String = label.chars().take(LABEL_MAX - 3).collect();
            return format!("{}...", cut.trim_end_matches(is_py_space));
        }
        return label;
    }
    String::new()
}

/// `str.isprintable()` pour un caractère (table `NON_PRINTABLE` : catégories Cc, Cf, Cs, Co,
/// Cn, Zl, Zp et Zs sauf l'espace).
pub fn is_py_printable(c: char) -> bool {
    let cp = c as u32;
    NON_PRINTABLE
        .binary_search_by(|&(lo, hi)| {
            if hi < cp {
                std::cmp::Ordering::Less
            } else if lo > cp {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_err()
}

/// Cœur commun de `repr(str)` et `ascii(str)` : choix des guillemets (apostrophes, sauf si
/// le texte en contient sans contenir de guillemets doubles), échappements `\\`, `\t`, `\n`,
/// `\r`, puis `\xNN`, `\uNNNN` ou `\UNNNNNNNN` pour ce que `keep` ne laisse pas tel quel.
fn py_quote(s: &str, keep: impl Fn(char) -> bool) -> String {
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
    for c in s.chars() {
        let cp = c as u32;
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ if c == quote => {
                out.push('\\');
                out.push(c);
            }
            _ if keep(c) => out.push(c),
            _ if cp <= 0xff => out.push_str(&format!("\\x{cp:02x}")),
            _ if cp <= 0xffff => out.push_str(&format!("\\u{cp:04x}")),
            _ => out.push_str(&format!("\\U{cp:08x}")),
        }
    }
    out.push(quote);
    out
}

/// `ascii(s)` de Python : représentation entre apostrophes, non-ASCII échappé.
pub fn py_ascii(s: &str) -> String {
    py_quote(s, |c| (' '..='~').contains(&c))
}

/// `repr(s)` de Python pour une `str` (le `%r` des messages de `telmi2flam.py`).
pub fn py_repr(s: &str) -> String {
    py_quote(s, is_py_printable)
}

/// Liste ASCII des caractères remplacés (`_describe_replaced`), sans le nom Unicode
/// (divergence de texte assumée, voir l'en-tête du module).
pub fn describe_replaced(replaced: &Replaced, limit: usize) -> String {
    let items: Vec<String> = replaced
        .iter()
        .map(|(&ch, rep)| {
            let rep = if rep.is_empty() {
                "(supprime)".to_owned()
            } else {
                py_ascii(rep)
            };
            format!("U+{:04X} -> {rep}", ch as u32)
        })
        .collect();
    let more = if items.len() > limit { " ..." } else { "" };
    let shown = &items[..items.len().min(limit)];
    format!("{}{more}", shown.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_unicode_identique_a_unicode_normalization() {
        let (a, b, c) = unicode_normalization::UNICODE_VERSION;
        assert_eq!(
            UNIDATA_VERSION,
            format!("{a}.{b}.{c}"),
            "tables Python et unicode-normalization sur des versions d'Unicode différentes"
        );
    }

    #[test]
    fn tables_triees() {
        assert!(PY_WHITESPACE.windows(2).all(|w| w[0] < w[1]));
        assert!(LETTER_OR_NUMBER.windows(2).all(|w| w[0].1 < w[1].0));
        assert!(LETTER_OR_NUMBER.iter().all(|&(a, b)| a <= b));
    }

    #[test]
    fn espaces_de_python() {
        assert_eq!(flat(" a\u{1c}b\u{85}c\u{200b}d "), "a b c\u{200b}d");
        assert_eq!(font_safe("\u{3a3}\u{3c2}\u{130}", None), "??I");
    }
}
