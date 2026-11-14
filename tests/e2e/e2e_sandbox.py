"""
Bac a sable Lua du player (#575) : le player n'offre pas aux scripts plus que
l'appareil (io, os, package, debug a nil ; ni dofile ni loadfile ; require limite
au dossier de l'histoire), pour qu'une histoire malveillante lancee dans le player
n'agisse pas sur le poste.

    python tests/e2e/e2e_sandbox.py --build <dossier contenant flam-player.exe>

Un script par voie d'evasion, lance comme histoire (dossier .plain) ou comme script
direct, avec un dossier de travail propre. Chaque scenario verifie :
  - la reponse des scripts (message d'erreur Lua de l'appareil : "attempt to index a
    nil value (global 'io')", pas un message invente) ;
  - qu'aucun fichier n'est cree, modifie ou supprime hors du dossier de l'histoire
    (instantane du dossier temporaire du test, sentinelle intacte, aucun code de
    l'histoire place hors de son dossier n'a tourne) ;
  - que le player reste en vie jusqu'a "quit" puis s'arrete proprement (code 0).
Scenarios : A) io.open hors de l'histoire ; B) os.execute / remove / rename / getenv /
tmpname ; C) os.exit ; D) package.loadlib et require d'un module C ; E) require("io"),
package.loaded.io et les voies detournees (_G, load, coroutine, metatable des
chaines) ; F) bytecode : load avec les modes "b" / "bt" / defaut, fonction de lecture,
string.dump ; G) loadfile / dofile hors de l'histoire ; H) require("../x") (histoire,
histoire --strict, script direct) ; I) images et sons : chemins absolus, ".." et
antislash refuses (par defaut et en --strict) ; J) usage legitime (require, rechargement
d'un module par package.loaded[nom] = nil, load texte, collectgarbage, chaines) qui doit
continuer a marcher ; K) --unsafe-libs rouvre io / os / package.loadlib / dofile /
string.dump avec un avertissement, mais pas debug ; sans l'option, aucun avertissement.
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

SKIP = 77
BS = chr(92)

# Morceau Lua 5.4 precompile (string.dump(load("return 'BYTECODE_RAN'"), true)) : renvoie
# la chaine BYTECODE_RAN s'il est charge puis execute. Genere avec le Lua 5.4.8 du
# player (x64, petit-boutiste, entiers 64 bits).
BYTECODE_HEX = (
    "1b4c7561540019930d0a1a0a0408087856000000000000000000000028774001808080000102845100"
    "000003000000460002014600010181048d42595445434f44455f52414e810100008080808080")
BYTECODE = bytes.fromhex(BYTECODE_HEX)
FAILS = []


def lua_bytes(data):
    """Litteral Lua "..." avec des echappements decimaux (aucun caractere special)."""
    return '"' + "".join(BS + "%03d" % b for b in data) + '"'


def lua_path(p):
    """Chemin sous forme de chaine longue Lua, separateurs normalises."""
    return "[==[" + p.replace(BS, "/") + "]==]"


def check(cond, what, out=""):
    if cond:
        print("[OK] " + what)
    else:
        print("[ECHEC] " + what)
        if out:
            print("    --- sortie du player ---\n    " + out.replace("\n", "\n    ")[:3000])
        FAILS.append(what)


def be32(n):
    return bytes([(n >> 24) & 255, (n >> 16) & 255, (n >> 8) & 255, n & 255])


def write(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(data if isinstance(data, bytes) else data.encode("utf-8"))


def lif():
    """LIF minimal 2x2 (meme gabarit que tests/lua/regress_hunt1_lif_dims.lua)."""
    return (b"LIFF" + be32(2) + be32(2) + bytes([0xA2]) +
            bytes([0xFE, 0x1F, 0x00, 0xC0 | 0x3F]) + bytes(7) + bytes([1]))


def mp3(frames=8):
    return (bytes([0xFF, 0xFB, 0x90, 0x00]) + bytes(413)) * frames


def snapshot(root):
    """Contenu du dossier temporaire du test, hors sauvegardes du player (saves/)."""
    snap = {}
    for d, dirs, files in os.walk(root):
        rel = os.path.relpath(d, root).replace(BS, "/")
        if rel == "saves" or rel.startswith("saves/"):
            continue
        for n in dirs:
            if rel == "." and n == "saves":
                continue
            snap[(rel + "/" + n).lstrip("./")] = None
        for n in files:
            p = os.path.join(d, n)
            with open(p, "rb") as f:
                snap[(rel + "/" + n).lstrip("./")] = f.read()
    return snap


class Env:
    """Dossier temporaire du test : histoire, fichiers hors histoire, dossier de travail."""

    def __init__(self, exe):
        self.exe = exe
        self.root = tempfile.mkdtemp(prefix="flam-sbx-")
        r = self.root
        self.story = os.path.join(r, "story.plain")
        self.cwd = os.path.join(r, "cwd")
        os.makedirs(self.cwd)
        # hors de l'histoire : sentinelle, code qui ne doit jamais tourner, ressources
        self.sentinel = os.path.join(r, "sentinel.txt")
        write(self.sentinel, "SECRET")
        self.outfile = os.path.join(r, "pwned.txt")            # ne doit jamais exister
        self.outlua = os.path.join(r, "outside.lua")
        write(self.outlua, "OUTSIDE_RAN = true\nreturn 'OUTSIDE_RAN'\n")
        self.outmod = os.path.join(r, "outside_mod.lua")
        write(self.outmod, "OUTSIDE_RAN = true\nreturn true\n")
        self.outluac = os.path.join(r, "outside.luac")
        write(self.outluac, BYTECODE)
        write(os.path.join(r, "outside.lif"), lif())
        write(os.path.join(r, "outside.mp3"), mp3())
        # dans l'histoire : un module, une image, un son (temoins "usage legitime")
        write(os.path.join(self.story, "script", "ok_mod.lua"), "return { ok = true }\n")
        write(os.path.join(self.story, "img", "a.lif"), lif())
        write(os.path.join(self.story, "sounds", "a.mp3"), mp3())
        # module C pour require : copie de la bibliotheque SDL a cote du player, nommee
        # cmod.dll dans le dossier de travail (".\\?.dll" de package.cpath, Windows)
        self.have_cmod = False
        if sys.platform == "win32":
            for n in ("SDL2.dll", "SDL2d.dll"):
                src = os.path.join(os.path.dirname(exe), n)
                if os.path.isfile(src):
                    shutil.copy(src, os.path.join(self.cwd, "cmod.dll"))
                    self.have_cmod = True
                    break
        self.lib = {"win32": "kernel32.dll", "darwin": "libSystem.dylib"}.get(sys.platform, "libc.so.6")

    def sub(self, code):
        for k, v in (("@OUTFILE@", self.outfile), ("@SENTINEL@", self.sentinel),
                     ("@OUTLUAC@", self.outluac), ("@OUTLUA@", self.outlua),
                     ("@OUTDIR@", self.root), ("@LIB@", self.lib)):
            code = code.replace(k, lua_path(v) if k != "@LIB@" else v)
        return code

    def close(self):
        shutil.rmtree(self.root, ignore_errors=True)


def run(env, code, mode="story", extra=(), timeout=25):
    """Lance le player sur `code` ; retourne (sortie, code de sortie ou None, avant, apres)."""
    if mode == "story":
        write(os.path.join(env.story, "main.lua"), env.sub(code))
        target = env.story
    else:
        target = os.path.join(env.root, "direct.lua")
        write(target, env.sub(code))
    # etat initial identique pour chaque scenario (un script non confine peut avoir
    # supprime ou cree des fichiers pendant le scenario precedent)
    write(env.sentinel, "SECRET")
    if os.path.exists(env.outfile):
        os.remove(env.outfile)
    before = snapshot(env.root)
    penv = dict(os.environ)
    penv.update({"SDL_VIDEODRIVER": "dummy", "SDL_AUDIODRIVER": "dummy",
                 "SDL_RENDER_DRIVER": "software", "FLAM_SCREENSHOT_AUTO_MS": "0"})
    proc = subprocess.Popen(
        [env.exe, "--stdin-control", "--watchdog", "5000", *extra,
         "--save-dir", os.path.join(env.root, "saves"), target],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        env=penv, cwd=env.cwd)
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
        deadline = time.monotonic() + timeout
        while (time.monotonic() < deadline and proc.poll() is None
               and not any(l == "DONE" for l in list(lines))):
            time.sleep(0.05)
        if proc.poll() is None:
            time.sleep(0.4)
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
    return "\n".join(lines), rc, before, snapshot(env.root)


def common(env, out, rc, before, after, name):
    """Controles communs : player vivant puis arret propre, rien hors de l'histoire."""
    check("DONE" in out.splitlines() and rc == 0,
          name + " : le player reste en vie jusqu'a quit puis s'arrete proprement (code %r)" % rc, out)
    check(before == after, name + " : aucun fichier cree, modifie ou supprime hors de l'histoire",
          "avant/apres : %r" % sorted(set(before) ^ set(after)))
    check(not os.path.exists(env.outfile), name + " : le fichier hors de l'histoire n'existe pas")
    kept = open(env.sentinel, "rb").read() if os.path.isfile(env.sentinel) else None
    check(kept == b"SECRET", name + " : sentinelle intacte")


