#!/usr/bin/env python3
"""
compat.py - Campagne de compatibilite telmi2flam : telechargement, conversion,
validation, (execution optionnelle) et rapport pour un lot d'histoires Telmi.

Usage :
    python compat.py --catalog <url-ou-fichier.json> [--folder <dossier>] [--cache .cache/telmi]
                     [--only <uuid|motif>] [--limit N] [--jobs N] [--force]
                     [--player flam-player.exe]

Sorties (sous --cache) :
    archives/<uuid>-<version>.zip          archives telechargees
    converted/<uuid>-<version>.plain.pk    histoires converties
    compat/report.json, compat/report.md   rapport

Voir README.md (section « Campagne de compatibilite »).
"""

import argparse
import concurrent.futures
import contextlib
import fnmatch
import hashlib
import http.client
import io
import json
import os
import re
import sys
import time
import urllib.error
import urllib.request
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
if HERE not in sys.path:
    sys.path.insert(0, HERE)

DEFAULT_CACHE = os.path.join(".cache", "telmi")
RETRIES = 3
TIMEOUT = 30
CHUNK = 1 << 20

STEPS = ("download", "convert", "validate", "run")
GLOBAL_STATUSES = ("OK", "OK_AVERTISSEMENTS", "ECHEC_CONVERSION", "ECHEC_VALIDATION",
                   "ECHEC_EXECUTION", "ECHEC_TELECHARGEMENT")
# Statuts de explore() consideres comme un echec d'execution
RUN_FAIL = ("lua_error", "crash", "timeout", "stuck")


class DownloadError(Exception):
    pass


# ---------------------------------------------------------------------------
# Sources
# ---------------------------------------------------------------------------
def _safe(s):
    """Fragment utilisable dans un nom de fichier."""
    return re.sub(r"[^A-Za-z0-9._-]", "_", str(s)) or "_"


def story_key(uuid, version):
    return "%s-%s" % (_safe(uuid), _safe(version))


def _first(d, *keys):
    for k in keys:
        v = d.get(k)
        if v not in (None, ""):
            return v
    return None


def _fetch_bytes(src, timeout=TIMEOUT):
    if re.match(r"^https?://", src):
        with urllib.request.urlopen(src, timeout=timeout) as r:
            return r.read()
    with open(src, "rb") as f:
        return f.read()


def load_catalog(src, fetch=_fetch_bytes):
    """Lit un catalogue Telmi Store (url ou fichier). Tolerant : champs manquants,
    entrees non objet ignorees. Retourne une liste d'entrees normalisees."""
    doc = json.loads(fetch(src).decode("utf-8-sig"))
    items = doc.get("data") if isinstance(doc, dict) else doc
    if not isinstance(items, list):
        raise ValueError("catalogue %s : liste 'data' absente" % src)
    out = []
    for it in items:
        if not isinstance(it, dict):
            continue
        uuid = _first(it, "uuid", "id")
        download = _first(it, "download", "url")
        if uuid is None:
            continue
        size = _first(it, "size", "fileSize")
        out.append({
            "source": src,
            "uuid": str(uuid),
            "version": str(_first(it, "version") or "0"),
            "title": str(_first(it, "title", "name") or uuid),
            "license": _first(it, "license", "licence"),
            "category": _first(it, "category", "categorie"),
            "download": download,
            "size": size if isinstance(size, int) and not isinstance(size, bool) else None,
            "path": None,
            "fingerprint": (str(_first(it, "updated_at", "updatedAt")) if _first(
                it, "updated_at", "updatedAt") is not None else None),
        })
    return out


def _zip_identity(path):
    """(uuid, titre) lus dans metadata.json de l'archive, sinon (None, None)."""
    try:
        with zipfile.ZipFile(path) as zf:
            metas = sorted((n for n in zf.namelist() if n.split("/")[-1] == "metadata.json"
                            and "__MACOSX" not in n), key=lambda n: n.count("/"))
            if metas:
                meta = json.loads(zf.read(metas[0]).decode("utf-8-sig"))
                if isinstance(meta, dict):
                    u, t = meta.get("uuid"), meta.get("title")
                    return (str(u) if u else None), (str(t) if t else None)
    except Exception:
        pass
    return None, None


def _file_fingerprint(path):
    """Taille + mtime : detecte un zip local remplace."""
    try:
        st = os.stat(path)
        return "%d:%d" % (st.st_size, st.st_mtime_ns)
    except OSError:
        return None


