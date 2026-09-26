#!/usr/bin/env python3
"""
validate.py — Validation hors-GUI d'une histoire FLAM (.plain.pk) generee.

Verifie :
  1. structure du .pk (entrees requises, methode stored, uuid.bin, noms surs)
  2. syntaxe Lua de main.lua et script/*.lua (compilation sans execution) :
     via lupa si installe, sinon via un interpreteur Lua externe (variable
     FLAM_LUA, ou lua54/lua5.4/lua dans le PATH), sinon controle ignore (WARN)
  3. coherence : refs actions/stages, assets image/audio presents, indices
     d'inventaire, accessibilite
  4. en-tetes LIF (img/*.lif) et tables .mp3map (sounds/*.mp3map)

Les donnees de nodes.lua sont lues par un petit parseur Python du sous-ensemble
Lua genere par telmi2flam.py (aucune dependance requise).

Usage : python validate.py <histoire.plain.pk>
"""
import os
import re
import shutil
import subprocess
import sys
import tempfile
import zipfile

try:
    from lupa import LuaRuntime, LuaError
except ImportError:   # lupa optionnel
    LuaRuntime = None
    LuaError = Exception

LIF_MAGIC = b"liff"
LIF_END = b"\x00\x00\x00\x00\x00\x00\x00\x01"
LIF_MAX_DIM = 2047  # 11 bits de lv_img_header_t (LVGL 8.3), cf. lif_decoder.c
# Runtime Lunii indispensable au demarrage (meme liste que telmi2flam.py)
RUNTIME_SCRIPTS = ("global.lua", "progressionManager.lua", "button.lua", "button-theme.lua",
                   "button-theme-default.lua", "button-theme-interactive.lua", "v-scroll.lua",
                   "v-container.lua", "audio-player_1_0_0.lua", "carousel_1_0_0.lua",
                   "image-choice_1_0_0.lua", "list-choice_1_0_0.lua", "title-card.lua")
RUNTIME_UI_LIF = ("audio-player-pause.lif", "arrow-left-18x18-ui-000.lif",
                  "arrow-right-18x18-ui-000.lif", "arrow-right-ui-000.lif")


def lua_to_py(v):
    """Convertit recursivement une table Lua (lupa) en dict/list Python."""
    if type(v).__name__ == "_LuaTable":
        keys = list(v.keys())
        # sequence 1..n -> liste
        if keys and all(isinstance(k, int) for k in keys) and \
           sorted(keys) == list(range(1, len(keys) + 1)):
            return [lua_to_py(v[k]) for k in range(1, len(keys) + 1)]
        return {k: lua_to_py(v[k]) for k in keys}
    return v


# ---------------------------------------------------------------------------
# Parseur du sous-ensemble Lua "donnees" produit par telmi2flam.py
# (return { ... } avec chaines, nombres, booleens, nil, tables)
# ---------------------------------------------------------------------------
class LuaDataError(Exception):
    pass


_TOKEN = re.compile(r"""
    (?P<ws>\s+|--[^\n]*)
  | (?P<str>"(?:[^"\\\n]|\\.)*")
  | (?P<num>0[xX][0-9A-Fa-f]+|(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?)
  | (?P<name>[A-Za-z_][A-Za-z0-9_]*)
  | (?P<op>[{}\[\]=,;-])
""", re.VERBOSE | re.DOTALL)

_ESC = {"n": "\n", "r": "\r", "t": "\t", "a": "\a", "b": "\b", "f": "\f",
        "v": "\v", "\\": "\\", '"': '"', "'": "'", "\n": "\n"}


def _unescape(body):
    out, i, n = [], 0, len(body)
    while i < n:
        c = body[i]
        if c != "\\":
            out.append(c)
            i += 1
            continue
        i += 1
        if i >= n:
            raise LuaDataError("echappement incomplet")
        c = body[i]
        if c.isdigit():
            j = i
            while j < n and j < i + 3 and body[j].isdigit():
                j += 1
            code = int(body[i:j])
            if code > 255:
                raise LuaDataError("echappement \\%d hors borne" % code)
            out.append(chr(code))
            i = j
        elif c in _ESC:
            out.append(_ESC[c])
            i += 1
        else:
            raise LuaDataError("echappement inconnu \\%s" % c)
    return "".join(out)


