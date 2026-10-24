"""
Matrice de reechantillonnage de la parite (PORT-RUST.md §4.3, point 2), sans dependance.

Ecrit, dans un dossier choisi par l'appelant (temporaire : rien n'est commite), des PNG
(modes x tailles sources impaires ou premieres x formes x contenus) et l'index `matrix.json`
(un element par PNG et par operation reelle du convertisseur : scene 320x212, vignette
104x136 en mode `bandes`, inventaire 128x128, titre 89x120, borne 2047).

Les `.lif` attendus sont figes dans `crates/telmi2flam/tests/data/matrix_expected.json`
(sortie de l'ancien oracle Python, Pillow 12.3.0, PORT-RUST.md §4.5) avec l'empreinte de chaque
PNG (`png_fingerprint` : en-tetes et donnees decompressees, independante de zlib) : le contenu
produit doit donc rester identique (graine fixe, PNG ecrits a la main, filtre 0, palettes
8 bits). `matrix_render.rs` refuse un PNG dont l'empreinte a change.

    python make_matrix.py DOSSIER [--quick]
"""
import argparse
import json
import os
import random
import struct
import sys
import zlib

SEED = 20260101

# RGB16 / RGBA16 : PNG 16 bits par canal. LT / RGBT / RGBT16 : tRNS de couleur cle (gris,
# RGB 8 bits, RGB 16 bits), distinct de PT (palette avec tRNS par entree). "1" : gris 1 bit.
MODES = ("L", "LA", "P", "PT", "RGB", "RGBA", "1", "I16", "RGB16", "RGBA16", "LT", "RGBT",
         "RGBT16")
ALPHA_MODES = ("RGBA", "LA", "PT", "RGBA16")
SIZES = (1, 3, 97, 641, 2049)
QUICK_SIZES = (1, 3, 97)
CONTENTS = ("gradient", "noise", "alpha")
NOISE_MAX = 641          # au-dela, le bruit (incompressible) rendrait les PNG enormes...
# ...sauf pour la plus grande taille, en carre et sur ces deux modes seulement : la source qui
# declenche la borne 2047 avec un contenu non compressible.
NOISE_BIG_MODES = ("RGBA", "L")
OPS = ("scene", "thumb", "inventory", "title", "bound")


