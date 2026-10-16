//! `read_info` et sortie de `--info` : métadonnées de l'histoire, sans conversion, en JSON
//! identique à `json.dumps(read_info(...), ensure_ascii=True, separators=(",", ":"))`.
//!
//! Le décodage de `metadata.json` suit `load_json` de `telmi2flam.py` (`utf-8-sig`,
//! `json.loads`, `NaN`/`Infinity` refusés) et les cas limites tranchés au §3 de
//! `PORT-RUST.md` : flottant hors plage, entier hors 64 bits, imbrication de plus de
//! `MAX_DEPTH` niveaux et surrogate isolé donnent `invalid_content` (code 4).

use std::io::{Read, Seek};
use std::path::Path;

use crate::archive::{find_root, fold, Archive};
use crate::error::ConvError;

/// Imbrication maximale acceptée : sous la limite de récursion de Python (environ
/// 990 niveaux, au-delà `RecursionError`, code 6), pour que l'écart reste « Rust code 4 ».
const MAX_DEPTH: usize = 512;

/// Valeur JSON, réduite à ce dont `read_info` a besoin.
#[derive(Debug, Clone, PartialEq)]
enum Value {
    Null,
    Bool,
    /// Entier, déjà écrit comme `str(int)` de Python (`-0` → `0`).
    Int(String),
    Float(f64),
    Str(String),
    /// Liste : contenu validé puis oublié.
    Array,
    /// Objet : clés dans l'ordre de première apparition, la dernière valeur gagne.
    Object(Vec<(String, Value)>),
}

struct Parser<'a> {
    s: &'a [u8],
    pos: usize,
    what: &'a str,
}