def scan_folder(folder):
    """Entrees pour les .zip d'un dossier (pas de telechargement)."""
    out = []
    for name in sorted(os.listdir(folder)):
        path = os.path.join(folder, name)
        if not name.lower().endswith(".zip") or not os.path.isfile(path):
            continue
        stem = name[:-4]
        uuid, title = _zip_identity(path)
        out.append({"source": folder, "uuid": uuid or stem, "version": "local",
                    "title": title or stem, "license": None, "category": None,
                    "download": None, "size": None, "path": path,
                    "fingerprint": _file_fingerprint(path)})
    return out


def collect(catalogs, folders, fetch=_fetch_bytes):
    """Fusionne les sources ; premiere occurrence d'un (uuid, version) gagnante."""
    seen, out = set(), []
    for c in catalogs:
        for e in load_catalog(c, fetch):
            k = story_key(e["uuid"], e["version"])
            if k not in seen:
                seen.add(k)
                out.append(e)
    for d in folders:
        for e in scan_folder(d):
            k = story_key(e["uuid"], e["version"])
            if k not in seen:
                seen.add(k)
                out.append(e)
    return out


def select(entries, only=(), limit=None):
    """--only : uuid exact, motif glob (*?) ou sous-chaine du uuid/titre."""
    if only:
        def match(e):
            for p in only:
                pl = p.lower()
                hay = (e["uuid"].lower(), e["title"].lower())
                if e["uuid"] == p or any(h == pl for h in hay):
                    return True
                if any(c in p for c in "*?["):
                    if any(fnmatch.fnmatchcase(h, pl) for h in hay):
                        return True
                elif any(pl in h for h in hay):
                    return True
            return False
        entries = [e for e in entries if match(e)]
    if limit is not None:
        entries = entries[:limit]
    return entries


# ---------------------------------------------------------------------------
# Telechargement (reprise, verification de taille, tentatives)
# ---------------------------------------------------------------------------
def _urlopen(url, headers, timeout):
    return urllib.request.urlopen(urllib.request.Request(url, headers=headers), timeout=timeout)


def _total_size(resp, offset):
    """Taille totale annoncee par la reponse, ou None."""
    cr = resp.headers.get("Content-Range") if hasattr(resp, "headers") else None
    if cr:
        m = re.search(r"/(\d+)$", cr)
        if m:
            return int(m.group(1))
    cl = resp.headers.get("Content-Length") if hasattr(resp, "headers") else None
    if cl and cl.isdigit():
        return int(cl) + (offset if getattr(resp, "status", 200) == 206 else 0)
    return None


def download(entry, archives_dir, opener=_urlopen, retries=RETRIES, timeout=TIMEOUT,
             sleep=time.sleep):
    """Telecharge l'archive de `entry` ; retourne (chemin, message). Leve DownloadError.
    - fichier final deja present et de la bonne taille : rien a faire ;
    - sinon reprise du .part par requete Range."""
    url = entry.get("download")
    if not url:
        raise DownloadError("pas d'URL de telechargement dans le catalogue")
    if not isinstance(url, str):
        raise DownloadError("URL de telechargement invalide (%s)" % type(url).__name__)
    os.makedirs(archives_dir, exist_ok=True)
    final = os.path.join(archives_dir, story_key(entry["uuid"], entry["version"]) + ".zip")
    part = final + ".part"
    sidecar = final + ".size"

    def known_size():
        if entry.get("size"):
            return entry["size"]
        try:
            with open(sidecar) as f:
                return int(f.read().strip())
        except (OSError, ValueError):
            return None

    if os.path.isfile(final):
        exp = known_size()
        have = os.path.getsize(final)
        if have > 0 and (exp is None or exp == have):
            return final, "deja telecharge (%d octets)" % have
        os.remove(final)

    last = None
    for attempt in range(1, retries + 1):
        try:
            offset = os.path.getsize(part) if os.path.isfile(part) else 0
            headers = {"Range": "bytes=%d-" % offset} if offset else {}
            try:
                resp = opener(url, headers, timeout)
            except urllib.error.HTTPError as e:
                if e.code == 416 and offset:       # .part deja complet ?
                    exp = known_size()
                    if (exp == offset) if exp is not None else zipfile.is_zipfile(part):
                        resp = None
                    else:
                        os.remove(part)
                        raise DownloadError("reprise impossible (HTTP 416), .part supprime")
                else:
                    raise
            total = offset
            if resp is not None:
                with contextlib.closing(resp):
                    if offset and getattr(resp, "status", 200) != 206:
                        offset = 0                  # le serveur ignore Range : on repart de zero
                    expected = _total_size(resp, offset)
                    with open(part, "ab" if offset else "wb") as f:
                        while True:
                            buf = resp.read(CHUNK)
                            if not buf:
                                break
                            f.write(buf)
                total = os.path.getsize(part)
                if expected is not None and total != expected:
                    raise DownloadError("telechargement incomplet (%d/%d octets)"
                                        % (total, expected))
                if entry.get("size") and total != entry["size"]:
                    raise DownloadError("taille %d differente de celle du catalogue (%d)"
                                        % (total, entry["size"]))
                if expected is not None:
                    with open(sidecar, "w") as f:
                        f.write(str(expected))
            if total == 0:
                raise DownloadError("fichier vide")
            os.replace(part, final)
            return final, "telecharge (%d octets, tentative %d)" % (total, attempt)
        except (OSError, DownloadError, ValueError, TypeError,
                http.client.HTTPException) as e:   # URLError, timeout... sont des OSError
            last = e
            if attempt < retries:
                sleep(min(2 ** (attempt - 1), 5))
    raise DownloadError("%s apres %d tentatives : %s" % (type(last).__name__, retries, last))