NIL_IDX = "attempt to index a nil value (global '%s')"

# ---------------------------------------------------------------------------
SC_IO = r'''
print("IO_TYPE " .. type(io))
local ok, e = pcall(function()
  local f = assert(io.open(@OUTFILE@, "w")); f:write("pwned"); f:close()
end)
print("IO_RESULT " .. tostring(ok) .. " " .. tostring(e))
local ok2, e2 = pcall(function() return io.popen("echo x") end)
print("POPEN " .. tostring(ok2) .. " " .. tostring(e2))
print("DONE")
'''

SC_OS = r'''
print("OS_TYPE " .. type(os))
local function try(label, fn)
  local ok, e = pcall(fn)
  print("OS_" .. label .. " " .. tostring(ok) .. " " .. tostring(e))
end
try("EXECUTE", function() return os.execute('echo pwned > "' .. @OUTFILE@ .. '"') end)
try("REMOVE", function() return os.remove(@SENTINEL@) end)
try("RENAME", function() return os.rename(@SENTINEL@, @OUTFILE@) end)
try("GETENV", function() return os.getenv("PATH") end)
try("TMPNAME", function() return os.tmpname() end)
try("TIME", function() return os.time() end)
try("CLOCK", function() return os.clock() end)
try("DATE", function() return os.date() end)
print("DONE")
'''

