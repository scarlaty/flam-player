"""Genere la table de parite JSON / texte / Lua du port Rust (#125) a partir de
tools/telmi2flam/telmi2flam.py, qui sert d'oracle.

Sorties (commitees) :
  - crates/telmi2flam/tests/data/text_cases.json : cas et resultats de Python, lus par
    crates/telmi2flam/tests/text_parity.rs ;
  - crates/telmi2flam/src/text_tables.rs : tables Unicode de Python (espaces de str.split(),
    categories L* / N* de unicodedata) utilisees par le Rust.

A lancer avec la version de Python embarquee par l'installateur (build-converter.ps1 :
3.14.x, Unicode 16.0.0), depuis la racine du depot :
    python crates/telmi2flam/tests/data/gen_text_cases.py
La version Unicode est ecrite dans les deux sorties ; le test Rust verifie qu'elle est
celle de la crate unicode-normalization (une table regeneree avec un autre Python echoue).
"""

import json
import os
import random
import struct
import sys
import types
import unicodedata

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", "..", "..", ".."))
sys.path.insert(0, os.path.join(ROOT, "tools", "telmi2flam"))
try:
    import PIL  # noqa: F401  (telmi2flam importe Pillow ; inutile pour le texte)
except ImportError:
    pil = types.ModuleType("PIL")
    pil.Image = pil.ImageChops = types.SimpleNamespace(LANCZOS=1)
    sys.modules["PIL"] = pil
import telmi2flam as t  # noqa: E402

JSON_OUT = os.path.join(HERE, "text_cases.json")
RS_OUT = os.path.join(ROOT, "crates", "telmi2flam", "src", "text_tables.rs")
BLOCK = 0x1000  # points de code par empreinte de la verification exhaustive


def bits(f):
    return str(struct.unpack("<Q", struct.pack("<d", f))[0])


def tag(v):
    """Valeur Python -> JSON sans perte de type (int / float / cles non chaines)."""
    if v is None or isinstance(v, (bool, str)):
        return v
    if isinstance(v, int):
        return {"i": str(v)}
    if isinstance(v, float):
        return {"f": bits(v)}
    if isinstance(v, list):
        return {"l": [tag(x) for x in v]}
    if isinstance(v, dict):
        return {"d": [[tag(k), tag(x)] for k, x in v.items()]}
    raise TypeError(type(v))


