#!/usr/bin/env python3
"""
telmi2flam.py — Convertisseur d'histoires TELMI (.zip) -> FLAM (.plain.pk).

Usage :
    python telmi2flam.py <histoire-telmi.zip> [-o sortie.plain.pk] [--keep-size]

Voir README.md pour les details des formats.
"""

import argparse
import hashlib
import io
import json
import os
import re
import sys
import zipfile

from PIL import Image

import lif
import mp3map

SCREEN_W, SCREEN_H = 320, 240
THUMB_W, THUMB_H = 128, 96
ENGINE_MAIN = os.path.join(os.path.dirname(os.path.abspath(__file__)), "engine", "main.lua")
ENGINE_STORY = os.path.join(os.path.dirname(os.path.abspath(__file__)), "engine", "story.lua")
RUNTIME_DIR = os.path.join(os.path.dirname(os.path.abspath(__file__)), "runtime", "script")
RUNTIME_IMG_DIR = os.path.join(os.path.dirname(os.path.abspath(__file__)), "runtime", "img", "script")


# ---------------------------------------------------------------------------
# Serialisation Lua
# ---------------------------------------------------------------------------
_LUA_ID = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")


def lua_repr(value, indent=0):
    pad = "  " * indent
    pad1 = "  " * (indent + 1)
    if value is None:
        return "nil"
    if value is True:
        return "true"
    if value is False:
        return "false"
    if isinstance(value, (int, float)):
        return repr(value)
    if isinstance(value, str):
        s = value.replace("\\", "\\\\").replace('"', '\\"')
        s = s.replace("\n", "\\n").replace("\r", "\\r")
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
            if isinstance(k, str) and _LUA_ID.match(k):
                key = k
            else:
                key = "[" + lua_repr(k) + "]"
            items.append(pad1 + key + " = " + lua_repr(v, indent + 1))
        return "{\n" + ",\n".join(items) + "\n" + pad + "}"
    raise TypeError("type non serialisable: %r" % type(value))


# ---------------------------------------------------------------------------
# Conversion des assets
# ---------------------------------------------------------------------------
def png_to_lif(png_bytes, target=None):
    """Convertit un PNG en .lif. target=(w,h) pour redimensionner, None = taille native."""
    im = Image.open(io.BytesIO(png_bytes)).convert("RGBA")
    if target is not None and im.size != target:
        im = im.resize(target, Image.LANCZOS)
    w, h = im.size
    return lif.encode(im.tobytes(), w, h)


def png_to_fit_lif(png_bytes, maxw, maxh):
    """Redimensionne en gardant le ratio pour tenir dans maxw x maxh."""
    im = Image.open(io.BytesIO(png_bytes)).convert("RGBA")
    im.thumbnail((maxw, maxh), Image.LANCZOS)
    w, h = im.size
    return lif.encode(im.tobytes(), w, h)


def png_to_thumb_lif(png_bytes):
    """Vignette : on tient dans THUMB_W x THUMB_H en gardant le ratio, paysage (w>h)."""
    im = Image.open(io.BytesIO(png_bytes)).convert("RGBA")
    im.thumbnail((THUMB_W, THUMB_H), Image.LANCZOS)
    w, h = im.size
    if h >= w:   # eviter le chemin transpose du decodeur (h>w) : forcer paysage
        nh = max(1, int(h * (THUMB_W / w)))  # ne devrait pas arriver pour du 4:3
    return lif.encode(im.tobytes(), w, h)


# ---------------------------------------------------------------------------
# Convertisseur principal
# ---------------------------------------------------------------------------
def base_no_ext(name):
    return os.path.splitext(os.path.basename(name))[0]


