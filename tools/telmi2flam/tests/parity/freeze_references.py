"""
Gel des references de parite (#140, PORT-RUST.md §4.5) : script a usage unique, lance une
derniere fois avec l'ancien oracle Python avant sa suppression, puis supprime du depot.

    python freeze_references.py --oracle-src ARBRE --revision SHA

ARBRE : copie (git archive) d'une revision qui contient encore l'oracle
(tools/telmi2flam/telmi2flam.py, tests/parity/make_corpus.py, tests/test_telmi2flam.py).
A lancer avec Python 3.14.8 et Pillow 12.3.0 (ORACLE-VERSIONS.txt), sous Windows.

Ecrit dans le depot courant :
  - crates/telmi2flam/tests/data/parity_corpus.zip : corpus synthetique (archives de
    make_corpus.py), index corpus.json, resultat normalise de l'oracle par cas
    (expected.json, forme de parity.run_converter) et contenu des entrees issues d'un JPEG
    (blobs/<sha256>, pour la regle jpeg_bounded) ;
  - crates/telmi2flam/tests/data/matrix_expected.json : SHA-256 de chaque PNG de
    make_matrix.py et du .lif de l'oracle pour chaque cellule.
"""
import argparse
import hashlib
import io
import json
import os
import shutil
import sys
import tempfile
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.dirname(os.path.dirname(HERE))
REPO = os.path.dirname(os.path.dirname(TOOL))
DATA = os.path.join(REPO, "crates", "telmi2flam", "tests", "data")
sys.dont_write_bytecode = True
sys.path.insert(0, HERE)
sys.path.insert(0, TOOL)

import make_matrix  # noqa: E402
import parity  # noqa: E402

FIXED = (1980, 1, 1, 0, 0, 0)


def _sha(data):
    return hashlib.sha256(data).hexdigest()


def _zip(path, files):
    """ZIP deterministe (ordre donne, date fixe, deflate niveau 9)."""
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as z:
        for name, data in files:
            zi = zipfile.ZipInfo(name, FIXED)
            zi.compress_type = zipfile.ZIP_DEFLATED
            zi.create_system = 0
            zi.external_attr = 0
            z.writestr(zi, data, compresslevel=9)
    with open(path, "wb") as f:
        f.write(buf.getvalue())


def _json(obj):
    return (json.dumps(obj, indent=1, sort_keys=True, ensure_ascii=True) + "\n").encode("ascii")


def freeze_corpus(oracle_src, frozen_with):
    tool = os.path.join(oracle_src, "tools", "telmi2flam")
    sys.path[:0] = [os.path.join(tool, "tests", "parity"), os.path.join(tool, "tests"), tool]
    import make_corpus as mc
    oracle = [sys.executable, os.path.join(tool, "telmi2flam.py")]
    tmp = tempfile.mkdtemp(prefix="t2f_freeze_")
    try:
        index = mc.write_corpus(tmp)
        expected, blobs = {}, {}
        for e in index:
            jpeg = e.get("jpeg_entries", ())
            run, _ = parity.run_converter(oracle, os.path.join(tmp, e["archive"]),
                                          os.path.join(tmp, "work", e["id"]), e["args"],
                                          keep=jpeg)
            if run["error_code"] == "harness_error":
                raise SystemExit("oracle inutilisable sur %s : %s" % (e["id"], run["info"]))
            for key in ("entries", "plain_entries"):
                for entry in run.get(key) or ():
                    data = entry.pop("data", None)
                    if data is not None:
                        blobs[entry["sha256"]] = data
            expected[e["id"]] = run
        files = [("corpus.json", _json(index)),
                 ("expected.json", _json({"schema": parity.REFERENCES_SCHEMA,
                                          "frozen_with": frozen_with, "cases": expected}))]
        for e in index:
            with open(os.path.join(tmp, e["archive"]), "rb") as f:
                files.append((e["archive"], f.read()))
        files += [("blobs/" + sha, blobs[sha]) for sha in sorted(blobs)]
        _zip(os.path.join(DATA, "parity_corpus.zip"), files)
        return len(index)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def freeze_matrix(oracle_src, frozen_with):
    sys.path.insert(0, os.path.join(oracle_src, "tools", "telmi2flam"))
    import telmi2flam as T

    def lif(png, op):
        if op == "scene":
            return T.png_to_scene_lif(png)
        if op == "thumb":
            return T.png_to_thumb_lif(png, "bandes")
        if op == "inventory":
            return T.png_to_fit_lif(png, T.INV_W, T.INV_H)
        if op == "title":
            return T.png_to_fit_lif(png, 89, 120, align="left")
        if op == "bound":
            return T.png_to_lif(png, None)
        raise ValueError(op)

    tmp = tempfile.mkdtemp(prefix="t2f_freeze_matrix_")
    try:
        entries = make_matrix.write_matrix(tmp)
        pngs, lifs = {}, {}
        for e in entries:
            with open(os.path.join(tmp, e["png"]), "rb") as f:
                png = f.read()
            pngs[os.path.splitext(os.path.basename(e["png"]))[0]] = _sha(png)
            lifs[e["id"]] = _sha(lif(png, e["op"]))
        doc = {"schema": 1, "frozen_with": frozen_with,
               "generator": "tools/telmi2flam/tests/parity/make_matrix.py",
               "pngs": pngs, "lifs": lifs}
        with open(os.path.join(DATA, "matrix_expected.json"), "wb") as f:
            f.write(_json(doc))
        return len(lifs)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--oracle-src", required=True)
    ap.add_argument("--revision", required=True, help="revision de l'oracle (SHA du commit)")
    ap.add_argument("--only", choices=["corpus", "matrix"])
    args = ap.parse_args()
    from PIL import Image
    if sys.platform != "win32" or sys.version_info[:3] != (3, 14, 8) \
            or Image.__version__ != "12.3.0":
        raise SystemExit("gel a faire sous Windows avec Python 3.14.8 + Pillow 12.3.0")
    frozen_with = {"python": "3.14.8", "pillow": "12.3.0", "revision": args.revision}
    if args.only != "matrix":
        print("corpus : %d cas figes" % freeze_corpus(os.path.abspath(args.oracle_src),
                                                      frozen_with))
    if args.only != "corpus":
        print("matrice : %d cellules figees" % freeze_matrix(os.path.abspath(args.oracle_src),
                                                             frozen_with))


if __name__ == "__main__":
    main()