SC_EXIT = r'''
print("BEFORE_EXIT")
local ok, e = pcall(function() os.exit(7) end)
print("AFTER_EXIT " .. tostring(ok) .. " " .. tostring(e))
print("DONE")
'''

SC_NATIVE = r'''
print("LOADLIB_TYPE " .. type(package.loadlib))
local ok, e = pcall(function() return package.loadlib("@LIB@", "*") end)
print("LOADLIB_RESULT " .. tostring(ok) .. " " .. tostring(e))
print("CPATH_TYPE " .. type(package.cpath) .. " PATH_TYPE " .. type(package.path))
print("SEARCHPATH_TYPE " .. type(package.searchpath))
local ok2, e2 = pcall(require, "cmod")
print("CMOD " .. tostring(ok2) .. " " .. tostring(e2):gsub("\n", " | "))
print("DONE")
'''

SC_LIBS = r'''
local function t(v) return type(v) end
print("PL_IO " .. t(package.loaded.io) .. " PL_OS " .. t(package.loaded.os) ..
      " PL_DEBUG " .. t(package.loaded.debug))
print("REQ_IO " .. tostring((pcall(require, "io"))))
print("REQ_OS " .. tostring((pcall(require, "os"))))
print("REQ_DEBUG " .. tostring((pcall(require, "debug"))))
print("REQ_PACKAGE_LOADLIB " .. t((package.loaded.package or {}).loadlib))
print("G_IO " .. t(_G.io) .. " " .. t(_G["i" .. "o"]) .. " " .. t(package.loaded._G.io))
print("G_OS " .. t(_G.os) .. " " .. t(_G["o" .. "s"]) .. " " .. t(package.loaded._G.os))
print("LOAD_IO " .. t(load("return io")()))
print("LOAD_ENV_IO " .. t(load("return io", "c", "t", _G)()))
print("CO_IO " .. t(coroutine.wrap(function() return io end)()))
print("STR_MT_DUMP " .. t(getmetatable("").__index.dump) .. " DUMP " .. t(string.dump))
print("DEBUG_G " .. t(debug) .. " " .. t(_G.debug))
print("PRELOAD " .. t(package.preload))
print("SEARCHERS " .. t(package.searchers) .. " " .. #package.searchers)
-- aucun chercheur ne rend une bibliotheque C ou un fichier hors histoire
for i, s in ipairs(package.searchers) do
  local ok, r = pcall(s, "io")
  print("SEARCHER_" .. i .. " " .. tostring(ok) .. " " .. t(r))
end
print("DONE")
'''

