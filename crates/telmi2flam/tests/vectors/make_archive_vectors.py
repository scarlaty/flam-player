#!/usr/bin/env python3
"""Vecteurs de parite pour la lecture d'archive du port Rust (#124).

Produit de petites archives synthetiques (aucun contenu sous licence) dans `archive/` et
`archive/expected.json` : pour chaque archive, ce que `zipfile` (Python 3.14.8, l'oracle) et
`telmi2flam.py` en font (ouverture, `namelist()`, lecture de chaque nom, recherche repliee,
`find_root`, sortie de `--info`). Le test Rust `tests/archive_parity.rs` compare son
implementation a ces resultats.

A lancer sous Windows (comme le launcher) avec Python 3.14.8 et Pillow 12.3.0 (versions de
build-converter.ps1), depuis la racine du depot :
    python crates/telmi2flam/tests/vectors/make_archive_vectors.py
"""

import bz2
import contextlib
import hashlib
import io
import json
import lzma
import os
import random
import struct
import sys
import zipfile
import zlib
from compression import zstd

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "archive")
sys.path.insert(0, os.path.join(HERE, "..", "..", "..", "..", "tools", "telmi2flam"))
import telmi2flam  # noqa: E402

DATE = (2024, 1, 1, 0, 0, 0)
META = {"title": "Le château", "uuid": "u-1", "version": 2, "age": 3.5,
        "category": "Conte", "description": "Il était\n\"une\" fois \U0001F600 \\ \x01",
        "image": "Cover.PNG"}


def build(files, compress=zipfile.ZIP_STORED, comment=b""):
    """Archive en memoire. `files` : liste (nom | ZipInfo, octets | str)."""
    buf = io.BytesIO()
    with contextlib.redirect_stderr(io.StringIO()):
        import warnings
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            with zipfile.ZipFile(buf, "w") as z:
                for name, data in files:
                    zi = name if isinstance(name, zipfile.ZipInfo) else zipfile.ZipInfo(name, DATE)
                    if not isinstance(name, zipfile.ZipInfo):
                        zi.compress_type = (compress.get(name, zipfile.ZIP_STORED)
                                            if isinstance(compress, dict) else compress)
                    z.writestr(zi, data.encode("utf-8") if isinstance(data, str) else data)
                z.comment = comment
    return buf.getvalue()


def story(meta=None, prefix="", extra=()):
    m = META if meta is None else meta
    raw = m if isinstance(m, (bytes, str)) else json.dumps(m, ensure_ascii=False)
    return ([(prefix + "metadata.json", raw), (prefix + "nodes.json", "{}"),
             (prefix + "cover.png", b"\x89PNG fake"), (prefix + "images/s0.png", b"img" * 50)]
            + list(extra))


def patch_all(data, old, new):
    assert old in data, old
    return data.replace(old, new)


def set_flag(data, name, bit, central=True, local=True):
    """Pose `bit` dans les drapeaux de l'entree `name` (en-tete local +6, central +8)."""
    b = bytearray(data)
    raw = name.encode("utf-8")
    i = b.find(raw)
    while i != -1:
        if local and b[i - 30:i - 26] == b"PK\x03\x04":
            b[i - 30 + 6] |= bit
        elif central and b[i - 46:i - 42] == b"PK\x01\x02":
            b[i - 46 + 8] |= bit
        i = b.find(raw, i + 1)
    return bytes(b)


def set_central_u16(data, name, off, value):
    b = bytearray(data)
    raw = name.encode("utf-8")
    i = b.find(raw)
    while i != -1:
        if b[i - 46:i - 42] == b"PK\x01\x02":
            struct.pack_into("<H", b, i - 46 + off, value)
        i = b.find(raw, i + 1)
    return bytes(b)


def set_method(data, name, method):
    b = bytearray(data)
    raw = name.encode("utf-8")
    i = b.find(raw)
    while i != -1:
        if b[i - 30:i - 26] == b"PK\x03\x04":
            struct.pack_into("<H", b, i - 30 + 8, method)
        elif b[i - 46:i - 42] == b"PK\x01\x02":
            struct.pack_into("<H", b, i - 46 + 10, method)
        i = b.find(raw, i + 1)
    return bytes(b)


def set_central_u32(data, name, off, value):
    b = bytearray(data)
    raw = name.encode("utf-8")
    i = b.find(raw)
    while i != -1:
        if b[i - 46:i - 42] == b"PK\x01\x02":
            struct.pack_into("<L", b, i - 46 + off, value)
        i = b.find(raw, i + 1)
    return bytes(b)


def unicode_path(name_bytes, target, crc=None):
    zi = zipfile.ZipInfo(name_bytes.decode("cp437"), DATE)
    c = zlib.crc32(name_bytes) if crc is None else crc
    payload = struct.pack("<BL", 1, c) + target.encode("utf-8")
    zi.extra = struct.pack("<HH", 0x7075, len(payload)) + payload
    return zi


def duplicates():
    return build(story(meta={"title": "A"}) + [("metadata.json", json.dumps({"title": "B"})),
                                               ("images/s0.png", b"second")])


