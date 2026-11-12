"""
Extraction d'un .plain.pk et dossier .plain d'auteur (#567).

    python tests/e2e/e2e_pk_extract.py --build <dossier contenant flam-player.exe>

flam-player est lance sur un .plain.pk (pilotes SDL dummy, --stdin-control) :
  1. dossier .plain d'auteur (main.lua, aucun temoin) a cote du .pk : il est
     joue tel quel, jamais ecrase, et aucun temoin n'est cree ;
  2. dossier absent : extraction complete, temoin .extracted, pas de .extracting ;
  3. .extracting present (extraction interrompue) : re-extraction ;
  4. .extracted different de l'empreinte du .pk : re-extraction ;
  5. .extracted a jour : le dossier n'est pas touche.
Code de sortie : 0 OK, 1 echec, 77 prerequis absents (flam-player).
"""
import argparse
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import zipfile

SKIP = 77
LUA = 'local l = lv.label.new(window)\nlv.label.set_text(l, "pk")\n-- %s\n'


def make_pk(path, tag):
    with zipfile.ZipFile(path, "w", zipfile.ZIP_STORED) as z:
        z.writestr("main.lua", LUA % tag)


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="") as f:
        f.write(text)


def read(path):
    with open(path, encoding="utf-8", newline="") as f:
        return f.read()


def check(cond, what, err=""):
    if not cond:
        print("[ECHEC] " + what)
        if err:
            print("--- stderr du player ---\n" + err)
        raise SystemExit(1)
    print("[OK] " + what)


def run_player(exe, pk, tmp, wait_for):
    """Lance le player sur pk, attend une trace, l'arrete par 'quit'."""
    env = dict(os.environ)
    env.update({"SDL_VIDEODRIVER": "dummy", "SDL_AUDIODRIVER": "dummy",
                "SDL_RENDER_DRIVER": "software", "FLAM_SCREENSHOT_AUTO_MS": "0"})
    proc = subprocess.Popen(
        [exe, "--stdin-control", "--save-dir", tmp, pk], stdin=subprocess.PIPE,
        stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, env=env,
        cwd=os.path.dirname(exe))
    lines = []

    def pump():
        for raw in proc.stderr:
            lines.append(raw.decode("utf-8", "replace").rstrip())

    t = threading.Thread(target=pump, daemon=True)
    t.start()
    try:
        deadline = time.monotonic() + 20
        while (time.monotonic() < deadline and proc.poll() is None
               and not any(wait_for in x for x in lines)):
            time.sleep(0.05)
        time.sleep(0.3)
        if proc.poll() is None:
            try:
                proc.stdin.write(b"quit\n")
                proc.stdin.flush()
            except OSError:
                pass
            try:
                proc.wait(10)
            except subprocess.TimeoutExpired:
                pass
    finally:
        if proc.poll() is None:
            proc.kill()
            proc.wait()
    t.join(2)
    return "\n".join(lines)


def fresh(tmp, name):
    root = os.path.join(tmp, name)
    shutil.rmtree(root, ignore_errors=True)
    os.makedirs(root)
    pk = os.path.join(root, "foo.plain.pk")
    make_pk(pk, "PK")
    return root, pk, os.path.join(root, "foo.plain")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--build", required=True, help="dossier contenant flam-player")
    a = ap.parse_args()
    exe = os.path.join(os.path.abspath(a.build),
                       "flam-player.exe" if sys.platform == "win32" else "flam-player")
    if not os.path.isfile(exe):
        print("[SKIP] e2e_pk_extract : %s introuvable" % exe)
        return SKIP
    tmp = tempfile.mkdtemp(prefix="flam-pkx-")
    try:
        # 1. dossier d'auteur
        root, pk, d = fresh(tmp, "auteur")
        write(d + "/main.lua", LUA % "AUTEUR")
        write(d + "/notes.txt", "a moi")
        err = run_player(exe, pk, tmp, "dossier d'auteur")
        check("dossier d'auteur" in err, "trace du dossier d'auteur", err)
        check(read(d + "/main.lua") == LUA % "AUTEUR", "main.lua d'auteur intact", err)
        check(read(d + "/notes.txt") == "a moi", "fichier d'auteur intact", err)
        check(not os.path.exists(d + "/.extracted")
              and not os.path.exists(d + "/.extracting"),
              "aucun temoin cree dans le dossier d'auteur", err)

        # 2. dossier absent
        root, pk, d = fresh(tmp, "absent")
        err = run_player(exe, pk, tmp, "Extracted")
        check(read(d + "/main.lua") == LUA % "PK", "extraction d'un dossier absent", err)
        check(os.path.isfile(d + "/.extracted"), "temoin .extracted pose", err)
        check(not os.path.exists(d + "/.extracting"), "pas de .extracting", err)

        # 5. a jour : modification locale conservee
        write(d + "/main.lua", LUA % "LOCAL")
        err = run_player(exe, pk, tmp, "Extracted")
        check(read(d + "/main.lua") == LUA % "LOCAL",
              "dossier a jour non touche", err)

        # 4. empreinte differente
        write(d + "/.extracted", "0 0\n")
        err = run_player(exe, pk, tmp, "Extracted")
        check(read(d + "/main.lua") == LUA % "PK", "empreinte differente : re-extraction", err)
        check(not os.path.exists(d + "/.extracting"), "pas de .extracting apres re-extraction", err)

        # 3. extraction interrompue
        root, pk, d = fresh(tmp, "interrompu")
        write(d + "/main.lua", LUA % "PARTIEL")
        write(d + "/.extracting", "")
        err = run_player(exe, pk, tmp, "Extracted")
        check(read(d + "/main.lua") == LUA % "PK", ".extracting : re-extraction", err)
        check(os.path.isfile(d + "/.extracted")
              and not os.path.exists(d + "/.extracting"),
              "temoins apres reprise", err)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    print("e2e_pk_extract : OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