SC_BYTECODE = r'''
local BIN = @BIN@
print("DUMP_TYPE " .. type(string.dump))
local function try(label, f, e)
  print("LOAD_" .. label .. " " .. tostring(f ~= nil) .. " " .. tostring(e))
  if f then print("RAN_" .. label .. " " .. tostring(f())) end
end
try("B", load(BIN, "bin", "b"))
try("BT", load(BIN, "bin", "bt"))
try("DEFAULT", load(BIN))
try("NILMODE", load(BIN, "bin", nil))
local sent = false
try("READER", load(function() if not sent then sent = true; return BIN end end, "bin", "b"))
try("T", load(BIN, "bin", "t"))
local ok, e = pcall(function() return load(string.dump(function() return 1 end)) end)
print("DUMP_LOAD " .. tostring(ok) .. " " .. tostring(e))
print("TEXT " .. tostring(load("return 21 * 2")()))
print("TEXT_MODE_T " .. tostring(load("return 5", "c", "t")()))
print("TEXT_ENV " .. tostring(load("return x", "c", "t", { x = 7 })()))
print("TEXT_READER " .. tostring(load(function() if not s2 then s2 = true; return "return 9" end end)()))
print("DONE")
'''

SC_FILES = r'''
print("FILE_TYPES " .. type(dofile) .. " " .. type(loadfile))
local ok, e = pcall(function() return dofile(@OUTLUA@) end)
print("DOFILE " .. tostring(ok) .. " " .. tostring(e))
local ok2, e2 = pcall(function() return loadfile(@OUTLUA@) end)
print("LOADFILE " .. tostring(ok2) .. " " .. tostring(e2))
local ok3, e3 = pcall(function() return dofile(@OUTLUAC@) end)
print("DOFILE_BIN " .. tostring(ok3) .. " " .. tostring(e3))
local ok4, e4 = pcall(function() return dofile("../outside.lua") end)
print("DOFILE_REL " .. tostring(ok4) .. " " .. tostring(e4))
print("OUTSIDE " .. tostring(OUTSIDE_RAN))
print("DONE")
'''

SC_REQUIRE = r'''
local function try(label, name)
  local ok, e = pcall(require, name)
  print("REQ_" .. label .. " " .. tostring(ok) .. " " .. tostring(e):gsub("\n", " | "))
end
try("DOTDOT", "../outside_mod")
try("DOTDOT2", "../../outside_mod")
try("DOTDOT3", "script/../../outside_mod")
try("BACKSLASH", "..\\outside_mod")
try("ABS", @OUTDIR@ .. "/outside_mod")
try("DEVICE", "nul")
print("OUTSIDE " .. tostring(OUTSIDE_RAN))
print("DONE")
'''

SC_ASSETS = r'''
local function img(label, p)
  local ok, d = pcall(lv.img_src.load, p)
  print("IMG_" .. label .. " " .. tostring(ok and d ~= nil))
end
img("INSIDE", "a.lif")
img("DOTDOT", "../../outside.lif")
img("DOTDOT_BS", "..\\..\\outside.lif")
img("ABS", @OUTDIR@ .. "/outside.lif")
img("DEVICE", "NUL")
local function snd(label, p)
  print("SND_" .. label .. " " .. tostring(audio.load(0, p)))
end
snd("INSIDE", "a.mp3")
snd("DOTDOT", "../../outside.mp3")
snd("DOTDOT_BS", "..\\..\\outside.mp3")
snd("ABS", @OUTDIR@ .. "/outside.mp3")
snd("DEVICE", "NUL")
print("DONE")
'''

