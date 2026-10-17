#!/usr/bin/env python3
"""
Outil de parite pour le port Rust de telmi2flam (#123, note PORT-RUST.md §4).

Compare un convertisseur candidat (le port Rust) a l'oracle (telmi2flam.py, Pillow
epingle). Ce qui est compare, par archive :

  - le statut et le code de sortie ;
  - le code de l'evenement `error` (--progress-json) ;
  - le MULTIENSEMBLE des codes d'avertissement (pas le texte) ;
  - la sortie brute de `--info` (JSON, ensure_ascii, separateurs compacts) ;
  - la LISTE ORDONNEE des entrees du `.plain.pk` : nom, methode, drapeau UTF-8 (bit 11)
    et contenu octet a octet. Pas les octets du conteneur (horodatage, voir §4.1) ;
  - avec `--plain`, les fichiers du dossier `.plain` (chemin, contenu) ;
  - ce qui reste dans le dossier de sortie : aucun `.part` (meme apres un echec), et pas
    de sortie finale quand la conversion echoue.

Les ecarts admis forment une liste fermee (`ALLOWED_DEVIATIONS`, §4.1) ; tout autre
ecart fait echouer la comparaison (code de sortie 1). Les delegations
`unsupported_format` (code 7, decision D4) sont comptees a part.

    python parity.py corpus DOSSIER [--oracle CMD] [--candidate CMD] [--report FICHIER]
    python parity.py matrix DOSSIER [--candidate-lif-dir DOSSIER] [--report FICHIER]
    python parity.py compare SORTIE_A SORTIE_B [--report FICHIER]

`corpus` et `matrix` lisent les index `corpus.json` / `matrix.json` produits par
tests/parity/make_corpus.py. Sans `--candidate`, le candidat est l'oracle : Python
contre Python, zero ecart attendu. CMD est une ligne de commande (separee comme un
shell, chemins avec des /), suivie de `archive.zip -o sortie --progress-json ...`.

`validate.py` ne s'applique qu'a la sortie du CANDIDAT : la sortie de l'oracle n'y passe
pas (elle sert de reference, un paquet que `validate.py` refuserait serait un ecart de
l'oracle a traiter a part, pas un ecart de parite).
"""
import argparse
import contextlib
import importlib.util
import hashlib
import io
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import zipfile
from collections import Counter

HERE = os.path.dirname(os.path.abspath(__file__))
sys.dont_write_bytecode = True

REPORT_SCHEMA = 1
ORACLE_SCRIPT = os.path.join(HERE, "telmi2flam.py")

# Etiquettes de cas (voir tests/parity/make_corpus.py)
TAG_JSON_LIMIT = "json_limit"
TAG_ENCRYPTED = "encrypted_entry"
TAG_BAD_UTF8_NAME = "invalid_utf8_name"
TAG_NEGATIVE_OFFSET = "negative_offset"
TAG_DELEGATED = "delegated"

DELEGATION_CODE = "unsupported_format"
DELEGATION_EXIT = 7


# ---------------------------------------------------------------------------
# Resultat d'une execution
# ---------------------------------------------------------------------------
def _sha(data):
    return hashlib.sha256(data).hexdigest()


def entries_from_pk(path):
    """Liste ordonnee des entrees d'un `.plain.pk` : nom, methode, drapeau UTF-8, taille,
    SHA-256 du contenu. Doublons conserves (infolist, pas namelist)."""
    out = []
    with zipfile.ZipFile(path) as z:
        for zi in z.infolist():
            out.append({"name": zi.filename, "method": zi.compress_type,
                        "utf8": bool(zi.flag_bits & 0x800), "size": zi.file_size,
                        "sha256": _sha(z.read(zi))})
    return out


def entries_from_dir(path):
    """Entrees d'un dossier `.plain` : chemins relatifs en `/`, tries ; methode et
    drapeau UTF-8 sans objet (None)."""
    out = []
    for dp, dn, fns in os.walk(path):
        dn.sort()
        for fn in sorted(fns):
            full = os.path.join(dp, fn)
            with open(full, "rb") as f:
                data = f.read()
            out.append({"name": os.path.relpath(full, path).replace(os.sep, "/"),
                        "method": None, "utf8": None, "size": len(data), "sha256": _sha(data)})
    out.sort(key=lambda e: e["name"])
    return out


