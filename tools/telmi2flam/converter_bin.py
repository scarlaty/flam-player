"""
converter_bin.py - Lancement du binaire Rust `telmi2flam` en processus enfant, pour les outils
de campagne (compat.py, tests d'explore.py). Issue #140, PR 20 de PORT-RUST.md.

Le binaire est cherche, dans l'ordre :
  1. l'argument `explicit` (option --converter des outils) ;
  2. la variable d'environnement FLAM_TELMI2FLAM ;
  3. <depot>/target/release/telmi2flam(.exe), ou $CARGO_TARGET_DIR/release/... si cette
     variable est definie (convention du depot : `cargo build -p telmi2flam --release`).

Protocole interprete comme le launcher (`apps/flam-launcher/src-tauri/src/install/convert.rs`) :
`--progress-json`, evenements JSON sur stdout (start, step, warning, done, error), code de
sortie 0 (succes, a condition que l'evenement `done` ait ete emis et que la sortie existe),
130 (annulation), sinon l'evenement `error` (code, message, exit), sinon `exit_<N>` et la
derniere ligne non vide de stderr.
"""
import json
import os
import subprocess

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
ENV_VAR = "FLAM_TELMI2FLAM"
EXE_NAME = "telmi2flam.exe" if os.name == "nt" else "telmi2flam"
DEFAULT_TIMEOUT = 900   # secondes : une histoire de plusieurs centaines de Mio reste bien en deca


class ConverterNotFound(Exception):
    """Le binaire `telmi2flam` est introuvable ou inutilisable."""


def default_path():
    target = os.environ.get("CARGO_TARGET_DIR") or os.path.join(REPO, "target")
    return os.path.join(target, "release", EXE_NAME)


def find_converter(explicit=None):
    """Chemin du binaire telmi2flam ; leve ConverterNotFound avec un message actionnable."""
    path = explicit or os.environ.get(ENV_VAR) or default_path()
    if os.path.isfile(path):
        return os.path.abspath(path)
    source = ("--converter" if explicit else ENV_VAR if os.environ.get(ENV_VAR)
              else "chemin par defaut")
    raise ConverterNotFound(
        "binaire telmi2flam introuvable (%s) : %s. Le construire avec "
        "`cargo build -p telmi2flam --release`, ou indiquer --converter / %s."
        % (source, path, ENV_VAR))


def _parse_events(stdout):
    events = []
    for line in stdout.splitlines():
        try:
            ev = json.loads(line)
        except ValueError:
            continue
        if isinstance(ev, dict):
            events.append(ev)
    return events


def convert(exe, zip_path, out_path, extra_args=(), timeout=DEFAULT_TIMEOUT):
    """Convertit `zip_path` en `out_path` avec le binaire `exe`.

    Retourne (warnings, error) : `warnings` = messages des evenements `warning` ;
    `error` = None en cas de succes, sinon un libelle "<code> : <message>" (code de l'evenement
    `error`, ou `exit_<N>`, `annule`, `timeout`...). Ne leve que ConverterNotFound (binaire
    non lancable)."""
    out_dir = os.path.dirname(os.path.abspath(out_path))
    os.makedirs(out_dir, exist_ok=True)
    cmd = [exe, zip_path, "-o", out_path, "--progress-json"] + list(extra_args)
    try:
        proc = subprocess.run(cmd, cwd=out_dir, stdin=subprocess.DEVNULL, capture_output=True,
                              timeout=timeout)
    except subprocess.TimeoutExpired:
        _remove(out_path)
        return [], "timeout : conversion interrompue apres %d s" % timeout
    except OSError as e:
        raise ConverterNotFound("impossible de lancer %s : %s" % (exe, e))
    stdout = proc.stdout.decode("utf-8", "replace")
    stderr = proc.stderr.decode("utf-8", "replace")
    events = _parse_events(stdout)
    warnings = [str(e.get("message", e.get("code", "avertissement"))) for e in events
                if e.get("event") == "warning"]
    done = any(e.get("event") == "done" for e in events)
    reported = next((e for e in reversed(events) if e.get("event") == "error"), None)
    code = proc.returncode
    if code == 0 and done and os.path.isfile(out_path):
        return warnings, None
    _remove(out_path)
    if code == 130:
        return warnings, "annule : conversion annulee (code de sortie 130)"
    if reported is not None:
        return warnings, "%s : %s" % (reported.get("code", "unknown"), reported.get("message", ""))
    last = next((ln.strip() for ln in reversed(stderr.splitlines()) if ln.strip()),
                "aucun message")
    return warnings, "exit_%d : %s" % (code, last)


def _remove(path):
    try:
        if os.path.isfile(path):
            os.remove(path)
    except OSError:
        pass