SC_LEGIT = r'''
local m = require("ok_mod")
print("LEGIT_REQ " .. tostring(m.ok))
package.loaded["ok_mod"] = nil
local m2 = require("ok_mod")
print("LEGIT_RELOAD " .. tostring(m2 ~= m and m2.ok))
print("LEGIT_LOAD " .. load("return 1 + 1")())
print("LEGIT_LOAD_ENV " .. tostring(load("return x", "c", "bt", { x = 3 })()))
print("LEGIT_GC " .. type(collectgarbage("count")) .. " " .. tostring(collectgarbage("collect")))
local o = setmetatable({}, { __index = function(_, k) return k .. "!" end })
print("LEGIT_META " .. o.x .. " " .. tostring(getmetatable(o) ~= nil))
print("LEGIT_STR " .. string.format("%d", 5) .. string.rep("a", 3) .. math.floor(2.5) ..
      #table.concat({ 1, 2 }) .. tostring(utf8 ~= nil))
local n = 0
for _ in pairs(package.loaded) do n = n + 1 end
print("LEGIT_LOADED " .. tostring(n > 0))
print("LEGIT_IMG " .. tostring(lv.img_src.load("a.lif") ~= nil))
print("LEGIT_AUDIO " .. tostring(audio.load(0, "a.mp3")))
print("LEGIT_PCALL " .. tostring((pcall(error, "x"))))
print("DONE")
'''

SC_UNSAFE = r'''
local function t(v) return type(v) end
print("UNSAFE io=" .. t(io) .. " os=" .. t(os) .. " loadlib=" .. t(package.loadlib) ..
      " dofile=" .. t(dofile) .. " loadfile=" .. t(loadfile) .. " dump=" .. t(string.dump) ..
      " debug=" .. t(debug) .. " pl_debug=" .. t(package.loaded.debug))
print("DONE")
'''


