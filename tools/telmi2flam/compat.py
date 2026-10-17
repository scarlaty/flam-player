#!/usr/bin/env python3
"""
compat.py - Campagne de compatibilite telmi2flam : telechargement, conversion,
validation, (execution optionnelle) et rapport pour un lot d'histoires Telmi.

Usage :
    python compat.py --catalog <url-ou-fichier.json> [--folder <dossier>] [--cache .cache/telmi]
                     [--only <uuid|motif>] [--limit N] [--jobs N] [--force]
                     [--run|--no-run] [--player flam-player.exe] [--run-timeout S]
                     [--max-steps N] [--seed N] [--seeds N] [--run-jobs N]
                     [--reinterpret] [--benign-error REGEX]

Sorties (sous --cache) :
    archives/<uuid>-<version>.zip          archives telechargees
    converted/<uuid>-<version>.plain.pk    histoires converties
    compat/report.json, compat/report.md   rapport
    compat/status.json                     statut par uuid (lu par la surcouche)

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
import threading
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
# Statuts de explore() toujours consideres comme un echec d'execution (`timeout`,
# `max_steps`, `loop` / `exhausted` / `conditions` sans erreur sont des explorations partielles ; `stuck`
# depend de sa raison : voir interpret_run)
RUN_FAIL = ("lua_error", "crash")
# Raisons d'arret d'une boucle SANS erreur (#42) : l'histoire tourne encore mais le
# pilote n'y trouve plus rien de nouveau. `stuck` = raison d'avant #42 (resultats bruts
# conserves), qui confondait boucle et ecran fige.
LOOP_REASONS = ("loop", "stuck")
# Reglages par defaut de l'etape run (~0,6 s par ecran : 1000 actions ~ 10 min)
DEFAULT_RUN_TIMEOUT = 600
DEFAULT_MAX_STEPS = 1000
# Un timeout n'est un avertissement que si l'histoire a vraiment progresse
MIN_SCENES_PARTIAL = 3
# Limites d'extraction (zip-bomb) pour les archives venant d'une source externe
MAX_UNZIPPED = 2 << 30          # 2 Gio decompresses
MAX_ZIP_ENTRIES = 20000
# Messages stderr benins connus : expressions regulieres (insensibles a la casse),
# alimentees au fil du triage (issue #7). Une erreur d'explore() qui correspond a
# l'une d'elles est ignoree. Extensible par --benign-error (repetable).
BENIGN_ERRORS = ()


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


def check_zip_size(path, max_bytes=MAX_UNZIPPED, max_entries=MAX_ZIP_ENTRIES):
    """Leve ValueError si l'archive annonce trop d'entrees ou trop de donnees
    decompressees (protection zip-bomb) ; les zips invalides sont laisses a la conversion."""
    try:
        with zipfile.ZipFile(path) as zf:
            infos = zf.infolist()
    except (zipfile.BadZipFile, OSError):
        return
    total = sum(i.file_size for i in infos)
    if len(infos) > max_entries:
        raise ValueError("archive trop volumineuse : %d entrees (max %d)"
                         % (len(infos), max_entries))
    if total > max_bytes:
        raise ValueError("archive trop volumineuse : %d Mio decompresses (max %d Mio)"
                         % (total >> 20, max_bytes >> 20))


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


_SIG_RULES = (
    (re.compile(r"[A-Za-z]:[\\/][^\s:\"'<>|]*"), "<chemin>"),            # chemins Windows
    (re.compile(r"(?<![\w.])/(?:[\w.@~+-]+/)+[\w.@~+-]*"), "<chemin>"),  # chemins Unix
    (re.compile(r"0x[0-9a-fA-F]+"), "0xN"),
    (re.compile(r"\d+"), "N"),
)


def error_signature(msg):
    """Message d'erreur normalise (sans chemins, adresses, numeros de ligne) pour
    regrouper les memes erreurs d'une histoire a l'autre."""
    sig = str(msg).strip()
    for rx, rep in _SIG_RULES:
        sig = rx.sub(rep, sig)
    return re.sub(r"\s+", " ", sig)[:160]


def group_errors(errors):
    """[{signature, count, example}] par signature normalisee, ordre d'apparition."""
    groups = {}
    for e in errors:
        sig = error_signature(e)
        g = groups.setdefault(sig, {"signature": sig, "count": 0, "example": str(e)[:300]})
        g["count"] += 1
    return list(groups.values())


