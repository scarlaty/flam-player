#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Valide un homebrew FLAM (.plain.pk) avant de le mettre sur le device.

Usage : python flam_check.py <homebrew.plain.pk>

Controles (ERR = bloque le device a coup sur ou presque, WARN = a verifier) :
  archive   ZIP STORED, sans data descriptor, noms surs, pas de doublon de casse,
            ordre info.plain / version / main.lua en tete, uuid.bin en dernier
  meta      info.plain 4 lignes (titre/sous-titre/000000/titre), version, uuid 16 o
  runtime   framework Lunii present (global.lua & co, img/script/*.lif, img/empty.lif)
  lua       syntaxe (lupa, ou lua/luac du PATH, ou FLAM_LUA), pas de require au
            top-level de main.lua, setup() defini, Global non local, modules
            Global.load_module(...) et require(...) resolus dans script/,
            module : create/clean presents, bibliotheques absentes du device
            (io, os, debug), audio.seek, texte hors police
  assets    en-tetes LIF (magic, 1..2047 px, canal, fin), portrait > 64 px,
            images/audios references en litteral presents, .mp3map coherents
"""
import os
import re
import shutil
import subprocess
import sys
import tempfile
import zipfile

for _s in (sys.stdout, sys.stderr):
    try:
        _s.reconfigure(errors="replace")   # console Windows cp1252 : pas de crash
    except AttributeError:
        pass

LIF_END = b"\x00\x00\x00\x00\x00\x00\x00\x01"
RUNTIME_SCRIPTS = ("global.lua", "progressionManager.lua", "button.lua", "button-theme.lua",
                   "button-theme-default.lua", "button-theme-interactive.lua", "v-scroll.lua",
                   "v-container.lua")
RUNTIME_UI_LIF = ("audio-player-pause.lif",)
_SAFE_PART = re.compile(r"^[A-Za-z0-9_][A-Za-z0-9_.-]*\Z")


# ---------------------------------------------------------------------------
# Mini-lexer Lua : separe code, commentaires et chaines
# ---------------------------------------------------------------------------
_LONG_OPEN = re.compile(r"\[(=*)\[")
_ESC_DEC = re.compile(r"\\(\d{1,3})")


def _unescape_bytes(s):
    r"""Decode les sequences \ddd (octets) d'une chaine courte Lua en texte UTF-8."""
    if "\\" not in s:
        return s
    raw = bytearray()
    i = 0
    while i < len(s):
        m = _ESC_DEC.match(s, i)
        if m and int(m.group(1)) < 256:
            raw.append(int(m.group(1)))
            i = m.end()
        else:
            raw += s[i].encode("utf-8")
            i += 1
    return raw.decode("utf-8", "replace")


def lua_lex(src):
    r"""Retourne (code, nocomment, [(ligne, litteral)]).
    code      : commentaires -> blancs, chaines -> "" (lignes conservees) ;
    nocomment : commentaires -> blancs, chaines gardees ;
    litteraux : contenu des chaines (sequences \ddd decodees)."""
    code, keep, lits = [], [], []
    i, n, line = 0, len(src), 1
    while i < n:
        c = src[i]
        if c == "-" and src.startswith("--", i):
            m = _LONG_OPEN.match(src, i + 2)
            if m:
                end = src.find("]" + m.group(1) + "]", i)
                end = n if end < 0 else end + len(m.group(1)) + 2
            else:
                end = src.find("\n", i)
                end = n if end < 0 else end
            blank = "\n" * src.count("\n", i, end)
            line += len(blank)
            code.append(blank)
            keep.append(blank)
            i = end
        elif c in "\"'":
            j, buf, start = i + 1, [], line
            while j < n and src[j] != c and src[j] != "\n":
                if src[j] == "\\" and j + 1 < n:
                    if src[j + 1] == "\n":      # \ + vrai saut de ligne
                        line += 1
                    elif src[j + 1] == "z":     # \z : saute les blancs suivants
                        k = j + 2
                        while k < n and src[k] in " \t\r\n":
                            line += src[k] == "\n"
                            k += 1
                        j = k
                        continue
                    buf.append(src[j:j + 2])
                    j += 2
                else:
                    buf.append(src[j])
                    j += 1
            lits.append((start, _unescape_bytes("".join(buf))))
            nl = "\n" * (line - start)
            code.append('""' + nl)
            keep.append(src[i:j + 1])
            i = j + 1
        elif c == "[" and _LONG_OPEN.match(src, i):
            m = _LONG_OPEN.match(src, i)
            close = "]" + m.group(1) + "]"
            end = src.find(close, m.end())
            end = n if end < 0 else end
            body = src[m.end():end]
            lits.append((line, body))
            line += body.count("\n")
            code.append('""' + "\n" * body.count("\n"))
            keep.append(src[i:end + len(close)])
            i = end + len(close)
        else:
            if c == "\n":
                line += 1
            code.append(c)
            keep.append(c)
            i += 1
    return "".join(code), "".join(keep), lits


_BLOCK = re.compile(r"\b(function|do|if|repeat|end|until)\b")


def depth_at(code):
    """Fonction pos -> profondeur de bloc (0 = top-level), jeton par jeton."""
    marks = []          # (position, profondeur APRES ce mot-cle)
    depth = 0
    for m in _BLOCK.finditer(code):
        depth += 1 if m.group(1) in ("function", "do", "if", "repeat") else -1
        marks.append((m.start(), depth))

    def at(pos):
        d = 0
        for p, dd in marks:
            if p >= pos:
                break
            d = dd
        return d
    return at


def line_of(text, pos):
    return text.count("\n", 0, pos) + 1


# ---------------------------------------------------------------------------
def lua_syntax(sources):
    """{nom: texte} -> (methode|None, {nom: erreur})."""
    try:
        try:
            from lupa.lua54 import LuaRuntime, LuaError     # lupa >= 2 : Lua 5.4 explicite
        except ImportError:
            from lupa import LuaRuntime, LuaError
        lua = LuaRuntime()
        if "5.4" not in str(lua.eval("_VERSION")):
            raise ImportError("lupa n'embarque pas Lua 5.4")
        errs = {}
        for name, src in sources.items():
            try:
                lua.compile(src)
            except LuaError as e:
                errs[name] = str(e)
        return "lupa", errs
    except ImportError:
        pass
    exe = os.environ.get("FLAM_LUA")
    if not (exe and os.path.isfile(exe)):
        exe = next((shutil.which(c) for c in ("luac5.4", "luac54", "luac", "lua5.4", "lua54", "lua")
                    if shutil.which(c)), None)
    if exe is None:
        return None, {}
    is_luac = "luac" in os.path.basename(exe).lower()
    try:
        v = subprocess.run([exe, "-v"], capture_output=True, text=True, errors="replace", timeout=10)
        if "5.4" not in v.stdout + v.stderr:
            return None, {}          # autre version de Lua : faux positifs possibles
    except (OSError, subprocess.SubprocessError):
        return None, {}
    errs = {}
    with tempfile.TemporaryDirectory() as td:
        for k, (name, src) in enumerate(sources.items()):
            p = os.path.join(td, "f%d.lua" % k)
            with open(p, "wb") as f:
                f.write(src.encode("utf-8"))
            if is_luac:
                cmd = [exe, "-p", p]
            else:
                # compile sans executer : le chemin est dans le code -e (pas de script
                # passe a lua, qui l'executerait)
                lit = "[==[" + p + "]==]"
                cmd = [exe, "-e", "local f,e=loadfile(%s) if not f then io.stderr:write(e) "
                                  "os.exit(1) end os.exit(0)" % lit]
            r = subprocess.run(cmd, capture_output=True, text=True, errors="replace",
                               stdin=subprocess.DEVNULL, timeout=30)
            if r.returncode != 0:
                errs[name] = (r.stderr or r.stdout).strip().replace(p, name)
    return os.path.basename(exe), errs


def check_lif(data):
    if len(data) < 13 + len(LIF_END):
        return "trop court", None
    if data[:4] != b"liff":
        return "magic %r (attendu b'liff')" % data[:4], None
    w, h = int.from_bytes(data[4:8], "big"), int.from_bytes(data[8:12], "big")
    if not (1 <= w <= 2047 and 1 <= h <= 2047):
        return "dimensions %dx%d hors 1..2047" % (w, h), (w, h)
    if data[12] != 0xA2:
        return "canal 0x%02X (attendu 0xA2)" % data[12], (w, h)
    if data[-8:] != LIF_END:
        return "marqueur de fin absent", (w, h)
    return None, (w, h)


def check_mp3map(mm, mp3):
    if len(mm) < 12 or (len(mm) - 12) % 8:
        return "taille %d (12 + 8*N attendu)" % len(mm)
    first = int.from_bytes(mm[4:8], "little")
    if first + 1 >= len(mp3) or mp3[first] != 0xFF or (mp3[first + 1] & 0xE0) != 0xE0:
        return "offset du 1er frame %d ne pointe pas un header MPEG" % first
    prev = -1
    for k in range((len(mm) - 12) // 8):
        off = int.from_bytes(mm[12 + 8 * k:16 + 8 * k], "little")
        if off < prev or off + 1 >= len(mp3) or mp3[off] != 0xFF:
            return "enregistrement %d incoherent (offset %d)" % (k, off)
        prev = off
    return None


# ---------------------------------------------------------------------------
def check(pk_path):
    errs, warns, oks = [], [], []
    try:
        zf = zipfile.ZipFile(pk_path)
    except (OSError, zipfile.BadZipFile) as e:
        print("=== %s ===\n  [ERR]  archive illisible : %s" % (pk_path, e))
        return 1
    infos = zf.infolist()
    names = [i.filename for i in infos]
    nameset = set(names)

    def read(n):
        return zf.read(n) if n in nameset else None

    # --- archive ---
    bad = [i.filename for i in infos if i.compress_type != zipfile.ZIP_STORED]
    if bad:
        errs.append("%d entrees compressees (ZIP_STORED obligatoire), ex. %s" % (len(bad), bad[0]))
    dd = [i.filename for i in infos if i.flag_bits & 0x08]
    if dd:
        errs.append("%d entrees avec data descriptor (bit 3), ex. %s" % (len(dd), dd[0]))
    if not bad and not dd:
        oks.append("%d entrees, toutes STORED sans data descriptor" % len(infos))
    unsafe = [n for n in names if n.endswith("/") or any(not _SAFE_PART.match(p) for p in n.split("/"))]
    if unsafe:
        errs.append("noms refuses (seulement [A-Za-z0-9_.-], pas de dossier vide) : %s" % unsafe[:5])
    lower = {}
    for n in names:
        if n.lower() in lower:
            errs.append("doublon de casse : %s / %s (FAT du device)" % (lower[n.lower()], n))
        lower[n.lower()] = n
    weird = [n for n in names if (".lua" in (n[:-4] if n.endswith(".lua") else n))
             or (n.endswith(".plain") and n != "info.plain")]
    if weird:
        errs.append("noms mal geres par l'import (.lua/.plain hors extension) : %s" % weird[:5])
    if names[:3] != ["info.plain", "version", "main.lua"]:
        warns.append("ordre de tete %s (attendu info.plain, version, main.lua)" % names[:3])
    if not names or names[-1] != "uuid.bin":
        warns.append("uuid.bin n'est pas la derniere entree")

    # --- meta ---
    info = read("info.plain")
    if info is None:
        errs.append("info.plain absent")
    else:
        try:
            lines = info.decode("utf-8").split("\n")
        except UnicodeDecodeError:
            lines = []
            errs.append("info.plain n'est pas en UTF-8")
        if lines:
            if len(lines) != 4 or lines[2] != "000000" or not lines[0].strip():
                errs.append("info.plain : attendu 'TITRE\\nSOUS-TITRE\\n000000\\nTITRE', lu %r"
                            % info[:120])
            else:
                oks.append("info.plain : titre %r" % lines[0])
            out = sorted(set(c for c in "".join(lines) if not (0x20 <= ord(c) <= 0x7E or 0xA0 <= ord(c) <= 0xFF)))
            if out:
                errs.append("info.plain : caracteres hors police %s" % [("U+%04X" % ord(c)) for c in out])
    ver = read("version")
    if ver not in (b"1", b"2"):
        errs.append("version = %r (attendu b'1')" % ver)
    uuid = read("uuid.bin")
    if uuid is None or len(uuid) != 16:
        errs.append("uuid.bin absent ou != 16 octets")
    else:
        oks.append("uuid.bin = %s" % uuid.hex())

    # --- runtime ---
    miss = [("script/" + s) for s in RUNTIME_SCRIPTS if "script/" + s not in nameset]
    miss += [("img/script/" + s) for s in RUNTIME_UI_LIF if "img/script/" + s not in nameset]
    if "img/empty.lif" not in nameset:
        miss.append("img/empty.lif")
    if miss:
        errs.append("framework runtime incomplet : %s" % miss)
    else:
        oks.append("framework runtime present")

    # --- lua ---
    sources = {}
    for n in names:
        if n == "main.lua" or (n.startswith("script/") and n.endswith(".lua")):
            raw = zf.read(n)
            try:
                sources[n] = raw.decode("utf-8")
            except UnicodeDecodeError:
                errs.append("%s : pas en UTF-8 (enregistrer les sources en UTF-8)" % n)
                sources[n] = raw.decode("latin-1")
            if raw.startswith(b"\xef\xbb\xbf"):
                errs.append("%s : BOM UTF-8 en tete (le retirer)" % n)
            if raw.startswith(b"\x1bLua"):
                errs.append("%s : bytecode Lua refuse, livrer le source" % n)
    method, lerrs = lua_syntax(sources)
    if method is None:
        warns.append("syntaxe Lua NON verifiee (python -m pip install lupa, ou luac/lua 5.4 "
                     "dans le PATH, ou FLAM_LUA=chemin ; a defaut : simulateur --strict)")
    else:
        for n, e in sorted(lerrs.items()):
            errs.append("%s : erreur de syntaxe : %s" % (n, e))
        if not lerrs:
            oks.append("%d fichiers Lua : syntaxe OK (%s)" % (len(sources), method))

    lexed = {n: lua_lex(s) for n, s in sources.items()}   # (code, nocomment, lits)

    if "main.lua" not in sources:
        errs.append("main.lua absent")
    else:
        code, _, _ = lexed["main.lua"]
        depth = depth_at(code)
        for m in re.finditer(r"\brequire\b\s*[(\"]", code):
            if depth(m.start()) == 0:
                errs.append("main.lua:%d : require() au top-level (le device n'installe le "
                            "chercheur de modules qu'apres le chargement) -> le deplacer dans "
                            "setup()" % line_of(code, m.start()))
        for m in re.finditer(r"\blocal\s+Global\b", code):
            if depth(m.start()) == 0:
                errs.append("main.lua:%d : 'local Global' -> Global doit etre une globale"
                            % line_of(code, m.start()))
        if re.search(r"\blocal\s+(function\s+setup\b|setup\s*=)", code):
            errs.append("main.lua : setup ne doit pas etre local")
        elif not re.search(r"\bfunction\s+setup\s*\(|(^|[^.\w])setup\s*=\s*function\b", code):
            errs.append("main.lua : function setup() absente (appelee par le firmware)")
        if not re.search(r"Global\s*\.\s*load_module\s*\(", code):
            warns.append("main.lua : aucun Global.load_module(...) -> sur device seul le module "
                         "courant est rendu (UI dessinee hors module = ecran noir)")

    # modules et require resolus dans script/ (hors commentaires)
    for n, (_, keep, _) in lexed.items():
        for m in re.finditer(r"load_module\s*\(\s*[\"']([^\"']+)[\"']\s*,\s*[\"']([^\"']*)[\"']", keep):
            target = "script/%s%s.lua" % (m.group(1), "_" + m.group(2) if m.group(2) else "")
            if target not in nameset:
                errs.append("%s:%d : load_module(%r, %r) -> %s absent"
                            % (n, line_of(keep, m.start()), m.group(1), m.group(2), target))
        for m in re.finditer(r"\brequire\b\s*\(?\s*[\"']([^\"']+)[\"']", keep):
            if "script/%s.lua" % m.group(1) not in nameset:
                errs.append("%s:%d : require(%r) -> script/%s.lua absent (le device ne cherche "
                            "que dans script/)" % (n, line_of(keep, m.start()), m.group(1), m.group(1)))

    # fichiers du jeu = hors framework embarque par le skill
    rt_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "assets", "runtime", "script")
    if os.path.isdir(rt_dir):
        known_runtime = set("script/" + f for f in os.listdir(rt_dir) if f.endswith(".lua"))
    else:
        known_runtime = set("script/" + s for s in RUNTIME_SCRIPTS)
    game_files = [n for n in sources if n not in known_runtime]
    oks.append("fichiers Lua du jeu : %s" % game_files)

    for n in game_files:
        code, _, lits = lexed[n]
        for lib in ("io", "os", "debug"):
            m = re.search(r"(?<![.:\w])%s\s*\.\s*[a-z]+" % lib, code)
            if m:
                errs.append("%s:%d : %s.* absent du device (sandbox : lv.*, string, math, "
                            "table, require)" % (n, line_of(code, m.start()), lib))
        if re.search(r"\baudio\s*\.\s*seek\s*\(", code):
            warns.append("%s : audio.seek pendant la lecture gele le device -> "
                         "audio.pause() ; audio.seek(s) ; audio.play()" % n)
        for ln, s in lits:
            bad_chars = sorted(set(c for c in s if ord(c) > 0xFF or 0x7F <= ord(c) < 0xA0))
            if bad_chars:
                warns.append("%s:%d : texte hors police (rectangle vide si affiche) : %s"
                             % (n, ln, " ".join("U+%04X" % ord(c) for c in bad_chars
                                                if c != "\ufffd") or "octets non UTF-8"))
        if n.startswith("script/") and re.match(r"script/.+_\d+(_\d+)*\.lua$", n):
            if not re.search(r"\bcreate\s*=|function\s+[\w.:]*[.:]create\s*\(", code):
                errs.append("%s : module sans fonction create(args)" % n)
            if not re.search(r"\bclean\s*=|function\s+[\w.:]*[.:]clean\s*\(", code):
                errs.append("%s : module sans fonction clean() (appelee par "
                            "Global.cleanCurrentModule, sinon erreur au retour)" % n)
            rets = [m for m in re.finditer(r"\breturn\b", code)]
            mdepth = depth_at(code)
            last = rets[-1] if rets else None
            if last is None or mdepth(last.start()) != 0 or \
                    not re.match(r"return\s*(\{|[A-Za-z_]\w*\s*;?\s*$)", code[last.start():].strip()):
                errs.append("%s : le module doit se terminer par 'return <table>'" % n)
            nnew = len(re.findall(r"\blv\s*\.\s*timer\s*\.\s*new\b", code))
            if nnew and not re.search(r"\blv\s*\.\s*timer\s*\.\s*del\b", code):
                warns.append("%s : lv.timer.new sans lv.timer.del (timer a detruire dans clean)" % n)
            if "EVENT_KEY" in code and "set_editing" not in code:
                warns.append("%s : EVENT_KEY sans lv.group.set_editing(document, true) : la "
                             "molette deplacera le focus au lieu d'envoyer les touches" % n)

    # references d'assets en litteral (noms complets seulement : pas de nom construit
    # par concatenation ou string.format) ; FAT : comparaison sans la casse
    img_names = set(n[4:].lower() for n in names if n.startswith("img/"))
    snd_names = set(n[7:].lower() for n in names if n.startswith("sounds/"))
    full = re.compile(r"^[A-Za-z0-9_][A-Za-z0-9_./-]*\.(lif|mp3)$")
    for n in game_files:
        for ln, s in lexed[n][2]:
            if not full.match(s):
                continue
            low = s.lower()
            if low.endswith(".lif") and low not in img_names:
                errs.append("%s:%d : image %r absente (chemin relatif a img/, ex. 'perso.lif')"
                            % (n, ln, s))
            if low.endswith(".mp3"):
                if low not in snd_names:
                    errs.append("%s:%d : audio %r absent (chemin relatif a sounds/)" % (n, ln, s))
                elif low + "map" not in snd_names:
                    warns.append("%s:%d : %s sans .mp3map" % (n, ln, s))

    # --- assets ---
    lif_bad = 0
    for n in names:
        if n.startswith("img/") and n.endswith(".lif"):
            e, dims = check_lif(zf.read(n))
            if e:
                errs.append("LIF %s : %s" % (n, e))
                lif_bad += 1
            elif not n.startswith("img/script/") and dims[1] > dims[0] and dims[1] > 64:
                warns.append("LIF %s portrait %dx%d > 64 px : le decodeur le transpose "
                             "(poser sur un canevas carre)" % (n, dims[0], dims[1]))
        elif n.startswith("img/") and not n.endswith(".lif"):
            errs.append("%s : seul le .lif est lisible dans img/" % n)
    if not lif_bad:
        oks.append("en-tetes LIF valides")
    for n in names:
        if n.startswith("sounds/") and n.endswith(".mp3map"):
            mp3 = read(n[:-3])
            if mp3 is None:
                warns.append("%s sans son .mp3" % n)
            else:
                e = check_mp3map(zf.read(n), mp3)
                if e:
                    errs.append("mp3map %s : %s" % (n, e))
    size = os.path.getsize(pk_path)
    if size > 50 * 1024 * 1024:
        warns.append("paquet de %.0f Mo : reduire images/audios" % (size / 1e6))
    zf.close()

    print("=== %s ===" % pk_path)
    for o in oks:
        print("  [OK]   %s" % o)
    for w in warns:
        print("  [WARN] %s" % w)
    for e in errs:
        print("  [ERR]  %s" % e)
    print("--- %d OK, %d warnings, %d erreurs ---" % (len(oks), len(warns), len(errs)))
    return 1 if errs else 0


if __name__ == "__main__":
    if len(sys.argv) != 2:
        print(__doc__.split("\n\n")[1], file=sys.stderr)
        sys.exit(2)
    sys.exit(check(sys.argv[1]))