def convert(zip_path, out_path=None, keep_size=False, emit_plain=False, selector="carousel"):
    zf = zipfile.ZipFile(zip_path, "r")
    names = zf.namelist()

    # Repertoire racine de l'histoire dans le zip
    root = ""
    for n in names:
        if n.endswith("metadata.json"):
            root = n[: -len("metadata.json")]
            break

    def read(name):
        full = root + name
        if full in zf.namelist():
            return zf.read(full)
        return None

    metadata = json.loads(read("metadata.json").decode("utf-8"))
    nodes = json.loads(read("nodes.json").decode("utf-8"))
    # notes.json (optionnel) : titres/textes narratifs par scene. Sert a donner
    # un label texte aux choix (carrousel). Absent dans beaucoup d'histoires.
    notes_raw = read("notes.json")
    notes = json.loads(notes_raw.decode("utf-8")) if notes_raw else {}

    title = metadata.get("title", "Histoire")
    uuid_str = metadata.get("uuid", title)
    uuid16 = hashlib.md5(uuid_str.encode("utf-8")).digest()  # 16 octets deterministes

    target = None if keep_size else (SCREEN_W, SCREEN_H)

    # Sorties accumulees : nom dans le .pk -> bytes
    out_files = {}
    stats = {"img": 0, "audio": 0, "mp3map": 0}

    # --- Index des entrees du zip par nom de fichier court ---
    def find_entry(subdir, filename):
        full = root + subdir + filename
        if full in zf.namelist():
            return full
        return None

    img_cache = {}    # "s7.png" -> "s7.lif"
    audio_cache = {}  # "s5.mp3" -> "s5.mp3"

    def conv_image(png_name, is_inventory=False):
        if not png_name:
            return None
        if png_name in img_cache:
            return img_cache[png_name]
        entry = find_entry("images/", png_name)
        if entry is None:
            print("  ! image manquante:", png_name, file=sys.stderr)
            img_cache[png_name] = None
            return None
        data = zf.read(entry)
        tgt = None if is_inventory else target  # icones inventaire : taille native
        lif_bytes = png_to_lif(data, tgt)
        lif_name = base_no_ext(png_name) + ".lif"
        out_files["img/" + lif_name] = lif_bytes
        img_cache[png_name] = lif_name
        stats["img"] += 1
        return lif_name

    def conv_audio(mp3_name):
        if not mp3_name:
            return None
        if mp3_name in audio_cache:
            return audio_cache[mp3_name]
        entry = find_entry("audios/", mp3_name)
        if entry is None:
            print("  ! audio manquant:", mp3_name, file=sys.stderr)
            audio_cache[mp3_name] = None
            return None
        data = zf.read(entry)
        out_files["sounds/" + mp3_name] = data
        try:
            mm, _dur, _n = mp3map.build(data)
            out_files["sounds/" + mp3_name + "map"] = mm
            stats["mp3map"] += 1
        except Exception as e:
            print("  ! mp3map echoue pour", mp3_name, ":", e, file=sys.stderr)
        audio_cache[mp3_name] = mp3_name
        stats["audio"] += 1
        return mp3_name

    # --- Stages ---
    lua_stages = {}
    for sid, st in nodes.get("stages", {}).items():
        entry = {}
        img = conv_image(st.get("image"))
        if img:
            entry["image"] = img
        aud = conv_audio(st.get("audio"))
        if aud:
            entry["audio"] = aud
        ok = st.get("ok")
        if ok:
            entry["ok"] = {"action": ok.get("action"), "index": ok.get("index", 0)}
        home = st.get("home")
        if home:
            entry["home"] = {"action": home.get("action"), "index": home.get("index", 0)}
        ctrl = st.get("control", {}) or {}
        entry["ctrl"] = {
            "ok": bool(ctrl.get("ok", True)),
            "home": bool(ctrl.get("home", False)),
            "autoplay": bool(ctrl.get("autoplay", False)),
        }
        items = st.get("items")
        if items:
            entry["items"] = [
                {"type": it.get("type", 0), "item": it.get("item", 0),
                 "number": it.get("number", 0)} for it in items
            ]
        if st.get("inventoryReset"):
            entry["reset"] = True
        # Label texte du choix (carrousel) : notes.json text sinon notes.
        note = notes.get(sid, {}) or {}
        label = (note.get("text") or "").strip() or (note.get("notes") or "").strip()
        if label:
            entry["text"] = label
        lua_stages[sid] = entry

    # --- Actions ---
    lua_actions = {}
    for aid, lst in nodes.get("actions", {}).items():
        out_list = []
        for e in lst:
            item = {"stage": e.get("stage")}
            conds = e.get("conditions")
            if conds:
                item["cond"] = [
                    {"cmp": c.get("comparator", 2), "item": c.get("item", 0),
                     "num": c.get("number", 0)} for c in conds
                ]
            out_list.append(item)
        lua_actions[aid] = out_list

    # --- Inventaire ---
    lua_inventory = None
    inv = nodes.get("inventory")
    if inv:
        lua_inventory = []
        for it in inv:
            img = conv_image(it.get("image"), is_inventory=True)
            lua_inventory.append({
                "name": it.get("name", ""),
                "init": it.get("initialNumber", 0),
                "max": it.get("maxNumber", 0),
                "display": it.get("display", 0),
                "image": img,
            })

    # --- Titre + vignette ---
    lua_title = None
    cover_name = metadata.get("image") or "cover.png"
    cover_bytes = read(cover_name) or read("title.png") or read("cover.png")
    title_audio = None
    if read("title.mp3") is not None:
        out_files["sounds/title.mp3"] = read("title.mp3")
        try:
            mm, _d, _n = mp3map.build(read("title.mp3"))
            out_files["sounds/title.mp3map"] = mm
            stats["mp3map"] += 1
        except Exception:
            pass
        title_audio = "title.mp3"
        stats["audio"] += 1
    title_image = None
    if cover_bytes is not None:
        # title-card place le cover a taille native dans un slot ~89x120 (bas-droite)
        out_files["img/title.lif"] = png_to_fit_lif(cover_bytes, 89, 120)
        stats["img"] += 1
        title_image = "title.lif"
        # vignette de la bibliotheque
        out_files["img/thumbnail.lif"] = png_to_thumb_lif(cover_bytes)
        stats["img"] += 1
    if title_image or title_audio:
        lua_title = {}
        if title_image:
            lua_title["image"] = title_image
        if title_audio:
            lua_title["audio"] = title_audio

    # --- startAction ---
    sa = nodes.get("startAction", {})
    lua_start = {"action": sa.get("action"), "index": sa.get("index", 0)}

    # --- nodes.lua ---
    # "Chapitres" = scenes avec audio (= contenu ecoute). Le format TELMI n'a
    # pas de notion de chapitre ; les scenes sans audio sont du pur routage
    # (transitions), on ne les compte pas. totalChapters sert au calcul de la
    # progression (getProgressionValue = #chapitres_visites / totalChapters).
    audio_stage_count = sum(1 for e in lua_stages.values() if e.get("audio"))
    data_table = {
        "meta": {"title": title, "subtitle": metadata.get("category", "")},
        "totalChapters": max(1, audio_stage_count),
        "start": lua_start,
        "title": lua_title,
        "inventory": lua_inventory,
        "stages": lua_stages,
        "actions": lua_actions,
        "selector": selector,
    }
    nodes_lua = "-- Genere par telmi2flam.py — ne pas editer a la main\nreturn " \
                + lua_repr(data_table, 0) + "\n"
    # Dans script/ : c'est la convention des histoires officielles ; le firmware
    # reel ne resout require() que depuis script/ (le simulateur cherche aussi la racine).
    out_files["script/nodes.lua"] = nodes_lua.encode("utf-8")

    # --- main.lua (bootstrap) + script/story.lua (branch) ---
    with open(ENGINE_MAIN, "rb") as f:
        out_files["main.lua"] = f.read()
    with open(ENGINE_STORY, "rb") as f:
        out_files["script/story.lua"] = f.read()

    # --- bibliotheque standard Lunii (framework) : requise par le firmware reel ---
    nlib = 0
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
    subtitle = metadata.get("category", "") or title
    info = "%s\n%s\n000000\n%s" % (title, subtitle, title)
    out_files["info.plain"] = info.encode("utf-8")
    out_files["version"] = b"1"
    out_files["uuid.bin"] = uuid16

    zf.close()

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
        for name in order:
            dest = os.path.join(plain_dir, *name.split("/"))
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
                    help="garder la resolution native des images (pas de resize 320x240)")
    ap.add_argument("--selector", choices=["carousel", "image"], default="carousel",
                    help="affichage des choix multiples : carousel (defaut, 3 vignettes + "
                         "label) ou image (image-choice plein ecran d'origine)")
    args = ap.parse_args()
    convert(args.input, args.output, args.keep_size, args.plain, args.selector)


if __name__ == "__main__":
    main()
