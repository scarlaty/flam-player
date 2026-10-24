"""Vecteurs de parité du décodage PNG (port Rust, issue #126).

L'oracle est `_open_rgba` de `tools/telmi2flam/telmi2flam.py`, exécuté avec **Python 3.14.8
et Pillow 12.3.0** exactement (versions de `apps/flam-launcher/scripts/build-converter.ps1`) :
le script refuse de tourner avec une autre version. Les PNG sont construits à la main
(filtres, Adam7, chunks altérés), sans contenu sous licence. Déterministe : relancer le
script ne doit rien changer (`git status` propre).

    python -I crates/telmi2flam/tests/vectors/png/gen_png_vectors.py
    python -I .../gen_png_vectors.py --matrix <dossier>   # matrice de make_corpus.py (#123)

Fichiers produits (lus par `tests/png_vectors.rs`) :
- `<nom>.png` et `manifest.tsv` : `nom  attendu  rust`, où `attendu` vaut
  `rgba:<largeur>x<hauteur>:<sha256 de tobytes()>` ou `unreadable` (exception levée par
  `_open_rgba`, donc avertissement `image_unreadable`), et `rust` vaut `same` (même résultat
  exigé) ou `delegate` (cas non modélisé : le Rust délègue à Python, code 7) ;
- `combos.tsv` : `nom  png  attendu`, PNG en hexadécimal (#459) : `tRNS` rangé sous un mode
  puis image décodée sous un autre (IHDR répété), `tRNS` répété ou après l'image, IHDR pas en
  tête ; le Rust doit donner le même résultat ;
- `alterations.tsv` : `base  opération  argument  attendu`, résultat de `_open_rgba` sur
  chaque troncature (`cut`, longueur) et chaque octet inversé (`xor`, position) des fichiers
  de base listés. Le Rust peut déléguer quelques cas (comptés par le test), jamais diverger.

Avec `--matrix <dossier>` : écrit dans `<dossier>` (hors dépôt) les PNG de la matrice de
rééchantillonnage de `tools/telmi2flam/tests/parity/make_corpus.py` (tailles 1, 3, 97, 641 ;
2049 seulement si `--big`) et un `manifest.tsv` au même format, lu par le test ignoré
`matrice_make_corpus` (`TELMI2FLAM_PNG_MATRIX=<dossier>`).
"""

import hashlib
import os
import random
import struct
import sys
import zlib

PYTHON = (3, 14, 8)
PILLOW = "12.3.0"

HERE = os.path.dirname(os.path.abspath(__file__))
TOOLS = os.path.join(HERE, "..", "..", "..", "..", "..", "tools", "telmi2flam")


def check_versions():
    import PIL
    if tuple(sys.version_info[:3]) != PYTHON or PIL.__version__ != PILLOW:
        sys.exit("oracle attendu : Python %s + Pillow %s (trouvé : Python %s, Pillow %s)"
                 % (".".join(map(str, PYTHON)), PILLOW, sys.version.split()[0], PIL.__version__))


def oracle(data):
    import telmi2flam
    try:
        im = telmi2flam._open_rgba(data)
        raw = im.tobytes()
    except Exception:
        return "unreadable"
    assert im.mode == "RGBA"
    return "rgba:%dx%d:%s" % (im.size[0], im.size[1], hashlib.sha256(raw).hexdigest())


# ---------------------------------------------------------------------------
# Construction de PNG
# ---------------------------------------------------------------------------
SIG = b"\x89PNG\r\n\x1a\n"


def chunk(kind, data, crc=None):
    if crc is None:
        crc = zlib.crc32(kind + data)
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", crc & 0xFFFFFFFF)


def ihdr(w, h, bd, ct, interlace=0, comp=0, filt=0):
    return chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, bd, ct, comp, filt, interlace))


CHANNELS = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}
ADAM7 = ((0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2),
         (0, 1, 1, 2))


def pack_row(samples, bd):
    """Échantillons (entiers) -> octets d'une ligne PNG, bits de poids fort d'abord."""
    if bd == 8:
        return bytes(samples)
    if bd == 16:
        return b"".join(struct.pack(">H", s) for s in samples)
    out = bytearray()
    acc = nbits = 0
    for s in samples:
        acc = (acc << bd) | s
        nbits += bd
        if nbits == 8:
            out.append(acc)
            acc = nbits = 0
    if nbits:
        out.append(acc << (8 - nbits))
    return bytes(out)


def paeth(a, b, c):
    p = a + b - c
    pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
    if pa <= pb and pa <= pc:
        return a
    return b if pb <= pc else c