# ---------------------------------------------------------------------------
# Hash du convertisseur (reprise)
# ---------------------------------------------------------------------------
def converter_hash(root=HERE):
    """Hash des sources du convertisseur (hors tests, compat, explore, docs)."""
    h = hashlib.sha256()
    skip_files = {"compat.py", "explore.py"}
    for dp, dns, fns in os.walk(root):
        dns[:] = sorted(d for d in dns if d not in ("tests", "__pycache__"))
        for fn in sorted(fns):
            if fn in skip_files or not fn.endswith((".py", ".lua")):
                continue
            p = os.path.join(dp, fn)
            h.update(os.path.relpath(p, root).replace("\\", "/").encode())
            with open(p, "rb") as f:
                h.update(f.read())
    return h.hexdigest()[:16]


# ---------------------------------------------------------------------------
# Etapes
# ---------------------------------------------------------------------------
def _step(status, messages=None, seconds=0.0):
    return {"status": status, "messages": list(messages or []), "seconds": round(seconds, 2)}


def do_convert(zip_path, out_path):
    """Convertit en capturant avertissements et exceptions."""
    import telmi2flam
    os.makedirs(os.path.dirname(out_path), exist_ok=True)
    out, err = io.StringIO(), io.StringIO()
    t0 = time.time()
    exc = None
    try:
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            telmi2flam.convert(zip_path, out_path)
    except Exception as e:                 # ConvError, BadZipFile, OSError, bug imprevu...
        exc = "%s: %s" % (type(e).__name__, e)
    dt = time.time() - t0
    warns = [l.strip().lstrip("!").strip() for l in err.getvalue().splitlines()
             if l.strip().startswith("!")]
    if exc:
        return _step("fail", [exc] + warns, dt)
    return _step("warn" if warns else "ok", warns, dt)


def do_validate(pk_path):
    import validate
    out = io.StringIO()
    t0 = time.time()
    try:
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(io.StringIO()):
            rc = validate.main(pk_path)
    except Exception as e:
        return _step("fail", ["validate a plante : %s: %s" % (type(e).__name__, e)],
                     time.time() - t0)
    dt = time.time() - t0
    lines = out.getvalue().splitlines()
    errs = [l.split("]", 1)[1].strip() for l in lines if l.strip().startswith("[ERR]")]
    warns = [l.split("]", 1)[1].strip() for l in lines if l.strip().startswith("[WARN]")]
    if rc != 0:
        return _step("fail", errs or ["validate code %s" % rc], dt)
    return _step("warn" if warns else "ok", warns, dt)