def parse_lua_data(src):
    toks = []
    pos = 0
    while pos < len(src):
        m = _TOKEN.match(src, pos)
        if not m:
            raise LuaDataError("caractere inattendu a l'offset %d : %r" % (pos, src[pos:pos + 10]))
        pos = m.end()
        kind = m.lastgroup
        if kind == "ws":
            continue
        toks.append((kind, m.group(kind)))
    toks.append(("eof", None))
    i = [0]

    def peek(k=0):
        return toks[i[0] + k]

    def take(kind=None, val=None):
        t = toks[i[0]]
        if (kind and t[0] != kind) or (val is not None and t[1] != val):
            raise LuaDataError("attendu %s, trouve %r" % (val or kind, t[1]))
        i[0] += 1
        return t

    def value():
        kind, v = peek()
        if kind == "str":
            take()
            return _unescape(v[1:-1])
        if kind == "num" or (kind == "op" and v == "-"):
            neg = False
            if kind == "op":
                take()
                neg = True
                kind, v = peek()
                if kind != "num":
                    raise LuaDataError("nombre attendu apres '-'")
            take()
            if v[:2].lower() == "0x":
                num = int(v, 16)
            elif re.fullmatch(r"\d+", v):
                num = int(v)
            else:
                num = float(v)
            return -num if neg else num
        if kind == "name" and v in ("true", "false", "nil"):
            take()
            return {"true": True, "false": False, "nil": None}[v]
        if kind == "op" and v == "{":
            return table()
        raise LuaDataError("valeur attendue, trouve %r" % v)

    def table():
        take("op", "{")
        seq, hsh = [], {}
        while not (peek()[0] == "op" and peek()[1] == "}"):
            kind, v = peek()
            if kind == "op" and v == "[":
                take()
                k = value()
                take("op", "]")
                take("op", "=")
                hsh[k] = value()
            elif kind == "name" and peek(1) == ("op", "="):
                take()
                take()
                hsh[v] = value()
            else:
                seq.append(value())
            if peek()[0] == "op" and peek()[1] in (",", ";"):
                take()
            elif not (peek()[0] == "op" and peek()[1] == "}"):
                raise LuaDataError("',' ou '}' attendu, trouve %r" % peek()[1])
        take("op", "}")
        if not hsh:
            return seq
        for n, v in enumerate(seq, 1):
            hsh[n] = v
        return hsh

    take("name", "return")
    result = value()
    take("eof")
    return result


# ---------------------------------------------------------------------------
# Syntaxe Lua : lupa, sinon interpreteur externe
# ---------------------------------------------------------------------------
_CHECKER = r"""
local bad = 0
for i = 1, #arg do
  local f, err = loadfile(arg[i])
  if not f then io.write(err, "\n"); bad = bad + 1 end
end
os.exit(bad == 0 and 0 or 1)
"""


def find_lua():
    exe = os.environ.get("FLAM_LUA")
    if exe and os.path.isfile(exe):
        return exe
    for cand in ("lua54", "lua5.4", "lua"):
        p = shutil.which(cand)
        if p:
            return p
    return None


def check_lua_syntax(sources):
    """sources : {nom: texte}. Retourne (methode, {nom: erreur}) ; methode None
    si aucun moyen de compiler n'est disponible."""
    errors = {}
    if LuaRuntime is not None:
        lua = LuaRuntime(unpack_returned_tuples=True)
        for name, src in sources.items():
            try:
                lua.compile(src)
            except LuaError as e:
                errors[name] = str(e)
        return "lupa", errors
    exe = find_lua()
    if exe is None:
        return None, errors
    with tempfile.TemporaryDirectory() as td:
        chk = os.path.join(td, "check.lua")
        with open(chk, "w", encoding="utf-8") as f:
            f.write(_CHECKER)
        paths = {}
        for k, (name, src) in enumerate(sources.items()):
            p = os.path.join(td, "f%d.lua" % k)
            with open(p, "wb") as f:
                f.write(src.encode("utf-8"))
            paths[p] = name
        r = subprocess.run([exe, chk] + list(paths), capture_output=True, text=True,
                           encoding="utf-8", errors="replace")
        for line in r.stdout.splitlines():
            for p, name in paths.items():
                if p in line or os.path.basename(p) in line:
                    errors[name] = line.replace(p, name)
                    break
        if r.returncode != 0 and not errors:
            errors["?"] = (r.stdout + r.stderr).strip() or "echec de %s" % exe
    return os.path.basename(exe), errors


