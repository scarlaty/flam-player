//! Lecture des JSON TELMI (`metadata.json`, `nodes.json`, `notes.json`) avec la sémantique de
//! `load_json` et `_expect` de `tools/telmi2flam/telmi2flam.py`.
//!
//! Comportements de Python reproduits :
//! - BOM UTF-8 toléré une seule fois en tête (`decode("utf-8-sig")`) ; un second BOM, ou un BOM
//!   ailleurs qu'en tête, est une erreur, comme pour `json.loads` ;
//! - `NaN`, `Infinity` et `-Infinity` refusés (`parse_constant`) ;
//! - clés dupliquées : la dernière valeur gagne, à la position de la première (`dict`) ;
//! - `1` est un entier et `1.0`, `1e5` des flottants, comme le scanner de `json`.
//!
//! Divergences assumées (`PORT-RUST.md` §3, écart `json_limit` de `parity.py`), où Python
//! accepte le document ou échoue plus loin avec une erreur interne (code 6) : flottant hors
//! plage (`1e400`), entier hors de `i64::MIN..=u64::MAX`, imbrication au-delà de [`MAX_DEPTH`],
//! surrogate isolé (`"\ud800"`). Le Rust rend alors [`ContentError`] (`invalid_content`, code 4).
//!
//! L'analyseur est écrit à la main (pas de `serde_json`) : la fonction `preserve_order` de
//! `serde_json`, nécessaire à l'ordre de `dict`, s'unifierait dans tout le workspace et
//! changerait l'ordre des objets JSON du launcher. Aucune entrée ne peut le faire paniquer :
//! la profondeur est bornée, les nombres sont vérifiés, les indices restent dans le texte.

use std::fmt;

use indexmap::IndexMap;

/// Profondeur maximale d'imbrication (listes et objets) acceptée. Les histoires TELMI ne
/// dépassent pas quelques niveaux ; la borne protège la pile de l'analyse récursive.
pub const MAX_DEPTH: usize = 512;

/// Objet JSON dans l'ordre d'insertion, comme `dict` de Python.
pub type Object = IndexMap<String, Value>;

/// Valeur JSON décodée, avec la distinction entier / flottant de Python.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    /// Entier de `i64::MAX + 1` à `u64::MAX` (aussi un `int` pour Python). Variante à part
    /// pour garder `Value` compacte : la pile de l'analyse récursive en dépend (voir
    /// `profondeur_maximale_tient_dans_une_pile_de_1_mo`).
    UInt(u64),
    Float(f64),
    Str(String),
    List(Vec<Value>),
    Object(Object),
}

/// Type d'une valeur JSON, pour [`expect`] (`dict`, `list`, `str`, `int`, `float`, `bool`,
/// `None` en Python).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Object,
    List,
    Str,
    Int,
    Float,
    Bool,
    Null,
}

impl Kind {
    /// Nom français du type (`_TYPE_FR` de Python).
    pub fn name_fr(self) -> &'static str {
        match self {
            Kind::Object => "objet",
            Kind::List => "liste",
            Kind::Str => "chaine",
            Kind::Int => "entier",
            Kind::Float => "nombre",
            Kind::Bool => "booleen",
            Kind::Null => "null",
        }
    }
}

impl Value {
    /// Type de la valeur.
    pub fn kind(&self) -> Kind {
        match self {
            Value::Null => Kind::Null,
            Value::Bool(_) => Kind::Bool,
            Value::Int(_) | Value::UInt(_) => Kind::Int,
            Value::Float(_) => Kind::Float,
            Value::Str(_) => Kind::Str,
            Value::List(_) => Kind::List,
            Value::Object(_) => Kind::Object,
        }
    }

    /// `v.get(key)` de Python pour un objet ; `None` pour les autres types.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(map) => map.get(key),
            _ => None,
        }
    }
}

/// Contenu TELMI invalide : `ConvError` de Python avec le code `invalid_content`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentError {
    pub message: String,
}

impl ContentError {
    /// Identifiant de l'événement `error` (`--progress-json`).
    pub const CODE: &'static str = "invalid_content";
    /// Code de sortie du CLI.
    pub const EXIT_CODE: u8 = 4;

    fn new(message: String) -> Self {
        ContentError { message }
    }
}

