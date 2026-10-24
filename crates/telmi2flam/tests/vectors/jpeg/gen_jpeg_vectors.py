"""Vecteurs de parité du décodage JPEG (port Rust, #459, décision D7).

L'oracle est `_open_rgba` de `tools/telmi2flam/telmi2flam.py`, exécuté avec **Pillow 12.3.0**
(libjpeg-turbo 3.1.4.1, versions de `tools/telmi2flam/ORACLE-VERSIONS.txt`) : le script refuse
de tourner avec une autre version de Pillow ou de libjpeg-turbo (le correctif de Python 3.14
n'intervient pas dans le décodage JPEG). Les JPEG sont écrits par Pillow à partir d'images
synthétiques (graine fixe), puis parfois retouchés octet par octet (marqueurs Adobe, JFIF,
identifiants de composantes, `SOF` altérés), sans contenu sous licence. Déterministe.

    python -I crates/telmi2flam/tests/vectors/jpeg/gen_jpeg_vectors.py [--check]

Fichiers produits (lus par `tests/jpeg_vectors.rs`) :
- `jpeg_cases.zip` (deflate) : `<nom>.jpg` pour chaque vecteur et chaque base d'altération,
  et `<nom>.rgba` (`tobytes()` de l'image RGBA de Python) pour chaque vecteur lisible ;
- `vectors.tsv` : `nom  attendu`, où `attendu` vaut `rgba:<largeur>x<hauteur>` (image lue :
  le Rust doit donner la même taille et chaque composante à ±4 près, alpha exact), `unreadable`
  (exception levée par `_open_rgba` : avertissement `image_unreadable`) ou `delegate` (lu par
  libjpeg-turbo mais pas par zune-jpeg : codage arithmétique ou sans perte, code 7) ;
- `alterations.tsv` : `base  opération  argument  attendu`, verdict de `_open_rgba` (`ok` ou
  `unreadable`) sur chaque troncature (`cut`, longueur) et chaque octet inversé (`xor`,
  position) des bases `alt_*`. Les écarts de verdict du Rust sont listés à la main dans
  `alterations_divergences.tsv` (écarts assumés de §4.4 : libjpeg-turbo poursuit sur des
  données corrompues, zune-jpeg non, ou l'inverse).

`--check` ne réécrit rien : il échoue si les fichiers ne sont pas ceux que l'oracle produit
(contenu des membres du ZIP comparé, pas les octets du flux deflate).
"""

import io
import os
import random
import struct
import sys
import zipfile

PILLOW = "12.3.0"
LIBJPEG_TURBO = "3.1.4.1"
FIXED_DATE = (2026, 1, 1, 0, 0, 0)

HERE = os.path.dirname(os.path.abspath(__file__))
TOOLS = os.path.normpath(os.path.join(HERE, "..", "..", "..", "..", "..", "tools", "telmi2flam"))
sys.path.insert(0, TOOLS)
sys.dont_write_bytecode = True


def check_versions():
    import PIL
    from PIL import features
    turbo = features.version("libjpeg_turbo")
    if PIL.__version__ != PILLOW or turbo != LIBJPEG_TURBO or sys.version_info[:2] != (3, 14):
        sys.exit("oracle attendu : Python 3.14, Pillow %s, libjpeg-turbo %s (trouvé : Python %s, "
                 "Pillow %s, libjpeg-turbo %s)" % (PILLOW, LIBJPEG_TURBO, sys.version.split()[0],
                                                   PIL.__version__, turbo))


def oracle(data):
    """Image RGBA de `_open_rgba`, ou None si Python lève une exception."""
    import telmi2flam
    try:
        im = telmi2flam._open_rgba(data)
        raw = im.tobytes()
    except Exception:
        return None
    assert im.mode == "RGBA"
    return im.size, raw


