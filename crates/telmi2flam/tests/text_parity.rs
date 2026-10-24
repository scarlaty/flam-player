//! Parité avec Python de `json`, `text` et `lua` : la table `tests/data/text_cases.json` est
//! produite par `tests/data/gen_text_cases.py` à partir de `tools/telmi2flam/telmi2flam.py`.

use telmi2flam::json::{expect, load_json, Kind, Value};
use telmi2flam::lua::{lua_repr, py_float_repr, Lua};
use telmi2flam::text::{
    choice_label, describe_replaced, flat, font_safe, is_py_space, py_repr, text, Replaced,
    LABEL_MAX, UNIDATA_VERSION,
};

fn table() -> Value {
    load_json(include_bytes!("data/text_cases.json"), "text_cases.json").unwrap()
}

fn field<'a>(v: &'a Value, key: &str) -> &'a Value {
    v.get(key).unwrap_or_else(|| panic!("champ {key} absent"))
}

fn list(v: &Value) -> &[Value] {
    match v {
        Value::List(items) => items,
        other => panic!("liste attendue : {other:?}"),
    }
}

fn str_of(v: &Value) -> &str {
    match v {
        Value::Str(s) => s,
        other => panic!("chaine attendue : {other:?}"),
    }
}

fn int_of(v: &Value) -> i64 {
    match v {
        Value::Int(i) => *i,
        other => panic!("entier attendu : {other:?}"),
    }
}

fn bits_of(v: &Value) -> f64 {
    f64::from_bits(str_of(v).parse().unwrap())
}

/// Forme étiquetée de `tag()` du générateur, calculée sur une valeur Rust.
fn tagged(v: &Value) -> Value {
    let one = |k: &str, x: Value| {
        let mut m = telmi2flam::json::Object::new();
        m.insert(k.to_owned(), x);
        Value::Object(m)
    };
    match v {
        Value::Null | Value::Bool(_) | Value::Str(_) => v.clone(),
        Value::Int(i) => one("i", Value::Str(i.to_string())),
        Value::UInt(u) => one("i", Value::Str(u.to_string())),
        Value::Float(f) => one("f", Value::Str(f.to_bits().to_string())),
        Value::List(items) => one("l", Value::List(items.iter().map(tagged).collect())),
        Value::Object(map) => one(
            "d",
            Value::List(
                map.iter()
                    .map(|(k, x)| Value::List(vec![Value::Str(k.clone()), tagged(x)]))
                    .collect(),
            ),
        ),
    }
}

/// Valeur Lua d'une forme étiquetée (clés non chaînes comprises).
fn untag_lua(v: &Value) -> Lua {
    match v {
        Value::Null => Lua::Nil,
        Value::Bool(b) => Lua::Bool(*b),
        Value::Str(s) => Lua::Str(s.clone()),
        Value::Object(m) => {
            let (k, x) = m.iter().next().unwrap();
            match k.as_str() {
                "i" => Lua::Int(str_of(x).parse().unwrap()),
                "f" => Lua::Float(bits_of(x)),
                "l" => Lua::List(list(x).iter().map(untag_lua).collect()),
                "d" => Lua::Table(
                    list(x)
                        .iter()
                        .map(|p| (untag_lua(&list(p)[0]), untag_lua(&list(p)[1])))
                        .collect(),
                ),
                _ => panic!("etiquette inconnue {k}"),
            }
        }
        other => panic!("forme etiquetee invalide : {other:?}"),
    }
}

fn replaced_pairs(r: &Replaced) -> Vec<(String, String)> {
    r.iter().map(|(c, s)| (c.to_string(), s.clone())).collect()
}

fn expected_pairs(v: &Value) -> Vec<(String, String)> {
    list(v)
        .iter()
        .map(|p| {
            (
                str_of(&list(p)[0]).to_owned(),
                str_of(&list(p)[1]).to_owned(),
            )
        })
        .collect()
}