def _filter_benign(errors, benign):
    """(erreurs conservees, nombre d'erreurs ignorees)."""
    pats = [re.compile(p, re.IGNORECASE) for p in benign]
    keep = [e for e in errors if not any(p.search(str(e)) for p in pats)]
    return keep, len(errors) - len(keep)


def interpret_run(res, benign=BENIGN_ERRORS):
    """Interprete UN resultat d'explore() : (statut ok|warn|fail, [messages]).
    - lua_error, crash, statut inconnu : fail ; stuck (unresponsive : ecran qui ne
      repond plus, no_trace, no_start) : fail ;
    - boucle sans erreur (loop, ou stuck/stuck d'avant #42) : warn si plus de 2 scenes
      visitees (ou toutes), sinon fail ; fail aussi si des retours de secours
      (dead_end_backs > 0) montrent que l'histoire s'est figee ;
    - timeout sans erreur : warn (exploration partielle) si plus de 2 scenes visitees,
      sinon fail (ecran noir, blocage au demarrage) ;
    - ok / max_steps sans erreur : warn si la couverture est partielle, sinon ok ;
    - ok / exhausted (plus rien de nouveau, sans erreur) : warn si aucune fin jouee ou
      couverture partielle, sinon ok ;
    - ok / conditions (#76 : des scènes restent derrière une condition non satisfaite) :
      toujours warn ;
    - lua_error dont toutes les erreurs sont benignes (liste d'exclusions) : erreurs
      ignorees, mais l'exploration qu'elles ont interrompue reste un avertissement."""
    status = res.get("status")
    done = res.get("done_reason")
    raw = res.get("errors")
    errors = [str(x) for x in raw] if isinstance(raw, (list, tuple)) else []
    msgs = [str(res[k]) for k in ("message", "error", "detail") if res.get(k)]
    real, ignored = _filter_benign(errors, benign)
    visited, total = res.get("scenes_visited"), res.get("scenes_total")
    partial = "exploration partielle %s/%s scènes" % (
        visited if visited is not None else "?", total if total is not None else "?")
    if status == "explore_exception":
        return "fail", msgs or ["explore a plante"]
    if status == "lua_error" and errors and not real:
        return "warn", ["explore: lua_error — interrompue sur erreur bénigne "
                        "(%d erreur(s) ignorée(s))" % ignored,
                        "exploration interrompue : " + partial] + msgs
    full = isinstance(visited, int) and isinstance(total, int) and total and visited >= total
    progressed = isinstance(visited, int) and visited >= MIN_SCENES_PARTIAL
    if status in RUN_FAIL or (status == "stuck" and done not in LOOP_REASONS):
        return "fail", ["explore: %s%s" % (status, "/" + done if done else "")] + msgs + real
    if status in ("loop", "stuck"):        # boucle : l'histoire tourne, rien de nouveau
        head = "explore: %s/%s" % (status, done or "?")
        if real:
            return "fail", [head + " avec erreurs"] + msgs + real
        notes = ["%d message(s) bénin(s) ignoré(s)" % ignored] if ignored else []
        stuck_backs = res.get("dead_end_backs")
        if isinstance(stuck_backs, int) and stuck_backs > 0:
            # retours de secours sur un écran sans action : l'histoire s'est figée, ce
            # n'est pas une boucle jouable (un ancien `stuck` n'est pas reclassé)
            why = ("ancien résultat stuck, non reclassé" if status == "stuck"
                   else "boucle avec blocage")
            return "fail", [head, "%s : %d retour(s) de secours sur un écran sans action"
                            % (why, stuck_backs)] + notes + msgs
        if progressed or full:
            return "warn", [head + " — boucle sans nouveauté, sans erreur",
                            ("couverture %s/%s scènes" % (visited, total)) if full else partial,
                            ] + notes + msgs
        return "fail", [head, "boucle sans progression (%s scène(s) visitée(s))" % visited
                        ] + notes + msgs
    if status not in ("ok", "timeout"):
        return "fail", ["explore: %s" % status, "statut explore inconnu"] + msgs + real
    if real:           # statut ok/timeout mais erreurs non benignes dans les traces
        return "fail", ["explore: %s avec erreurs" % status] + msgs + real
    notes = ["%d message(s) bénin(s) ignoré(s)" % ignored] if ignored else []
    if status == "timeout":
        if isinstance(visited, int) and visited >= MIN_SCENES_PARTIAL:
            return "warn", ["explore: timeout", partial] + notes + msgs
        return "fail", ["explore: timeout",
                        "aucune progression (%s scène(s) visitée(s)) : écran noir ou "
                        "blocage au démarrage ?" % visited] + notes + msgs
    if done == "max_steps" and not full:
        return "warn", ["explore: max_steps", partial] + notes + msgs
    if done == "exhausted" and not (full and res.get("end_reached")):
        return "warn", (["explore: exhausted — plus rien de nouveau, sans erreur",
                         partial if not full else "couverture %s/%s scènes" % (visited, total)]
                        + ([] if res.get("end_reached")
                           else ["aucune fin jouée (histoire cyclique ?)"])
                        + notes + msgs)
    if done == "conditions":
        # #76 : des scenes restent derriere une condition / un indexItem que le pilote
        # n'a pas su satisfaire (reprises sans nouveaute) : couverture partielle
        blocked = [str(b) for b in (res.get("blocked") or [])]
        shown = "; ".join(blocked[:BLOCKED_SHOWN]) + (
            " (+%d)" % (len(blocked) - BLOCKED_SHOWN) if len(blocked) > BLOCKED_SHOWN else "")
        return "warn", (["explore: conditions — scènes derrière une condition non satisfaite",
                         partial]
                        + (["bloquées (scène>cible) : " + shown] if blocked else [])
                        + notes + msgs)
    return "ok", notes + msgs