def entries_from(path):
    return entries_from_dir(path) if os.path.isdir(path) else entries_from_pk(path)


def parse_events(stdout):
    """Evenements JSON d'un flux --progress-json (les lignes non JSON sont ignorees)."""
    events = []
    for line in stdout.splitlines():
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            ev = json.loads(line)
        except ValueError:
            continue
        if isinstance(ev, dict) and "event" in ev:
            events.append(ev)
    return events


_PART_RE = re.compile(r"\.\d+\.part$")


def work_files(work_dir):
    """Contenu du dossier de sortie apres l'execution, pid des `.part` neutralise (il
    differe d'un moteur a l'autre). Trie."""
    return sorted(_PART_RE.sub(".<pid>.part", n) for n in os.listdir(work_dir))


def run_to_dict(exit_code, events, info_exit, info_stdout, entries, plain_entries=None,
                files=None):
    """Forme normalisee d'une execution (serialisable en JSON)."""
    err = next((e for e in events if e.get("event") == "error"), None)
    return {
        "status": "ok" if exit_code == 0 else "error",
        "exit": exit_code,
        "error_code": err.get("code") if err else None,
        "warnings": sorted(e.get("code") or "" for e in events if e.get("event") == "warning"),
        "info_exit": info_exit,
        "info": info_stdout,
        "entries": entries,
        "plain_entries": plain_entries,
        "files": files,
    }


def harness_error(exc):
    """Execution impossible (commande absente, delai depasse) : un ecart sur ce cas, sans
    interrompre le run ni le rapport."""
    return run_to_dict(None, [{"event": "error", "code": "harness_error"}], None,
                       "%s: %s" % (type(exc).__name__, exc), None)


def run_converter(cmd, archive, work_dir, args=(), timeout=300):
    """Execute `cmd archive -o sortie --progress-json args` puis `cmd archive --info`.
    Retourne la forme normalisee (voir run_to_dict) et le chemin de la sortie (ou None)."""
    os.makedirs(work_dir, exist_ok=True)
    out = os.path.join(work_dir, "out.plain.pk")
    try:
        r = subprocess.run(list(cmd) + [archive, "-o", out, "--progress-json"] + list(args),
                           capture_output=True, text=True, encoding="utf-8", errors="replace",
                           timeout=timeout)
    except (OSError, subprocess.TimeoutExpired) as e:
        return harness_error(e), None
    events = parse_events(r.stdout)
    entries = None
    plain_entries = None
    produced = None
    if r.returncode == 0 and os.path.exists(out):
        entries = entries_from_pk(out)
        produced = out
        plain_dir = out[:-len(".pk")]            # ".plain.pk" -> ".plain"
        if os.path.isdir(plain_dir):
            plain_entries = entries_from_dir(plain_dir)
    files = work_files(work_dir)
    try:
        ri = subprocess.run(list(cmd) + [archive, "--info"],
                            capture_output=True, text=True, encoding="utf-8", errors="replace",
                            timeout=timeout)
    except (OSError, subprocess.TimeoutExpired) as e:
        return harness_error(e), None
    # --info sans --progress-json : en cas d'erreur, seul le code de sortie compte (le texte
    # du message differe entre moteurs, §4.1) ; la ligne JSON n'est comparee qu'en succes.
    info = ri.stdout.strip() if ri.returncode == 0 else None
    return (run_to_dict(r.returncode, events, ri.returncode, info, entries, plain_entries, files),
            produced)


# ---------------------------------------------------------------------------
# Liste fermee des ecarts admis (§4.1)
# ---------------------------------------------------------------------------
class Deviation:
    """Un ecart admis : s'applique seulement aux cas portant `tag` (sauf `tag` None), et
    seulement si les deux resultats correspondent exactement au scenario decrit."""

    def __init__(self, rule_id, ref, description, tag, matches):
        self.id = rule_id
        self.ref = ref
        self.description = description
        self.tag = tag
        self.matches = matches


def _pair(run):
    return (run["exit"], run["error_code"])


