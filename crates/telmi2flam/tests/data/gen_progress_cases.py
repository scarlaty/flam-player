"""Genere les vecteurs de parite du protocole (#131) : evenements --progress-json, lignes
"  ! ..." de stderr et codes de sortie de tools/telmi2flam/telmi2flam.py, qui sert d'oracle.

Entree : les archives de crates/telmi2flam/tests/data/package_cases.zip (cases/<id>.zip et
leurs options dans expected.json, voir gen_package_cases.py).
Sortie (commitee) : crates/telmi2flam/tests/data/progress_cases.json, lue par
crates/telmi2flam/tests/progress_parity.rs. Pour chaque execution du CLI :
- `id`, `case` (archive de package_cases.zip, ou null), `args` (arguments apres l'archive ;
  `{in}` et `{out}` designent l'archive et la sortie) ;
- `exit` : code de sortie ;
- `events` : les evenements de stdout, normalises (voir `normalize`), ou null sans
  `--progress-json` ;
- `bangs` : les lignes de stderr qui commencent par "  ! ", sans ce prefixe.

Normalisation (la meme dans progress_parity.rs) : `input`, `output` et `version` remplaces
par "<input>", "<output>", "<version>", `message` de `error` retire ; seules les etapes `done == 0` ou `done == total`
sont gardees (les intermediaires dependent de l'horloge, espaces de 0,1 s).

Le CLI est lance comme le launcher : `--progress-json --thumb-fit bandes` (mode des vecteurs
de package_cases.zip), sans ffmpeg (FLAM_FFMPEG vide).

A lancer avec le Python de l'installateur (3.14.8, Pillow 12.3.0), depuis la racine du depot :
    python crates/telmi2flam/tests/data/gen_progress_cases.py
"""

import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", "..", "..", ".."))
CLI = os.path.join(ROOT, "tools", "telmi2flam", "telmi2flam.py")
CASES = os.path.join(HERE, "package_cases.zip")
OUT = os.path.join(HERE, "progress_cases.json")


_ADDR = re.compile(r" at 0x[0-9A-Fa-f]+>")


def stable(text):
    """Adresse memoire d'un objet Python (`<_io.BytesIO object at 0x...>`) neutralisee."""
    return _ADDR.sub(" at 0x...>", text)


def normalize(ev):
    ev = dict(ev)
    kind = ev.get("event")
    if kind == "warning":
        ev["message"] = stable(ev["message"])
    if kind == "start":
        ev["input"] = "<input>"
        ev["version"] = "<version>"
    elif kind == "done":
        ev["output"] = "<output>"
    elif kind == "error":
        del ev["message"]                  # texte propre a chaque moteur (chemins temporaires)
    elif kind == "step" and ev["done"] not in (0, ev["total"]):
        return None
    return ev


def options(case):
    args = []
    if case["keep_size"]:
        args.append("--keep-size")
    if case["allow_missing"]:
        args.append("--allow-missing")
    if case["selector"] != "carousel":
        args += ["--selector", case["selector"]]
    return args


def run(rid, case_id, data, args, tmp):
    work = os.path.join(tmp, rid)
    os.makedirs(work)
    src = os.path.join(work, "in.zip")
    if data is not None:
        with open(src, "wb") as f:
            f.write(data)
    out = os.path.join(work, "out.plain.pk")
    argv = [a.replace("{in}", src).replace("{out}", out) for a in args]
    env = {**os.environ, "FLAM_FFMPEG": ""}
    r = subprocess.run([sys.executable, "-X", "utf8", CLI] + argv, capture_output=True,
                       cwd=work, env=env)
    events = None                  # sans --progress-json, stdout porte le resume humain
    if "--progress-json" in args:
        events = []
        for line in r.stdout.decode("ascii").splitlines():
            ev = normalize(json.loads(line))   # stdout ne contient que des evenements
            if ev is not None:
                events.append(ev)
    bangs = [stable(l[4:]) for l in r.stderr.decode("utf-8").splitlines() if l.startswith("  ! ")]
    return {"id": rid, "case": case_id, "args": args, "exit": r.returncode,
            "events": events, "bangs": bangs}


def main():
    if sys.version_info[:3] != (3, 14, 8):
        sys.exit("vecteurs a produire avec Python 3.14.8 (oracle de build-converter.ps1)")
    base = ["{in}", "-o", "{out}", "--progress-json", "--thumb-fit", "bandes"]
    runs = []
    tmp = tempfile.mkdtemp(prefix="progress_cases_")
    try:
        with zipfile.ZipFile(CASES) as z:
            cases = json.loads(z.read("expected.json"))["cases"]
            data = {c["id"]: z.read("cases/%s.zip" % c["id"]) for c in cases}
        for c in cases:
            runs.append(run(c["id"], c["id"], data[c["id"]], base + options(c), tmp))
        # Phase zip doublee (--plain) ou dossier seul (-o *.plain).
        for cid in ("missing_image_allowed", "pkg_name_collisions"):
            c = next(c for c in cases if c["id"] == cid)
            runs.append(run(cid + "+plain", cid, data[cid], base + options(c) + ["--plain"], tmp))
            runs.append(run(cid + "+dir", cid, data[cid],
                            ["{in}", "-o", "{out}.plain", "--progress-json", "--thumb-fit",
                             "bandes"] + options(c), tmp))
        # Sans --progress-json : aucun evenement, memes lignes "  !".
        runs.append(run("missing_image_allowed+human", "missing_image_allowed",
                        data["missing_image_allowed"],
                        ["{in}", "-o", "{out}", "--thumb-fit", "bandes", "--allow-missing"], tmp))
        # Archive absente (start puis error), arguments invalides (error seul).
        runs.append(run("absent", None, None, base, tmp))
        runs.append(run("bad_selector", None, None, base + ["--selector", "nope"], tmp))
        runs.append(run("no_input", None, None, ["--progress-json"], tmp))
        runs.append(run("info_absent", None, None, ["{in}", "--info", "--progress-json"], tmp))
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    body = "[\n%s\n]\n" % ",\n".join(json.dumps(r, ensure_ascii=True, separators=(",", ":"))
                                    for r in runs)
    with open(OUT + ".part", "w", encoding="ascii", newline="\n") as f:
        f.write(body)
    os.replace(OUT + ".part", OUT)
    print("%d executions ecrites dans %s" % (len(runs), OUT))


if __name__ == "__main__":
    main()