fn from_hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn table_produite_par_le_python_de_l_oracle() {
    let t = table();
    assert_eq!(
        str_of(field(&t, "unidata_version")),
        UNIDATA_VERSION,
        "table generee avec un Python d'une autre version d'Unicode : relancer \
         gen_text_cases.py avec le Python de ORACLE-VERSIONS.txt"
    );
    // Même Python que l'oracle épinglé : une montée de version dans ORACLE-VERSIONS.txt impose
    // de régénérer la table.
    let pins = include_str!("../../../tools/telmi2flam/ORACLE-VERSIONS.txt");
    let oracle = pins
        .lines()
        .find_map(|l| l.trim().strip_prefix("python="))
        .expect("python= absent de ORACLE-VERSIONS.txt");
    assert_eq!(str_of(field(&t, "python")), oracle);
    assert_eq!(int_of(field(&t, "label_max")), LABEL_MAX as i64);
}

#[test]
fn espaces_de_str_split() {
    let t = table();
    let expected: Vec<u32> = list(field(&t, "py_whitespace"))
        .iter()
        .map(|v| int_of(v) as u32)
        .collect();
    let actual: Vec<u32> = (0..=0x10FFFFu32)
        .filter_map(char::from_u32)
        .filter(|&c| is_py_space(c))
        .map(|c| c as u32)
        .collect();
    assert_eq!(actual, expected);
}

#[test]
fn load_json_comme_python() {
    let t = table();
    for case in list(field(&t, "json")) {
        let raw = from_hex(str_of(field(case, "input")));
        let got = load_json(&raw, "x.json");
        let py = field(case, "python");
        let label = String::from_utf8_lossy(&raw[..raw.len().min(60)]).into_owned();
        if case.get("divergence").is_some() {
            assert!(got.is_err(), "divergence json_limit attendue : {label}");
        } else if let Some(v) = py.get("ok") {
            assert_eq!(tagged(&got.unwrap()), *v, "{label}");
        } else if let Some(n) = py.get("ok_nested_lists") {
            let mut v = got.unwrap();
            let mut depth = 0;
            // `Value` implémente `Drop` : on descend par emprunt mutable.
            while let Value::List(items) = &mut v {
                depth += 1;
                match items.pop() {
                    Some(x) => v = x,
                    None => break,
                }
            }
            assert_eq!(depth, int_of(n), "{label}");
        } else {
            assert!(py.get("error").is_some(), "{label} : {py:?}");
            assert!(got.is_err(), "erreur attendue : {label}");
        }
    }
}

#[test]
fn expect_comme_python() {
    let t = table();
    for case in list(field(&t, "expect")) {
        let value = match field(case, "value") {
            Value::Null => None,
            v => Some(load_json(str_of(v).as_bytes(), "v").unwrap()),
        };
        let kinds: Vec<Kind> = list(field(case, "types"))
            .iter()
            .map(|k| match str_of(k) {
                "dict" => Kind::Object,
                "list" => Kind::List,
                "str" => Kind::Str,
                "int" => Kind::Int,
                "float" => Kind::Float,
                other => panic!("{other}"),
            })
            .collect();
        let optional = matches!(field(case, "optional"), Value::Bool(true));
        let got = expect(value.as_ref(), &kinds, "nodes.json.f", optional);
        match case.get("error") {
            Some(msg) => assert_eq!(got.unwrap_err().message, str_of(msg)),
            None => {
                let ok = got.unwrap().map_or(Value::Null, tagged);
                assert_eq!(ok, *field(case, "ok"), "{case:?}");
            }
        }
    }
}

