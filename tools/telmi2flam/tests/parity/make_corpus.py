"""
Generateur du corpus de parite (#123, note de conception PORT-RUST.md §4.3).

Produit, dans un dossier choisi par l'appelant (un dossier temporaire : rien n'est
commite, aucun contenu sous licence), deux jeux :

  1. le corpus synthetique : des archives TELMI reprenant les cas de
     tests/test_telmi2flam.py et tests/test_progress.py (modes PNG, tRNS, 16 bits,
     portraits, noms NFD et cp437, JSON limites, assets manquants, archives corrompues,
     MP3 MPEG1/2/2.5, ID3, trames cassees...). Index : `corpus.json`.
  2. la matrice de reechantillonnage : des PNG (modes x tailles sources impaires ou
     premieres x contenus varies) et, pour chacun, les operations d'image reelles du
     convertisseur (scene 320x212, vignette 104x136, inventaire 128x128, titre 89x120,
     borne 2047). Index : `matrix.json`. Le resultat attendu (`.lif`) se compare octet
     a octet : `python_lif()` fournit celui de l'oracle Python.

    python make_corpus.py DOSSIER [--only corpus|matrix] [--quick]

Les images et les paquets sont deterministes (graine fixe) : deux executions donnent
les memes octets.
"""
import argparse
import io
import json
import os
import random
import struct
import sys
import zipfile
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
TESTS = os.path.dirname(HERE)
TOOL = os.path.dirname(TESTS)
for _p in (TESTS, TOOL):
    if _p not in sys.path:
        sys.path.insert(0, _p)
sys.dont_write_bytecode = True

from PIL import Image  # noqa: E402

# Generateurs de paquets synthetiques des tests existants (reutilises, pas dupliques).
from test_telmi2flam import (MP3, base_nodes, mp3_frames, png, raw_png,  # noqa: E402
                             std_files)

SEED = 20260101
FIXED_DATE = (1980, 1, 1, 0, 0, 0)

# Etiquettes des cas (consommees par parity.py pour la liste fermee des ecarts admis)
TAG_JSON_LIMIT = "json_limit"          # §3 : JSON limites, divergence assumee du Rust (code 4)
TAG_ENCRYPTED = "encrypted_entry"      # §3 : entree chiffree (Python 6, Rust 3)
TAG_BAD_UTF8_NAME = "invalid_utf8_name"    # §4.1 : nom UTF-8 invalide, bit 11 (Python 6, Rust 3)
TAG_NEGATIVE_OFFSET = "negative_offset"    # §4.1 : position d'en-tete locale < 0 (Python 6, Rust 3)
TAG_DELEGATED = "delegated"            # D4 : image non PNG (Rust : code 7) ; bzip2/LZMA lus (#458)


# ---------------------------------------------------------------------------
# Archives
# ---------------------------------------------------------------------------
def zip_bytes(files, compress=zipfile.ZIP_STORED, encrypted=()):
    """Archive en memoire, horodatage fixe. `files` : liste (nom, octets | str).
    `compress` : methode par defaut, ou dict {nom: methode}. `encrypted` : noms dont le
    bit de chiffrement (bit 0) est pose apres ecriture, contenu inchange."""
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as z:
        for name, data in files:
            zi = zipfile.ZipInfo(name, FIXED_DATE)
            zi.compress_type = (compress.get(name, zipfile.ZIP_STORED)
                                if isinstance(compress, dict) else compress)
            z.writestr(zi, data.encode("utf-8") if isinstance(data, str) else data)
    data = bytearray(buf.getvalue())
    for name in encrypted:
        raw = name.encode("utf-8")
        i = data.find(raw)
        while i != -1:
            if data[i - 30:i - 26] == b"PK":      # en-tete local : drapeaux a +6
                data[i - 30 + 6] |= 0x01
            elif data[i - 46:i - 42] == b"PK":    # repertoire central : +8
                data[i - 46 + 8] |= 0x01
            i = data.find(raw, i + 1)
    return bytes(data)