# ---------------------------------------------------------------------------
# Controles de format
# ---------------------------------------------------------------------------
def unsafe_name(name):
    parts = name.split("/")
    return (name.startswith("/") or "\\" in name or ":" in name or "\x00" in name
            or ".." in parts or any(p == "" for p in parts[:-1]))


def check_lif(data):
    """Retourne (erreur|None, (w, h)|None)."""
    if len(data) < 13 + len(LIF_END):
        return "trop court (%d octets)" % len(data), None
    if data[:4] != LIF_MAGIC:
        return "magic %r (attendu 'liff')" % data[:4], None
    w = int.from_bytes(data[4:8], "big")
    h = int.from_bytes(data[8:12], "big")
    if not (1 <= w <= LIF_MAX_DIM and 1 <= h <= LIF_MAX_DIM):
        return "dimensions %dx%d hors 1..%d" % (w, h, LIF_MAX_DIM), (w, h)
    if data[12] != 0xA2:
        return "canal 0x%02X (attendu 0xA2)" % data[12], (w, h)
    if data[-len(LIF_END):] != LIF_END:
        return "marqueur de fin absent", (w, h)
    return None, (w, h)


def check_mp3map(mm, mp3):
    """Retourne une erreur (str) ou None."""
    if len(mm) < 12 or (len(mm) - 12) % 8:
        return "taille %d invalide (12 + 8*N attendu)" % len(mm)
    total = int.from_bytes(mm[0:4], "little")
    first = int.from_bytes(mm[4:8], "little")
    if mp3 is not None and first + 1 >= len(mp3):
        return "offset du 1er frame %d >= taille mp3 %d" % (first, len(mp3))
    if mp3 is not None and (mp3[first] != 0xFF or (mp3[first + 1] & 0xE0) != 0xE0):
        return "offset du 1er frame %d ne pointe pas un header MPEG" % first
    prev_off, prev_u = -1, -1
    n = (len(mm) - 12) // 8
    if n > total // 88200 + 1:
        return "%d enregistrements pour %.1f s" % (n, total / 88200)
    for k in range(n):
        o = 12 + 8 * k
        off = int.from_bytes(mm[o:o + 4], "little")
        u = int.from_bytes(mm[o + 4:o + 8], "little")
        if off < prev_off or u < prev_u:
            return "enregistrement %d non croissant" % k
        if mp3 is not None:
            if off + 1 >= len(mp3):
                return "enregistrement %d : offset %d hors fichier" % (k, off)
            if mp3[off] != 0xFF or (mp3[off + 1] & 0xE0) != 0xE0:
                return "enregistrement %d : offset %d ne pointe pas un header MPEG" % (k, off)
        prev_off, prev_u = off, u
    return None


