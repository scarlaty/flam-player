"""Genere les vecteurs de parite de l'analyse des arguments (#133) : pour une suite d'arguments,
ce que fait `main()` de tools/telmi2flam/telmi2flam.py (oracle) : code de sortie, stdout,
stderr, et, si la conversion (ou --info) serait lancee, les arguments reçus.

Entree : aucune. Sortie (commitee) : crates/telmi2flam/tests/data/cli_cases.json, lue par
crates/telmi2flam/tests/cli_parity.rs.

L'oracle tourne dans le processus, avec `convert` et `read_info` remplaces par des
enregistreurs (aucun fichier lu) ; `sys.argv[0]` vaut `telmi2flam` pour que l'usage porte le
nom du binaire (c'est lui qui regle les retours a la ligne d'argparse). La version est
remplacee par `<version>`. L'option `--cancel-on-stdin-eof` (propre au port, absente de Python)
n'est pas dans les jeux : tout prefixe de `--c...` est donc exclu.

Les cas sont : une liste fixe (formes d'argparse notables), puis des suites tirees au hasard
(graine fixe) dans un lexique d'options, de valeurs et de faux amis.

A lancer avec le Python de l'oracle (3.14.8), depuis la racine du depot :
    python crates/telmi2flam/tests/data/gen_cli_cases.py
"""

import contextlib
import io
import json
import os
import random
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", "..", "..", ".."))
sys.path.insert(0, os.path.join(ROOT, "tools", "telmi2flam"))
import telmi2flam as T  # noqa: E402

OUT = os.path.join(HERE, "cli_cases.json")

FIXED = [
    [], ["-h"], ["--help"], ["--version"], ["--version", "x"], ["x", "--version"],
    ["--progress-json"], ["--bogus", "--progress-json"], ["x.zip", "y.zip"],
    ["x.zip", "--bogus"], ["x.zip", "-o"], ["x.zip", "-o", "--plain"],
    ["x.zip", "-o", "-1.pk"], ["x.zip", "-o", "-1"], ["x.zip", "-o", "-x y"],
    ["x.zip", "-o", "-"], ["x.zip", "--selector", "nope"], ["x.zip", "--selector"],
    ["x.zip", "--selector=image"], ["x.zip", "--thumb-fit", "x"],
    ["x.zip", "--thumb-fit=rogner"], ["x.zip", "--prog"], ["x.zip", "--p"],
    ["x.zip", "--pl"], ["x.zip", "--out=a.pk"], ["x.zip", "-oa.pk"], ["x.zip", "-o=a.pk"],
    ["x.zip", "--plain=1"], ["x.zip", "-pk"], ["x.zip", "-h"], ["--", "x.zip"],
    ["x.zip", "--", "-y"], ["-o", "a", "--", "x.zip"], ["-x"], ["-1"], ["--version=1"],
    ["x.zip", "--kee"], ["x.zip", "--a"], ["x.zip", "--info", "--plain"], ["--in"],
    ["x.zip", "-"], ["-"], ["-o"], ["--output"], ["x.zip", "-hx"], ["x.zip", "-o", "a", "-o", "b"],
    ["a b.zip"], ["x.zip", "--thumb"], ["x.zip", "--s=image"], [""], ["x.zip", ""],
    ["--keep-size", "--allow-missing", "--plain", "--info", "--progress-json", "x.zip"],
    ["x.zip", "--thumb-fit", "bandes", "--selector", "image", "-o", "dir.plain"],
]

LEXICON = [
    "x.zip", "y.zip", "a b.zip", "", "-o", "--output", "--output=a", "-oa", "-o=a",
    "--plain", "--keep-size", "--allow-missing", "--selector", "--selector=image",
    "carousel", "image", "nope", "--thumb-fit", "flou", "rogner", "bandes",
    "--progress-json", "--info", "-h", "--help", "--version", "--", "-", "-1", "-1.pk",
    "-1.5", "-x y", "--bogus", "-x", "--p", "--pl", "--prog", "--k", "--a", "--al", "--s",
    "--se", "--t", "--i", "--in", "--o", "--out", "-pk", "--plain=1", "-hx", "-ho", "-oh",
    "--selector=", "--thumb-fit=bandes", "--info=1", "--he", "--ver", "--v", "--vers=2",
]


class Rec(Exception):
    pass


def run(argv):
    seen = {}

    def convert(inp, out, keep_size, plain, selector, allow_missing, progress=None,
                thumb_fit=None):
        seen.update(input=inp, output=out, keep_size=keep_size, plain=plain,
                    selector=selector, allow_missing=allow_missing,
                    progress_json=progress is not None, info=False, thumb_fit=thumb_fit)

    def read_info(inp):
        seen.update(input=inp, info=True)
        return {}

    real = (T.convert, T.read_info)
    T.convert, T.read_info = convert, read_info
    T._stdout_dead[0] = False
    out, err = io.StringIO(), io.StringIO()
    old_argv0 = sys.argv[0]
    sys.argv[0] = "telmi2flam"
    try:
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            try:
                code = T.main(list(argv))
            except SystemExit as e:
                code = e.code
    finally:
        sys.argv[0] = old_argv0
        T.convert, T.read_info = real
    if seen.get("info"):
        seen = {"input": seen["input"], "info": True}
        # `--info` : seuls l'archive et le mode comptent pour l'analyse
    stdout = out.getvalue().replace(T.__version__, "<version>")
    return {"argv": list(argv), "exit": code, "stdout": stdout, "stderr": err.getvalue(),
            "run": seen or None}


def main():
    rng = random.Random(133)
    argvs = [list(a) for a in FIXED]
    seen = {json.dumps(a) for a in argvs}
    while len(argvs) < 900:
        a = [rng.choice(LEXICON) for _ in range(rng.randint(0, 5))]
        k = json.dumps(a)
        if k not in seen:
            seen.add(k)
            argvs.append(a)
    cases = [run(a) for a in argvs]
    # L'usage et l'aide, repetes dans la plupart des cas, sont factorises : `<usage>` et
    # `<help>` dans stdout/stderr (voir cli_parity.rs).
    usage = next(c for c in cases if c["argv"] == [])["stderr"].split("telmi2flam: error")[0]
    helped = next(c for c in cases if c["argv"] == ["-h"])["stdout"]
    assert helped.startswith(usage)
    help_body = helped[len(usage):]
    for c in cases:
        for key in ("stdout", "stderr"):
            c[key] = c[key].replace(usage + help_body, "<usage><help>").replace(usage, "<usage>")
    doc = {"usage": usage, "help": help_body, "cases": cases}
    with open(OUT, "w", encoding="utf-8", newline="\n") as f:
        json.dump(doc, f, ensure_ascii=True, separators=(",", ":"))
        f.write("\n")
    print(len(cases), "cas ->", OUT)


if __name__ == "__main__":
    main()