def zip_bad_utf8_name(files, name="audios/s0.mp3"):
    """Archive dont l'entree `name` porte le bit 11 (UTF-8) et un nom d'octets non UTF-8
    (dernier octet 0xFF, meme longueur) dans l'en-tete local et le repertoire central."""
    data = bytearray(zip_bytes(files))
    raw = name.encode("utf-8")
    bad = raw[:-1] + b"\xff"
    count = 0
    i = data.find(raw)
    while i != -1:
        # i >= 30 / 46 : sinon l'indice negatif lirait la fin de l'archive.
        if i >= 30 and data[i - 30:i - 26] == b"PK\x03\x04":    # en-tete local : drapeaux +6
            data[i - 30 + 7] |= 0x08                  # octet haut des drapeaux : bit 11
        elif i >= 46 and data[i - 46:i - 42] == b"PK\x01\x02":  # repertoire central : +8
            data[i - 46 + 9] |= 0x08
        else:
            i = data.find(raw, i + 1)
            continue
        data[i:i + len(raw)] = bad
        count += 1
        i = data.find(raw, i + 1)
    assert count == 2, count
    return bytes(data)


def zip_negative_header_offset(files):
    """Archive dont le decalage du repertoire central (EOCD) est trop grand : le decalage
    de concatenation devient negatif, donc la position de l'en-tete local de la premiere
    entree aussi (zipfile : seek negatif, OSError). OSError seulement sur un fichier reel,
    comme le lit le convertisseur : sur un BytesIO, le seek negatif leve ValueError."""
    data = bytearray(zip_bytes(files))
    eocd = data.rfind(b"PK\x05\x06")
    off = struct.unpack_from("<I", data, eocd + 16)[0]
    struct.pack_into("<I", data, eocd + 16, off + 1000)
    return bytes(data)


def with_json(files, name, raw):
    """Remplace le contenu d'une entree JSON par `raw` (octets ou str)."""
    return [(n, raw if n == name else d) for n, d in files]


def two_node_choices():
    n = base_nodes()
    for i in (1, 2):
        n["stages"]["s%d" % i] = {"image": "s0.png", "control": {}}
    n["stages"]["s0.m0"] = {"image": "s0.png", "control": {}}
    n["actions"]["a0"] = [{"stage": "s0"}, {"stage": "s1"}, {"stage": "s2"}, {"stage": "s0.m0"}]
    return n


CHOICE_NOTES = {"s0": {"title": "Le chateau", "text": "Il etait une fois " * 20},
                "s0.m0": {"title": "s0.m0", "notes": ""},
                "s1": {"text": "Un texte\nsur deux lignes " + "x" * 80},
                "s2": {"notes": "note"}}


class Case:
    def __init__(self, cid, build, args=(), tags=()):
        self.id = cid
        self.build = build
        self.args = list(args)
        self.tags = list(tags)


def _mp3_variant(b1, b2, n=120):
    return mp3_frames(b1, b2, n)


def _crc_corrupt():
    data = zip_bytes(std_files())
    marker = MP3[:64]
    off = data.find(marker)
    b = bytearray(data)
    b[off + 10] ^= 0xFF
    return bytes(b)


def _cp437_names():
    """Archive dont un nom est en cp437 sans drapeau UTF-8 (zip Windows historique)."""
    n = base_nodes()
    n["stages"]["s0"]["image"] = "forêt.png"
    n["stages"]["s0"]["audio"] = "forêt.mp3"
    files = std_files(nodes=n) + [("images/forXt.png", png(64, 48)), ("audios/forXt.mp3", MP3)]
    data = zip_bytes(files)
    return data.replace(b"forXt", "forêt".encode("cp437"))


def _duplicate_names():
    import warnings
    buf = io.BytesIO()
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        with zipfile.ZipFile(buf, "w") as z:
            for name, data in std_files():
                z.writestr(zipfile.ZipInfo(name, FIXED_DATE), data)
            z.writestr(zipfile.ZipInfo("images/s0.png", FIXED_DATE), png(32, 24, (0, 200, 0, 255)))
    return buf.getvalue()


def _deep_json():
    deep = "[" * 5000 + "]" * 5000
    return std_files(nodes=False) + [
        ("nodes.json", '{"extra": %s, "startAction": {"action": "a0", "index": 0}, '
                       '"stages": %s, "actions": %s}' % (
                           deep, json.dumps(base_nodes()["stages"]), json.dumps(base_nodes()["actions"])))]


def _gray16(im):
    """Image gris 16 bits (mode I;16, ecrit en PNG 16 bits) : niveaux 8 bits x 257."""
    g = im.convert("L")
    return Image.frombytes("I;16", g.size, b"".join(struct.pack("<H", v * 257) for v in g.tobytes()))


