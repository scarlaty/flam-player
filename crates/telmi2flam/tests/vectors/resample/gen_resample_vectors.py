"""Vecteurs de parité du redimensionnement et des conversions en LIF (port Rust, issue #127).

L'oracle est `png_to_lif`, `png_to_scene_lif`, `png_to_fit_lif` et `png_to_thumb_lif` de
`tools/telmi2flam/telmi2flam.py`, exécutés avec **Python 3.14.8 et Pillow 12.3.0** exactement
(versions de `apps/flam-launcher/scripts/build-converter.ps1`) : le script refuse de tourner
avec une autre version. Déterministe : relancer le script ne doit rien changer (`git status`
propre).

    python -I crates/telmi2flam/tests/vectors/resample/gen_resample_vectors.py
    python -I .../gen_resample_vectors.py --matrix <dossier> [--big]

Sans argument : écrit ici des PNG de petite taille (cas limites : 1 px, rapports extrêmes,
images plus de 100 fois plus hautes que larges, tailles égales aux cibles ou juste au-dessus,
alpha partiel, bruit) et `manifest.tsv` : `nom  opération  attendu`, où `attendu` vaut
`lif:<octets>:<sha256>` ou `unreadable` (exception levée par l'oracle). Lu par
`tests/resample_vectors.rs`.

Avec `--matrix <dossier>` : la matrice de rééchantillonnage de
`tools/telmi2flam/tests/parity/make_corpus.py` (`write_matrix`, opérations `python_lif`),
écrite hors dépôt dans `<dossier>` (PNG et `manifest.tsv` au même format) ; tailles 1, 3, 97,
641, et 2049 seulement avec `--big`. Lue par le test ignoré `matrice_make_corpus`
(`TELMI2FLAM_RESAMPLE_MATRIX=<dossier>`).
"""

import hashlib
import io
import os
import random
import sys

PYTHON = (3, 14, 8)
PILLOW = "12.3.0"

HERE = os.path.dirname(os.path.abspath(__file__))
TOOLS = os.path.join(HERE, "..", "..", "..", "..", "..", "tools", "telmi2flam")
OPS = ("scene", "thumb", "inventory", "title", "bound")


def check_versions():
    import PIL
    if tuple(sys.version_info[:3]) != PYTHON or PIL.__version__ != PILLOW:
        sys.exit("oracle attendu : Python %s + Pillow %s (trouvé : Python %s, Pillow %s)"
                 % (".".join(map(str, PYTHON)), PILLOW, sys.version.split()[0], PIL.__version__))


def oracle(data, op):
    sys.path.insert(0, os.path.join(TOOLS, "tests", "parity"))
    sys.path.insert(0, os.path.join(TOOLS, "tests"))
    sys.path.insert(0, TOOLS)
    import make_corpus
    try:
        lif = make_corpus.python_lif(data, op)
    except Exception:
        return "unreadable"
    return "lif:%d:%s" % (len(lif), hashlib.sha256(lif).hexdigest())


def write(path, data):
    with open(path, "wb") as f:
        f.write(data)


def write_lines(path, lines):
    write(path, ("\n".join(lines) + "\n").encode("utf-8"))


def png_bytes(im):
    b = io.BytesIO()
    im.save(b, "PNG")
    return b.getvalue()


def content(w, h, kind, seed):
    """Image RGBA : bruit (alpha plein ou partiel), dégradé, ou alpha binaire."""
    from PIL import Image
    rng = random.Random("%s/%d/%d/%s" % (seed, w, h, kind))
    if kind in ("noise", "noise_alpha"):
        im = Image.frombytes("RGBA", (w, h), rng.randbytes(4 * w * h))
        if kind == "noise":
            im.putalpha(255)
        return im
    px = []
    for y in range(h):
        for x in range(w):
            r = x * 255 // max(1, w - 1)
            g = y * 255 // max(1, h - 1)
            if kind == "gradient":
                a = 255
            elif kind == "alpha":
                a = (x + y) * 255 // max(1, w + h - 2)
            else:  # "holes" : alpha 0 ou 255, couleur non nulle sous l'alpha 0
                a = 0 if (x * 7 + y * 3) % 5 == 0 else 255
            px.append(bytes((r, g, (r + g) // 2, a)))
    return Image.frombytes("RGBA", (w, h), b"".join(px))


def cases():
    """(nom, PNG) : petites images ciblant les chemins du port."""
    sizes = [
        (1, 1), (1, 2), (2, 1), (3, 3), (2, 3), (5, 4),
        # cibles exactes et voisines (vignette, inventaire, titre, scène)
        (128, 96), (129, 96), (128, 97), (127, 95), (128, 128), (89, 120), (90, 121),
        (320, 212), (321, 212), (320, 213), (160, 106),
        # rapports extrêmes, et plus de 100 fois plus haut que large (deux passes de resize)
        (1, 101), (1, 250), (2, 201), (2, 300), (3, 299), (300, 2), (250, 1), (401, 3),
        (1, 2049), (2049, 1), (3, 2049), (2049, 2), (2, 2048), (2048, 2), (1, 2047),
        # portraits autour du seuil de _avoid_transpose (h > 64)
        (64, 65), (10, 64), (65, 66), (40, 200),
        # agrandissements et réductions de plus de 2x
        (7, 5), (17, 13), (31, 23), (45, 67), (640, 480), (641, 481), (480, 640),
    ]
    out = []
    for w, h in sizes:
        # bruit (incompressible) seulement sur de petites images : dépôt léger
        kinds = ["gradient", "holes"] if w * h > 200000 else ["gradient", "alpha", "holes"]
        if w * h <= 1300:
            kinds += ["noise", "noise_alpha"]
        for kind in kinds:
            out.append(("%dx%d_%s" % (w, h, kind), png_bytes(content(w, h, kind, 127))))
    return out


def main_vectors():
    for f in os.listdir(HERE):
        if f.endswith(".png"):
            os.remove(os.path.join(HERE, f))
    lines = ["# nom\toperation\tattendu (genere par gen_resample_vectors.py, "
             "Python 3.14.8 + Pillow 12.3.0)"]
    for name, data in cases():
        write(os.path.join(HERE, name + ".png"), data)
        for op in OPS:
            lines.append("%s\t%s\t%s" % (name, op, oracle(data, op)))
    write_lines(os.path.join(HERE, "manifest.tsv"), lines)


def main_matrix(out_dir, big):
    sys.path.insert(0, os.path.join(TOOLS, "tests", "parity"))
    sys.path.insert(0, os.path.join(TOOLS, "tests"))
    sys.path.insert(0, TOOLS)
    import make_corpus
    sizes = [s for s in make_corpus.SIZES if big or s <= 641]
    entries = make_corpus.write_matrix(out_dir, sizes)
    lines = ["# matrice make_corpus.py : nom\toperation\tattendu"]
    cache = {}
    for e in entries:
        name = os.path.splitext(e["png"])[0]
        if name not in cache:
            with open(os.path.join(out_dir, e["png"]), "rb") as f:
                cache = {name: f.read()}
        lines.append("%s\t%s\t%s" % (name, e["op"], oracle(cache[name], e["op"])))
    write_lines(os.path.join(out_dir, "manifest.tsv"), lines)


if __name__ == "__main__":
    check_versions()
    if len(sys.argv) >= 3 and sys.argv[1] == "--matrix":
        main_matrix(sys.argv[2], "--big" in sys.argv[3:])
    else:
        main_vectors()