_RANK = {"ok": 0, "warn": 1, "fail": 2}


RAW_ERR_HEAD = 100          # erreurs conservees dans la fiche : les N premieres...
RAW_ERR_TAIL = 100          # ... et les N dernieres
RAW_ERR_LEN = 200           # caracteres par erreur
BLOCKED_KEPT = 100          # scenes bloquees (done_reason conditions) gardees dans la fiche
BLOCKED_SHOWN = 5           # ... et citees dans le message de l'etape run
BLOCKED_STATUS = 50         # ... et exposees dans status.json
_RAW_SCALARS = ("status", "done_reason", "scenes_visited", "scenes_total", "coverage",
                "end_reached", "duration_s", "steps", "ends", "back_ends", "dead_end_backs",
                "choices_explored", "home_escapes", "exit_code", "cond_frontier")


def bound_errors(errors):
    """Premieres et dernieres erreurs, tronquees : la vraie erreur ne se perd pas derriere
    des lignes benignes, et la fiche reste de taille raisonnable."""
    errs = [str(x)[:RAW_ERR_LEN] for x in errors]
    if len(errs) > RAW_ERR_HEAD + RAW_ERR_TAIL:
        errs = errs[:RAW_ERR_HEAD] + errs[-RAW_ERR_TAIL:]
    return errs


def bound_raw(res, seed):
    """Resultat brut d'explore() borne, conserve dans la fiche pour pouvoir le
    reinterpreter (liste benigne modifiee) sans relancer le player."""
    raw = {k: res[k] for k in _RAW_SCALARS
           if k in res and (res[k] is None or isinstance(res[k], (str, int, float, bool)))}
    for k in ("message", "error", "detail"):
        if res.get(k):
            raw[k] = str(res[k])[:RAW_ERR_LEN]
    if isinstance(res.get("errors"), (list, tuple)):
        raw["errors"] = bound_errors(res["errors"])
    if isinstance(res.get("scenes"), list):
        raw["scenes"] = [str(x)[:100] for x in res["scenes"][:5000]]
    if isinstance(res.get("dead_end_scenes"), list):
        raw["dead_end_scenes"] = [str(x)[:RAW_ERR_LEN] for x in res["dead_end_scenes"][:200]]
    if isinstance(res.get("blocked"), list):
        raw["blocked"] = [str(x)[:RAW_ERR_LEN] for x in res["blocked"][:BLOCKED_KEPT]]
    raw["seed"] = seed
    return raw