#[test]
fn texte_comme_python() {
    let t = table();
    let tx = field(&t, "text");
    for pair in list(field(tx, "flat")) {
        let p = list(pair);
        assert_eq!(flat(str_of(&p[0])), str_of(&p[1]), "{:?}", str_of(&p[0]));
    }
    for case in list(field(tx, "font_safe")) {
        let input = str_of(field(case, "input"));
        let mut rep = Replaced::new();
        assert_eq!(
            font_safe(input, Some(&mut rep)),
            str_of(field(case, "output")),
            "{input:?}"
        );
        assert_eq!(
            replaced_pairs(&rep),
            expected_pairs(field(case, "replaced")),
            "{input:?}"
        );
        assert_eq!(text(input, None), str_of(field(case, "text")), "{input:?}");
    }
    for case in list(field(tx, "choice_label")) {
        let note = load_json(str_of(field(case, "note")).as_bytes(), "note").unwrap();
        let node_id = match field(case, "node_id") {
            Value::Null => None,
            v => Some(str_of(v)),
        };
        let mut rep = Replaced::new();
        let got = choice_label(&note, node_id, Some(&mut rep));
        assert_eq!(got, str_of(field(case, "output")), "{case:?}");
        assert_eq!(
            replaced_pairs(&rep),
            expected_pairs(field(case, "replaced"))
        );
    }
    for case in list(field(tx, "describe_replaced")) {
        let rep: Replaced = expected_pairs(field(case, "replaced"))
            .into_iter()
            .map(|(c, r)| (c.chars().next().unwrap(), r))
            .collect();
        let limit = int_of(field(case, "limit")) as usize;
        assert_eq!(
            describe_replaced(&rep, limit),
            str_of(field(case, "output"))
        );
    }
}

fn fnv1a(h: &mut u64, bytes: &[u8]) {
    for &b in bytes {
        *h ^= u64::from(b);
        *h = h.wrapping_mul(0x100000001B3);
    }
}

/// Tous les points de code : `font_safe(chr(cp))` comparé à Python par empreinte FNV-1a de
/// blocs de 4096 points de code. Un bloc différent signale un écart de tables Unicode.
#[test]
fn font_safe_exhaustif_comme_python() {
    let t = table();
    let ex = field(&t, "font_safe_exhaustive");
    let block = int_of(field(ex, "block")) as u32;
    let mut diffs = Vec::new();
    for (i, digest) in list(field(ex, "fnv1a64")).iter().enumerate() {
        let start = i as u32 * block;
        let mut h = 0xCBF29CE484222325u64;
        for c in (start..start + block).filter_map(char::from_u32) {
            fnv1a(
                &mut h,
                font_safe(c.encode_utf8(&mut [0; 4]), None).as_bytes(),
            );
            fnv1a(&mut h, b"\xff");
        }
        if format!("{h:016x}") != str_of(digest) {
            diffs.push(format!("U+{start:04X}..U+{:04X}", start + block - 1));
        }
    }
    assert!(diffs.is_empty(), "blocs differents de Python : {diffs:?}");
}

/// `repr(str)` : noms choisis (guillemets, échappements, non imprimables) puis tous les points
/// de code par blocs (empreinte FNV-1a), ce qui contrôle aussi la table `isprintable`.
#[test]
fn py_repr_comme_python() {
    let t = table();
    for case in list(field(&t, "py_repr")) {
        let c = list(case);
        assert_eq!(py_repr(str_of(&c[0])), str_of(&c[1]), "{:?}", c[0]);
    }
    let ex = field(&t, "py_repr_exhaustive");
    let block = int_of(field(ex, "block")) as u32;
    let mut diffs = Vec::new();
    for (i, digest) in list(field(ex, "fnv1a64")).iter().enumerate() {
        let start = i as u32 * block;
        let mut h = 0xCBF29CE484222325u64;
        for c in (start..start + block).filter_map(char::from_u32) {
            fnv1a(&mut h, py_repr(c.encode_utf8(&mut [0; 4])).as_bytes());
            fnv1a(&mut h, b"\xff");
        }
        if format!("{h:016x}") != str_of(digest) {
            diffs.push(format!("U+{start:04X}..U+{:04X}", start + block - 1));
        }
    }
    assert!(diffs.is_empty(), "blocs differents de Python : {diffs:?}");
}

