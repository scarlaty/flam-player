#!/usr/bin/env python3
"""Vecteurs de parite pour la lecture d'archive du port Rust (#124).

Produit de petites archives synthetiques (aucun contenu sous licence) dans `archive/` et
`archive/expected.json` : pour chaque archive, ce que `zipfile` (Python 3.12) et
`telmi2flam.py` en font (ouverture, `namelist()`, lecture de chaque nom, recherche repliee,
`find_root`, sortie de `--info`). Le test Rust `tests/archive_parity.rs` compare son
implementation a ces resultats.

A lancer sous Windows (comme le launcher) avec Pillow 12.3.0, depuis la racine du depot :
    python crates/telmi2flam/tests/vectors/make_archive_vectors.py
"""

import contextlib
import io
import json
import os
import struct
import sys
import zipfile
import zlib

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
    return c


# Ecarts admis (PORT-RUST.md §3 et §4.1) : code de sortie attendu du Rust pour --info.
RUST_INFO_EXIT = {
    "encrypted_metadata": 3,        # Python : RuntimeError, internal_error (6)
    "utf8_flag_invalid_name": 3,    # Python : UnicodeDecodeError, internal_error (6)
    "lzma_metadata": 7,             # Python lit le LZMA ; le Rust delegue (D4)
    "meta_big_int": 4,              # entier hors 64 bits (json_limit)
    "meta_lone_surrogate": 4,       # surrogate isole (json_limit)
}


def classify(exc):
    if isinstance(exc, NotImplementedError):   # sous-classe de RuntimeError
        return "corrupt"
    if isinstance(exc, RuntimeError):
        return "encrypted"
    if isinstance(exc, (UnicodeDecodeError, OSError)) and not isinstance(exc, zipfile.BadZipFile):
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
                if info.compress_type in (zipfile.ZIP_BZIP2, zipfile.ZIP_LZMA):
                    reads.append({"name": n, "error": "unsupported"})
                else:
                    reads.append({"name": n, "hex": content.hex()})
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
