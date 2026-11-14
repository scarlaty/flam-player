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
     declenchement. Chaque scenario doit etre interrompu et le player quitter ;
  4. F) `debug` n'est pas expose aux scripts (comme sur l'appareil : sandbox sans
     io/os/debug/package), ni par `require("debug")` : `debug.sethook()` retirait
     le watchdog (la boucle `while true do end` qui suit n'etait jamais
     interrompue) et debug.getupvalue / getregistry rendaient les fonctions
     d'origine des gardes de coroutines ;
  5. G) plafond memoire de l'etat Lua (--lua-mem, 256 Mo par defaut) : un
     string.rep de 32 Mo avec --lua-mem 16, fonction C longue invisible au hook
     de comptage, leve "not enough memory" au lieu d'allouer ; l'etat reste
     utilisable ensuite ; H) memoire remplie jusqu'aux derniers octets par le
     script : le player reste en vie (reserve de l'hote hors script, pas de
     PANIC) ; I) valeurs de --lua-mem invalides refusees (code 2 et message) ;
     J) fonction d'avertissement de Lua conservee (warn("@on") + erreur dans un
     __gc : la sortie "error in __gc" apparait).
Limites non couvertes (documentees dans docs/player.md) : boucle dans un
finaliseur __gc (Lua coupe les hooks pendant les finaliseurs) et motif de
string.find a retour arriere catastrophique (temps passe dans une fonction C).
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
# D : des coroutines preparees avant le declenchement : leur reprise (fonction
# rendue par wrap) ne doit pas s'executer (ran reste a 0)
SCEN_D = _lua("ran = 0",
              "local leaves = {}",
              "for i = 1, 2000 do",
              "  leaves[i] = coroutine.wrap(function()",
              "    ran = ran + 1",
              "    while true do pcall(function() while true do end end) end",
              "  end)",
              "end",
              "local root = coroutine.wrap(function()",
              "  pcall(coroutine.wrap(function() while true do end end))",
              "  for i = 1, #leaves do pcall(leaves[i]) end",
              "  print('LEAVES_RAN', ran)",
              "end)",
              "root()")
# E : idem avec coroutine.close et des variables a fermer en attente (__close)
SCEN_E = _lua("ran = 0",
              "local cos = {}",
              "for i = 1, 2000 do",
              "  local co = coroutine.create(function()",
              "    local x <close> = setmetatable({}, {__close = function()",
              "      ran = ran + 1",
              "      while true do pcall(function() while true do end end) end",
              "    end})",
              "    coroutine.yield()",
              "  end)",
              "  coroutine.resume(co)",
              "  cos[i] = co",
              "end",
              "local root = coroutine.wrap(function()",
              "  pcall(coroutine.wrap(function() while true do end end))",
              "  for i = 1, #cos do pcall(coroutine.close, cos[i]) end",
              "  print('CLOSERS_RAN', ran)",
              "end)",
              "root()")
# F : debug.sethook() retire le watchdog quand `debug` est expose
SCEN_F = _lua("print('DEBUG_TYPE ' .. type(debug) .. ' ' .. type(package.loaded.debug))",
              "print('REQUIRE_DEBUG ' .. tostring((pcall(require, 'debug'))))",
              "if debug then debug.sethook() end",
              "while true do end")
# G : fonction C longue, plafonnee par l'allocateur
SCEN_G = _lua("local ok, e = pcall(string.rep, 'x', 32 * 1024 * 1024)",
              "print('REP_RESULT ' .. tostring(ok) .. ' ' .. tostring(e):sub(1, 40))",
              "local small = string.rep('y', 1000)",
              "print('AFTER_OK ' .. #small)")
# H : memoire remplie jusqu'aux derniers octets (chaines de taille decroissante,
# sans fermeture ni concatenation pendant le remplissage), puis retour a l'hote
SCEN_H = _lua("warn('@on')",
              "fill = {}",
              "for i = 1, 100000 do fill[i] = true end",
              "collectgarbage('stop')",
              "local k = 0",
              "local rep, pcall = string.rep, pcall",
              "local function put(v) k = k + 1; fill[k] = v end",
              "local sz = 1 << 20",
              "while sz > 200 do",
              "  local ok, s = pcall(rep, 'x', sz)",
              "  if ok then put(s) else sz = sz // 2 end",
              "end",
              "for len = 200, 41, -1 do",
              "  while true do",
              "    local ok, s = pcall(rep, 'x', len)",
              "    if not ok then break end",
              "    put(s)",
              "  end",
              "end",
              "for len = 40, 1, -1 do",
              "  local ok, s = pcall(rep, 'x', len)",
              "  if ok then put(s) end",
              "end",
              "for i = 1, 90000 do",
              "  local ok, s = pcall(tostring, i)",
              "  if not ok then break end",
              "  put(s)",
              "end",
              # memoire gardee PLEINE jusqu'au retour a l'hote : print alloue (et io n'existe
              # plus), warn sur des litteraux n'alloue pas (sortie : "Lua warning: ...")
              "local ok1 = pcall(rep, 'x', 1 << 20)",
              "if collectgarbage('count') >= 16000 then warn('FILLED_16MB') end",
              "if ok1 then warn('PROBE_1MB true') else warn('PROBE_1MB false') end")
# J : warn() et un finaliseur en erreur
SCEN_J = _lua("warn('@on')",
              "setmetatable({}, {__gc = function() error('boom_gc') end})",
              "collectgarbage()",
              "print('WARN_DONE')")