def raw_zip(entries):
    """Archive assemblee a la main : `entries` = (nom, methode, octets compresses, crc, taille).

    Permet des flux deflate arbitraires (hostiles) et des tailles ou CRC faux."""
    out = bytearray()
    central = bytearray()
    for name, method, comp, crc, size in entries:
        raw = name.encode("utf-8")
        offset = len(out)
        out += struct.pack("<LHHHHHLLLHH", 0x04034b50, 20, 0x0800, method, 0, 0x21,
                           crc, len(comp), size, len(raw), 0) + raw + comp
        central += struct.pack("<LHHHHHHLLLHHHHHLL", 0x02014b50, 20, 20, 0x0800, method, 0, 0x21,
                               crc, len(comp), size, len(raw), 0, 0, 0, 0, 0, offset) + raw
    cd = len(out)
    out += central
    out += struct.pack("<LHHHHLLH", 0x06054b50, 0, 0, len(entries), len(entries),
                       len(central), cd, 0)
    return bytes(out)


def raw_deflate(data, level=9):
    c = zlib.compressobj(level, zlib.DEFLATED, -15)
    return c.compress(data) + c.flush()


class Bits:
    """Ecriture de bits deflate (LSB d'abord ; codes de Huffman MSB d'abord)."""

    def __init__(self):
        self.v = 0
        self.n = 0

    def put(self, value, count):
        self.v |= value << self.n
        self.n += count

    def huff(self, code, count):
        for i in range(count - 1, -1, -1):
            self.put((code >> i) & 1, 1)

    def bytes(self):
        return self.v.to_bytes((self.n + 7) // 8, "little")


def fixed_block(literals=b"", match=None, final=True, end=True):
    """Bloc a Huffman fixe : litteraux, puis eventuellement (longueur 3..10, distance 1..4)."""
    b = Bits()
    b.put(1 if final else 0, 1)
    b.put(1, 2)
    for ch in literals:
        if ch < 144:
            b.huff(0x30 + ch, 8)
        else:
            b.huff(0x190 + ch - 144, 9)
    if match:
        length, dist = match
        b.huff(length - 3 + 1, 7)                 # codes 257..264 : longueurs 3..10
        b.huff(dist - 1, 5)                       # codes 0..3 : distances 1..4
    if end:
        b.huff(0, 7)
    return b.bytes()


def deflate_story(entry_name, comp, content, crc=None, size=None):
    meta = json.dumps(META, ensure_ascii=False).encode("utf-8")
    return raw_zip([("metadata.json", 0, meta, zlib.crc32(meta), len(meta)),
                    ("nodes.json", 0, b"{}", zlib.crc32(b"{}"), 2),
                    ("cover.png", 0, b"\x89PNG fake", zlib.crc32(b"\x89PNG fake"), 9),
                    (entry_name, 8, comp, zlib.crc32(content) if crc is None else crc,
                     len(content) if size is None else size)])


def deflate_cases():
    c = {}
    words = [b"alpha", b"beta", b"gamma", b"delta", b"epsilon", b"zeta", b"eta", b"theta"]
    rnd = 12345

    def rand():
        nonlocal rnd
        rnd = (rnd * 1103515245 + 12345) & 0x7FFFFFFF
        return rnd >> 8

    text = b" ".join(words[rand() % 8] + str(rand() % 50).encode() for _ in range(4000))
    comp = raw_deflate(text)
    assert comp[0] & 7 in (4, 5), "bloc dynamique attendu"      # BFINAL, BTYPE=2
    c["deflate_dynamic"] = deflate_story("images/d.png", comp, text)
    big = b"".join(bytes([rand() & 0xFF]) * (1 + rand() % 9) for _ in range(60000))
    assert len(big) > 65536
    c["deflate_big"] = deflate_story("images/big.png", raw_deflate(big), big)
    c["deflate_big_fast"] = deflate_story("images/big2.png", raw_deflate(big, 1), big)
    # Flux tronque (sans fin de flux) et octets apres la fin du flux.
    c["deflate_truncated_stream"] = deflate_story("images/t.png", comp[:len(comp) // 2], text)
    c["deflate_no_end_block"] = deflate_story(
        "images/t.png", fixed_block(b"abcabc", end=False, final=False), b"abcabc")
    c["deflate_trailing_bytes"] = deflate_story("images/x.png", comp + b"\x00garbage!!", text)
    # file_size plus petit que le contenu : la lecture s'arrete avant la fin du flux, les
    # octets en trop ne sont jamais atteints (le reliquat `tail` du Rust n'est pas exerce ici,
    # mais par le test unitaire `sortie_aux_bornes_d_un_tour` de zipread.rs).
    c["deflate_trailing_bytes_short_size"] = deflate_story("images/x.png", comp + b"junk", text,
                                                           size=len(text) - 5)
    # Tables et blocs invalides.
    c["deflate_reserved_type"] = deflate_story("images/r.png", b"\x07" + b"\x00" * 8, b"")
    # Bloc dynamique, HDIST = 32 codes de distance (> 30) : refuse par zlib et miniz_oxide.
    c["deflate_hdist_too_big"] = deflate_story("images/b.png", b"\x05" + b"\xff" * 30, b"")
    # Bloc dynamique dont la table des longueurs de code est vide (les 11 longueurs HCLEN a 0).
    c["deflate_code_lengths_empty"] = deflate_story(
        "images/b.png", b"\xed\xfd" + b"\x00" * 30, b"")
    c["deflate_hlit_too_big"] = deflate_story("images/b.png", b"\xfd" + b"\xff" * 30, b"")
    c["deflate_stored_bad_len"] = deflate_story("images/s.png", b"\x01\x05\x00\x00\x00abc", b"abc")
    c["deflate_stored_ok"] = deflate_story("images/s.png", b"\x01\x03\x00\xfc\xffabc", b"abc")
    c["deflate_zlib_wrapper"] = deflate_story("images/z.png", zlib.compress(b"abc"), b"abc")
    # Distance au-dela du debut de la sortie : 'a' puis copie a distance 4 (invalide).
    c["deflate_distance_too_far"] = deflate_story(
        "images/f.png", fixed_block(b"a", match=(3, 4)), b"aaaa")
    c["deflate_distance_ok"] = deflate_story(
        "images/f.png", fixed_block(b"ab", match=(4, 2)), b"ababab")
    c["deflate_distance_ok_overlap"] = deflate_story(
        "images/f.png", fixed_block(b"a", match=(10, 1)), b"a" * 11)
    c["deflate_empty"] = deflate_story("images/e.png", raw_deflate(b""), b"")
    # file_size faux dans les en-tetes (local et central), CRC du contenu reel.
    c["deflate_size_smaller"] = deflate_story("images/d.png", comp, text, size=len(text) - 7)
    c["deflate_size_larger"] = deflate_story("images/d.png", comp, text, size=len(text) + 7)
    c["deflate_size_zero"] = deflate_story("images/d.png", comp, text, size=0)
    c["deflate_size_huge"] = deflate_story("images/d.png", comp, text, size=0xFFFFFFF0)
    c["deflate_crc_wrong"] = deflate_story("images/d.png", comp, text, crc=1)
    jm = b'{"title": "x"}'
    c["deflate_metadata_truncated"] = raw_zip([
        ("metadata.json", 8, raw_deflate(jm)[:-3], zlib.crc32(jm), len(jm)),
        ("nodes.json", 0, b"{}", zlib.crc32(b"{}"), 2)])
    meta = json.dumps({"title": "Zé", "image": "cover.png"}).encode()
    c["deflate_metadata_ok"] = raw_zip([
        ("metadata.json", 8, raw_deflate(meta), zlib.crc32(meta), len(meta)),
        ("nodes.json", 8, raw_deflate(b"{}"), zlib.crc32(b"{}"), 2),
        ("cover.png", 0, b"png", zlib.crc32(b"png"), 3)])
    # Champ 0x7075 vide ou sans nom : ignore.
    zi = zipfile.ZipInfo("empty-up.txt", DATE)
    zi.extra = struct.pack("<HH", 0x7075, 0)
    c["unicode_path_empty"] = build(story(extra=[(zi, b"x")]))
    zi = zipfile.ZipInfo("short-up.txt", DATE)
    zi.extra = struct.pack("<HHBL", 0x7075, 5, 1, zlib.crc32(b"short-up.txt"))
    c["unicode_path_empty_target"] = build(story(extra=[(zi, b"x")]))
    return c


def codec_story(entry_name, method, comp, content, crc=None, size=None):
    """Histoire minimale dont `entry_name` est compressee avec `method` (12, 14 ou 93)."""
    meta = json.dumps(META, ensure_ascii=False).encode("utf-8")
    return raw_zip([("metadata.json", 0, meta, zlib.crc32(meta), len(meta)),
                    ("nodes.json", 0, b"{}", zlib.crc32(b"{}"), 2),
                    ("cover.png", 0, b"\x89PNG fake", zlib.crc32(b"\x89PNG fake"), 9),
                    (entry_name, method, comp, zlib.crc32(content) if crc is None else crc,
                     len(content) if size is None else size)])


def lzma_zip(data):
    """Flux LZMA tel que l'ecrit `zipfile` (en-tete ZIP, marqueur de fin)."""
    c = zipfile.LZMACompressor()
    return c.compress(data) + c.flush()


class LzmaEnc:
    """Encodeur LZMA1 minimal (litteraux, correspondances a distance nouvelle, marqueur de fin
    facultatif) pour fabriquer des flux que `zipfile` n'ecrit pas : sans marqueur de fin,
    distance invalide. lc=3, lp=0, pb=2, en-tete ZIP compris."""

    def __init__(self, dict_size=1 << 16):
        self.low, self.range, self.cache, self.cache_size = 0, 0xFFFFFFFF, 0, 1
        self.buf = bytearray()
        self.p = {}
        self.state, self.rep0, self.out = 0, 0, bytearray()
        self.header = struct.pack("<BBH", 9, 4, 5) + bytes([3 + 9 * (0 + 5 * 2)]) + \
            struct.pack("<L", dict_size)

    def _shift_low(self):
        if self.low < 0xFF000000 or self.low >= 1 << 32:
            temp = self.cache
            while True:
                self.buf.append((temp + (self.low >> 32)) & 0xFF)
                temp = 0xFF
                self.cache_size -= 1
                if self.cache_size == 0:
                    break
            self.cache = (self.low >> 24) & 0xFF
        self.cache_size += 1
        self.low = (self.low & 0x00FFFFFF) << 8

    def _norm(self):
        while self.range < 1 << 24:
            self.range = (self.range << 8) & 0xFFFFFFFF
            self._shift_low()

    def _bit(self, key, b):
        p = self.p.get(key, 1024)
        bound = (self.range >> 11) * p
        if b:
            self.low += bound
            self.range -= bound
            self.p[key] = p - (p >> 5)
        else:
            self.range = bound
            self.p[key] = p + ((2048 - p) >> 5)
        self._norm()

    def _tree(self, name, v, n):
        m = 1
        for i in reversed(range(n)):
            b = (v >> i) & 1
            self._bit((name, m), b)
            m = (m << 1) | b

    def _reverse(self, name, base, v, n):
        m = 1
        for i in range(n):
            b = (v >> i) & 1
            self._bit((name, base + m), b)
            m = (m << 1) | b

    def _len(self, name, v, ps):
        if v < 8:
            self._bit((name, "c"), 0)
            self._tree((name, "low", ps), v, 3)
        elif v < 16:
            self._bit((name, "c"), 1)
            self._bit((name, "c2"), 0)
            self._tree((name, "mid", ps), v - 8, 3)
        else:
            self._bit((name, "c"), 1)
            self._bit((name, "c2"), 1)
            self._tree((name, "high"), v - 16, 8)

    def literal(self, byte):
        pos = len(self.out)
        self._bit(("m", self.state, pos & 3), 0)
        prev = self.out[-1] if self.out else 0
        ctx = prev >> 5
        sym, matched = 1, self.state >= 7
        mbyte = self.out[pos - self.rep0 - 1] if matched else 0
        for i in reversed(range(8)):
            b = (byte >> i) & 1
            if matched:
                mb = (mbyte >> i) & 1
                self._bit(("lit", ctx, ((1 + mb) << 8) + sym), b)
                matched = mb == b
            else:
                self._bit(("lit", ctx, sym), b)
            sym = (sym << 1) | b
        self.out.append(byte)
        s = self.state
        self.state = 0 if s < 4 else (s - 3 if s < 10 else s - 6)

    def match(self, dist, length, copy=True):
        """Correspondance `length` (2..273) a la distance 0-based `dist` (0xFFFFFFFF : fin)."""
        pos = len(self.out)
        self._bit(("m", self.state, pos & 3), 1)
        self._bit(("r", self.state), 0)
        self._len("len", length - 2, pos & 3)
        self.state = 7 if self.state < 7 else 10
        if dist < 4:
            slot = dist
        else:
            n = dist.bit_length() - 1
            slot = 2 * n + ((dist >> (n - 1)) & 1)
        self._tree(("slot", min(length - 2, 3)), slot, 6)
        if slot >= 4:
            direct = (slot >> 1) - 1
            base = (2 | (slot & 1)) << direct
            rem = dist - base
            if slot < 14:
                self._reverse("spec", base - slot, rem, direct)
            else:
                for i in reversed(range(direct - 4)):
                    self.range >>= 1
                    if (rem >> (i + 4)) & 1:
                        self.low += self.range
                    self._norm()
                self._reverse("align", 0, rem & 15, 4)
        self.rep0 = dist
        if copy:
            for _ in range(length):
                self.out.append(self.out[-dist - 1])

    def finish(self, eos=True):
        if eos:
            self.match(0xFFFFFFFF, 2, copy=False)
        for _ in range(5):
            self._shift_low()
        return self.header + bytes(self.buf)


def lzma_crafted(eos=True, bad_dist=False):
    e = LzmaEnc()
    for ch in b"abcabcabc lzma sans marqueur ":
        e.literal(ch)
    e.match(2, 9)
    e.literal(ord("Z"))
    e.match(len(e.out) + 5 if bad_dist else 10, 20, copy=not bad_dist)
    for ch in b" fin":
        e.literal(ch)
    return e.finish(eos), bytes(e.out)


def python_read(method, comp):
    """Ce que le decompresseur de `zipfile` rend pour `comp` en un appel (contenu partiel d'un
    flux tronque, par exemple), ou None s'il echoue."""
    d = zipfile._get_decompressor(method)
    try:
        return d.decompress(comp)
    except Exception:  # noqa: BLE001
        return None


def codec_cases():
    """bzip2 (12), LZMA (14), Zstandard (93) : flux de `zipfile`, flux fabriques, pannes
    (flux corrompu, tronque, CRC faux, bombe de decompression avec `file_size` minuscule)."""
    c = {}
    rnd = random.Random(458)
    text = b" ".join(rnd.choice([b"alpha", b"beta", b"gamma", b"delta", b"zeta"]) +
                     str(rnd.randrange(100)).encode() for _ in range(3000))
    # RLE1 de bzip2 : series de 1 a 300 octets identiques (comptes 0 a 255, series de 4 pile).
    runs = b"".join(bytes([rnd.randrange(256)]) * n for n in list(range(1, 301)) * 2)
    noise = rnd.randbytes(105_000)              # incompressible : blocs bzip2 > 100 000 octets
    zeros = b"\0" * 5_000_000                   # bombe : 5 Mo de zeros
    BZ, LZ, ZS = zipfile.ZIP_BZIP2, zipfile.ZIP_LZMA, zipfile.ZIP_ZSTANDARD

    def add(cid, method, comp, content, **kw):
        c[cid] = codec_story("images/c.png", method, comp, content, **kw)

    # Flux valides, ecrits comme par zipfile.
    for name, data in (("empty", b""), ("text", text), ("runs", runs)):
        add("bzip2_" + name, BZ, bz2.compress(data, 9), data)
        add("lzma_" + name, LZ, lzma_zip(data), data)
        add("zstd_" + name, ZS, zstd.compress(data), data)
    # Niveau 1 (blocs de 100 000 octets) : le premier bloc, incompressible, fait plus de
    # 100 000 octets compresses (cas ou la crate bzip2-rs echoue).
    add("bzip2_multiblock", BZ, bz2.compress(noise + text, 1), noise + text)
    add("zstd_noise", ZS, zstd.compress(noise), noise)          # blocs bruts (Raw_Block)
    add("zstd_checksum", ZS, zstd.compress(text, options={
        zstd.CompressionParameter.checksum_flag: 1}), text)
    add("zstd_level19", ZS, zstd.compress(runs + text, 19), runs + text)
    # Flux LZMA que zipfile n'ecrit pas : sans marqueur de fin (lu par Python), distance
    # au-dela de la sortie (refusee).
    comp, out = lzma_crafted(eos=False)
    add("lzma_no_eos", LZ, comp, out)
    comp, out = lzma_crafted(eos=True)
    add("lzma_crafted_eos", LZ, comp, out)
    comp, out = lzma_crafted(eos=True, bad_dist=True)
    add("lzma_bad_distance", LZ, comp, out)
    # Octets apres la fin du flux : ignores.
    add("bzip2_trailing", BZ, bz2.compress(text, 9) + b"garbage", text)
    add("lzma_trailing", LZ, lzma_zip(text) + b"garbage", text)
    add("zstd_two_frames", ZS, zstd.compress(text) + zstd.compress(b"autre"), text)
    skip = struct.pack("<LL", 0x184D2A50, 3) + b"abc"
    add("zstd_skippable_first", ZS, skip + zstd.compress(text), b"")
    # Flux corrompus (octet du milieu inverse) : Python echoue (OSError, LZMAError, ZstdError).
    for name, method, comp in (("bzip2", BZ, bz2.compress(text, 9)), ("lzma", LZ, lzma_zip(text)),
                               ("zstd", ZS, zstd.compress(text))):
        bad = bytearray(comp)
        bad[len(bad) // 2] ^= 0x55
        add(name + "_corrupt", method, bytes(bad), text)
        # Tronque : CRC du contenu complet (faux), puis CRC du contenu partiel que rend Python.
        cut = comp[:len(comp) * 2 // 3]
        add(name + "_truncated", method, cut, text)
        part = python_read(method, cut)
        assert part is not None
        add(name + "_truncated_partial", method, cut, part)
    # Troncature apres plusieurs blocs : Python rend les blocs complets.
    comp = bz2.compress(noise + text, 1)
    cut = comp[:len(comp) - 3000]
    part = python_read(BZ, cut)
    assert 0 < len(part) < len(noise + text), len(part)
    add("bzip2_multiblock_truncated_partial", BZ, cut, part)
    big = b" ".join(rnd.choice([b"un", b"deux", b"trois", b"quatre", b"cinq"]) +
                    str(rnd.randrange(1000)).encode() for _ in range(40000))
    comp = zstd.compress(big)
    cut = comp[:len(comp) * 3 // 4]
    part = python_read(ZS, cut)
    assert 0 < len(part) < len(big)
    add("zstd_multiblock_truncated_partial", ZS, cut, part)
    # Bloc brut Zstandard tronque : libzstd en rend le debut (le Rust aussi, `finish`).
    # Ecart documente (RUST_READ_ERROR) : bzip2 sans marqueur de fin dont le dernier bloc
    # depasse le tampon de sortie initial de Python (32 Kio), qui en perd la fin.
    comp = zstd.compress(noise[:50000])
    cut = comp[:len(comp) // 2]
    add("zstd_raw_truncated_partial", ZS, cut, python_read(ZS, cut))
    comp = bz2.compress(text + text, 9)
    cut = comp[:-10]
    part = python_read(BZ, cut)
    assert len(part) == 32768, len(part)
    # file_size du contenu complet (sinon data[:file_size] masque l'ecart), CRC du partiel.
    add("bzip2_no_eos_big_block", BZ, cut, part, size=2 * len(text))
    # bzip2 sans marqueur de fin (blocs complets) ; contenu de moins de 32 Kio : Python rend
    # tout le bloc (au-dela, il peut en perdre la fin, ecart documente).
    small = text[:20000]
    comp = bz2.compress(small, 9)
    for k in range(1, 12):
        if python_read(BZ, comp[:-k]) != small:
            break
        add("bzip2_no_eos_%d" % k, BZ, comp[:-k], small)
    bad = bytearray(bz2.compress(small, 9))
    bad[-2] ^= 0x01                             # CRC du flux
    add("bzip2_stream_crc", BZ, bytes(bad), small)
    zc = bytearray(zstd.compress(text, options={zstd.CompressionParameter.checksum_flag: 1}))
    zc[-1] ^= 0x01
    add("zstd_checksum_bad", ZS, bytes(zc), text)
    zr = bytearray(zstd.compress(text))
    zr[4] |= 0x08
    add("zstd_reserved_bit", ZS, bytes(zr), text)
    for wlog in (27, 28):
        frame = struct.pack("<L", 0xFD2FB528) + bytes([0, (wlog - 10) << 3]) + b"\x01\x00\x00"
        add("zstd_window_%d" % wlog, ZS, frame, b"")
    raw3 = bytes([(3 << 3) | 1, 0, 0]) + b"abc"
    for fcs in (2, 3, 5):
        add("zstd_fcs_%d" % fcs, ZS, struct.pack("<L", 0xFD2FB528) + bytes([0x20, fcs]) + raw3,
            b"abc")
    add("zstd_bad_magic", ZS, b"\x00" * 16, b"")
    # Taille annoncee sur 8 octets a 0xFF..FF : ZSTD_CONTENTSIZE_UNKNOWN, non controlee.
    add("zstd_fcs_unknown", ZS, struct.pack("<L", 0xFD2FB528) + bytes([0xC0, 10 << 3]) +
        b"\xff" * 8 + raw3, b"abc")
    # Fenetre 2^28 avec taille annoncee : raccourci « single-pass » de libzstd (lu si la sortie
    # tient dans 32 Kio et la trame est entiere), refus sinon.
    for n, cut in ((256, 0), (32768, 0), (40000, 0), (256, 1)):
        frame = (struct.pack("<L", 0xFD2FB528) + bytes([0x40, 18 << 3]) +
                 struct.pack("<H", n - 256) +
                 bytes([((n << 3) | 1) & 0xFF, (n >> 5) & 0xFF, (n >> 13) & 0xFF]) + b"x" * n)
        add("zstd_big_window_%d%s" % (n, "_truncated" if cut else ""), ZS,
            frame[:len(frame) - cut], b"x" * n)
    # Fenetre annoncee de 3,75 Gio, taille annoncee 256, puis 2000 blocs RLE de 128 Kio : Python
    # echoue des que la sortie depasse 32 Kio ; le Rust aussi, en memoire bornee.
    rle = bytes([((131072 << 3) | 2) & 0xFF, (131072 >> 5) & 0xFF, (131072 >> 13) & 0xFF]) + b"r"
    add("zstd_big_window_rle_flood", ZS, struct.pack("<L", 0xFD2FB528) +
        bytes([0x40, (21 << 3) | 7]) + struct.pack("<H", 0) + rle * 2000 +
        bytes([(0 << 3) | 1, 0, 0]), b"r" * 256)
    # Bloc brut plus grand que Block_Maximum_Size (fenetre 1 Kio) : refuse des l'en-tete.
    n = 2000
    add("zstd_block_too_big", ZS, struct.pack("<L", 0xFD2FB528) + bytes([0x00, 0]) +
        bytes([((n << 3) | 1) & 0xFF, (n >> 5) & 0xFF, (n >> 13) & 0xFF]) + b"y" * n, b"y" * n)
    lz = bytearray(lzma_zip(text))
    lz[9] = 1                                   # premier octet du decodeur d'intervalle
    add("lzma_rc_first_byte", LZ, bytes(lz), text)
    lz = bytearray(lzma_zip(text))
    lz[4] = (2 * 5 + 2) * 9 + 3                 # lc + lp > 4
    add("lzma_lclp", LZ, bytes(lz), text)
    add("lzma_header_only", LZ, lzma_zip(b"")[:9], b"")
    lz = bytearray(lzma_zip(text))
    lz[5:9] = struct.pack("<L", 1)              # dictionnaire minuscule (4096 au moins)
    add("lzma_dict_tiny", LZ, bytes(lz), text)
    # Bombe : 5 Mo de zeros, file_size = 100 (CRC des 100 premiers octets) : Python decode tout
    # le flux, ne garde que 100 octets. Avec une fin de flux corrompue, Python echoue.
    head = zeros[:100]
    for name, method, comp in (("bzip2", BZ, bz2.compress(zeros, 9)),
                               ("lzma", LZ, lzma_zip(zeros)),
                               ("zstd", ZS, zstd.compress(zeros, options={
                                   zstd.CompressionParameter.checksum_flag: 1}))):
        add(name + "_bomb", method, comp, head)
        bad = bytearray(comp)
        bad[-3] ^= 0x55
        add(name + "_bomb_corrupt_tail", method, bytes(bad), head)
    # file_size plus grand que le contenu : tout est rendu (le CRC decide).
    add("bzip2_size_larger", BZ, bz2.compress(text, 9), text, size=len(text) + 50)
    add("lzma_size_larger", LZ, lzma_zip(text), text, size=len(text) + 50)
    add("zstd_size_larger", ZS, zstd.compress(text), text, size=len(text) + 50)
    return c


def cases():
    c = {}
    c["story_stored"] = build(story())
    c["story_deflate"] = build(story(), compress=zipfile.ZIP_DEFLATED)
    c["story_nested"] = build([("__MACOSX/Histoire/._metadata.json", b"\x00\x05"),
                               ("Histoire/sous/metadata.json", "{}"), ("Histoire/sous/nodes.json", "{}")]
                              + story(prefix="Histoire/"))
    # metadata.json en double : find_root voit deux candidats a la racine (erreur Python).
    c["duplicates"] = duplicates()
    c["duplicates_image"] = build(story(extra=[("images/s0.png", b"second"),
                                               ("IMAGES/S0.PNG", b"upper")]))
    nfd = "Été"
    c["names_nfd"] = build(story(extra=[("images/%s.png" % nfd, b"nfd"),
                                        ("images/%s.png" % nfd.lower(), b"nfd2")]))
    cp = build(story(extra=[("images/forXt.png", b"cp437")]))
    c["names_cp437"] = patch_all(cp, b"forXt", "forêt".encode("cp437"))
    c["names_utf8_flag"] = build(story(extra=[("images/été 漢\U0001F600.png", b"u")]))
    c["unicode_path_extra"] = build(story(extra=[
        (unicode_path(b"abc.txt", "été.txt"), b"x"),
        (unicode_path(b"bad.txt", "ignore.txt", crc=1), b"y")]))
    bs = build(story(extra=[("dirXfile.txt", b"bs"), ("aYb.txt", b"nul")]))
    c["names_backslash_nul"] = patch_all(patch_all(bs, b"dirXfile", b"dir\\file"), b"aYb", b"a\x00b")
    data = build(story())
    i = data.find(b"img" * 50)
    corrupt = bytearray(data)
    corrupt[i + 3] ^= 0xFF
    c["crc_corrupt_image"] = bytes(corrupt)
    i = data.find(b'{"title"')
    corrupt = bytearray(data)
    corrupt[i + 3] ^= 0xFF
    c["crc_corrupt_metadata"] = bytes(corrupt)
    c["truncated"] = data[:len(data) // 2]
    c["truncated_cd"] = data[:-30]
    c["not_a_zip"] = b"pas un zip du tout, mais assez long pour la recherche"
    c["empty"] = b""
    c["encrypted_image"] = set_flag(data, "images/s0.png", 0x01)
    c["encrypted_metadata"] = set_flag(data, "metadata.json", 0x01)
    c["unknown_method_metadata"] = set_method(data, "metadata.json", 9)
    c["unknown_method_image"] = set_method(data, "images/s0.png", 97)
    c["bzip2_image"] = build(story(), compress={"images/s0.png": zipfile.ZIP_BZIP2})
    c["lzma_metadata"] = build(story(), compress={"metadata.json": zipfile.ZIP_LZMA})
    c["zstd_image"] = build(story(), compress={"images/s0.png": zipfile.ZIP_ZSTANDARD})
    c["comment_and_prefix"] = b"MZ prefixe autoextractible " * 4 + build(story(), comment=b"PK\x05 commentaire")
    c["overlapped"] = set_central_u32(data, "metadata.json", 20, 5000)
    invalid = b"images/" + bytes([0xFF]) + b".png"
    b = bytearray(patch_all(build(story(extra=[("images/Q.png", b"q")])), b"images/Q.png", invalid))
    j = b.find(invalid)
    while j != -1:
        if b[j - 46:j - 42] == b"PK\x01\x02":
            b[j - 46 + 9] |= 0x08            # drapeau UTF-8 (bit 11) du repertoire central
        j = b.find(invalid, j + 1)
    c["utf8_flag_invalid_name"] = bytes(b)
    c["local_name_differs"] = (lambda d: d[:d.find(b"images/s0.png")] + b"images/s1.png"
                               + d[d.find(b"images/s0.png") + 13:])(data)
    c["extract_version_64"] = (lambda d: bytes(bytearray(d[:d.find(b"PK\x01\x02") + 6]) + b"\x40"
                                               + d[d.find(b"PK\x01\x02") + 7:]))(data)
    zi = zipfile.ZipInfo("x.txt", DATE)
    zi.extra = b"\x01\x00\x10\x00ab"
    c["corrupt_extra_field"] = build(story(extra=[(zi, b"x")]))
    dd = build(story(), compress=zipfile.ZIP_DEFLATED)
    k = dd.find(b"images/s0.png") + 13
    bad = bytearray(dd)
    bad[k + 2] ^= 0x55
    c["deflate_corrupt_image"] = bytes(bad)
    c["root_none"] = build([("readme.txt", "rien"), ("Metadata.json", "{}"), ("nodes.json", "{}")])
    c["root_ambiguous"] = build(story(prefix="b/") + story(prefix="a/"))
    c["meta_types"] = build(story(meta=(
        '{"title": true, "uuid": [1], "version": 1e16, "age": 0.1, "category": -0,'
        ' "description": {"a": 1}, "image": 5, "title2": 1e-7}')))
    c["meta_numbers"] = build(story(meta=(
        '{"title": 18446744073709551615, "uuid": -9223372036854775808, "version": 1E2,'
        ' "age": -0.0, "category": 123456789.125, "description": 5e-324, "image": ""}')))
    c["meta_dup_keys_bom"] = build(story(meta=b"\xef\xbb\xbf" + json.dumps(
        {"title": "A"}).encode()[:-1] + b', "image": "absent.png", "title": "B"}'))
    c["meta_invalid_json"] = build(story(meta="{bad json"))
    c["meta_list"] = build(story(meta="[]"))
    c["meta_nan"] = build(story(meta='{"title": NaN}'))
    c["meta_not_utf8"] = build(story(meta=b'{"title": "\xe9"}'))
    c["meta_big_int"] = build(story(meta='{"age": 123456789012345678901234567890}'))
    c["meta_lone_surrogate"] = build(story(meta='{"title": "\\ud800"}'))
    c["cover_title_png"] = build([("metadata.json", '{"title": "T"}'), ("nodes.json", "{}"),
                                  ("TITLE.PNG", b"t")])
    c["cover_none"] = build([("metadata.json", '{"title": "T", "image": "x.png"}'),
                             ("nodes.json", "{}")])
    c.update(deflate_cases())
    c.update(codec_cases())
    return c


# Ecarts admis (PORT-RUST.md §3 et §4.1) : code de sortie attendu du Rust pour --info.
RUST_INFO_EXIT = {
    "encrypted_metadata": 3,        # Python : RuntimeError, internal_error (6)
    "utf8_flag_invalid_name": 3,    # Python : UnicodeDecodeError, internal_error (6)
    "meta_big_int": 4,              # entier hors 64 bits (json_limit)
    "meta_lone_surrogate": 4,       # surrogate isole (json_limit)
}


# Ecarts admis de lecture (PORT-RUST.md section 3) : (archive, entree) -> erreur du Rust.
# miniz_oxide refuse tout de suite un bloc dynamique dont la table des longueurs de code est
# vide (les longueurs HCLEN toutes a 0), que zlib accepte en attendant la suite du flux ;
# Python ne signale pas un flux tronque.
RUST_READ_ERROR = {
    ("deflate_code_lengths_empty", "images/b.png"): "corrupt",
    # #458 : bzip2 sans marqueur de fin, Python perd la fin du dernier bloc (tampon de sortie
    # de 32 Kio plein quand l'entree est epuisee) ; le Rust rend le bloc entier (CRC faux).
    ("bzip2_no_eos_big_block", "images/c.png"): "corrupt",
}


def classify(exc):
    if isinstance(exc, NotImplementedError):   # sous-classe de RuntimeError
        return "corrupt"
    if isinstance(exc, RuntimeError):
        return "encrypted"
    # OSError : aussi le flux bzip2 invalide (« Invalid data stream ») ; LZMAError, ZstdError :
    # flux LZMA ou Zstandard invalide. Python echoue en internal_error (6), le Rust en code 3.
    if (isinstance(exc, (UnicodeDecodeError, OSError, lzma.LZMAError, zstd.ZstdError))
            and not isinstance(exc, zipfile.BadZipFile)):
        return "python_fails_too"
    if isinstance(exc, (zipfile.BadZipFile, NotImplementedError, EOFError, zlib.error)):
        return "corrupt"
    raise exc


def describe(cid, data):
    path = os.path.join(OUT, cid + ".zip")
    with open(path, "wb") as f:
        f.write(data)
    exp = {"id": cid}
    try:
        zf = zipfile.ZipFile(path)
    except Exception as e:  # noqa: BLE001
        exp["open"] = classify(e)
    else:
        with zf:
            exp["open"] = "ok"
            names = zf.namelist()
            exp["namelist"] = names
            reads = []
            for n in dict.fromkeys(names):
                info = zf.getinfo(n)
                try:
                    content = zf.read(n)
                except Exception as e:  # noqa: BLE001
                    reads.append({"name": n, "error": classify(e)})
                    continue
                if (len(content) > 65536 and info.compress_type
                        in (zipfile.ZIP_BZIP2, zipfile.ZIP_LZMA, zipfile.ZIP_ZSTANDARD)):
                    # gros contenus des cas bzip2/LZMA/Zstandard : empreinte et taille
                    reads.append({"name": n, "sha256": hashlib.sha256(content).hexdigest(),
                                  "len": len(content)})
                else:
                    reads.append({"name": n, "hex": content.hex()})
                if (cid, n) in RUST_READ_ERROR:
                    assert "hex" in reads[-1], "ecart devenu inutile : Python refuse aussi"
                    reads[-1]["rust_error"] = RUST_READ_ERROR[(cid, n)]
            exp["reads"] = reads
            index = {}
            for n in names:
                index.setdefault(telmi2flam._fold(n), n)
            probes = sorted(set(names) | {n.upper() for n in names}
                            | {telmi2flam.unicodedata.normalize("NFC", n) for n in names}
                            | {"absent.png"})
            exp["lookup"] = [[p, p if p in set(names) else index.get(telmi2flam._fold(p))]
                             for p in probes]
            try:
                exp["root"] = telmi2flam.find_root(names)
            except telmi2flam.ConvError as e:
                exp["root_error"] = [e.code, e.exit_code]
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        code = telmi2flam.main([path, "--info"])
    exp["info_exit_python"] = code
    exp["info_exit"] = RUST_INFO_EXIT.get(cid, code)
    exp["info_stdout"] = out.getvalue() if code == 0 else ""
    if exp["info_exit"] != code:
        exp["info_stdout"] = ""
    return exp


def main():
    if sys.version_info[:3] != (3, 14, 8):
        sys.exit("vecteurs a produire avec Python 3.14.8 (oracle de build-converter.ps1)")
    if os.name != "nt":
        sys.exit("vecteurs a produire sous Windows (noms avec \\ : _sanitize_filename)")
    os.makedirs(OUT, exist_ok=True)
    for f in os.listdir(OUT):
        os.remove(os.path.join(OUT, f))
    expected = [describe(cid, data) for cid, data in cases().items()]
    with open(os.path.join(OUT, "expected.json"), "w", encoding="utf-8", newline="\n") as f:
        json.dump(expected, f, ensure_ascii=True, indent=1)
        f.write("\n")
    print("%d vecteurs ecrits dans %s" % (len(expected), OUT))


if __name__ == "__main__":
    main()
