"""
Arguments de ligne de commande non ASCII (#648).

    python tests/e2e/e2e_argv_unicode.py --build <dossier contenant flam-player.exe>

SDL2main fournit argv en UTF-8 ; le player le reconvertit en page de code ANSI.
flam-player est lance (pilotes SDL dummy, --stdin-control, puis "quit") sur :
  1. un script au nom accentue (Cafe avec accent aigu) et un nom avec espace et
     accents : le script s'execute et le player sort avec le code 0 (avant la
     correction, le build de debug sortait en 0xC0000374, corruption du tas : les
     chaines de argv de SDL2main etaient remplacees puis liberees par SDL) ;
  2. un script dont le nom contient un caractere absent de la page de code ANSI
     (coche U+2713) : repli sur le nom court 8.3, qui finit toujours par .lua ;
  3. une archive .plain.pk au nom accentue, puis au nom hors page ANSI : le nom
     court (HISTOI~1.PK) ne finit plus par .plain.pk, l'archive est quand meme
     reconnue (extraction dans <nom court sans .pk>), l'histoire est jouee.
Les cas 2 et 3 (noms hors page ANSI) sont sautes, avec une trace, si les noms
courts sont desactives sur le volume temporaire.
Windows seulement. Code de sortie : 0 OK, 1 echec, 77 prerequis absents.
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


def check(cond, what, err=""):
    if not cond:
        print("[ECHEC] " + what)
        if err:
            print("--- sortie du player ---\n" + err)
        raise SystemExit(1)
    print("[OK] " + what)


def short_name_available(path):
    """Vrai si le volume donne un nom court 8.3 different pour path (existant)."""
    import ctypes
    buf = ctypes.create_unicode_buffer(1024)
    n = ctypes.windll.kernel32.GetShortPathNameW(path, buf, 1024)
    return 0 < n < 1024 and buf.value.lower() != path.lower()


def run(exe, target, tmp, marker):
    """Lance le player sur target, attend le marqueur, envoie 'quit'.
    Retourne (sortie, code de sortie ou None)."""
    env = dict(os.environ)
    env.update({"SDL_VIDEODRIVER": "dummy", "SDL_AUDIODRIVER": "dummy",
                "SDL_RENDER_DRIVER": "software", "FLAM_SCREENSHOT_AUTO_MS": "0"})
    proc = subprocess.Popen(
        [exe, "--stdin-control", "--save-dir", os.path.join(tmp, "save"), target],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        env=env, cwd=os.path.dirname(exe))
    lines = []

    def pump(stream):
        for raw in stream:
            lines.append(raw.decode("utf-8", "replace").rstrip())

    threads = [threading.Thread(target=pump, args=(s,), daemon=True)
               for s in (proc.stderr, proc.stdout)]
    for t in threads:
        t.start()
    rc = None
    try:
        deadline = time.monotonic() + 20
        while (time.monotonic() < deadline and proc.poll() is None
               and not any(marker in x for x in lines)):
            time.sleep(0.05)
        time.sleep(0.5)
        if proc.poll() is None:
            try:
                proc.stdin.write(b"quit\n")
                proc.stdin.flush()
            except OSError:
                pass
        try:
            rc = proc.wait(10)
        except subprocess.TimeoutExpired:
            rc = None
    finally:
        if proc.poll() is None:
            proc.kill()
            proc.wait()
    for t in threads:
        t.join(2)
    return "\n".join(lines), rc


def write_lua(path, marker):
    with open(path, "w", encoding="utf-8", newline="") as f:
        f.write("print('%s')\n" % marker)


def write_pk(path, marker):
    with zipfile.ZipFile(path, "w", zipfile.ZIP_STORED) as z:
        z.writestr("main.lua", "print('%s')\n" % marker)


def play(exe, tmp, name, kind, marker):
    path = os.path.join(tmp, name)
    (write_pk if kind == "pk" else write_lua)(path, marker)
    out, rc = run(exe, path, tmp, marker)
    return path, out, rc


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--build", required=True, help="dossier contenant flam-player")
    a = ap.parse_args()
    if sys.platform != "win32":
        print("[SKIP] e2e_argv_unicode : Windows seulement")
        return SKIP
    exe = os.path.join(os.path.abspath(a.build), "flam-player.exe")
    if not os.path.isfile(exe):
        print("[SKIP] e2e_argv_unicode : %s introuvable" % exe)
        return SKIP
    tmp = tempfile.mkdtemp(prefix="flam-argv-")
    try:
        for name, marker in (("Café.lua", "ARGV_CAFE"),
                             ("Histoire été.lua", "ARGV_ETE")):
            _, out, rc = play(exe, tmp, name, "lua", marker)
            check(marker in out, "%s : script execute" % ascii(name), out)
            check(rc == 0, "%s : sortie propre apres quit (code %r)" % (ascii(name), rc), out)

        # Archive au nom accentue : jouee telle quelle ou par nom court selon la page ANSI
        _, out, rc = play(exe, tmp, "Café.plain.pk", "pk", "ARGV_PK_CAFE")
        check("ARGV_PK_CAFE" in out, "Cafe (accent).plain.pk : histoire jouee", out)
        check(rc == 0, "Cafe.plain.pk : sortie propre (code %r)" % rc, out)

        probe = os.path.join(tmp, "Sonde✓.txt")
        with open(probe, "w") as f:
            f.write("x")
        if not short_name_available(probe):
            print("[SKIP] noms courts 8.3 desactives sur le volume : cas hors page ANSI saute")
            return 0
        _, out, rc = play(exe, tmp, "Script✓.lua", "lua", "ARGV_CHECK_LUA")
        check("ARGV_CHECK_LUA" in out, "script avec caractere hors page ANSI : execute", out)
        check(rc == 0, "script hors page ANSI : sortie propre (code %r)" % rc, out)

        _, out, rc = play(exe, tmp, "Histoire✓.plain.pk", "pk", "ARGV_PK_CHECK")
        check("refusee" not in out, "Histoire(coche).plain.pk : cible non refusee", out)
        check("ARGV_PK_CHECK" in out, "Histoire(coche).plain.pk : histoire jouee", out)
        check(rc == 0, "Histoire(coche).plain.pk : sortie propre (code %r)" % rc, out)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    print("OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
