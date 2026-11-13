"""
Watchdog Lua du player (#575) : un pcall() ne doit pas avaler le watchdog.

    python tests/e2e/e2e_watchdog.py --build <dossier contenant flam-player.exe>

flam-player est lance sur un script .lua (pilotes SDL dummy, --stdin-control,
--watchdog 300) :
  1. boucle infinie protegee par pcall (while true do pcall(<boucle infinie>) end) :
     le watchdog declenche, le script est interrompu pour de bon et le player
     repond a "quit" (avant la correction : la boucle repartait apres chaque
     erreur avalee, le player ne rendait jamais la main) ;
  2. script legitime qui utilise pcall (erreur attrapee) : termine sans
     declenchement du watchdog ;
  3. boucles infinies reparties sur plusieurs coroutines (le hook est propre a
     chaque lua_State) : A) coroutine bloquee puis boucle pcall dans l'etat
     principal ; B) gestionnaire d'erreur de xpcall qui est une coroutine en
     boucle pcall ; C) coroutines neuves creees en boucle sous pcall apres le
     declenchement. Chaque scenario doit etre interrompu et le player quitter.
Code de sortie : 0 OK, 1 echec, 77 prerequis absents (flam-player).
"""
import argparse
import os
import subprocess
import sys
import tempfile
import threading
import time

SKIP = 77

LOOP_IN_PCALL = "while true do pcall(function() while true do end end) end\n"
def _lua(*lines):
    return "\n".join(lines) + "\n"


# A : le declenchement a lieu dans la coroutine, la boucle pcall est dans l'etat principal
SCEN_A = _lua("pcall(coroutine.wrap(function() while true do end end))",
              "while true do pcall(function() while true do end end) end")
# B : le gestionnaire d'erreur est une coroutine qui boucle sous pcall
SCEN_B = _lua("local B = coroutine.wrap(function()",
              "  while true do pcall(function() while true do end end) end",
              "end)",
              "xpcall(function() while true do end end, B)",
              "while true do end")
# C : apres le declenchement, des coroutines neuves sont creees en boucle sous pcall
SCEN_C = _lua("local function spawn()",
              "  return coroutine.wrap(function() while true do pcall(function() while true do end end) end end)",
              "end",
              "pcall(spawn())",
              "while true do pcall(function() pcall(spawn()) end) end")
LEGIT = ('local ok, err = pcall(error, "attendue")\n'
         'local ok2 = pcall(function() return 1 end)\n'
         'print("LEGIT_DONE", ok, ok2)\n')


def check(cond, what, err=""):
    if not cond:
        print("[ECHEC] " + what)
        if err:
            print("--- sortie du player ---\n" + err)
        raise SystemExit(1)
    print("[OK] " + what)


def run(exe, script, tmp, wait_for):
    """Lance le player sur script, attend une trace, envoie 'quit'.
    Retourne (sortie, code de sortie ou None si le player ne s'est pas arrete)."""
    env = dict(os.environ)
    env.update({"SDL_VIDEODRIVER": "dummy", "SDL_AUDIODRIVER": "dummy",
                "SDL_RENDER_DRIVER": "software", "FLAM_SCREENSHOT_AUTO_MS": "0"})
    proc = subprocess.Popen(
        [exe, "--stdin-control", "--watchdog", "300", "--save-dir", tmp, script],
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
               and not any(wait_for in x for x in lines)):
            time.sleep(0.05)
        time.sleep(0.5)
        try:
            proc.stdin.write(b"quit\n")
            proc.stdin.flush()
        except OSError:
            pass
        try:
            rc = proc.wait(8)
        except subprocess.TimeoutExpired:
            rc = None
    finally:
        if proc.poll() is None:
            proc.kill()
            proc.wait()
    for t in threads:
        t.join(2)
    return "\n".join(lines), rc


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--build", required=True, help="dossier contenant flam-player")
    a = ap.parse_args()
    exe = os.path.join(os.path.abspath(a.build),
                       "flam-player.exe" if sys.platform == "win32" else "flam-player")
    if not os.path.isfile(exe):
        print("[SKIP] e2e_watchdog : %s introuvable" % exe)
        return SKIP
    tmp = tempfile.mkdtemp(prefix="flam-wd-")
    try:
        s1 = os.path.join(tmp, "loop.lua")
        with open(s1, "w", newline="") as f:
            f.write(LOOP_IN_PCALL)
        out, rc = run(exe, s1, tmp, "watchdog")
        check("watchdog" in out, "boucle dans pcall : watchdog declenche", out)
        check(rc == 0, "boucle dans pcall : le player rend la main et quitte (code %r)" % rc, out)
        check("Erreur Lua" in out, "boucle dans pcall : le script est interrompu", out)

        for tag, src in (("A", SCEN_A), ("B", SCEN_B), ("C", SCEN_C)):
            sc = os.path.join(tmp, "coro_%s.lua" % tag)
            with open(sc, "w", newline="") as f:
                f.write(src)
            out, rc = run(exe, sc, tmp, "Erreur Lua")
            check("Erreur Lua" in out and "watchdog" in out,
                  "coroutines %s : script interrompu par le watchdog" % tag, out)
            check(rc == 0, "coroutines %s : le player quitte (code %r)" % (tag, rc), out)

        s2 = os.path.join(tmp, "legit.lua")
        with open(s2, "w", newline="") as f:
            f.write(LEGIT)
        out, rc = run(exe, s2, tmp, "LEGIT_DONE")
        check("LEGIT_DONE" in out, "script avec pcall : execute jusqu'au bout", out)
        check("watchdog" not in out, "script avec pcall : pas de declenchement", out)
        check(rc == 0, "script avec pcall : sortie propre", out)
    finally:
        import shutil
        shutil.rmtree(tmp, ignore_errors=True)
    print("OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
