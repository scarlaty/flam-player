"""
Tests du convertisseur telmi2flam (unittest stdlib, dependance : Pillow deja
requise par le convertisseur).

    python -m unittest discover -s tools/telmi2flam/tests -v

Les paquets TELMI sont synthetiques (generes a la volee). Le chargement de
nodes.lua par un vrai Lua 5.4 est verifie si un interpreteur est trouve
(variable FLAM_LUA, ou lua54 / lua5.4 / lua dans le PATH), sinon ces
verifications sont sautees (le parseur Python de validate.py reste utilise).
"""
import contextlib
import io
import json
import os
import random
import shutil
import subprocess
import sys
import tempfile
import unittest
import unittest.mock
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.dirname(HERE)
sys.path.insert(0, TOOL)
sys.dont_write_bytecode = True

from PIL import Image  # noqa: E402

import lif  # noqa: E402
import mp3map  # noqa: E402
import telmi2flam as T  # noqa: E402
import validate  # noqa: E402


def png(w, h, color=(200, 10, 10, 255)):
    b = io.BytesIO()
    Image.new("RGBA", (w, h), color).save(b, "PNG")
    return b.getvalue()


def mp3_frames(b1=0xFB, b2=0x90, n=200):
    hdr = mp3map._parse_frame_header(bytes([0xFF, b1, b2, 0]), 0)
    return (bytes([0xFF, b1, b2, 0]) + bytes(hdr[0] - 4)) * n


def raw_png(w, h, bd, ct, pixels, trns):
    """PNG minimal (filtre 0) : pixels = liste de lignes d'echantillons
    (entiers gris, ou tuples RGB), trns = octets bruts du chunk tRNS (None :
    pas de tRNS)."""
    import struct
    import zlib

    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d))
    raw = b""
    for row in pixels:
        raw += b"\x00"
        samples = [c for px in row for c in (px if isinstance(px, tuple) else (px,))]
        if bd == 16:
            raw += b"".join(struct.pack(">H", v) for v in samples)
        elif bd == 8:
            raw += bytes(samples)
        else:
            acc, n, out = 0, 0, bytearray()
            for v in samples:
                acc = (acc << bd) | v
                n += bd
                if n == 8:
                    out.append(acc)
                    acc, n = 0, 0
            if n:
                out.append(acc << (8 - n))
            raw += bytes(out)
    ihdr = struct.pack(">IIBBBBB", w, h, bd, ct, 0, 0, 0)
    trns_chunk = chunk(b"tRNS", trns) if trns is not None else b""
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + trns_chunk
            + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))


MP3 = mp3_frames()


def base_nodes():
    return {"startAction": {"action": "a0", "index": 0},
            "stages": {"s0": {"image": "s0.png", "audio": "s0.mp3", "ok": None, "home": None,
                              "control": {"ok": True, "home": False, "autoplay": True}}},
            "actions": {"a0": [{"stage": "s0"}]}}


def std_files(meta=None, nodes=None):
    meta = meta if meta is not None else {"title": "T", "uuid": "u1"}
    files = [("metadata.json", json.dumps(meta)),
             ("images/s0.png", png(64, 48)),
             ("audios/s0.mp3", MP3),
             ("title.png", png(64, 48))]
    if nodes is not False:
        files.append(("nodes.json", json.dumps(nodes if nodes is not None else base_nodes())))
    return files


def lif_dims(data):
    return int.from_bytes(data[4:8], "big"), int.from_bytes(data[8:12], "big")


def find_lua():
    return validate.find_lua()


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="t2f_")

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def mkzip(self, files, name="in"):
        p = os.path.join(self.tmp, name + ".zip")
        with zipfile.ZipFile(p, "w") as z:
            for n, d in files:
                z.writestr(n, d)
        return p

    def convert(self, files, name="in", **kw):
        p = self.mkzip(files, name)
        out = kw.pop("out", os.path.join(self.tmp, name + ".plain.pk"))
        err = io.StringIO()
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(err):
            T.convert(p, out, **kw)
        self.stderr = err.getvalue()
        return out

    def nodes(self, pk):
        with zipfile.ZipFile(pk) as z:
            src = z.read("script/nodes.lua").decode("utf-8")
        return src, validate.parse_lua_data(src)

    def assertValid(self, pk):
        with contextlib.redirect_stdout(io.StringIO()) as out:
            rc = validate.main(pk)
        self.assertEqual(rc, 0, out.getvalue())
        src, data = self.nodes(pk)
        lua = find_lua()
        if lua:
            chk = os.path.join(self.tmp, "chk.lua")
            nl = os.path.join(self.tmp, "nodes_chk.lua")
            with open(nl, "w", encoding="utf-8", newline="\n") as f:
                f.write(src)
            with open(chk, "w") as f:
                f.write("local t = dofile(arg[1])\n"
                        "assert(type(t) == 'table' and type(t.stages) == 'table' "
                        "and type(t.actions) == 'table')\nprint('LUA_OK')\n")
            r = subprocess.run([lua, chk, nl], capture_output=True, text=True)
            self.assertIn("LUA_OK", r.stdout, r.stdout + r.stderr)
        return data


