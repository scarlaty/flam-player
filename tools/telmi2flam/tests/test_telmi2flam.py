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
import shutil
import subprocess
import sys
import tempfile
import unittest
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.dirname(HERE)
sys.path.insert(0, TOOL)
sys.dont_write_bytecode = True

from PIL import Image  # noqa: E402

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
        self.assertEqual(r.returncode, 1)
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
            self.assertEqual(lif_dims(z.read("img/thumbnail.lif")), (128, 96))
            w, h = lif_dims(z.read("img/title.lif"))
            self.assertGreaterEqual(w, h)

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

    # hunt3 : runtime Lunii absent ou incomplet -> ConvError (pas de paquet muet)
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

    def test_warn_48k(self):
        tc = Base("setUp")
        tc.setUp()
        try:
            n = base_nodes()
            files = [(a, mp3_frames(0xFB, 0x94, 100) if a == "audios/s0.mp3" else b)
                     for a, b in std_files(nodes=n)]
            tc.convert(files)
            self.assertIn("48000", tc.stderr)
        finally:
            tc.tearDown()


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

if __name__ == "__main__":
    unittest.main()