def filter_rows(rows, bpp, first_filter):
    """Applique les filtres 0 à 4 en alternance (à partir de `first_filter`)."""
    out = bytearray()
    prev = bytes(len(rows[0])) if rows else b""
    for i, row in enumerate(rows):
        f = (first_filter + i) % 5
        enc = bytearray()
        for x, v in enumerate(row):
            a = row[x - bpp] if x >= bpp else 0
            b = prev[x]
            c = prev[x - bpp] if x >= bpp else 0
            pred = (0, a, b, (a + b) // 2, paeth(a, b, c))[f]
            enc.append((v - pred) & 0xFF)
        out.append(f)
        out += enc
        prev = row
    return bytes(out)


def scanlines(w, h, bd, ct, pixels, interlace, first_filter=0):
    """Données décompressées (filtrées) ; `pixels[y][x]` = tuple d'échantillons."""
    bpp = max(1, CHANNELS[ct] * bd // 8)
    passes = ADAM7 if interlace else ((0, 0, 1, 1),)
    out = b""
    for (x0, y0, dx, dy) in passes:
        rows = []
        for y in range(y0, h, dy):
            samples = [s for x in range(x0, w, dx) for s in pixels[y][x]]
            if samples:
                rows.append(pack_row(samples, bd))
        if rows:
            out += filter_rows(rows, bpp, first_filter)
    return out


def make_png(w, h, bd, ct, pixels, interlace=0, before=(), after=(), idat_split=None,
             level=6, first_filter=0, trailer=(chunk(b"IEND", b""),)):
    data = zlib.compress(scanlines(w, h, bd, ct, pixels, interlace, first_filter), level)
    idats = []
    if idat_split:
        pos = 0
        for n in idat_split:
            idats.append(chunk(b"IDAT", data[pos:pos + n]))
            pos += n
        idats.append(chunk(b"IDAT", data[pos:]))
    else:
        idats.append(chunk(b"IDAT", data))
    return (SIG + ihdr(w, h, bd, ct, interlace) + b"".join(before) + b"".join(idats)
            + b"".join(after) + b"".join(trailer))


def rand_pixels(rng, w, h, bd, ct, maxval=None):
    top = (1 << bd) - 1 if maxval is None else maxval
    return [[tuple(rng.randint(0, top) for _ in range(CHANNELS[ct])) for _ in range(w)]
            for _ in range(h)]


def palette(n, rng):
    return bytes(rng.randrange(256) for _ in range(3 * n))


# ---------------------------------------------------------------------------
# Cas
# ---------------------------------------------------------------------------
MODES = ((1, 0), (2, 0), (4, 0), (8, 0), (16, 0), (8, 2), (16, 2), (1, 3), (2, 3), (4, 3),
         (8, 3), (8, 4), (16, 4), (8, 6), (16, 6))
SIZES = ((1, 1), (3, 2), (7, 5), (17, 9), (33, 3))


def actl(n, loops=0):
    return chunk(b"acTL", struct.pack(">II", n, loops))


def fctl(seq, w, h, x=0, y=0, dispose=0, blend=0):
    return chunk(b"fcTL", struct.pack(">IIIIIHHBB", seq, w, h, x, y, 1, 10, dispose, blend))


def fdat(seq, data):
    return chunk(b"fdAT", struct.pack(">I", seq) + data)


def zdata(w, h, bd, ct, pixels, interlace=0):
    return zlib.compress(scanlines(w, h, bd, ct, pixels, interlace))


def apng_cases(add, _rng, base_px, good):
    """APNG (#459) : Pillow ouvre la première image ; zone `fcTL`, séquences, `fdAT`."""
    rng = random.Random(4591)
    head = SIG + ihdr(9, 7, 8, 2)
    z = zdata(9, 7, 8, 2, base_px)
    sub = rand_pixels(rng, 4, 3, 8, 2)
    zsub = zdata(4, 3, 8, 2, sub)
    end = chunk(b"IEND", b"")
    add("apng_deux_images", head + actl(2) + fctl(0, 9, 7) + chunk(b"IDAT", z)
        + fctl(1, 4, 3, 2, 2) + fdat(2, zsub) + end)
    add("apng_defaut_hors_animation", head + actl(1) + chunk(b"IDAT", z) + fctl(0, 4, 3)
        + fdat(1, zsub) + end)
    add("apng_defaut_hors_animation_2", head + actl(2) + chunk(b"IDAT", z) + fctl(0, 4, 3)
        + fdat(1, zsub) + fctl(2, 9, 7) + fdat(3, z) + end)
    add("apng_fdat_premier", head + actl(1) + fctl(0, 9, 7) + fdat(1, z) + end)
    add("apng_fdat_premier_partiel", head + actl(1) + fctl(0, 4, 3, 5, 4) + fdat(1, zsub) + end)
    add("apng_fdat_sans_fctl", head + actl(1) + fdat(0, z) + end)
    add("apng_fdat_seq_faux", head + actl(1) + fctl(0, 9, 7) + fdat(2, z) + end)
    add("apng_fdat_court", head + actl(1) + fctl(0, 9, 7) + chunk(b"fdAT", b"\x00\x00\x01")
        + chunk(b"IDAT", z) + end)
    add("apng_fctl_seq_1", head + actl(1) + fctl(1, 9, 7) + chunk(b"IDAT", z) + end)
    add("apng_fctl_court", head + actl(1) + chunk(b"fcTL", bytes(25)) + chunk(b"IDAT", z) + end)
    add("apng_fctl_crc_faux", head + actl(1) + fctl(0, 9, 7)[:-1] + b"\x00"
        + chunk(b"IDAT", z) + end)
    add("apng_fctl_hors_image", head + actl(1) + fctl(0, 9, 7, 1, 0) + chunk(b"IDAT", z) + end)
    add("apng_fctl_vide", head + actl(1) + fctl(0, 0, 7) + chunk(b"IDAT", z) + end)
    add("apng_fctl_hauteur_nulle", head + actl(1) + fctl(0, 9, 0) + chunk(b"IDAT", z) + end)
    add("apng_fctl_avant_ihdr", SIG + fctl(0, 0, 0) + ihdr(9, 7, 8, 2) + chunk(b"IDAT", z) + end)
    add("apng_fctl_sans_actl", head + fctl(0, 9, 7) + chunk(b"IDAT", z) + end)
    add("apng_actl_court", head + chunk(b"acTL", b"\x00\x00\x00\x01") + chunk(b"IDAT", z) + end)
    add("apng_actl_double", head + actl(2) + actl(2) + fctl(0, 9, 7) + chunk(b"IDAT", z)
        + fctl(5, 0, 0) + end)
    add("apng_actl_zero", head + actl(0) + fctl(0, 9, 7) + chunk(b"IDAT", z) + fctl(9, 1, 1)
        + end)
    add("apng_actl_trop", head + actl(0x80000001) + chunk(b"IDAT", z) + end)
    add("apng_actl_max", head + actl(0x80000000) + chunk(b"IDAT", z) + fctl(0, 1, 1) + end)
    # zone partielle : le reste garde le pixel nul du mode (palette : entrée 0, tRNS…)
    for name, bd, ct, extra in (("gris", 8, 0, [chunk(b"tRNS", b"\x00\x00")]),
                                ("palette", 4, 3, [chunk(b"PLTE", palette(16, rng)),
                                                   chunk(b"tRNS", b"\x40\x80")]),
                                ("rgba16", 16, 6, []), ("rgb16", 16, 2, []),
                                ("gris1", 1, 0, [chunk(b"tRNS", b"\x00\x01")])):
        for il in (0, 1):
            px = rand_pixels(rng, 5, 3, bd, ct)
            add("apng_partiel_%s_i%d" % (name, il),
                SIG + ihdr(11, 6, bd, ct, il) + b"".join(extra) + actl(1)
                + fctl(0, 5, 3, 4, 2) + chunk(b"IDAT", zdata(5, 3, bd, ct, px, il)) + end)
    # suite des données en fdAT
    add("apng_idat_puis_fdat", head + actl(1) + fctl(0, 9, 7) + chunk(b"IDAT", z[:20])
        + fdat(1, z[20:]) + end)
    add("apng_idat_puis_fdat_seq_faux", head + actl(1) + fctl(0, 9, 7) + chunk(b"IDAT", z[:20])
        + fdat(3, z[20:]) + end)
    add("apng_idat_puis_fdat_court", head + actl(1) + fctl(0, 9, 7) + chunk(b"IDAT", z[:20])
        + chunk(b"fdAT", b"\x00\x00") + end)
    add("apng_idat_puis_fdat_vide", head + actl(1) + fctl(0, 9, 7) + chunk(b"IDAT", z[:20])
        + fdat(1, b"") + fdat(2, z[20:]) + end)
    add("apng_idat_puis_fctl", head + actl(1) + fctl(0, 9, 7) + chunk(b"IDAT", z[:20])
        + fctl(1, 9, 7) + end)
    # après l'image : animé (arrêt au fcTL suivant) ou non (chunks vérifiés)
    tail_bad = fctl(1, 99, 7) + fdat(2, zsub) + end
    add("apng_anime_fctl_suivant_faux", head + actl(2) + fctl(0, 9, 7) + chunk(b"IDAT", z)
        + tail_bad)
    add("apng_fixe_fctl_suivant_faux", head + actl(1) + fctl(0, 9, 7) + chunk(b"IDAT", z)
        + tail_bad)
    add("apng_fixe_suite_valide", head + actl(1) + fctl(0, 9, 7) + chunk(b"IDAT", z)
        + fctl(1, 4, 3) + fdat(2, zsub) + end)
    add("apng_fixe_fdat_seq_faux", head + actl(1) + fctl(0, 9, 7) + chunk(b"IDAT", z)
        + fdat(7, zsub) + end)
    add("apng_fixe_fdat_tronque", head + actl(1) + fctl(0, 9, 7) + chunk(b"IDAT", z)
        + struct.pack(">I", 50) + b"fdAT" + struct.pack(">I", 1) + b"xx")
    add("apng_fixe_fdat_court", head + actl(1) + fctl(0, 9, 7) + chunk(b"IDAT", z)
        + chunk(b"fdAT", b"\x01") + end)
    add("apng_sans_apng_fdat_apres", good[:-12] + fdat(1, zsub) + end)
    add("apng_actl_apres_court", good[:-12] + chunk(b"acTL", b"\x00") + end)
    add("apng_actl_apres_ok", good[:-12] + actl(3) + end)
    add("apng_fctl_apres_court", good[:-12] + chunk(b"fcTL", bytes(10)) + end)
    add("apng_anime_trns_apres", SIG + ihdr(5, 3, 8, 0) + actl(2) + fctl(0, 5, 3)
        + chunk(b"IDAT", zdata(5, 3, 8, 0, [[((x + y) % 4,) for x in range(5)] for y in range(3)]))
        + fctl(1, 5, 3) + chunk(b"tRNS", b"\x00\x01") + fdat(2, b"") + end)
    add("apng_fixe_trns_apres", SIG + ihdr(5, 3, 8, 0) + actl(1) + fctl(0, 5, 3)
        + chunk(b"IDAT", zdata(5, 3, 8, 0, [[((x + y) % 4,) for x in range(5)] for y in range(3)]))
        + chunk(b"tRNS", b"\x00\x01") + fctl(1, 5, 3) + end)
    # IHDR réduit après le fcTL : zone hors de l'image au chargement
    add("apng_ihdr_reduit_apres_fctl", head + actl(1) + fctl(0, 9, 7) + ihdr(5, 3, 8, 2)
        + chunk(b"IDAT", zdata(5, 3, 8, 2, [r[:5] for r in base_px[:3]])) + end)
    add("apng_ihdr_agrandi_apres_fctl", head + actl(1) + fctl(0, 9, 7) + ihdr(12, 8, 8, 2)
        + chunk(b"IDAT", z) + end)


def ihdr_cases(add, _rng):
    """IHDR absent en tête, répété, changeant de taille ou de mode (#459)."""
    rng = random.Random(4592)
    end = chunk(b"IEND", b"")
    px = rand_pixels(rng, 6, 4, 8, 2)
    z = zdata(6, 4, 8, 2, px)
    add("ihdr_double_identique", SIG + ihdr(6, 4, 8, 2) + ihdr(6, 4, 8, 2) + chunk(b"IDAT", z)
        + end)
    add("ihdr_double_taille", SIG + ihdr(9, 9, 8, 2) + ihdr(6, 4, 8, 2) + chunk(b"IDAT", z) + end)
    add("ihdr_double_mode_inconnu", SIG + ihdr(6, 4, 8, 2) + ihdr(6, 4, 3, 0) + chunk(b"IDAT", z)
        + end)
    add("ihdr_double_taille_mode_inconnu", SIG + ihdr(6, 4, 8, 2) + ihdr(3, 2, 3, 0)
        + chunk(b"IDAT", z) + end)
    add("ihdr_double_entrelace_collant", SIG + ihdr(6, 4, 8, 2, 1) + ihdr(6, 4, 8, 2)
        + chunk(b"IDAT", zdata(6, 4, 8, 2, px, 1)) + end)
    add("ihdr_double_filtre", SIG + ihdr(6, 4, 8, 2) + ihdr(6, 4, 8, 2, filt=1)
        + chunk(b"IDAT", z) + end)
    add("ihdr_double_court", SIG + ihdr(6, 4, 8, 2) + chunk(b"IHDR", b"\x00" * 12)
        + chunk(b"IDAT", z) + end)
    add("ihdr_premier_invalide", SIG + ihdr(6, 4, 3, 0) + ihdr(6, 4, 8, 2) + chunk(b"IDAT", z)
        + end)
    add("ihdr_apres_texte_et_plte", SIG + chunk(b"tEXt", b"a\x00b") + chunk(b"PLTE", bytes(12))
        + ihdr(6, 4, 8, 2) + chunk(b"IDAT", z) + end)
    pxp = [[((x + y) % 4,) for x in range(6)] for y in range(4)]
    zp = zdata(6, 4, 8, 3, pxp)
    add("plte_avant_ihdr", SIG + chunk(b"PLTE", palette(4, rng)) + ihdr(6, 4, 8, 3)
        + chunk(b"IDAT", zp) + end)
    add("plte_sous_rgb_puis_palette", SIG + ihdr(6, 4, 8, 2) + chunk(b"PLTE", palette(4, rng))
        + ihdr(6, 4, 8, 3) + chunk(b"IDAT", zp) + end)
    add("plte_palette_puis_rgb", SIG + ihdr(6, 4, 8, 3) + chunk(b"PLTE", palette(4, rng))
        + ihdr(6, 4, 8, 2) + chunk(b"IDAT", z) + end)
    add("trns_avant_ihdr", SIG + chunk(b"tRNS", b"\x00\x01") + ihdr(6, 4, 8, 0)
        + chunk(b"IDAT", zdata(6, 4, 8, 0, pxp)) + end)
    add("ihdr_apres_idat_autre_mode", SIG + ihdr(6, 4, 8, 2) + chunk(b"IDAT", z)
        + ihdr(6, 4, 8, 0) + chunk(b"tRNS", b"\x00\x01") + end)
    add("ihdr_apres_idat_court", SIG + ihdr(6, 4, 8, 2) + chunk(b"IDAT", z)
        + chunk(b"IHDR", b"\x00") + end)
    add("ihdr_apres_idat_puis_fctl", SIG + ihdr(6, 4, 8, 2) + chunk(b"IDAT", z)
        + ihdr(2, 2, 8, 2) + fctl(0, 3, 3) + end)


# Combinaisons de transparence (#459) : `tRNS` rangé sous un mode, image décodée sous un autre
# (IHDR répété), clé brute de `_png_trns_key` et masque de `_trns_mask` (premier IHDR).
TRNS_SET_MODES = MODES
TRNS_CROSS_SET = ((1, 0), (2, 0), (8, 0), (16, 0), (8, 2), (16, 2), (4, 3), (8, 4), (16, 6))
TRNS_CROSS_EFF = ((1, 0), (4, 0), (8, 0), (16, 0), (8, 2), (16, 2), (8, 3), (8, 4))
TRNS_PAYLOADS = (b"\x00\x01", b"\x01\x02", b"\x00\x00\x00\x01\x00\x02", b"\x00\x03" * 2,
                 b"\x00\x01\x00\x02\x00\x03\x00\x04", b"\x00", b"", b"\xff\x00",
                 b"\x00\xff\xff")


def combo_cases():
    out = []
    end = chunk(b"IEND", b"")
    w, h = 4, 2

    def body(bd, ct):
        top = min((1 << bd) - 1, 5) if ct != 3 else 4
        if bd == 16:
            px = [[tuple(((x + y + c) % 4) * 0x0101 + (x % 2) for c in range(CHANNELS[ct]))
                   for x in range(w)] for y in range(h)]
        else:
            px = [[tuple((x + y + c) % (top + 1) for c in range(CHANNELS[ct]))
                   for x in range(w)] for y in range(h)]
        plte = [chunk(b"PLTE", bytes((i * 37 + c * 11) % 256 for i in range(6)
                                     for c in range(3)))] if ct == 3 else []
        return plte, zdata(w, h, bd, ct, px)

    pairs = [(s, s) for s in TRNS_SET_MODES]
    pairs += [(s, e) for s in TRNS_CROSS_SET for e in TRNS_CROSS_EFF if s != e]
    for (sb, sc), (eb, ec) in pairs:
        same = (sb, sc) == (eb, ec)
        plte, zd = body(eb, ec)
        first = ihdr(w, h, sb, sc)
        second = b"" if same else ihdr(w, h, eb, ec)
        image = b"".join(plte) + chunk(b"IDAT", zd)
        for pi, payload in enumerate(TRNS_PAYLOADS):
            if not same and pi not in (0, 1, 2, 7):
                continue
            trns = chunk(b"tRNS", payload)
            name = "c_%d_%d_p%d_%d_%d" % (sb, sc, pi, eb, ec)
            out.append((name, SIG + first + trns + second + image + end))
            if pi in (0, 2):
                # tRNS après l'image (lu par load_end, absent de _png_trns_key)
                out.append((name + "_apres", SIG + first + second + image + trns + end))
            if pi in (0, 2) and same:
                # IHDR pas en tête : pas de masque de _trns_mask
                out.append((name + "_texte", SIG + chunk(b"tEXt", b"a\x00b") + first + trns
                            + image + end))
            if pi == 1 and same:
                out.append((name + "_double", SIG + first + chunk(b"tRNS", b"\x00\x00") + trns
                            + image + chunk(b"tRNS", TRNS_PAYLOADS[2]) + end))
    return out


def cases():
    rng = random.Random(126)
    out = []

    def add(name, data, rust="same"):
        out.append((name, data, rust))

    # --- tous les modes de Pillow, tailles, Adam7, filtres
    for bd, ct in MODES:
        for (w, h) in SIZES:
            for il in (0, 1):
                px = rand_pixels(rng, w, h, bd, ct)
                before = [chunk(b"PLTE", palette(1 << bd, rng))] if ct == 3 else []
                add("mode_%d_%d_%dx%d_i%d" % (bd, ct, w, h, il),
                    make_png(w, h, bd, ct, px, il, before, first_filter=w % 5))

    # --- tRNS (clés brutes, valeurs non présentes, octet bas/haut)
    def gray_trns(bd, key, name, il=0, w=9, h=4):
        top = (1 << bd) - 1
        px = [[((x * 7 + y * 3) % (top + 1),) for x in range(w)] for y in range(h)]
        add(name, make_png(w, h, bd, 0, px, il, [chunk(b"tRNS", struct.pack(">H", key))]))

    for bd in (1, 2, 4, 8, 16):
        top = (1 << bd) - 1
        gray_trns(bd, top // 3 if bd > 1 else 1, "trns_gris%d" % bd)
        gray_trns(bd, 0, "trns_gris%d_zero" % bd, il=1)
        gray_trns(bd, 0xFFFF if bd < 16 else 0x1234, "trns_gris%d_hors" % bd)
    gray_trns(8, 0x0105, "trns_gris8_octet_haut")    # clé 16 bits : seul l'octet bas compte ?
    gray_trns(1, 2, "trns_gris1_cle2")
    gray_trns(2, 0x100 + 2, "trns_gris2_cle_haute")
    for bd in (8, 16):
        w, h = 6, 5
        px = rand_pixels(rng, w, h, bd, 2, maxval=3 if bd == 8 else None)
        key = px[2][3]
        add("trns_rgb%d" % bd, make_png(w, h, bd, 2, px, 0,
                                        [chunk(b"tRNS", struct.pack(">HHH", *key))]))
        if bd == 16:
            # même octet haut, octet bas différent : Pillow ne compare que l'octet haut
            k2 = tuple((v & 0xFF00) | ((v + 1) & 0xFF) for v in key)
            add("trns_rgb16_octet_bas", make_png(w, h, bd, 2, px, 1,
                                                 [chunk(b"tRNS", struct.pack(">HHH", *k2))]))
        else:
            add("trns_rgb8_cle_haute", make_png(w, h, bd, 2, px, 0, [chunk(
                b"tRNS", struct.pack(">HHH", *(0x100 | v for v in key)))]))
    # tRNS de longueur inattendue
    px8 = [[((x + y) % 4,) for x in range(5)] for y in range(3)]
    add("trns_gris_court", make_png(5, 3, 8, 0, px8, 0, [chunk(b"tRNS", b"\x01")]))
    add("trns_gris_long", make_png(5, 3, 8, 0, px8, 0, [chunk(b"tRNS", b"\x00\x01\x00\x02")]))
    add("trns_gris1_long", make_png(5, 3, 1, 0, [[(x % 2,) for x in range(5)]] * 3, 0,
                                    [chunk(b"tRNS", b"\x00\x01\x00\x02\x00\x03")]))
    pxrgb = rand_pixels(rng, 4, 3, 8, 2, maxval=2)
    add("trns_rgb_court", make_png(4, 3, 8, 2, pxrgb, 0, [chunk(b"tRNS", b"\x00\x01\x00")]))
    add("trns_rgb_long", make_png(4, 3, 8, 2, pxrgb, 0, [chunk(b"tRNS", bytes(8))]))
    add("trns_vide_gris", make_png(5, 3, 8, 0, px8, 0, [chunk(b"tRNS", b"")]))
    add("trns_sur_rgba", make_png(4, 3, 8, 6, rand_pixels(rng, 4, 3, 8, 6), 0,
                                  [chunk(b"tRNS", bytes(6))]))
    add("trns_sur_la", make_png(4, 3, 8, 4, rand_pixels(rng, 4, 3, 8, 4), 0,
                                [chunk(b"tRNS", bytes(2))]))
    add("trns_double", make_png(5, 3, 8, 0, px8, 0, [chunk(b"tRNS", b"\x00\x01"),
                                                     chunk(b"tRNS", b"\x00\x02")]))
    add("trns_apres_idat", make_png(5, 3, 8, 0, px8, 0, after=[chunk(b"tRNS", b"\x00\x01")]))

    # --- palettes
    def pal_png(name, nbits, n_pal, trns=None, idx_max=None, il=0, plte_raw=None,
                rust="same", w=11, h=6, before_ihdr=False):
        top = (1 << nbits) - 1 if idx_max is None else idx_max
        px = [[((x * 5 + y * 3) % (top + 1),) for x in range(w)] for y in range(h)]
        before = []
        if plte_raw is not None:
            before.append(chunk(b"PLTE", plte_raw))
        elif n_pal is not None:
            before.append(chunk(b"PLTE", palette(n_pal, rng)))
        if trns is not None:
            before.append(chunk(b"tRNS", trns))
        add(name, make_png(w, h, nbits, 3, px, il, before), rust)

    pal_png("pal8_complete", 8, 256, idx_max=255, w=32, h=8)
    pal_png("pal8_incomplete", 8, 10, idx_max=40)          # indices hors palette
    pal_png("pal4_incomplete", 4, 3)
    pal_png("pal_absente", 8, None, idx_max=255, w=32, h=8)
    pal_png("pal_vide", 8, None, plte_raw=b"", idx_max=20)
    pal_png("pal_non_multiple_3", 8, None, plte_raw=bytes(range(20)), idx_max=10)
    pal_png("pal_trop_longue", 8, None, plte_raw=bytes(range(256)) * 4, idx_max=255, w=32)
    pal_png("pal_trns_simple", 8, 16, trns=b"\xff\xff\x00\xff", idx_max=15)
    pal_png("pal_trns_simple_seul", 8, 16, trns=b"\x00", idx_max=15)
    pal_png("pal_trns_alpha", 8, 16, trns=bytes((i * 17) % 256 for i in range(16)), idx_max=15)
    pal_png("pal_trns_court", 4, 16, trns=b"\x10\x80", il=1)
    pal_png("pal_trns_hors_palette", 8, 4, trns=bytes(range(0, 200, 10)), idx_max=30)
    pal_png("pal_trns_simple_hors", 8, 4, trns=b"\xff" * 9 + b"\x00", idx_max=12)
    pal_png("pal_trns_300", 8, 256, trns=bytes(i % 251 for i in range(300)), idx_max=255, w=32)
    pal_png("pal_trns_simple_300", 8, 256, trns=b"\xff" * 280 + b"\x00", idx_max=255, w=32)
    pal_png("pal_trns_vide", 8, 16, trns=b"", idx_max=15)
    pal_png("pal_770", 8, None, plte_raw=bytes(i % 256 for i in range(770)), idx_max=255, w=32)
    for n in (1, 2, 4):
        pal_png("pal_plte_%d" % n, 8, None, plte_raw=bytes(range(10, 10 + n)), idx_max=3)
    pal_png("pal_771",8, None, plte_raw=bytes(i % 256 for i in range(771)), idx_max=255, w=32)
    pal_png("pal_trns_256", 8, 256, trns=bytes(i % 7 for i in range(256)), idx_max=255, w=32)
    pal_png("pal_trns_257", 8, 256, trns=bytes(i % 7 for i in range(257)), idx_max=255, w=32)
    pal_png("pal_trns_simple_nl", 8, 16, trns=b"\xff\x00\n", idx_max=15)
    pal_png("pal_trns_ff_seuls", 8, 16, trns=b"\xff" * 300, idx_max=15)
    pxp = [[((x + y) % 4,) for x in range(5)] for y in range(3)]
    add("pal_plte_apres_idat", make_png(5, 3, 8, 3, pxp, 0, [chunk(b"PLTE", palette(4, rng))],
                                        after=[chunk(b"PLTE", palette(4, rng))]))
    pal_png("pal_trns_ff", 8, 16, trns=b"\xff\xff\xff", idx_max=15)
    pal_png("pal_trns_deux_zeros", 8, 16, trns=b"\x00\xff\x00", idx_max=15)

    # --- chunks : CRC, ordre, ancillaires
    base_px = rand_pixels(rng, 9, 7, 8, 2)

    def rgb(name, rust="same", **kw):
        add(name, make_png(9, 7, 8, 2, base_px, **kw), rust)

    rgb("rgb_reference")
    good = make_png(9, 7, 8, 2, base_px)
    add("crc_ihdr_faux", good[:29] + bytes([good[29] ^ 1]) + good[30:])
    rgb("crc_ancillaire_faux", before=[chunk(b"tEXt", b"k\x00v", crc=1)])
    rgb("crc_inconnu_faux", before=[chunk(b"abCd", b"xyz", crc=1)])
    idat_bad = bytearray(good)
    p = idat_bad.index(b"IEND") - 8          # dernier octet du CRC de l'IDAT
    idat_bad[p] ^= 0xFF
    add("crc_idat_faux", bytes(idat_bad))
    rgb("crc_apres_idat_faux", after=[chunk(b"tEXt", b"k\x00v", crc=1)])
    rgb("sans_iend", trailer=())
    rgb("dechets_apres_iend", trailer=(chunk(b"IEND", b""), b"garbage" * 3))
    rgb("dechets_apres_idat", trailer=(b"\x00\x01",))
    rgb("idat_morcele", idat_split=[1, 0, 3, 0, 7])
    rgb("idat_morcele_entrelace", idat_split=[2, 5, 1], interlace=1)
    add("sans_idat", SIG + ihdr(9, 7, 8, 2) + chunk(b"IEND", b""))
    add("iend_avant_idat", SIG + ihdr(9, 7, 8, 2) + chunk(b"IEND", b"")
        + good[33:])
    add("sans_ihdr", SIG + good[33:])
    add("ihdr_pas_premier", SIG + chunk(b"tEXt", b"a\x00b") + good[8:])
    add("ihdr_double", good[:33] + ihdr(9, 7, 8, 0) + good[33:])
    add("ihdr_court", SIG + chunk(b"IHDR", struct.pack(">IIBB", 9, 7, 8, 2)) + good[33:])
    add("ihdr_long", SIG + chunk(b"IHDR", struct.pack(">IIBBBBB", 9, 7, 8, 2, 0, 0, 0) + b"xy")
        + good[33:])
    add("ihdr_compression_1", SIG + ihdr(9, 7, 8, 2, comp=1) + good[33:])
    add("ihdr_filtre_1", SIG + ihdr(9, 7, 8, 2, filt=1) + good[33:])
    zdata = zlib.compress(scanlines(9, 7, 8, 2, base_px, 1))
    add("ihdr_entrelacement_2", SIG + ihdr(9, 7, 8, 2, interlace=2) + chunk(b"IDAT", zdata)
        + chunk(b"IEND", b""))
    for bd, ct in ((16, 3), (4, 2), (2, 4), (3, 0), (8, 1), (8, 5), (32, 6)):
        add("mode_invalide_%d_%d" % (bd, ct), SIG + ihdr(9, 7, bd, ct) + good[33:])
    add("largeur_nulle", SIG + ihdr(0, 7, 8, 2) + good[33:])
    add("hauteur_nulle", SIG + ihdr(9, 0, 8, 2) + good[33:])
    add("bombe_2x", SIG + ihdr(178956971, 1, 1, 0) + good[33:])
    add("bombe_geante", SIG + ihdr(0xFFFFFFFF, 0xFFFFFFFF, 8, 6) + good[33:])
    add("dimensions_hors_borne", SIG + ihdr(0x80000000, 1, 1, 0) + good[33:])
    add("bombe_juste_dessous", SIG + ihdr(178956970, 1, 1, 0) + good[33:])   # données absentes
    add("nom_chunk_invalide", SIG + ihdr(9, 7, 8, 2) + chunk(b"a-cd", b"") + good[33:])
    rgb("nom_chunk_invalide_apres", after=[chunk(b"a-cd", b"xx")])
    rgb("chunk_coupe_apres", trailer=(struct.pack(">I", 100) + b"abCd" + b"xx",))
    rgb("idat_coupe_apres", trailer=(struct.pack(">I", 100) + b"IDAT" + b"xx",))
    rgb("ihdr_court_apres", after=[chunk(b"IHDR", b"abc")])
    rgb("chunk_entre_idat", idat_split=[10], rust="same")
    gi = good.index(b"IDAT") - 4
    gl = struct.unpack(">I", good[gi:gi + 4])[0]
    idat_chunk = good[gi:gi + 12 + gl]
    zd = idat_chunk[8:-4]
    add("chunk_entre_idat_vrai", good[:gi] + chunk(b"IDAT", zd[:10]) + chunk(b"tEXt", b"a\x00b")
        + chunk(b"IDAT", zd[10:]) + chunk(b"IEND", b""))
    add("adler_faux", good[:gi] + chunk(b"IDAT", zd[:-1] + bytes([zd[-1] ^ 1]))
        + chunk(b"IEND", b""))
    add("sans_adler", good[:gi] + chunk(b"IDAT", zd[:-4]) + chunk(b"IEND", b""))
    add("zlib_tronque", good[:gi] + chunk(b"IDAT", zd[:len(zd) // 2]) + chunk(b"IEND", b""))
    add("zlib_entete_faux", good[:gi] + chunk(b"IDAT", b"\x78\x00" + zd[2:])
        + chunk(b"IEND", b""))
    add("zlib_dictionnaire", good[:gi] + chunk(b"IDAT", b"\x78\x20" + b"\x00\x00\x00\x01"
                                               + zd[2:]) + chunk(b"IEND", b""))
    raw = scanlines(9, 7, 8, 2, base_px, 0)
    add("donnees_en_trop", good[:gi] + chunk(b"IDAT", zlib.compress(raw + b"\x00" * 50))
        + chunk(b"IEND", b""))
    add("donnees_manquantes", good[:gi] + chunk(b"IDAT", zlib.compress(raw[:-5]))
        + chunk(b"IEND", b""))
    add("dechets_apres_zlib", good[:gi] + chunk(b"IDAT", zd + b"garbage") + chunk(b"IEND", b""))
    bad_filter = bytearray(raw)
    bad_filter[1 + 27] = 5                                # filtre de la 2e ligne
    add("filtre_5", good[:gi] + chunk(b"IDAT", zlib.compress(bytes(bad_filter)))
        + chunk(b"IEND", b""))
    bad_filter[1 + 27] = 0xFF
    add("filtre_255", good[:gi] + chunk(b"IDAT", zlib.compress(bytes(bad_filter)))
        + chunk(b"IEND", b""))
    add("zlib_stocke", good[:gi] + chunk(b"IDAT", zlib.compress(raw, 0)) + chunk(b"IEND", b""))

    # ancillaires connus de Pillow, valides ou malformés, avant et après l'IDAT
    big = zlib.compress(b"\x00" * (1024 * 1024 + 1))
    exact = zlib.compress(b"\x00" * (1024 * 1024))
    anc = {
        "gama_court": b"gAMA" + b"\x00\x01",
        "gama_ok": b"gAMA" + struct.pack(">I", 45455),
        "chrm_impair": b"cHRM" + b"\x00" * 5,
        "srgb_vide": b"sRGB",
        "srgb_ok": b"sRGB" + b"\x00",
        "phys_court": b"pHYs" + b"\x00" * 8,
        "phys_ok": b"pHYs" + b"\x00\x00\x0b\x13\x00\x00\x0b\x13\x01",
        "text_ok": b"tEXt" + b"Title\x00Bonjour \xe9t\xe9",
        "text_sans_nul": b"tEXt" + b"abc",
        "ztxt_ok": b"zTXt" + b"k\x00\x00" + zlib.compress(b"valeur"),
        "ztxt_methode": b"zTXt" + b"k\x00\x01" + zlib.compress(b"valeur"),
        "ztxt_corrompu": b"zTXt" + b"k\x00\x00" + b"\x78\x9cgarbage",
        "ztxt_tronque": b"zTXt" + b"k\x00\x00" + zlib.compress(b"valeur" * 50)[:10],
        "ztxt_trop_grand": b"zTXt" + b"k\x00\x00" + big,
        "ztxt_1mio": b"zTXt" + b"k\x00\x00" + exact,
        "ztxt_vide": b"zTXt",
        "itxt_ok": b"iTXt" + b"k\x00\x00\x00fr\x00cl\x00val\xc3\xa9",
        "itxt_compresse": b"iTXt" + b"k\x00\x01\x00fr\x00cl\x00" + zlib.compress(b"v"),
        "itxt_trop_grand": b"iTXt" + b"k\x00\x01\x00\x00\x00" + big,
        "itxt_corrompu": b"iTXt" + b"k\x00\x01\x00\x00\x00garbage",
        "itxt_utf8_faux": b"iTXt" + b"k\x00\x00\x00\x00\x00\xff\xfe",
        "iccp_ok": b"iCCP" + b"nom\x00\x00" + zlib.compress(b"profil"),
        "iccp_sans_nul": b"iCCP" + b"\x00" + zlib.compress(b"profil"),
        "iccp_vide": b"iCCP",
        "iccp_nom_seul": b"iCCP" + b"nom\x00",
        "iccp_methode": b"iCCP" + b"nom\x00\x02" + zlib.compress(b"profil"),
        "iccp_corrompu": b"iCCP" + b"nom\x00\x00garbage",
        "iccp_trop_grand": b"iCCP" + b"nom\x00\x00" + big,
        "exif_ok": b"eXIf" + b"MM\x00\x2a",
        "plte_sur_rgb": b"PLTE" + bytes(range(30)),
    }
    for name, body in anc.items():
        kind, payload = body[:4], body[4:]
        rgb("anc_avant_" + name, before=[chunk(kind, payload)])
        rgb("anc_apres_" + name, after=[chunk(kind, payload)])
    many = [chunk(b"zTXt", b"k%d\x00\x00" % i + exact) for i in range(65)]
    rgb("text_memoire_epuisee", before=many)
    rgb("actl_delegue", before=[chunk(b"acTL", struct.pack(">II", 1, 0))])
    rgb("fctl_delegue", after=[chunk(b"fcTL", bytes(26))])
    apng_cases(add, rng, base_px, good)
    ihdr_cases(add, rng)
    # autres formats : délégation si Pillow sait peut-être les lire, sinon illisibles comme
    # pour Pillow (src/identify.rs, #129)
    add("pas_png", b"GIF89a" + bytes(20), "delegate")
    add("vide", b"")
    add("signature_seule", SIG)
    add("signature_tronquee", SIG[:5])
    return out


def alteration_bases():
    """Petits fichiers de base pour les tables de troncatures et d'octets inversés."""
    rng = random.Random(1260)
    px = rand_pixels(rng, 5, 4, 8, 3, maxval=5)
    pal = make_png(5, 4, 8, 3, px, 1, [chunk(b"PLTE", palette(6, rng)),
                                       chunk(b"tRNS", b"\x00\x80\xff")],
                   after=[chunk(b"tEXt", b"k\x00v")], idat_split=[12])
    g16 = make_png(4, 3, 16, 0, rand_pixels(rng, 4, 3, 16, 0), 0,
                   [chunk(b"tRNS", b"\x12\x34"), chunk(b"gAMA", struct.pack(">I", 45455))])
    rgba = make_png(6, 5, 8, 6, rand_pixels(rng, 6, 5, 8, 6), 1,
                    [chunk(b"zTXt", b"k\x00\x00" + zlib.compress(b"texte"))])
    # APNG à zone partielle, données continuées en fdAT, tRNS avant et après l'image (#459)
    rng = random.Random(4594)
    zg = zdata(3, 2, 8, 0, rand_pixels(rng, 3, 2, 8, 0, maxval=3))
    apng = (SIG + ihdr(5, 4, 8, 0) + chunk(b"tRNS", b"\x00\x02") + actl(1)
            + fctl(0, 3, 2, 1, 1) + chunk(b"IDAT", zg[:9]) + fdat(1, zg[9:])
            + chunk(b"tRNS", b"\x00\x01") + fctl(2, 2, 2) + chunk(b"IEND", b""))
    # IHDR répété (gris 2 bits puis RGB 16 bits) et tRNS rangé sous le premier mode
    zr = zdata(3, 2, 16, 2, rand_pixels(rng, 3, 2, 16, 2, maxval=3))
    double = (SIG + ihdr(3, 2, 2, 0) + chunk(b"tRNS", b"\x00\x01") + ihdr(3, 2, 16, 2)
              + chunk(b"IDAT", zr) + chunk(b"IEND", b""))
    return [("alt_palette", pal), ("alt_gris16", g16), ("alt_rgba", rgba), ("alt_apng", apng),
            ("alt_ihdr_double", double)]


def write(path, data):
    with open(path, "wb") as f:
        f.write(data)


def write_lines(path, lines):
    write(path, ("\n".join(lines) + "\n").encode("utf-8"))


def main_vectors():
    sys.path.insert(0, TOOLS)
    for f in os.listdir(HERE):
        if f.endswith(".png"):
            os.remove(os.path.join(HERE, f))
    lines = ["# nom\tattendu\trust (genere par gen_png_vectors.py, Python 3.14.8 + Pillow 12.3.0)"]
    for name, data, rust in cases():
        write(os.path.join(HERE, name + ".png"), data)
        lines.append("%s\t%s\t%s" % (name, oracle(data), rust))
    for name, data in alteration_bases():
        write(os.path.join(HERE, name + ".png"), data)
        lines.append("%s\t%s\tsame" % (name, oracle(data)))
    write_lines(os.path.join(HERE, "manifest.tsv"), lines)

    combos = ["# nom\tpng (hexadecimal)\tattendu (genere par gen_png_vectors.py)"]
    for name, data in combo_cases():
        combos.append("%s\t%s\t%s" % (name, data.hex(), oracle(data)))
    write_lines(os.path.join(HERE, "combos.tsv"), combos)

    alt =["# base\toperation\targument\tattendu (genere par gen_png_vectors.py)"]
    for name, data in alteration_bases():
        for n in range(len(data)):
            alt.append("%s\tcut\t%d\t%s" % (name, n, oracle(data[:n])))
        for i in range(8, len(data)):
            d = bytearray(data)
            d[i] ^= 0xFF
            alt.append("%s\txor\t%d\t%s" % (name, i, oracle(bytes(d))))
    write_lines(os.path.join(HERE, "alterations.tsv"), alt)


def main_matrix(out_dir, big):
    sys.path.insert(0, TOOLS)
    sys.path.insert(0, os.path.join(TOOLS, "tests"))
    sys.path.insert(0, os.path.join(TOOLS, "tests", "parity"))
    import make_corpus
    os.makedirs(out_dir, exist_ok=True)
    sizes = [s for s in make_corpus.SIZES if big or s <= 641]
    lines = ["# matrice make_corpus.py : nom\tattendu\trust"]
    for mode in make_corpus.MODES:
        for w in sizes:
            for h in sizes:
                if w != h and max(w, h) > 97:
                    continue
                for content in make_corpus.CONTENTS:
                    data = make_corpus.matrix_png(mode, w, h, content)
                    if data is None:
                        continue
                    name = "%s_%dx%d_%s" % (mode, w, h, content)
                    write(os.path.join(out_dir, name + ".png"), data)
                    lines.append("%s\t%s\tsame" % (name, oracle(data)))
    write_lines(os.path.join(out_dir, "manifest.tsv"), lines)


if __name__ == "__main__":
    check_versions()
    if len(sys.argv) >= 3 and sys.argv[1] == "--matrix":
        main_matrix(sys.argv[2], "--big" in sys.argv[3:])
    else:
        main_vectors()