LEGIT = ('local ok, err = pcall(error, "attendue")\n'
         'local ok2 = pcall(function() return 1 end)\n'
         'print("LEGIT_DONE", ok, ok2)\n')


def s2_dummy(tmp):
    """Script sans effet pour les tests d'arguments (jamais execute si l'argument est refuse)."""
    p = os.path.join(tmp, "noop.lua")
    with open(p, "w", newline="") as f:
        f.write("print('NOOP')\n")
    return p


def check(cond, what, err=""):
    if not cond:
        print("[ECHEC] " + what)
        if err:
            print("--- sortie du player ---\n" + err)
        raise SystemExit(1)
    print("[OK] " + what)


def run(exe, script, tmp, wait_for, extra=()):
    """Lance le player sur script, attend une trace, envoie 'quit'.
    Retourne (sortie, code de sortie ou None si le player ne s'est pas arrete)."""
    env = dict(os.environ)
    env.update({"SDL_VIDEODRIVER": "dummy", "SDL_AUDIODRIVER": "dummy",
                "SDL_RENDER_DRIVER": "software", "FLAM_SCREENSHOT_AUTO_MS": "0"})
    proc = subprocess.Popen(
        [exe, "--stdin-control", "--watchdog", "300", *extra, "--save-dir", tmp, script],
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
        if proc.poll() is None:
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

        for tag, src, trace in (("D", SCEN_D, "LEAVES_RAN"), ("E", SCEN_E, "CLOSERS_RAN")):
            sc = os.path.join(tmp, "coro_%s.lua" % tag)
            with open(sc, "w", newline="") as f:
                f.write(src)
            t0 = time.monotonic()
            out, rc = run(exe, sc, tmp, "Erreur Lua")
            check("watchdog" in out and (trace + "\t0") in out,
                  "coroutines %s : aucune coroutine preparee ne s'execute apres le declenchement" % tag, out)
            check(rc == 0, "coroutines %s : le player quitte (code %r, %.1f s)" % (tag, rc, time.monotonic() - t0), out)

        sf = os.path.join(tmp, "nodebug.lua")
        with open(sf, "w", newline="") as f:
            f.write(SCEN_F)
        out, rc = run(exe, sf, tmp, "Erreur Lua")
        check("DEBUG_TYPE nil nil" in out, "debug absent de _G et de package.loaded", out)
        check("REQUIRE_DEBUG false" in out, "require('debug') refuse", out)
        check("Erreur Lua" in out and "watchdog" in out,
              "debug.sethook() inaccessible : la boucle est interrompue", out)
        check(rc == 0, "debug : le player quitte (code %r)" % rc, out)

        sg = os.path.join(tmp, "mem.lua")
        with open(sg, "w", newline="") as f:
            f.write(SCEN_G)
        out, rc = run(exe, sg, tmp, "AFTER_OK", ("--lua-mem", "16"))
        check("REP_RESULT false not enough memory" in out,
              "string.rep de 32 Mo refuse par --lua-mem 16", out)
        check("AFTER_OK 1000" in out, "etat Lua utilisable apres le refus", out)
        check(rc == 0, "memoire : le player quitte (code %r)" % rc, out)

        sh = os.path.join(tmp, "exhaust.lua")
        with open(sh, "w", newline="") as f:
            f.write(SCEN_H)
        out, rc = run(exe, sh, tmp, "PROBE_1MB", ("--lua-mem", "16", "--watchdog", "120000"))
        check("FILLED_16MB" in out, "memoire remplie jusqu'au plafond (16 Mo)", out)
        check("PROBE_1MB false" in out, "memoire pleine : 1 Mo refuse au script (reserve de l'hote fermee)", out)
        check("PANIC" not in out, "memoire pleine : pas de PANIC de l'etat Lua", out)
        check(rc == 0, "memoire pleine : le player reste en vie et quitte (code %r)" % rc, out)

        sj = os.path.join(tmp, "warn.lua")
        with open(sj, "w", newline="") as f:
            f.write(SCEN_J)
        out, rc = run(exe, sj, tmp, "WARN_DONE")
        check("error in __gc" in out, "warn('@on') : l'erreur du finaliseur est signalee", out)
        check(rc == 0, "warn : le player quitte (code %r)" % rc, out)

        # I : valeurs de --lua-mem invalides
        for bad in ("abc", "-5", "1", "7", "99999999999999999999", "20000", "", "8x"):
            p = subprocess.run([exe, "--lua-mem", bad, s2_dummy(tmp)],
                               capture_output=True, timeout=20,
                               env=dict(os.environ, SDL_VIDEODRIVER="dummy",
                                        SDL_AUDIODRIVER="dummy"))
            err = p.stderr.decode("utf-8", "replace")
            check(p.returncode == 2 and "--lua-mem" in err and "invalide" in err,
                  "--lua-mem %r refuse (code %r)" % (bad, p.returncode), err)
        # sans valeur, en dernier argument : refuse (et non pris pour le chemin de l'histoire)
        p = subprocess.run([exe, s2_dummy(tmp), "--lua-mem"], capture_output=True, timeout=20,
                           env=dict(os.environ, SDL_VIDEODRIVER="dummy", SDL_AUDIODRIVER="dummy"))
        err = p.stderr.decode("utf-8", "replace")
        check(p.returncode == 2 and "demande une valeur" in err,
              "--lua-mem sans valeur refuse (code %r)" % p.returncode, err)

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
