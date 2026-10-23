//! Sérialisation Lua (`lua_repr` de `tools/telmi2flam/telmi2flam.py`) : mêmes octets que
//! Python, y compris les flottants au format `repr()` de Python.

use std::fmt;

use crate::json::Value;

/// Mots réservés de Lua 5.4, plus `global` (réservé en Lua 5.5) : interdits comme clé nue.
pub const LUA_KEYWORDS: [&str; 23] = [
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while", "global",
];

/// Valeur sérialisable en Lua (types Python acceptés par `lua_repr`).
#[derive(Debug, Clone, PartialEq)]
pub enum Lua {
    Nil,
    Bool(bool),
    Int(i128),
    Float(f64),
    Str(String),
    /// Liste Python : table Lua séquentielle.
    List(Vec<Lua>),
    /// `dict` Python : paires dans l'ordre d'insertion, clés distinctes.
    Table(Vec<(Lua, Lua)>),
}

impl Lua {
    /// Valeur Lua d'une valeur JSON (clés d'objet : chaînes). Parcours sans récursion, les
    /// conteneurs en cours sur le tas : la pile d'appels ne dépend pas de l'imbrication (#189).
    pub fn from_json(v: &Value) -> Lua {
        let mut stack: Vec<FromJson<'_>> = Vec::new();
        let mut cur = v;
        'descend: loop {
            let mut done = match cur {
                Value::List(items) => {
                    stack.push(FromJson::List(
                        items.iter(),
                        Vec::with_capacity(items.len()),
                    ));
                    None
                }
                Value::Object(map) => {
                    let pairs = Vec::with_capacity(map.len());
                    stack.push(FromJson::Table(map.iter(), pairs, String::new()));
                    None
                }
                Value::Null => Some(Lua::Nil),
                Value::Bool(b) => Some(Lua::Bool(*b)),
                Value::Int(i) => Some(Lua::Int(i128::from(*i))),
                Value::UInt(u) => Some(Lua::Int(i128::from(*u))),
                Value::Float(f) => Some(Lua::Float(*f)),
                Value::Str(s) => Some(Lua::Str(s.clone())),
            };
            while let Some(top) = stack.last_mut() {
                if let Some(lua) = done.take() {
                    match top {
                        FromJson::List(_, items) => items.push(lua),
                        FromJson::Table(_, pairs, key) => {
                            pairs.push((Lua::Str(std::mem::take(key)), lua));
                        }
                    }
                }
                let next = match top {
                    FromJson::List(it, _) => it.next(),
                    FromJson::Table(it, _, key) => it.next().map(|(k, v)| {
                        k.clone_into(key);
                        v
                    }),
                };
                if let Some(child) = next {
                    cur = child;
                    continue 'descend;
                }
                done = stack.pop().map(|frame| match frame {
                    FromJson::List(_, items) => Lua::List(items),
                    FromJson::Table(_, pairs, _) => Lua::Table(pairs),
                });
            }
            // Pile vide : `done` porte la racine (un conteneur fermé ou un scalaire).
            return done.unwrap_or(Lua::Nil);
        }
    }
}

/// Destruction sans récursion (#189), comme pour [`Value`] : les enfants (clés comprises)
/// passent sur une pile du tas avant destruction.
impl Drop for Lua {
    fn drop(&mut self) {
        let nested = match self {
            Lua::List(items) => items.iter().any(Lua::is_container),
            Lua::Table(pairs) => pairs
                .iter()
                .any(|(k, v)| k.is_container() || v.is_container()),
            _ => false,
        };
        if !nested {
            return; // la colle générée ne descend que d'un niveau
        }
        let mut stack = Vec::new();
        self.move_children(&mut stack);
        while let Some(mut v) = stack.pop() {
            v.move_children(&mut stack);
        }
    }
}

impl Lua {
    fn is_container(&self) -> bool {
        matches!(self, Lua::List(_) | Lua::Table(_))
    }

    /// Déplace les enfants d'un conteneur dans `out`, le laissant vide.
    fn move_children(&mut self, out: &mut Vec<Lua>) {
        match self {
            Lua::List(items) => out.append(items),
            Lua::Table(pairs) => {
                for (k, v) in pairs.drain(..) {
                    out.push(k);
                    out.push(v);
                }
            }
            _ => {}
        }
    }
}

/// Conteneur JSON en cours de conversion : enfants restants, valeurs Lua déjà converties (et
/// clé de l'enfant en cours pour un objet).
enum FromJson<'a> {
    List(std::slice::Iter<'a, Value>, Vec<Lua>),
    Table(
        indexmap::map::Iter<'a, String, Value>,
        Vec<(Lua, Lua)>,
        String,
    ),
}

/// Nombre non fini : `ValueError` de Python (`NaN`/`Infinity` non sérialisables en Lua).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LuaError(pub String);