def fnv1a(data):
    h = 0xCBF29CE484222325
    for b in data:
        h = ((h ^ b) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return h


# --------------------------------------------------------------------------- JSON
LIMIT = "json_limit"  # divergence assumee (PORT-RUST.md §3) : le Rust rend invalid_content
JSON_CASES = [
    (b'{"a": 1, "b": 2, "a": 3}', None),
    (b'{"a": {"x": 1}, "a": [2]}', None),
    (b'\xef\xbb\xbf{"a": 1}', None),
    (b'\xef\xbb\xbf\xef\xbb\xbf{}', None),
    (b' \xef\xbb\xbf{}', None),
    (b'NaN', None), (b'[Infinity]', None), (b'[-Infinity]', None), (b'{"a": NaN}', None),
    (b'[nan]', None), (b'[-NaN]', None),
    (b'1e400', LIMIT), (b'-1e400', LIMIT), (b'[1e99999999999999999999]', LIMIT),
    (b'1e-400', None), (b'5e-324', None), (b'1.7976931348623157e308', None),
    (b'123456789012345678901234567890', LIMIT),
    (b'9223372036854775807', None), (b'-9223372036854775808', None),
    (b'9223372036854775808', None), (b'18446744073709551615', None),
    (b'18446744073709551616', LIMIT), (b'-9223372036854775809', LIMIT),
    (b'"\\ud800"', LIMIT), (b'"\\udc00"', LIMIT), (b'"\\ud800\\u0041"', LIMIT),
    (b'"\\ud800\\udbff"', LIMIT), (b'"a\\udfffb"', LIMIT), (b'{"\\ud800": 1}', LIMIT),
    (b'"\\ud83d\\ude00"', None), (b'"\\u00e9\\u00E9\\/\\b\\f\\n\\r\\t\\"\\\\"', None),
    (b'[' * 512 + b']' * 512, None),
    (b'[' * 513 + b']' * 513, LIMIT),
    (b'{"a":' * 513 + b'1' + b'}' * 513, LIMIT),
    (b'\xff', None), (b'"\xed\xa0\x80"', None), (b'"\xc3"', None), (b'"\xc0\xaf"', None),
    (b'', None), (b' ', None), (b'{"a":1,}', None), (b'[1,]', None), (b'[,1]', None),
    (b'01', None), (b'1.', None), (b'.5', None), (b'-', None), (b'+1', None), (b'1e', None),
    (b'"\x01"', None), (b'"\t"', None), (b'"\x7f\xc2\x85\xe2\x80\xa8"', None),
    (b'"\\x41"', None), (b'"\\u12"', None), (b'"\\u12g4"', None), (b'"abc', None),
    (b' \t\n\r[1, 2] \r\n', None), (b'[1] x', None), (b'[1]\x00', None), (b'{"a" 1}', None),
    (b'{1: 2}', None), (b"{'a': 1}", None), (b'\xc2\xa0[1]', None), (b'\x0c[1]', None),
    (b'true', None), (b'false', None), (b'null', None), (b'tru', None), (b'True', None),
    (b'1E5', None), (b'1e+5', None), (b'-0', None), (b'-0.0', None), (b'0.1', None),
    (b'1.5e-7', None), (b'0e0', None), (b'-0e-0', None), (b'100', None), (b'1.0', None),
    (b'"\xc3\xa9\xf0\x9f\x98\x80"', None), (b'[]', None), (b'{}', None),
    (b'{"": 0, "k": [true, false, null, -1.25e3, "s"]}', None),
    (b'"' + b'a' * 1000 + b'"', None),
    (b'1' * 30, LIMIT),
]


def json_case(raw, limit):
    case = {"input": raw.hex()}
    try:
        v = t.load_json(raw, "x.json")
        if raw.startswith(b"[[[["):  # imbrication : la forme etiquetee serait trop profonde
            depth = 0
            while isinstance(v, list) and len(v) == 1:
                v, depth = v[0], depth + 1
            assert v == [], raw[:20]
            case["python"] = {"ok_nested_lists": depth + 1}
        else:
            case["python"] = {"ok": tag(v)}
    except t.ConvError:
        case["python"] = {"error": "ConvError"}
    except Exception as e:  # RecursionError, ValueError... : non attrape par Python (code 6)
        case["python"] = {"exception": type(e).__name__}
    if limit:
        assert "error" not in case["python"], raw[:40]
        case["python"] = {"accepted_or_crashed": next(iter(case["python"]))}
        case["divergence"] = limit
    return case


# --------------------------------------------------------------------------- _expect
EXPECT_TYPES = {"dict": dict, "list": list, "str": str, "int": int, "float": float}
EXPECT_CASES = [
    ('true', ["int"], True), ('1', ["int"], True), ('1.5', ["int"], True),
    ('1', ["int", "float"], True), ('1.5', ["int", "float"], False),
    ('true', ["int", "float"], False), ('null', ["int"], True), ('null', ["int"], False),
    (None, ["str"], True), (None, ["str"], False), ('"x"', ["dict"], False),
    ('[1]', ["list"], False), ('{}', ["list"], False), ('"s"', ["str"], False),
    ('1', ["float"], False), ('2.0', ["float"], False), ('[]', ["dict", "list"], False),
    ('false', ["str"], True),
]


def expect_case(src, names, optional):
    v = None if src is None else json.loads(src)
    tys = int if names == ["int"] else (EXPECT_TYPES[names[0]] if len(names) == 1
                                         else tuple(EXPECT_TYPES[n] for n in names))
    case = {"value": src, "types": names, "optional": optional}
    try:
        r = t._expect(v, tys, "nodes.json.f", optional=optional)
        case["ok"] = tag(r)
    except t.ConvError as e:
        case["error"] = str(e)
    return case


# --------------------------------------------------------------------------- texte
FLAT_CASES = [
    "", "   ", "a", "  lead  trail  ", "a\x1cb\x1dc\x1ed\x1ff", "a\x85b", "a​b",
    "a᠎b", "　x y z", "a\tb\nc\x0bd\x0ce\rf", "a\xa0b", "x y",
    " a b c d", "a﻿b", "a\x00b", "   ",
]
FONT_CASES = [
    "Héllo", "Cœur Œuvre", "“guillemets” — tiret…", "€5™",
    "ﬁn", "őŐ", "Ω", "\U0001F600", "日本", "́", "é",
    "Å", "½", "⅓", "①", "µ", "μ", "İ", "σςΣ",
    "​", "a b", "\x85", "\x1c\x1d\x1e\x1f", "\x00", "\t", "\x7f", "﻿",
    "\U0001d400", "Ａ", "ĳ", "ẞ", "Ǆ", "Ω", "　", "\xa0", " ",
    "ا", "٠", "Ⅷ", "\xdf", "\U0001F1EB\U0001F1F7", "ǽ", "‘’‚‛′",
    "‹›„‟″", "‐‑‒–―−•",
    "\U0001CCD6", "\U00010D50", "\U0001E5D0", "ͅ", "ำ", "가",
    "가", "नि", "\U0001F44D\U0001F3FD", "ǟ", "Å̊",
    " x", "¨", "˜", "․‥", "㎡", "ﷺ", "ΐ",
]
LABEL_CASES = [
    ({"title": "s0.m0"}, "s0.m0"), ({"title": "s0.m0", "text": "Aller"}, "s0.m0"),
    ({"title": "  s0.m0\t"}, "s0.m0"), ({"title": "  ", "text": "Hello  world"}, None),
    ({"title": "A" * 45}, None), ({"title": "A" * 40}, None), ({"title": "A" * 41}, None),
    ({"title": "Le petit chat est parti      bien loin dans la foret"}, None),
    ({"title": "\U0001F600\U0001F600", "text": "Suite"}, None), ({"title": "\U0001F600"}, None),
    ([1, 2], None), ("titre", None), (None, None), ({"title": 5, "text": "Cinq"}, None),
    ({"notes": "Note — seule"}, None), ({"title": "İstanbul σς"}, None),
    ({"title": "é" * 41}, None), ({"title": "ﬁ" * 30}, None),
    ({"title": "…" * 20}, None), ({"title": "a\x1cb\x85c"}, None),
    ({"title": "x" * 36 + "　　　　y"}, None), ({}, None),
    ({"text": "s1"}, "s1"), ({"title": " "}, None),
]


def replaced_list(rep):
    return [[ch, r] for ch, r in sorted(rep.items())]


def text_cases():
    out = {"flat": [[s, t._flat(s)] for s in FLAT_CASES], "font_safe": [], "choice_label": [],
           "describe_replaced": []}
    for s in FONT_CASES + FLAT_CASES:
        rep = {}
        out["font_safe"].append({"input": s, "output": t._font_safe(s, rep),
                                 "text": t._text(s), "replaced": replaced_list(rep)})
    for note, node_id in LABEL_CASES:
        rep = {}
        out["choice_label"].append({"note": json.dumps(note), "node_id": node_id,
                                    "output": t._choice_label(note, node_id, rep),
                                    "replaced": replaced_list(rep)})
    # _describe_replaced sans unicodedata.name (divergence de texte assumee, voir text.rs) :
    # meme format que Python, nom omis ; ascii() de Python est verifie tel quel.
    rep = {}
    t._font_safe("".join(FONT_CASES), rep)
    for limit in (3, 12, 1000):
        items = ["U+%04X -> %s" % (ord(ch), ascii(r) if r else "(supprime)")
                 for ch, r in sorted(rep.items())]
        more = " ..." if len(items) > limit else ""
        out["describe_replaced"].append({"limit": limit, "replaced": replaced_list(rep),
                                         "output": ", ".join(items[:limit]) + more})
    return out


def exhaustive():
    digests = []
    for start in range(0, 0x110000, BLOCK):
        buf = bytearray()
        for cp in range(start, start + BLOCK):
            if 0xD800 <= cp <= 0xDFFF:
                continue
            buf += t._font_safe(chr(cp)).encode("utf-8") + b"\xff"
        digests.append("%016x" % fnv1a(buf))
    return digests


# --------------------------------------------------------------------------- Lua
def lua_cases():
    vals = [None, True, False, 0, -5, 2 ** 63 - 1, -2 ** 63, 1.0, -0.0, 0.0, 0.1, 1e16, 1e15,
            1e-5, 1e-4, 123.456, 1e22, 5e-324, "", "a\"b\\c", "l1\nl2\r", "\x00\x01\x1f\x7f\x1c",
            "\x85\xe9\U0001F600 ", "\t", "\\n", [], {}, [1, [2, []]],
            {"a": 1, "end": 2, "global": 3, "_x": 4, "1a": 5, "a b": 6, "\xe9": 7, "a\n": 8,
             "": 9, "A_1": 10, "goto": [1, {"k": None}], "nil": {}},
            {1: "x", 2.5: "y"}, [{"x": [1.5, "s"]}, []]]
    out = [{"value": tag(v), "indent": 0, "output": t.lua_repr(v)} for v in vals]
    out.append({"value": tag({"a": [1, {"b": 2}]}), "indent": 2,
                "output": t.lua_repr({"a": [1, {"b": 2}]}, 2)})
    return out


def float_cases():
    fl = [0.0, -0.0, 1.0, -1.0, 0.1, 0.2, 0.3, 1 / 3, 2 / 3, 1e16, 1e15, 9999999999999998.0,
          1e17, 123456789012345678.0, 1e-4, 1e-5, 0.0001234, 0.00001234, 1.5e-7, 5e-324,
          2.2250738585072014e-308, 1.7976931348623157e308, 2.0 ** 53, 2.0 ** 53 + 2, 1e22,
          1e23, 123.456, 100.0, 1e100, 1.25e-100, 4.35, 0.5, 9.5e15, 1.2345678901234567e-5,
          float("inf"), float("-inf")]
    rnd = random.Random(125)
    while len(fl) < 1200:
        f = struct.unpack("<d", struct.pack("<Q", rnd.getrandbits(64)))[0]
        if f == f:
            fl.append(f)
    for _ in range(300):
        # milieux exacts entre deux candidats les plus courts : arrondi au pair de Python
        fl.append(float(rnd.randint(2 ** 47, 2 ** 52)) + rnd.choice((0.25, 0.75, 0.125, 0.375)))
        fl.append(rnd.randint(1, 2 ** 53) * 2.0 ** rnd.randint(-80, 80))
        fl.append(rnd.uniform(-1e6, 1e6))
        fl.append(float(rnd.randint(1, 10 ** 17)) / 10 ** rnd.randint(0, 20))
    return [[bits(f), repr(f)] for f in fl]


def gen_tables():
    ws = [cp for cp in range(0x110000) if chr(cp).isspace()]
    ranges = []
    for cp in range(0x110000):
        if unicodedata.category(chr(cp))[0] in "LN":
            if ranges and ranges[-1][1] == cp - 1:
                ranges[-1][1] = cp
            else:
                ranges.append([cp, cp])
    lines = [
        "//! Tables Unicode de Python, générées par `tests/data/gen_text_cases.py` : ne pas",
        "//! modifier à la main. Python %s, Unicode %s." % (sys.version.split()[0],
                                                         unicodedata.unidata_version),
        "",
        "/// Version Unicode de `unicodedata` du Python qui a produit ces tables.",
        "pub const UNIDATA_VERSION: &str = \"%s\";" % unicodedata.unidata_version,
        "",
        "/// Points de code `c` tels que `c.isspace()` (séparateurs de `str.split()`).",
        "#[rustfmt::skip]",
        "pub(crate) const PY_WHITESPACE: &[char] = &[",
    ]
    lines += ["    '\\u{%x}'," % cp for cp in ws]
    lines += [
        "];",
        "",
        "/// Intervalles (inclus) des points de code de catégorie générale L* ou N*.",
        "#[rustfmt::skip]",
        "pub(crate) const LETTER_OR_NUMBER: &[(u32, u32)] = &[",
    ]
    lines += ["    (0x%x, 0x%x)," % (a, b) for a, b in ranges]
    lines += ["];", ""]
    return "\n".join(lines), ws


def main():
    rs, ws = gen_tables()
    data = {
        "generator": "crates/telmi2flam/tests/data/gen_text_cases.py",
        "python": sys.version.split()[0],
        "unidata_version": unicodedata.unidata_version,
        "label_max": t.LABEL_MAX,
        "py_whitespace": ws,
        "json": [json_case(raw, lim) for raw, lim in JSON_CASES],
        "expect": [expect_case(*c) for c in EXPECT_CASES],
        "text": text_cases(),
        "font_safe_exhaustive": {"block": BLOCK, "fnv1a64": exhaustive()},
        "lua_repr": lua_cases(),
        "float_repr": float_cases(),
    }
    with open(JSON_OUT, "w", encoding="ascii", newline="\n") as f:
        json.dump(data, f, ensure_ascii=True, indent=1)
        f.write("\n")
    with open(RS_OUT, "w", encoding="utf-8", newline="\n") as f:
        f.write(rs)


if __name__ == "__main__":
    main()
