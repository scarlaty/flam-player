"""Genere la table de parite du graphe nodes.json -> Lua du port Rust (#128) a partir de
tools/telmi2flam/telmi2flam.py, qui sert d'oracle.

Sortie (commitee) : crates/telmi2flam/tests/data/graph_cases.json, lue par
crates/telmi2flam/tests/graph_parity.rs.

Chaque cas est une archive TELMI convertie de bout en bout par Python (`convert`, sortie en
dossier `.plain`) : cas du corpus de parite (tests/parity/make_corpus.py), plus des cas
propres au graphe (types, valeurs Python « fausses », chapitres, cles Lua, graphes aleatoires a
graine fixe, grands graphes). On garde les cas ou Python atteint le graphe (metadata.json et
nodes.json lus) et reussit, ou echoue en invalid_content / missing_asset dans le graphe.
Pour chaque cas : nodes.json et notes.json bruts, options, et soit le script/nodes.lua produit
(avec les noms d'assets et l'en-tete captures dans la table passee a lua_repr), soit l'erreur.

A lancer avec le Python de l'oracle (ORACLE-VERSIONS.txt : 3.14.8, Pillow 12.3.0),
depuis la racine du depot :
    python crates/telmi2flam/tests/data/gen_graph_cases.py
"""

import json
import os
import random
import shutil
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", "..", "..", ".."))
TOOL = os.path.join(ROOT, "tools", "telmi2flam")
for _p in (os.path.join(TOOL, "tests", "parity"), os.path.join(TOOL, "tests"), TOOL):
    if _p not in sys.path:
        sys.path.insert(0, _p)
sys.dont_write_bytecode = True
# Table deterministe : pas de reechantillonnage audio (#46), donc pas de ffmpeg, quelle que soit la machine.
os.environ["FLAM_FFMPEG"] = ""

import make_corpus as mc  # noqa: E402
import telmi2flam as t  # noqa: E402
from test_telmi2flam import base_nodes, std_files  # noqa: E402

OUT = os.path.join(HERE, "graph_cases.json")
SEED = 20261017
JSON_LIMIT_IDS = ("g_notes_json_limit",)  # divergence assumee (PORT-RUST.md section 3)
GRAPH_ERRORS = ("invalid_content", "missing_asset")


# --------------------------------------------------------------------------- capture
class Capture:
    def __init__(self):
        self.reset()

    def reset(self):
        self.table = None
        self.loaded = {}
        self.raw = {}
        self.calls = []


CAP = Capture()
_lua_repr = t.lua_repr
_load_json = t.load_json
_check_asset_name = t.check_asset_name


def check_asset_name(name, what):
    # Appele par conv_image / conv_audio pour chaque nom non vide, avant le cache : la suite
    # des `what` est celle des appels au trait Assets du Rust (graphe seulement, pas la
    # couverture « metadata.json image »).
    if what.startswith(("stage ", "inventory[")):
        CAP.calls.append(what)
    return _check_asset_name(name, what)


def lua_repr(value, indent=0):
    if indent == 0 and isinstance(value, dict) and "stages" in value and CAP.table is None:
        CAP.table = value
    return _lua_repr(value, indent)


def load_json(raw, what):
    CAP.raw[what] = raw
    v = _load_json(raw, what)
    CAP.loaded[what] = v
    return v


t.lua_repr = lua_repr
t.load_json = load_json
t.check_asset_name = check_asset_name


# --------------------------------------------------------------------------- cas
def zipc(nodes, notes=None):
    # nodes.json compact : la table commitee reste petite
    files = std_files(nodes=False) + [("nodes.json", json.dumps(nodes, separators=(",", ":")))]
    if notes is not None:
        files.append(("notes.json", notes if isinstance(notes, (str, bytes)) else json.dumps(notes)))
    return mc.zip_bytes(files)


def variant(fn):
    n = base_nodes()
    fn(n)
    return n