def build_run_step(raws, params, benign, seconds):
    """Etape run a partir des resultats bruts bornes (un par parcours) : interpretation,
    fusion de la couverture, regroupement des erreurs. Sert aussi a reinterpreter."""
    runs = []                  # (resultat brut, statut, messages, graine)
    for raw in raws:
        st, msgs = interpret_run(raw, benign)
        runs.append((raw, st, msgs, raw.get("seed")))
    pick = runs[0]
    for r in runs:
        if _RANK[r[1]] > _RANK[pick[1]]:
            pick = r
    status, messages = pick[1], list(pick[2])
    if len(runs) > 1 and status != "ok":
        messages = ["[graine %s] %s" % (pick[3], m) for m in messages]
    st = _step(status, messages, seconds)
    results = [r[0] for r in runs]
    st["seeds"] = [r[3] for r in runs]
    if len(runs) == 1:
        st["seed"] = runs[0][3]
    # couverture fusionnee : union des scenes si fournies, sinon le meilleur parcours
    sets = [set(r["scenes"]) for r in results if isinstance(r.get("scenes"), list)]
    counts = [r["scenes_visited"] for r in results if isinstance(r.get("scenes_visited"), int)]
    if sets and len(sets) == len(results):
        visited = len(set().union(*sets))
    elif counts:
        visited = max(counts)
    else:
        visited = None
    totals = [r["scenes_total"] for r in results if isinstance(r.get("scenes_total"), int)]
    total = max(totals) if totals else None
    if visited is not None:
        st["scenes_visited"] = visited
    if total is not None or any("scenes_total" in r for r in results):
        st["scenes_total"] = total
    if visited is not None and total:
        st["coverage"] = round(min(1.0, visited / total), 3)
    if any("end_reached" in r for r in results):
        st["end_reached"] = any(bool(r.get("end_reached")) for r in results)
    for k in ("steps", "ends", "dead_end_backs"):
        vals = [r[k] for r in results if isinstance(r.get(k), int)]
        if vals:
            st[k] = sum(vals)
    dead = sorted({d for r in results for d in (r.get("dead_end_scenes") or [])})
    if dead:
        st["dead_end_scenes"] = dead
    # scenes bloquees : union des parcours, hors cibles atteintes par un autre parcours
    reached = set().union(*sets) if sets and len(sets) == len(results) else set()
    blocked = sorted({b for r in results for b in (r.get("blocked") or [])
                      if r.get("done_reason") == "conditions"
                      and b.split(">", 1)[-1].split("@", 1)[0] not in reached})
    if blocked:
        st["blocked"] = blocked
    if pick[0].get("done_reason"):
        st["done_reason"] = pick[0]["done_reason"]
    if len(runs) > 1:
        st["runs"] = [{"seed": r[3], "status": r[0].get("status"),
                       "done_reason": r[0].get("done_reason"), "result": r[1],
                       "scenes_visited": r[0].get("scenes_visited"),
                       "steps": r[0].get("steps")} for r in runs]
    errs = [str(x) for r in results if isinstance(r.get("errors"), (list, tuple))
            for x in r["errors"]]
    kept, ignored = _filter_benign(errs, benign)
    if kept:
        st["error_signatures"] = group_errors(kept)
    st["benign_ignored"] = ignored
    st["benign"] = list(benign)
    durs = [r["duration_s"] for r in results if isinstance(r.get("duration_s"), (int, float))]
    st["duration_s"] = round(sum(durs), 2) if durs else round(seconds, 2)
    st["params"] = params
    st["raw"] = results
    return st


def run_params(timeout_s, seed, max_steps, seeds):
    return {"timeout_s": timeout_s, "max_steps": max_steps, "seed": seed,
            "seeds": max(1, int(seeds))}


