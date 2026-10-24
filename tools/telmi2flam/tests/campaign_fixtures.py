"""
Generateurs de paquets TELMI synthetiques pour les tests des outils de campagne (compat.py,
explore.py, parity.py) et le test de bout en bout `tests/e2e/e2e_telmi.py`.

Repris de l'ancien `test_telmi2flam.py`, supprime avec le convertisseur Python (#140) :
memes fonctions, memes signatures, memes paquets. Seule difference, sans dependance : les PNG
sont encodes a la main (zlib de la bibliotheque standard) au lieu de Pillow ; l'image
(taille, couleur RGBA) est la meme, les octets du PNG peuvent differer. Les MP3 sont
identiques a l'octet pres.
"""
import io
import json
import os
import shutil
import struct
import sys
import tempfile
import unittest
import zipfile
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.dirname(HERE)
if TOOL not in sys.path:
    sys.path.insert(0, TOOL)


def _chunk(kind, data):
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))


def png(w, h, color=(200, 10, 10, 255)):
    """PNG RGBA 8 bits w x h d'une seule couleur (filtre 0, zlib niveau 6)."""
    row = b"\x00" + bytes(color) * w
    ihdr = struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)
    return (b"\x89PNG\r\n\x1a\n" + _chunk(b"IHDR", ihdr)
            + _chunk(b"IDAT", zlib.compress(row * h, 6)) + _chunk(b"IEND", b""))


# Tables MPEG audio (debit en kbit/s par indice) : version (3 = MPEG1, 2 = MPEG2,
# 0 = MPEG2.5), couche (1 = III, 2 = II, 3 = I).
_BITRATES = {
    (3, 3): [0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448],
    (3, 2): [0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384],
    (3, 1): [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320],
    (2, 3): [0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256],
    (2, 2): [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160],
    (2, 1): [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160],
}
for _layer in (1, 2, 3):
    _BITRATES[(0, _layer)] = _BITRATES[(2, _layer)]
_SAMPLERATES = {3: [44100, 48000, 32000], 2: [22050, 24000, 16000], 0: [11025, 12000, 8000]}


def mp3_frame_len(b1, b2):
    """Longueur d'une trame MPEG audio d'en-tete FF b1 b2 (regles de l'ancien mp3map.py,
    comme `crates/telmi2flam/src/mp3map.rs`)."""
    version, layer = (b1 >> 3) & 3, (b1 >> 1) & 3
    if b1 & 0xE0 != 0xE0 or version == 1 or layer == 0:
        raise ValueError("en-tete MPEG invalide : %02X %02X" % (b1, b2))
    bitrate_index, sr_index, padding = (b2 >> 4) & 15, (b2 >> 2) & 3, (b2 >> 1) & 1
    if bitrate_index in (0, 15) or sr_index == 3:
        raise ValueError("debit ou frequence invalide : %02X" % b2)
    bitrate = _BITRATES[(version, layer)][bitrate_index] * 1000
    samplerate = _SAMPLERATES[version][sr_index]
    if layer == 3:
        return (12 * bitrate // samplerate + padding) * 4
    coef = 144 if (version == 3 or layer == 2) else 72
    return coef * bitrate // samplerate + padding


def mp3_frames(b1=0xFB, b2=0x90, n=200):
    """n trames MPEG audio vides (en-tete FF b1 b2 00 puis zeros) ; defaut : MPEG1 couche
    III, 128 kbit/s, 44 100 Hz."""
    return (bytes([0xFF, b1, b2, 0]) + bytes(mp3_frame_len(b1, b2) - 4)) * n


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


def zip_bytes(files, compress=zipfile.ZIP_STORED):
    """Archive en memoire, horodatage fixe (1980-01-01). `files` : liste (nom, octets | str) ;
    `compress` : methode de toutes les entrees, ou dict {nom: methode} (defaut stored)."""
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as z:
        for name, data in files:
            zi = zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0))
            zi.compress_type = (compress.get(name, zipfile.ZIP_STORED)
                                if isinstance(compress, dict) else compress)
            z.writestr(zi, data.encode("utf-8") if isinstance(data, str) else data)
    return buf.getvalue()


class Base(unittest.TestCase):
    """Dossier temporaire par test et archive TELMI ecrite a la demande."""

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