def scene(ok=None, autoplay=True, home=None, ctrl_home=False, audio="s0.mp3"):
    st = {"image": "s0.png", "ok": ok, "home": home,
          "control": {"ok": True, "home": ctrl_home, "autoplay": autoplay}}
    if audio:
        st["audio"] = audio
    return st


def tr(a, i=0):
    return {"action": a, "index": i}


def graph_cases():
    cases = []

    def add(cid, build, args=()):
        cases.append((cid, build, list(args)))

    def setp(path, value):
        def fn(n):
            obj = n
            for k in path[:-1]:
                obj = obj[k]
            obj[path[-1]] = value
        return fn

    # valeurs « fausses » / types pour ok, home, control, items, conditions
    s0 = ("stages", "s0")
    for i, v in enumerate([{}, [], 0, "", False, None, 0.0, "a0", [1], 1, {"action": "a0"},
                           {"action": "a0", "index": None}, {"action": "a0", "indexItem": None},
                           {"action": "a0", "indexItem": 2, "index": 5},
                           {"action": "a0", "index": 1.5}, {"action": 3},
                           {"action": "zz", "index": 7}, {"index": 0}]):
        add("g_ok_%d" % i, lambda v=v: zipc(variant(setp(s0 + ("ok",), v))))
        add("g_home_%d" % i, lambda v=v: zipc(variant(setp(s0 + ("home",), v))))
    for i, v in enumerate([None, {}, [], "x", {"ok": None, "home": None, "autoplay": None},
                           {"ok": 0, "home": "x", "autoplay": [0]}, {"ok": "", "home": 1.5},
                           {"autoplay": {}}, {"home": True}]):
        add("g_control_%d" % i, lambda v=v: zipc(variant(setp(s0 + ("control",), v))))
    for i, v in enumerate([[], {}, "x", [{}], [{"playingTime": 1}], [{"playingTime": 0, "assignItem": 3}],
                           [{"assignItem": None}], [{"number": 1.5, "type": 2, "item": 1}],
                           [{"number": True}], [{"type": "x"}], [{"item": 1.0}], ["x"],
                           [{"playingTime": "", "number": -0.0}], [{"number": 1e300}],
                           [{"number": 12345678901234567}, {"assignItem": -4, "playingTime": []}],
                           # entiers au-dela de i64 (json::Value::UInt) : int de Python
                           [{"type": 2 ** 64 - 1, "item": 2 ** 63, "assignItem": 2 ** 63 + 5},
                            {"number": 2 ** 64 - 1}, {"number": -2 ** 63}, {"playingTime": 2 ** 63}]]):
        add("g_items_%d" % i, lambda v=v: zipc(variant(setp(s0 + ("items",), v))))
    for i, v in enumerate([True, 1, 0, "", "x", [], None, 0.5]):
        add("g_reset_%d" % i, lambda v=v: zipc(variant(setp(s0 + ("inventoryReset",), v))))
    for i, v in enumerate([None, "", 3, ["x.png"]]):
        add("g_image_%d" % i, lambda v=v: zipc(variant(setp(s0 + ("image",), v))))
        add("g_audio_%d" % i, lambda v=v: zipc(variant(setp(s0 + ("audio",), v))))
    # noms refuses : le message cite le nom avec repr() (guillemets, echappements, Unicode)
    for i, v in enumerate(["../x.png", "/x.png", "a\\b.png", "c:x.png", "a//b.png", "a/./b.png",
                           "a\x01.png/..", "it's/../x", 'say "hi"/../x', "it's \"x\"/../x",
                           "a\tb\nc\rd:x", "\x7f\x80\x9f\xa0:", "é​/../x",
                           " 　:x", "\U0001f98a/..", "\U000e0001�:",
                           "a\x00b", "x/../\\y"]):
        add("g_bad_name_%d" % i, lambda v=v: zipc(variant(setp(s0 + ("image",), v))))
    add("g_bad_name_audio", lambda: zipc(variant(setp(s0 + ("audio",), "../é'\x1b"))))
    add("g_bad_name_inventory", lambda: zipc(variant(setp(
        ("inventory",), [{"name": "n", "image": "..\\Ā"}]))))
    a0 = ("actions", "a0")
    for i, v in enumerate([[], {}, "x", None, [None], ["s0"], [{}], [{"stage": 1}],
                           [{"stage": "s0", "conditions": None}],
                           [{"stage": "s0", "conditions": []}],
                           [{"stage": "s0", "conditions": {}}],
                           [{"stage": "s0", "conditions": [{}]}],
                           [{"stage": "s0", "conditions": [{"compareItem": 2, "number": 4}]}],
                           [{"stage": "s0", "conditions": [{"compareItem": None}]}],
                           [{"stage": "s0", "conditions": [{"number": -0.0, "comparator": 5}]}],
                           [{"stage": "s0", "conditions": [{"number": 2.5e-7}]}],
                           [{"stage": "s0", "conditions": [{"comparator": 1.0}]}],
                           [{"stage": "s0", "conditions": [{"number": "1"}]}],
                           [{"stage": "s0", "conditions": [5]}],
                           [{"stage": "absent"}, {"stage": "s0"}],
                           [{"stage": "s0", "conditions": [{"compareItem": 2 ** 64 - 1,
                                                            "item": 2 ** 63},
                                                           {"number": 2 ** 64 - 1,
                                                            "comparator": 2 ** 63}]}]]):
        add("g_action_%d" % i, lambda v=v: zipc(variant(setp(a0, v))))
    for i, v in enumerate([None, [], {}, "x", {"a0": None}]):
        add("g_actions_%d" % i, lambda v=v: zipc(variant(setp(("actions",), v))))
    for i, v in enumerate([None, [], {}, "x"]):
        add("g_stages_%d" % i, lambda v=v: zipc(variant(setp(("stages",), v))))
    for i, v in enumerate([None, {}, [], {"action": "a0"}, {"action": "a0", "index": None},
                           {"action": 1}, {"action": "zz", "index": 4}, {"index": 0},
                           {"action": "a0", "index": True}, {"action": "a0", "index": -3},
                           {"action": "a0", "index": 2 ** 64 - 1}]):
        add("g_start_%d" % i, lambda v=v: zipc(variant(setp(("startAction",), v))))
    add("g_start_absent", lambda: zipc(variant(lambda n: n.pop("startAction"))))
    # startAction invalide ET asset manquant : missing_asset passe avant (ordre de Python)
    add("g_start_after_missing", lambda: zipc(variant(lambda n: (
        n.pop("startAction"), n["stages"]["s0"].update(audio="absent.mp3")))))

    # inventaire
    inv = {"name": "Cle", "initialNumber": 1, "maxNumber": 3, "display": 1, "image": "s0.png"}
    for i, v in enumerate([None, [], {}, "x", [None], [{}], [inv],
                           [dict(inv, name=12)], [dict(inv, name=1.5)], [dict(inv, name=True)],
                           [dict(inv, name=None)], [dict(inv, name=[1])], [dict(inv, name=1e16)],
                           [dict(inv, name="Clé — \U0001F511 or")],
                           [dict(inv, initialNumber=2.5, maxNumber=-0.0)],
                           [dict(inv, maxNumber="x")], [dict(inv, display=1.0)],
                           [dict(inv, initialNumber=None)], [dict(inv, image="")],
                           [dict(inv, image=None)], [dict(inv, image="absent.png")],
                           [dict(inv, image="../x.png")], [inv, dict(inv, image="s0.png")],
                           [dict(inv, name=2 ** 64 - 1, maxNumber=2 ** 63, display=2 ** 64 - 2)]]):
        add("g_inventory_%d" % i, lambda v=v: zipc(variant(setp(("inventory",), v))))
    add("g_inventory_missing_allowed", lambda: zipc(variant(setp(
        ("inventory",), [dict(inv, image="absent.png")]))), ["--allow-missing"])

    # cles Lua : mots reserves, non identifiants, controles, Unicode
    def keys(n):
        st = n["stages"].pop("s0")
        for k in ("end", "global", "s0.m0", "1abc", "_ok", "a b", "s\x01\x7f", "été",
                  'q"\\\n\r', "nil", "x "):
            n["stages"][k] = dict(st)
        n["actions"]["a0"] = [{"stage": k} for k in n["stages"]]
        n["actions"]["and"] = [{"stage": "end"}]
    add("g_keys", lambda: zipc(variant(keys)))

    # labels (notes.json)
    two = mc.two_node_choices()
    add("g_notes_choices", lambda: zipc(two, notes=mc.CHOICE_NOTES))
    for i, notes in enumerate(["", "[]", "{oops", "1e400", "null", "3", "\"x\"",
                               {"s0": "x"}, {"s0": {"title": 5, "text": "  ", "notes": "N"}},
                               {"s1": {"title": "“Le château”…"},
                                "s2": {"title": "’" * 60}, "s0": {"title": "\U0001F98A"}},
                               {"s1": {"title": "a" * 37 + " " + "b" * 5},
                                "s2": {"text": "x " * 30}}]):
        add("g_notes_%d" % i, lambda notes=notes: zipc(two, notes=notes))
    # entier de notes.json au-dela de u64 : Python le lit, le Rust ignore le fichier
    # (divergence assumee json_limit) ; au-dela de i64 mais dans u64 : pas de divergence
    add("g_notes_u64", lambda: zipc(two, notes='{"x": 12345678901234567890}'))
    add("g_notes_json_limit", lambda: zipc(two, notes='{"x": 123456789012345678901234567890}'))
    add("g_selector_image", lambda: zipc(two, notes=mc.CHOICE_NOTES), ["--selector", "image"])

    # chapitres (tests de test_telmi2flam.py)
    def longest(home_active, orphan):
        def fn(n):
            n["stages"]["s0"]["ok"] = tr("menu")
            n["stages"]["oA"] = scene(tr("a1"), autoplay=False)
            n["stages"]["oB"] = scene(tr("b1"), autoplay=False)
            n["actions"]["menu"] = [{"stage": "oA"}, {"stage": "oB"}]
            n["stages"]["a1"] = scene(tr("a2"))
            n["stages"]["a2"] = scene(tr("a3"))
            n["stages"]["a3"] = scene(tr("a1"))
            n["stages"]["b1"] = scene(tr("b2"), home=tr("a1"), ctrl_home=home_active)
            n["stages"]["b2"] = scene(None)
            for k in ("a1", "a2", "a3", "b1", "b2"):
                n["actions"][k] = [{"stage": k}]
            if orphan:
                n["stages"]["orphan"] = scene(None)
                n["actions"]["orph"] = [{"stage": "orphan"}]
        return fn
    for h in (False, True):
        for o in (False, True):
            add("g_chapters_longest_%d%d" % (h, o), lambda h=h, o=o: zipc(variant(longest(h, o))))

    def options(autoplay2, index_item):
        def fn(n):
            opt = scene(tr("a3"), autoplay=False)
            n["stages"]["s0"]["ok"] = {"action": "a1", "indexItem": 0} if index_item else tr("a1")
            n["stages"]["s1"] = dict(opt)
            n["stages"]["s2"] = dict(opt, control={"ok": True, "home": False, "autoplay": autoplay2})
            n["stages"]["s3"] = scene(None)
            n["actions"]["a1"] = [{"stage": "s1"}, {"stage": "s2"}]
            n["actions"]["a3"] = [{"stage": "s3"}]
            if index_item:
                n["inventory"] = [{"name": "n", "initialNumber": 0, "maxNumber": 5}]
        return fn
    for ap in (False, True):
        for ii in (False, True):
            add("g_chapters_options_%d%d" % (ap, ii), lambda ap=ap, ii=ii: zipc(variant(options(ap, ii))))

    def cond_direct(n):
        n["stages"]["s0"]["ok"] = tr("a1")
        n["stages"]["s1"] = scene(None, autoplay=False)
        n["stages"]["s2"] = scene(None, autoplay=False)
        n["actions"]["a1"] = [{"stage": "s1", "conditions": [{"item": 0, "number": 1}]},
                              {"stage": "s2"}]
    add("g_chapters_conditions", lambda: zipc(variant(cond_direct)))
    add("g_chapters_none", lambda: zipc(variant(setp(s0 + ("audio",), None))))
    add("g_chapters_start_absent_action", lambda: zipc(variant(setp(("startAction", "action"), "zz"))))

    # grands graphes : chaine (profondeur), cycle unique, etoile (S stages -> une action de E)
    def chain(size, loop):
        def fn(n):
            n["stages"] = {}
            n["actions"] = {}
            for i in range(size):
                nxt = (i + 1) % size if loop else i + 1
                n["stages"]["s%d" % i] = scene(tr("a%d" % nxt) if (loop or nxt < size) else None)
                n["actions"]["a%d" % i] = [{"stage": "s%d" % i}]
            n["startAction"] = tr("a0")
        return fn
    add("g_big_chain", lambda: zipc(variant(chain(300, False))))
    add("g_big_cycle", lambda: zipc(variant(chain(300, True))))

    def star(n):
        n["stages"] = {"s%d" % i: scene(tr("hub"), autoplay=bool(i % 3)) for i in range(150)}
        n["actions"] = {"hub": [{"stage": "s%d" % i} for i in range(150)],
                        "a0": [{"stage": "s0"}]}
    add("g_big_star", lambda: zipc(variant(star)))

    # graphes aleatoires a graine fixe (chapitres : composantes, home, conditions, autoplay)
    rng = random.Random(SEED)
    for k in range(60):
        ns, na = rng.randint(1, 25), rng.randint(1, 12)

        def build(ns=ns, na=na, r=random.Random(rng.random())):
            n = {"stages": {}, "actions": {}}
            acts = ["a%d" % i for i in range(na)] + ["zz"]
            sids = ["s%d" % i for i in range(ns)] + ["ghost"]
            for i in range(ns):
                st = {"image": "s0.png", "control": {"ok": True, "home": r.random() < 0.4,
                                                      "autoplay": r.random() < 0.5}}
                if r.random() < 0.7:
                    st["audio"] = "s0.mp3"
                if r.random() < 0.8:
                    st["ok"] = tr(r.choice(acts), r.randint(0, 3))
                if r.random() < 0.4:
                    st["home"] = tr(r.choice(acts))
                n["stages"]["s%d" % i] = st
            for a in acts[:-1]:
                lst = []
                for _ in range(r.randint(0, 4)):
                    e = {"stage": r.choice(sids)}
                    if r.random() < 0.2:
                        e["conditions"] = [{"comparator": r.randint(0, 5), "item": 0,
                                            "number": r.randint(0, 3)}]
                    lst.append(e)
                n["actions"][a] = lst
            n["startAction"] = tr(r.choice(acts[:-1]))
            return zipc(n)
        add("g_random_%02d" % k, build)
    return cases