impl fmt::Display for ContentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ContentError {}

/// Décode un JSON TELMI (`load_json` de Python). `what` nomme le fichier dans les messages.
pub fn load_json(raw: &[u8], what: &str) -> Result<Value, ContentError> {
    let text = std::str::from_utf8(raw)
        .map_err(|e| ContentError::new(format!("{what} : encodage non UTF-8 ({e})")))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut parser = Parser {
        src: text,
        bytes: text.as_bytes(),
        pos: 0,
        depth: 0,
    };
    parser.document().map_err(|fail| {
        ContentError::new(match fail {
            Fail::Constant(name) => format!("valeur JSON non supportee : {name}"),
            Fail::Syntax(msg, pos) => format!("{what} : JSON invalide ({msg} : position {pos})"),
            Fail::Limit(msg, pos) => {
                format!("{what} : JSON non pris en charge ({msg} : position {pos})")
            }
        })
    })
}

/// Vérifie le type d'un champ (`_expect` de Python). `v` vaut `None` pour une clé absente ;
/// absente ou `null`, la valeur est acceptée (`Ok(None)`) si `optional`. Un booléen n'est
/// jamais accepté, même là où un entier l'est (`bool` hérite de `int` en Python).
pub fn expect<'a>(
    v: Option<&'a Value>,
    types: &[Kind],
    what: &str,
    optional: bool,
) -> Result<Option<&'a Value>, ContentError> {
    let v = v.filter(|v| !matches!(v, Value::Null));
    match v {
        None if optional => return Ok(None),
        Some(x) if x.kind() != Kind::Bool && types.contains(&x.kind()) => return Ok(Some(x)),
        _ => {}
    }
    let expected: Vec<&str> = types.iter().map(|k| k.name_fr()).collect();
    let found = v.map_or(Kind::Null, Value::kind).name_fr();
    Err(ContentError::new(format!(
        "{what} : {} attendu, {found} trouve",
        expected.join("/")
    )))
}