# ---------------------------------------------------------------------------
# Images et JPEG
# ---------------------------------------------------------------------------
def content(w, h, kind, seed):
    from PIL import Image
    rng = random.Random(seed)
    im = Image.new("RGB", (w, h))
    px = []
    for y in range(h):
        for x in range(w):
            if kind == "noise":
                px.append((rng.randrange(256), rng.randrange(256), rng.randrange(256)))
            elif kind == "grad":
                px.append((x * 255 // max(1, w - 1), y * 255 // max(1, h - 1), (x + y) * 3 % 256))
            else:
                px.append(((x // 7) * 40 % 256, (y // 5) * 60 % 256, ((x ^ y) * 3) % 256))
    im.putdata(px)
    return im


def jpeg(im, **kw):
    b = io.BytesIO()
    im.save(b, "JPEG", **kw)
    return b.getvalue()


def segments(data):
    """Segments avant le premier SOS : liste de (position, marqueur, longueur totale)."""
    out = []
    pos = 2
    while pos < len(data):
        assert data[pos] == 0xFF, pos
        m = data[pos + 1]
        n = struct.unpack(">H", data[pos + 2:pos + 4])[0]
        out.append((pos, m, n + 2))
        if m == 0xDA:
            break
        pos += n + 2
    return out


def drop(data, marker):
    """Retire tous les segments `marker` situés avant le premier SOS."""
    for pos, m, n in reversed(segments(data)):
        if m == marker:
            data = data[:pos] + data[pos + n:]
    return data


def insert(data, seg):
    """Insère un segment juste après SOI."""
    return data[:2] + seg + data[2:]


def seg(marker, body):
    return bytes([0xFF, marker]) + struct.pack(">H", len(body) + 2) + body


def adobe(transform, length=12):
    body = b"Adobe" + b"\x00\x64\x00\x00\x00\x00" + bytes([transform])
    return seg(0xEE, body[:length])


def patch_sof(data, fn):
    """Applique `fn(bytearray du corps du SOF)` au premier SOF."""
    for pos, m, n in segments(data):
        if m in (0xC0, 0xC1, 0xC2):
            body = bytearray(data[pos + 4:pos + n])
            fn(body)
            return data[:pos + 4] + bytes(body) + data[pos + n:]
    raise AssertionError("pas de SOF")


def set_marker(data, old, new):
    for pos, m, _ in segments(data):
        if m == old:
            return data[:pos + 1] + bytes([new]) + data[pos + 2:]
    raise AssertionError("marqueur absent")


def vectors():
    """(nom, octets JPEG)."""
    out = []
    seed = 459
    # RGB : tailles, contenus, sous-échantillonnages 4:4:4 / 4:2:2 / 4:2:0, qualités,
    # séquentiel et progressif
    for (w, h), kinds in (((1, 1), ("grad", "blocks")), ((17, 9), ("noise", "grad", "blocks")),
                          ((33, 21), ("noise", "grad", "blocks")), ((64, 48), ("grad", "blocks"))):
        for kind in kinds:
            seed += 1
            im = content(w, h, kind, seed)
            for sub in (0, 1, 2):
                for q in (30, 95):
                    for prog in (False, True):
                        name = "rgb_%dx%d_%s_s%d_q%d_%s" % (w, h, kind, sub, q, "p" if prog else "b")
                        out.append((name, jpeg(im, quality=q, subsampling=sub, progressive=prog)))
    noise = content(33, 21, "noise", 1)
    out.append(("rgb_101x67_grad_s2_q75", jpeg(content(101, 67, "grad", 2), quality=75)))
    out.append(("rgb_optimise", jpeg(noise, quality=75, optimize=True)))
    out.append(("rgb_restart_blocs", jpeg(noise, quality=75, restart_marker_blocks=1)))
    out.append(("rgb_restart_lignes", jpeg(noise, quality=75, restart_marker_rows=1)))
    out.append(("rgb_restart_progressif", jpeg(noise, quality=75, progressive=True,
                                               restart_marker_blocks=2)))
    # gris et CMYK (Pillow écrit le CMYK en polarité Adobe, transformation 0)
    for (w, h) in ((17, 9), (33, 21), (64, 48)):
        for kind in ("noise", "grad"):
            seed += 1
            im = content(w, h, kind, seed)
            for q in (50, 90):
                for prog in (False, True):
                    out.append(("l_%dx%d_%s_q%d_%s" % (w, h, kind, q, "p" if prog else "b"),
                                jpeg(im.convert("L"), quality=q, progressive=prog)))
                    if (w, h) != (64, 48):
                        out.append(("cmyk_%dx%d_%s_q%d_%s" % (w, h, kind, q, "p" if prog else "b"),
                                    jpeg(im.convert("CMYK"), quality=q, progressive=prog)))
    # espace de couleur choisi par libjpeg-turbo : JFIF, puis Adobe, puis identifiants
    base = jpeg(noise, quality=75, subsampling=0)
    cmyk = jpeg(noise.convert("CMYK"), quality=75)
    nojfif = drop(base, 0xE0)

    def ids(data, a, b, c):
        def fn(body):
            body[6], body[9], body[12] = a, b, c
        return patch_sof(data, fn)

    def sos_ids(data, a, b, c):
        pos, _, n = segments(data)[-1]
        body = bytearray(data[pos + 4:pos + n])
        body[1], body[3], body[5] = a, b, c
        return data[:pos + 4] + bytes(body) + data[pos + n:]

    out += [
        ("cs_rgb_adobe0", insert(nojfif, adobe(0))),
        ("cs_rgb_ids", sos_ids(ids(nojfif, 82, 71, 66), 82, 71, 66)),
        ("cs_jfif_ids_rgb", sos_ids(ids(base, 82, 71, 66), 82, 71, 66)),
        ("cs_jfif_adobe0", insert(base, adobe(0))),
        ("cs_sans_jfif", nojfif),
        ("cs_adobe1", insert(nojfif, adobe(1))),
        ("cs_adobe_inconnu", insert(nojfif, adobe(7))),
        ("cs_adobe_court_lu", insert(nojfif, adobe(0, 8))),
        ("cs_adobe_trop_court", insert(nojfif, adobe(0, 6))),
        ("cs_cmyk_sans_adobe", drop(cmyk, 0xEE)),
        ("cs_ycck", insert(drop(cmyk, 0xEE), adobe(2))),
        ("cs_ycck_inconnu", insert(drop(cmyk, 0xEE), adobe(5))),
        ("cs_jfif_court", insert(nojfif, seg(0xE0, b"JFIF\x00\x01"))),
        ("cs_jfif_sans_nul", insert(nojfif, seg(0xE0, b"JFIF" + bytes(10)))),
    ]
    # segments que Pillow analyse : ICC, Photoshop, commentaires, octets parasites
    sof_pos = next(p for p, m, _ in segments(base) if m == 0xC0)
    out += [
        ("icc_court_avant_sof", insert(base, seg(0xE2, b"ICC_PROFILE\x00\x01"))),
        ("icc_court_apres_sof", base[:sof_pos + 19] + seg(0xE2, b"ICC_PROFILE\x00\x01")
         + base[sof_pos + 19:]),
        ("icc_long", insert(base, seg(0xE2, b"ICC_PROFILE\x00\x01\x01" + bytes(20)))),
        ("photoshop_tronque", insert(base, seg(0xED, b"Photoshop 3.0\x008BIM\x03\xed"))),
        ("photoshop_ok", insert(base, seg(0xED, b"Photoshop 3.0\x008BIM\x04\x04\x00\x00"
                                       + struct.pack(">I", 2) + b"ab"))),
        # ResolutionInfo vide : struct.error rattrape, boucle arretee avant le bloc tronque
        ("photoshop_resolution_court", insert(base, seg(0xED, b"Photoshop 3.0\x008BIM\x03\xed"
                                                     b"\x00\x00" + bytes(4) + b"8BIM\x00\x00"))),
        ("commentaire", insert(base, seg(0xFE, b"bonjour"))),
        ("app15_inconnu", insert(base, seg(0xEF, b"\xff\xd9\xff\xda"))),
        ("app_longueur_1", insert(base, b"\xff\xe3\x00\x01")),
        ("parasites_avant_dqt", base[:sof_pos] + b"\x00\x12\x34" + base[sof_pos:]),
        ("remplissage_ff", base[:sof_pos] + b"\xff\xff" + base[sof_pos:]),
        ("ff00_avant_marqueur", base[:sof_pos] + b"\xff\x00" + base[sof_pos:]),
        ("donnees_apres_eoi", base + b"garbage\xff\xd8"),
        ("rst_avant_sos", base[:sof_pos] + b"\xff\xd0" + base[sof_pos:]),
        ("dnl_avant_sos", base[:sof_pos] + seg(0xDC, b"\x00\x15") + base[sof_pos:]),
        ("tem_avant_sos", base[:sof_pos] + b"\xff\x01" + base[sof_pos:]),
        ("jpg0_avant_sos", base[:sof_pos] + seg(0xF0, b"xx") + base[sof_pos:]),
        ("soi_double", base[:sof_pos] + b"\xff\xd8" + base[sof_pos:]),
        ("eoi_avant_sos", base[:sof_pos] + b"\xff\xd9" + base[sof_pos:]),
        ("tables_puis_image", b"\xff\xd8" + base[2:sof_pos] + b"\xff\xd9" + base),
    ]
    # SOF : précision, composantes, dimensions, types, doublons
    def body_set(i, v):
        def fn(body):
            body[i] = v
        return fn

    def dims(w, h):
        def fn(body):
            body[1:5] = struct.pack(">HH", h, w)
        return fn

    sof = base[sof_pos:sof_pos + 19]
    out += [
        ("sof_12_bits", patch_sof(base, body_set(0, 12))),
        ("sof_2_composantes", patch_sof(base, body_set(5, 2))),
        ("sof_largeur_nulle", patch_sof(base, dims(0, 21))),
        ("sof_bombe", patch_sof(base, dims(65535, 65535))),
        ("sof_bombe_juste", patch_sof(base, dims(13400, 13400))),
        ("sof_trop_large", patch_sof(base, dims(65501, 1))),
        ("sof_double", base[:sof_pos] + sof + base[sof_pos:]),
        ("sof_echantillonnage_5", patch_sof(base, body_set(7, 0x51))),
        ("sof_echantillonnage_0", patch_sof(base, body_set(7, 0x01))),
        ("sof_mcu_18_blocs", patch_sof(base, body_set(7, 0x44))),
        ("sof_sof1", set_marker(base, 0xC0, 0xC1)),
        ("sof_sans_perte", set_marker(base, 0xC0, 0xC3)),
        ("sof_arithmetique", set_marker(base, 0xC0, 0xC9)),
        ("sof_arithmetique_progressif", set_marker(base, 0xC0, 0xCA)),
        ("sof_differentiel", set_marker(base, 0xC0, 0xC5)),
        ("sof_dhp", set_marker(base, 0xC0, 0xDE)),
        ("sos_composante_inconnue", sos_ids(base, 1, 2, 9)),
        ("sos_composante_double", sos_ids(base, 1, 2, 2)),
        ("sans_sof", drop(base, 0xC0)),
        ("sans_dqt", drop(base, 0xDB)),
        ("sans_dht", drop(base, 0xC4)),
        ("dri_longueur_5", base[:sof_pos] + seg(0xDD, b"\x00\x01\x00") + base[sof_pos:]),
    ]
    # apres le balayage unique : TEM ignore, second SOS refuse (JERR_EOI_EXPECTED)
    sos_pos, _, sos_n = segments(base)[-1]
    out += [
        ("tem_apres_balayage", base[:-2] + b"\xff\x01\xff\xd9"),
        ("sos_apres_balayage", base[:-2] + base[sos_pos:]),
    ]
    assert sos_n
    # fins de flux : EOI absent, coupé dans les données, EOI ajouté après une coupure
    prog = jpeg(noise, quality=75, progressive=True)
    for name, data in (("seq", base), ("prog", prog)):
        n = len(data)
        out += [
            ("fin_%s_sans_eoi" % name, data[:-2]),
            ("fin_%s_ff_seul" % name, data[:-1]),
            ("fin_%s_moitie_eoi" % name, data[:n * 3 // 4] + b"\xff\xd9"),
            ("fin_%s_moitie_dht" % name, data[:n * 3 // 4] + b"\xff\xc4"),
            ("fin_%s_eoi_puis_parasites" % name, data + b"\x00" * 9),
        ]
    out += followups_491(base, noise, sof_pos, prog)
    return out


# Nombre maximal de balayages lus par le Rust (`MAX_SCANS` de `src/jpeg.rs`) : au-delà, délégué.
MAX_SCANS = 1000


def markers(data):
    """Marqueurs d'un JPEG sans redémarrage : liste de (position, octet bas), données des
    balayages sautées (`FF 00` et remplissage `FF FF`)."""
    out = []
    pos = 2
    while pos < len(data) - 1:
        if data[pos] != 0xFF or data[pos + 1] in (0x00, 0xFF):
            pos += 1
            continue
        m = data[pos + 1]
        out.append((pos, m))
        if m in (0xD8, 0xD9) or 0xD0 <= m <= 0xD7 or m == 0x01:
            pos += 2
        else:
            pos += 2 + struct.unpack(">H", data[pos + 2:pos + 4])[0]
    return out


def first_scan_end(data):
    """Position du marqueur qui suit les données du premier balayage."""
    ms = markers(data)
    k = next(i for i, (_, m) in enumerate(ms) if m == 0xDA)
    return ms[k + 1][0]


def ac_first_chunk(data):
    """(début, fin) d'un balayage AC de premier passage (Ss > 0, Ah = 0) d'un JPEG
    progressif, `DHT` qui le précèdent compris : le répéter juste après ne change pas l'image
    (coefficients réaffectés aux mêmes valeurs)."""
    ms = markers(data)
    for k, (pos, m) in enumerate(ms):
        if m != 0xDA:
            continue
        n = data[pos + 4]
        ss, ah = data[pos + 5 + 2 * n], data[pos + 7 + 2 * n] >> 4
        if ss == 0 or ah != 0:
            continue
        start = k
        while start > 0 and ms[start - 1][1] == 0xC4:
            start -= 1
        return ms[start][0], ms[k + 1][0]
    raise AssertionError("pas de balayage AC de premier passage")


def dqt_precision(data, pq):
    """Tables de quantification réécrites en 16 bits avec la précision `pq` (mêmes valeurs)."""
    for pos, m, n in reversed(segments(data)):
        if m != 0xDB:
            continue
        body, new = data[pos + 4:pos + n], b""
        while body:
            new += bytes([pq << 4 | body[0] & 15]) + b"".join(
                struct.pack(">H", v) for v in body[1:65])
            body = body[65:]
        data = data[:pos] + seg(0xDB, new) + data[pos + n:]
    return data


def minimal_gray(w, h, eoi=True):
    """JPEG gris uni (niveau 128) écrit à la main : tables de Huffman d'un seul code `0`
    (DC : différence 0 ; AC : fin de bloc), chaque bloc vaut deux bits nuls ; `w * h / 64`
    multiple de 4 : données faites d'octets nuls, sans bourrage final."""
    blocks = (w // 8) * (h // 8)
    assert w % 8 == 0 and h % 8 == 0 and blocks % 4 == 0
    one_code = bytes([1] + [0] * 15) + b"\x00"
    return (b"\xff\xd8" + seg(0xDB, b"\x00" + bytes([1] * 64))
            + seg(0xC0, b"\x08" + struct.pack(">HH", h, w) + b"\x01\x01\x11\x00")
            + seg(0xC4, b"\x00" + one_code) + seg(0xC4, b"\x10" + one_code)
            + seg(0xDA, b"\x01\x01\x00\x00\x3f\x00") + bytes(blocks // 4)
            + (b"\xff\xd9" if eoi else b""))


def followups_491(base, noise, sof_pos, prog):
    """Suivis de la revue de #459 (#491)."""
    out = []
    # m4 : erreur de `_open` (rattrapée par `Image.open`) et `PCD_` à l'octet 2048 : Pillow lit
    # une image PCD (768 x 512, données à 96 x 2048) ; erreur de libjpeg : illisible ; erreur de
    # libjpeg puis erreur de `_open` : `_open` passe d'abord (PCD).
    def pcd(data):
        return (data.ljust(2048, b"\x00") + b"PCD_").ljust(96 * 2048 + 768 * 512 * 3 // 2,
                                                            b"\x80")
    sof = base[sof_pos:sof_pos + 19]
    out += [
        ("pcd_polyglotte", pcd(base[:sof_pos] + b"\xff\x01" + base[sof_pos:])),
        ("pcd_polyglotte_court", (base[:sof_pos] + b"\xff\x01" + base[sof_pos:])
         .ljust(2048, b"\x00") + b"PCD_"),
        ("pcd_erreur_libjpeg", pcd(base[:sof_pos] + sof + base[sof_pos:])),
        ("pcd_libjpeg_puis_pillow", pcd(base[:sof_pos] + sof + b"\xff\x01" + base[sof_pos:])),
    ]
    # m6 : DQT après le premier balayage, reste plus court qu'une table (libjpeg lit 64 valeurs
    # au-delà du segment : JERR_BAD_LENGTH), table entière valide (index 2, inutilisée)
    end = first_scan_end(prog)
    ones = bytes([1] * 64)
    for name, body in (("dqt_reste_apres_sos", b"\x02" + ones + b"\x00"),
                       ("dqt_16_bits_impair_apres_sos", b"\x12" + bytes(127)),
                       ("dqt_apres_sos", b"\x02" + ones),
                       ("dqt_pq_2_apres_sos", b"\x22" + ones * 2)):
        out.append((name, prog[:end] + seg(0xDB, body) + prog[end:]))
    # vecteurs de la revue : DQT en 16 bits (précision 1 et 2), facteur 3, table de Huffman
    # inutilisée invalide, arithmétique (table 2, table de quantification absente)
    gray = jpeg(flat_rgb(48, 48, (128, 128, 128)), quality=75, subsampling=0)

    def factor(body):
        body[7] = 0x31
    sos_pos = segments(base)[-1][0]
    out += [
        ("dqt_16_bits", dqt_precision(base, 1)),
        ("dqt_pq_2", dqt_precision(base, 2)),
        ("sof_facteur_3", patch_sof(gray, factor)),
        ("dht_inutilisee_invalide",
         insert(base, seg(0xC4, b"\x02" + bytes([2] + [0] * 15) + b"\x00\x01"))),
        ("sof_arithmetique_table_2", set_marker(base[:sos_pos + 6] + b"\x22"
                                                + base[sos_pos + 7:], 0xC0, 0xC9)),
        ("sof_arithmetique_sans_dqt", set_marker(drop(base, 0xDB), 0xC0, 0xC9)),
    ]
    # DAC (lu par libjpeg quel que soit le codage) : avant le SOS, après le balayage
    out += [
        ("dac_avant_sos", insert(base, seg(0xCC, b"\x00\x10\x10\x05"))),
        ("dac_valeur_invalide", insert(base, seg(0xCC, b"\x00\x01"))),
        ("dac_index_32", insert(base, seg(0xCC, b"\x20\x05"))),
        ("dac_longueur_impaire", insert(base, seg(0xCC, b"\x10\x05\x00"))),
        ("dac_apres_balayage", base[:-2] + seg(0xCC, b"\x10\x05") + b"\xff\xd9"),
        ("dac_apres_balayage_invalide", base[:-2] + seg(0xCC, b"\x00\x01") + b"\xff\xd9"),
        ("dac_progressif_index_32", prog[:end] + seg(0xCC, b"\x20\x05") + prog[end:]),
    ]
    # copy_entropy : marqueur invalide précédé de remplissage juste avant un RSTn (écarté par
    # libjpeg en resynchronisant), remplissage avant EOI
    rst = jpeg(noise, quality=75, restart_marker_blocks=1)
    at = rst.index(b"\xff\xd0")
    out += [
        ("rst_remplissage_marqueur_invalide", rst[:at] + b"\xff\xff\x05" + rst[at:]),
        ("remplissage_avant_eoi", base[:-2] + b"\xff\xff\xff\xd9"),
    ]
    # balayage unique sans EOI : données utiles jusqu'au bout, faites de bits nuls (couper les
    # 8 derniers octets ne change rien pour zune-jpeg, qui complète par des 0)
    out += [
        ("huffman_un_code", minimal_gray(64, 64)),
        ("fin_seq_zeros_sans_eoi", minimal_gray(64, 64, eoi=False)),
    ]
    # m11 : balayages comptés (MAX_SCANS lus, un de plus délégué) : un balayage AC de premier
    # passage répété, image inchangée
    small = jpeg(content(17, 9, "noise", 11).convert("L"), quality=75, progressive=True)
    start, stop = ac_first_chunk(small)
    count = small.count(b"\xff\xda")
    for name, total in (("prog_balayages_max", MAX_SCANS), ("prog_balayages_trop", MAX_SCANS + 1)):
        out.append((name, small[:stop] + small[start:stop] * (total - count) + small[stop:]))
    return out


def flat_rgb(w, h, color):
    from PIL import Image
    return Image.new("RGB", (w, h), color)


def alteration_bases():
    noise = content(17, 9, "noise", 3)
    return [
        ("alt_sequentiel", jpeg(noise, quality=60, restart_marker_blocks=1)),
        ("alt_progressif", jpeg(noise, quality=60, progressive=True)),
        ("alt_gris", jpeg(noise.convert("L"), quality=60)),
    ]


# ---------------------------------------------------------------------------
# Écriture
# ---------------------------------------------------------------------------
def build():
    members = {}
    rows = ["# nom\tattendu (genere par gen_jpeg_vectors.py)"]
    names = set()
    for name, data in vectors():
        assert name not in names, name
        names.add(name)
        members[name + ".jpg"] = data
        res = oracle(data)
        if res is None:
            rows.append("%s\tunreadable" % name)
        else:
            (w, h), raw = res
            members[name + ".rgba"] = raw
            rows.append("%s\trgba:%dx%d" % (name, w, h))
    alts = ["# base\toperation\targument\tattendu (genere par gen_jpeg_vectors.py)"]
    for name, data in alteration_bases():
        members[name + ".jpg"] = data
        for i in range(len(data)):
            alts.append("%s\tcut\t%d\t%s" % (name, i, "unreadable" if oracle(data[:i]) is None
                                               else "ok"))
        for i in range(len(data)):
            d = bytearray(data)
            d[i] ^= 0xFF
            alts.append("%s\txor\t%d\t%s" % (name, i, "unreadable" if oracle(bytes(d)) is None
                                               else "ok"))
    return members, "\n".join(rows) + "\n", "\n".join(alts) + "\n"


def write_zip(path, members):
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        for name in sorted(members):
            z.writestr(zipfile.ZipInfo(name, FIXED_DATE), members[name], zipfile.ZIP_DEFLATED, 9)


def main(check=False):
    check_versions()
    members, rows, alts = build()
    zip_path = os.path.join(HERE, "jpeg_cases.zip")
    files = {"vectors.tsv": rows, "alterations.tsv": alts}
    if check:
        bad = []
        for name, text in files.items():
            with open(os.path.join(HERE, name), encoding="utf-8", newline="") as f:
                if f.read() != text:
                    bad.append(name)
        with zipfile.ZipFile(zip_path) as z:
            have = {n: z.read(n) for n in z.namelist()}
        if have != members:
            bad.append("jpeg_cases.zip")
        if bad:
            sys.exit("vecteurs JPEG perimes : %s (relancer gen_jpeg_vectors.py)" % ", ".join(bad))
        print("vecteurs JPEG a jour")
        return
    for name, text in files.items():
        with open(os.path.join(HERE, name), "w", encoding="utf-8", newline="\n") as f:
            f.write(text)
    write_zip(zip_path, members)
    print("%d vecteurs, %d alterations" % (rows.count("\n") - 1, alts.count("\n") - 1))


if __name__ == "__main__":
    main("--check" in sys.argv[1:])