# --------------------------------------------------------------------------- conversion
def args_of(argv):
    kw = {"selector": "carousel", "allow_missing": False, "keep_size": False}
    it = iter(argv)
    for a in it:
        if a == "--selector":
            kw["selector"] = next(it)
        elif a == "--allow-missing":
            kw["allow_missing"] = True
        elif a == "--keep-size":
            kw["keep_size"] = True
    return kw


def digest(data, keep=1500):
    """Texte tel quel s'il est court (diagnostic lisible), sinon empreinte FNV-1a 64 bits et
    longueur (meme forme que tests/vectors/gen_vectors.py)."""
    if len(data) <= keep:
        return data.decode("utf-8")
    h = 0xCBF29CE484222325
    for b in data:
        h = ((h ^ b) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return "fnv1a64:%016x:%d" % (h, len(data))


def raw_field(raw):
    if raw is None:
        return None
    try:
        return {"text": raw.decode("utf-8")}
    except UnicodeDecodeError:
        return {"hex": raw.hex()}


def run(cid, data, argv, tmp):
    CAP.reset()
    src = os.path.join(tmp, "in.zip")
    with open(src, "wb") as f:
        f.write(data)
    out = os.path.join(tmp, "out.plain")
    shutil.rmtree(out, ignore_errors=True)
    kw = args_of(argv)
    warnings = []
    error = None
    try:
        t.convert(src, out, keep_size=kw["keep_size"], selector=kw["selector"],
                  allow_missing=kw["allow_missing"],
                  progress=lambda e: warnings.append(e["code"]) if e["event"] == "warning" else None)
    except t.ConvError as e:
        error = {"code": e.code, "exit": e.exit_code, "message": str(e)}
    except Exception:  # noqa: BLE001  (archive corrompue, etc. : hors du graphe)
        return None
    meta = CAP.loaded.get("metadata.json")
    if "nodes.json" not in CAP.loaded or not isinstance(meta, dict):
        return None
    if error is not None and (error["code"] not in GRAPH_ERRORS
                              or "metadata.json image" in error["message"]):
        return None
    case = {"id": cid, "selector": kw["selector"], "allow_missing": kw["allow_missing"],
            "nodes": raw_field(CAP.raw["nodes.json"]), "notes": raw_field(CAP.raw.get("notes.json")),
            "warnings": warnings,
            "missing": error is not None and error["code"] == "missing_asset"
            or "assets_missing_allowed" in warnings,
            # `what` des assets du graphe que Python a convertis, dans l'ordre des appels.
            "calls": list(CAP.calls)}
    tab = CAP.table
    if tab is not None:
        assets = {}
        for sid, st in tab["stages"].items():
            assets["stage %s.image" % sid] = st.get("image")
            assets["stage %s.audio" % sid] = st.get("audio")
        for i, it in enumerate(tab["inventory"] or []):
            assets["inventory[%d].image" % i] = it["image"]
        title = tab["title"] or {}
        case["header"] = {"title": tab["meta"]["title"], "subtitle": tab["meta"]["subtitle"],
                          "title_image": title.get("image"), "title_audio": title.get("audio")}
        case["assets"] = [[k, v] for k, v in assets.items()]
    if cid in JSON_LIMIT_IDS:
        case["divergence"] = "json_limit"
    if error is not None:
        case["error"] = error
    else:
        with open(os.path.join(out, "script", "nodes.lua"), "rb") as f:
            case["nodes_lua"] = digest(f.read())
        with open(os.path.join(out, "main.lua"), "rb") as f:
            case["main_lua"] = digest(f.read(), 0)
    return case


def main():
    if sys.version_info[:3] != (3, 14, 8):
        sys.exit("vecteurs a produire avec Python 3.14.8 (oracle de ORACLE-VERSIONS.txt)")
    builds = [(c.id, c.build, c.args) for c in mc.corpus_cases()
              if mc.TAG_JSON_LIMIT not in c.tags] + graph_cases()
    cases = []
    tmp = tempfile.mkdtemp(prefix="graph_cases_")
    try:
        for cid, build, argv in builds:
            case = run(cid, build(), argv, tmp)
            if case is not None:
                cases.append(case)
            elif cid.startswith("g_"):
                # un cas du graphe qui n'atteint plus le graphe (ou echoue ailleurs) ne doit
                # pas disparaitre de la table : seuls les cas du corpus peuvent etre ecartes
                sys.exit("cas %s ecarte : il n'atteint plus le graphe" % cid)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    # un cas par ligne : diff lisible, fichier compact
    with open(OUT, "w", encoding="utf-8", newline="\n") as f:
        f.write('{"python": "%d.%d.%d", "cases": [\n' % sys.version_info[:3])
        f.write(",\n".join(json.dumps(c, ensure_ascii=True, separators=(",", ":"))
                           for c in cases))
        f.write("\n]}\n")
    print("%d cas ecrits dans %s" % (len(cases), OUT))


if __name__ == "__main__":
    main()
