#!/usr/bin/env python3
"""
telmi2flam.py — Convertisseur d'histoires TELMI (.zip) -> FLAM (.plain.pk).

Usage :
    python telmi2flam.py <histoire-telmi.zip> [-o sortie.plain.pk] [--plain]
                         [--keep-size] [--allow-missing] [--selector carousel|image]

Voir README.md pour les details des formats.
"""

import argparse
import hashlib
import io
import json
import math
import os
import re
import sys
import unicodedata
import zipfile

from PIL import Image, ImageChops

import lif
import mp3map

SCREEN_W, SCREEN_H = 320, 240
THUMB_W, THUMB_H = 128, 96
INV_W, INV_H = 128, 128      # icones d'inventaire
MAX_DIM = 2047               # borne du decodeur LIF (lif_decoder.c, 11 bits lv_img_header_t)
LABEL_MAX = 40               # longueur max du label d'un choix (carrousel)
ENGINE_MAIN = os.path.join(os.path.dirname(os.path.abspath(__file__)), "engine", "main.lua")
ENGINE_STORY = os.path.join(os.path.dirname(os.path.abspath(__file__)), "engine", "story.lua")
RUNTIME_DIR = os.path.join(os.path.dirname(os.path.abspath(__file__)), "runtime", "script")
RUNTIME_IMG_DIR = os.path.join(os.path.dirname(os.path.abspath(__file__)), "runtime", "img", "script")
# Modules framework indispensables au demarrage (main.lua -> require("global") et
# ses require, modules d'ecran utilises par engine/) et icones UI qu'ils chargent.
# Meme liste dans validate.py (RUNTIME_SCRIPTS / RUNTIME_UI_LIF).
RUNTIME_SCRIPTS = ("global.lua", "progressionManager.lua", "button.lua", "button-theme.lua",
                   "button-theme-default.lua", "button-theme-interactive.lua", "v-scroll.lua",
                   "v-container.lua", "audio-player_1_0_0.lua", "carousel_1_0_0.lua",
                   "image-choice_1_0_0.lua", "list-choice_1_0_0.lua", "title-card.lua")
RUNTIME_UI_LIF = ("audio-player-pause.lif", "arrow-left-18x18-ui-000.lif",
                  "arrow-right-18x18-ui-000.lif", "arrow-right-ui-000.lif")


# ---------------------------------------------------------------------------
# Serialisation Lua
# ---------------------------------------------------------------------------
_LUA_ID = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*\Z")  # \Z : "$" accepterait un "\n" final
# Mots reserves Lua 5.4 (+ "global", reserve en Lua 5.5) : interdits comme cle nue
LUA_KEYWORDS = frozenset("""
    and break do else elseif end false for function goto if in local nil not
    or repeat return then true until while global
""".split())
_LUA_CTRL = re.compile(r"[\x00-\x1f\x7f]")


class ConvError(Exception):
    """Erreur de conversion attendue (paquet TELMI invalide) : message sans traceback."""


def lua_repr(value, indent=0):
    pad = "  " * indent
    pad1 = "  " * (indent + 1)
    if value is None:
        return "nil"
    if value is True:
        return "true"
    if value is False:
        return "false"
    if isinstance(value, float) and not math.isfinite(value):
        raise ValueError("nombre non fini (NaN/Infinity) non serialisable en Lua")
    if isinstance(value, (int, float)):
        return repr(value)
    if isinstance(value, str):
        s = value.replace("\\", "\\\\").replace('"', '\\"')
        s = s.replace("\n", "\\n").replace("\r", "\\r")
        # autres caracteres de controle : sequence decimale Lua \ddd
        s = _LUA_CTRL.sub(lambda m: "\\%03d" % ord(m.group(0)), s)
        return '"' + s + '"'
    if isinstance(value, list):
        if not value:
            return "{}"
        items = [pad1 + lua_repr(v, indent + 1) for v in value]
        return "{\n" + ",\n".join(items) + "\n" + pad + "}"
    if isinstance(value, dict):
        if not value:
            return "{}"
        items = []
        for k, v in value.items():
            if isinstance(k, str) and _LUA_ID.match(k) and k not in LUA_KEYWORDS:
                key = k
            else:
                key = "[" + lua_repr(k) + "]"
            items.append(pad1 + key + " = " + lua_repr(v, indent + 1))
        return "{\n" + ",\n".join(items) + "\n" + pad + "}"
    raise TypeError("type non serialisable: %r" % type(value))


# ---------------------------------------------------------------------------
# Conversion des assets
# ---------------------------------------------------------------------------
def _png_ihdr(png_bytes):
    """(profondeur de bits, type de couleur) lus dans l'IHDR, (None, None) si
    ce n'est pas un PNG."""
    if (len(png_bytes) >= 26 and png_bytes[:8] == b"\x89PNG\r\n\x1a\n"
            and png_bytes[12:16] == b"IHDR"):
        return png_bytes[24], png_bytes[25]
    return None, None