ALLOWED_DEVIATIONS = (
    Deviation(
        "encrypted_entry", "§4.1 / §3",
        "entree chiffree : le Rust rend archive_unreadable (code 3), Python internal_error (6)",
        TAG_ENCRYPTED,
        lambda o, c: _pair(o) == (6, "internal_error") and _pair(c) == (3, "archive_unreadable")),
    Deviation(
        "invalid_utf8_name", "§4.1 / §3",
        "nom d'entree UTF-8 invalide avec le bit 11 : le Rust rend archive_unreadable "
        "(code 3), Python internal_error (6, UnicodeDecodeError)",
        TAG_BAD_UTF8_NAME,
        lambda o, c: _pair(o) == (6, "internal_error") and _pair(c) == (3, "archive_unreadable")),
    Deviation(
        "negative_offset", "§4.1 / §3",
        "position d'en-tete local negative : le Rust rend archive_unreadable (code 3), "
        "Python internal_error (6, OSError)",
        TAG_NEGATIVE_OFFSET,
        lambda o, c: _pair(o) == (6, "internal_error") and _pair(c) == (3, "archive_unreadable")),
    Deviation(
        "json_limit", "§4.1 / §3",
        "JSON limite (flottant hors plage, entier hors 64 bits, imbrication profonde, "
        "surrogate isole) : le Rust rend invalid_content (code 4) ; Python accepte (0) ou "
        "echoue en internal_error (6)",
        TAG_JSON_LIMIT,
        lambda o, c: _pair(c) == (4, "invalid_content")
        and (o["exit"] == 0 or _pair(o) == (6, "internal_error"))),
)


def is_delegation(candidate, tags=()):
    """Delegation D4 : comptee a part, jamais un ecart. Admise seulement sur les cas
    etiquetes `delegated` (image non PNG, entree bzip2/LZMA) : un candidat qui delegue
    autre chose est en ecart."""
    return TAG_DELEGATED in tags and _pair(candidate) == (DELEGATION_EXIT, DELEGATION_CODE)


# ---------------------------------------------------------------------------
# Comparaison
# ---------------------------------------------------------------------------
def compare_entries(a, b):
    """Ecarts entre deux listes d'entrees (None : pas de sortie)."""
    if a is None and b is None:
        return []
    if a is None or b is None:
        return [{"kind": "entries", "detail": "sortie presente d'un seul cote",
                 "oracle": None if a is None else len(a),
                 "candidate": None if b is None else len(b)}]
    diffs = []
    if [e["name"] for e in a] != [e["name"] for e in b]:
        diffs.append({"kind": "entries", "detail": "noms ou ordre differents",
                      "oracle": [e["name"] for e in a], "candidate": [e["name"] for e in b]})
    for i, (x, y) in enumerate(zip(a, b)):
        for field in ("name", "method", "utf8", "size", "sha256"):
            if x[field] != y[field]:
                diffs.append({"kind": "entry", "index": i, "name": x["name"], "field": field,
                              "oracle": x[field], "candidate": y[field]})
    return diffs


def stray_files(run):
    """Fichiers qui ne devraient pas exister dans le dossier de sortie : un `.part` (meme
    apres un echec), une sortie finale quand la conversion echoue. Verifie sur le candidat
    pour TOUS les verdicts, y compris les ecarts admis et les delegations."""
    names = run.get("files")
    if names is None:
        return []
    diffs = []
    parts = [n for n in names if n.endswith(".part")]
    if parts:
        diffs.append({"kind": "part", "oracle": None, "candidate": parts})
    if run["exit"] != 0 and any(n.startswith("out.plain") and not n.endswith(".part")
                                for n in names):
        diffs.append({"kind": "output_on_failure", "oracle": None, "candidate": names})
    return diffs


def compare_runs(oracle, candidate, tags=()):
    """Compare deux resultats normalises. Retourne
    {"verdict": identical|allowed|delegated|diff, "rule": id|None, "diffs": [...]}."""
    stray = stray_files(candidate)
    if stray:
        return {"verdict": "diff", "rule": None, "diffs": stray}
    if is_delegation(candidate, tags):
        return {"verdict": "delegated", "rule": None, "diffs": []}
    for rule in ALLOWED_DEVIATIONS:
        if rule.tag in tags and rule.matches(oracle, candidate):
            return {"verdict": "allowed", "rule": rule.id, "diffs": []}
    diffs = []
    for key in ("status", "exit", "error_code", "warnings", "info_exit", "info"):
        if oracle[key] != candidate[key]:
            diffs.append({"kind": key, "oracle": oracle[key], "candidate": candidate[key]})
    diffs += compare_entries(oracle["entries"], candidate["entries"])
    diffs += [dict(d, kind="plain_" + d["kind"])
              for d in compare_entries(oracle.get("plain_entries"), candidate.get("plain_entries"))]
    if oracle.get("files") != candidate.get("files"):
        diffs.append({"kind": "files", "oracle": oracle.get("files"),
                      "candidate": candidate.get("files")})
    if "validation" in candidate and candidate["validation"] not in (None, 0):
        diffs.append({"kind": "validate", "oracle": None, "candidate": candidate["validation"]})
    return {"verdict": "diff" if diffs else "identical", "rule": None, "diffs": diffs}