def do_run(pk_path, explore, player_exe, timeout_s, seed, max_steps):
    if explore is None:
        return _step("skipped", ["pas de fonction explore() fournie"])
    t0 = time.time()
    try:
        res = explore(player_exe, pk_path, timeout_s=timeout_s, seed=seed, max_steps=max_steps)
    except Exception as e:
        return _step("fail", ["explore a plante : %s: %s" % (type(e).__name__, e)],
                     time.time() - t0)
    dt = time.time() - t0
    res = res if isinstance(res, dict) else {}
    status = res.get("status")
    msgs = ["explore: %s" % status]
    for k in ("message", "error", "detail"):
        if res.get(k):
            msgs.append(str(res[k]))
    errors = res.get("errors")
    if isinstance(errors, (list, tuple)):
        msgs += [str(x) for x in errors]
    if status == "ok":
        st = _step("ok", msgs[1:], dt)
    elif status in RUN_FAIL:
        st = _step("fail", msgs, dt)
    else:
        st = _step("fail", msgs + ["statut explore inconnu"], dt)
    for k in ("scenes_visited", "scenes_total", "end_reached"):
        if k in res:
            st[k] = res[k]
    # duree mesuree par explore si fournie, sinon par la campagne
    dur = res.get("duration_s")
    st["duration_s"] = round(dur, 2) if isinstance(dur, (int, float)) else round(dt, 2)
    return st


def global_status(steps):
    d, c, v, r = (steps.get(k, {}).get("status") for k in STEPS)
    if d == "fail":
        return "ECHEC_TELECHARGEMENT"
    if c == "fail":
        return "ECHEC_CONVERSION"
    if v == "fail":
        return "ECHEC_VALIDATION"
    if r == "fail":
        return "ECHEC_EXECUTION"
    if "warn" in (d, c, v, r):
        return "OK_AVERTISSEMENTS"
    return "OK"


def process_story(entry, cache, opener=None, sleep=time.sleep):
    """Telechargement + conversion + validation (sans execution). Retourne la fiche."""
    archives = os.path.join(cache, "archives")
    converted = os.path.join(cache, "converted")
    key = story_key(entry["uuid"], entry["version"])
    rec = {k: entry.get(k) for k in ("source", "uuid", "version", "title", "license", "category")}
    rec["key"] = key
    rec["fingerprint"] = entry.get("fingerprint")
    steps = {}
    rec["steps"] = steps
    t0 = time.time()
    # Filet de securite : aucune exception ne doit faire echouer toute la campagne
    try:
        if entry.get("path"):
            zip_path = entry["path"]
            steps["download"] = _step("skipped", ["archive locale"])
        else:
            try:
                kw = {"sleep": sleep}
                if opener:
                    kw["opener"] = opener
                zip_path, msg = download(entry, archives, **kw)
                steps["download"] = _step("ok", [msg], time.time() - t0)
            except Exception as e:
                steps["download"] = _step("fail", ["%s: %s" % (type(e).__name__, e)]
                                          if not isinstance(e, DownloadError) else [str(e)],
                                          time.time() - t0)
    except Exception as e:
        steps["download"] = _step("fail", ["%s: %s" % (type(e).__name__, e)])
    if steps["download"]["status"] == "fail":
        for s in STEPS[1:]:
            steps[s] = _step("skipped", ["telechargement en echec"])
    else:
        try:
            pk = os.path.join(converted, key + ".plain.pk")
            steps["convert"] = do_convert(zip_path, pk)
            if steps["convert"]["status"] == "fail":
                steps["validate"] = _step("skipped", ["conversion en echec"])
            else:
                steps["validate"] = do_validate(pk)
                rec["output"] = pk
        except Exception as e:
            msg = ["%s: %s" % (type(e).__name__, e)]
            if "convert" not in steps:
                steps["convert"] = _step("fail", msg)
                steps["validate"] = _step("skipped", ["conversion en echec"])
            else:
                steps["validate"] = _step("fail", msg)
        steps["run"] = _step("skipped", ["pas encore execute"])
    rec["status"] = global_status(steps)
    return rec


def _worker(args):
    entry, cache = args
    return process_story(entry, cache)


# ---------------------------------------------------------------------------
# Campagne
# ---------------------------------------------------------------------------
def _load_previous(report_path):
    """(hash du convertisseur du rapport, [fiches]) ; fiches sans hash propre -> hash du rapport."""
    try:
        with open(report_path, encoding="utf-8") as f:
            doc = json.load(f)
        stories = [dict(s, converter_hash=s.get("converter_hash") or doc.get("converter_hash"))
                   for s in doc.get("stories", []) if isinstance(s, dict) and "key" in s]
        return doc.get("converter_hash"), stories
    except (OSError, ValueError, TypeError):
        return None, []


