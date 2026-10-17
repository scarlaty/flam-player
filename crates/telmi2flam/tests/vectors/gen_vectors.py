"""Vecteurs de parité des codecs LIF et mp3map (port Rust, issue #122).

Produit, à partir des modules Python de référence (`tools/telmi2flam/lif.py` et `mp3map.py`),
de petites entrées synthétiques et les sorties attendues, lues par `tests/vectors.rs`.
Déterministe : relancer le script ne doit rien changer (`git status` propre).

    python crates/telmi2flam/tests/vectors/gen_vectors.py

Fichiers produits :
- `lif/manifest.tsv` (`nom  largeur  hauteur`), et pour chaque image `<nom>.rgba` (entrée
  RGBA8888), `<nom>.lif` (`lif.encode`) et `<nom>.q.rgba` (`lif.quantize_rgba`) ;
- `mp3map/manifest.tsv` : `nom  entrée  résultat`, où l'entrée est `file` (`<nom>.mp3`) ou
  `repeat:<trame hex>:<nombre>` (trop gros pour être commité), et le résultat
  `ok  <sortie>  <entrées>  <durée : bits du flottant en hex>  <trames>  <fréquences>` ou
  `err  <message de l'exception>` ; la sortie est en hexadécimal, ou `fnv1a64:<empreinte>:<taille>`
  au-delà de 256 octets ;
- `mp3map/truncations.tsv` : résultat de `mp3map.build` sur chaque préfixe d'un fichier,
  regroupé par plages de longueurs `début  fin` (bornes incluses) de même résultat.
"""

import os
import random
import struct
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "..", "..", "..", "tools", "telmi2flam"))

import lif  # noqa: E402
import mp3map  # noqa: E402


def write(path, data):
    with open(path, "wb") as f:
        f.write(data)


def write_lines(path, lines):
    write(path, ("\n".join(lines) + "\n").encode("utf-8"))


# ---------------------------------------------------------------------------
# LIF
# ---------------------------------------------------------------------------

def lif_images():
    rng = random.Random(122)
    imgs = []

    def add(name, w, h, px):
        imgs.append((name, w, h, bytes(px)))

    add("zero", 0, 0, b"")
    add("un_pixel", 1, 1, [200, 100, 50, 255])
    add("vide_8x8", 8, 8, bytes(8 * 8 * 4))
    # Dégradé avec rampe d'alpha : deltas, vert étendu, alpha changeant.
    add("degrade", 16, 8, [v for y in range(8) for x in range(16)
                           for v in (x * 16, y * 32, (x * y * 4) & 0xFF, 255 - x * 8)])
    # Bruit : couleurs pleines, alpha de trois valeurs, extrêmes 0 et 255.
    add("bruit", 13, 7, [v for _ in range(13 * 7)
                         for v in (rng.randrange(256), rng.randrange(256), rng.randrange(256),
                                   rng.choice((0, 128, 255)))])
    # Plages de longueurs 1, 61, 62, 63, 64, 130 : bornes du RLE (62 au plus).
    runs = []
    for i, n in enumerate((1, 61, 62, 63, 64, 130)):
        runs += [(i * 40, 255 - i * 40, 128, 255)] * n
    add("plages", len(runs), 1, [v for p in runs for v in p])
    # Petits pas sur chaque canal, avec repli modulo (31 -> 0, 63 -> 0) et retours au cache.
    steps = []
    r, g, b = 248, 252, 0
    for i in range(120):
        r = (r + (8, -8, 0, 16)[i % 4]) % 256
        g = (g + (4, 0, -4, 8, 128)[i % 5]) % 256
        b = (b + (0, 8, -16, 24)[i % 4]) % 256
        steps.append((r, g, b, 255 if i % 17 else 254))
    add("pas", 12, 10, [v for p in steps for v in p])
    # Image haute (3 x 80) : encodée normalement ; le décodeur peut la transposer.
    add("haute", 3, 80, [v for y in range(80) for x in range(3)
                         for v in (y * 3, x * 80, 255 - y * 3, 255)])
    return imgs