def do_run(pk_path, explore, player_exe, timeout_s, seed, max_steps, seeds=1,
           benign=BENIGN_ERRORS):
    """Etape run : `seeds` parcours (graines seed, seed+1...), couverture fusionnee ;
    s'arrete au premier parcours en echec. Le statut est le pire des parcours. La fiche
    recopie done_reason, steps, ends, seed(s), dead_end_backs, la couverture, les
    erreurs groupees par signature, et le resultat brut borne (`raw`, voir
    reinterpret_run)."""
    if explore is None:
        return _step("skipped", ["pas de fonction explore() fournie"])
    params = run_params(timeout_s, seed, max_steps, seeds)
    t0 = time.time()
    raws = []
    for sd in range(seed, seed + params["seeds"]):
        try:
            res = explore(player_exe, pk_path, timeout_s=timeout_s, seed=sd, max_steps=max_steps)
        except Exception as e:
            raws.append({"status": "explore_exception", "seed": sd, "message":
                         ("explore a plante : %s: %s" % (type(e).__name__, e))[:RAW_ERR_LEN]})
            break
        raw = bound_raw(res if isinstance(res, dict) else {}, sd)
        raws.append(raw)
        if interpret_run(raw, benign)[0] == "fail":
            break
    return build_run_step(raws, params, benign, time.time() - t0)


def reinterpret_run(run, benign):
    """Recalcule l'etape run a partir de son resultat brut conserve, sans relancer le
    player. Retourne None si la fiche n'a pas de brut (etape non executee, ancienne fiche)."""
    raws = run.get("raw") if isinstance(run, dict) else None
    if not raws or run.get("status") == "skipped":
        return None
    return build_run_step(raws, run.get("params"), benign, run.get("seconds", 0.0))


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


def process_story(entry, cache, opener=None, sleep=time.sleep, max_unzipped=MAX_UNZIPPED):
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
            try:
                check_zip_size(zip_path, max_unzipped)
                steps["convert"] = do_convert(zip_path, pk)
            except ValueError as e:
                steps["convert"] = _step("fail", [str(e)])
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
    entry, cache, max_unzipped = args
    return process_story(entry, cache, max_unzipped=max_unzipped)


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
                 player_exe=None, run_timeout=DEFAULT_RUN_TIMEOUT, seed=0,
                 max_steps=DEFAULT_MAX_STEPS, log=print, seeds=1, benign=BENIGN_ERRORS,
                 max_unzipped=MAX_UNZIPPED, run_jobs=1, reinterpret=False):
    """Evalue chaque entree ; ecrit report.json / report.md ; retourne le document.
    Le rapport conserve les fiches precedentes hors selection (--only/--limit) : celles
    evaluees avec un autre convertisseur sont marquees `stale` (perimees) dans le rapport.
    report.json et status.json sont reecrits (atomiquement) apres chaque histoire.
    `seeds` : nombre de parcours par histoire (graines seed, seed+1...).
    `run_jobs` : explorations simultanees (threads, un player sans fenetre chacun).
    `reinterpret` : recalcule l'etape run des fiches reutilisees depuis leur resultat brut
    (fait aussi automatiquement quand la liste benigne a change), sans relancer le player."""
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
        _write_atomic(os.path.join(compat_dir, STATUS_FILE),
                      json.dumps(build_status(doc), ensure_ascii=False, indent=2))
        if md:
            _write_atomic(os.path.join(compat_dir, "report.md"), render_markdown(doc))
        return doc

    params = run_params(run_timeout, seed, max_steps, seeds)
    benign = list(benign)
    results, todo = {}, []
    for e, k in zip(entries, keys):
        old = prev.get(k)
        reusable = (old is not None and not force and old.get("converter_hash") == chash
                    and old.get("fingerprint") == e.get("fingerprint")
                    and old.get("status") != "ECHEC_TELECHARGEMENT"
                    and not (explore is not None
                             and old.get("steps", {}).get("run", {}).get("status") == "skipped"
                             and old.get("steps", {}).get("validate", {}).get("status")
                             in ("ok", "warn"))
                    # run deja fait avec d'autres reglages (ou sans `params` : fiche
                    # anterieure) : a refaire
                    and not (explore is not None
                             and old.get("steps", {}).get("run", {}).get("status") != "skipped"
                             and old.get("steps", {}).get("run", {}).get("params") != params))
        if reusable:
            run = old.get("steps", {}).get("run", {})
            if run.get("raw") and (reinterpret or run.get("benign") != benign):
                new = reinterpret_run(run, benign)
                if new is not None:
                    old = dict(old, steps=dict(old["steps"], run=new))
                    old["status"] = global_status(old["steps"])
                    log("~ %s : run reinterprete (%s)" % (k, old["status"]))
            results[k] = old
            log("= %s : deja evalue (%s)" % (k, old["status"]))
        else:
            todo.append(e)

    lock = threading.Lock()

    def finalize(rec):
        rec["converter_hash"] = chash
        rec["evaluated"] = time.strftime("%Y-%m-%dT%H:%M:%S")
        with lock:
            results[rec["key"]] = rec
            write(results)
            log("%s %s : %s" % ("+" if rec["status"].startswith("OK") else "!", rec["key"],
                                rec["status"]))

    def run_step(rec):
        rec["steps"]["run"] = do_run(rec["output"], explore, player_exe, run_timeout, seed,
                                     max_steps, seeds, benign)
        rec["status"] = global_status(rec["steps"])
        finalize(rec)

    futures = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, run_jobs)) as pool:
        def finish(rec):
            if rec.get("output") and rec["steps"]["validate"]["status"] in ("ok", "warn"):
                if run_jobs > 1:
                    futures.append(pool.submit(run_step, rec))
                else:
                    run_step(rec)
            else:
                finalize(rec)

        if jobs > 1 and len(todo) > 1:
            with concurrent.futures.ProcessPoolExecutor(max_workers=jobs) as ex:
                for rec in ex.map(_worker, [(e, cache, max_unzipped) for e in todo]):
                    finish(rec)
        else:
            for e in todo:
                finish(process_story(e, cache, max_unzipped=max_unzipped))
        for f in futures:
            f.result()
    write(results)
    return write(results, md=True)


