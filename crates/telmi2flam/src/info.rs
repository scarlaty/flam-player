//! `read_info` et sortie de `--info` : métadonnées de l'histoire, sans conversion, en JSON
//! identique à `json.dumps(read_info(...), ensure_ascii=True, separators=(",", ":"))`.
//!
//! Le décodage de `metadata.json` est celui de [`crate::json::load_json`] (`load_json` de
//! `telmi2flam.py`) et ses cas limites tranchés au §3 de `PORT-RUST.md` : flottant hors
//! plage, entier hors 64 bits, imbrication de plus de [`crate::json::MAX_DEPTH`] niveaux et
//! surrogate isolé donnent `invalid_content` (code 4).

use std::io::{Read, Seek};
use std::path::Path;

use crate::archive::{find_root, fold, Archive};
use crate::error::ConvError;
use crate::json::{expect, load_json, Kind, Value};
use crate::progress::json_str;

/// Valeur scalaire de `read_info` (`str`, `int` ou `float`, jamais `bool`), sinon `null`.
#[derive(Debug, Clone, PartialEq)]
pub enum Scalar {
    Str(String),
    Int(i128),
    Float(f64),
}

/// Résultat de `read_info`, dans l'ordre des clés de Python.
#[derive(Debug, Clone, PartialEq)]
pub struct Info {
    pub title: Option<Scalar>,
    pub uuid: Option<Scalar>,
    pub version: Option<Scalar>,
    pub age: Option<Scalar>,
    pub category: Option<Scalar>,
    pub description: Option<Scalar>,
    pub cover: bool,
}

/// `read_info` sur un fichier : métadonnées de l'histoire, sans conversion.
pub fn read_info(path: &Path) -> Result<Info, ConvError> {
    read_info_from(&mut Archive::open(path)?)
}

/// `read_info` sur une archive déjà ouverte.
pub fn read_info_from<R: Read + Seek>(archive: &mut Archive<R>) -> Result<Info, ConvError> {
    let root = find_root(archive.names())?;
    let raw = archive
        .read(&format!("{root}metadata.json"))?
        .ok_or_else(|| ConvError::archive_unreadable("metadata.json introuvable"))?;
    let metadata = load_json(&raw, "metadata.json")?;
    expect(Some(&metadata), &[Kind::Object], "metadata.json", false)?;
    let get = |key: &str| metadata.get(key);
    let cover = match get("image") {
        Some(Value::Str(s)) if !s.is_empty() => Some(s.as_str()),
        _ => None,
    };
    let has_cover = cover
        .into_iter()
        .chain(["title.png", "cover.png"])
        .any(|c| archive.contains_folded(&fold(&format!("{root}{c}"))));
    let scalar = |key: &str| match get(key) {
        Some(Value::Str(s)) => Some(Scalar::Str(s.clone())),
        Some(Value::Int(i)) => Some(Scalar::Int(i128::from(*i))),
        Some(Value::UInt(u)) => Some(Scalar::Int(i128::from(*u))),
        Some(Value::Float(f)) => Some(Scalar::Float(*f)),
        _ => None,
    };
    Ok(Info {
        title: scalar("title"),
        uuid: scalar("uuid"),
        version: scalar("version"),
        age: scalar("age"),
        category: scalar("category"),
        description: scalar("description"),
        cover: has_cover,
    })
}

/// `repr(float)` de Python : forme la plus courte, notation scientifique si l'exposant
/// décimal est < -4 ou >= 16, `.0` ajouté aux entiers.
pub fn py_float_repr(f: f64) -> String {
    if f.is_nan() {
        return "NaN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    // `{:e}` donne aussi la forme la plus courte qui relit le même flottant.
    let s = format!("{:e}", f.abs());
    let (mantissa, exp) = s.split_once('e').unwrap_or((&s, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let sign = if f.is_sign_negative() { "-" } else { "" };
    let decpt = exp + 1;
    if !(-4 < decpt && decpt <= 16) {
        let (first, rest) = digits.split_at(1);
        let frac = if rest.is_empty() {
            String::new()
        } else {
            format!(".{rest}")
        };
        let esign = if exp < 0 { '-' } else { '+' };
        return format!("{sign}{first}{frac}e{esign}{:02}", exp.abs());
    }
    let n = digits.len() as i32;
    let body = if decpt <= 0 {
        format!("0.{}{digits}", "0".repeat((-decpt) as usize))
    } else if decpt >= n {
        format!("{digits}{}.0", "0".repeat((decpt - n) as usize))
    } else {
        let (a, b) = digits.split_at(decpt as usize);
        format!("{a}.{b}")
    };
    format!("{sign}{body}")
}

impl Info {
    /// Ligne JSON de `--info` (sans fin de ligne).
    pub fn to_json(&self) -> String {
        let mut out = String::from("{");
        let fields = [
            ("title", &self.title),
            ("uuid", &self.uuid),
            ("version", &self.version),
            ("age", &self.age),
            ("category", &self.category),
            ("description", &self.description),
        ];
        for (key, value) in fields {
            json_str(key, &mut out);
            out.push(':');
            match value {
                None => out.push_str("null"),
                Some(Scalar::Str(s)) => json_str(s, &mut out),
                Some(Scalar::Int(i)) => out.push_str(&i.to_string()),
                Some(Scalar::Float(f)) => out.push_str(&py_float_repr(*f)),
            }
            out.push(',');
        }
        out.push_str("\"cover\":");
        out.push_str(if self.cover { "true" } else { "false" });
        out.push('}');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repr_des_flottants_comme_python() {
        for (f, s) in [
            (0.0, "0.0"),
            (-0.0, "-0.0"),
            (1.0, "1.0"),
            (1.5, "1.5"),
            (0.1, "0.1"),
            (1e16, "1e+16"),
            (1e15, "1000000000000000.0"),
            (123456789012345680.0, "1.2345678901234568e+17"),
            (0.0001, "0.0001"),
            (0.00001, "1e-05"),
            (1.5e-7, "1.5e-07"),
            (1e100, "1e+100"),
            (5e-324, "5e-324"),
            (f64::MAX, "1.7976931348623157e+308"),
            (-2.5, "-2.5"),
            (100.0, "100.0"),
        ] {
            assert_eq!(py_float_repr(f), s, "{f}");
        }
    }

    #[test]
    fn json_ascii_comme_python() {
        let mut out = String::new();
        json_str("é\"\\\n\u{7f}\u{1}😀/", &mut out);
        // `%` remplace la barre oblique inverse dans l'attendu.
        let want = r#""%u00e9%"%%%n%u007f%u0001%ud83d%ude00/""#.replace('%', "\\");
        assert_eq!(out, want);
    }
}