enum Fail {
    /// `NaN`, `Infinity`, `-Infinity` : refusés par `parse_constant`, comme en Python.
    Constant(&'static str),
    /// JSON invalide, aussi pour Python.
    Syntax(&'static str, usize),
    /// Accepté par Python, refusé ici (divergence assumée `json_limit`).
    Limit(&'static str, usize),
}

struct Parser<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    depth: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn rest(&self) -> &[u8] {
        &self.bytes[self.pos..]
    }

    fn syntax<T>(&self, msg: &'static str) -> Result<T, Fail> {
        Err(Fail::Syntax(msg, self.pos))
    }

    /// Espaces JSON de Python (`[ \t\n\r]*`), comme la grammaire standard.
    fn ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn document(&mut self) -> Result<Value, Fail> {
        if self.src.starts_with('\u{feff}') {
            return self.syntax("BOM UTF-8 inattendu");
        }
        self.ws();
        let v = self.value()?;
        self.ws();
        if self.pos != self.bytes.len() {
            return self.syntax("donnees en trop");
        }
        Ok(v)
    }

    fn literal(&mut self, word: &str) -> bool {
        if self.rest().starts_with(word.as_bytes()) {
            self.pos += word.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self) -> Result<Value, Fail> {
        match self.peek() {
            Some(b'"') => Ok(Value::Str(self.string()?)),
            Some(b'{') => self.nested(Self::object),
            Some(b'[') => self.nested(Self::array),
            Some(b'n') if self.literal("null") => Ok(Value::Null),
            Some(b't') if self.literal("true") => Ok(Value::Bool(true)),
            Some(b'f') if self.literal("false") => Ok(Value::Bool(false)),
            Some(b'N') if self.rest().starts_with(b"NaN") => Err(Fail::Constant("NaN")),
            Some(b'I') if self.rest().starts_with(b"Infinity") => Err(Fail::Constant("Infinity")),
            Some(b'-') if self.rest().starts_with(b"-Infinity") => Err(Fail::Constant("-Infinity")),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => self.syntax("valeur attendue"),
        }
    }

    fn nested(&mut self, parse: fn(&mut Self) -> Result<Value, Fail>) -> Result<Value, Fail> {
        if self.depth >= MAX_DEPTH {
            return Err(Fail::Limit("imbrication trop profonde", self.pos));
        }
        self.depth += 1;
        let v = parse(self);
        self.depth -= 1;
        v
    }

    fn object(&mut self) -> Result<Value, Fail> {
        self.pos += 1; // '{'
        let mut map = Object::new();
        self.ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Value::Object(map));
        }
        loop {
            if self.peek() != Some(b'"') {
                return self.syntax("nom de propriete entre guillemets attendu");
            }
            let key = self.string()?;
            self.ws();
            if self.peek() != Some(b':') {
                return self.syntax("':' attendu");
            }
            self.pos += 1;
            self.ws();
            let v = self.value()?;
            // `IndexMap::insert` sur une clé existante remplace la valeur sans la déplacer :
            // c'est `dict(pairs)` de Python.
            map.insert(key, v);
            self.ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                    self.ws();
                }
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Value::Object(map));
                }
                _ => return self.syntax("',' ou '}' attendu"),
            }
        }
    }

    fn array(&mut self) -> Result<Value, Fail> {
        self.pos += 1; // '['
        let mut items = Vec::new();
        self.ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Value::List(items));
        }
        loop {
            items.push(self.value()?);
            self.ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                    self.ws();
                }
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Value::List(items));
                }
                _ => return self.syntax("',' ou ']' attendu"),
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, Fail> {
        let digits = self.bytes.get(self.pos..self.pos + 4);
        let value = digits
            .filter(|d| d.iter().all(u8::is_ascii_hexdigit))
            .and_then(|d| std::str::from_utf8(d).ok())
            .and_then(|d| u32::from_str_radix(d, 16).ok());
        match value {
            Some(v) => {
                self.pos += 4;
                Ok(v)
            }
            None => self.syntax("echappement \\uXXXX invalide"),
        }
    }

    fn string(&mut self) -> Result<String, Fail> {
        self.pos += 1; // '"'
        let mut out = String::new();
        loop {
            let start = self.pos;
            while matches!(self.peek(), Some(b) if b != b'"' && b != b'\\' && b >= 0x20) {
                self.pos += 1;
            }
            // Coupe sur un octet ASCII : la tranche reste de l'UTF-8 valide.
            out.push_str(&self.src[start..self.pos]);
            match self.peek() {
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    self.escape(&mut out)?;
                }
                Some(_) => return self.syntax("caractere de controle dans une chaine"),
                None => return self.syntax("chaine non terminee"),
            }
        }
    }

    fn escape(&mut self, out: &mut String) -> Result<(), Fail> {
        let c = match self.peek() {
            Some(b'"') => '"',
            Some(b'\\') => '\\',
            Some(b'/') => '/',
            Some(b'b') => '\u{8}',
            Some(b'f') => '\u{c}',
            Some(b'n') => '\n',
            Some(b'r') => '\r',
            Some(b't') => '\t',
            Some(b'u') => {
                let at = self.pos;
                self.pos += 1;
                let hi = self.hex4()?;
                let cp = if (0xD800..0xDC00).contains(&hi) && self.rest().starts_with(b"\\u") {
                    self.pos += 2;
                    let lo = self.hex4()?;
                    if !(0xDC00..0xE000).contains(&lo) {
                        return Err(Fail::Limit("surrogate isole", at));
                    }
                    0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                } else {
                    hi
                };
                // Surrogate isolé : `char::from_u32` refuse 0xD800..=0xDFFF.
                let c = char::from_u32(cp).ok_or(Fail::Limit("surrogate isole", at))?;
                out.push(c);
                return Ok(());
            }
            _ => return self.syntax("echappement invalide"),
        };
        self.pos += 1;
        out.push(c);
        Ok(())
    }

    fn digits(&mut self) -> usize {
        let start = self.pos;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
        self.pos - start
    }

    /// Nombre au sens de `NUMBER_RE` de Python :
    /// `(-?(?:0|[1-9][0-9]*))(\.[0-9]+)?([eE][-+]?[0-9]+)?`.
    fn number(&mut self) -> Result<Value, Fail> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek() {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => {
                self.digits();
            }
            _ => return self.syntax("valeur attendue"),
        }
        let mut is_float = false;
        if self.peek() == Some(b'.') && matches!(self.bytes.get(self.pos + 1), Some(b'0'..=b'9')) {
            self.pos += 1;
            self.digits();
            is_float = true;
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            let save = self.pos;
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if self.digits() == 0 {
                self.pos = save; // pas d'exposant : la suite est une erreur de syntaxe
            } else {
                is_float = true;
            }
        }
        let text = &self.src[start..self.pos];
        if is_float {
            match text.parse::<f64>() {
                Ok(f) if f.is_finite() => Ok(Value::Float(f)),
                _ => Err(Fail::Limit("flottant hors plage", start)),
            }
        } else {
            text.parse::<i64>()
                .map(Value::Int)
                .or_else(|_| text.parse::<u64>().map(Value::UInt))
                .map_err(|_| Fail::Limit("entier hors 64 bits", start))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(s: &str) -> Result<Value, ContentError> {
        load_json(s.as_bytes(), "t.json")
    }

    #[test]
    fn cles_dupliquees_derniere_valeur_a_la_premiere_position() {
        let v = load(r#"{"a": 1, "b": 2, "a": 3}"#).unwrap();
        let Value::Object(map) = v else { panic!() };
        let pairs: Vec<_> = map.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
        assert_eq!(pairs, [("a", Value::Int(3)), ("b", Value::Int(2))]);
    }

    #[test]
    fn entiers_dans_la_plage_64_bits_signee_ou_non() {
        assert_eq!(load("-0").unwrap(), Value::Int(0));
        assert_eq!(load("18446744073709551615").unwrap(), Value::UInt(u64::MAX));
        assert_eq!(load("-9223372036854775808").unwrap(), Value::Int(i64::MIN));
        assert!(load("18446744073709551616").is_err());
        assert!(load("-9223372036854775809").is_err());
    }

    #[test]
    fn imbrication_bornee_sans_debordement_de_pile() {
        let ok = "[".repeat(MAX_DEPTH) + &"]".repeat(MAX_DEPTH);
        assert!(load(&ok).is_ok());
        let over = "[".repeat(MAX_DEPTH + 1) + &"]".repeat(MAX_DEPTH + 1);
        assert!(load(&over).is_err());
        // Entrée hostile : un million de crochets ouvrants, refusée sans récursion profonde.
        assert!(load(&"[".repeat(1_000_000)).is_err());
        assert!(load(&"{\"a\":".repeat(100_000)).is_err());
    }

    #[test]
    fn profondeur_maximale_tient_dans_une_pile_de_1_mo() {
        // Pile du fil principal sous Windows : 1 Mo. Analyse et libération en mode debug.
        let doc = "[{\"k\":".repeat(MAX_DEPTH / 2) + "1" + &"}]".repeat(MAX_DEPTH / 2);
        let h = std::thread::Builder::new()
            .stack_size(1 << 20)
            .spawn(move || load(&doc).is_ok())
            .unwrap();
        assert!(h.join().unwrap());
    }

    #[test]
    fn entrees_hostiles_sans_panique() {
        for s in [
            "\"\\u",
            "\"\\ud800\\u",
            "\"\\ud800\\uzzzz\"",
            "-",
            "1e",
            "1e+",
            "\"\\",
            "{\"a\"",
            "{\"a\":",
            "[1,",
            "\"\u{e9}",
            "1.",
            "-0.e1",
            "\u{feff}",
            "",
        ] {
            assert!(load(s).is_err(), "{s:?}");
        }
        let big = "9".repeat(100_000);
        assert!(load(&big).is_err());
        assert_eq!(load(&format!("0.{big}e-5")).unwrap(), Value::Float(0.00001));
    }

    #[test]
    fn expect_suit_python() {
        let one = Value::Int(1);
        let t = Value::Bool(true);
        assert_eq!(
            expect(Some(&one), &[Kind::Int], "x", false).unwrap(),
            Some(&one)
        );
        let e = expect(Some(&t), &[Kind::Int, Kind::Float], "x", false).unwrap_err();
        assert_eq!(e.message, "x : entier/nombre attendu, booleen trouve");
        assert_eq!(expect(None, &[Kind::Str], "x", true).unwrap(), None);
        let e = expect(Some(&Value::Null), &[Kind::Str], "x", false).unwrap_err();
        assert_eq!(e.message, "x : chaine attendu, null trouve");
    }
}