impl fmt::Display for LuaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for LuaError {}

/// Littéral Lua de `value`, indenté comme `lua_repr(value, indent)` de Python.
pub fn lua_repr(value: &Lua, indent: usize) -> Result<String, LuaError> {
    let mut out = String::new();
    write_repr(&mut out, value, indent)?;
    Ok(out)
}

/// `^[A-Za-z_][A-Za-z0-9_]*\Z` : identifiant Lua ASCII.
fn is_identifier(s: &str) -> bool {
    let mut bytes = s.bytes();
    matches!(bytes.next(), Some(b) if b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn write_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            // autres caractères de contrôle : séquence décimale Lua \ddd
            '\0'..='\u{1f}' | '\u{7f}' => out.push_str(&format!("\\{:03}", c as u32)),
            _ => out.push(c),
        }
    }
    out.push('"');
}

/// Morceau de sortie restant à écrire par [`write_repr`], empilé en ordre inverse.
enum Task<'a> {
    Repr(&'a Lua, usize),
    Key(&'a Lua),
    Text(&'static str),
    Pad(usize),
}

/// Écrit `value` comme la récursion de Python, mais sans récursion : ce qu'il reste à écrire
/// est une pile de tâches sur le tas, si bien que la pile d'appels ne dépend pas de
/// l'imbrication (#189). Les tâches d'un conteneur sont empilées à l'envers pour sortir dans
/// l'ordre ; coût transitoire : deux à cinq tâches de 24 octets par élément du conteneur
/// ouvert, négligeable pour une histoire TELMI.
fn write_repr(out: &mut String, value: &Lua, indent: usize) -> Result<(), LuaError> {
    let mut tasks = vec![Task::Repr(value, indent)];
    while let Some(task) = tasks.pop() {
        let (value, indent) = match task {
            Task::Text(s) => {
                out.push_str(s);
                continue;
            }
            Task::Pad(n) => {
                push_pad(out, n);
                continue;
            }
            Task::Key(k) => {
                match k {
                    Lua::Str(s) if is_identifier(s) && !LUA_KEYWORDS.contains(&s.as_str()) => {
                        out.push_str(s)
                    }
                    _ => {
                        out.push('[');
                        tasks.push(Task::Text("]"));
                        tasks.push(Task::Repr(k, 0));
                    }
                }
                continue;
            }
            Task::Repr(value, indent) => (value, indent),
        };
        match value {
            Lua::Nil => out.push_str("nil"),
            Lua::Bool(true) => out.push_str("true"),
            Lua::Bool(false) => out.push_str("false"),
            Lua::Int(i) => out.push_str(&i.to_string()),
            Lua::Float(f) if !f.is_finite() => {
                return Err(LuaError(
                    "nombre non fini (NaN/Infinity) non serialisable en Lua".to_owned(),
                ))
            }
            Lua::Float(f) => out.push_str(&py_float_repr(*f)),
            Lua::Str(s) => write_str(out, s),
            Lua::List(items) if items.is_empty() => out.push_str("{}"),
            Lua::Table(items) if items.is_empty() => out.push_str("{}"),
            Lua::List(items) => {
                out.push_str("{\n");
                close(&mut tasks, indent);
                for (i, item) in items.iter().enumerate().rev() {
                    tasks.push(Task::Repr(item, indent + 1));
                    tasks.push(Task::Pad(indent + 1));
                    if i > 0 {
                        tasks.push(Task::Text(",\n"));
                    }
                }
            }
            Lua::Table(items) => {
                out.push_str("{\n");
                close(&mut tasks, indent);
                for (i, (k, v)) in items.iter().enumerate().rev() {
                    tasks.push(Task::Repr(v, indent + 1));
                    tasks.push(Task::Text(" = "));
                    tasks.push(Task::Key(k));
                    tasks.push(Task::Pad(indent + 1));
                    if i > 0 {
                        tasks.push(Task::Text(",\n"));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Fermeture d'un conteneur non vide : retour à la ligne, marge, `}`.
fn close(tasks: &mut Vec<Task<'_>>, indent: usize) {
    tasks.push(Task::Text("}"));
    tasks.push(Task::Pad(indent));
    tasks.push(Task::Text("\n"));
}

fn push_pad(out: &mut String, indent: usize) {
    for _ in 0..indent {
        out.push_str("  ");
    }
}

/// `repr(f)` de Python : chiffres les plus courts qui relisent le même flottant, notation
/// exponentielle si l'exposant décimal `decpt` (valeur = 0,d1d2… × 10^decpt) vérifie
/// `decpt <= -4` ou `decpt > 16`, `.0` ajouté aux entiers (`format_float_short`, mode `r`).
pub fn py_float_repr(f: f64) -> String {
    if f.is_nan() {
        return "nan".to_owned();
    }
    if f.is_infinite() {
        return if f > 0.0 { "inf" } else { "-inf" }.to_owned();
    }
    let (digits, decpt) = shortest_digits(f.abs());
    let n = digits.len() as i32;
    let sign = if f.is_sign_negative() { "-" } else { "" };
    let body = if decpt <= -4 || decpt > 16 {
        let (first, rest) = digits.split_at(1);
        let x = decpt - 1;
        let dot = if rest.is_empty() {
            String::new()
        } else {
            format!(".{rest}")
        };
        format!(
            "{first}{dot}e{}{:02}",
            if x < 0 { '-' } else { '+' },
            x.abs()
        )
    } else if decpt <= 0 {
        format!("0.{}{digits}", "0".repeat((-decpt) as usize))
    } else if decpt >= n {
        format!("{digits}{}.0", "0".repeat((decpt - n) as usize))
    } else {
        let (a, b) = digits.split_at(decpt as usize);
        format!("{a}.{b}")
    };
    format!("{sign}{body}")
}

/// Chiffres significatifs (sans zéros finaux) et exposant `decpt` d'un `{:e}` de Rust.
fn sci_digits(sci: &str) -> (String, i32) {
    let (mantissa, exp) = sci.split_once('e').unwrap_or((sci, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let mut digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    while digits.len() > 1 && digits.ends_with('0') {
        digits.pop();
    }
    (digits, exp + 1)
}

/// `true` si `0.digits × 10^decpt` se relit en `a`.
fn reads_back(digits: &str, decpt: i32, a: f64) -> bool {
    format!("0.{digits}e{decpt}").parse::<f64>() == Ok(a)
}

/// Ajoute 1 au dernier chiffre (avec retenue).
fn increment(digits: &str, decpt: i32) -> (String, i32) {
    let mut d: Vec<u8> = digits.bytes().collect();
    let mut i = d.len();
    while i > 0 {
        i -= 1;
        if d[i] == b'9' {
            d[i] = b'0';
        } else {
            d[i] += 1;
            let mut s = String::from_utf8(d).unwrap_or_default();
            while s.len() > 1 && s.ends_with('0') {
                s.pop();
            }
            return (s, decpt);
        }
    }
    ("1".to_owned(), decpt + 1)
}

/// Chiffres les plus courts qui relisent `a` (positif, fini), et `decpt`, comme `_Py_dg_dtoa`
/// en mode 0. `{:e}` de Rust donne les mêmes chiffres, sauf quand `a` tombe exactement au
/// milieu de deux candidats les plus courts : Python prend alors le dernier chiffre pair
/// (arrondi au pair), Rust arrondit au-dessus (ex. 580289400337833.25 : Python « .2 »).
fn shortest_digits(a: f64) -> (String, i32) {
    let (digits, decpt) = sci_digits(&format!("{a:e}"));
    // Un milieu exact n'existe que pour un plus court de 16 chiffres ou plus (l'écart relatif
    // entre deux f64 voisins dépasse 1e-16) : inutile de payer le développement exact avant.
    if digits.len() < 15 {
        return (digits, decpt);
    }
    // Développement décimal exact de `a` (au plus 767 chiffres significatifs).
    let (exact, exact_decpt) = sci_digits(&format!("{a:.800e}"));
    if exact.len() == digits.len() + 1 && exact.ends_with('5') {
        let down = &exact[..digits.len()];
        // '0' vaut 48 : l'octet ASCII a la parité du chiffre.
        let even = down.bytes().last().is_some_and(|b| b.is_multiple_of(2));
        let (cand, cand_decpt) = if even {
            (down.to_owned(), exact_decpt)
        } else {
            increment(down, exact_decpt)
        };
        if reads_back(&cand, cand_decpt, a) {
            return (cand, cand_decpt);
        }
    }
    (digits, decpt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flottants_facon_python() {
        for (f, s) in [
            (1.0, "1.0"),
            (-0.0, "-0.0"),
            (0.0, "0.0"),
            (1e16, "1e+16"),
            (1e15, "1000000000000000.0"),
            (1e-5, "1e-05"),
            (1e-4, "0.0001"),
            (0.1, "0.1"),
            (5e-324, "5e-324"),
            (1.7976931348623157e308, "1.7976931348623157e+308"),
            (123456789012345678.0, "1.2345678901234568e+17"),
            (f64::from_bits(4827997516116503882), "580289400337833.2"),
            (0.5, "0.5"),
            (2.5, "2.5"),
        ] {
            assert_eq!(py_float_repr(f), s);
        }
    }

    #[test]
    fn non_fini_refuse() {
        assert!(lua_repr(&Lua::Float(f64::NAN), 0).is_err());
        assert!(lua_repr(&Lua::List(vec![Lua::Float(f64::INFINITY)]), 0).is_err());
    }

    #[test]
    fn cles_et_chaines() {
        let t = Lua::Table(vec![
            (Lua::Str("end".into()), Lua::Int(1)),
            (Lua::Str("ok_1".into()), Lua::Str("a\"\\\n\u{1}".into())),
        ]);
        assert_eq!(
            lua_repr(&t, 0).unwrap(),
            "{\n  [\"end\"] = 1,\n  ok_1 = \"a\\\"\\\\\\n\\001\"\n}"
        );
    }

    /// Chaîne complète à la profondeur maximale acceptée par `load_json`
    /// ([`crate::json::MAX_DEPTH`]) : analyse, conversion, sérialisation et destruction dans
    /// une pile de 256 Kio, même en debug (#189). Un débordement aborte le processus de test.
    #[test]
    fn profondeur_maximale_dans_une_pile_de_256_kio() {
        use crate::json::tests::{nested_docs, on_small_stack};
        use crate::json::{load_json, MAX_DEPTH};
        for (label, doc) in nested_docs(MAX_DEPTH) {
            let lines = on_small_stack(move || {
                let json = load_json(doc.as_bytes(), "t.json").unwrap();
                let lua = Lua::from_json(&json);
                drop(json);
                let out = lua_repr(&lua, 0).unwrap();
                drop(lua);
                // Une ligne par ouverture, une par fermeture, une pour la feuille.
                out.lines().count()
            });
            assert_eq!(lines, 2 * MAX_DEPTH + 1, "{label}");
        }
    }

    /// Clé conteneur profonde : la destruction itérative empile aussi les clés de `Table`.
    #[test]
    fn destruction_d_une_cle_profonde_dans_une_pile_de_256_kio() {
        use crate::json::tests::on_small_stack;
        let depth = crate::json::MAX_DEPTH;
        on_small_stack(move || {
            let mut key = Lua::Int(1);
            for _ in 0..depth {
                key = Lua::Table(vec![(key, Lua::List(vec![Lua::Nil]))]);
            }
            drop(Lua::Table(vec![(key, Lua::Bool(true))]));
        });
    }

    /// Le parcours itératif écrit les mêmes octets que la récursion de Python : clés
    /// conteneurs (indentation remise à 0), conteneurs vides, ordre et séparateurs.
    #[test]
    fn repr_iterative_comme_python() {
        let t = Lua::Table(vec![
            (
                Lua::Str("a".into()),
                Lua::List(vec![Lua::Int(1), Lua::List(vec![]), Lua::Table(vec![])]),
            ),
            (
                Lua::Table(vec![(
                    Lua::Int(1),
                    Lua::List(vec![Lua::Bool(true), Lua::Nil]),
                )]),
                Lua::Table(vec![(Lua::Str("b c".into()), Lua::Float(0.5))]),
            ),
            (Lua::Int(-3), Lua::Str("x".into())),
        ]);
        let want = "{\n  a = {\n    1,\n    {},\n    {}\n  },\n  [{\n  [1] = {\n    true,\n    nil\n  }\n}] = {\n    [\"b c\"] = 0.5\n  },\n  [-3] = \"x\"\n}";
        assert_eq!(lua_repr(&t, 0).unwrap(), want);
        assert_eq!(
            lua_repr(&Lua::List(vec![Lua::Int(1)]), 2).unwrap(),
            "{\n      1\n    }"
        );
        // Nombre non fini au fond d'une clé : même refus.
        let bad = Lua::Table(vec![(Lua::List(vec![Lua::Float(f64::NAN)]), Lua::Nil)]);
        assert!(lua_repr(&bad, 0).is_err());
    }

    #[test]
    fn from_json_garde_ordre_et_types() {
        use crate::json::{load_json, Value};
        let raw = br#"{"z":[1,18446744073709551615,-2.5,null,true,"s",{}],"a":{"b":[]}}"#;
        let v = load_json(raw, "t").unwrap();
        let want = Lua::Table(vec![
            (
                Lua::Str("z".into()),
                Lua::List(vec![
                    Lua::Int(1),
                    Lua::Int(i128::from(u64::MAX)),
                    Lua::Float(-2.5),
                    Lua::Nil,
                    Lua::Bool(true),
                    Lua::Str("s".into()),
                    Lua::Table(vec![]),
                ]),
            ),
            (
                Lua::Str("a".into()),
                Lua::Table(vec![(Lua::Str("b".into()), Lua::List(vec![]))]),
            ),
        ]);
        assert_eq!(Lua::from_json(&v), want);
        assert_eq!(Lua::from_json(&Value::Int(7)), Lua::Int(7));
        assert_eq!(Lua::from_json(&Value::List(vec![])), Lua::List(vec![]));
    }
}