STATUS_FILE = "status.json"


def _coverage(run):
    """Bloc `coverage` de status.json, ou None si l'etape run n'a rien mesure."""
    if "scenes_visited" not in run:
        return None
    vis, tot = run.get("scenes_visited"), run.get("scenes_total")
    ratio = run.get("coverage")
    if ratio is None and isinstance(vis, int) and tot:
        ratio = round(vis / tot, 3)
    return {"scenes_visited": vis, "scenes_total": tot, "ratio": ratio,
            "end_reached": run.get("end_reached")}


def _summary(rec):
    """Resume d'une ligne, lisible dans l'interface."""
    st, steps = rec["status"], rec["steps"]
    run = steps.get("run", {})
    if st.startswith("ECHEC"):
        step = {"ECHEC_TELECHARGEMENT": "download", "ECHEC_CONVERSION": "convert",
                "ECHEC_VALIDATION": "validate", "ECHEC_EXECUTION": "run"}[st]
        msgs = steps.get(step, {}).get("messages") or []
        label = {"download": "Téléchargement", "convert": "Conversion",
                 "validate": "Validation", "run": "Exécution"}[step]
        return ("%s en échec : %s" % (label, "; ".join(msgs[:2]) if msgs else "?"))[:200]
    cov = _coverage(run)
    where = (" (%s/%s scènes)" % (cov["scenes_visited"], cov["scenes_total"]
                                  if cov["scenes_total"] is not None else "?")) if cov else ""
    if st == "OK":
        if run.get("status") == "skipped":
            return "Converti et validé ; exécution non testée"
        return "Compatible%s" % where
    for step in STEPS:                       # OK_AVERTISSEMENTS
        s = steps.get(step, {})
        if s.get("status") == "warn":
            if step == "run":
                if s.get("done_reason") in ("loop", "exhausted", "stuck"):
                    return "Jouable, boucle sans erreur%s" % where
                return "Jouable, exploration partielle%s" % where
            msgs = s.get("messages") or []
            return ("Avertissements (%s) : %s" % (step, msgs[0] if msgs else "?"))[:200]
    return "Avertissements"


STATUS_SCHEMA_VERSION = 1


def _version_key(v):
    """Tuple d'entiers pour une version « 1.2.3 » / « 4 » / « v2 », sinon None."""
    m = re.fullmatch(r"v?(\d+(?:\.\d+)*)", str(v).strip())
    return tuple(int(x) for x in m.group(1).split(".")) if m else None