def compare_lif_dirs(oracle_dir, candidate_dir):
    """Compare deux dossiers de `.lif` (matrice de reechantillonnage), octet a octet.
    Retourne la liste de resultats par fichier (les fichiers sont apparies par nom)."""
    def names(d):
        return {n for n in os.listdir(d) if n.endswith(".lif")}
    a, b = names(oracle_dir), names(candidate_dir)
    results = []
    for n in sorted(a | b):
        if n not in b:
            results.append({"id": n[:-4], "verdict": "diff", "rule": None, "diffs": [
                {"kind": "lif", "detail": "absent du candidat"}]})
            continue
        if n not in a:
            results.append({"id": n[:-4], "verdict": "diff", "rule": None, "diffs": [
                {"kind": "lif", "detail": "absent de l'oracle"}]})
            continue
        with open(os.path.join(oracle_dir, n), "rb") as f:
            x = f.read()
        with open(os.path.join(candidate_dir, n), "rb") as f:
            y = f.read()
        if x == y:
            results.append({"id": n[:-4], "verdict": "identical", "rule": None, "diffs": []})
        else:
            first = next((i for i, (p, q) in enumerate(zip(x, y)) if p != q), min(len(x), len(y)))
            results.append({"id": n[:-4], "verdict": "diff", "rule": None, "diffs": [
                {"kind": "lif", "detail": "octets differents", "first_difference": first,
                 "oracle_size": len(x), "candidate_size": len(y),
                 "oracle_sha256": _sha(x), "candidate_sha256": _sha(y)}]})
    return results


def validate_output(pk_path):
    """Code de retour de validate.py sur une sortie (0 : valide)."""
    sys.path.insert(0, HERE)
    try:
        import validate
        with contextlib.redirect_stdout(io.StringIO()):
            return validate.main(pk_path)
    finally:
        sys.path.remove(HERE)


# ---------------------------------------------------------------------------
# Rapport
# ---------------------------------------------------------------------------
def summarize(cases):
    counts = Counter(c["verdict"] for c in cases)
    return {"total": len(cases),
            "identical": counts["identical"], "allowed": counts["allowed"],
            "delegated": counts["delegated"], "diff": counts["diff"]}


def build_report(cases, kind):
    allowed = Counter(c["rule"] for c in cases if c["verdict"] == "allowed")
    return {"schema": REPORT_SCHEMA, "kind": kind, "summary": summarize(cases),
            "allowed_deviations": dict(sorted(allowed.items())),
            "delegations": sorted(c["id"] for c in cases if c["verdict"] == "delegated"),
            "cases": cases}


def write_report(path, report):
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        json.dump(report, f, indent=1, sort_keys=True, ensure_ascii=True)
        f.write("\n")


def exit_code(report):
    # un run vide (index vide) ne prouve rien : echec
    return 1 if report["summary"]["diff"] or not report["summary"]["total"] else 0


# ---------------------------------------------------------------------------
# Modes
# ---------------------------------------------------------------------------
def split_cmd(text):
    return shlex.split(text, posix=True)


def default_oracle():
    return [sys.executable, ORACLE_SCRIPT]


def run_corpus(corpus_dir, oracle, candidate, validate=True, keep_work=None, progress=None):
    """Compare chaque archive de corpus.json. Retourne la liste des resultats."""
    with open(os.path.join(corpus_dir, "corpus.json"), encoding="utf-8") as f:
        entries = json.load(f)
    work = keep_work or tempfile.mkdtemp(prefix="t2f_parity_")
    results = []
    try:
        for e in entries:
            archive = os.path.join(corpus_dir, e["archive"])
            o, _ = run_converter(oracle, archive, os.path.join(work, "oracle", e["id"]), e["args"])
            c, produced = run_converter(candidate, archive,
                                        os.path.join(work, "candidate", e["id"]), e["args"])
            if validate and produced is not None:
                c["validation"] = validate_output(produced)
            res = compare_runs(o, c, e.get("tags", ()))
            res["id"] = e["id"]
            results.append(res)
            if progress:
                progress(res)
    finally:
        if keep_work is None:
            shutil.rmtree(work, ignore_errors=True)
    return results


