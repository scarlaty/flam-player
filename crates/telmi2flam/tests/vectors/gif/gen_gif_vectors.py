"""Vecteurs de parité du décodage GIF (port Rust, issue #459).

L'oracle est `_open_rgba` de `tools/telmi2flam/telmi2flam.py`, exécuté avec **Python 3.14 et
Pillow 12.3.0** : le script refuse de tourner avec une autre version de Pillow. Les GIF sont
écrits par Pillow (animations, entrelacement, transparence, palettes) ou construits à la main
(LZW maison, blocs altérés), sans contenu sous licence. Déterministe.

    python -I crates/telmi2flam/tests/vectors/gif/gen_gif_vectors.py

Fichiers produits (lus par `tests/gif_vectors.rs`) :
- `vectors.tsv` : `nom  gif (hexadécimal)  attendu`, où `attendu` vaut
  `rgba:<largeur>x<hauteur>:<sha256 de tobytes()>` ou `unreadable` ;
- `alterations.tsv` : `base  opération  argument  attendu` pour chaque troncature (`cut`) et
  chaque octet inversé (`xor`) des bases (la base est dans `vectors.tsv`).
"""

import hashlib
import io
import os
import random
import struct
import sys

PILLOW = "12.3.0"
HERE = os.path.dirname(os.path.abspath(__file__))
TOOLS = os.path.join(HERE, "..", "..", "..", "..", "..", "tools", "telmi2flam")


def check_versions():
    import PIL
    if PIL.__version__ != PILLOW or sys.version_info[:2] != (3, 14):
        sys.exit("oracle attendu : Python 3.14 + Pillow %s (trouvé : Python %s, Pillow %s)"
                 % (PILLOW, sys.version.split()[0], PIL.__version__))


def oracle(data):
    import telmi2flam
    try:
        im = telmi2flam._open_rgba(data)
        raw = im.tobytes()
    except Exception:
        return "unreadable"
    assert im.mode == "RGBA"
    return "rgba:%dx%d:%s" % (im.size[0], im.size[1], hashlib.sha256(raw).hexdigest())


def lzw_encode(indices, bits, clear_every=None, end=True, early_change=0):
    """Encodeur LZW GIF simple (codes de taille variable, table jusqu'à 4096)."""
    clear = 1 << bits
    eoi = clear + 1
    out = []
    size = bits + 1
    table = {(i,): i for i in range(clear)}
    nxt = clear + 2
    out.append((clear, size))
    w = ()
    count = 0
    for k in indices:
        wk = w + (k,)
        if wk in table:
            w = wk
            continue
        out.append((table[w], size))
        if nxt < 4096:
            table[wk] = nxt
            nxt += 1
            if nxt - 1 == (1 << size) - early_change and size < 12:
                size += 1
        w = (k,)
        count += 1
        if clear_every and count % clear_every == 0:
            out.append((table[w], size))
            out.append((clear, size))
            table = {(i,): i for i in range(clear)}
            nxt = clear + 2
            size = bits + 1
            w = ()
    if w:
        out.append((table[w], size))
    if end:
        out.append((eoi, size))
    acc = nb = 0
    data = bytearray()
    for code, sz in out:
        acc |= code << nb
        nb += sz
        while nb >= 8:
            data.append(acc & 0xFF)
            acc >>= 8
            nb -= 8
    if nb:
        data.append(acc & 0xFF)
    return bytes(data)


def literal_stream(segments, bits):
    """Train de bits LZW fait de littéraux seuls : pour chaque segment, clear, littéraux, code
    de fin, avec la taille de code que suit `GifDecode.c`."""
    clear = 1 << bits
    acc = nb = 0
    data = bytearray()
    size = bits + 1
    nxt = clear + 2

    def put(code):
        nonlocal acc, nb
        acc |= code << nb
        nb += size
        while nb >= 8:
            data.append(acc & 0xFF)
            acc >>= 8
            nb -= 8

    for seg in segments:
        put(clear)
        size, nxt = bits + 1, clear + 2
        for k, v in enumerate(seg):
            put(v)
            if k > 0 and nxt < 4096:
                if nxt == (1 << size) - 1 and size < 12:
                    size += 1
                nxt += 1
        put(clear + 1)
    if nb:
        data.append(acc & 0xFF)
    return bytes(data)