def _trns_mask(im, png_bytes):
    """Masque alpha (mode L) d'une cle tRNS que Pillow n'applique pas : gris
    1/2/4 bits (pixels remis en 8 bits mais cle brute), gris 16 bits et RGB
    16 bits (cle 16 bits). None si rien a faire (gris 8 bits, palette, RGB 8
    bits : convert("RGBA") gere deja la cle)."""
    trns = im.info.get("transparency")
    if trns is None:
        return None
    bd, ct = _png_ihdr(png_bytes)
    if ct == 0 and isinstance(trns, int) and bd in (1, 2, 4):
        k = trns * 255 // ((1 << bd) - 1)
        return im.convert("L").point(lambda v: 0 if v == k else 255)
    if ct == 0 and isinstance(trns, int) and bd == 16:
        # comparaison exacte sur les valeurs 16 bits (avant la remise en 8 bits)
        # (ImageChops ne gere pas le mode I : deux ecarts lineaires, ecretes
        # a 0..255 par la conversion en L -> 0 si egal, 255 sinon)
        ii = im.convert("I")
        above = ii.point(lambda v: v * 255 - trns * 255).convert("L")
        below = ii.point(lambda v: v * -255 + trns * 255).convert("L")
        return ImageChops.lighter(above, below)
    if ct == 2 and isinstance(trns, tuple) and len(trns) == 3 and bd == 16:
        # Pillow ne garde que l'octet de poids fort des echantillons 16 bits :
        # on compare donc l'octet de poids fort de la cle
        chans = [c.point(lambda v, kk=(kv >> 8): 0 if v == kk else 255)
                 for c, kv in zip(im.convert("RGB").split(), trns)]
        return ImageChops.lighter(ImageChops.lighter(chans[0], chans[1]), chans[2])
    return None


def _open_rgba(png_bytes):
    im = Image.open(io.BytesIO(png_bytes))
    im.load()
    mask = _trns_mask(im, png_bytes)
    if im.mode in ("I;16", "I;16B", "I;16L", "I"):
        # niveaux de gris 16/32 bits : convert("RGBA") ecrete a 255 (image
        # blanche) au lieu de remettre a l'echelle -> ramener en 8 bits
        im = im.point(lambda v: v / 256).convert("L")
    if mask is not None:
        im = im.convert("RGB" if im.mode == "RGB" else "L")
        im.info.pop("transparency", None)
        im = im.convert("RGBA")
        im.putalpha(mask)
        return im
    return im.convert("RGBA")


def _bound(im, maxw=MAX_DIM, maxh=MAX_DIM):
    """Borne la taille (le decodeur LIF refuse au-dela de 2047 px)."""
    if im.size[0] > maxw or im.size[1] > maxh:
        im.thumbnail((maxw, maxh), Image.LANCZOS)
    return im


def _avoid_transpose(im, align="center"):
    """Le decodeur (lif_decoder.c) teste une transposition si h > w et
    max(w,h) > 64 : on colle l'image portrait sur un canevas carre transparent
    (w == h) pour ne jamais declencher cette heuristique."""
    w, h = im.size
    if h > w and h > 64:
        canvas = Image.new("RGBA", (h, h), (0, 0, 0, 0))
        x = 0 if align == "left" else (h - w) // 2
        canvas.paste(im, (x, 0))
        im = canvas
    return im


def _encode(im):
    w, h = im.size
    return lif.encode(im.tobytes(), w, h)


def _require_runtime(d, names, what):
    """Le runtime Lunii (runtime/script, runtime/img/script) doit accompagner
    le convertisseur : sinon le paquet genere ne peut pas demarrer."""
    if not os.path.isdir(d):
        raise ConvError("%s introuvables : dossier %s absent (le runtime doit "
                        "accompagner telmi2flam.py)" % (what, d))
    missing = [n for n in names if not os.path.isfile(os.path.join(d, n))]
    if missing:
        raise ConvError("%s incomplets dans %s : %s absent(s)" % (what, d, ", ".join(missing)))


def png_to_lif(png_bytes, target=None):
    """Convertit un PNG en .lif. target=(w,h) pour redimensionner, None = taille native
    (bornee a 2047 px)."""
    im = _open_rgba(png_bytes)
    if target is not None and im.size != target:
        im = im.resize(target, Image.LANCZOS)
    return _encode(_avoid_transpose(_bound(im)))


def png_to_fit_lif(png_bytes, maxw, maxh, align="center"):
    """Redimensionne en gardant le ratio pour tenir dans maxw x maxh."""
    im = _open_rgba(png_bytes)
    im.thumbnail((maxw, maxh), Image.LANCZOS)
    return _encode(_avoid_transpose(im, align))


