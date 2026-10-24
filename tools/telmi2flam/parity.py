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
    python parity.py campaign ARCHIVES --candidate CMD [--oracle CMD] [--out DOSSIER]
                              [--jobs N] [--timeout S] [--limit N] [--force]

`campaign` (#135, §4.3) compare les deux moteurs sur les archives reelles, par lots avec
reprise (un etat par archive sous DOSSIER/cases/, rapport DOSSIER/report.json, defaut
.cache/parity). Seule la delegation D4 y est admise ; le rapport compte les delegations par
cause, format d'image et methode ZIP. Le journal ne donne que des comptes : le rapport, lui,
nomme les archives et reste hors du depot.

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
    drapeau UTF-8 sans objet (None). Un dossier vide compte comme une entree `nom/`
    (taille et SHA-256 None) : un moteur qui en laisse un est en ecart."""
    out = []
    for dp, dn, fns in os.walk(path):
        dn.sort()
        if dp != path and not dn and not fns:
            out.append({"name": os.path.relpath(dp, path).replace(os.sep, "/") + "/",
                        "method": None, "utf8": None, "size": None, "sha256": None})
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
        # texte de l'erreur : jamais compare (§4.1), sert au classement des delegations
        "error_message": err.get("message") if err else None,
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
    Retourne la forme normalisee (voir run_to_dict) et le chemin du `.plain.pk` produit (ou
    None). Sortie `out.plain.pk`, sauf si `args` porte `-o NOM` (NOM pris dans `work_dir`,
    par exemple `out.plain` : dossier seul, sans `.plain.pk`). NOM doit commencer par
    `out.plain` (sortie laissee apres un echec reperee par stray_files)."""
    os.makedirs(work_dir, exist_ok=True)
    args = list(args)
    out_name = "out.plain.pk"
    if "-o" in args:
        i = args.index("-o")
        if i + 1 >= len(args) or not args[i + 1].startswith("out.plain"):
            raise ValueError("cas de corpus : -o out.plain[...] attendu, recu %r" % args[i:])
        out_name = args[i + 1]
        del args[i:i + 2]
    if any(a == "--output" or a.startswith("--output=") for a in args):
        raise ValueError("cas de corpus : sortie a donner par -o, pas --output")
    out = os.path.join(work_dir, out_name)
    # Reechantillonnage MP3 desactive (#46) : l'oracle Python et le candidat Rust doivent
    # produire la meme sortie quel que soit le ffmpeg present sur la machine.
    env = {**os.environ, "FLAM_FFMPEG": ""}
    try:
        r = subprocess.run(list(cmd) + [archive, "-o", out, "--progress-json"] + list(args),
                           capture_output=True, text=True, encoding="utf-8", errors="replace",
                           timeout=timeout, env=env)
    except (OSError, subprocess.TimeoutExpired) as e:
        return harness_error(e), None
    events = parse_events(r.stdout)
    entries = None
    plain_entries = None
    produced = None
    if r.returncode == 0 and out.endswith(".plain"):
        # -o x.plain : seul le dossier est ecrit (aucun .plain.pk a lire ni a valider)
        if os.path.isdir(out):
            plain_entries = entries_from_dir(out)
    elif r.returncode == 0 and os.path.exists(out):
        entries = entries_from_pk(out)
        produced = out
        plain_dir = out[:-len(".pk")]            # ".plain.pk" -> ".plain"
        if os.path.isdir(plain_dir):
            plain_entries = entries_from_dir(plain_dir)
    files = work_files(work_dir)
    try:
        ri = subprocess.run(list(cmd) + [archive, "--info"],
                            capture_output=True, text=True, encoding="utf-8", errors="replace",
                            timeout=timeout, env=env)
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
        "Python internal_error (6, OSError : seek negatif sur un fichier reel ; sur un "
        "BytesIO, ce serait ValueError)",
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
    previous = sys.modules.get("make_corpus")
    sys.modules["make_corpus"] = module
    try:
        spec.loader.exec_module(module)
    except BaseException:
        # chargement rate : pas de module a moitie initialise laisse dans sys.modules
        if previous is None:
            sys.modules.pop("make_corpus", None)
        else:
            sys.modules["make_corpus"] = previous
        raise
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


# ---------------------------------------------------------------------------
# Campagne sur les archives reelles (#135, §4.3)
# ---------------------------------------------------------------------------
# Archives reelles : aucune etiquette de cas. Seule la delegation D4 est admise ; un ecart
# de la liste §4.1 (JSON limite, entree chiffree...) n'y est pas presume et reste un ecart,
# a classer a la main.
CAMPAIGN_TAGS = (TAG_DELEGATED,)
CAMPAIGN_TIMEOUT = 1800
ZIP_METHODS = {zipfile.ZIP_STORED: "stored", zipfile.ZIP_DEFLATED: "deflate",
               zipfile.ZIP_BZIP2: "bzip2", zipfile.ZIP_LZMA: "lzma", 93: "zstd"}
IMAGE_EXTS = (".png", ".jpg", ".jpeg", ".gif", ".bmp", ".webp", ".tif", ".tiff", ".avif",
              ".heic", ".heif", ".ico", ".svg")


HEIF_BRANDS = (b"avif", b"avis", b"heic", b"heix", b"mif1")


def image_format(head, filename=""):
    """Format d'image d'apres les premiers octets (None : pas une image reconnue). Les
    signatures trop larges (`BM` sur deux octets, `<?xml`, qui est tout XML) ne comptent que
    si `filename` a une extension d'image ; `ftyp` exige une marque AVIF/HEIF (sinon c'est
    un MP4/M4A)."""
    image_ext = filename.lower().endswith(IMAGE_EXTS)
    if head.startswith(b"\x89PNG\r\n\x1a\n"):
        return "png"
    if head.startswith(b"\xff\xd8\xff"):
        return "jpeg"
    if head[:6] in (b"GIF87a", b"GIF89a"):
        return "gif"
    if head.startswith(b"BM") and image_ext:
        return "bmp"
    if head[:4] == b"RIFF" and head[8:12] == b"WEBP":
        return "webp"
    if head[:4] in (b"II*\0", b"MM\0*"):
        return "tiff"
    if head[4:8] == b"ftyp" and head[8:12] in HEIF_BRANDS:
        return "avif_heif"
    if head[:4] == b"\0\0\x01\0":
        return "ico"
    start = head.lstrip()[:5]
    if start == b"<svg " or (start == b"<?xml" and image_ext):
        return "svg"
    return None


def archive_inventory(path):
    """Methodes ZIP et formats d'image (par signature) des entrees d'une archive. Une entree
    a extension d'image sans signature reconnue compte comme `inconnu`. Les noms ne sont pas
    gardes : seuls les comptes sortent d'ici."""
    methods, images = Counter(), Counter()
    try:
        with zipfile.ZipFile(path) as z:
            for zi in z.infolist():
                if zi.is_dir():
                    continue
                methods[ZIP_METHODS.get(zi.compress_type, "methode_%d" % zi.compress_type)] += 1
                try:
                    with z.open(zi) as f:
                        head = f.read(16)
                except Exception:  # entree illisible : le convertisseur en decidera
                    head = b""
                fmt = image_format(head, zi.filename)
                if fmt is None and zi.filename.lower().endswith(IMAGE_EXTS):
                    fmt = "inconnu"
                if fmt:
                    images[fmt] += 1
    except Exception as e:  # archive illisible : comptee, sans arreter la campagne
        return {"error": type(e).__name__, "methods": {}, "images": {}}
    return {"error": None, "methods": dict(sorted(methods.items())),
            "images": dict(sorted(images.items()))}


# Expressions ancrees sur la forme exacte des messages du candidat (zipread.rs, png.rs) : un
# nom d'asset contenant « compression 3 » ou « format non pris en charge » ne doit pas
# fausser la cause.
_COMPRESSION_RE = re.compile(r": compression (\d+) \(bzip2/LZMA/Zstandard\)")
_IMAGE_WHY_RE = re.compile(r"^.* : format non pris en charge : (.+)$")
# Raisons du decodeur PNG (`PngError::Unsupported`) : liste fermee, le reste est `autre`.
IMAGE_REASONS = ("APNG", "IHDR absent en t\u00eate ou r\u00e9p\u00e9t\u00e9", "pas un PNG",
                 "tRNS apr\u00e8s l'IDAT", "tRNS de longueur inattendue", "tRNS r\u00e9p\u00e9t\u00e9")
SUSPECT_CAUSES = ("autre", "metadata")


def delegation_cause(message):
    """Cause d'une delegation d'apres le message du candidat, sans nom d'asset :
    `zip:<methode>`, `image:<raison>` (raison fixe du decodeur PNG), `metadata` ou `autre`."""
    message = message or ""
    m = _COMPRESSION_RE.search(message)
    if m:
        return "zip:" + ZIP_METHODS.get(int(m.group(1)), "methode_" + m.group(1))
    m = _IMAGE_WHY_RE.match(message)
    if m and m.group(1).strip() in IMAGE_REASONS:
        return "image:" + m.group(1).strip()
    if message.startswith("metadata.json"):
        return "metadata"
    return "autre"


def delegation_warning(by_cause):
    """Avertissement (chiffres seuls) si des delegations sont `autre` ou `metadata` : hors D4
    strict (zip non lu, PNG non decode), elles sont a classer a la main. None sinon."""
    parts = ["%d `%s`" % (by_cause[c], c) for c in SUSPECT_CAUSES if by_cause.get(c)]
    if not parts:
        return None
    return "attention : delegations hors D4 strict a classer a la main : " + ", ".join(parts)


def command_fingerprint(cmd):
    """SHA-256 des fichiers nommes dans une commande (interpreteur, script, binaire) : un
    resultat obtenu avec un autre moteur n'est pas repris."""
    h = hashlib.sha256()

    def add_file(path):
        with open(path, "rb") as f:
            h.update(_sha(f.read()).encode("ascii"))

    for i, token in enumerate(cmd):
        h.update(token.encode("utf-8") + b"\0")
        # le programme (jeton 0) est resolu comme le ferait subprocess : par le PATH
        path = shutil.which(token) if i == 0 else None
        if path is None and os.path.isfile(token):
            path = token
        if i == 0 and path is None:
            raise ValueError("commande introuvable : %s" % token)
        if path is None:
            continue
        add_file(path)
        if path.lower().endswith(".py"):
            # script Python : ses modules voisins (lif.py, mp3map.py...) font partie du moteur
            folder = os.path.dirname(os.path.abspath(path))
            for name in sorted(os.listdir(folder)):
                if name.endswith(".py"):
                    h.update(name.encode("utf-8") + b"\0")
                    add_file(os.path.join(folder, name))
    return h.hexdigest()


def validate_subprocess(pk_path, timeout):
    """validate.py dans un processus a part (redirect_stdout n'est pas sur entre threads)."""
    try:
        r = subprocess.run([sys.executable, os.path.join(HERE, "validate.py"), pk_path],
                           capture_output=True, timeout=timeout)
    except (OSError, subprocess.TimeoutExpired) as e:
        return "harness_error: %s" % type(e).__name__
    return r.returncode


def campaign_case(archive, oracle, candidate, work, timeout=CAMPAIGN_TIMEOUT, validate=True):
    """Une archive : oracle puis candidat (jamais en meme temps, pour borner la charge),
    comparaison, inventaire. Le dossier de travail est supprime, sauf en cas d'ecart."""
    shutil.rmtree(work, ignore_errors=True)
    o, _ = run_converter(oracle, archive, os.path.join(work, "oracle"), timeout=timeout)
    c, produced = run_converter(candidate, archive, os.path.join(work, "candidate"),
                                timeout=timeout)
    if validate and produced is not None:
        c["validation"] = validate_subprocess(produced, timeout)
    res = compare_runs(o, c, CAMPAIGN_TAGS)
    res["oracle"] = {k: o[k] for k in ("exit", "error_code", "error_message", "warnings")}
    res["candidate"] = {k: c[k] for k in ("exit", "error_code", "error_message", "warnings")}
    res["candidate"]["validation"] = c.get("validation")
    if res["verdict"] == "delegated":
        res["delegation"] = delegation_cause(c["error_message"])
    res["inventory"] = archive_inventory(archive)
    if res["verdict"] != "diff":
        shutil.rmtree(work, ignore_errors=True)
    return res


def _harness_failure(res):
    """Execution impossible (delai, commande absente, sortie illisible) : refaite a la reprise."""
    return (any(d.get("kind") == "harness" for d in res.get("diffs", ()))
            or any((res.get(side) or {}).get("error_code") == "harness_error"
                   for side in ("oracle", "candidate"))
            or str((res.get("candidate") or {}).get("validation")).startswith("harness_error"))


def _reusable(prev, engines):
    """Etat repris : meme oracle, meme candidat, validation au moins aussi complete (un etat
    obtenu sans validate.py n'est pas repris par une campagne qui valide), forme attendue."""
    if not isinstance(prev, dict) or not all(k in prev for k in ("verdict", "diffs",
                                                                 "inventory")):
        return False
    old = prev.get("engines")
    if not isinstance(old, dict) or _harness_failure(prev):
        return False
    if (old.get("oracle"), old.get("candidate")) != (engines["oracle"], engines["candidate"]):
        return False
    return not engines["validate"] or old.get("validate") == engines["validate"]


def _write_json_atomic(path, doc):
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8", newline="\n") as f:
        json.dump(doc, f, indent=1, sort_keys=True, ensure_ascii=True)
        f.write("\n")
    os.replace(tmp, path)


def campaign_summary(cases):
    """Comptes seulement (publiables) : verdicts, delegations par cause, formats d'image et
    methodes ZIP des archives deleguees et de tout le corpus, seuil de bascule §4.3."""
    s = summarize(cases)
    delegated = [c for c in cases if c["verdict"] == "delegated"]

    def per_archive(selection, key):
        n = Counter()
        for c in selection:
            n.update(c["inventory"][key].keys())
        return dict(sorted(n.items()))

    def per_entry(selection, key):
        n = Counter()
        for c in selection:
            n.update(c["inventory"][key])
        return dict(sorted(n.items()))

    diff_kinds = Counter()
    for c in cases:
        if c["verdict"] == "diff":
            diff_kinds.update({d["kind"] for d in c["diffs"]})
    return {
        "summary": s,
        "delegations_by_cause": dict(sorted(Counter(c["delegation"] for c in delegated).items())),
        "delegated_archives": {"images": per_archive(delegated, "images"),
                               "zip_methods": per_archive(delegated, "methods")},
        "corpus_archives": {"images": per_archive(cases, "images"),
                            "zip_methods": per_archive(cases, "methods")},
        "corpus_entries": {"images": per_entry(cases, "images"),
                           "zip_methods": per_entry(cases, "methods")},
        "inventory_errors": sum(1 for c in cases if c["inventory"]["error"]),
        "diff_kinds": dict(sorted(diff_kinds.items())),
        # §4.3 : aucun ecart hors liste, ni sur les statuts ni sur les entrees
        "threshold_met": bool(s["total"]) and s["diff"] == 0,
    }


def run_campaign(archives_dir, oracle, candidate, out_dir, jobs=1, timeout=CAMPAIGN_TIMEOUT,
                 force=False, limit=None, validate=True, progress=None):
    """Compare chaque `*.zip` de `archives_dir`, par lots avec reprise : un fichier d'etat par
    archive sous `out_dir/cases/`, repris s'il a ete obtenu avec les memes moteurs (empreintes)
    sauf `force`. Rapport `out_dir/report.json`, recalcule a chaque archive terminee."""
    import concurrent.futures
    import threading

    names = sorted(n for n in os.listdir(archives_dir) if n.lower().endswith(".zip"))
    if limit is not None:
        names = names[:limit]
    engines = {"oracle": command_fingerprint(oracle), "candidate": command_fingerprint(candidate),
               # empreinte de validate.py, ou None sans validation
               "validate": command_fingerprint([sys.executable, os.path.join(HERE, "validate.py")])
               if validate else None}
    cases_dir = os.path.join(out_dir, "cases")
    work_root = os.path.join(out_dir, "work")
    os.makedirs(cases_dir, exist_ok=True)
    results = {}
    todo = []
    for n in names:
        state = os.path.join(cases_dir, n[:-4] + ".json")
        if not force and os.path.exists(state):
            try:
                with open(state, encoding="utf-8") as f:
                    prev = json.load(f)
                if _reusable(prev, engines):
                    results[n] = prev
                    continue
            except (ValueError, OSError):
                pass  # etat tronque ou illisible : refait
        todo.append(n)
    lock = threading.Lock()

    def write_report():
        cases = [results[n] for n in names if n in results]
        doc = {"schema": REPORT_SCHEMA, "kind": "campaign", "engines": engines,
               "pending": len(names) - len(cases),
               # --limit : sous-ensemble des archives, pas un resultat de campagne complet
               "partial": limit is not None}
        doc.update(campaign_summary(cases))
        doc["cases"] = cases
        _write_json_atomic(os.path.join(out_dir, "report.json"), doc)
        return doc

    def harness_failure(n, e):
        # seul le nom du type d'exception : ni message ni chemin
        return {"verdict": "diff", "rule": None, "id": n[:-4], "engines": engines,
                "diffs": [{"kind": "harness", "oracle": None,
                           "candidate": "%s" % type(e).__name__}],
                "inventory": archive_inventory(os.path.join(archives_dir, n))}

    def one(n):
        archive = os.path.join(archives_dir, n)
        try:
            res = campaign_case(archive, oracle, candidate, os.path.join(work_root, n[:-4]),
                                timeout, validate)
        except Exception as e:  # sortie illisible, etc. : un ecart, la campagne continue
            res = {"verdict": "diff", "rule": None,
                   "diffs": [{"kind": "harness", "oracle": None,
                              "candidate": "%s: %s" % (type(e).__name__, e)}],
                   "inventory": archive_inventory(archive)}
        res["id"] = n[:-4]
        res["engines"] = engines
        _write_json_atomic(os.path.join(cases_dir, n[:-4] + ".json"), res)
        with lock:
            results[n] = res
            write_report()
            if progress:
                progress(len(results), len(names), res)
        return res

    write_report()
    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, jobs)) as ex:
        for n, fut in [(n, ex.submit(one, n)) for n in todo]:
            try:
                fut.result()
            except Exception as e:  # echec imprevu du harnais : un ecart, sans trace sur stderr
                with lock:
                    results[n] = harness_failure(n, e)
                    write_report()
    return write_report()


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
    p = sub.add_parser("campaign", help="archives reelles, par lots avec reprise (#135)")
    p.add_argument("dir", help="dossier des archives *.zip (lu seulement)")
    p.add_argument("--candidate", required=True, help="commande du candidat")
    p.add_argument("--oracle", help="commande de l'oracle (defaut : telmi2flam.py)")
    p.add_argument("--out", default=os.path.join(".cache", "parity"),
                   help="etat et rapport (defaut %(default)s ; jamais dans le depot)")
    p.add_argument("--jobs", type=int, default=1,
                   help="archives en parallele, chacune oracle puis candidat (defaut 1)")
    p.add_argument("--timeout", type=int, default=CAMPAIGN_TIMEOUT,
                   help="delai par conversion, en secondes (defaut %(default)s)")
    p.add_argument("--limit", type=int, help="seulement les N premieres archives")
    p.add_argument("--force", action="store_true", help="refaire les archives deja comparees")
    p.add_argument("--no-validate", action="store_true",
                   help="ne pas passer validate.py sur la sortie du candidat")
    args = ap.parse_args(argv)

    if args.mode == "campaign":
        oracle = split_cmd(args.oracle) if args.oracle else default_oracle()

        def progress(done, total, res):
            # comptes seulement : ni nom d'archive ni contenu dans le journal
            print("%d/%d %s" % (done, total, res["verdict"]), flush=True)
        doc = run_campaign(args.dir, oracle, split_cmd(args.candidate), args.out,
                           jobs=args.jobs, timeout=args.timeout, force=args.force,
                           limit=args.limit, validate=not args.no_validate, progress=progress)
        s = doc["summary"]
        print("campagne : %d archives, %d identiques, %d deleguees, %d ecarts, %d en attente ; "
              "seuil (par. 4.3) %s" % (s["total"], s["identical"], s["delegated"], s["diff"],
                                 doc["pending"], "atteint" if doc["threshold_met"] else "non atteint"))
        print("delegations par cause : %s" % json.dumps(doc["delegations_by_cause"]))
        warning = delegation_warning(doc["delegations_by_cause"])
        if warning:
            print(warning)
        if doc["partial"]:
            print("attention : rapport partiel (--limit %d archives)" % args.limit)
        return 0 if doc["threshold_met"] and not doc["pending"] else 1
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