#[test]
fn lua_repr_comme_python() {
    let t = table();
    for case in list(field(&t, "lua_repr")) {
        let v = untag_lua(field(case, "value"));
        let indent = int_of(field(case, "indent")) as usize;
        assert_eq!(
            lua_repr(&v, indent).unwrap(),
            str_of(field(case, "output")),
            "{v:?}"
        );
    }
    for pair in list(field(&t, "float_repr")) {
        let p = list(pair);
        let f = bits_of(&p[0]);
        assert_eq!(py_float_repr(f), str_of(&p[1]), "bits {}", str_of(&p[0]));
    }
}

#[test]
fn json_vers_lua() {
    let v = load_json(br#"{"a": [1, 2.5, "x"], "end": null}"#, "t").unwrap();
    assert_eq!(
        lua_repr(&Lua::from_json(&v), 0).unwrap(),
        "{\n  a = {\n    1,\n    2.5,\n    \"x\"\n  },\n  [\"end\"] = nil\n}"
    );
}

/// Même xorshift64* que `xorshift` de `gen_text_cases.py`.
fn xorshift(seed: u64) -> impl FnMut() -> u64 {
    let mut x = seed;
    move || {
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

/// Séquences de caractères tirées d'un lot de caractères délicats : `font_safe` (sortie et
/// remplacements), `text` et `flat` comparés à Python par empreinte FNV-1a de blocs de séquences.
#[test]
fn sequences_de_caracteres_comme_python() {
    let t = table();
    let sq = field(&t, "font_safe_sequences");
    let seed: u64 = str_of(field(sq, "seed")).parse().unwrap();
    let (count, block, max_len) = (
        int_of(field(sq, "count")) as usize,
        int_of(field(sq, "block")) as usize,
        int_of(field(sq, "max_len")) as u64,
    );
    let pool: Vec<char> = list(field(sq, "pool"))
        .iter()
        .map(|v| char::from_u32(int_of(v) as u32).unwrap())
        .collect();
    let expected = list(field(sq, "fnv1a64"));
    assert_eq!(expected.len() * block, count);
    let mut next = xorshift(seed);
    let mut h = 0xCBF2_9CE4_8422_2325u64;
    let mut diffs = Vec::new();
    for i in 1..=count {
        let n = 1 + next() % max_len;
        let s: String = (0..n)
            .map(|_| pool[(next() % pool.len() as u64) as usize])
            .collect();
        let mut rep = Replaced::new();
        fnv1a(&mut h, font_safe(&s, Some(&mut rep)).as_bytes());
        fnv1a(&mut h, b"\xff");
        for (c, r) in &rep {
            fnv1a(&mut h, c.to_string().as_bytes());
            fnv1a(&mut h, b"\x00");
            fnv1a(&mut h, r.as_bytes());
            fnv1a(&mut h, b"\x01");
        }
        fnv1a(&mut h, b"\xff");
        fnv1a(&mut h, text(&s, None).as_bytes());
        fnv1a(&mut h, b"\xff");
        fnv1a(&mut h, flat(&s).as_bytes());
        fnv1a(&mut h, b"\xfe");
        if i % block == 0 {
            let want = str_of(&expected[i / block - 1]);
            if format!("{h:016x}") != want {
                diffs.push(format!("séquences {}..{}", i - block, i - 1));
            }
            h = 0xCBF2_9CE4_8422_2325;
        }
    }
    assert!(diffs.is_empty(), "blocs différents de Python : {diffs:?}");
}

#[test]
fn lua_refus_comme_python() {
    let t = table();
    let cases = list(field(&t, "lua_errors"));
    assert!(cases.len() >= 5);
    for case in cases {
        let v = untag_lua(field(case, "value"));
        assert_eq!(
            lua_repr(&v, 0).unwrap_err().to_string(),
            str_of(field(case, "message")),
            "{v:?}"
        );
    }
}