def _write_atomic(path, text):
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        f.write(text)
    os.replace(tmp, path)


def run_campaign(entries, cache=DEFAULT_CACHE, jobs=1, force=False, explore=None,
                 player_exe=None, run_timeout=120, seed=0, max_steps=200, log=print):
    """Evalue chaque entree ; ecrit report.json / report.md ; retourne le document.
    Le rapport conserve les fiches precedentes hors selection (--only/--limit) : celles
    evaluees avec un autre convertisseur sont marquees `stale` (perimees) dans le rapport.
    report.json est reecrit (atomiquement) apres chaque histoire."""
    compat_dir = os.path.join(cache, "compat")
    os.makedirs(compat_dir, exist_ok=True)
    report_path = os.path.join(compat_dir, "report.json")
    chash = converter_hash()
    prev_hash, prev_list = _load_previous(report_path)
    prev = {s["key"]: s for s in prev_list}
    keys = [story_key(e["uuid"], e["version"]) for e in entries]
    selected = set(keys)

    def build_doc(results):
        stories = []
        for s in prev_list:                      # fiches precedentes hors selection
            if s["key"] not in selected:
                stories.append(dict(s, stale=(s.get("converter_hash") != chash)))
        for k in keys:                           # selection : resultat frais, sinon ancienne fiche
            r = results.get(k) or prev.get(k)
            if r is not None:
                stories.append(dict(r, stale=(r.get("converter_hash") != chash)))
        return {"generated": time.strftime("%Y-%m-%dT%H:%M:%S"), "converter_hash": chash,
                "stories": stories}

    def write(results, md=False):
        doc = build_doc(results)
        _write_atomic(report_path, json.dumps(doc, ensure_ascii=False, indent=2))
        if md:
            _write_atomic(os.path.join(compat_dir, "report.md"), render_markdown(doc))
        return doc

    results, todo = {}, []
    for e, k in zip(entries, keys):
        old = prev.get(k)
        reusable = (old is not None and not force and old.get("converter_hash") == chash
                    and old.get("fingerprint") == e.get("fingerprint")
                    and old.get("status") != "ECHEC_TELECHARGEMENT"
                    and not (explore is not None
                             and old.get("steps", {}).get("run", {}).get("status") == "skipped"
                             and old.get("steps", {}).get("validate", {}).get("status")
                             in ("ok", "warn")))
        if reusable:
            results[k] = old
            log("= %s : deja evalue (%s)" % (k, old["status"]))
        else:
            todo.append(e)

    def finish(rec):
        pk = rec.get("output")
        if pk and rec["steps"]["validate"]["status"] in ("ok", "warn"):
            rec["steps"]["run"] = do_run(pk, explore, player_exe, run_timeout, seed, max_steps)
            rec["status"] = global_status(rec["steps"])
        rec["converter_hash"] = chash
        results[rec["key"]] = rec
        write(results)
        log("%s %s : %s" % ("+" if rec["status"].startswith("OK") else "!", rec["key"],
                            rec["status"]))

    if jobs > 1 and len(todo) > 1:
        with concurrent.futures.ProcessPoolExecutor(max_workers=jobs) as ex:
            for rec in ex.map(_worker, [(e, cache) for e in todo]):
                finish(rec)
    else:
        for e in todo:
            finish(process_story(e, cache))
    return write(results, md=True)


def _cause(rec):
    """Premiere cause d'echec, generalisee pour regrouper les memes problemes."""
    for s in STEPS:
        st = rec["steps"].get(s, {})
        if st.get("status") == "fail":
            msg = (st.get("messages") or ["?"])[0]
            msg = re.sub(r"\d+", "N", msg)
            return s, msg[:160]
    return None, None