def main():
    # sortie du player avec des caracteres hors de la page de code de la console
    sys.stdout.reconfigure(errors="replace")
    ap = argparse.ArgumentParser()
    ap.add_argument("--build", required=True, help="dossier contenant flam-player")
    a = ap.parse_args()
    exe = os.path.join(os.path.abspath(a.build),
                       "flam-player.exe" if sys.platform == "win32" else "flam-player")
    if not os.path.isfile(exe):
        print("[SKIP] e2e_sandbox : %s introuvable" % exe)
        return SKIP
    env = Env(exe)
    try:
        # A : io
        out, rc, b, f = run(env, SC_IO)
        common(env, out, rc, b, f, "A io")
        check("IO_TYPE nil" in out, "A io : `io` vaut nil", out)
        check("IO_RESULT false" in out and NIL_IDX % "io" in out,
              "A io : io.open leve l'erreur de l'appareil (" + NIL_IDX % "io" + ")", out)
        check("POPEN false" in out and NIL_IDX % "io" in out, "A io : io.popen refuse", out)

        # B : os
        out, rc, b, f = run(env, SC_OS)
        common(env, out, rc, b, f, "B os")
        check("OS_TYPE nil" in out, "B os : `os` vaut nil", out)
        for label in ("EXECUTE", "REMOVE", "RENAME", "GETENV", "TMPNAME", "TIME", "CLOCK", "DATE"):
            check(any(l.startswith("OS_%s false" % label) and NIL_IDX % "os" in l for l in out.splitlines()),
                  "B os : os.%s leve l'erreur de l'appareil" % label.lower(), out)

        # C : os.exit
        out, rc, b, f = run(env, SC_EXIT)
        common(env, out, rc, b, f, "C os.exit")
        check("AFTER_EXIT false" in out and NIL_IDX % "os" in out,
              "C os.exit : erreur Lua rattrapable, le player ne s'arrete pas", out)

        # D : code natif
        out, rc, b, f = run(env, SC_NATIVE)
        common(env, out, rc, b, f, "D natif")
        check("LOADLIB_TYPE nil" in out, "D natif : package.loadlib vaut nil", out)
        check("LOADLIB_RESULT false" in out and "attempt to call a nil value (field 'loadlib')" in out,
              "D natif : package.loadlib leve l'erreur Lua d'une valeur nil", out)
        check("CPATH_TYPE nil PATH_TYPE nil" in out and "SEARCHPATH_TYPE nil" in out,
              "D natif : package.cpath, package.path et package.searchpath absents", out)
        check("CMOD false" in out and "luaopen_cmod" not in out and "error loading module" not in out,
              "D natif : require d'un module C ne charge aucune bibliotheque" +
              ("" if env.have_cmod else " (cmod.dll non fourni sur cette plateforme)"), out)

        # E : voies detournees
        out, rc, b, f = run(env, SC_LIBS)
        common(env, out, rc, b, f, "E require(io)")
        check("PL_IO nil PL_OS nil PL_DEBUG nil" in out, "E : package.loaded.io / os / debug valent nil", out)
        check("REQ_IO false" in out and "REQ_OS false" in out and "REQ_DEBUG false" in out,
              "E : require('io'), require('os'), require('debug') echouent", out)
        check("REQ_PACKAGE_LOADLIB nil" in out, "E : package.loaded.package.loadlib vaut nil", out)
        check("G_IO nil nil nil" in out and "G_OS nil nil nil" in out,
              "E : _G.io / _G.os et package.loaded._G.io / os valent nil", out)
        check("LOAD_IO nil" in out and "LOAD_ENV_IO nil" in out and "CO_IO nil" in out,
              "E : load (avec ou sans environnement) et coroutine ne retrouvent pas io", out)
        check("STR_MT_DUMP nil DUMP nil" in out, "E : string.dump absent, y compris par la metatable des chaines", out)
        check("DEBUG_G nil nil" in out, "E : debug absent de _G", out)
        check("PRELOAD table" in out and "SEARCHERS table" in out,
              "E : package.preload et package.searchers existent (le runtime en depend)", out)
        for i in range(1, 4):
            bad = [l for l in out.splitlines() if l.startswith("SEARCHER_%d true" % i) and not l.endswith("string")
                   and not l.endswith("nil") and not l.endswith("function")]
            check(not bad, "E : le chercheur n.%d ne rend ni module ni bibliotheque" % i, out)
        check(not any(l.startswith("SEARCHER_") and l.endswith(" true table") for l in out.splitlines()),
              "E : aucun chercheur ne rend la table io", out)
        n_search = [l for l in out.splitlines() if l.startswith("SEARCHERS table")]
        check(n_search and int(n_search[0].split()[-1]) <= 2,
              "E : seuls package.preload et le chercheur de l'hote restent (%s)" % (n_search[:1],), out)

        # F : bytecode
        src = SC_BYTECODE.replace("@BIN@", lua_bytes(BYTECODE))
        out, rc, b, f = run(env, src)
        common(env, out, rc, b, f, "F bytecode")
        check("DUMP_TYPE nil" in out, "F bytecode : string.dump retire", out)
        for label in ("B", "BT", "DEFAULT", "NILMODE", "READER", "T"):
            check(("LOAD_%s false" % label) in out and ("RAN_%s" % label) not in out,
                  "F bytecode : load refuse un morceau binaire (mode %s)" % label, out)
        check("attempt to load a binary chunk" in out,
              "F bytecode : message de load habituel (attempt to load a binary chunk)", out)
        check("BYTECODE_RAN" not in out, "F bytecode : aucun morceau binaire n'a ete execute", out)
        check("DUMP_LOAD false" in out, "F bytecode : load(string.dump(f)) impossible", out)
        check("TEXT 42" in out and "TEXT_MODE_T 5" in out and "TEXT_ENV 7" in out and "TEXT_READER 9" in out,
              "F bytecode : load de source texte (chaine, mode t, environnement, fonction) inchange", out)

        # G : loadfile / dofile
        out, rc, b, f = run(env, SC_FILES)
        common(env, out, rc, b, f, "G loadfile/dofile")
        check("FILE_TYPES nil nil" in out, "G : dofile et loadfile valent nil (comme l'appareil)", out)
        check("DOFILE false" in out and "attempt to call a nil value (global 'dofile')" in out,
              "G : dofile hors de l'histoire leve l'erreur de l'appareil", out)
        check("LOADFILE false" in out and "DOFILE_BIN false" in out and "DOFILE_REL false" in out,
              "G : loadfile, dofile d'un morceau binaire, dofile('../x') refuses", out)
        check("OUTSIDE nil" in out and "BYTECODE_RAN" not in out and "OUTSIDE_RAN" not in out.replace("OUTSIDE nil", ""),
              "G : aucun code hors de l'histoire n'a tourne", out)

        # H : require("../x") (histoire, histoire strict, script direct)
        for label, mode, extra in (("histoire", "story", ()), ("--strict", "story", ("--strict",)),
                                   ("script direct", "script", ())):
            out, rc, b, f = run(env, SC_REQUIRE, mode, extra)
            common(env, out, rc, b, f, "H require (%s)" % label)
            for k in ("DOTDOT", "DOTDOT2", "DOTDOT3", "BACKSLASH", "ABS", "DEVICE"):
                check(("REQ_%s false" % k) in out, "H require (%s) : require refuse le chemin %s" % (label, k), out)
            check("OUTSIDE nil" in out, "H require (%s) : le module hors de l'histoire n'a pas tourne" % label, out)

        # I : images et sons
        for label, extra in (("defaut", ()), ("--strict", ("--strict",))):
            out, rc, b, f = run(env, SC_ASSETS, "story", extra)
            common(env, out, rc, b, f, "I assets (%s)" % label)
            check("IMG_INSIDE true" in out and "SND_INSIDE 0" in out,
                  "I assets (%s) : image et son du dossier de l'histoire charges" % label, out)
            for k in ("DOTDOT", "DOTDOT_BS", "ABS", "DEVICE"):
                check(("IMG_%s false" % k) in out, "I assets (%s) : image %s refusee" % (label, k), out)
                check(("SND_%s -1" % k) in out, "I assets (%s) : son %s refuse (-1)" % (label, k), out)

        # J : usage legitime
        out, rc, b, f = run(env, SC_LEGIT)
        common(env, out, rc, b, f, "J usage legitime")
        for needle in ("LEGIT_REQ true", "LEGIT_RELOAD true", "LEGIT_LOAD 2", "LEGIT_LOAD_ENV 3",
                       "LEGIT_GC number", "LEGIT_META x! true", "LEGIT_STR 5aaa22true",
                       "LEGIT_LOADED true", "LEGIT_IMG true", "LEGIT_AUDIO 0", "LEGIT_PCALL false"):
            check(needle in out, "J usage legitime : " + needle, out)

        # K : --unsafe-libs
        out, rc, b, f = run(env, SC_UNSAFE, "story", ("--unsafe-libs",))
        check("UNSAFE io=table os=table loadlib=function dofile=function loadfile=function dump=function "
              "debug=nil pl_debug=nil" in out,
              "K --unsafe-libs : io, os, loadlib, dofile, loadfile, string.dump rouverts, debug toujours retire", out)
        check("ATTENTION : --unsafe-libs" in out, "K --unsafe-libs : avertissement sur stderr", out)
        check("DONE" in out.splitlines() and rc == 0, "K --unsafe-libs : le player s'arrete proprement", out)
        out, rc, b, f = run(env, SC_UNSAFE)
        check("UNSAFE io=nil os=nil loadlib=nil dofile=nil loadfile=nil dump=nil debug=nil pl_debug=nil" in out,
              "K defaut : rien n'est rouvert", out)
        check("ATTENTION" not in out, "K defaut : aucun avertissement", out)
    finally:
        env.close()

    if FAILS:
        print("\n%d verification(s) en echec :" % len(FAILS))
        for x in FAILS:
            print("  - " + x)
        return 1
    print("OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