def _newer(a, b):
    """True si la fiche `a` doit remplacer `b` (meme uuid) : version la plus recente si les
    deux se comparent (numeriquement), sinon la derniere evaluee (`evaluated`)."""
    ka, kb = _version_key(a["version"]), _version_key(b["version"])
    if ka is not None and kb is not None and ka != kb:
        return ka > kb
    return (a.get("evaluated") or "") >= (b.get("evaluated") or "")


def build_status(doc):
    """Contenu de status.json (voir README) : enveloppe versionnee
    {schema_version, generated, converter_hash, stories: {uuid: {...}}}."""
    chosen = {}
    for s in doc["stories"]:
        prior = chosen.get(s["uuid"])
        if prior is None or _newer(s, prior):
            chosen[s["uuid"]] = s
    stories = {}
    for uuid, s in chosen.items():
        run = s["steps"].get("run", {})
        stories[uuid] = {
            "version": s["version"],
            "title": s["title"],
            "status": s["status"],
            "summary": _summary(s),
            "coverage": _coverage(run),
            "run": run.get("status", "skipped"),
            "run_reason": run.get("done_reason"),
            "stale": bool(s.get("stale")),
        }
        if run.get("blocked"):
            stories[uuid]["blocked"] = list(run["blocked"][:BLOCKED_STATUS])
    return {"schema_version": STATUS_SCHEMA_VERSION, "generated": doc["generated"],
            "converter_hash": doc["converter_hash"], "stories": stories}


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
    runs = [s for s in stories if s["steps"].get("run", {}).get("status") not in (None, "skipped")]
    if runs:
        L += ["", "## Execution (couverture)", "",
              "| Titre | Scenes visitees | Couverture | Fin atteinte | Fins | Actions | "
              "Arret | Impasses (retours) | Graine(s) | Duree (s) | Resultat |",
              "|---|---|---|---|---|---|---|---|---|---|---|"]
        for s in runs:
            r = s["steps"]["run"]
            tot = r.get("scenes_total")
            end = r.get("end_reached")
            cov = r.get("coverage")
            L.append("| %s | %s/%s | %s | %s | %s | %s | %s | %s | %s | %s | %s |" % (
                _md(s["title"]), r.get("scenes_visited", "?"), tot if tot is not None else "?",
                "%d %%" % round(100 * cov) if isinstance(cov, (int, float)) else "?",
                "oui" if end else ("non" if end is not None else "?"),
                r.get("ends", "?"), r.get("steps", "?"), _md(r.get("done_reason", "?")),
                r.get("dead_end_backs", "?"),
                _md(",".join(str(x) for x in r.get("seeds", []))) or "?",
                r.get("duration_s", "?"), r["status"]))
        cond = [s for s in runs if s["steps"]["run"].get("blocked")]
        if cond:
            L += ["", "## Scenes derriere une condition (arret `conditions`)", ""]
            for s in cond:
                b = s["steps"]["run"]["blocked"]
                L.append("- %s : %s%s" % (
                    _md(s["title"]), _md("; ".join(b[:BLOCKED_SHOWN * 2])).replace("`", "'"),
                    " (+%d)" % (len(b) - BLOCKED_SHOWN * 2) if len(b) > BLOCKED_SHOWN * 2 else ""))
        sigs = {}
        for s in runs:
            for g in s["steps"]["run"].get("error_signatures", []):
                e = sigs.setdefault(g["signature"], {"count": 0, "stories": []})
                e["count"] += g["count"]
                e["stories"].append(s["title"])
        if sigs:
            L += ["", "## Erreurs d'execution par signature", ""]
            for sig, e in sorted(sigs.items(), key=lambda kv: (-kv[1]["count"], kv[0])):
                L.append("- `%s` : %d occurrence(s), %d histoire(s) (%s)" % (
                    _md(sig).replace("`", "'"), e["count"], len(set(e["stories"])),
                    _md(", ".join(sorted(set(e["stories"]))[:5]))))
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