class TestConversion(Base):
    def test_ok(self):
        pk = self.convert(std_files())
        data = self.assertValid(pk)
        self.assertEqual(data["stages"]["s0"]["audio"], "s0.mp3")
        self.assertEqual(data["stages"]["s0"]["image"], "s0.lif")

    # F21 : zip-slip
    def test_traversal_rejected(self):
        for bad in ("../../EVIL.mp3", "..\\EVIL.mp3", "C:EVIL.mp3", "/abs.mp3", "a/../../b.mp3"):
            n = base_nodes()
            n["stages"]["s0"]["audio"] = bad
            files = std_files(nodes=n) + [("audios/" + bad, MP3)]
            with self.assertRaises(T.ConvError, msg=bad):
                self.convert(files, emit_plain=True)
        # rien n'a ete ecrit hors du dossier temporaire
        self.assertFalse(os.path.exists(os.path.join(os.path.dirname(self.tmp), "EVIL.mp3")))

    def test_names_canonical(self):
        n = base_nodes()
        n["stages"]["s0"]["audio"] = "Scène 1 #x.mp3"
        n["stages"]["s0"]["image"] = "Scène 1.png"
        files = std_files(nodes=n) + [("audios/Scène 1 #x.mp3", MP3),
                                      ("images/Scène 1.png", png(64, 48))]
        pk = self.convert(files, emit_plain=True)
        data = self.assertValid(pk)
        st = data["stages"]["s0"]
        self.assertRegex(st["audio"], r"^[A-Za-z0-9_-]+\.mp3$")
        self.assertRegex(st["image"], r"^[A-Za-z0-9_-]+\.lif$")
        plain = pk[:-3]
        for dp, _dn, fns in os.walk(plain):
            for fn in fns:
                full = os.path.realpath(os.path.join(dp, fn))
                self.assertTrue(full.startswith(os.path.realpath(plain)))

    # F14 : image corrompue
    def test_corrupt_png(self):
        n = base_nodes()
        n["stages"]["s0"]["image"] = "bad.png"
        pk = self.convert(std_files(nodes=n) + [("images/bad.png", b"\x89PNGgarbage")])
        self.assertIn("bad.png", self.stderr)
        data = self.assertValid(pk)
        self.assertEqual(data["stages"]["s0"]["image"], "empty.lif")

    def test_corrupt_cover(self):
        files = [(a, b"garbage" if a == "title.png" else b) for a, b in std_files()]
        pk = self.convert(files)
        self.assertIn("couverture illisible", self.stderr)
        self.assertValid(pk)

    # F29 : audio manquant / casse
    def test_missing_audio_fails(self):
        n = base_nodes()
        n["stages"]["s0"]["audio"] = "absent.mp3"
        with self.assertRaises(T.ConvError) as cm:
            self.convert(std_files(nodes=n))
        self.assertIn("absent.mp3", str(cm.exception))

    def test_missing_audio_allowed(self):
        n = base_nodes()
        n["stages"]["s0"]["audio"] = "absent.mp3"
        n["stages"]["s0"]["image"] = "absent.png"
        pk = self.convert(std_files(nodes=n), allow_missing=True)
        data = self.assertValid(pk)
        self.assertEqual(data["stages"]["s0"]["audio"], "silent.mp3")
        self.assertNotIn("image", data["stages"]["s0"])

    def test_case_insensitive(self):
        n = base_nodes()
        n["stages"]["s0"]["image"] = "S0.PNG"
        n["stages"]["s0"]["audio"] = "S0.Mp3"
        data = self.assertValid(self.convert(std_files(nodes=n)))
        self.assertIn("image", data["stages"]["s0"])
        self.assertIn("audio", data["stages"]["s0"])

    # hunt1 : zip macOS (noms NFD) alors que nodes.json est en NFC
    def test_unicode_nfd_names(self):
        n = base_nodes()
        n["stages"]["s0"]["image"] = "forêt.png"
        n["stages"]["s0"]["audio"] = "forêt.mp3"
        files = std_files(nodes=n) + [("images/forêt.png", png(64, 48)),
                                      ("audios/Forêt.mp3", MP3)]
        data = self.assertValid(self.convert(files))
        self.assertIn("image", data["stages"]["s0"])
        self.assertNotEqual(data["stages"]["s0"]["audio"], "silent.mp3")

    # F16 : paquets imparfaits
    def test_bom(self):
        files = [(a, "﻿" + b if a.endswith(".json") else b) for a, b in std_files()]
        self.assertValid(self.convert(files))

    def test_missing_nodes(self):
        with self.assertRaises(T.ConvError):
            self.convert(std_files(nodes=False))

    def test_bad_json(self):
        files = std_files(nodes=False) + [("nodes.json", "{bad json")]
        with self.assertRaises(T.ConvError):
            self.convert(files)

    def test_bad_types(self):
        cases = []
        n = base_nodes(); n["stages"]["s0"]["ok"] = "a0"; cases.append(n)
        n = base_nodes(); n["actions"]["a0"] = {"stage": "s0"}; cases.append(n)
        n = base_nodes(); n["stages"]["s0"]["image"] = ["x.png"]; cases.append(n)
        n = base_nodes(); n["stages"] = []; cases.append(n)
        n = base_nodes(); n["actions"]["a0"][0]["conditions"] = [{"item": "x"}]; cases.append(n)
        for n in cases:
            with self.assertRaises(T.ConvError, msg=json.dumps(n)):
                self.convert(std_files(nodes=n))

    def test_notes_list_ignored(self):
        pk = self.convert(std_files() + [("notes.json", "[]")])
        self.assertIn("notes.json ignore", self.stderr)
        self.assertValid(pk)

    def test_main_exit_code(self):
        bad = os.path.join(self.tmp, "notzip.zip")
        with open(bad, "wb") as f:
            f.write(b"pas un zip")
        r = subprocess.run([sys.executable, os.path.join(TOOL, "telmi2flam.py"), bad,
                            "-o", os.path.join(self.tmp, "x.plain.pk")],
                           capture_output=True, text=True)
        self.assertEqual(r.returncode, 3)
        self.assertIn("ERREUR", r.stderr)
        self.assertNotIn("Traceback", r.stderr)

    # F17 : detection de la racine
    def test_root_macosx(self):
        files = [("__MACOSX/._metadata.json", b"\x00\x05\x16\x07binary"),
                 ("xxx_metadata.json", "{}")] + std_files()
        self.assertValid(self.convert(files))

    def test_root_nested_and_shallowest(self):
        files = [("story/" + a, b) for a, b in std_files()]
        files += [("story/extra/" + a, b) for a, b in std_files(meta={"title": "Autre"})]
        pk = self.convert(files)
        with zipfile.ZipFile(pk) as z:
            self.assertTrue(z.read("info.plain").startswith(b"T\n"))

    def test_root_ambiguous(self):
        files = [("a/" + x, y) for x, y in std_files()] + [("b/" + x, y) for x, y in std_files()]
        with self.assertRaises(T.ConvError) as cm:
            self.convert(files)
        self.assertIn("plusieurs", str(cm.exception))

    # F18 : cles = mots reserves Lua
    def test_lua_keyword_keys(self):
        n = base_nodes()
        n["stages"]["end"] = n["stages"].pop("s0")
        n["stages"]["global"] = {"control": {}}
        n["actions"]["repeat"] = [{"stage": "end"}]
        n["actions"]["a0"] = [{"stage": "end"}, {"stage": "global"}]
        data = self.assertValid(self.convert(std_files(nodes=n)))
        self.assertIn("end", data["stages"])
        self.assertIn("repeat", data["actions"])

    # F19 : NaN / Infinity
    def test_nan_rejected(self):
        raw = json.dumps(base_nodes()).replace('"index": 0', '"index": NaN')
        files = std_files(nodes=False) + [("nodes.json", raw)]
        with self.assertRaises(T.ConvError):
            self.convert(files)
        with self.assertRaises(ValueError):
            T.lua_repr({"x": float("inf")})

    def test_lua_string_escapes(self):
        lua = find_lua()
        if not lua:
            self.skipTest("interpreteur Lua absent")
        s = "a\x00b\x1a\x7f c\\\"\n\r\t9"
        script = os.path.join(self.tmp, "esc.lua")
        out = os.path.join(self.tmp, "esc.bin")
        with open(script, "w", encoding="utf-8", newline="\n") as f:
            f.write("local s = " + T.lua_repr(s) + "\nlocal f = io.open(arg[1], 'wb')\n"
                    "f:write(s)\nf:close()\n")
        subprocess.run([lua, script, out], check=True)
        with open(out, "rb") as f:
            self.assertEqual(f.read(), s.encode("utf-8"))
        self.assertEqual(validate.parse_lua_data("return " + T.lua_repr(s)), s)

    # F48 : collisions de noms
    def test_name_collisions(self):
        n = base_nodes()
        n["stages"]["s1"] = {"image": "empty.png", "audio": "silent.mp3", "control": {}}
        n["stages"]["s2"] = {"image": "title.png", "audio": "title.mp3", "control": {}}
        n["stages"]["s3"] = {"image": "s0.jpg", "control": {}}
        n["actions"]["a0"] = [{"stage": s} for s in ("s0", "s1", "s2", "s3")]
        files = std_files(nodes=n) + [("images/empty.png", png(64, 48)),
                                      ("images/title.png", png(64, 48)),
                                      ("images/s0.jpg", png(64, 48)),
                                      ("audios/silent.mp3", MP3),
                                      ("audios/title.mp3", MP3),
                                      ("title.mp3", MP3)]
        pk = self.convert(files)
        data = self.assertValid(pk)
        st = data["stages"]
        imgs = [st[s]["image"] for s in ("s0", "s1", "s2", "s3")]
        self.assertEqual(len(set(i.lower() for i in imgs)), 4)
        self.assertNotIn(st["s1"]["image"], ("empty.lif",))
        self.assertNotIn(st["s2"]["image"], ("title.lif", "thumbnail.lif"))
        self.assertNotEqual(st["s1"]["audio"], "silent.mp3")
        self.assertNotEqual(st["s2"]["audio"], "title.mp3")
        with zipfile.ZipFile(pk) as z:
            self.assertEqual(lif_dims(z.read("img/empty.lif")), (8, 8))
            self.assertEqual(z.read("sounds/silent.mp3")[:4], bytes([0xFF, 0xFB, 0x90, 0x00]))
            self.assertEqual(lif_dims(z.read("img/" + st["s2"]["image"])), (T.VISUAL_W, T.VISUAL_H))

    # Image de scene 4:3 : ajustee sans deformation dans la zone 320x212
    # (sous le bandeau de 28 px), centree avec des bandes transparentes.
    def test_scene_image_fits_visual_area(self):
        import lif
        self.assertEqual(T.png_to_scene_lif.__defaults__, (320, 212))
        data = T.png_to_scene_lif(png(640, 480))
        w, h = lif_dims(data)
        self.assertEqual((w, h), (320, 212))
        rgba = lif.decode(data)[0]
        alpha = lambda x, y: rgba[(y * w + x) * 4 + 3]
        # 640x480 -> 283x212 : bandes de 18 px (gauche) et 19 px (droite)
        self.assertEqual(alpha(0, 106), 0)
        self.assertEqual(alpha(17, 106), 0)
        self.assertEqual(alpha(18, 106), 255)
        self.assertEqual(alpha(300, 106), 255)
        self.assertEqual(alpha(301, 106), 0)
        self.assertEqual(alpha(160, 0), 255)
        self.assertEqual(alpha(160, 211), 255)
        # image plus petite que la zone : agrandie, pas repetee
        w2, h2 = lif_dims(T.png_to_scene_lif(png(64, 48)))
        self.assertEqual((w2, h2), (320, 212))

    # F49 / F50 : inventaire
    def test_inventory_icons(self):
        n = base_nodes()
        n["inventory"] = [{"name": "x", "initialNumber": 1, "maxNumber": 3, "display": 0,
                           "image": "s0.png"},
                          {"name": "y", "initialNumber": 0, "maxNumber": 3, "display": 0,
                           "image": "big.png"}]
        pk = self.convert(std_files(nodes=n) + [("images/big.png", png(5000, 300))])
        data = self.assertValid(pk)
        inv0, inv1 = data["inventory"][0]["image"], data["inventory"][1]["image"]
        self.assertNotEqual(inv0, data["stages"]["s0"]["image"])
        self.assertTrue(inv0.endswith("_inv.lif"))
        with zipfile.ZipFile(pk) as z:
            self.assertEqual(lif_dims(z.read("img/" + data["stages"]["s0"]["image"])), (T.VISUAL_W, T.VISUAL_H))
            w, h = lif_dims(z.read("img/" + inv0))
            self.assertLessEqual(max(w, h), 128)
            w, h = lif_dims(z.read("img/" + inv1))
            self.assertLessEqual(max(w, h), 128)

    def test_keep_size_bound(self):
        n = base_nodes()
        n["stages"]["s0"]["image"] = "wide.png"
        pk = self.convert(std_files(nodes=n) + [("images/wide.png", png(5000, 40))],
                          keep_size=True)
        data = self.assertValid(pk)
        with zipfile.ZipFile(pk) as z:
            w, h = lif_dims(z.read("img/" + data["stages"]["s0"]["image"]))
        self.assertLessEqual(max(w, h), 2047)

    # F51 : couverture portrait
    def test_portrait_cover(self):
        files = [(a, png(480, 640) if a == "title.png" else b) for a, b in std_files()]
        pk = self.convert(files)
        self.assertValid(pk)
        with zipfile.ZipFile(pk) as z:
            self.assertEqual(lif_dims(z.read("img/thumbnail.lif")), (104, 136))
            w, h = lif_dims(z.read("img/title.lif"))
            self.assertGreaterEqual(w, h)

    def test_thumb_fit_transmis_a_la_conversion(self):
        files = [(a, png(400, 200) if a == "title.png" else b) for a, b in std_files()]
        tops = {}
        for fit in ("bandes", "flou"):
            pk = self.convert(files, name="t_" + fit, thumb_fit=fit)
            with zipfile.ZipFile(pk) as z:
                rgba, w, h = lif.decode(z.read("img/thumbnail.lif"))
            self.assertEqual((w, h), (104, 136))
            tops[fit] = rgba[3]
        self.assertEqual(tops, {"bandes": 0, "flou": 255})

    # F52 : label des choix
    def test_choice_label(self):
        n = base_nodes()
        n["stages"]["s1"] = {"image": "s0.png", "control": {}}
        n["stages"]["s2"] = {"image": "s0.png", "control": {}}
        n["stages"]["s0.m0"] = {"image": "s0.png", "control": {}}
        n["actions"]["a0"] = [{"stage": "s0"}, {"stage": "s1"}, {"stage": "s2"},
                              {"stage": "s0.m0"}]
        notes = {"s0": {"title": "Le chateau", "text": "Il etait une fois " * 20},
                 # titre par defaut de l'editeur TELMI = id du noeud : pas un label
                 "s0.m0": {"title": "s0.m0", "notes": ""},
                 "s1": {"text": "Un texte\nsur deux lignes " + "x" * 80},
                 "s2": {"notes": "note"}}
        data = self.assertValid(self.convert(std_files(nodes=n) +
                                             [("notes.json", json.dumps(notes))]))
        st = data["stages"]
        self.assertEqual(st["s0"]["text"], "Le chateau")
        self.assertLessEqual(len(st["s1"]["text"]), 40)
        self.assertNotIn("\n", st["s1"]["text"])
        self.assertEqual(st["s2"]["text"], "note")
        self.assertNotIn("text", st["s0.m0"])

    # F53 : titre multi-lignes
    def test_title_newline(self):
        meta = {"title": "Ligne1\nLigne2", "category": "Cat\r\negorie", "uuid": "u"}
        pk = self.convert(std_files(meta=meta))
        self.assertValid(pk)
        with zipfile.ZipFile(pk) as z:
            lines = z.read("info.plain").decode("utf-8").split("\n")
        self.assertEqual(lines, ["Ligne1 Ligne2", "Cat egorie", "000000", "Ligne1 Ligne2"])

    # F54 : UUID sans metadata.uuid
    def test_uuid_from_content(self):
        n2 = base_nodes()
        n2["stages"]["s0"]["control"]["home"] = True
        pk1 = self.convert(std_files(meta={"title": "Meme"}), name="a")
        self.assertIn("uuid", self.stderr)
        pk2 = self.convert(std_files(meta={"title": "Meme"}, nodes=n2), name="b")
        with zipfile.ZipFile(pk1) as z1, zipfile.ZipFile(pk2) as z2:
            self.assertNotEqual(z1.read("uuid.bin"), z2.read("uuid.bin"))

    def test_png16(self):
        b = io.BytesIO()
        Image.new("I;16", (64, 48)).save(b, "PNG")
        n = base_nodes()
        n["stages"]["s0"]["image"] = "g16.png"
        self.assertValid(self.convert(std_files(nodes=n) + [("images/g16.png", b.getvalue())]))

    # hunt2 : gris 16 bits (I;16) remis a l'echelle, pas ecrete a 255 (blanc)
    def test_png16_gray_levels(self):
        for val, expect in ((32768, 128), (1000, 3)):
            b = io.BytesIO()
            Image.new("I;16", (8, 8), val).save(b, "PNG")
            px = T._open_rgba(b.getvalue()).getpixel((0, 0))
            self.assertLessEqual(abs(px[0] - expect), 2, (val, px))
            self.assertEqual(px[0], px[1])
            self.assertEqual(px[3], 255)

    # hunt2 : --keep-size borne a 2047 px (11 bits lv_img_header_t, lif_decoder.c)
    def test_keep_size_2047(self):
        self.assertEqual(T.MAX_DIM, 2047)
        self.assertEqual(validate.LIF_MAX_DIM, 2047)
        n = base_nodes()
        n["stages"]["s0"]["image"] = "big.png"
        n["inventory"] = [{"name": "x", "image": "big.png", "initialNumber": 0, "maxNumber": 1}]
        pk = self.convert(std_files(nodes=n) + [("images/big.png", png(3000, 1000))],
                          keep_size=True)
        data = self.assertValid(pk)
        with zipfile.ZipFile(pk) as z:
            w, h = lif_dims(z.read("img/" + data["stages"]["s0"]["image"]))
        self.assertLessEqual(max(w, h), 2047)
        self.assertGreater(w, 2000)

    # hunt2 : id finissant par "\n" -> cle entre crochets (pas de cle nue tronquee)
    def test_id_trailing_newline(self):
        self.assertIsNone(T._LUA_ID.match("a0\n"))
        n = base_nodes()
        n["startAction"]["action"] = "a0\n"
        n["actions"] = {"a0\n": [{"stage": "s1\n"}], "a1": [{"stage": "s1"}]}
        n["stages"]["s1\n"] = dict(n["stages"]["s0"])
        n["stages"]["s1"] = dict(n["stages"]["s0"])
        pk = self.convert(std_files(nodes=n))
        data = self.assertValid(pk)
        self.assertIn("a0\n", data["actions"])
        self.assertIn("s1\n", data["stages"])
        self.assertIn("s1", data["stages"])

    # hunt2 : notes.json illisible (optionnel) ignore avec avertissement
    def test_notes_invalid_ignored(self):
        for raw in ('{"s0": {"title": "\xe9t\xe9"}}'.encode("latin-1"),
                    b'{"s0": {"title": ', b'{"s0": NaN}'):
            pk = self.convert(std_files() + [("notes.json", raw)])
            self.assertIn("notes.json ignore", self.stderr, raw)
            self.assertValid(pk)

    # hunt2 : totalChapters ne compte pas les options de choix (jamais chapitres)
    def test_total_chapters_excludes_choice_options(self):
        n = base_nodes()
        opt = {"image": "s0.png", "audio": "s0.mp3", "ok": {"action": "a3", "index": 0},
               "control": {"ok": True, "home": False, "autoplay": False}}
        n["stages"]["s0"]["ok"] = {"action": "a1", "index": 0}
        n["stages"]["s1"] = dict(opt)
        n["stages"]["s2"] = dict(opt)
        n["stages"]["s3"] = {"image": "s0.png", "audio": "s0.mp3", "ok": None,
                             "control": {"ok": True, "home": False, "autoplay": True}}
        n["actions"]["a1"] = [{"stage": "s1"}, {"stage": "s2"}]
        n["actions"]["a3"] = [{"stage": "s3"}]
        data = self.assertValid(self.convert(std_files(nodes=n)))
        self.assertEqual(data["totalChapters"], 2)   # s0 + s3
        # option autoplay : jouee directement par enterStage -> chapitre
        n["stages"]["s2"]["control"] = {"ok": True, "home": False, "autoplay": True}
        data = self.assertValid(self.convert(std_files(nodes=n), name="b"))
        self.assertEqual(data["totalChapters"], 3)

    # #43 : indexItem = index (choix preselectionne si cibles non autoplay) :
    # ses cibles non autoplay sont des options, pas des chapitres.
    def test_total_chapters_index_item_targets(self):
        n = base_nodes()
        n["inventory"] = [{"name": "n", "initialNumber": 0, "maxNumber": 5}]
        opt = {"image": "s0.png", "audio": "s0.mp3", "ok": {"action": "a3", "index": 0},
               "control": {"ok": True, "home": False, "autoplay": False}}
        n["stages"]["s0"]["ok"] = {"action": "a1", "indexItem": 0}
        n["stages"]["s1"] = dict(opt)
        n["stages"]["s2"] = dict(opt)
        n["stages"]["s3"] = {"image": "s0.png", "audio": "s0.mp3", "ok": None,
                             "control": {"ok": True, "home": False, "autoplay": True}}
        n["actions"]["a1"] = [{"stage": "s1"}, {"stage": "s2"}]
        n["actions"]["a3"] = [{"stage": "s3"}]
        data = self.assertValid(self.convert(std_files(nodes=n)))
        self.assertEqual(data["stages"]["s0"]["ok"], {"action": "a1", "indexItem": 0})
        self.assertEqual(data["totalChapters"], 2)   # s0 + s3 (avant : 3)
        # cibles autoplay : jouees directement -> chapitres
        for k in ("s1", "s2"):
            n["stages"][k]["control"] = {"ok": True, "home": False, "autoplay": True}
        data = self.assertValid(self.convert(std_files(nodes=n), name="b"))
        self.assertEqual(data["totalChapters"], 3)

    # Maxicours B9 : branches exclusives (menu de niveaux) -> totalChapters =
    # chapitres de la plus longue partie possible, pas la somme des branches
    # (sinon la jauge "Reprendre" plafonne a 31-36 %).
    def test_total_chapters_longest_run(self):
        def scene(ok, autoplay=True, home=None, ctrl_home=False):
            return {"image": "s0.png", "audio": "s0.mp3", "ok": ok, "home": home,
                    "control": {"ok": True, "home": ctrl_home, "autoplay": autoplay}}
        tr = lambda a: {"action": a, "index": 0}
        n = base_nodes()
        n["stages"]["s0"]["ok"] = tr("menu")
        # options du menu (non autoplay, jamais chapitres) -> branche A ou B
        n["stages"]["oA"] = scene(tr("a1"), autoplay=False)
        n["stages"]["oB"] = scene(tr("b1"), autoplay=False)
        n["actions"]["menu"] = [{"stage": "oA"}, {"stage": "oB"}]
        # branche A : a1 -> a2 -> a3 -> retour a a1 (cycle), 3 chapitres
        n["stages"]["a1"] = scene(tr("a2"))
        n["stages"]["a2"] = scene(tr("a3"))
        n["stages"]["a3"] = scene(tr("a1"))
        # branche B : b1 -> b2 -> fin, 2 chapitres ; home vers la branche A ignore
        # sans ctrl.home (story.lua homeOf)
        n["stages"]["b1"] = scene(tr("b2"), home=tr("a1"))
        n["stages"]["b2"] = scene(None)
        for k in ("a1", "a2", "a3", "b1", "b2"):
            n["actions"][k] = [{"stage": k}]
        data = self.assertValid(self.convert(std_files(nodes=n)))
        self.assertEqual(data["totalChapters"], 4)     # s0 + a1 a2 a3 (avant : 6)
        # home actif (ctrl.home) de b1 vers la branche A : b1 puis A = 1 + 1 + 3
        n["stages"]["b1"]["control"]["home"] = True
        data = self.assertValid(self.convert(std_files(nodes=n), name="h"))
        self.assertEqual(data["totalChapters"], 5)
        # stage non atteignable depuis start : ne compte pas
        n["stages"]["orphan"] = scene(None)
        n["actions"]["orph"] = [{"stage": "orphan"}]
        data = self.assertValid(self.convert(std_files(nodes=n), name="o"))
        self.assertEqual(data["totalChapters"], 5)

    # Maxicours B4 : polices du player limitees a U+0020-007E et U+00A0-00FF
    # (src/fonts/*.c) -> translitteration du texte affiche a la conversion.
    def test_font_safe_transliteration(self):
        fs = T._font_safe
        self.assertEqual(fs("L\u2019\u00e9t\u00e9 \u2018x\u2019"), "L'\u00e9t\u00e9 'x'")
        self.assertEqual(fs("\u201cA\u201d \u201eB\u201f"), '"A" "B"')
        self.assertEqual(fs("a\u2013b\u2014c"), "a-b-c")
        self.assertEqual(fs("Fin\u2026"), "Fin...")
        self.assertEqual(fs("\u0153uvre \u0152IL"), "oeuvre OEIL")
        self.assertEqual(T._text("a\u202fb\u00a0c\u2009d"), "a b c d")
        # accents Latin-1 gardes, y compris sous forme decomposee (NFD)
        self.assertEqual(fs("\u00e9\u00e0\u00e7\u00ab\u00bb\u00c9"), "\u00e9\u00e0\u00e7\u00ab\u00bb\u00c9")
        self.assertEqual(fs("e\u0301te\u0301"), "\u00e9t\u00e9")
        # approche NFKD, sinon suppression (emoji, symbole) ou "?" (lettre)
        self.assertEqual(fs("\u0151 \ufb01"), "o fi")
        self.assertEqual(T._text("Renard \U0001F98A\ufe0f !"), "Renard !")
        self.assertEqual(fs("\u4e2d"), "?")
        rep = {}
        fs("\u2019\U0001F98A\u00e9", rep)
        self.assertEqual(rep, {"\u2019": "'", "\U0001F98A": ""})
        # bout en bout : titre, sous-titre, labels et inventaire
        meta = {"title": "L\u2019\u00e9t\u00e9 \u2013 \u00ab \u00c9pisode \u00bb \U0001F98A",
                "category": "Cat\u00e9gorie\u202f: \u0153uvre", "uuid": "u"}
        n = base_nodes()
        n["stages"]["s1"] = {"image": "s0.png", "control": {}}
        n["stages"]["s2"] = {"image": "s0.png", "control": {}}
        n["actions"]["a0"] = [{"stage": "s0"}, {"stage": "s1"}, {"stage": "s2"}]
        n["inventory"] = [{"name": "Cl\u00e9 \u2014 or", "initialNumber": 0, "maxNumber": 1}]
        notes = {"s0": {"title": "\u201cLe ch\u00e2teau\u201d\u2026"},
                 "s1": {"title": "\U0001F98A", "text": "Repli texte"},
                 "s2": {"title": "\u2019" * 60}}
        pk = self.convert(std_files(meta=meta, nodes=n) + [("notes.json", json.dumps(notes))])
        data = self.assertValid(pk)
        with zipfile.ZipFile(pk) as z:
            info = z.read("info.plain").decode("utf-8")
        self.assertEqual(info.split("\n"),
                         ["L'\u00e9t\u00e9 - \u00ab \u00c9pisode \u00bb",
                          "Cat\u00e9gorie : oeuvre", "000000",
                          "L'\u00e9t\u00e9 - \u00ab \u00c9pisode \u00bb"])
        self.assertEqual(data["meta"]["title"], "L'\u00e9t\u00e9 - \u00ab \u00c9pisode \u00bb")
        st = data["stages"]
        self.assertEqual(st["s0"]["text"], '"Le ch\u00e2teau"...')
        self.assertEqual(st["s1"]["text"], "Repli texte")   # label vide apres suppression
        self.assertLessEqual(len(st["s2"]["text"]), T.LABEL_MAX)
        self.assertEqual(data["inventory"][0]["name"], "Cl\u00e9 - or")
        texts = [info, data["meta"]["subtitle"], data["inventory"][0]["name"]]
        texts += [s["text"] for s in st.values() if "text" in s]
        for t in texts:
            self.assertTrue(all(T._in_font(c) or c == "\n" for c in t), ascii(t))
        # avertissement ASCII listant les caracteres remplaces
        self.assertIn("hors police", self.stderr)
        self.assertIn("U+2019", self.stderr)
        self.assertIn("U+1F98A", self.stderr)
        self.assertTrue(all(ord(c) < 128 for c in self.stderr), ascii(self.stderr))
        # texte deja compatible : aucun avertissement
        self.convert(std_files(meta={"title": "\u00c9t\u00e9", "uuid": "u"}), name="lat")
        self.assertNotIn("hors police", self.stderr)


    # hunt3 : cle tRNS des PNG gris 1/2/4/16 bits et RGB 16 bits appliquee
    def test_png_trns_low_depth_and_16bit(self):
        import struct
        cases = [
            # (bd, ct, fond (transparent), autre pixel (opaque), cle tRNS brute)
            (1, 0, 1, 0, struct.pack(">H", 1)),
            (2, 0, 2, 1, struct.pack(">H", 2)),
            (2, 0, 0, 3, struct.pack(">H", 0)),
            (4, 0, 15, 0, struct.pack(">H", 15)),
            (4, 0, 7, 8, struct.pack(">H", 7)),
            (8, 0, 200, 201, struct.pack(">H", 200)),
            (16, 0, 40000, 40001, struct.pack(">H", 40000)),
            (16, 2, (1000, 2000, 3000), (1000, 2000, 50000), struct.pack(">HHH", 1000, 2000, 3000)),
            (8, 2, (10, 20, 30), (10, 20, 31), struct.pack(">HHH", 10, 20, 30)),
        ]
        for bd, ct, bg, fg, trns in cases:
            rows = [[bg] * 16 for _ in range(16)]
            rows[8][8] = fg
            data = raw_png(16, 16, bd, ct, rows, trns)
            im = T._open_rgba(data)
            self.assertEqual(im.getpixel((0, 0))[3], 0, (bd, ct, bg))
            px = im.getpixel((8, 8))
            self.assertEqual(px[3], 255, (bd, ct, fg, px))
            if ct == 0 and bd < 16:
                self.assertEqual(px[0], fg * 255 // ((1 << bd) - 1), (bd, px))
            elif ct == 0:
                self.assertLessEqual(abs(px[0] - fg // 256), 1, px)
        # sans tRNS : toujours opaque
        for bd, ct, v in ((1, 0, 1), (4, 0, 15), (16, 0, 40000), (16, 2, (1, 2, 3))):
            im = T._open_rgba(raw_png(4, 4, bd, ct, [[v] * 4] * 4, None))
            self.assertEqual(im.getpixel((0, 0))[3], 255, (bd, ct))
        # bout en bout : icone d'inventaire gris 1 bit -> LIF avec coin transparent
        n = base_nodes()
        n["inventory"] = [{"name": "x", "image": "ic.png", "initialNumber": 0, "maxNumber": 1}]
        rows = [[1] * 16 for _ in range(16)]
        rows[8][8] = 0
        ic = raw_png(16, 16, 1, 0, rows, struct.pack(">H", 1))
        pk = self.convert(std_files(nodes=n) + [("images/ic.png", ic)])
        self.assertValid(pk)
        import lif
        with zipfile.ZipFile(pk) as z:
            names = [x for x in z.namelist() if x.startswith("img/") and "inv" in x]
            self.assertTrue(names, z.namelist())
            rgba, w, h = lif.decode(z.read(names[0]))
        self.assertEqual(rgba[3], 0)                          # coin : transparent
        c = ((h // 2) * w + w // 2) * 4
        self.assertEqual(rgba[c + 3], 255)                    # centre : opaque

    # #26 : la cle tRNS est lue dans les octets du PNG, pas dans info["transparency"]
    # (brute avant Pillow 12, mise a l'echelle 8 bits pour le gris 1 bit ensuite)
    def test_png_trns_key_raw_and_info_independent(self):
        import struct
        self.assertEqual(T._png_trns_key(raw_png(4, 4, 1, 0, [[1] * 4] * 4, struct.pack(">H", 1))), 1)
        self.assertEqual(T._png_trns_key(raw_png(4, 4, 16, 2, [[(1, 2, 3)] * 4] * 4,
                                                 struct.pack(">HHH", 1, 2, 3))), (1, 2, 3))
        self.assertIsNone(T._png_trns_key(raw_png(4, 4, 1, 0, [[1] * 4] * 4, None)))
        self.assertIsNone(T._png_trns_key(b"pas un png"))
        # info["transparency"] fausse (valeur brute ou mise a l'echelle) : meme resultat
        data = raw_png(4, 4, 1, 0, [[1, 0, 1, 0]] * 4, struct.pack(">H", 1))
        for fake in (1, 255):
            im = Image.open(io.BytesIO(data))
            im.load()
            im.info["transparency"] = fake
            mask = T._trns_mask(im, data)
            self.assertEqual([mask.getpixel((x, 0)) for x in range(4)], [0, 255, 0, 255], fake)

    # hunt3 : runtime du projet absent ou incomplet -> ConvError (pas de paquet muet)
    def test_runtime_missing(self):
        self.assertEqual(tuple(T.RUNTIME_SCRIPTS), tuple(validate.RUNTIME_SCRIPTS))
        self.assertEqual(tuple(T.RUNTIME_UI_LIF), tuple(validate.RUNTIME_UI_LIF))
        for n in T.RUNTIME_SCRIPTS:
            self.assertTrue(os.path.isfile(os.path.join(T.RUNTIME_DIR, n)), n)
        for n in T.RUNTIME_UI_LIF:
            self.assertTrue(os.path.isfile(os.path.join(T.RUNTIME_IMG_DIR, n)), n)
        saved = (T.RUNTIME_DIR, T.RUNTIME_IMG_DIR)
        partial = os.path.join(self.tmp, "partial")
        os.makedirs(partial)
        shutil.copy(os.path.join(saved[0], "global.lua"), partial)
        try:
            for rd, rid in ((os.path.join(self.tmp, "nope"), saved[1]),
                            (saved[0], os.path.join(self.tmp, "nope")),
                            (partial, saved[1])):
                T.RUNTIME_DIR, T.RUNTIME_IMG_DIR = rd, rid
                with self.assertRaises(T.ConvError):
                    self.convert(std_files(), name="noruntime")
        finally:
            T.RUNTIME_DIR, T.RUNTIME_IMG_DIR = saved

    # #129 : le manifeste unique liste exactement ce que _convert copie dans le paquet
    def test_runtime_manifest_matches_dirs(self):
        with open(T.RUNTIME_MANIFEST, encoding="utf-8") as f:
            listed = [ln.strip().lstrip("!") for ln in f
                      if ln.strip() and not ln.startswith("#")]
        on_disk = (["script/" + n for n in sorted(os.listdir(T.RUNTIME_DIR)) if n.endswith(".lua")]
                   + ["img/script/" + n for n in sorted(os.listdir(T.RUNTIME_IMG_DIR))
                      if n.endswith(".lif")])
        self.assertEqual(listed, on_disk)
        self.assertIn("global.lua", T.RUNTIME_SCRIPTS)
        self.assertIn("audio-player-pause.lif", T.RUNTIME_UI_LIF)

    # #129 : manifeste absent -> ConvError a la conversion, pas d'exception a l'import
    def test_runtime_manifest_missing_is_a_conv_error(self):
        self.assertEqual(T._runtime_required(os.path.join(self.tmp, "absent.txt")), (None, None))
        with self.assertRaises(T.ConvError):
            T._require_runtime(T.RUNTIME_DIR, None, "scripts framework")

class TestMp3map(unittest.TestCase):
    def ref_44k(self, data):
        """Formule historique (mp3map-tool de reference) pour du 44,1 kHz MPEG1 L3."""
        start = mp3map._skip_id3v2(data)
        offs, pos = [], start
        while pos < len(data):
            h = mp3map._parse_frame_header(data, pos)
            offs.append(pos)
            pos += h[0]
        nf = len(offs)
        x = 34 if nf > 100 else 22
        total = max(0, nf * 2304 - x * 1152)
        out = bytearray(total.to_bytes(4, "little") + offs[0].to_bytes(4, "little") + bytes(4))
        for k in range(1, total // 88200 + 1):
            fi = min(-(-(k * 88200) // 2304), nf - 1)
            out += offs[fi].to_bytes(4, "little") + (fi * 2304).to_bytes(4, "little")
        return bytes(out)

    def test_44k_unchanged(self):
        for nf in (10, 49, 101, 500):
            for id3 in (b"", b"ID3\x04\x00\x00\x00\x00\x01\x00" + bytes(128)):
                # alterne padding 0/1 et plusieurs debits
                d = id3 + b"".join(mp3_frames(0xFB, b2, 1) for b2 in [0x90, 0x92, 0x50] * nf)
                self.assertEqual(mp3map.build(d)[0], self.ref_44k(d))

    def test_other_rates_duration(self):
        for b1, b2, spf, sr in ((0xFB, 0x94, 1152, 48000), (0xFB, 0x98, 1152, 32000),
                                (0xF3, 0x90, 576, 22050), (0xF5, 0x94, 1152, 24000)):
            n = int(60 * sr / spf)
            info = {}
            _mm, dur, nr = mp3map.build(mp3_frames(b1, b2, n), info)
            self.assertEqual(info["samplerates"], {sr})
            self.assertAlmostEqual(dur, 60, delta=1.0)
            self.assertEqual(nr, int(dur))

    def test_mpeg2_layer2_frame_len(self):
        # MPEG2 Layer II 80 kbps 24 kHz : 144 * 80000 / 24000 = 480 octets
        h = mp3map._parse_frame_header(bytes([0xFF, 0xF5, 0x94, 0x00]), 0)
        self.assertEqual(h, (480, 1152, 24000))
        # MPEG2 Layer III 64 kbps 22050 Hz : 72 * 64000 / 22050 = 208
        h = mp3map._parse_frame_header(bytes([0xFF, 0xF3, 0x80, 0x00]), 0)
        self.assertEqual(h, (208, 576, 22050))

    def test_junk_rejected(self):
        with self.assertRaises(ValueError):
            mp3map.build(os.urandom(512 * 1024))

    @staticmethod
    def _pk_audio(pk):
        with zipfile.ZipFile(pk) as z:
            names = [n for n in z.namelist() if n.startswith("sounds/") and n.endswith(".mp3")
                     and "silent" not in n]
            return {n: z.read(n) for n in names}

    def _conv_rate(self, b1, b2, ffmpeg):
        tc = Base("setUp")
        tc.setUp()
        old = os.environ.get("FLAM_FFMPEG")
        os.environ["FLAM_FFMPEG"] = ffmpeg
        try:
            src = mp3_frames(b1, b2, 400)
            files = [(a, src if a == "audios/s0.mp3" else b) for a, b in std_files()]
            pk = tc.convert(files)
            return src, TestMp3map._pk_audio(pk), tc.stderr
        finally:
            if old is None:
                os.environ.pop("FLAM_FFMPEG", None)
            else:
                os.environ["FLAM_FFMPEG"] = old
            tc.tearDown()

    def test_fallback_without_ffmpeg(self):
        missing = os.path.join(tempfile.gettempdir(), "t2f_no_such_ffmpeg.exe")
        for b1, b2, sr in ((0xFB, 0x94, 48000), (0xF5, 0x94, 24000)):
            src, out, err = self._conv_rate(b1, b2, missing)
            self.assertEqual(list(out.values()), [src])
            self.assertIn(str(sr), err)
            self.assertIn("ffmpeg", err)

    def test_44k_copied_unchanged(self):
        src, out, err = self._conv_rate(0xFB, 0x90, "")
        self.assertEqual(list(out.values()), [src])
        self.assertNotIn("Hz", err)

    @unittest.skipUnless(T._find_ffmpeg(), "ffmpeg absent")
    def test_resample_to_44k(self):
        exe = T._find_ffmpeg()
        for b1, b2, sr in ((0xFB, 0x94, 48000), (0xF5, 0x94, 24000), (0xF3, 0x90, 22050)):
            src, out, err = self._conv_rate(b1, b2, exe)
            (data,) = out.values()
            self.assertNotEqual(data, src)
            info = {}
            mp3map.build(data, info)
            self.assertEqual(info["samplerates"], {44100}, err)
            self.assertNotIn("audio_samplerate", err)
            self.assertNotIn(" ! ", err)


class FakeFfmpeg:
    """Remplace subprocess.run : ecrit `out` comme sortie de ffmpeg (ou leve un delai depasse)."""

    def __init__(self, out=None, rc=0, stderr=b"", timeout=False):
        self.out, self.rc, self.stderr, self.timeout = out, rc, stderr, timeout
        self.calls = []

    def __call__(self, cmd, **kw):
        self.calls.append((cmd, kw))
        if self.timeout:
            raise subprocess.TimeoutExpired(cmd, kw.get("timeout"))
        if self.out is not None:
            with open(cmd[-1][len("file:"):], "wb") as fh:
                fh.write(self.out)
        return subprocess.CompletedProcess(cmd, self.rc, b"", self.stderr)


class TestFfmpegSimule(Base):
    """Resampling audio (#46) avec un faux ffmpeg : delai, sortie invalide ou vide, debit,
    cache par contenu, chemin relatif, title.mp3 et fichier multi-frequences (#274)."""

    SRC_48K = mp3_frames(0xFB, 0x94, 400)       # 128 kbit/s, 48 kHz, 9,6 s
    OUT_44K = mp3_frames(0xFB, 0x90, 400)

    def setUp(self):
        super().setUp()
        self.exe = os.path.join(self.tmp, "ffmpeg-factice.exe")
        with open(self.exe, "wb") as fh:
            fh.write(b"x")
        patcher = unittest.mock.patch.dict(os.environ, {"FLAM_FFMPEG": self.exe})
        patcher.start()
        self.addCleanup(patcher.stop)

    def run_with(self, fake):
        with unittest.mock.patch.object(T.subprocess, "run", fake):
            return T._resample_mp3(self.SRC_48K, 9.6)

    def conv(self, fake, files):
        with unittest.mock.patch.object(T.subprocess, "run", fake):
            pk = self.convert(files)
        return TestMp3map._pk_audio(pk)

    def with_s0(self, data, **kw):
        return [(a, data if a == "audios/s0.mp3" else b) for a, b in std_files(**kw)]

    def test_delai_depasse(self):
        res = self.run_with(FakeFfmpeg(timeout=True))
        self.assertIsInstance(res, str)
        self.assertIn("trop long", res)

    def test_echec_de_ffmpeg(self):
        res = self.run_with(FakeFfmpeg(rc=1, stderr=b"ligne 1\nGrosse erreur\n"))
        self.assertEqual(res, "ffmpeg a echoue (Grosse erreur)")

    def test_sortie_vide(self):
        res = self.run_with(FakeFfmpeg(out=b""))
        self.assertEqual(res, "ffmpeg n'a rien produit")

    def test_sortie_valide_et_commande(self):
        fake = FakeFfmpeg(out=self.OUT_44K)
        self.assertEqual(self.run_with(fake), self.OUT_44K)
        (cmd, kw), = fake.calls
        self.assertEqual(cmd[0], self.exe)
        self.assertIs(kw["stdin"], subprocess.DEVNULL)
        self.assertEqual(kw["timeout"], T.FFMPEG_TIMEOUT)
        self.assertEqual(cmd[cmd.index("-ar") + 1], "44100")
        self.assertEqual(cmd[cmd.index("-b:a") + 1], "128k")

    def test_debit_sans_les_tags(self):
        # ID3v2 de 200 Ko (pochette) + ID3v1 : l'ancien calcul sur la taille totale donnait 192k
        size = 200000
        syncsafe = bytes([(size >> 21) & 0x7F, (size >> 14) & 0x7F, (size >> 7) & 0x7F,
                          size & 0x7F])
        id3 = b"ID3\x04\x00\x00" + syncsafe + bytes(size)
        data = id3 + self.SRC_48K + b"TAG" + bytes(125)
        _mm, dur, _n = mp3map.build(data, {})
        fake = FakeFfmpeg(out=self.OUT_44K)
        with unittest.mock.patch.object(T.subprocess, "run", fake):
            T._resample_mp3(data, dur)
        cmd = fake.calls[0][0]
        self.assertEqual(cmd[cmd.index("-b:a") + 1], "128k")

    def test_chemin_relatif_resolu_en_absolu(self):
        old = os.getcwd()
        os.chdir(self.tmp)
        try:
            os.environ["FLAM_FFMPEG"] = "ffmpeg-factice.exe"
            found = T._find_ffmpeg()
            os.environ["FLAM_FFMPEG"] = "absent.exe"
            missing = T._find_ffmpeg()
        finally:
            os.chdir(old)
        self.assertTrue(os.path.isabs(found))
        self.assertEqual(os.path.normcase(os.path.realpath(found)),
                         os.path.normcase(os.path.realpath(self.exe)))
        self.assertIsNone(missing)

    def test_sortie_invalide_repli_sur_la_copie(self):
        out = self.conv(FakeFfmpeg(out=os.urandom(4096)), self.with_s0(self.SRC_48K))
        self.assertEqual(list(out.values()), [self.SRC_48K])
        self.assertIn("illisible", self.stderr)
        self.assertIn("48000", self.stderr)

    def test_sortie_a_une_autre_frequence_refusee(self):
        out = self.conv(FakeFfmpeg(out=self.SRC_48K), self.with_s0(self.SRC_48K))
        self.assertEqual(list(out.values()), [self.SRC_48K])
        self.assertIn("illisible ou inattendue", self.stderr)

    def test_ffmpeg_remplace_le_fichier(self):
        fake = FakeFfmpeg(out=self.OUT_44K)
        out = self.conv(fake, self.with_s0(self.SRC_48K))
        self.assertEqual(list(out.values()), [self.OUT_44K])
        self.assertIn("reechantillonne", self.stderr)
        self.assertEqual(len(fake.calls), 1)

    def test_title_mp3_hors_44k(self):
        fake = FakeFfmpeg(out=self.OUT_44K)
        files = list(std_files()) + [("title.mp3", self.SRC_48K)]
        with unittest.mock.patch.object(T.subprocess, "run", fake):
            pk = self.convert(files)
        with zipfile.ZipFile(pk) as z:
            title = z.read("sounds/title.mp3")
        self.assertEqual(title, self.OUT_44K)
        info = {}
        mp3map.build(title, info)
        self.assertEqual(info["samplerates"], {44100})
        self.assertEqual(len(fake.calls), 1)

    def test_fichier_multi_frequences(self):
        mixed = mp3_frames(0xFB, 0x90, 200) + mp3_frames(0xFB, 0x94, 200)
        info = {}
        mp3map.build(mixed, info)
        self.assertEqual(info["samplerates"], {44100, 48000})
        fake = FakeFfmpeg(out=self.OUT_44K)
        out = self.conv(fake, self.with_s0(mixed))
        self.assertEqual(list(out.values()), [self.OUT_44K])
        self.assertEqual(len(fake.calls), 1)
        self.assertIn("48000", self.stderr)

    def test_meme_contenu_sous_deux_noms_un_seul_ffmpeg(self):
        def stage(audio):
            return {"image": "s0.png", "audio": audio, "ok": None, "home": None,
                    "control": {"ok": True, "home": False, "autoplay": True}}
        nodes = {"startAction": {"action": "a0", "index": 0},
                 "stages": {"s0": stage("s0.mp3"), "s1": stage("copie.mp3")},
                 "actions": {"a0": [{"stage": "s0"}, {"stage": "s1"}]}}
        files = self.with_s0(self.SRC_48K, nodes=nodes) + [("audios/copie.mp3", self.SRC_48K)]
        fake = FakeFfmpeg(out=self.OUT_44K)
        out = self.conv(fake, files)
        self.assertEqual(sorted(out.values()), [self.OUT_44K, self.OUT_44K])
        self.assertEqual(len(fake.calls), 1)


class TestValidate(Base):
    def rewrite(self, pk, changes):
        out = pk + ".mod.pk"
        with zipfile.ZipFile(pk) as zi, zipfile.ZipFile(out, "w", zipfile.ZIP_STORED) as zo:
            for i in zi.infolist():
                if i.filename in changes:
                    if changes[i.filename] is not None:
                        zo.writestr(i.filename, changes[i.filename])
                else:
                    zo.writestr(i.filename, zi.read(i.filename))
            for k, v in changes.items():
                if k not in zi.namelist() and v is not None:
                    zo.writestr(k, v)
        return out

    def run_validate(self, pk):
        with contextlib.redirect_stdout(io.StringIO()) as out:
            rc = validate.main(pk)
        return rc, out.getvalue()

    def test_detects_problems(self):
        n = base_nodes()
        n["inventory"] = [{"name": "x", "initialNumber": 0, "maxNumber": 3, "display": 0}]
        n["actions"]["a0"][0]["conditions"] = [{"comparator": 0, "item": 5, "number": 1}]
        pk = self.convert(std_files(nodes=n))
        rc, out = self.run_validate(pk)
        self.assertEqual(rc, 1)
        self.assertIn("inventaire", out)
        cases = {
            "uuid.bin": None,                                  # absent : pas de plantage
            "img/s0.lif": b"xxxx" + bytes(30),                 # en-tete LIF invalide
            "sounds/s0.mp3map": bytes(12) + (99).to_bytes(4, "little") + bytes(4),
            "../evil.txt": b"x",                               # nom dangereux
        }
        pk = self.convert(std_files(), name="ok")
        for k, v in cases.items():
            rc, out = self.run_validate(self.rewrite(pk, {k: v}))
            self.assertEqual(rc, 1, "%s\n%s" % (k, out))
            self.assertNotIn("Traceback", out)
        if validate.LuaRuntime is not None or find_lua():
            rc, out = self.run_validate(self.rewrite(pk, {"main.lua": b"local = 1\n"}))
            self.assertEqual(rc, 1, out)
            self.assertIn("main.lua : ERREUR syntaxe", out)

    # #44 : home -> backAction -> backStage -> backChildAction vide = convention
    # Telmi Sync "quitter l'histoire" : pas d'avertissement ; une action vide
    # atteinte par ok reste signalee.
    def back_nodes(self, ok_to_empty=False):
        n = base_nodes()
        n["stages"]["s0"]["control"] = {"ok": True, "home": True, "autoplay": True}
        n["stages"]["s0"]["home"] = {"action": "backAction", "index": 0}
        n["stages"]["backStage"] = {"image": None, "audio": None,
                                    "ok": {"action": "backChildAction", "index": 0},
                                    "home": None, "control": {"ok": True, "home": False, "autoplay": True}}
        n["actions"]["backAction"] = [{"stage": "backStage"}]
        n["actions"]["backChildAction"] = []
        if ok_to_empty:
            n["stages"]["s0"]["ok"] = {"action": "finVide", "index": 0}
            n["actions"]["finVide"] = []
        return n

    def test_back_child_action_empty_no_warning(self):
        rc, out = self.run_validate(self.convert(std_files(nodes=self.back_nodes())))
        self.assertEqual(rc, 0, out)
        self.assertNotIn("vide", out)

    def test_empty_action_via_ok_still_warns(self):
        rc, out = self.run_validate(self.convert(std_files(nodes=self.back_nodes(True))))
        self.assertIn("stage s0.ok -> action 'finVide' vide", out)
        self.assertNotIn("backStage.ok -> action 'backChildAction' vide", out)

    def test_shared_empty_action_still_warns(self):
        # meme action vide atteinte par la chaine home ET par l'ok d'une scene reelle
        n = self.back_nodes()
        n["stages"]["s0"]["ok"] = {"action": "backChildAction", "index": 0}
        rc, out = self.run_validate(self.convert(std_files(nodes=n)))
        self.assertIn("stage s0.ok -> action 'backChildAction' vide", out)

    # hunt1 : index -1 (tirage aleatoire, spec TELMI) n'est pas "hors de l'action"
    def test_random_index_accepted(self):
        n = base_nodes()
        n["stages"]["s1"] = {"image": "s0.png", "control": {}}
        n["actions"]["a1"] = [{"stage": "s0"}, {"stage": "s1"}]
        n["stages"]["s0"]["ok"] = {"action": "a1", "index": -1}
        n["stages"]["s1"]["home"] = {"action": "a1", "index": 5}
        rc, out = self.run_validate(self.convert(std_files(nodes=n)))
        self.assertEqual(rc, 0, out)
        self.assertNotIn("index -1", out)
        self.assertIn("index 5 hors", out)


    # hunt3 : validate signale un paquet sans runtime (setup() echouerait)
    def test_runtime_entries_required(self):
        pk = self.convert(std_files())
        for k in ("script/global.lua", "script/list-choice_1_0_0.lua",
                  "img/script/arrow-right-ui-000.lif"):
            rc, out = self.run_validate(self.rewrite(pk, {k: None}))
            self.assertEqual(rc, 1, "%s\n%s" % (k, out))
            self.assertIn(k + " absente", out)

    # Maxicours B11 : dossier .plain -> memes controles (et non "archive
    # illisible : Permission denied")
    def test_plain_dir(self):
        plain = self.convert(std_files(), out=os.path.join(self.tmp, "d.plain"))
        self.assertTrue(os.path.isdir(plain))
        rc, out = self.run_validate(plain)
        self.assertEqual(rc, 0, out)
        self.assertNotIn("illisible", out)
        self.assertIn("STORED non applicable", out)
        self.assertIn("nodes.lua charge", out)
        self.assertIn("tous les stages sont atteignables", out)
        # meme nombre de controles OK que le .pk, hors STORED (remplace par le
        # constat "dossier .plain")
        pk = self.convert(std_files(), name="same")
        rc_pk, out_pk = self.run_validate(pk)
        self.assertEqual(rc_pk, 0, out_pk)
        self.assertEqual(out.count("[OK]"), out_pk.count("[OK]"))
        # les erreurs sont detectees dans le dossier aussi
        os.remove(os.path.join(plain, "script", "global.lua"))
        with open(os.path.join(plain, "img", "s0.lif"), "wb") as f:
            f.write(b"xxxx" + bytes(30))
        rc, out = self.run_validate(plain)
        self.assertEqual(rc, 1, out)
        self.assertIn("script/global.lua absente", out)
        self.assertIn("LIF img/s0.lif", out)
        self.assertNotIn("Traceback", out)


def banded_png(w, h):
    """Image opaque a deux bandes horizontales (haut rouge, bas bleu)."""
    im = Image.new("RGBA", (w, h), (200, 20, 20, 255))
    im.paste((20, 20, 200, 255), (0, h // 2, w, h))
    b = io.BytesIO()
    im.save(b, "PNG")
    return b.getvalue()


class TestThumbFit(unittest.TestCase):
    """Vignette 104x136 (format officiel mesure sur la Flam), 3 modes de remplissage."""

    SIZES = ((400, 200), (200, 400), (300, 300), (3, 2), (1, 1))

    @staticmethod
    def _score(rgba, w, h):
        """score_coherence de lif_decoder.c : somme des ecarts RVB entre pixels voisins."""
        total = 0
        for y in range(h):
            row = rgba[y * w * 4:(y + 1) * w * 4]
            for x in range(1, w):
                a, b = x * 4, x * 4 - 4
                total += (abs(row[a] - row[b]) + abs(row[a + 1] - row[b + 1])
                          + abs(row[a + 2] - row[b + 2]))
        return total

    @classmethod
    def decoder_transposes(cls, rgba, w, h):
        """Reproduit la decision de lif_decode_mem : transposition si h > w, max(w, h) > 64
        et que la lecture colonne-major est au moins 12 % plus coherente."""
        if not (max(w, h) > 64 and h > w):
            return False
        transposed = bytearray(len(rgba))
        for y in range(h):
            for x in range(w):
                s, d = (x * h + y) * 4, (y * w + x) * 4
                transposed[d:d + 4] = rgba[s:s + 4]
        row, col = cls._score(rgba, w, h), cls._score(bytes(transposed), w, h)
        return row > 0 and col < row and 1.0 - col / row >= 0.12

    def thumb(self, data, fit=T.DEFAULT_THUMB_FIT):
        rgba, w, h = lif.decode(T.png_to_thumb_lif(data, fit))
        return rgba, w, h

    @staticmethod
    def alpha(rgba, w, x, y):
        return rgba[(y * w + x) * 4 + 3]

    def test_taille_officielle(self):
        self.assertEqual((T.THUMB_W, T.THUMB_H), (104, 136))
        self.assertEqual(T.DEFAULT_THUMB_FIT, "flou")
        for fit in T.THUMB_FITS:
            for w, h in self.SIZES:
                _, tw, th = self.thumb(banded_png(w, h), fit)
                self.assertEqual((tw, th), (104, 136), (fit, w, h))

    def test_bandes_transparentes_haut_et_bas(self):
        rgba, w, h = self.thumb(banded_png(400, 200), "bandes")
        for x in (0, 51, 103):
            self.assertEqual(self.alpha(rgba, w, x, 0), 0)
            self.assertEqual(self.alpha(rgba, w, x, 135), 0)
            self.assertEqual(self.alpha(rgba, w, x, 68), 255)

    def test_le_decodeur_ne_transpose_pas_la_vignette(self):
        # La vignette est portrait (104x136, comme les officielles) : validate.py l'exempte de
        # l'avertissement « LIF portrait », a condition que l'heuristique du decodeur ne se
        # declenche pas sur le contenu produit.
        rng = random.Random(285)
        noise = Image.new("RGBA", (200, 300))
        noise.putdata([(rng.randrange(256), rng.randrange(256), rng.randrange(256), 255)
                       for _ in range(200 * 300)])
        b = io.BytesIO()
        noise.save(b, "PNG")
        sources = [banded_png(*size) for size in self.SIZES] + [b.getvalue()]
        # colonnes de degrade : lues en colonnes, elles paraissent plus coherentes
        grad = Image.new("RGBA", (100, 300))
        grad.putdata([(y * 255 // 299, 40, 200 - y * 200 // 299, 255)
                      for y in range(300) for _x in range(100)])
        b = io.BytesIO()
        grad.save(b, "PNG")
        sources.append(b.getvalue())
        for data in sources:
            for fit in T.THUMB_FITS:
                rgba, w, h = self.thumb(data, fit)
                self.assertEqual((w, h), (104, 136))
                self.assertFalse(self.decoder_transposes(rgba, w, h), fit)

    def test_rogner_sans_transparence(self):
        for src in self.SIZES:
            rgba, w, h = self.thumb(banded_png(*src), "rogner")
            self.assertTrue(all(rgba[i] == 255 for i in range(3, len(rgba), 4)), src)

    def test_flou_sans_transparence_et_centre_identique(self):
        data = banded_png(400, 200)
        rgba, w, h = self.thumb(data, "flou")
        self.assertTrue(all(rgba[i] == 255 for i in range(3, len(rgba), 4)))
        im = T._open_rgba(data)
        ref, _, _ = lif.decode(T._encode(T._thumb_contain(im)))
        # image reduite : 104x52 centree (lignes 42 a 93)
        for y in range(42, 94):
            self.assertEqual(rgba[y * w * 4:(y + 1) * w * 4], ref[y * w * 4:(y + 1) * w * 4], y)
        # le haut est rempli (pas une bande vide) et differe de la bande de l'image
        self.assertNotEqual(rgba[0:4], bytes(4))

    def test_flou_remplit_aussi_les_cotes_d_une_image_tres_haute(self):
        rgba, w, h = self.thumb(banded_png(20, 400), "flou")
        self.assertTrue(all(rgba[i] == 255 for i in range(3, len(rgba), 4)))

    def test_flou_opaque_meme_avec_une_image_transparente(self):
        im = Image.new("RGBA", (200, 100), (255, 0, 0, 0))
        im.paste((0, 0, 255, 255), (50, 25, 150, 75))
        b = io.BytesIO()
        im.save(b, "PNG")
        rgba, w, h = self.thumb(b.getvalue(), "flou")
        self.assertTrue(all(rgba[i] == 255 for i in range(3, len(rgba), 4)))

    def test_ratio_extreme_sans_explosion_memoire(self):
        for size in ((2047, 1), (1, 2047)):
            for fit in T.THUMB_FITS:
                _, tw, th = self.thumb(banded_png(*size), fit)
                self.assertEqual((tw, th), (104, 136), (fit, size))

    def test_flam_build_identique_au_convertisseur(self):
        sys.path.insert(0, os.path.join(os.path.dirname(TOOL), "..", "skills", "flam-homebrew", "scripts"))
        import flam_build
        for fit in T.THUMB_FITS:
            for size in ((400, 200), (200, 400), (1, 1)):
                data = banded_png(*size)
                self.assertEqual(flam_build.png_to_thumb(data, fit),
                                 T.png_to_thumb_lif(data, fit), (fit, size))

    def test_modes_differents(self):
        data = banded_png(400, 200)
        outs = {fit: T.png_to_thumb_lif(data, fit) for fit in T.THUMB_FITS}
        self.assertEqual(len(set(outs.values())), 3)

    def test_deterministe(self):
        data = banded_png(333, 177)
        for fit in T.THUMB_FITS:
            self.assertEqual(T.png_to_thumb_lif(data, fit), T.png_to_thumb_lif(data, fit))

    def test_option_invalide_refusee(self):
        with self.assertRaises(ValueError):
            T.png_to_thumb_lif(banded_png(10, 10), "etirer")
        with self.assertRaises(T.ConvError) as cm:
            T.convert("absent.zip", thumb_fit="etirer")
        self.assertEqual(cm.exception.exit_code, T.EXIT_USAGE)
        err = io.StringIO()
        with contextlib.redirect_stderr(err), self.assertRaises(SystemExit) as se:
            T.main(["in.zip", "--thumb-fit", "etirer"])
        self.assertNotEqual(se.exception.code, 0)

    def test_cli_transmet_le_mode(self):
        calls = []
        orig = T.convert
        T.convert = lambda *a, **k: calls.append(k)
        try:
            self.assertEqual(T.main(["in.zip", "--thumb-fit", "rogner"]), T.EXIT_OK)
            self.assertEqual(T.main(["in.zip"]), T.EXIT_OK)
        finally:
            T.convert = orig
        self.assertEqual([c["thumb_fit"] for c in calls], ["rogner", "flou"])


if __name__ == "__main__":
    unittest.main()