def sub_blocks(data, size=255):
    out = bytearray()
    for i in range(0, len(data), size):
        part = data[i:i + size]
        out.append(len(part))
        out += part
    out.append(0)
    return bytes(out)


def gif(w, h, frames, gpal=None, version=b"GIF89a", trailer=b";", bg=0):
    """frames : liste de dict(x, y, w, h, indices, bits, lpal, interlace, gce, ext)."""
    flags = 0
    pal = b""
    if gpal is not None:
        n = max(1, (len(gpal) // 3 - 1).bit_length())
        n = max(n, 1)
        flags = 0x80 | (n - 1)
        pal = gpal + bytes(3 * (1 << n) - len(gpal))
    out = bytearray(version + struct.pack("<HHBBB", w, h, flags, bg, 0) + pal)
    for f in frames:
        out += f.get("ext", b"")
        if f.get("gce") is not None:
            tflag, tidx, disp = f["gce"]
            out += b"!\xf9\x04" + bytes([(disp << 2) | tflag]) + b"\x0a\x00" + bytes([tidx]) + b"\x00"
        lflags = 0x40 if f.get("interlace") else 0
        lpal = b""
        if f.get("lpal") is not None:
            n = max(1, (len(f["lpal"]) // 3 - 1).bit_length())
            lflags |= 0x80 | (n - 1)
            lpal = f["lpal"] + bytes(3 * (1 << n) - len(f["lpal"]))
        out += b"," + struct.pack("<HHHHB", f["x"], f["y"], f["w"], f["h"], lflags) + lpal
        bits = f.get("bits", 8)
        indices = f["indices"]
        if f.get("interlace"):
            rows = [indices[y * f["w"]:(y + 1) * f["w"]] for y in range(f["h"])]
            order = (list(range(0, f["h"], 8)) + list(range(4, f["h"], 8))
                     + list(range(2, f["h"], 4)) + list(range(1, f["h"], 2)))
            indices = [v for y in order for v in rows[y]]
        raw = f.get("raw")
        if raw is None:
            raw = sub_blocks(lzw_encode(indices, bits, f.get("clear_every"), f.get("end", True)))
        out += bytes([f.get("code_size", bits)]) + raw
    out += trailer
    return bytes(out)


def pillow_gif(im, **kw):
    b = io.BytesIO()
    im.save(b, "GIF", **kw)
    return b.getvalue()


def cases():
    from PIL import Image
    rng = random.Random(459)
    out = []

    def add(name, data):
        out.append((name, data))

    pal16 = bytes(rng.randrange(256) for _ in range(48))
    pal256 = bytes(rng.randrange(256) for _ in range(768))
    ident = bytes(v for i in range(256) for v in (i, i, i))
    # --- écrits par Pillow
    for (w, h) in ((1, 1), (7, 5), (33, 21), (64, 48)):
        im = Image.new("P", (w, h))
        im.putpalette(pal256)
        im.putdata([rng.randrange(256) for _ in range(w * h)])
        add("pil_p_%dx%d" % (w, h), pillow_gif(im))
        add("pil_p_%dx%d_entrelace" % (w, h), pillow_gif(im, interlace=True))
        add("pil_p_%dx%d_transp" % (w, h), pillow_gif(im, transparency=3))
        g = Image.new("L", (w, h))
        g.putdata([(x * 37) % 256 for x in range(w * h)])
        add("pil_l_%dx%d" % (w, h), pillow_gif(g))
        rgb = Image.new("RGB", (w, h))
        rgb.putdata([(rng.randrange(256), rng.randrange(256), rng.randrange(256))
                     for _ in range(w * h)])
        add("pil_rgb_%dx%d" % (w, h), pillow_gif(rgb))
    a = Image.new("P", (20, 10), 1)
    a.putpalette(pal16)
    b2 = a.copy()
    b2.paste(5, (3, 3, 9, 7))
    add("pil_anime", pillow_gif(a, save_all=True, append_images=[b2], duration=50, loop=0))
    add("pil_anime_transp", pillow_gif(a, save_all=True, append_images=[b2], transparency=1,
                                       disposal=2))
    # --- construits à la main
    def idx(w, h, top=15):
        return [rng.randrange(top + 1) for _ in range(w * h)]

    i45 = idx(4, 5)
    add("main_global", gif(4, 5, [dict(x=0, y=0, w=4, h=5, indices=i45, bits=4)], pal16))
    add("main_local", gif(4, 5, [dict(x=0, y=0, w=4, h=5, indices=i45, bits=4, lpal=pal16)]))
    add("main_sans_palette", gif(4, 5, [dict(x=0, y=0, w=4, h=5, indices=i45, bits=4)]))
    add("main_identite_globale", gif(4, 5, [dict(x=0, y=0, w=4, h=5, indices=i45, bits=8)],
                                     ident))
    add("main_identite_locale", gif(4, 5, [dict(x=0, y=0, w=4, h=5, indices=i45, bits=8,
                                                 lpal=ident)], pal16))
    add("main_identite_locale_transp", gif(4, 5, [dict(x=0, y=0, w=4, h=5, indices=i45, bits=8,
                                                       lpal=ident, gce=(1, 3, 0))], pal16))
    add("main_identite_locale_sans_globale", gif(4, 5, [dict(x=0, y=0, w=4, h=5, indices=i45,
                                                             bits=8, lpal=ident, gce=(1, 3, 0))]))
    add("main_transp_sans_palette", gif(4, 5, [dict(x=0, y=0, w=4, h=5, indices=i45, bits=4,
                                                    gce=(1, 7, 0))]))
    add("main_cadre_partiel", gif(10, 8, [dict(x=3, y=2, w=4, h=5, indices=i45, bits=4)], pal16))
    add("main_cadre_partiel_transp", gif(10, 8, [dict(x=3, y=2, w=4, h=5, indices=i45, bits=4,
                                                      gce=(1, 9, 2))], pal16))
    add("main_cadre_deborde", gif(5, 4, [dict(x=3, y=2, w=4, h=5, indices=i45, bits=4)], pal16))
    add("main_cadre_vide", gif(5, 4, [dict(x=0, y=0, w=0, h=5, indices=[], bits=4)], pal16))
    add("main_ecran_nul", gif(0, 0, [dict(x=0, y=0, w=4, h=5, indices=i45, bits=4)], pal16))
    for n in (1, 2, 3, 5, 6, 7, 8, 9, 10, 11, 12, 13):
        i3 = idx(6, 13)
        add("main_entrelace_h%d" % n, gif(6, n, [dict(x=0, y=0, w=6, h=n, indices=i3[:6 * n],
                                                      bits=4, interlace=True)], pal16))
    big = idx(64, 64, 255)
    add("main_table_pleine", gif(64, 64, [dict(x=0, y=0, w=64, h=64, indices=big, bits=8)],
                                 pal256))
    add("main_clear_regulier", gif(64, 64, [dict(x=0, y=0, w=64, h=64, indices=big, bits=8,
                                                 clear_every=300)], pal256))
    for bits in (1, 2, 3, 7):
        ib = [rng.randrange(1 << bits) for _ in range(30)]
        add("main_bits_%d" % bits, gif(6, 5, [dict(x=0, y=0, w=6, h=5, indices=ib, bits=bits)],
                                       pal16))
    add("main_bits_0", gif(3, 1, [dict(x=0, y=0, w=3, h=1, indices=[0, 0, 0], bits=0,
                                       raw=sub_blocks(bytes([0b11111001, 0])))], pal16))
    add("main_bits_13", gif(3, 1, [dict(x=0, y=0, w=3, h=1, indices=[0] * 3, bits=8,
                                        code_size=13)], pal16))
    add("main_sans_fin", gif(4, 5, [dict(x=0, y=0, w=4, h=5, indices=i45, bits=4, end=False)],
                             pal16))
    add("main_fin_prematuree", gif(4, 5, [dict(x=0, y=0, w=4, h=5, indices=i45[:10], bits=4)],
                                   pal16))
    add("main_trop_de_pixels", gif(4, 5, [dict(x=0, y=0, w=4, h=5, indices=i45 + i45, bits=4)],
                                   pal16))
    add("main_code_invalide", gif(4, 5, [dict(x=0, y=0, w=4, h=5, indices=i45, bits=4,
                                              raw=sub_blocks(bytes([0x10, 0xff, 0xff, 0xff])))],
                                  pal16))
    add("main_premier_code_haut", gif(4, 5, [dict(x=0, y=0, w=4, h=5, indices=i45, bits=4,
                                                  raw=sub_blocks(bytes([0x10, 0x12, 0, 0])))],
                                      pal16))
    add("main_sous_blocs_courts", gif(4, 5, [dict(x=0, y=0, w=4, h=5, indices=i45, bits=4,
                                                  raw=sub_blocks(lzw_encode(i45, 4), 1))],
                                      pal16))
    add("main_sans_trailer", gif(4, 5, [dict(x=0, y=0, w=4, h=5, indices=i45, bits=4)], pal16,
                                 trailer=b""))
    add("main_gif87a", gif(4, 5, [dict(x=0, y=0, w=4, h=5, indices=i45, bits=4)], pal16,
                           version=b"GIF87a"))
    ext = (b"!\xfe\x05hello\x03abc\x00" + b"!\xff\x0bNETSCAPE2.0\x03\x01\x00\x00\x00"
           + b"!\x01\x0c" + bytes(12) + b"\x00" + b"\x00garbage")
    add("main_extensions", gif(4, 5, [dict(x=0, y=0, w=4, h=5, indices=i45, bits=4, ext=ext,
                                           gce=(1, 2, 1))], pal16))
    add("main_gce_court", gif(4, 5, [dict(x=0, y=0, w=4, h=5, indices=i45, bits=4,
                                          ext=b"!\xf9\x02\x01\x00\x00")], pal16))
    add("main_gce_sans_transp", gif(4, 5, [dict(x=0, y=0, w=4, h=5, indices=i45, bits=4,
                                                gce=(0, 2, 0))], pal16))
    add("main_deux_gce", gif(4, 5, [dict(x=0, y=0, w=4, h=5, indices=i45, bits=4,
                                         ext=b"!\xf9\x04\x01\x0a\x00\x05\x00", gce=(0, 2, 0))],
                             pal16))
    add("main_sans_image", gif(4, 5, [], pal16))
    add("main_palette_globale_courte", (b"GIF89a" + struct.pack("<HHBBB", 4, 5, 0x87, 0, 0)
                                        + pal16))
    add("main_bombe", gif(65535, 65535, [dict(x=0, y=0, w=4, h=5, indices=i45, bits=4)], pal16))
    add("main_bombe_cadre", gif(4, 5, [dict(x=65000, y=65000, w=535, h=535, indices=[0],
                                            bits=4)], pal16))
    add("main_grand_vide", gif(3000, 2000, [dict(x=0, y=0, w=4, h=5, indices=i45, bits=4)],
                               pal16))
    add("entete_court", b"GIF89a\x04\x00")
    add("entete_seul", b"GIF89a" + struct.pack("<HHBBB", 4, 5, 0, 0, 0))
    return out


def big_cases():
    """Données de plus de 64 Kio (relance par blocs d'`ImageFile.load`), écrites en fichiers
    `<nom>.gif` (référencés `@<nom>.gif` dans `vectors.tsv`)."""
    from PIL import Image
    rng = random.Random(4596)
    pal = bytes(rng.randrange(256) for _ in range(48))
    i = [rng.randrange(16) for _ in range(20)]
    # deux moitiés séparées par un code de fin, dans un même train de bits : Pillow relance le
    # décodeur sur le bloc suivant de 64 Kio, qui reprend après le code de fin
    raw = sub_blocks(literal_stream([i[:10], i[10:]], 4))
    pad = b";" + bytes(rng.randrange(256) for _ in range(70000))
    out = [("grand_fin_puis_suite", gif(4, 5, [dict(x=0, y=0, w=4, h=5, indices=i, bits=4,
                                                     raw=raw)], pal,
                                         trailer=pad)),
           ("grand_fin_sans_suite", gif(4, 5, [dict(x=0, y=0, w=4, h=5, indices=i, bits=4,
                                                     raw=raw)], pal))]
    im = Image.new("P", (256, 256))
    im.putpalette(bytes(rng.randrange(256) for _ in range(768)))
    im.putdata([rng.randrange(256) for _ in range(256 * 256)])
    out.append(("grand_bruit_256", pillow_gif(im)))
    return out


def alteration_bases():
    rng = random.Random(4595)
    pal = bytes(rng.randrange(256) for _ in range(12))
    i = [rng.randrange(4) for _ in range(5 * 4)]
    from PIL import Image
    a = Image.new("P", (6, 4), 1)
    a.putpalette(bytes(rng.randrange(256) for _ in range(24)))
    a.putpixel((2, 1), 3)
    b = a.copy()
    b.paste(2, (0, 0, 3, 2))
    buf = io.BytesIO()
    a.save(buf, "GIF", save_all=True, append_images=[b], transparency=1, duration=40, loop=0)
    return [
        ("alt_pillow_anime", buf.getvalue()),
        ("alt_simple", gif(5, 4, [dict(x=0, y=0, w=5, h=4, indices=i, bits=2, gce=(1, 1, 0))],
                           pal)),
        ("alt_entrelace", gif(7, 5, [dict(x=1, y=0, w=5, h=4, indices=i, bits=2,
                                          interlace=True, lpal=pal)])),
    ]


def write_lines(path, lines):
    with open(path, "wb") as f:
        f.write(("\n".join(lines) + "\n").encode("utf-8"))


def main():
    sys.path.insert(0, TOOLS)
    lines = ["# nom\tgif (hexadecimal)\tattendu (genere par gen_gif_vectors.py, Pillow 12.3.0)"]
    bases = alteration_bases()
    for name, data in cases() + bases:
        lines.append("%s\t%s\t%s" % (name, data.hex(), oracle(data)))
    for name, data in big_cases():
        with open(os.path.join(HERE, name + ".gif"), "wb") as f:
            f.write(data)
        lines.append("%s\t@%s.gif\t%s" % (name, name, oracle(data)))
    write_lines(os.path.join(HERE, "vectors.tsv"), lines)
    alt = ["# base\toperation\targument\tattendu (genere par gen_gif_vectors.py)"]
    for name, data in bases:
        for n in range(len(data)):
            alt.append("%s\tcut\t%d\t%s" % (name, n, oracle(data[:n])))
        for k in range(6, len(data)):
            d = bytearray(data)
            d[k] ^= 0xFF
            alt.append("%s\txor\t%d\t%s" % (name, k, oracle(bytes(d))))
    write_lines(os.path.join(HERE, "alterations.tsv"), alt)


if __name__ == "__main__":
    check_versions()
    main()