def _rgba_content(content, w, h, rng):
    """Pixels RGBA (octets, ligne par ligne) : bruit a graine fixe opaque, degrade opaque, ou
    degrade a alpha partiel (diagonal). Construit par tranches d'octets, sans boucle par
    pixel (2049 x 2049)."""
    n = w * h
    if content == "noise":
        data = bytearray(rng.randbytes(4 * n))
        data[3::4] = b"\xff" * n
        return bytes(data)
    r_row = bytes(x * 255 // max(1, w - 1) for x in range(w))
    a_table = bytes(v * 255 // max(1, w + h - 2) for v in range(w + h - 1))
    mix = {}                       # gv -> table (rv + gv) // 2
    rows = []
    for y in range(h):
        gv = y * 255 // max(1, h - 1)
        if gv not in mix:
            mix[gv] = bytes((v + gv) // 2 for v in range(256))
        row = bytearray(4 * w)
        row[0::4] = r_row
        row[1::4] = bytes([gv]) * w
        row[2::4] = r_row.translate(mix[gv])
        row[3::4] = b"\xff" * w if content == "gradient" else a_table[y:y + w]
        rows.append(bytes(row))
    return b"".join(rows)


def _channels(rgba, *idx):
    """Canaux choisis (0 = R ... 3 = A), entrelaces dans cet ordre."""
    n = len(rgba) // 4
    out = bytearray(len(idx) * n)
    for k, i in enumerate(idx):
        out[k::len(idx)] = rgba[i::4]
    return bytes(out)


def _gray(rgba):
    """Niveau de gris : le canal B (melange (R + G) / 2 du degrade, aleatoire pour le bruit)."""
    return rgba[2::4]


def _palette_index(rgba, shifts):
    """Indice de palette par pixel : (R >> 6) << s0 | (G >> 6) << s1 | (B >> k) << s2, champs
    de bits disjoints additionnes en entiers longs (sans boucle par pixel)."""
    total = 0
    for i, (keep, shift) in enumerate(shifts):
        table = bytes(((v >> (8 - keep)) << shift) for v in range(256))
        total += int.from_bytes(rgba[i::4].translate(table), "big")
    n = len(rgba) // 4
    return total.to_bytes(n, "big")


_LOW = bytes((v * 7 + 3) % 256 for v in range(256))


def _wide16(v):
    """Echantillon 8 bits -> 16 bits : octet haut v, octet bas derive (pas v * 257, pour que
    le bit de poids faible compte)."""
    return (v << 8) | _LOW[v]


def _widen(data):
    """Octets 8 bits -> echantillons 16 bits gros-boutistes (voir _wide16)."""
    wide = bytearray(2 * len(data))
    wide[0::2] = data
    wide[1::2] = data.translate(_LOW)
    return bytes(wide)


def _bits(gray, w, h):
    """Gris 8 bits -> 1 bit par pixel (seuil 128), lignes completees a l'octet."""
    digits = gray.translate(bytes(0x31 if v >= 128 else 0x30 for v in range(256)))
    pad = (-w) % 8
    out = []
    for y in range(h):
        line = digits[y * w:(y + 1) * w] + b"0" * pad
        out.append(int(line, 2).to_bytes(len(line) // 8, "big") if line else b"")
    return b"".join(out)


def _chunk(kind, data):
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))


def _png(w, h, depth, color_type, data, plte=None, trns=None):
    """PNG ecrit a la main, filtre 0 sur chaque ligne, zlib niveau 6."""
    stride = len(data) // h
    raw = b"".join(b"\x00" + data[y * stride:(y + 1) * stride] for y in range(h))
    ihdr = struct.pack(">IIBBBBB", w, h, depth, color_type, 0, 0, 0)
    return (b"\x89PNG\r\n\x1a\n" + _chunk(b"IHDR", ihdr)
            + (_chunk(b"PLTE", plte) if plte is not None else b"")
            + (_chunk(b"tRNS", trns) if trns is not None else b"")
            + _chunk(b"IDAT", zlib.compress(raw, 6)) + _chunk(b"IEND", b""))


def _trns_key(rgba, w, h):
    """Cle tRNS RGB : le pixel le moins gris (ecart max - min des canaux le plus grand) parmi
    le centre et les quatre coins, premier en cas d'egalite. Une cle aux canaux distincts
    fait echouer une permutation de canaux dans le port."""
    spots = ((w // 2, h // 2), (w - 1, 0), (0, h - 1), (0, 0), (w - 1, h - 1))
    pixels = [tuple(rgba[4 * (y * w + x):4 * (y * w + x) + 3]) for x, y in spots]
    return max(pixels, key=lambda px: max(px) - min(px))


def matrix_png(mode, w, h, content, seed=SEED):
    """PNG d'une cellule de la matrice ; None si la combinaison n'a pas de sens (alpha partiel
    sur un mode sans canal alpha, bruit trop grand)."""
    if content == "alpha" and mode not in ALPHA_MODES:
        return None
    if content == "noise" and max(w, h) > NOISE_MAX and not (
            w == h == max(SIZES) and mode in NOISE_BIG_MODES):
        return None
    rng = random.Random("%s/%s/%d/%d/%s" % (seed, mode, w, h, content))
    rgba = _rgba_content(content, w, h, rng)
    if mode == "RGBA":
        return _png(w, h, 8, 6, rgba)
    if mode == "RGB":
        return _png(w, h, 8, 2, _channels(rgba, 0, 1, 2))
    if mode == "L":
        return _png(w, h, 8, 0, _gray(rgba))
    if mode == "LA":
        return _png(w, h, 8, 4, _channels(rgba, 2, 3))
    if mode == "1":
        return _png(w, h, 1, 0, _bits(_gray(rgba), w, h))
    if mode == "P":
        # 64 couleurs : 2 bits par canal
        plte = b"".join(bytes((r * 85, g * 85, b * 85))
                        for r in range(4) for g in range(4) for b in range(4))
        return _png(w, h, 8, 3, _palette_index(rgba, ((2, 4), (2, 2), (2, 0))), plte)
    if mode == "PT":
        # 32 couleurs (2 bits R, 2 bits G, 1 bit B), alpha par entree de palette
        plte = b"".join(bytes((r * 85, g * 85, b * 255))
                        for r in range(4) for g in range(4) for b in range(2))
        trns = bytes((i * 8) % 256 for i in range(32))
        return _png(w, h, 8, 3, _palette_index(rgba, ((2, 3), (2, 1), (1, 0))), plte, trns)
    if mode == "I16":
        return _png(w, h, 16, 0, _widen(_gray(rgba)))
    if mode == "RGB16":
        return _png(w, h, 16, 2, _widen(_channels(rgba, 0, 1, 2)))
    if mode == "RGBA16":
        return _png(w, h, 16, 6, _widen(rgba))
    if mode == "LT":
        g = _gray(rgba)
        key = g[(h // 2) * w + w // 2]
        return _png(w, h, 8, 0, g, trns=struct.pack(">H", key))
    if mode == "RGBT":
        return _png(w, h, 8, 2, _channels(rgba, 0, 1, 2),
                    trns=struct.pack(">HHH", *_trns_key(rgba, w, h)))
    if mode == "RGBT16":
        # Cle du meme pixel apres elargissement : Pillow lit le RGB 16 bits en 8 bits (octet
        # haut) mais compare la cle 16 bits telle quelle : la transparence ne s'applique
        # jamais des qu'un canal de la cle depasse 255 (comportement fige de l'oracle).
        return _png(w, h, 16, 2, _widen(_channels(rgba, 0, 1, 2)),
                    trns=struct.pack(">HHH", *(_wide16(v) for v in _trns_key(rgba, w, h))))
    raise ValueError(mode)


def png_fingerprint(data):
    """Empreinte d'un PNG de la matrice, independante de l'implementation de zlib (zlib,
    zlib-ng...) : SHA-256 des chunks autres que IDAT/IEND (type, longueur, donnees), puis de
    `IDAT` et des donnees IDAT decompressees. Meme calcul dans matrix_render.rs."""
    import hashlib
    h = hashlib.sha256()
    idat, pos = [], 8
    while pos + 8 <= len(data):
        n, kind = struct.unpack(">I4s", data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + n]
        if kind == b"IDAT":
            idat.append(body)
        elif kind != b"IEND":
            h.update(kind + struct.pack(">I", n) + body)
        pos += 12 + n
    h.update(b"IDAT" + zlib.decompress(b"".join(idat)))
    return h.hexdigest()


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
    """Ecrit les PNG dans out_dir/matrix et l'index out_dir/matrix.json ({id, png, op}, un
    par PNG et par operation). Retourne l'index."""
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
    with open(os.path.join(out_dir, "matrix.json"), "w", encoding="utf-8", newline="\n") as f:
        json.dump(entries, f, indent=1, ensure_ascii=True)
        f.write("\n")
    return entries


def main(argv=None):
    ap = argparse.ArgumentParser(description="Genere la matrice de reechantillonnage")
    ap.add_argument("out_dir", help="dossier de sortie (temporaire, jamais commite)")
    ap.add_argument("--quick", action="store_true", help="matrice reduite (petites tailles)")
    args = ap.parse_args(argv)
    os.makedirs(args.out_dir, exist_ok=True)
    print("matrice : %d cellules" % len(write_matrix(
        args.out_dir, QUICK_SIZES if args.quick else SIZES)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