def gen_lif(out_dir):
    os.makedirs(out_dir, exist_ok=True)
    manifest = ["# nom\tlargeur\thauteur (genere par gen_vectors.py)"]
    for name, w, h, px in lif_images():
        enc = lif.encode(px, w, h)
        q = lif.quantize_rgba(px, w, h)
        dec, dw, dh = lif.decode(enc)
        assert (dec, dw, dh) == (q, w, h), name
        write(os.path.join(out_dir, name + ".rgba"), px)
        write(os.path.join(out_dir, name + ".lif"), enc)
        write(os.path.join(out_dir, name + ".q.rgba"), q)
        manifest.append("%s\t%d\t%d" % (name, w, h))
    write_lines(os.path.join(out_dir, "manifest.tsv"), manifest)


# ---------------------------------------------------------------------------
# mp3map
# ---------------------------------------------------------------------------

def header(version, layer, bitrate_index, sr_index, padding=0):
    """4 octets d'en-tête : version 3 = MPEG1, 2 = MPEG2, 0 = MPEG2.5 ; layer 1 = III."""
    b1 = 0xE0 | (version << 3) | (layer << 1) | 1
    b2 = (bitrate_index << 4) | (sr_index << 2) | (padding << 1)
    return bytes((0xFF, b1, b2, 0xC0))


def frame(rng, version, layer, bitrate_index, sr_index, padding=0, zero=False):
    hdr = header(version, layer, bitrate_index, sr_index, padding)
    length = mp3map._parse_frame_header(hdr, 0)[0]
    body = bytes(length - 4) if zero else bytes(rng.randrange(256) for _ in range(length - 4))
    return hdr + body


def frames(rng, count, *args, pad_every=0):
    out = b""
    for i in range(count):
        pad = 1 if pad_every and i % pad_every == 0 else 0
        out += frame(rng, *args, padding=pad)
    return out


def id3(size, footer=False):
    syncsafe = bytes(((size >> 21) & 0x7F, (size >> 14) & 0x7F, (size >> 7) & 0x7F, size & 0x7F))
    tag = b"ID3\x04\x00" + bytes((0x10 if footer else 0,)) + syncsafe + bytes(size)
    return tag + (b"3DI" + bytes(7) if footer else b"")


def junk(rng, n):
    """Octets quelconques avec de fausses synchronisations (0xFFFx non chaînées)."""
    out = bytearray(rng.randrange(256) for _ in range(n))
    for i in range(0, n - 4, 37):
        out[i:i + 4] = b"\xff\xfb\x90\x00"
    return bytes(out)


def mp3_cases():
    rng = random.Random(22)
    m1l3 = (3, 1, 1, 0)        # MPEG1 Layer III 32 kbit/s 44,1 kHz : 104 octets
    cases = [
        ("mpeg1_l3_id3", id3(20) + frames(rng, 60, *m1l3, pad_every=3)),
        ("mpeg1_l3_long", frames(rng, 120, *m1l3)),           # > 100 trames : x = 34
        ("mpeg2_l3_22k", frames(rng, 90, 2, 1, 1, 0, pad_every=2)),
        ("mpeg2_l3_24k", frames(rng, 160, 2, 1, 1, 1)),       # 2116,8 unités par trame
        ("mpeg25_l3_8k", frames(rng, 40, 0, 1, 1, 2)),
        ("mpeg25_l2_12k", frames(rng, 30, 0, 2, 1, 1)),
        ("mpeg1_l2_48k", frames(rng, 50, 3, 2, 1, 1)),
        ("mpeg1_l1_32k", frames(rng, 140, 3, 3, 1, 2, pad_every=5)),
        ("mpeg2_l1_16k", frames(rng, 60, 2, 3, 1, 2)),
        ("frequences_mixtes", frames(rng, 30, 3, 1, 1, 0) + frames(rng, 30, 3, 1, 1, 1)
         + frames(rng, 30, 3, 1, 1, 2)),
        ("id3_pied", id3(33, footer=True) + frames(rng, 50, *m1l3)),
        ("dechets_en_tete", junk(rng, 500) + frames(rng, 50, *m1l3)),
        ("dechets_au_milieu", frames(rng, 30, *m1l3) + junk(rng, 300) + frames(rng, 30, *m1l3)),
        ("derniere_trame_coupee", frames(rng, 50, *m1l3)[:-40]),
        ("trame_isolee_finale", frames(rng, 50, *m1l3) + header(3, 1, 1, 0)),
        ("silent", (bytes([0xFF, 0xFB, 0x90, 0x00]) + bytes(413)) * 24),
        # Erreurs.
        ("vide", b""),
        ("id3_seul", b"ID3"),
        ("id3_coupe", b"ID3\x04\x00\x00\x00\x00\x01"),
        ("id3_demesure", b"ID3\x04\x00\x10\x7f\x7f\x7f\x7f" + frames(rng, 5, *m1l3)),
        ("zeros", bytes(1000)),
        ("ff", b"\xff" * 1000),
        ("dechets", junk(rng, 3000)),
        ("non_mpeg", frames(rng, 3, *m1l3) + junk(rng, 5000)),
        ("en_tetes_reserves", b"".join(header(v, l, b, s) + bytes(100)
                                       for v, l, b, s in ((1, 1, 1, 0), (3, 0, 1, 0),
                                                          (3, 1, 0, 0), (3, 1, 15, 0),
                                                          (3, 1, 1, 3)) * 4)),
    ]
    return cases