def _find_player(player=None):
    """Chemin de flam-player (--player, $FLAM_BUILD_DIR, <depot>/build) ou None."""
    try:
        import explore
        return explore.find_player(player)
    except Exception:
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
    ap.add_argument("--run", action=argparse.BooleanOptionalAction, default=None,
                    help="etape run (exploration sans fenetre) ; active par defaut si le "
                         "player est trouve, --no-run pour la desactiver")
    ap.add_argument("--player", help="flam-player(.exe) ou son dossier (defaut : "
                                     "$FLAM_BUILD_DIR, sinon <depot>/build)")
    ap.add_argument("--run-timeout", type=int, default=DEFAULT_RUN_TIMEOUT,
                    help="delai de l'exploration par parcours, en s (defaut %(default)s)")
    ap.add_argument("--seed", type=int, default=0, help="graine du premier parcours")
    ap.add_argument("--seeds", type=int, default=1,
                    help="nombre de parcours par histoire (graines seed, seed+1...), "
                         "couverture fusionnee (defaut %(default)s)")
    ap.add_argument("--max-steps", type=int, default=DEFAULT_MAX_STEPS,
                    help="actions maximales par parcours (defaut %(default)s)")
    ap.add_argument("--run-jobs", type=int, default=1,
                    help="explorations simultanees, un player sans fenetre chacun "
                         "(defaut %(default)s ; --jobs ne concerne que telechargement/conversion)")
    ap.add_argument("--reinterpret", action="store_true",
                    help="recalcule l'etape run des fiches existantes depuis leur resultat "
                         "brut conserve, sans relancer le player")
    ap.add_argument("--benign-error", action="append", default=[], metavar="REGEX",
                    help="message d'erreur benin a ignorer (regex, insensible a la casse), "
                         "repetable ; s'ajoute a compat.BENIGN_ERRORS")
    ap.add_argument("--max-unzipped-mb", type=int, default=MAX_UNZIPPED >> 20,
                    help="taille decompressee maximale d'une archive, en Mio (defaut %(default)s)")
    args = ap.parse_args(argv)
    if args.limit is not None and args.limit < 0:
        ap.error("--limit doit etre positif ou nul")
    if args.run_jobs < 1:
        ap.error("--run-jobs doit etre au moins 1")
    if args.seeds < 1:
        ap.error("--seeds doit etre au moins 1")
    if args.run_timeout < 1 or args.max_steps < 1 or args.max_unzipped_mb < 1:
        ap.error("--run-timeout, --max-steps et --max-unzipped-mb doivent etre positifs")
    for rx in args.benign_error:
        try:
            re.compile(rx)
        except re.error as e:
            ap.error("--benign-error %r invalide : %s" % (rx, e))
    if not args.catalog and not args.folder:
        ap.error("au moins un --catalog ou --folder est requis")
    try:
        entries = select(collect(args.catalog, args.folder), args.only, args.limit)
    except (OSError, ValueError) as e:
        print("ERREUR : %s" % e, file=sys.stderr)
        return 2
    explore, player = None, None
    if args.run is not False:
        explore = _load_explore()
        player = _find_player(args.player)
        if explore is None or player is None:
            why = ("explore.py inutilisable" if explore is None else
                   "flam-player introuvable (--player, FLAM_BUILD_DIR ou build/)")
            if args.run:
                print("ERREUR : --run demande mais %s" % why, file=sys.stderr)
                return 2
            print("etape run desactivee : %s" % why, file=sys.stderr)
            explore = None
    if explore is not None and args.run_timeout < args.max_steps * 0.6:
        print("note : --run-timeout %d s est court pour %d actions (~0,6 s par ecran) ; "
              "les grandes histoires finiront en timeout (exploration partielle)"
              % (args.run_timeout, args.max_steps), file=sys.stderr)
    doc = run_campaign(entries, args.cache, max(1, args.jobs), args.force, explore,
                       player, args.run_timeout, args.seed, args.max_steps,
                       seeds=args.seeds, benign=tuple(BENIGN_ERRORS) + tuple(args.benign_error),
                       max_unzipped=args.max_unzipped_mb << 20, run_jobs=args.run_jobs,
                       reinterpret=args.reinterpret)
    bad = sum(1 for s in doc["stories"] if s["status"].startswith("ECHEC"))
    print("%d histoire(s), %d echec(s) -> %s" % (
        len(doc["stories"]), bad, os.path.join(args.cache, "compat", "report.md")))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