def load_make_corpus():
    """Charge tests/parity/make_corpus.py par son chemin (il tire Pillow et les generateurs
    des tests : seul le mode `matrix` en a besoin, `corpus` n'importe rien de tout cela)."""
    path = os.path.join(HERE, "tests", "parity", "make_corpus.py")
    spec = importlib.util.spec_from_file_location("make_corpus", path)
    module = importlib.util.module_from_spec(spec)
    sys.modules["make_corpus"] = module
    spec.loader.exec_module(module)
    return module


def run_matrix(matrix_dir, candidate_lif_dir=None):
    """Compare les `.lif` de la matrice : oracle Python contre `candidate_lif_dir`
    (sans dossier : Python rendu une seconde fois, controle de determinisme)."""
    make_corpus = load_make_corpus()
    tmp = tempfile.mkdtemp(prefix="t2f_matrix_")
    try:
        oracle_dir = os.path.join(tmp, "oracle")
        make_corpus.render_matrix_python(matrix_dir, oracle_dir)
        if candidate_lif_dir is None:
            candidate_lif_dir = os.path.join(tmp, "candidate")
            make_corpus.render_matrix_python(matrix_dir, candidate_lif_dir)
        return compare_lif_dirs(oracle_dir, candidate_lif_dir)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def run_compare(a, b):
    """Deux sorties (.plain.pk ou dossiers .plain) : liste des entrees uniquement."""
    diffs = compare_entries(entries_from(a), entries_from(b))
    return [{"id": "%s <> %s" % (os.path.basename(a), os.path.basename(b)),
             "verdict": "diff" if diffs else "identical", "rule": None, "diffs": diffs}]


def main(argv=None):
    ap = argparse.ArgumentParser(description="Parite Python / port Rust de telmi2flam")
    sub = ap.add_subparsers(dest="mode", required=True)
    p = sub.add_parser("corpus", help="comparer un convertisseur candidat a l'oracle")
    p.add_argument("dir", help="dossier produit par make_corpus.py")
    p.add_argument("--oracle", help="commande de l'oracle (defaut : telmi2flam.py)")
    p.add_argument("--candidate", help="commande du candidat (defaut : l'oracle)")
    p.add_argument("--no-validate", action="store_true",
                   help="ne pas passer validate.py sur la sortie du candidat")
    p.add_argument("--report", help="rapport JSON")
    p = sub.add_parser("matrix", help="comparer les .lif de la matrice, octet a octet")
    p.add_argument("dir", help="dossier produit par make_corpus.py")
    p.add_argument("--candidate-lif-dir", help="dossier des <id>.lif du candidat")
    p.add_argument("--report", help="rapport JSON")
    p = sub.add_parser("compare", help="comparer deux sorties (.plain.pk ou dossiers)")
    p.add_argument("a")
    p.add_argument("b")
    p.add_argument("--report", help="rapport JSON")
    args = ap.parse_args(argv)

    if args.mode == "corpus":
        oracle = split_cmd(args.oracle) if args.oracle else default_oracle()
        candidate = split_cmd(args.candidate) if args.candidate else oracle
        cases = run_corpus(args.dir, oracle, candidate, validate=not args.no_validate)
    elif args.mode == "matrix":
        cases = run_matrix(args.dir, args.candidate_lif_dir)
    else:
        cases = run_compare(args.a, args.b)
    report = build_report(cases, args.mode)
    if args.report:
        write_report(args.report, report)
    s = report["summary"]
    print("parite (%s) : %d cas, %d identiques, %d admis, %d delegues, %d ecarts"
          % (args.mode, s["total"], s["identical"], s["allowed"], s["delegated"], s["diff"]))
    for c in cases:
        if c["verdict"] == "diff":
            print("  ECART %s : %s" % (c["id"], ", ".join(sorted({d["kind"] for d in c["diffs"]}))))
    return exit_code(report)


if __name__ == "__main__":
    sys.exit(main())
