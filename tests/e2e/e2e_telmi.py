"""
Test de bout en bout : paquet TELMI synthetique -> telmi2flam -> flam-player.

    python tests/e2e/e2e_telmi.py --build <dossier contenant flam-player.exe>

Etapes :
  1. genere un paquet TELMI (generateurs de tools/telmi2flam/tests) :
     demarrage -> noeud de passage sans audio (F01) -> scene audio -> choix a
     2 options (carrousel) -> option 2 -> scene -> fin d'histoire (F08) ;
  2. le convertit (telmi2flam.convert) en .plain.pk + dossier .plain ;
  3. A (fumee) : flam-player sur le .plain.pk (extraction pk_reader, title-card
     sans title.mp3 -> silent.mp3 -> menu Start), quelques secondes ;
  4. B (parcours) : flam-player sur une copie du .plain dont main.lua est
     suivi d'un pilote Lua (timer LVGL) qui joue l'histoire comme un
     utilisateur : Demarrer -> scene -> choix n.2 -> scene -> fin -> menu
     Start (de nouveau "Demarrer") -> goto_library (retour bibliotheque :
     fermeture du lua_State).
Pilotes SDL "dummy" (video et audio) : ni fenetre ni carte son.
Echec si : crash/sortie anticipee, erreur Lua ou assertion sur stderr,
parcours non termine dans le delai.
Code de sortie : 0 OK, 1 echec, 77 prerequis absents (Pillow, flam-player).
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
TOOL = os.path.join(ROOT, "tools", "telmi2flam")
sys.path.insert(0, TOOL)
sys.path.insert(0, os.path.join(TOOL, "tests"))
sys.dont_write_bytecode = True

SKIP = 77

try:
    import test_telmi2flam as G  # generateurs png() / mp3_frames()
    import telmi2flam as T
except ImportError as e:  # Pillow absent
    print("[SKIP] e2e : import impossible (%s)" % e)
    sys.exit(SKIP)

# Motifs d'erreur sur stderr (messages de main.c, fw_globals.c, bindings)
ERR_RE = re.compile(r"Erreur|error|ERROR|ASSERT|traceback|attempt to|boucle infinie|"
                    r"cannot open|CRASH|unprotected", re.IGNORECASE)


def story_files():
    """Paquet TELMI : start -> p0 (sans audio) -> s0 -> choix {s1, s2} ;
    s1 -> fin, s2 -> s3 -> fin. Les options sont jouees (audio de focus) par
    le carrousel ; OK suit leur transition ok."""
    ctl = {"ok": True, "home": False, "autoplay": True}
    # options d'un menu : non autoplay (sinon TELMI enchaine sur l'index sans
    # afficher le choix, F27) ; OK en fin d'audio pour avancer
    opt = {"ok": True, "home": False, "autoplay": False}
    mp3 = G.mp3_frames(n=40)   # ~1 s
    nodes = {
        "startAction": {"action": "a0", "index": 0},
        "stages": {
            "p0": {"image": None, "audio": None, "ok": {"action": "a1", "index": 0},
                   "home": None, "control": ctl},
            "s0": {"image": "s0.png", "audio": "s0.mp3", "ok": {"action": "a2", "index": 0},
                   "home": None, "control": ctl},
            "s1": {"image": "s1.png", "audio": "s1.mp3", "ok": None, "home": None, "control": opt},
            "s2": {"image": "s2.png", "audio": "s2.mp3", "ok": {"action": "a3", "index": 0},
                   "home": None, "control": opt},
            "s3": {"image": "s0.png", "audio": "s3.mp3", "ok": None, "home": None, "control": ctl},
        },
        "actions": {
            "a0": [{"stage": "p0"}],
            "a1": [{"stage": "s0"}],
            "a2": [{"stage": "s1"}, {"stage": "s2"}],
            "a3": [{"stage": "s3"}],
        },
    }
    notes = {"s1": {"title": "Option un"}, "s2": {"title": "Option deux"}}
    return [
        ("metadata.json", json.dumps({"title": "E2E Test", "uuid": "e2e-0001"})),
        ("nodes.json", json.dumps(nodes)),
        ("notes.json", json.dumps(notes)),
        ("title.png", G.png(64, 48)),
        ("images/s0.png", G.png(320, 240, (10, 200, 10, 255))),
        ("images/s1.png", G.png(320, 240, (10, 10, 200, 255))),
        ("images/s2.png", G.png(320, 240, (200, 200, 10, 255))),
        ("audios/s0.mp3", mp3),
        ("audios/s1.mp3", mp3),
        ("audios/s2.mp3", mp3),
        ("audios/s3.mp3", mp3),
    ]


# Pilote ajoute a la fin de main.lua (copie) : joue l'histoire via les vrais
# modules runtime (clics / touches LVGL), ecrit son resultat dans un fichier.
DRIVER = r'''
-- ==== [E2E] pilote automatique (ajoute par tests/e2e/e2e_telmi.py) ====
do
    local OUT = %(out)s
    local function report(s)
        local f = io.open(OUT, "a"); if f then f:write(s, "\n"); f:close() end
        print("E2E " .. s); io.stdout:flush()
    end
    local _setup = setup
    function setup()
        _setup()
        local seq, cur, age, stage, done = {}, nil, 0, 0, false
        local chose, afterChoice = false, false
        local ticks = 0
        local timer
        timer = lv.timer.new(function()
            if done then return end
            ticks = ticks + 1
            if ticks > 240 then   -- ~60 s
                done = true; report("FAIL delai depasse : " .. table.concat(seq, " > "))
                return
            end
            local m, name = Global.current_module, Global.current_module_name or "?"
            if m == nil then return end
            local box = m.answerContainer or m.parentContainer or m
            local id = name .. "@" .. tostring(box)
            if id ~= cur then
                cur, age, stage = id, 0, 0
                seq[#seq + 1] = name
                report("step " .. name)
                if chose and name:match("^audio%%-player") then afterChoice = true end
                return
            end
            age = age + 1
            if age < 4 then return end          -- ~1 s : animations, audio d'intro
            if name == "list-choice_1_0_0" and stage == 0 then
                local btn = lv.obj.get_child(m.answerContainer, 0)
                local label = btn and m.answers[btn] and m.answers[btn].label or "?"
                if label == "Demarrer l'histoire" and afterChoice then
                    done = true
                    report("DONE " .. table.concat(seq, " > "))
                    goto_library()            -- retour bibliotheque (lua_close)
                    return
                end
                stage = 1
                report("click " .. label)
                lv.event.send(btn, lv.EVENT_CLICKED)
            elseif name == "carousel_1_0_0" or name == "image-choice_1_0_0" then
                if stage == 0 then
                    stage = 1
                    report("key NEXT")
                    lv.event.send(m.parentContainer, lv.EVENT_KEY, 19)
                elseif stage == 1 and age >= 6 then
                    stage = 2
                    chose = true
                    report("key ENTER (option " .. tostring(m.answerIterator) .. ")")
                    lv.event.send(m.parentContainer, lv.EVENT_KEY, 10)
                end
            elseif name:match("^audio%%-player") and m.parentContainer and age >= 12 and stage < 3 then
                -- scene non autoplay : toujours la apres ~3 s (audio ~1 s) => OK
                stage = stage + 1
                report("key ENTER (fin de scene)")
                lv.event.send(m.parentContainer, lv.EVENT_KEY, 10)
            end
        end, 250, nil)
    end
end
'''


def lua_str(s):
    return '"' + s.replace("\\", "/").replace('"', '\\"') + '"'


def run_player(exe, args, cwd, timeout, until=None):
    """Lance flam-player ; until() -> True arrete l'attente. Retourne
    (code ou None si tue, stdout, stderr, duree)."""
    env = dict(os.environ)
    env.setdefault("SDL_VIDEODRIVER", "dummy")
    env.setdefault("SDL_AUDIODRIVER", "dummy")
    # le pilote video dummy n'a pas de rendu accelere : forcer le rendu
    # logiciel (sinon SDL_CreateRenderer(ACCELERATED) echoue)
    env.setdefault("SDL_RENDER_DRIVER", "software")
    out_p, err_p = os.path.join(cwd, "stdout.log"), os.path.join(cwd, "stderr.log")
    with open(out_p, "wb") as fo, open(err_p, "wb") as fe:
        t0 = time.time()
        p = subprocess.Popen([exe] + args, cwd=cwd, stdout=fo, stderr=fe, env=env)
        code = None
        while time.time() - t0 < timeout:
            code = p.poll()
            if code is not None:
                break
            if until is not None and until():
                time.sleep(2.0)   # laisser le retour bibliotheque se faire (crash ?)
                code = p.poll()
                break
            time.sleep(0.2)
        if code is None:
            p.kill()
            p.wait()
        dt = time.time() - t0
    with open(out_p, "rb") as f:
        out = f.read().decode("utf-8", "replace")
    with open(err_p, "rb") as f:
        err = f.read().decode("utf-8", "replace")
    return code, out, err, dt


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--build", required=True, help="dossier contenant flam-player.exe")
    ap.add_argument("--keep", action="store_true", help="garder le dossier temporaire")
    a = ap.parse_args()
    exe = os.path.join(a.build, "flam-player.exe")
    if not os.path.isfile(exe):
        print("[SKIP] e2e : %s introuvable" % exe)
        return SKIP

    tmp = tempfile.mkdtemp(prefix="flam_e2e_")
    fails = []
    try:
        # 1-2. paquet TELMI -> FLAM
        zpath = os.path.join(tmp, "e2e.zip")
        with zipfile.ZipFile(zpath, "w") as z:
            for n, d in story_files():
                z.writestr(n, d)
        conv = os.path.join(tmp, "conv")
        os.makedirs(conv)
        pk = os.path.join(conv, "E2E.plain.pk")
        T.convert(zpath, pk, emit_plain=True)
        plain = pk[:-3]
        print("[e2e] converti : %s" % pk)

        # 3. A : fumee sur le .plain.pk (extraction par le player)
        runA = os.path.join(tmp, "A")
        os.makedirs(runA)
        pkA = os.path.join(runA, "E2E.plain.pk")
        shutil.copy(pk, pkA)
        code, out, err, dt = run_player(exe, [pkA], runA, 6.0)
        print("[e2e] A : %s apres %.1f s" % ("toujours actif (arrete)" if code is None
                                               else "sortie code %s" % code, dt))
        if code is not None:
            fails.append("A : flam-player s'est arrete seul (code %s)" % code)
        if "Loading story" not in out:
            fails.append("A : histoire non chargee (pas de 'Loading story')")
        if not os.path.isfile(os.path.join(runA, "E2E.plain", "main.lua")):
            fails.append("A : .plain.pk non extrait")
        bad = [l for l in err.splitlines() if ERR_RE.search(l)]
        if bad:
            fails.append("A : erreurs sur stderr :\n    " + "\n    ".join(bad[:20]))

        # 4. B : parcours complet pilote
        runB = os.path.join(tmp, "B")
        plainB = os.path.join(runB, "E2E.plain")
        shutil.copytree(plain, plainB)
        result = os.path.join(runB, "e2e_result.txt")
        with open(os.path.join(plainB, "main.lua"), "a", encoding="utf-8", newline="\n") as f:
            f.write(DRIVER % {"out": lua_str(result)})

        def finished():
            if not os.path.isfile(result):
                return False
            with open(result, encoding="utf-8") as f:
                txt = f.read()
            return "DONE" in txt or "FAIL" in txt

        code, out, err, dt = run_player(exe, [plainB], runB, 90.0, until=finished)
        steps = open(result, encoding="utf-8").read() if os.path.isfile(result) else ""
        print("[e2e] B : %.1f s, pilote :\n    %s" % (dt, "\n    ".join(steps.splitlines()) or "(rien)"))
        if code is not None:
            fails.append("B : flam-player s'est arrete seul (code %s)" % code)
        if "DONE" not in steps:
            fails.append("B : parcours non termine")
        else:
            seq = steps.split("DONE", 1)[1]
            for need in ("list-choice", "audio-player", "carousel"):
                if need not in seq:
                    fails.append("B : module %s jamais affiche" % need)
            if "option 2" not in steps:
                fails.append("B : option 2 non choisie")
            if "Current function : s3" not in out:
                fails.append("B : scene s3 (suite de l'option 2) jamais jouee")
            if seq.count("carousel") != 1:
                fails.append("B : parcours inattendu (%d choix)" % seq.count("carousel"))
        if "returning to story browser" not in err:
            fails.append("B : pas de retour bibliotheque apres la fin")
        bad = [l for l in err.splitlines() if ERR_RE.search(l)]
        if bad:
            fails.append("B : erreurs sur stderr :\n    " + "\n    ".join(bad[:20]))
        lua_err = [l for l in out.splitlines() if re.search(r"attempt to|stack traceback", l)]
        if lua_err:
            fails.append("B : erreurs Lua sur stdout :\n    " + "\n    ".join(lua_err[:20]))
        if fails and not a.keep:
            print("[e2e] stderr B (fin) :\n" + "\n".join(err.splitlines()[-30:]))
    finally:
        if a.keep or fails:
            print("[e2e] dossier conserve : %s" % tmp)
        else:
            shutil.rmtree(tmp, ignore_errors=True)

    if fails:
        for f in fails:
            print("[ECHEC] " + f)
        return 1
    print("[OK] e2e telmi2flam -> flam-player")
    return 0


if __name__ == "__main__":
    sys.exit(main())