def _png_mode(mode):
    im = Image.new("RGBA", (64, 48))
    px = im.load()
    for y in range(48):
        for x in range(64):
            px[x, y] = (x * 4 % 256, y * 5 % 256, (x + y) * 2 % 256, 255 - x * 3 % 256)
    b = io.BytesIO()
    if mode == "P":
        im.convert("RGB").quantize(16).save(b, "PNG")
    elif mode == "PT":
        q = im.convert("RGB").quantize(16)
        q.save(b, "PNG", transparency=0)
    elif mode == "I16":
        _gray16(im).save(b, "PNG")
    elif mode == "1":
        im.convert("1").save(b, "PNG")
    else:
        im.convert(mode).save(b, "PNG")
    return b.getvalue()


def corpus_cases():
    S = std_files
    cases = []

    def add(cid, build, args=(), tags=()):
        cases.append(Case(cid, build, args, tags))

    # --- conversions nominales et options
    add("ok_basic", lambda: zip_bytes(S()))
    add("ok_keep_size", lambda: zip_bytes(S()), ["--keep-size"])
    add("ok_plain", lambda: zip_bytes(S()), ["--plain"])
    add("ok_plain_choices", lambda: zip_bytes(
        S(nodes=two_node_choices()) + [("notes.json", json.dumps(CHOICE_NOTES))]), ["--plain"])
    add("ok_keep_size_big", lambda: zip_bytes(
        [(n, png(2300, 100) if n == "images/s0.png" else d) for n, d in S()]), ["--keep-size"])
    add("ok_no_uuid", lambda: zip_bytes(S(meta={"title": "Sans uuid"})))
    add("ok_font_replaced", lambda: zip_bytes(S(meta={"title": "日本語 et café", "uuid": "u2"})))
    add("ok_choices_carousel", lambda: zip_bytes(
        S(nodes=two_node_choices()) + [("notes.json", json.dumps(CHOICE_NOTES))]))
    add("ok_choices_image", lambda: zip_bytes(
        S(nodes=two_node_choices()) + [("notes.json", json.dumps(CHOICE_NOTES))]),
        ["--selector", "image"])

    def inventory():
        n = base_nodes()
        n["inventory"] = [{"name": "x", "initialNumber": 1, "maxNumber": 3, "display": 0,
                           "image": "s0.png"},
                          {"name": "y", "initialNumber": 0, "maxNumber": 3, "display": 0,
                           "image": "big.png"}]
        n["actions"]["a0"][0]["conditions"] = [{"comparator": 0, "item": 0, "number": 1}]
        return zip_bytes(S(nodes=n) + [("images/big.png", png(500, 300))])
    add("ok_inventory", inventory)

    def lua_keywords():
        n = base_nodes()
        n["stages"]["end"] = n["stages"].pop("s0")
        n["stages"]["global"] = {"control": {}}
        n["actions"]["repeat"] = [{"stage": "end"}]
        n["actions"]["a0"] = [{"stage": "end"}, {"stage": "global"}]
        return zip_bytes(S(nodes=n))
    add("ok_lua_keywords", lua_keywords)

    # --- images : modes, tRNS, 16 bits, portraits
    for mode in ("L", "LA", "P", "PT", "RGB", "RGBA", "1", "I16"):
        def build(mode=mode):
            data = _png_mode(mode)
            return zip_bytes([(n, data if n in ("images/s0.png", "title.png") else d) for n, d in S()])
        add("img_mode_" + mode, build)
    gray = [[0, 1, 0, 1], [1, 0, 1, 0]]
    raw_cases = {
        "trns_gray1": (1, 0, gray, b"\x00\x01"),
        "trns_gray2": (2, 0, [[0, 1, 2, 3], [3, 2, 1, 0]], b"\x00\x02"),
        "trns_gray4": (4, 0, [[0, 5, 10, 15], [15, 10, 5, 0]], b"\x00\x0a"),
        "trns_gray16": (16, 0, [[0, 256, 65535, 4660], [4660, 65535, 256, 0]], b"\x12\x34"),
        "trns_rgb16": (16, 2, [[(1, 2, 3), (4660, 22136, 39612)], [(65535, 0, 1), (1, 2, 3)]],
                       struct.pack(">HHH", 4660, 22136, 39612)),
        "gray16_plain": (16, 0, [[0, 256, 65535, 4660], [4660, 65535, 256, 0]], None),
        "rgb16_plain": (16, 2, [[(1, 2, 3), (65535, 0, 1)], [(40000, 20000, 100), (0, 0, 0)]], None),
    }
    for cid, (bd, ct, pix, trns) in raw_cases.items():
        def build(bd=bd, ct=ct, pix=pix, trns=trns):
            data = raw_png(len(pix[0]), len(pix), bd, ct, pix, trns)
            return zip_bytes([(n, data if n == "images/s0.png" else d) for n, d in S()])
        add("img_" + cid, build)
    add("img_portrait_scene", lambda: zip_bytes(
        [(n, png(40, 120) if n == "images/s0.png" else d) for n, d in S()]))
    add("img_portrait_cover", lambda: zip_bytes(
        [(n, png(40, 120) if n == "title.png" else d) for n, d in S()]))
    add("img_corrupt_scene", lambda: zip_bytes(
        [(n, b"\x89PNGgarbage" if n == "images/s0.png" else d) for n, d in S()]))
    add("img_corrupt_cover", lambda: zip_bytes(
        [(n, b"garbage" if n == "title.png" else d) for n, d in S()]))

    def jpeg():
        b = io.BytesIO()
        Image.new("RGB", (64, 48), (10, 20, 30)).save(b, "JPEG")
        return zip_bytes([(n, b.getvalue() if n == "images/s0.png" else d) for n, d in S()])
    add("img_jpeg_delegated", jpeg, tags=[TAG_DELEGATED])

    # --- noms
    def nfd():
        n = base_nodes()
        n["stages"]["s0"]["image"] = "forêt.png"
        n["stages"]["s0"]["audio"] = "forêt.mp3"
        return zip_bytes(S(nodes=n) + [("images/forêt.png", png(64, 48)),
                                       ("audios/Forêt.mp3", MP3)])
    add("names_nfd", nfd)
    add("names_cp437", _cp437_names)
    add("names_case_insensitive", lambda: zip_bytes(S(nodes={
        **base_nodes(), "stages": {"s0": {**base_nodes()["stages"]["s0"],
                                          "image": "S0.PNG", "audio": "S0.Mp3"}}})))
    add("names_duplicates", _duplicate_names)

    def non_ascii():
        n = base_nodes()
        n["stages"]["s0"]["audio"] = "Scène 1 #x.mp3"
        n["stages"]["s0"]["image"] = "Scène 1.png"
        return zip_bytes(S(nodes=n) + [("audios/Scène 1 #x.mp3", MP3),
                                       ("images/Scène 1.png", png(64, 48))])
    add("names_non_ascii_utf8", non_ascii)
    for tag, bad in (("dotdot", "../../EVIL.mp3"), ("backslash", "..\\EVIL.mp3"),
                     ("drive", "C:EVIL.mp3"), ("absolute", "/abs.mp3")):
        def build(bad=bad):
            n = base_nodes()
            n["stages"]["s0"]["audio"] = bad
            return zip_bytes(S(nodes=n) + [("audios/" + bad, MP3)])
        add("names_traversal_" + tag, build)

    # --- racines
    add("root_nested", lambda: zip_bytes([("story/" + a, b) for a, b in S()]
                                         + [("story/extra/" + a, b) for a, b in S(meta={"title": "Autre"})]))
    add("root_macosx", lambda: zip_bytes([("__MACOSX/._metadata.json", b"\x00\x05\x16\x07binary"),
                                          ("xxx_metadata.json", "{}")] + S()))
    add("root_ambiguous", lambda: zip_bytes([("a/" + x, y) for x, y in S()]
                                            + [("b/" + x, y) for x, y in S()]))
    add("root_none", lambda: zip_bytes([("readme.txt", "rien")]))
    add("bom", lambda: zip_bytes([(a, "﻿" + b if a.endswith(".json") else b) for a, b in S()]))

    # --- JSON : cas limites
    nodes_json = json.dumps(base_nodes())
    add("json_inf_float", lambda: zip_bytes(with_json(S(), "nodes.json", nodes_json.replace('"index": 0', '"index": 1e400'))),
        tags=[TAG_JSON_LIMIT])
    add("json_big_int", lambda: zip_bytes(with_json(S(), "nodes.json", nodes_json.replace(
        '"index": 0', '"index": 123456789012345678901234567890'))), tags=[TAG_JSON_LIMIT])
    add("json_deep_nesting", lambda: zip_bytes(_deep_json()), tags=[TAG_JSON_LIMIT])
    # surrogate isole dans un identifiant de scene : Python echoue a l'encodage UTF-8
    lone = chr(92) + "ud800"
    add("json_lone_surrogate", lambda: zip_bytes(with_json(
        S(), "nodes.json", nodes_json.replace('"s0"', '"s%s"' % lone))), tags=[TAG_JSON_LIMIT])
    add("json_nan", lambda: zip_bytes(with_json(S(), "nodes.json", nodes_json.replace('"index": 0', '"index": NaN'))))
    add("json_infinity_literal", lambda: zip_bytes(with_json(
        S(), "nodes.json", nodes_json.replace('"index": 0', '"index": Infinity'))))
    add("json_duplicate_keys", lambda: zip_bytes(with_json(
        S(), "metadata.json", '{"title": "A", "title": "B", "uuid": "u1"}')))
    add("json_invalid", lambda: zip_bytes(with_json(S(), "nodes.json", "{bad json")))
    add("json_notes_list", lambda: zip_bytes(S() + [("notes.json", "[]")]))
    add("json_notes_invalid", lambda: zip_bytes(S() + [("notes.json", "{oops")]))

    def bad_types(i):
        def build():
            n = base_nodes()
            if i == 0:
                n["stages"]["s0"]["ok"] = "a0"
            elif i == 1:
                n["actions"]["a0"] = {"stage": "s0"}
            elif i == 2:
                n["stages"]["s0"]["image"] = ["x.png"]
            elif i == 3:
                n["stages"] = []
            else:
                n["actions"]["a0"][0]["conditions"] = [{"item": "x"}]
            return zip_bytes(S(nodes=n))
        return build
    for i in range(5):
        add("json_bad_types_%d" % i, bad_types(i))
    add("json_missing_nodes", lambda: zip_bytes(S(nodes=False)))

    # --- assets manquants
    def missing_audio():
        n = base_nodes()
        n["stages"]["s0"]["audio"] = "absent.mp3"
        return zip_bytes(S(nodes=n))
    add("missing_audio", missing_audio)
    add("missing_audio_allowed", missing_audio, ["--allow-missing"])

    def missing_both():
        n = base_nodes()
        n["stages"]["s0"]["audio"] = "absent.mp3"
        n["stages"]["s0"]["image"] = "absent.png"
        return zip_bytes(S(nodes=n))
    add("missing_image", missing_both)
    # echec avec --plain : ni `.part`, ni `.plain.pk`, ni dossier `.plain` apres l'echec
    add("missing_audio_plain", missing_audio, ["--plain"])
    add("zip_crc_corrupt_plain", _crc_corrupt, ["--plain"])
    add("json_invalid_plain", lambda: zip_bytes(with_json(S(), "nodes.json", "{bad json")),
        ["--plain"])
    add("missing_image_allowed", missing_both, ["--allow-missing"])

    # --- archives corrompues ou particulieres
    add("zip_not_a_zip", lambda: b"pas un zip")
    add("zip_empty_file", lambda: b"")
    add("zip_truncated", lambda: zip_bytes(S())[:len(zip_bytes(S())) // 2])
    add("zip_crc_corrupt", _crc_corrupt)
    add("zip_encrypted_entry", lambda: zip_bytes(S(), encrypted=["audios/s0.mp3"]),
        tags=[TAG_ENCRYPTED])
    add("zip_invalid_utf8_name", lambda: zip_bad_utf8_name(S()), tags=[TAG_BAD_UTF8_NAME])
    add("zip_negative_header_offset", lambda: zip_negative_header_offset(S()),
        tags=[TAG_NEGATIVE_OFFSET])
    add("zip_deflate", lambda: zip_bytes(S(), compress=zipfile.ZIP_DEFLATED))
    # bzip2, LZMA, Zstandard : lus par le Rust (#458), plus de delegation admise.
    add("zip_bzip2_entry", lambda: zip_bytes(S(), compress={"audios/s0.mp3": zipfile.ZIP_BZIP2}))
    add("zip_lzma_entry", lambda: zip_bytes(S(), compress={"nodes.json": zipfile.ZIP_LZMA}))
    add("zip_zstd_entry", lambda: zip_bytes(S(), compress={"images/s0.png": zipfile.ZIP_ZSTANDARD}))
    add("zip_codecs_all", lambda: zip_bytes(S(), compress={
        "metadata.json": zipfile.ZIP_LZMA, "nodes.json": zipfile.ZIP_BZIP2,
        "audios/s0.mp3": zipfile.ZIP_ZSTANDARD, "images/s0.png": zipfile.ZIP_BZIP2}))

    # --- MP3
    def with_audio(data):
        return lambda: zip_bytes([(n, data if n == "audios/s0.mp3" else d) for n, d in S()])
    add("mp3_mpeg1", with_audio(_mp3_variant(0xFB, 0x90)))
    add("mp3_mpeg2", with_audio(_mp3_variant(0xF3, 0x90)))
    add("mp3_mpeg25", with_audio(_mp3_variant(0xE3, 0x90)))
    add("mp3_48khz_warning", with_audio(_mp3_variant(0xFB, 0x94)))
    id3 = b"ID3\x03\x00\x00\x00\x00\x00\x0a" + bytes(10)
    add("mp3_id3_prefix", with_audio(id3 + MP3))
    add("mp3_id3_footer_trailing", with_audio(id3 + MP3 + b"TAG" + bytes(125)))
    add("mp3_garbage", with_audio(b"\x00\x01\x02garbage" * 50))
    add("mp3_truncated_frame", with_audio(MP3[:len(MP3) // 3 + 7]))
    add("mp3_broken_between", with_audio(MP3[:400] + b"\x00" * 300 + MP3[400:]))
    add("mp3_empty", with_audio(b""))
    return cases


def output_cases():
    """Cas qui changent la sortie (`-o NOM`, NOM pris dans le dossier de travail par
    parity.py) : propres au banc de parite, hors de corpus_cases(), dont les generateurs des
    tables Rust (crates/telmi2flam/tests/data) appellent convert() sans option -o."""
    S = std_files
    return [
        # -o x.plain : dossier .plain seul, sans .plain.pk (succes, puis echec sans sortie)
        Case("ok_plain_dir_only", lambda: zip_bytes(S()), ["-o", "out.plain"], ()),
        Case("zip_crc_corrupt_plain_dir_only", _crc_corrupt, ["-o", "out.plain"], ()),
    ]


def write_corpus(out_dir):
    """Ecrit les archives dans out_dir/corpus et l'index out_dir/corpus.json. Retourne
    la liste des entrees {id, archive, args, tags}."""
    d = os.path.join(out_dir, "corpus")
    os.makedirs(d, exist_ok=True)
    entries = []
    for c in corpus_cases() + output_cases():
        name = c.id + ".zip"
        with open(os.path.join(d, name), "wb") as f:
            f.write(c.build())
        entries.append({"id": c.id, "archive": "corpus/" + name, "args": c.args, "tags": c.tags})
    _dump(os.path.join(out_dir, "corpus.json"), entries)
    return entries


# ---------------------------------------------------------------------------
# Matrice de reechantillonnage
# ---------------------------------------------------------------------------
# RGB16 / RGBA16 : PNG 16 bits par canal. LT / RGBT / RGBT16 : tRNS de couleur cle (gris,
# RGB 8 bits, RGB 16 bits), distinct de PT (palette avec tRNS par entree).
MODES = ("L", "LA", "P", "PT", "RGB", "RGBA", "1", "I16", "RGB16", "RGBA16", "LT", "RGBT",
         "RGBT16")
ALPHA_MODES = ("RGBA", "LA", "PT", "RGBA16")
SIZES = (1, 3, 97, 641, 2049)
QUICK_SIZES = (1, 3, 97)
CONTENTS = ("gradient", "noise", "alpha")
NOISE_MAX = 641          # au-dela, le bruit (incompressible) rendrait les PNG enormes...
# ...sauf pour la plus grande taille, en carre et sur ces deux modes seulement (PNG de
# 14,5 Mo en RGBA, 4,2 Mo en L) : la source qui declenche la borne 2047 avec un contenu non compressible.
NOISE_BIG_MODES = ("RGBA", "L")
# operations reelles du convertisseur : nom -> description
OPS = ("scene", "thumb", "inventory", "title", "bound")


def _rgba_content(content, w, h, rng):
    """Image RGBA de la matrice : bruit a graine fixe, degrade, ou degrade a alpha
    partiel. Construite par lignes d'octets (pas de boucle par pixel : 2049 x 2049)."""
    if content == "noise":
        im = Image.frombytes("RGBA", (w, h), rng.randbytes(4 * w * h))
        im.putalpha(255)
        return im
    r_row = bytes(x * 255 // max(1, w - 1) for x in range(w))
    g_col = [y * 255 // max(1, h - 1) for y in range(h)]
    r = Image.frombytes("L", (w, h), r_row * h)
    g = Image.frombytes("L", (w, h), b"".join(bytes([v]) * w for v in g_col))
    mix = {}                       # gv -> table (rv + gv) // 2, appliquee par translate
    b_rows = []
    for gv in g_col:
        if gv not in mix:
            mix[gv] = bytes((v + gv) // 2 for v in range(256))
        b_rows.append(r_row.translate(mix[gv]))
    b = Image.frombytes("L", (w, h), b"".join(b_rows))
    if content == "gradient":
        a = Image.new("L", (w, h), 255)
    else:
        table = [v * 255 // max(1, w + h - 2) for v in range(w + h - 1)]
        a = Image.frombytes("L", (w, h), b"".join(bytes(table[y:y + w]) for y in range(h)))
    return Image.merge("RGBA", (r, g, b, a))


def matrix_png(mode, w, h, content, seed=SEED):
    """PNG d'une cellule de la matrice ; None si la combinaison n'a pas de sens
    (alpha partiel sur un mode sans canal alpha)."""
    if content == "alpha" and mode not in ALPHA_MODES:
        return None
    if content == "noise" and max(w, h) > NOISE_MAX and not (
            w == h == max(SIZES) and mode in NOISE_BIG_MODES):
        return None
    rng = random.Random("%s/%s/%d/%d/%s" % (seed, mode, w, h, content))
    base = _rgba_content(content, w, h, rng)
    b = io.BytesIO()
    if mode in ("RGBA", "RGB", "L", "LA", "1"):
        base.convert(mode).save(b, "PNG")
    elif mode == "P":
        base.convert("RGB").quantize(64).save(b, "PNG")
    elif mode == "PT":
        # palette avec tRNS par entree (alpha partiel quand le contenu le demande)
        q = base.convert("RGB").quantize(32)
        q.info["transparency"] = bytes((i * 8) % 256 for i in range(32))
        q.save(b, "PNG", transparency=q.info["transparency"])
    elif mode == "I16":
        _gray16(base).save(b, "PNG")
    elif mode in ("RGB16", "RGBA16"):
        return _png16(base, mode == "RGBA16")
    elif mode == "LT":
        g = base.convert("L")
        return _png_trns(g, 8, 0, g.getpixel((w // 2, h // 2)).to_bytes(2, "big"))
    elif mode in ("RGBT", "RGBT16"):
        rgb = base.convert("RGB")
        key = _trns_key(rgb)
        if mode == "RGBT":
            return _png_trns(rgb, 8, 2, struct.pack(">HHH", *key))
        # 16 bits : la cle est celle du meme pixel apres elargissement (_wide16). Comportement
        # fige de l'oracle (Pillow 12.3.0), reproduit par le port Rust : Pillow lit l'image
        # RGB 16 bits en RGB 8 bits (octet haut) mais garde la cle tRNS sur 16 bits, comparee
        # telle quelle aux pixels 8 bits. Des qu'un canal de la cle depasse 255 (ici : tout
        # canal v > 0), aucun pixel ne l'egale et la transparence n'est jamais appliquee ; une
        # cle aux trois canaux < 256 viserait au contraire un autre pixel (cle (3, 3, 3) ->
        # pixel 8 bits (3, 3, 3)).
        return _png_trns(rgb, 16, 2, struct.pack(">HHH", *(_wide16(v) for v in key)))
    else:
        raise ValueError(mode)
    return b.getvalue()


def _trns_key(rgb):
    """Cle tRNS d'une image RGB : le pixel le moins gris (ecart max - min des canaux le plus
    grand) parmi le centre et les quatre coins, premier en cas d'egalite. Une cle franchement
    non grise, aux canaux distincts, fait echouer une permutation de canaux dans le port."""
    w, h = rgb.size
    spots = ((w // 2, h // 2), (w - 1, 0), (0, h - 1), (0, 0), (w - 1, h - 1))
    pixels = [rgb.getpixel(p) for p in spots]
    return max(pixels, key=lambda px: max(px) - min(px))


def _wide16(v):
    """Echantillon 8 bits -> 16 bits : octet haut v, octet bas derive (pas v * 257, pour
    que le bit de poids faible compte)."""
    return (v << 8) | _LOW[v]


_LOW = bytes((v * 7 + 3) % 256 for v in range(256))


def _chunk(kind, data):
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))


def _png_trns(im, depth, color_type, trns):
    """PNG ecrit a la main (filtre 0) : gris 8 bits (type 0), RGB 8 ou 16 bits (type 2),
    avec le chunk tRNS donne. `im` : image Pillow L ou RGB, rendue sans perte."""
    w, h = im.size
    data = im.tobytes()
    if depth == 16:
        data = _widen(data)
    return _png_raw(w, h, depth, color_type, data, trns)


def _widen(data):
    """Octets 8 bits -> echantillons 16 bits gros-boutistes (voir _wide16)."""
    wide = bytearray(2 * len(data))
    wide[0::2] = data
    wide[1::2] = data.translate(_LOW)
    return bytes(wide)


def _png16(im, with_alpha):
    """PNG RGB / RGBA 16 bits par canal : octet haut = le canal 8 bits, octet bas derive."""
    im = im if with_alpha else im.convert("RGB")
    return _png_raw(im.size[0], im.size[1], 16, 6 if with_alpha else 2, _widen(im.tobytes()),
                    None)


def _png_raw(w, h, depth, color_type, data, trns):
    stride = len(data) // h
    raw = b"".join(b"\x00" + data[y * stride:(y + 1) * stride] for y in range(h))
    ihdr = struct.pack(">IIBBBBB", w, h, depth, color_type, 0, 0, 0)
    return (b"\x89PNG\r\n\x1a\n" + _chunk(b"IHDR", ihdr)
            + (_chunk(b"tRNS", trns) if trns is not None else b"")
            + _chunk(b"IDAT", zlib.compress(raw, 6)) + _chunk(b"IEND", b""))


def matrix_cells(sizes=SIZES):
    cells = []
    for mode in MODES:
        for s in sizes:
            for shape in ("square", "wide", "tall"):
                w, h = {"square": (s, s), "wide": (s, max(1, s * 3 // 4)),
                        "tall": (max(1, s * 3 // 4), s)}[shape]
                for content in CONTENTS:
                    cells.append((mode, w, h, content))
    # dedoublonne (tailles 1 : les trois formes coincident)
    return list(dict.fromkeys(cells))


def write_matrix(out_dir, sizes=SIZES):
    """Ecrit les PNG dans out_dir/matrix et l'index out_dir/matrix.json
    ({id, png, op}, un par PNG et par operation)."""
    d = os.path.join(out_dir, "matrix")
    os.makedirs(d, exist_ok=True)
    entries = []
    for mode, w, h, content in matrix_cells(sizes):
        data = matrix_png(mode, w, h, content)
        if data is None:
            continue
        pid = "%s_%dx%d_%s" % (mode, w, h, content)
        with open(os.path.join(d, pid + ".png"), "wb") as f:
            f.write(data)
        for op in OPS:
            if op == "bound" and max(w, h) <= 2047:
                continue
            entries.append({"id": "%s__%s" % (pid, op), "png": "matrix/%s.png" % pid, "op": op})
    _dump(os.path.join(out_dir, "matrix.json"), entries)
    return entries


def python_lif(png_bytes, op):
    """`.lif` de l'oracle Python pour une operation de la matrice (memes appels que
    telmi2flam._convert)."""
    import telmi2flam as T
    if op == "scene":
        return T.png_to_scene_lif(png_bytes)
    if op == "thumb":
        # le port Rust ne fait que le mode `bandes` (le defaut Python est `flou`)
        return T.png_to_thumb_lif(png_bytes, "bandes")
    if op == "inventory":
        return T.png_to_fit_lif(png_bytes, T.INV_W, T.INV_H)
    if op == "title":
        return T.png_to_fit_lif(png_bytes, 89, 120, align="left")
    if op == "bound":
        return T.png_to_lif(png_bytes, None)
    raise ValueError(op)


def render_matrix_python(out_dir, dest_dir, entries=None):
    """Ecrit dest_dir/<id>.lif pour chaque entree de la matrice avec l'oracle Python."""
    if entries is None:
        with open(os.path.join(out_dir, "matrix.json"), encoding="utf-8") as f:
            entries = json.load(f)
    os.makedirs(dest_dir, exist_ok=True)
    for e in entries:
        with open(os.path.join(out_dir, e["png"]), "rb") as f:
            lif = python_lif(f.read(), e["op"])
        with open(os.path.join(dest_dir, e["id"] + ".lif"), "wb") as f:
            f.write(lif)
    return len(entries)


def _dump(path, obj):
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        json.dump(obj, f, indent=1, ensure_ascii=True)
        f.write("\n")


def main(argv=None):
    ap = argparse.ArgumentParser(description="Genere le corpus et la matrice de parite")
    ap.add_argument("out_dir", help="dossier de sortie (temporaire, jamais commite)")
    ap.add_argument("--only", choices=["corpus", "matrix"])
    ap.add_argument("--quick", action="store_true", help="matrice reduite (petites tailles)")
    args = ap.parse_args(argv)
    os.makedirs(args.out_dir, exist_ok=True)
    if args.only != "matrix":
        print("corpus : %d archives" % len(write_corpus(args.out_dir)))
    if args.only != "corpus":
        print("matrice : %d cellules" % len(write_matrix(
            args.out_dir, QUICK_SIZES if args.quick else SIZES)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