def main(pk_path):
    errs, warns, oks = [], [], []

    try:
        zf = zipfile.ZipFile(pk_path)
    except (OSError, zipfile.BadZipFile) as e:
        print("=== %s ===\n  [ERR]  archive illisible : %s" % (pk_path, e))
        return 1
    infos = zf.infolist()
    names = set(i.filename for i in infos)

    def read(name):
        return zf.read(name) if name in names else None

    # --- 1. structure ---
    nodes_name = "script/nodes.lua" if "script/nodes.lua" in names else "nodes.lua"
    required = ["info.plain", "version", "uuid.bin", "main.lua", nodes_name]
    required += ["script/" + n for n in RUNTIME_SCRIPTS]
    required += ["img/script/" + n for n in RUNTIME_UI_LIF]
    for r in required:
        if r in names:
            oks.append("entree %s" % r)
        else:
            errs.append("entree %s absente" % r)
    nonstored = [i.filename for i in infos if i.compress_type != zipfile.ZIP_STORED]
    if nonstored:
        errs.append("%d entrees non-stored (ex: %s)" % (len(nonstored), nonstored[0]))
    else:
        oks.append("toutes les entrees en STORED")
    uuid = read("uuid.bin")
    if uuid is not None:
        if len(uuid) == 16:
            oks.append("uuid.bin = 16 octets")
        else:
            errs.append("uuid.bin = %d octets (attendu 16)" % len(uuid))
    bad_names = sorted(n for n in names if unsafe_name(n))
    if bad_names:
        errs.append("%d noms d'entrees dangereux (ex: %r)" % (len(bad_names), bad_names[0]))
    else:
        oks.append("tous les noms d'entrees sont surs")
    info = read("info.plain")
    if info is not None:
        lines = info.decode("utf-8", "replace").split("\n")
        if len(lines) != 4:
            errs.append("info.plain : %d lignes (attendu 4)" % len(lines))

    # --- 2. syntaxe Lua (compile sans executer) ---
    sources = {}
    for n in sorted(names):
        if n == "main.lua" or (n.startswith("script/") and n.endswith(".lua")) or n == nodes_name:
            sources[n] = zf.read(n).decode("utf-8", "replace")
    method, lerrs = check_lua_syntax(sources)
    if method is None:
        warns.append("syntaxe Lua non verifiee (ni lupa ni interpreteur lua : "
                     "pip install lupa ou definir FLAM_LUA)")
    else:
        for name, e in sorted(lerrs.items()):
            errs.append("%s : ERREUR syntaxe : %s" % (name, e))
        if not lerrs:
            oks.append("%d fichiers Lua : syntaxe valide (%s)" % (len(sources), method))

    # --- charger les donnees nodes.lua (data pure, sans effets de bord) ---
    data = None
    if nodes_name in sources:
        try:
            data = parse_lua_data(sources[nodes_name])
        except LuaDataError as e:
            if LuaRuntime is not None:
                try:
                    lua = LuaRuntime(unpack_returned_tuples=True)
                    data = lua_to_py(lua.execute(sources[nodes_name]))
                except LuaError as e2:
                    errs.append("nodes.lua : impossible a charger : %s" % e2)
            else:
                errs.append("nodes.lua : impossible a analyser : %s" % e)
        if data is not None and not isinstance(data, dict):
            errs.append("nodes.lua : table attendue")
            data = None

    # --- 4. en-tetes LIF et mp3map ---
    lif_bad, lif_portrait = [], []
    for n in sorted(names):
        if n.startswith("img/") and n.endswith(".lif"):
            e, dims = check_lif(zf.read(n))
            if e:
                lif_bad.append("%s : %s" % (n, e))
            elif not n.startswith("img/script/") and dims[1] > dims[0] and max(dims) > 64:
                lif_portrait.append(n)
    for m in lif_bad:
        errs.append("LIF " + m)
    if not lif_bad:
        oks.append("en-tetes LIF valides")
    if lif_portrait:
        warns.append("%d LIF portrait (heuristique de transposition du decodeur) ex: %s"
                     % (len(lif_portrait), lif_portrait[:3]))
    map_bad = []
    for n in sorted(names):
        if n.startswith("sounds/") and n.endswith(".mp3map"):
            e = check_mp3map(zf.read(n), read(n[:-3]))
            if e:
                map_bad.append("%s : %s" % (n, e))
    for m in map_bad:
        errs.append("mp3map " + m)
    if not map_bad:
        oks.append("tables mp3map coherentes")

    if data:
        stages = data.get("stages", {}) or {}
        actions = data.get("actions", {}) or {}
        if isinstance(stages, list):
            stages = {}
        if isinstance(actions, list):
            actions = {}
        oks.append("nodes.lua charge : %d stages, %d actions" % (len(stages), len(actions)))

        img_entries = set(n[len("img/"):] for n in names if n.startswith("img/"))
        snd_entries = set(n[len("sounds/"):] for n in names if n.startswith("sounds/"))

        inv = data.get("inventory") or []
        ninv = len(inv) if isinstance(inv, list) else 0
        bad_inv = []

        def check_inv_index(label, v):
            if v is None:
                return
            if not isinstance(v, int) or isinstance(v, bool) or not (0 <= v < ninv):
                bad_inv.append("%s = %r (inventaire de %d items)" % (label, v, ninv))

        def check_trans(label, tr):
            if not tr:
                return
            a = tr.get("action")
            if a not in actions:
                errs.append("%s -> action inconnue '%s'" % (label, a))
                return
            if not actions[a]:
                warns.append("%s -> action '%s' vide" % (label, a))
            idx = tr.get("index")
            # -1 : tirage aleatoire (spec TELMI, gere par story.lua)
            if isinstance(idx, int) and idx != -1 and not (0 <= idx < max(1, len(actions[a]))):
                warns.append("%s -> index %d hors de l'action '%s' (%d entrees)"
                             % (label, idx, a, len(actions[a])))
            check_inv_index(label + ".indexItem", tr.get("indexItem"))

        missing_img, missing_aud, missing_map = set(), set(), set()
        bad_stage_ref = []

        for sid, st in stages.items():
            check_trans("stage %s.ok" % sid, st.get("ok"))
            check_trans("stage %s.home" % sid, st.get("home"))
            img = st.get("image")
            if img and img not in img_entries:
                missing_img.add(img)
            aud = st.get("audio")
            if aud:
                if aud not in snd_entries:
                    missing_aud.add(aud)
                if (aud + "map") not in snd_entries:
                    missing_map.add(aud)
            for k, it in enumerate(st.get("items") or []):
                check_inv_index("stage %s.items[%d].item" % (sid, k), it.get("item"))
                check_inv_index("stage %s.items[%d].assignItem" % (sid, k), it.get("assignItem"))

        for aid, lst in actions.items():
            for e in lst:
                s = e.get("stage")
                if s not in stages:
                    bad_stage_ref.append("action %s -> stage inconnu '%s'" % (aid, s))
                for k, c in enumerate(e.get("cond") or []):
                    check_inv_index("action %s cond[%d].item" % (aid, k), c.get("item"))
                    check_inv_index("action %s cond[%d].itemB" % (aid, k), c.get("itemB"))

        for k, it in enumerate(inv if isinstance(inv, list) else []):
            img = it.get("image") if isinstance(it, dict) else None
            if img and img not in img_entries:
                missing_img.add(img)
        title = data.get("title") or {}
        if title.get("image") and title["image"] not in img_entries:
            missing_img.add(title["image"])
        if title.get("audio") and title["audio"] not in snd_entries:
            missing_aud.add(title["audio"])

        # start
        start = data.get("start", {}) or {}
        if start.get("action") not in actions:
            errs.append("start -> action inconnue '%s'" % start.get("action"))
        else:
            oks.append("start -> action '%s' OK" % start.get("action"))

        for m in bad_stage_ref:
            errs.append(m)
        for m in bad_inv[:10]:
            errs.append("indice d'inventaire hors bornes : " + m)
        if len(bad_inv) > 10:
            errs.append("... %d autres indices d'inventaire hors bornes" % (len(bad_inv) - 10))
        if not bad_inv:
            oks.append("indices d'inventaire dans les bornes (%d items)" % ninv)
        if missing_img:
            errs.append("%d images manquantes dans le .pk (ex: %s)"
                        % (len(missing_img), sorted(missing_img)[:3]))
        else:
            oks.append("toutes les images referencees sont presentes")
        if missing_aud:
            errs.append("%d audios manquants (ex: %s)"
                        % (len(missing_aud), sorted(missing_aud)[:3]))
        else:
            oks.append("tous les audios references sont presents")
        if missing_map:
            warns.append("%d audios sans .mp3map (ex: %s)"
                         % (len(missing_map), sorted(missing_map)[:3]))
        else:
            oks.append("tous les audios ont un .mp3map")

        # --- accessibilite (BFS depuis start) ---
        def resolve_stages(tr):
            if not tr:
                return []
            return [e.get("stage") for e in actions.get(tr.get("action"), [])]

        seen = set()
        frontier = list(resolve_stages(data.get("start")))
        while frontier:
            s = frontier.pop()
            if s in seen or s not in stages:
                continue
            seen.add(s)
            st = stages[s]
            for tr in (st.get("ok"), st.get("home")):
                for ns in resolve_stages(tr):
                    if ns not in seen:
                        frontier.append(ns)
        unreached = set(stages) - seen
        if unreached:
            warns.append("%d stages non atteignables depuis start (ex: %s)"
                         % (len(unreached), sorted(unreached)[:5]))
        else:
            oks.append("tous les stages sont atteignables depuis start")

    zf.close()

    # --- rapport ---
    print("=== %s ===" % pk_path)
    for o in oks:
        print("  [OK]   %s" % o)
    for w in warns:
        print("  [WARN] %s" % w)
    for e in errs:
        print("  [ERR]  %s" % e)
    print("--- %d OK, %d warnings, %d erreurs ---" % (len(oks), len(warns), len(errs)))
    return 1 if errs else 0


if __name__ == "__main__":
    if len(sys.argv) != 2:
        print("Usage : python validate.py <histoire.plain.pk>", file=sys.stderr)
        sys.exit(2)
    sys.exit(main(sys.argv[1]))
