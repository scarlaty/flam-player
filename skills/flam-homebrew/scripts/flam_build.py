#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Assemble un projet homebrew FLAM en .plain.pk (device) + dossier .plain (simulateur).

Usage :
    python flam_build.py <projet> [-o sortie.plain.pk] [--no-plain] [--no-check]
                             [--thumb-fit flou|rogner|bandes]

Projet (cree par new_project.py) :
    flam.json      {"title", "subtitle", "uuid_seed", "module", "version"}
    main.lua       bootstrap (setup -> Global.load_module(module, version).create)
    script/*.lua   code du jeu (modules), a plat (pas de sous-dossier)
    img/**         .png (converti en .lif) ou .lif ; img/thumbnail.png = vignette
    sounds/*.mp3   audio (+ .mp3map genere)

Le paquet recoit en plus le runtime Lua du projet (assets/runtime : script/*.lua,
img/script/*.lif), img/empty.lif, sounds/silent.mp3(+map) et les metadonnees
(info.plain, version, uuid.bin). Archive ZIP STORED, ordre : info.plain, version,
main.lua, script du module, puis le reste trie, uuid.bin en dernier.
Sans --no-check, flam_check.py est lance sur le .pk produit (code retour != 0 si erreur).
"""
import argparse
import hashlib
import io
import json
import os
import re
import sys
import unicodedata
import zipfile

for _s in (sys.stdout, sys.stderr):
    try:
        _s.reconfigure(errors="replace")   # console Windows cp1252 : pas de crash
    except AttributeError:
        pass

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import lif       # noqa: E402
import mp3map    # noqa: E402

RUNTIME_SCRIPT = os.path.join(HERE, "..", "assets", "runtime", "script")
RUNTIME_IMG = os.path.join(HERE, "..", "assets", "runtime", "img", "script")

MAX_DIM = 2047            # borne du decodeur LIF
# Vignette de bibliotheque : 104x136 (portrait), taille des vignettes des histoires officielles
THUMB_W, THUMB_H = 104, 136
THUMB_FITS = ("flou", "rogner", "bandes")
DEFAULT_THUMB_FIT = "flou"
_THUMB_BLUR_RADIUS = 8
ZIP_DATE = (2020, 1, 1, 0, 0, 0)   # date fixe : build reproductible
_SAFE_PART = re.compile(r"^[A-Za-z0-9_][A-Za-z0-9_.-]*\Z")


class BuildError(Exception):
    pass


# ---------------------------------------------------------------------------
# Texte : polices du player = ASCII imprimable + Latin-1 (U+0020-007E, U+00A0-00FF)
# ---------------------------------------------------------------------------
_FONT_MAP = {
    "‘": "'", "’": "'", "‚": "'", "‛": "'", "′": "'",
    "‹": "<", "›": ">", "“": '"', "”": '"', "„": '"',
    "‟": '"', "″": '"', "‐": "-", "‑": "-", "‒": "-",
    "–": "-", "—": "-", "―": "-", "−": "-", "•": "-",
    "…": "...", "œ": "oe", "Œ": "OE", "€": "EUR", "™": "TM",
}


def in_font(ch):
    o = ord(ch)
    return 0x20 <= o <= 0x7E or 0xA0 <= o <= 0xFF


def font_safe(s):
    """Texte affichable : accents Latin-1 gardes, typographie remplacee, emoji supprimes."""
    out = []
    for ch in unicodedata.normalize("NFC", str(s)):
        if in_font(ch):
            out.append(ch)
        elif ch.isspace():
            out.append(" ")
        elif ch in _FONT_MAP:
            out.append(_FONT_MAP[ch])
        else:
            base = "".join(c for c in unicodedata.normalize("NFKD", ch)
                           if not unicodedata.combining(c))
            if base and all(in_font(c) for c in base):
                out.append(base)
            elif unicodedata.category(ch)[0] in "LN":
                out.append("?")
    return " ".join("".join(out).split())


# ---------------------------------------------------------------------------
# Images
# ---------------------------------------------------------------------------
def _pil():
    try:
        from PIL import Image
    except ImportError:
        raise BuildError("Pillow requis pour convertir les PNG : pip install pillow")
    return Image


def _encode(im):
    return lif.encode(im.tobytes(), im.size[0], im.size[1])


def png_to_lif(data, name):
    """PNG -> LIF a taille native (bornee a 2047 px). Une image portrait de plus
    de 64 px est posee a gauche d'un canevas carre transparent : le decodeur
    transpose sinon l'image (heuristique h > w et max > 64)."""
    Image = _pil()
    im = Image.open(io.BytesIO(data))
    im.load()
    im = im.convert("RGBA")
    if im.size[0] > MAX_DIM or im.size[1] > MAX_DIM:
        im.thumbnail((MAX_DIM, MAX_DIM), Image.LANCZOS)
        print("  ! %s reduite a %dx%d (max %d px)" % (name, im.size[0], im.size[1], MAX_DIM))
    w, h = im.size
    if h > w and h > 64:
        canvas = Image.new("RGBA", (h, h), (0, 0, 0, 0))
        canvas.paste(im, (0, 0))
        im = canvas
        print("  ! %s portrait %dx%d -> canevas %dx%d (contenu a gauche)" % (name, w, h, h, h))
    return _encode(im)


def _scaled(im, scale):
    Image = _pil()
    nw, nh = max(1, round(im.size[0] * scale)), max(1, round(im.size[1] * scale))
    return im if (nw, nh) == im.size else im.resize((nw, nh), Image.LANCZOS)


def _thumb_cover(im):
    """Couvre THUMB_W x THUMB_H : fenetre centrale de la source au rapport de la vignette
    (evite une image intermediaire geante sur un ratio extreme), puis redimensionnee."""
    sw, sh = im.size
    if sw * THUMB_H > sh * THUMB_W:
        cw, ch = max(1, round(sh * THUMB_W / THUMB_H)), sh
    else:
        cw, ch = sw, max(1, round(sw * THUMB_H / THUMB_W))
    left, top = (sw - cw) // 2, (sh - ch) // 2
    return im.crop((left, top, left + cw, top + ch)).resize((THUMB_W, THUMB_H), _pil().LANCZOS)


def _thumb_contain(im):
    """Image entiere (agrandie si besoin), centree sur un canevas transparent."""
    Image = _pil()
    im = _scaled(im, min(THUMB_W / im.size[0], THUMB_H / im.size[1]))
    canvas = Image.new("RGBA", (THUMB_W, THUMB_H), (0, 0, 0, 0))
    canvas.paste(im, ((THUMB_W - im.size[0]) // 2, (THUMB_H - im.size[1]) // 2))
    return canvas


def png_to_thumb(data, fit=DEFAULT_THUMB_FIT):
    """Vignette bibliotheque 104x136. fit : flou (image entiere centree, fond = la meme
    image agrandie pour couvrir puis floutee), rogner (cover centre), bandes (image
    entiere centree, bandes transparentes)."""
    if fit not in THUMB_FITS:
        raise BuildError("remplissage de vignette inconnu : %r (attendu : %s)"
                         % (fit, ", ".join(THUMB_FITS)))
    Image = _pil()
    from PIL import ImageFilter
    im = Image.open(io.BytesIO(data))
    im.load()
    im = im.convert("RGBA")
    if fit == "rogner":
        out = _thumb_cover(im)
    elif fit == "bandes":
        out = _thumb_contain(im)
    else:
        # fond opaque : flouter du RGBA non premultiplie laisserait des halos sombres
        bg = _thumb_cover(im)
        base = Image.new("RGBA", bg.size, (128, 128, 128, 255))
        bg = Image.alpha_composite(base, bg).filter(ImageFilter.GaussianBlur(_THUMB_BLUR_RADIUS))
        out = Image.alpha_composite(bg, _thumb_contain(im))
    return _encode(out)


# ---------------------------------------------------------------------------
def _check_name(rel):
    for part in rel.split("/"):
        if not _SAFE_PART.match(part):
            raise BuildError("nom refuse %r : n'utiliser que [A-Za-z0-9_.-] (pas d'espace, "
                             "pas d'accent) dans les noms de fichiers/dossiers" % rel)
    base = rel.split("/")[-1]
    # Lunii.QT renomme *.lua -> *.lsf (replace sur tout le nom) et retire un suffixe .plain
    stem = rel[:-4] if rel.endswith(".lua") else rel
    if ".lua" in stem:
        raise BuildError("nom refuse %r : '.lua' seulement en extension finale" % rel)
    if base.endswith(".plain"):
        raise BuildError("nom refuse %r : suffixe .plain reserve a info.plain" % rel)


def _walk(root):
    for dp, dns, fns in os.walk(root):
        dns[:] = sorted(d for d in dns if not d.startswith("."))
        for fn in sorted(fns):
            if fn.startswith(".") or fn.lower() in ("thumbs.db", "desktop.ini"):
                continue
            p = os.path.join(dp, fn)
            yield p, os.path.relpath(p, root).replace(os.sep, "/")


def _read(p):
    with open(p, "rb") as f:
        return f.read()


def build(project, thumb_fit=DEFAULT_THUMB_FIT):
    cfg_path = os.path.join(project, "flam.json")
    if not os.path.isfile(cfg_path):
        raise BuildError("%s introuvable (creer le projet avec new_project.py)" % cfg_path)
    with open(cfg_path, "r", encoding="utf-8-sig") as f:
        cfg = json.load(f)
    if not isinstance(cfg, dict):
        raise BuildError("flam.json : objet JSON attendu")
    for k in ("title", "uuid_seed", "module", "version"):
        if not isinstance(cfg.get(k), str) or not cfg[k].strip():
            raise BuildError("flam.json : champ %r (chaine non vide) requis" % k)
    title = font_safe(cfg["title"])
    subtitle = font_safe(cfg.get("subtitle") or "") or title
    if not title:
        raise BuildError("flam.json : titre vide apres normalisation police")
    if title != cfg["title"].strip():
        print("  ! titre normalise pour la police : %r" % title)
    module_file = "script/%s_%s.lua" % (cfg["module"], cfg["version"])

    files = {}   # nom dans le .pk -> bytes
    origin = {}  # nom -> "projet" | "runtime"

    def add(name, data, src):
        key = name.lower()
        for other in files:
            if other.lower() == key:
                raise BuildError("%s (%s) entre en collision avec %s (%s) : renommer"
                                 % (name, src, other, origin[other]))
        files[name] = data
        origin[name] = src

    # --- runtime (Lua du projet) ---
    for p, rel in _walk(RUNTIME_SCRIPT):
        if rel.endswith(".lua") and "/" not in rel:
            add("script/" + rel, _read(p), "runtime")
    for p, rel in _walk(RUNTIME_IMG):
        if rel.endswith(".lif") and "/" not in rel:
            add("img/script/" + rel, _read(p), "runtime")
    add("img/empty.lif", _read(os.path.join(RUNTIME_IMG, "empty.lif")), "runtime")
    silent = (bytes([0xFF, 0xFB, 0x90, 0x00]) + bytes(413)) * 24   # ~0.6 s, 44.1 kHz
    add("sounds/silent.mp3", silent, "runtime")
    add("sounds/silent.mp3map", mp3map.build(silent)[0], "runtime")

    # --- code du jeu ---
    main_lua = os.path.join(project, "main.lua")
    if not os.path.isfile(main_lua):
        raise BuildError("main.lua absent du projet")
    add("main.lua", _read(main_lua), "projet")
    sdir = os.path.join(project, "script")
    if os.path.isdir(sdir):
        for p, rel in _walk(sdir):
            if "/" in rel:
                raise BuildError("script/%s : pas de sous-dossier dans script/ (require ne "
                                 "resout que script/<nom>.lua)" % rel)
            if not rel.endswith(".lua"):
                print("  ! script/%s ignore (seuls les .lua sont embarques)" % rel)
                continue
            _check_name("script/" + rel)
            add("script/" + rel, _read(p), "projet")
    if module_file not in files:
        raise BuildError("%s absent : flam.json annonce module=%r version=%r"
                         % (module_file, cfg["module"], cfg["version"]))

    # --- images ---
    nimg = 0
    idir = os.path.join(project, "img")
    if os.path.isdir(idir):
        for p, rel in _walk(idir):
            low = rel.lower()
            if low.startswith("script/"):
                raise BuildError("img/script/ est reserve au runtime : deplacer img/%s" % rel)
            if low == "thumbnail.png":
                add("img/thumbnail.lif", png_to_thumb(_read(p), thumb_fit), "projet")
                nimg += 1
                continue
            if low.endswith(".png"):
                out = rel[:-4] + ".lif"
                _check_name("img/" + out)
                add("img/" + out, png_to_lif(_read(p), "img/" + rel), "projet")
                nimg += 1
            elif low.endswith(".lif"):
                _check_name("img/" + rel)
                add("img/" + rel, _read(p), "projet")
                nimg += 1
            else:
                raise BuildError("img/%s : format non supporte (PNG ou LIF)" % rel)

    # --- audio ---
    naud = 0
    adir = os.path.join(project, "sounds")
    if os.path.isdir(adir):
        for p, rel in _walk(adir):
            low = rel.lower()
            if low.endswith(".mp3map"):
                continue          # regenere a partir du mp3
            if not low.endswith(".mp3"):
                raise BuildError("sounds/%s : seul le MP3 est lu par la Flam "
                                 "(convertir : MPEG1 Layer III, 44100 Hz)" % rel)
            _check_name("sounds/" + rel)
            data = _read(p)
            info = {}
            try:
                mm = mp3map.build(data, info)[0]
            except ValueError as e:
                raise BuildError("sounds/%s : MP3 invalide (%s)" % (rel, e))
            other = sorted(sr for sr in info.get("samplerates", ()) if sr != 44100)
            if other:
                print("  ! sounds/%s : %s Hz (firmware calibre pour 44100 Hz, "
                      "reechantillonner)" % (rel, "/".join(map(str, other))))
            add("sounds/" + rel, data, "projet")
            add("sounds/" + rel + "map", mm, "projet")
            naud += 1

    # --- metadonnees ---
    files["info.plain"] = ("%s\n%s\n000000\n%s" % (title, subtitle, title)).encode("utf-8")
    files["version"] = b"1"
    files["uuid.bin"] = hashlib.md5(cfg["uuid_seed"].encode("utf-8")).digest()
    return files, cfg, title, module_file, nimg, naud


def write(files, module_file, pk_path, plain_dir):
    head = ["info.plain", "version", "main.lua", module_file]
    order = [n for n in head if n in files]
    order += sorted(n for n in files if n not in head and n != "uuid.bin")
    order.append("uuid.bin")
    os.makedirs(os.path.dirname(os.path.abspath(pk_path)) or ".", exist_ok=True)
    with zipfile.ZipFile(pk_path, "w", zipfile.ZIP_STORED) as z:
        for name in order:
            zi = zipfile.ZipInfo(name, date_time=ZIP_DATE)
            zi.compress_type = zipfile.ZIP_STORED
            z.writestr(zi, files[name])
    if plain_dir:
        for name in order:
            dest = os.path.join(plain_dir, *name.split("/"))
            os.makedirs(os.path.dirname(dest), exist_ok=True)
            with open(dest, "wb") as f:
                f.write(files[name])


def main():
    ap = argparse.ArgumentParser(description="Build d'un homebrew FLAM (.plain.pk)")
    ap.add_argument("project", help="dossier du projet (contient flam.json)")
    ap.add_argument("-o", "--output", help="chemin du .plain.pk (defaut : <projet>/_out/<titre>.plain.pk)")
    ap.add_argument("--no-plain", action="store_true", help="ne pas ecrire le dossier .plain (simulateur)")
    ap.add_argument("--thumb-fit", choices=THUMB_FITS, default=DEFAULT_THUMB_FIT,
                    help="remplissage de la vignette 104x136 : flou (defaut), rogner ou bandes")
    ap.add_argument("--no-check", action="store_true", help="ne pas lancer flam_check.py")
    args = ap.parse_args()
    try:
        files, cfg, title, module_file, nimg, naud = build(args.project, args.thumb_fit)
    except (BuildError, json.JSONDecodeError, OSError) as e:
        print("ERREUR : %s" % e, file=sys.stderr)
        return 1
    pk = args.output
    if pk is None:
        slug = re.sub(r"[^A-Za-z0-9_-]+", "-", unicodedata.normalize("NFKD", title)
                      .encode("ascii", "ignore").decode()).strip("-").lower() or "homebrew"
        pk = os.path.join(args.project, "_out", slug + ".plain.pk")
    if not pk.endswith(".plain.pk"):
        print("ERREUR : la sortie doit finir par .plain.pk", file=sys.stderr)
        return 1
    plain_dir = None if args.no_plain else pk[:-3]
    if plain_dir and os.path.isdir(plain_dir):
        # Ne supprimer qu'une ancienne sortie de build (jamais un dossier quelconque)
        if not (os.path.isfile(os.path.join(plain_dir, "info.plain"))
                and os.path.isfile(os.path.join(plain_dir, "uuid.bin"))
                and not os.path.isfile(os.path.join(plain_dir, "flam.json"))):
            print("ERREUR : %s existe et n'est pas une sortie de build : choisir un autre -o "
                  "ou --no-plain" % plain_dir, file=sys.stderr)
            return 1
        import shutil
        shutil.rmtree(plain_dir)       # pas de fichiers perimes d'un build precedent
    write(files, module_file, pk, plain_dir)
    print("OK -> %s" % pk)
    if plain_dir:
        print("OK -> %s%s" % (plain_dir, os.sep))
    print("  titre : %s | module : %s | images : %d | audios : %d | %d entrees | %.0f Ko"
          % (title, module_file, nimg, naud, len(files), os.path.getsize(pk) / 1024))
    print("  uuid  : %s" % files["uuid.bin"].hex())
    if args.no_check:
        return 0
    import flam_check
    return flam_check.check(pk)


if __name__ == "__main__":
    sys.exit(main())