def result_columns(data):
    info = {}
    try:
        out, dur, num = mp3map.build(data, info)
    except Exception as e:  # même périmètre que add_mp3map de telmi2flam.py
        return ["err", str(e)]
    bits = struct.unpack("<Q", struct.pack("<d", dur))[0]
    rates = ",".join(str(r) for r in sorted(info["samplerates"]))
    return ["ok", digest(out), str(num), "%016x" % bits, str(info["frames"]), rates]


def digest(out):
    """Sortie en hexadécimal, ou empreinte FNV-1a 64 bits et taille au-delà de 256 octets."""
    if len(out) <= 256:
        return out.hex()
    h = 0xCBF29CE484222325
    for b in out:
        h = ((h ^ b) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return "fnv1a64:%016x:%d" % (h, len(out))


def gen_mp3map(out_dir):
    os.makedirs(out_dir, exist_ok=True)
    manifest = ["# nom\tentree\tresultat (genere par gen_vectors.py)"]
    for name, data in mp3_cases():
        write(os.path.join(out_dir, name + ".mp3"), data)
        manifest.append("\t".join([name, "file"] + result_columns(data)))

    # Dépassement de 32 bits de total_units (OverflowError) : ~49 Mo de trames MPEG2.5 Layer II
    # 8 kbit/s 8 kHz (144 octets, 12 700,8 unités chacune), décrites plutôt que commitées.
    hdr = header(0, 2, 1, 2)
    big = mp3map._parse_frame_header(hdr, 0)[0]
    one = hdr + bytes(big - 4)
    for name, count in (("debordement_juste_sous", 338_100), ("debordement", 340_000)):
        data = one * count
        manifest.append("\t".join([name, "repeat:%s:%d" % (one.hex(), count)]
                                  + result_columns(data)))
    write_lines(os.path.join(out_dir, "manifest.tsv"), manifest)

    # Troncatures : chaque préfixe d'un petit fichier (ID3, trames MPEG2 Layer III 8 kbit/s
    # 24 kHz de 24 octets, déchets) ; assez de trames pour produire des entrées.
    rng = random.Random(7)
    base = id3(6) + frames(rng, 20, 2, 1, 1, 1) + junk(rng, 40) + frames(rng, 50, 2, 1, 1, 1)
    write(os.path.join(out_dir, "troncature.mp3"), base)
    lines = ["# debut\tfin\tresultat : mp3map.build(troncature.mp3[:longueur]) "
             "(genere par gen_vectors.py)"]
    start, prev = 0, None
    for length in range(len(base) + 1):
        cols = result_columns(base[:length])
        if prev is not None and cols != prev:
            lines.append("\t".join([str(start), str(length - 1)] + prev))
            start = length
        prev = cols
    lines.append("\t".join([str(start), str(len(base))] + prev))
    write_lines(os.path.join(out_dir, "truncations.tsv"), lines)


if __name__ == "__main__":
    gen_lif(os.path.join(HERE, "lif"))
    gen_mp3map(os.path.join(HERE, "mp3map"))