def png_to_thumb_lif(png_bytes):
    """Vignette : on tient dans THUMB_W x THUMB_H en gardant le ratio. Une image
    portrait (ou carree) est centree sur un canevas paysage THUMB_W x THUMB_H."""
    im = _open_rgba(png_bytes)
    im.thumbnail((THUMB_W, THUMB_H), Image.LANCZOS)
    w, h = im.size
    if h >= w:
        canvas = Image.new("RGBA", (THUMB_W, THUMB_H), (0, 0, 0, 0))
        canvas.paste(im, ((THUMB_W - w) // 2, (THUMB_H - h) // 2))
        im = canvas
    return _encode(im)


# ---------------------------------------------------------------------------
# Lecture / validation de nodes.json
# ---------------------------------------------------------------------------
def _json_constant(name):
    raise ConvError("valeur JSON non supportee : %s" % name)


def load_json(raw, what):
    """Decode un JSON TELMI : BOM UTF-8 tolere, NaN/Infinity refuses."""
    try:
        return json.loads(raw.decode("utf-8-sig"), parse_constant=_json_constant)
    except UnicodeDecodeError as e:
        raise ConvError("%s : encodage non UTF-8 (%s)" % (what, e))
    except json.JSONDecodeError as e:
        raise ConvError("%s : JSON invalide (%s)" % (what, e))


_TYPE_FR = {dict: "objet", list: "liste", str: "chaine", int: "entier", float: "nombre",
            bool: "booleen", type(None): "null"}


def _is_int(v):
    return isinstance(v, int) and not isinstance(v, bool)


def _expect(v, types, what, optional=True):
    """Verifie le type d'un champ ; None accepte si optional."""
    if v is None and optional:
        return None
    if types is int:
        ok = _is_int(v)
    else:
        ok = isinstance(v, types) and not isinstance(v, bool)
    if not ok:
        tl = (types,) if isinstance(types, type) else types
        tn = "/".join(_TYPE_FR.get(t, t.__name__) for t in tl)
        raise ConvError("%s : %s attendu, %s trouve"
                        % (what, tn, _TYPE_FR.get(type(v), type(v).__name__)))
    return v


def _flat(s):
    """Aplatit une chaine sur une ligne (retours a la ligne, tabulations...)."""
    return " ".join(str(s).split())


def check_asset_name(name, what):
    """Refuse les noms d'assets pouvant sortir du dossier (zip-slip)."""
    _expect(name, str, what, optional=False)
    parts = name.split("/")
    if (not name or name.startswith("/") or "\\" in name or ":" in name
            or "\x00" in name or ".." in parts or "" in parts):
        raise ConvError("%s : nom d'asset refuse (chemin dangereux) : %r" % (what, name))
    return name


# ---------------------------------------------------------------------------
# Helpers de conversion des structures nodes.json -> Lua
# ---------------------------------------------------------------------------

def _conv_trans(tr, what):
    """Convertit une transition TELMI {action, index|indexItem} en dict Lua."""
    _expect(tr, dict, what, optional=False)
    out = {"action": _expect(tr.get("action"), str, what + ".action", optional=False)}
    if "indexItem" in tr:
        # index et indexItem sont mutuellement exclusifs (spec TELMI)
        out["indexItem"] = _expect(tr["indexItem"], int, what + ".indexItem", optional=False)
    else:
        out["index"] = _expect(tr.get("index", 0), int, what + ".index", optional=False)
    return out


def _conv_cond(c, what):
    """Convertit une condition TELMI {comparator, item, number|compareItem} en dict Lua."""
    _expect(c, dict, what, optional=False)
    out = {"cmp": _expect(c.get("comparator", 2), int, what + ".comparator", optional=False),
           "item": _expect(c.get("item", 0), int, what + ".item", optional=False)}
    if "compareItem" in c:
        out["itemB"] = _expect(c["compareItem"], int, what + ".compareItem", optional=False)
    else:
        out["num"] = _expect(c.get("number", 0), (int, float), what + ".number", optional=False)
    return out


def _conv_item(it, what):
    """Convertit une operation inventaire TELMI {type, item, number|assignItem|playingTime}."""
    _expect(it, dict, what, optional=False)
    out = {"type": _expect(it.get("type", 0), int, what + ".type", optional=False),
           "item": _expect(it.get("item", 0), int, what + ".item", optional=False)}
    if it.get("playingTime"):
        out["playingTime"] = True
    elif "assignItem" in it:
        out["assignItem"] = _expect(it["assignItem"], int, what + ".assignItem", optional=False)
    else:
        out["number"] = _expect(it.get("number", 0), (int, float), what + ".number", optional=False)
    return out


def _choice_label(note):
    """Label court d'un choix : title, sinon text, sinon notes ; tronque."""
    if not isinstance(note, dict):
        return ""
    for key in ("title", "text", "notes"):
        v = note.get(key)
        if isinstance(v, str) and v.strip():
            label = _flat(v)
            if len(label) > LABEL_MAX:
                label = label[:LABEL_MAX - 3].rstrip() + "..."
            return label
    return ""


# ---------------------------------------------------------------------------
# Convertisseur principal
# ---------------------------------------------------------------------------
def base_no_ext(name):
    return os.path.splitext(os.path.basename(name))[0]


def _warn(*args):
    print("  !", *args, file=sys.stderr)


def _fold(name):
    """Cle de recherche d'asset : NFC puis minuscules."""
    return unicodedata.normalize("NFC", name).lower()


def find_root(names):
    """Repertoire racine de l'histoire dans le zip : dossier contenant
    metadata.json ET nodes.json (nom exact), hors __MACOSX/ et fichiers ._*.
    Le candidat le moins profond gagne ; erreur si plusieurs au meme niveau."""
    nameset = set(names)
    cands = []
    for n in names:
        parts = n.split("/")
        if parts[-1] != "metadata.json" or "__MACOSX" in parts:
            continue
        if any(p.startswith("._") for p in parts):
            continue
        root = n[: -len("metadata.json")]
        if root + "nodes.json" in nameset:
            cands.append(root)
    if not cands:
        raise ConvError("metadata.json + nodes.json introuvables dans l'archive "
                        "(ce n'est pas une histoire TELMI ?)")
    depth = min(r.count("/") for r in cands)
    best = sorted(r for r in cands if r.count("/") == depth)
    if len(best) > 1:
        raise ConvError("plusieurs histoires dans l'archive : %s" % ", ".join(r or "/" for r in best))
    return best[0]


def convert(zip_path, out_path=None, keep_size=False, emit_plain=False, selector="carousel",
            allow_missing=False):
    with zipfile.ZipFile(zip_path, "r") as zf:
        return _convert(zf, out_path, keep_size, emit_plain, selector, allow_missing)


def _convert(zf, out_path, keep_size, emit_plain, selector, allow_missing):
    names = zf.namelist()
    nameset = set(names)
    # Index insensible a la casse (S0.PNG reference "s0.png" et inversement)
    # et a la normalisation Unicode (zip macOS : noms NFD, JSON : NFC)
    lower_index = {}
    for n in names:
        lower_index.setdefault(_fold(n), n)

    root = find_root(names)

    def lookup(rel):
        full = root + rel
        if full in nameset:
            return full
        return lower_index.get(_fold(full))

    def read(rel):
        full = lookup(rel)
        return zf.read(full) if full is not None else None

    metadata = load_json(zf.read(root + "metadata.json"), "metadata.json")
    nodes_raw = zf.read(root + "nodes.json")
    nodes = load_json(nodes_raw, "nodes.json")
    _expect(metadata, dict, "metadata.json", optional=False)
    _expect(nodes, dict, "nodes.json", optional=False)
    # notes.json (optionnel) : titres/textes narratifs par scene. Sert a donner
    # un label texte aux choix (carrousel). Absent dans beaucoup d'histoires.
    notes_raw = read("notes.json")
    try:
        notes = load_json(notes_raw, "notes.json") if notes_raw else {}
    except ConvError as e:
        # optionnel (Telmi Sync) : un notes.json illisible ne bloque pas la conversion
        _warn("notes.json ignore (%s)" % e)
        notes = {}
    if not isinstance(notes, dict):
        _warn("notes.json ignore (objet attendu, %s trouve)" % type(notes).__name__)
        notes = {}

    title = _flat(metadata.get("title") or "") or "Histoire"
    subtitle_src = metadata.get("category") or ""
    subtitle_src = _flat(subtitle_src) if isinstance(subtitle_src, (str, int, float)) else ""
    uuid_str = metadata.get("uuid")
    if isinstance(uuid_str, str) and uuid_str.strip():
        uuid16 = hashlib.md5(uuid_str.encode("utf-8")).digest()  # 16 octets deterministes
    else:
        # Sans uuid : hash du contenu (et non du titre, pour ne pas partager la
        # sauvegarde entre deux histoires de meme titre).
        uuid16 = hashlib.md5(b"telmi2flam:nodes.json:" + nodes_raw).digest()
        _warn("metadata.json sans uuid : UUID derive du contenu de nodes.json "
              "(change si l'histoire est modifiee)")

    target = None if keep_size else (SCREEN_W, SCREEN_H)

    # Sorties accumulees : nom dans le .pk -> bytes
    out_files = {}
    stats = {"img": 0, "audio": 0, "mp3map": 0}
    missing = []      # ("image"|"audio", nom)
    bad = []          # assets illisibles (repli applique)

    # --- Espace de noms des sorties (insensible a la casse, FAT/Windows) ---
    # Noms reserves par le convertisseur / le moteur.
    used = {"img/title.lif", "img/thumbnail.lif", "img/empty.lif",
            "sounds/title.mp3", "sounds/silent.mp3"}

    def alloc(prefix, base, ext):
        """Nom de sortie canonique : [A-Za-z0-9_-], unique dans prefix."""
        b = re.sub(r"[^A-Za-z0-9_-]+", "_", base).strip("_") or "asset"
        cand, i = b, 2
        while (prefix + cand + ext).lower() in used:
            cand = "%s_%d" % (b, i)
            i += 1
        used.add((prefix + cand + ext).lower())
        return cand + ext

    img_cache = {}    # ("s7.png", is_inventory) -> "s7.lif"
    audio_cache = {}  # "s5.mp3" -> "s5.mp3"

    # Progression (A03) : sur stderr uniquement si c'est un terminal
    stages_src = _expect(nodes.get("stages", {}), dict, "nodes.json stages") or {}
    inv_src = _expect(nodes.get("inventory"), list, "nodes.json inventory") or []
    progress_total = len(stages_src) + len(inv_src)
    progress = [0]
    show_progress = hasattr(sys.stderr, "isatty") and sys.stderr.isatty()

    def tick():
        progress[0] += 1
        if show_progress:
            sys.stderr.write("\r  conversion : %d/%d" % (progress[0], progress_total))
            if progress[0] == progress_total:
                sys.stderr.write("\n")
            sys.stderr.flush()

    def conv_image(png_name, what, is_inventory=False):
        if not png_name:
            return None
        check_asset_name(png_name, what)
        key = (png_name, is_inventory)
        if key in img_cache:
            return img_cache[key]
        entry = lookup("images/" + png_name)
        if entry is None:
            _warn("image manquante:", png_name)
            missing.append(("image", png_name))
            img_cache[key] = None
            return None
        data = zf.read(entry)
        try:
            if is_inventory:
                # icones inventaire : 128x128 max (taille native bornee avec --keep-size)
                if keep_size:
                    lif_bytes = png_to_lif(data, None)
                else:
                    lif_bytes = png_to_fit_lif(data, INV_W, INV_H)
            else:
                lif_bytes = png_to_lif(data, target)
        except Exception as e:
            _warn("image illisible %s (%s: %s) -> empty.lif" % (png_name, type(e).__name__, e))
            bad.append(png_name)
            img_cache[key] = "empty.lif"
            return "empty.lif"
        lif_name = alloc("img/", base_no_ext(png_name) + ("_inv" if is_inventory else ""), ".lif")
        out_files["img/" + lif_name] = lif_bytes
        img_cache[key] = lif_name
        stats["img"] += 1
        return lif_name

    def add_mp3map(pk_name, data, label):
        try:
            info = {}
            mm, _dur, _n = mp3map.build(data, info)
            out_files[pk_name + "map"] = mm
            stats["mp3map"] += 1
            other = sorted(sr for sr in info.get("samplerates", ()) if sr != 44100)
            if other:
                _warn("%s : %s Hz (le firmware est calibre pour 44100 Hz ; "
                      "reechantillonner conseille)" % (label, "/".join(map(str, other))))
        except Exception as e:
            _warn("mp3map echoue pour", label, ":", e)

    def conv_audio(mp3_name, what):
        if not mp3_name:
            return None
        check_asset_name(mp3_name, what)
        if mp3_name in audio_cache:
            return audio_cache[mp3_name]
        entry = lookup("audios/" + mp3_name)
        if entry is None:
            # Repli sur silent.mp3 : la scene garde son audio (donc sa transition
            # sur fin d'audio) au lieu de devenir un noeud de passage.
            _warn("audio manquant:", mp3_name, "-> silent.mp3")
            missing.append(("audio", mp3_name))
            audio_cache[mp3_name] = "silent.mp3"
            return "silent.mp3"
        data = zf.read(entry)
        out_name = alloc("sounds/", base_no_ext(mp3_name), ".mp3")
        out_files["sounds/" + out_name] = data
        add_mp3map("sounds/" + out_name, data, mp3_name)
        audio_cache[mp3_name] = out_name
        stats["audio"] += 1
        return out_name

    # --- Stages ---
    lua_stages = {}
    for sid, st in stages_src.items():
        what = "stage %s" % sid
        _expect(st, dict, what, optional=False)
        entry = {}
        img = conv_image(_expect(st.get("image"), str, what + ".image"), what + ".image")
        if img:
            entry["image"] = img
        aud = conv_audio(_expect(st.get("audio"), str, what + ".audio"), what + ".audio")
        if aud:
            entry["audio"] = aud
        ok = st.get("ok")
        if ok:
            entry["ok"] = _conv_trans(ok, what + ".ok")
        home = st.get("home")
        if home:
            entry["home"] = _conv_trans(home, what + ".home")
        ctrl = _expect(st.get("control"), dict, what + ".control") or {}
        entry["ctrl"] = {
            "ok": bool(ctrl.get("ok", True)),
            "home": bool(ctrl.get("home", False)),
            "autoplay": bool(ctrl.get("autoplay", False)),
        }
        items = _expect(st.get("items"), list, what + ".items")
        if items:
            entry["items"] = [_conv_item(it, "%s.items[%d]" % (what, i)) for i, it in enumerate(items)]
        if st.get("inventoryReset"):
            entry["reset"] = True
        # Label court du choix (carrousel) : notes.json title, sinon text, sinon notes.
        label = _choice_label(notes.get(sid))
        if label:
            entry["text"] = label
        lua_stages[sid] = entry
        tick()

    # --- Actions ---
    lua_actions = {}
    for aid, lst in (_expect(nodes.get("actions", {}), dict, "nodes.json actions") or {}).items():
        what = "action %s" % aid
        _expect(lst, list, what, optional=False)
        out_list = []
        for i, e in enumerate(lst):
            w = "%s[%d]" % (what, i)
            _expect(e, dict, w, optional=False)
            item = {"stage": _expect(e.get("stage"), str, w + ".stage", optional=False)}
            conds = _expect(e.get("conditions"), list, w + ".conditions")
            if conds:
                item["cond"] = [_conv_cond(c, "%s.conditions[%d]" % (w, j)) for j, c in enumerate(conds)]
            out_list.append(item)
        lua_actions[aid] = out_list

    # --- Inventaire ---
    lua_inventory = None
    if inv_src:
        lua_inventory = []
        for i, it in enumerate(inv_src):
            what = "inventory[%d]" % i
            _expect(it, dict, what, optional=False)
            img = conv_image(_expect(it.get("image"), str, what + ".image"), what + ".image",
                             is_inventory=True)
            name = it.get("name", "")
            lua_inventory.append({
                "name": _flat(name) if isinstance(name, (str, int, float)) else "",
                "init": _expect(it.get("initialNumber", 0), (int, float), what + ".initialNumber",
                                optional=False),
                "max": _expect(it.get("maxNumber", 0), (int, float), what + ".maxNumber",
                               optional=False),
                "display": _expect(it.get("display", 0), int, what + ".display", optional=False),
                "image": img,
            })
            tick()

    # --- Titre + vignette ---
    lua_title = None
    cover_name = metadata.get("image") if isinstance(metadata.get("image"), str) else None
    if cover_name:
        check_asset_name(cover_name, "metadata.json image")
    cover_bytes = (read(cover_name) if cover_name else None) or read("title.png") or read("cover.png")
    title_audio = None
    title_mp3 = read("title.mp3")
    if title_mp3 is not None:
        out_files["sounds/title.mp3"] = title_mp3
        add_mp3map("sounds/title.mp3", title_mp3, "title.mp3")
        title_audio = "title.mp3"
        stats["audio"] += 1
    title_image = None
    if cover_bytes is not None:
        try:
            # title-card place le cover a taille native dans un slot ~89x120 (bas-droite,
            # ancre a gauche) : un cover portrait est complete a droite en transparent.
            title_lif = png_to_fit_lif(cover_bytes, 89, 120, align="left")
            # vignette de la bibliotheque
            thumb_lif = png_to_thumb_lif(cover_bytes)
        except Exception as e:
            _warn("couverture illisible (%s: %s) : pas de titre illustre ni de vignette"
                  % (type(e).__name__, e))
            bad.append(cover_name or "title.png")
        else:
            out_files["img/title.lif"] = title_lif
            out_files["img/thumbnail.lif"] = thumb_lif
            stats["img"] += 2
            title_image = "title.lif"
    if title_image or title_audio:
        lua_title = {}
        if title_image:
            lua_title["image"] = title_image
        if title_audio:
            lua_title["audio"] = title_audio

    # --- Assets manquants : echec sauf --allow-missing ---
    if missing:
        msg = "%d asset(s) manquant(s) : %s" % (
            len(missing), ", ".join("%s %s" % m for m in missing[:5])
            + (" ..." if len(missing) > 5 else ""))
        if not allow_missing:
            raise ConvError(msg + " (utiliser --allow-missing pour convertir quand meme)")
        _warn(msg + " (--allow-missing : images omises, audios remplaces par silent.mp3)")

    # --- startAction ---
    sa = _expect(nodes.get("startAction"), dict, "nodes.json startAction", optional=False)
    lua_start = {"action": _expect(sa.get("action"), str, "startAction.action", optional=False),
                 "index": _expect(sa.get("index", 0), int, "startAction.index", optional=False)}

    # --- nodes.lua ---
    # "Chapitres" = scenes avec audio (= contenu ecoute). Le format TELMI n'a
    # pas de notion de chapitre ; les scenes sans audio sont du pur routage
    # (transitions), on ne les compte pas. totalChapters sert au calcul de la
    # progression (getProgressionValue = #chapitres_visites / totalChapters).
    # Seules les scenes entrees par enterStage (story.lua) enregistrent un
    # chapitre : une option choisie dans le carrousel (showChoice) suit
    # directement son ok sans passer par enterStage. On ne compte donc que les
    # scenes avec audio pouvant etre jouees comme scene, sinon la jauge ne peut
    # jamais atteindre 100 %. Regles d'enterAction/followTransition : action a
    # 1 entree, entree cible d'un indexItem, action avec conditions (le filtre
    # peut ramener a 1 entree : prudence), ou scene cible autoplay.
    item_actions = set()
    for e in lua_stages.values():
        for key in ("ok", "home"):
            tr = e.get(key)
            if tr and "indexItem" in tr:
                item_actions.add(tr["action"])
    playable = set()
    for aid, lst in lua_actions.items():
        direct = (len(lst) <= 1 or aid in item_actions
                  or any(it.get("cond") for it in lst))
        for it in lst:
            st = lua_stages.get(it["stage"])
            if st and (direct or st["ctrl"]["autoplay"]):
                playable.add(it["stage"])
    audio_stage_count = sum(1 for sid, e in lua_stages.items()
                            if e.get("audio") and sid in playable)
    data_table = {
        "meta": {"title": title, "subtitle": subtitle_src},
        "totalChapters": max(1, audio_stage_count),
        "start": lua_start,
        "title": lua_title,
        "inventory": lua_inventory,
        "stages": lua_stages,
        "actions": lua_actions,
        "selector": selector,
    }
    try:
        body = lua_repr(data_table, 0)
    except ValueError as e:
        raise ConvError("nodes.lua : %s" % e)
    nodes_lua = "-- Genere par telmi2flam.py — ne pas editer a la main\nreturn " + body + "\n"
    # Dans script/ : c'est la convention des histoires officielles ; le firmware
    # reel ne resout require() que depuis script/ (le simulateur cherche aussi la racine).
    out_files["script/nodes.lua"] = nodes_lua.encode("utf-8")

    # --- main.lua (bootstrap) + script/story.lua (branch) ---
    with open(ENGINE_MAIN, "rb") as f:
        out_files["main.lua"] = f.read()
    with open(ENGINE_STORY, "rb") as f:
        out_files["script/story.lua"] = f.read()

    # --- bibliotheque standard Lunii (framework) : requise par le firmware reel ---
    # sans elle, main.lua echoue des setup() (module 'global' not found)
    nlib = 0
    _require_runtime(RUNTIME_DIR, RUNTIME_SCRIPTS, "scripts framework")
    _require_runtime(RUNTIME_IMG_DIR, RUNTIME_UI_LIF, "assets UI")
    if os.path.isdir(RUNTIME_DIR):
        for fn in sorted(os.listdir(RUNTIME_DIR)):
            if fn.endswith(".lua"):
                with open(os.path.join(RUNTIME_DIR, fn), "rb") as f:
                    out_files["script/" + fn] = f.read()
                nlib += 1
    stats["lib"] = nlib

    # --- assets UI Lunii (fleches, play/pause, etc.) requis par les modules ---
    nui = 0
    if os.path.isdir(RUNTIME_IMG_DIR):
        for fn in sorted(os.listdir(RUNTIME_IMG_DIR)):
            if fn.endswith(".lif"):
                with open(os.path.join(RUNTIME_IMG_DIR, fn), "rb") as f:
                    out_files["img/script/" + fn] = f.read()
                nui += 1
    stats["ui"] = nui

    # --- cover par defaut audio-player : petite image transparente ---
    out_files["img/empty.lif"] = lif.encode(bytes(8 * 8 * 4), 8, 8)
    stats["img"] += 1

    # --- MP3 silencieux pour les entrees de menu (list-choice exige un audio
    #     string par entree, sinon il rejette l'entree -> menu vide -> crash) ---
    # Frame MPEG1 Layer III, 128 kbps, 44100 Hz, stereo, donnees a zero = silence.
    _silent_frame = bytes([0xFF, 0xFB, 0x90, 0x00]) + bytes(413)
    silent_mp3 = _silent_frame * 24   # ~0.6 s
    out_files["sounds/silent.mp3"] = silent_mp3
    try:
        mm, _d, _n = mp3map.build(silent_mp3)
        out_files["sounds/silent.mp3map"] = mm
    except Exception:
        pass

    # --- Metadonnees FLAM (format de reference : titre / sous-titre / id / titre) ---
    # title/subtitle sont aplatis sur une ligne : info.plain est lu ligne par ligne.
    subtitle = subtitle_src or title
    info = "%s\n%s\n000000\n%s" % (title, subtitle, title)
    out_files["info.plain"] = info.encode("utf-8")
    out_files["version"] = b"1"
    out_files["uuid.bin"] = uuid16

    # --- Nom de sortie ---
    if out_path is None:
        safe = re.sub(r"[^A-Za-z0-9_-]+", "_", title).strip("_") or "story"
        uhex = uuid16[:4].hex().upper()
        out_path = "%s.%s.plain.pk" % (safe, uhex)

    # -o pointant un dossier ".plain" (sans .pk) => on n'ecrit QUE le dossier extrait.
    dir_only = out_path.endswith(".plain")

    # Ordre canonique : metadonnees + code d'abord, assets tries ensuite,
    # uuid.bin en dernier. (Non destructif : on n'enleve rien de out_files,
    # pour pouvoir ecrire le .pk ET le dossier a partir des memes donnees.)
    head = ["info.plain", "version", "main.lua", "script/nodes.lua"]
    order = [n for n in head if n in out_files]
    order += sorted(n for n in out_files if n not in head and n != "uuid.bin")
    if "uuid.bin" in out_files:
        order.append("uuid.bin")
    # Garde-fou zip-slip : tous les noms de sortie sont relatifs et sans ".."
    for name in order:
        parts = name.split("/")
        if name.startswith("/") or "\\" in name or ":" in name or ".." in parts or "" in parts:
            raise ConvError("nom de sortie refuse : %r" % name)

    written = []

    # --- Ecriture .plain.pk (ZIP stored, format device) ---
    if not dir_only:
        os.makedirs(os.path.dirname(os.path.abspath(out_path)) or ".", exist_ok=True)
        with zipfile.ZipFile(out_path, "w", zipfile.ZIP_STORED) as out:
            for name in order:
                out.writestr(name, out_files[name])
        written.append(out_path)

    # --- Ecriture du dossier .plain extrait (option --plain, ou -o *.plain) ---
    #     Pratique pour le simulateur, qui charge un dossier .plain directement.
    if emit_plain or dir_only:
        if dir_only:
            plain_dir = out_path
        elif out_path.endswith(".plain.pk"):
            plain_dir = out_path[:-3]            # ".plain.pk" -> ".plain"
        else:
            plain_dir = out_path + ".plain"
        real_dir = os.path.realpath(plain_dir)
        for name in order:
            dest = os.path.join(plain_dir, *name.split("/"))
            if os.path.commonpath([real_dir, os.path.realpath(dest)]) != real_dir:
                raise ConvError("ecriture hors du dossier .plain refusee : %r" % name)
            os.makedirs(os.path.dirname(dest) or ".", exist_ok=True)
            with open(dest, "wb") as fh:
                fh.write(out_files[name])
        written.append(plain_dir + os.sep)

    # --- Resume ---
    for w in written:
        print("OK -> %s" % w)
    print("  titre   : %s" % title)
    print("  stages  : %d | actions : %d" % (len(lua_stages), len(lua_actions)))
    print("  images  : %d | audio : %d | mp3map : %d"
          % (stats["img"], stats["audio"], stats["mp3map"]))
    print("  lib     : %d scripts framework | %d assets UI" % (stats.get("lib", 0), stats.get("ui", 0)))
    print("  choix   : selecteur '%s'" % selector)
    if missing or bad:
        print("  alertes : %d manquant(s) | %d illisible(s)" % (len(missing), len(bad)))
    if not dir_only:
        print("  taille  : %.1f Mo" % (os.path.getsize(out_path) / 1e6))
    return out_path


def main():
    ap = argparse.ArgumentParser(description="Convertit une histoire TELMI (.zip) en FLAM (.plain.pk)")
    ap.add_argument("input", help="archive TELMI .zip")
    ap.add_argument("-o", "--output",
                    help="sortie : fichier .plain.pk (defaut) ou dossier .plain")
    ap.add_argument("--plain", action="store_true",
                    help="ecrire AUSSI le dossier .plain extrait a cote du .plain.pk (pour le simulateur)")
    ap.add_argument("--keep-size", action="store_true",
                    help="garder la resolution native des images (pas de resize 320x240, "
                         "bornee a 2047 px)")
    ap.add_argument("--allow-missing", action="store_true",
                    help="convertir malgre des images/audios manquants (audio -> silent.mp3)")
    ap.add_argument("--selector", choices=["carousel", "image"], default="carousel",
                    help="affichage des choix multiples : carousel (defaut, 3 vignettes + "
                         "label) ou image (image-choice plein ecran d'origine)")
    args = ap.parse_args()
    try:
        convert(args.input, args.output, args.keep_size, args.plain, args.selector,
                args.allow_missing)
    except ConvError as e:
        print("ERREUR : %s" % e, file=sys.stderr)
        sys.exit(1)
    except (zipfile.BadZipFile, OSError) as e:
        print("ERREUR : %s : %s" % (args.input, e), file=sys.stderr)
        sys.exit(1)
    except Exception as e:  # paquet imprevu : message court plutot qu'un traceback
        print("ERREUR inattendue (%s) : %s" % (type(e).__name__, e), file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