def render_markdown(doc):
    stories = doc["stories"]
    counts = {}
    for s in stories:
        counts[s["status"]] = counts.get(s["status"], 0) + 1
    L = ["# Rapport de compatibilite telmi2flam", "",
         "Genere le %s - convertisseur `%s` - %d histoire(s)." % (
             doc["generated"], doc["converter_hash"], len(stories)), "",
         "## Synthese", ""]
    for st in GLOBAL_STATUSES:
        if st in counts:
            L.append("- %s : %d" % (st, counts[st]))
    L += ["", "## Histoires", "",
          "| Titre | UUID | Version | Dl | Conv | Valid | Exec | Statut |",
          "|---|---|---|---|---|---|---|---|"]
    for s in stories:
        cells = [s["steps"].get(k, {}).get("status", "-") for k in STEPS]
        L.append("| %s | %s | %s | %s | %s |" % (
            _md(s["title"]), _md(s["uuid"]), _md(s["version"]), " | ".join(cells),
            s["status"] + (" (perime)" if s.get("stale") else "")))
    runs = [s for s in stories if "scenes_visited" in s["steps"].get("run", {})]
    if runs:
        L += ["", "## Execution (couverture)", "",
              "| Titre | Scenes visitees | Fin atteinte | Duree (s) | Statut |",
              "|---|---|---|---|---|"]
        for s in runs:
            r = s["steps"]["run"]
            tot = r.get("scenes_total")
            end = r.get("end_reached")
            L.append("| %s | %s/%s | %s | %s | %s |" % (
                _md(s["title"]), r.get("scenes_visited"), tot if tot is not None else "?",
                "oui" if end else ("non" if end is not None else "?"),
                r.get("duration_s", "?"), r["status"]))
    groups = {}
    for s in stories:
        step, cause = _cause(s)
        if step:
            groups.setdefault((step, cause), []).append(s)
    L += ["", "## Echecs par cause", ""]
    if not groups:
        L.append("Aucun echec.")
    for (step, cause), lst in sorted(groups.items(), key=lambda kv: (-len(kv[1]), kv[0])):
        L += ["### [%s] %s (%d)" % (step, _md(cause), len(lst)), ""]
        for s in lst:
            msgs = "; ".join(s["steps"][step].get("messages", [])[:3])
            L.append("- %s (`%s`) : %s" % (_md(s["title"]), s["key"], _md(msgs)))
        L.append("")
    return "\n".join(L) + "\n"


def _md(s):
    return str(s).replace("|", "\\|").replace("\n", " ")


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------
def _load_explore():
    """explore() du module explore.py s'il existe (issue #5), sinon None."""
    if not os.path.isfile(os.path.join(HERE, "explore.py")):
        return None
    try:
        import explore
        return getattr(explore, "explore", None)
    except Exception as e:
        print("explore.py inutilisable : %s: %s" % (type(e).__name__, e), file=sys.stderr)
        return None


def main(argv=None):
    ap = argparse.ArgumentParser(description="Campagne de compatibilite telmi2flam")
    ap.add_argument("--catalog", action="append", default=[],
                    help="catalogue Telmi Store (url ou fichier .json), repetable")
    ap.add_argument("--folder", action="append", default=[],
                    help="dossier de .zip locaux, repetable")
    ap.add_argument("--cache", default=DEFAULT_CACHE, help="racine du cache (defaut %(default)s)")
    ap.add_argument("--only", action="append", default=[],
                    help="uuid, motif (*?) ou sous-chaine du titre, repetable")
    ap.add_argument("--limit", type=int, help="nombre maximal d'histoires")
    ap.add_argument("--jobs", type=int, default=1, help="traitements paralleles (defaut 1)")
    ap.add_argument("--force", action="store_true", help="refaire meme si deja evalue")
    ap.add_argument("--player", help="flam-player(.exe) pour l'etape run (si explore.py existe)")
    ap.add_argument("--run-timeout", type=int, default=120)
    ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--max-steps", type=int, default=200)
    args = ap.parse_args(argv)
    if args.limit is not None and args.limit < 0:
        ap.error("--limit doit etre positif ou nul")
    if not args.catalog and not args.folder:
        ap.error("au moins un --catalog ou --folder est requis")
    try:
        entries = select(collect(args.catalog, args.folder), args.only, args.limit)
    except (OSError, ValueError) as e:
        print("ERREUR : %s" % e, file=sys.stderr)
        return 2
    explore = _load_explore() if args.player else None
    doc = run_campaign(entries, args.cache, max(1, args.jobs), args.force, explore,
                       args.player, args.run_timeout, args.seed, args.max_steps)
    bad = sum(1 for s in doc["stories"] if s["status"].startswith("ECHEC"))
    print("%d histoire(s), %d echec(s) -> %s" % (
        len(doc["stories"]), bad, os.path.join(args.cache, "compat", "report.md")))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