impl Parser<'_> {
    fn err(&self, msg: &str) -> ConvError {
        ConvError::invalid_content(format!(
            "{} : JSON invalide ({msg} : position {})",
            self.what, self.pos
        ))
    }

    fn ws(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.s.get(self.pos) {
            self.pos += 1;
        }
    }

    fn eat(&mut self, lit: &[u8]) -> bool {
        if self.s[self.pos..].starts_with(lit) {
            self.pos += lit.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, ConvError> {
        match self.s.get(self.pos) {
            Some(b'"') => Ok(Value::Str(self.string()?)),
            Some(b'{') => self.object(depth + 1),
            Some(b'[') => self.array(depth + 1),
            Some(b'n') if self.eat(b"null") => Ok(Value::Null),
            Some(b't') if self.eat(b"true") => Ok(Value::Bool),
            Some(b'f') if self.eat(b"false") => Ok(Value::Bool),
            Some(b'N') if self.eat(b"NaN") => Err(self.constant("NaN")),
            Some(b'I') if self.eat(b"Infinity") => Err(self.constant("Infinity")),
            Some(b'-') if self.s[self.pos..].starts_with(b"-Infinity") => {
                Err(self.constant("-Infinity"))
            }
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err(self.err("Expecting value")),
        }
    }

    /// `parse_constant` de `load_json` : refus explicite.
    fn constant(&self, name: &str) -> ConvError {
        ConvError::invalid_content(format!("valeur JSON non supportee : {name}"))
    }

    fn digits(&mut self) -> usize {
        let start = self.pos;
        while let Some(b'0'..=b'9') = self.s.get(self.pos) {
            self.pos += 1;
        }
        self.pos - start
    }

    /// `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][-+]?[0-9]+)?`, comme le scanner de Python.
    fn number(&mut self) -> Result<Value, ConvError> {
        let start = self.pos;
        self.eat(b"-");
        match self.s.get(self.pos) {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => {
                self.digits();
            }
            _ => return Err(self.err("Expecting value")),
        }
        let mut is_float = false;
        if self.s.get(self.pos) == Some(&b'.')
            && matches!(self.s.get(self.pos + 1), Some(b'0'..=b'9'))
        {
            self.pos += 1;
            self.digits();
            is_float = true;
        }
        if let Some(b'e' | b'E') = self.s.get(self.pos) {
            let save = self.pos;
            self.pos += 1;
            if let Some(b'+' | b'-') = self.s.get(self.pos) {
                self.pos += 1;
            }
            if self.digits() == 0 {
                self.pos = save;
            } else {
                is_float = true;
            }
        }
        // Que des caractères ASCII : la tranche est de l'UTF-8 valide.
        let text = std::str::from_utf8(&self.s[start..self.pos]).unwrap_or_default();
        if is_float {
            let f: f64 = text.parse().map_err(|_| self.err("nombre invalide"))?;
            if !f.is_finite() {
                return Err(self.err("flottant hors plage"));
            }
            Ok(Value::Float(f))
        } else {
            let i: i128 = text.parse().map_err(|_| self.err("entier hors 64 bits"))?;
            if i < i128::from(i64::MIN) || i > i128::from(u64::MAX) {
                return Err(self.err("entier hors 64 bits"));
            }
            Ok(Value::Int(i.to_string()))
        }
    }

    fn hex4(&mut self) -> Result<u32, ConvError> {
        let h = self
            .s
            .get(self.pos..self.pos + 4)
            .and_then(|h| std::str::from_utf8(h).ok())
            .filter(|h| h.bytes().all(|b| b.is_ascii_hexdigit()))
            .and_then(|h| u32::from_str_radix(h, 16).ok())
            .ok_or_else(|| self.err("Invalid \\uXXXX escape"))?;
        self.pos += 4;
        Ok(h)
    }

    /// Chaîne stricte : caractères de contrôle refusés ; surrogate isolé refusé (§3).
    fn string(&mut self) -> Result<String, ConvError> {
        self.pos += 1;
        let mut out = String::new();
        loop {
            let start = self.pos;
            while let Some(&b) = self.s.get(self.pos) {
                if b == b'"' || b == b'\\' || b < 0x20 {
                    break;
                }
                self.pos += 1;
            }
            // Le texte d'entrée est de l'UTF-8 valide et les bornes tombent sur de l'ASCII.
            out.push_str(std::str::from_utf8(&self.s[start..self.pos]).unwrap_or_default());
            match self.s.get(self.pos) {
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    let c = match self.s.get(self.pos) {
                        Some(b'"') => '"',
                        Some(b'\\') => '\\',
                        Some(b'/') => '/',
                        Some(b'b') => '\u{8}',
                        Some(b'f') => '\u{c}',
                        Some(b'n') => '\n',
                        Some(b'r') => '\r',
                        Some(b't') => '\t',
                        Some(b'u') => {
                            self.pos += 1;
                            let c = self.hex4()?;
                            let c = if (0xD800..0xDC00).contains(&c) && self.eat(b"\\u") {
                                let c2 = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&c2) {
                                    return Err(self.err("surrogate isole"));
                                }
                                0x10000 + ((c - 0xD800) << 10) + (c2 - 0xDC00)
                            } else {
                                c
                            };
                            out.push(char::from_u32(c).ok_or_else(|| self.err("surrogate isole"))?);
                            continue;
                        }
                        _ => return Err(self.err("Invalid \\escape")),
                    };
                    self.pos += 1;
                    out.push(c);
                }
                Some(_) => return Err(self.err("Invalid control character")),
                None => return Err(self.err("Unterminated string")),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Value, ConvError> {
        if depth > MAX_DEPTH {
            return Err(self.err("imbrication trop profonde"));
        }
        self.pos += 1;
        self.ws();
        if self.eat(b"]") {
            return Ok(Value::Array);
        }
        loop {
            self.ws();
            self.value(depth)?;
            self.ws();
            if self.eat(b"]") {
                return Ok(Value::Array);
            }
            if !self.eat(b",") {
                return Err(self.err("Expecting ',' delimiter"));
            }
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value, ConvError> {
        if depth > MAX_DEPTH {
            return Err(self.err("imbrication trop profonde"));
        }
        self.pos += 1;
        let mut fields: Vec<(String, Value)> = Vec::new();
        self.ws();
        if self.eat(b"}") {
            return Ok(Value::Object(fields));
        }
        loop {
            self.ws();
            if self.s.get(self.pos) != Some(&b'"') {
                return Err(self.err("Expecting property name enclosed in double quotes"));
            }
            let key = self.string()?;
            self.ws();
            if !self.eat(b":") {
                return Err(self.err("Expecting ':' delimiter"));
            }
            self.ws();
            let v = self.value(depth)?;
            // dict Python : la dernière valeur gagne, à la position de la première clé.
            match fields.iter_mut().find(|(k, _)| *k == key) {
                Some(slot) => slot.1 = v,
                None => fields.push((key, v)),
            }
            self.ws();
            if self.eat(b"}") {
                return Ok(Value::Object(fields));
            }
            if !self.eat(b",") {
                return Err(self.err("Expecting ',' delimiter"));
            }
        }
    }
}

/// `load_json` : BOM UTF-8 toléré (`utf-8-sig`), UTF-8 strict, puis `json.loads`.
fn load_json(raw: &[u8], what: &str) -> Result<Value, ConvError> {
    let raw = raw.strip_prefix(b"\xef\xbb\xbf").unwrap_or(raw);
    if let Err(e) = std::str::from_utf8(raw) {
        return Err(ConvError::invalid_content(format!(
            "{what} : encodage non UTF-8 ({e})"
        )));
    }
    let mut p = Parser {
        s: raw,
        pos: 0,
        what,
    };
    p.ws();
    let v = p.value(0)?;
    p.ws();
    if p.pos != raw.len() {
        return Err(p.err("Extra data"));
    }
    Ok(v)
}

/// Valeur scalaire de `read_info` (`str`, `int` ou `float`, jamais `bool`), sinon `null`.
#[derive(Debug, Clone, PartialEq)]
pub enum Scalar {
    Str(String),
    Int(String),
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
    let Value::Object(fields) = load_json(&raw, "metadata.json")? else {
        return Err(ConvError::invalid_content("metadata.json : objet attendu"));
    };
    let get = |key: &str| fields.iter().find(|(k, _)| k == key).map(|(_, v)| v);
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
        Some(Value::Int(i)) => Some(Scalar::Int(i.clone())),
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

/// Chaîne JSON avec `ensure_ascii=True` : tout ce qui n'est pas entre `' '` et `'~'` est
/// échappé, en `\uXXXX` minuscule (paires de surrogates au-delà du plan de base).
fn json_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            ' '..='~' => out.push(c),
            _ => {
                let mut buf = [0u16; 2];
                for unit in c.encode_utf16(&mut buf) {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            }
        }
    }
    out.push('"');
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
                Some(Scalar::Int(i)) => out.push_str(i),
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
        assert_eq!(out, bs(r#""%u00e9%"%%%n%u007f%u0001%ud83d%ude00/""#));
    }

    /// `%` remplace la barre oblique inverse dans les échappements de test.
    fn bs(s: &str) -> String {
        s.replace('%', "\\")
    }

    fn load(s: &str) -> Result<Value, ConvError> {
        load_json(s.as_bytes(), "metadata.json")
    }

    #[test]
    fn json_cas_limites() {
        // Clés dupliquées : la dernière gagne, à la place de la première.
        let v = load(r#"{"a": 1, "b": 2, "a": 3}"#).unwrap();
        assert_eq!(
            v,
            Value::Object(vec![
                ("a".into(), Value::Int("3".into())),
                ("b".into(), Value::Int("2".into()))
            ])
        );
        assert_eq!(load("-0").unwrap(), Value::Int("0".into()));
        assert_eq!(load("1E2").unwrap(), Value::Float(100.0));
        // Un seul BOM est retiré (`utf-8-sig`) : le second est une erreur JSON.
        assert_eq!(load("\u{feff}1").unwrap(), Value::Int("1".into()));
        assert_eq!(
            load("\u{feff}\u{feff}1").unwrap_err().code,
            "invalid_content"
        );
        assert_eq!(
            load_json(b"\xef\xbb\xbf 1 ", "m").unwrap(),
            Value::Int("1".into())
        );
        assert_eq!(
            load(&bs(r#""%ud83d%ude00 %u00e9%n%/""#)).unwrap(),
            Value::Str("😀 é\n/".into())
        );
        for bad in [
            "",
            "NaN",
            "Infinity",
            "-Infinity",
            "1e400",
            "123456789012345678901234567890",
            r#""%ud800""#,
            r#""%ud800%u0041""#,
            r#""%udc00""#,
            r#""%u12G4""#,
            "\"a\u{1}\"",
            "[1,]",
            "{\"a\":1,}",
            "01",
            "1.",
            "1 2",
            "{'a':1}",
            r#""%x""#,
            "[",
            "tru",
        ] {
            let bad = bs(bad);
            let e = load(&bad).unwrap_err();
            assert_eq!((e.code, e.exit_code), ("invalid_content", 4), "{bad:?}");
        }
        assert_eq!(
            load_json(b"\"\xff\"", "m").unwrap_err().code,
            "invalid_content"
        );
        assert!(load(&format!(
            "{}{}",
            "[".repeat(MAX_DEPTH),
            "]".repeat(MAX_DEPTH)
        ))
        .is_ok());
        let deep = format!("{}{}", "[".repeat(MAX_DEPTH + 1), "]".repeat(MAX_DEPTH + 1));
        assert_eq!(load(&deep).unwrap_err().code, "invalid_content");
        // Limites des entiers sur 64 bits.
        assert!(load("18446744073709551615").is_ok());
        assert!(load("-9223372036854775808").is_ok());
        assert!(load("18446744073709551616").is_err());
        assert!(load("-9223372036854775809").is_err());
    }
}
