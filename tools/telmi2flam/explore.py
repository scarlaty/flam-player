"""
explore.py — Explorateur automatique d'une histoire FLAM convertie, sans fenetre.

    python explore.py <histoire.plain | histoire.plain.pk> [--player flam-player.exe]
                      [--timeout 120] [--seed 0] [--max-steps 200] [--back-every 8]
                      [--stuck 30] [--strict] [--json] [--keep]

Lance le vrai flam-player (pilotes SDL "dummy" : ni fenetre ni carte son) sur une
copie de travail de l'histoire (dossier .plain copie, ou .plain.pk extrait) dont
main.lua est suivi d'un pilote Lua generique (timer LVGL) qui joue comme un
utilisateur :
  - menu Start : "Demarrer" / "Reprendre" ; apres une fin, redemarre tant qu'il
    reste des options de choix jamais essayees ;
  - choix (carrousel / image-choice) : option la moins essayee (parcours en
    largeur des options non visitees), egalites tirees au sort selon `seed` ;
  - scene audio : OK (ENTER) si la scene l'accepte, sinon saut en fin de piste
    (pause/seek/play) puis, au besoin, fin de piste simulee ;
  - toutes les `back_every` nouvelles scenes audio, en alternance :
    back_callback() puis "Reprendre", ou l'entree du menu contextuel (M).
Arret : toutes les options rencontrees essayees apres une fin (`complete`,
puis goto_library depuis le menu Start), `max_steps` actions, pas de nouvelle
scene / option / fin pendant 40 + nb de scenes vues actions (`stuck`), sortie
vers la bibliotheque (anciens moteurs), erreur, crash ou delai.
Le pilote trace sur stdout des lignes `[EXPLORE] ...` (scene=, choice, end,
step=, done) ; ce module les analyse pendant l'execution.

Resultat de explore() (dict) :
  status          ok | lua_error | crash | timeout | stuck
  errors          lignes d'erreur (stderr : Erreur Lua, callback error, assert,
                  traceback... ; stdout : attempt to / stack traceback)
  scenes_visited  nb de stages distincts atteints ; scenes_total : nb de stages
                  de nodes.lua (None si illisible)
  end_reached     au moins une fin d'histoire
  duration_s      duree de l'execution du player
  + steps, ends, back_ends (fins causees par un test de retour, non
    comptees), end_reasons, choices_explored, back_tests, menu_tests,
    done_reason, exit_code, workdir (si keep).
Code de sortie du CLI : 0 statut ok, 1 sinon, 77 flam-player introuvable.
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
sys.path.insert(0, HERE)

import validate  # noqa: E402  (parseur de nodes.lua)

SKIP = 77
EXE_NAME = "flam-player.exe" if os.name == "nt" else "flam-player"

# Motifs d'erreur sur stderr (messages de main.c, fw_globals.c, bindings)
ERR_RE = re.compile(r"Erreur|error|ERROR|ASSERT|traceback|attempt to|boucle infinie|"
                    r"cannot open|CRASH|unprotected", re.IGNORECASE)
# Erreurs Lua affichees sur stdout par les modules (print d'un pcall)
LUA_OUT_RE = re.compile(r"attempt to|stack traceback")
END_RE = re.compile(r"fin d'histoire : (.*)$")
TAG = "[EXPLORE] "


def run_player(exe, args, cwd, timeout, until=None, grace=2.0):
    """Lance flam-player sans fenetre ; until() -> True arrete l'attente (le
    player tourne encore `grace` s, pour detecter un crash). stdout/stderr vont
    dans cwd/stdout.log et cwd/stderr.log. Retourne (code de sortie, ou None si
    le player a ete arrete, stdout, stderr, duree)."""
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
                time.sleep(grace)   # laisser la suite se faire (retour bibliotheque, crash ?)
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


def find_player(player=None):
    """flam-player : `player` (fichier ou dossier), sinon $FLAM_BUILD_DIR, sinon
    <depot>/build. None si introuvable."""
    cands = []
    if player:
        cands.append(os.path.join(player, EXE_NAME) if os.path.isdir(player) else player)
    else:
        if os.environ.get("FLAM_BUILD_DIR"):
            cands.append(os.path.join(os.environ["FLAM_BUILD_DIR"], EXE_NAME))
        cands.append(os.path.join(ROOT, "build", EXE_NAME))
    for c in cands:
        if os.path.isfile(c):
            return os.path.abspath(c)
    return None


def lua_str(s):
    return '"' + s.replace("\\", "/").replace('"', '\\"') + '"'


def prepare(story_path, workdir):
    """Copie de travail : dossier .plain copie, ou .plain.pk (ZIP stored)
    extrait. Retourne le chemin du dossier .plain."""
    src = os.path.abspath(story_path).rstrip("\\/")
    name = os.path.basename(src)
    if os.path.isdir(src):
        dst = os.path.join(workdir, name)
        shutil.copytree(src, dst)
        return dst
    if not zipfile.is_zipfile(src):
        raise ValueError("ni dossier .plain ni archive .plain.pk : %s" % story_path)
    dst = os.path.join(workdir, name[:-3] if name.endswith(".pk") else name + ".plain")
    root = os.path.realpath(dst)
    with zipfile.ZipFile(src) as z:
        for info in z.infolist():
            target = os.path.realpath(os.path.join(dst, info.filename))
            if not target.startswith(root + os.sep):
                raise ValueError("entree hors du dossier : %s" % info.filename)
        z.extractall(dst)
    return dst


def count_stages(plain):
    try:
        with open(os.path.join(plain, "script", "nodes.lua"), encoding="utf-8") as f:
            return len(validate.parse_lua_data(f.read()).get("stages") or {})
    except (OSError, ValueError, AttributeError, validate.LuaDataError):
        return None


# Pilote ajoute a la fin de main.lua (copie de travail). Joue l'histoire via
# les vrais modules runtime (clics / touches LVGL) ; '%%' = '%' Lua.
DRIVER = r'''
-- ==== [EXPLORE] pilote automatique (ajoute par tools/telmi2flam/explore.py) ====
do
    local SEED, MAX_STEPS = %(seed)d, %(max_steps)d
    local BACK_EVERY = %(back_every)d     -- test retour / menu M toutes les N scenes audio (0 : jamais)
    local _print = print
    -- stdout redirige = tampon : vider a chaque ligne (le player est tue a la fin)
    print = function(...) _print(...); io.stdout:flush() end
    local function emit(s) _print("[EXPLORE] " .. s); io.stdout:flush() end

    local ctxEntries = nil   -- entrees du menu contextuel (touche M)
    if type(context_menu) == "table" and context_menu.set_entries then
        local _set = context_menu.set_entries
        context_menu.set_entries = function(t) ctxEntries = t; return _set(t) end
    end

    local _setup = setup
    function setup()
        _setup()
        math.randomseed(SEED)
        local N = require("nodes")
        local P = Global.progression
        local done, steps, noProg, ends = false, 0, 0, 0
        local seen, nSeen = {}, 0
        local optCount, pending = {}, 0
        local backTests, nextTest = 0, BACK_EVERY

        local function progress() noProg = 0 end
        local function finish(why)
            done = true
            emit("done reason=" .. why .. " steps=" .. steps .. " visited=" .. nSeen
                 .. " ends=" .. ends .. " pending=" .. pending)
        end
        local function step(what)
            steps = steps + 1
            noProg = noProg + 1
            emit("step=" .. steps .. " " .. what)
        end
        local function visit(id)
            if not seen[id] then seen[id] = true; nSeen = nSeen + 1; progress() end
            emit("scene=" .. tostring(id))
        end

        -- Scenes : toute entree dans un noeud passe par setProgression
        local _setProg = P.setProgression
        P.setProgression = function(d)
            local cf = type(d) == "table" and d.currentFunction
            if type(cf) == "string" and N.stages and N.stages[cf] then visit(cf) end
            return _setProg(d)
        end
        -- Fin d'histoire : endStory() remet la progression a zero (moteur
        -- actuel) ; les anciens moteurs sortaient par goto_library()
        -- Une fin declenchee par un test de retour (transition home) n'est pas
        -- une fin d'histoire jouee : tracee "via=back", non comptee.
        local inBack = false
        local function ended(via)
            if inBack then emit("end via=back"); return end
            ends = ends + 1
            progress()
            emit("end via=" .. via)
        end
        local _reset = P.resetProgress
        P.resetProgress = function(...) ended("reset"); return _reset(...) end
        local _goto = goto_library
        goto_library = function(...)
            if not done then ended("goto_library") end
            return _goto(...)
        end
        -- Chaque ecran = un load_module : generation pour detecter les changements
        local gen = 0
        local _load = Global.load_module
        Global.load_module = function(...) gen = gen + 1; return _load(...) end

        -- Options visibles d'une action (memes conditions que story.lua)
        local function cond(c)
            local inv = state.inv
            local slot = inv and inv[(c.item or 0) + 1]
            local v = slot and slot.value or 0
            local n = c.num or 0
            if c.itemB ~= nil then
                local sb = inv and inv[c.itemB + 1]
                n = sb and sb.value or 0
            end
            local cmp = c.cmp or 2
            if cmp == 0 then return v < n elseif cmp == 1 then return v <= n
            elseif cmp == 2 then return v == n elseif cmp == 3 then return v > n
            elseif cmp == 4 then return v >= n end
            return v ~= n
        end
        local function visibleOf(aid)
            local out = {}
            for _, e in ipairs((N.actions and N.actions[aid]) or {}) do
                local ok = true
                for _, c in ipairs(e.cond or {}) do
                    if not cond(c) then ok = false; break end
                end
                if ok then out[#out + 1] = e end
            end
            return out
        end
        -- Option la moins essayee ; egalites tirees au sort (seed)
        local function choose(aid, n)
            local vis = visibleOf(aid)
            if #vis ~= n then vis = {} end   -- moteur different : cle par position
            local keys, best, cands = {}, nil, {}
            for k = 1, n do
                local st = vis[k] and vis[k].stage
                keys[k] = tostring(aid) .. "#" .. (st and ("s:" .. tostring(st)) or tostring(k))
                if optCount[keys[k]] == nil then optCount[keys[k]] = 0; pending = pending + 1 end
                local c = optCount[keys[k]]
                if best == nil or c < best then best, cands = c, { k }
                elseif c == best then cands[#cands + 1] = k end
            end
            local k = cands[math.random(#cands)]
            return { k = k, n = n, keys = keys, vis = vis, aid = aid, nav = 0 }
        end

        local cur, age, acted, plan = -1, 0, 0, nil
        local function key(m, code) lv.event.send(m.parentContainer, lv.EVENT_KEY, code) end
        local function back(why)
            step("back " .. why)
            inBack = true
            local ok, e = pcall(back_callback)
            inBack = false
            if not ok then error(e, 0) end
        end

        local function onList(m)
            if acted > 0 and age %% 30 ~= 0 then return end
            local btn = m.answerContainer and lv.obj.get_child(m.answerContainer, 0)
            if btn == nil then return end
            local label = m.answers and m.answers[btn] and m.answers[btn].label or "?"
            if label == "Demarrer l'histoire" and ends > 0 then
                if pending == 0 then
                    finish("complete")
                    goto_library()          -- retour bibliotheque (lua_close)
                    return
                end
                noProg = 0                  -- redemarrage voulu : prefixe deja connu
            end
            acted = acted + 1
            step("click " .. label)
            lv.event.send(btn, lv.EVENT_CLICKED)
        end

        local function onChoice(m)
            local n = m.answers and #m.answers or 0
            if n == 0 or m.parentContainer == nil then return end
            if plan == nil then
                plan = choose(state.current_fun, n)
                emit("choice? action=" .. tostring(plan.aid) .. " options=" .. n)
            end
            if age %% 2 ~= 0 then return end
            if plan.sent then
                if age %% 20 == 0 then step("key ENTER (relance)"); key(m, 10) end
                return
            end
            local it = m.answerIterator
            if type(it) == "number" and it ~= plan.k then
                if plan.nav < 2 * n + 4 then
                    plan.nav = plan.nav + 1
                    -- 2 options : navigation bornee (pas de bouclage)
                    if plan.k > it then step("key NEXT"); key(m, 19)
                    else step("key PREV"); key(m, 20) end
                    return
                end
                plan.k = it                 -- navigation sans effet : option affichee
            end
            plan.sent = true
            local k = plan.k
            local okey = plan.keys[k]
            if optCount[okey] == 0 then pending = pending - 1; progress() end
            optCount[okey] = optCount[okey] + 1
            emit("choice action=" .. tostring(plan.aid) .. " option=" .. k .. "/" .. plan.n
                 .. " key=" .. okey)
            if plan.vis[k] then visit(plan.vis[k].stage) end
            step("key ENTER (option " .. plan.k .. "/" .. plan.n .. ")")
            key(m, 10)
        end

        local function onAudio(m)
            -- toutes les BACK_EVERY scenes NOUVELLES : un retour qui termine
            -- l'histoire (home) ne fait pas reboucler les tests sur le prefixe connu
            if acted == 0 and BACK_EVERY > 0 and nSeen >= nextTest then
                nextTest = nSeen + BACK_EVERY
                acted = 1
                backTests = backTests + 1
                if backTests %% 2 == 0 and ctxEntries and ctxEntries[1] and ctxEntries[1].cb then
                    step("menu M : " .. tostring(ctxEntries[1].title))
                    ctxEntries[1].cb()
                else
                    back("(test)")
                end
                return
            end
            if m.okCallback ~= nil then
                if acted == 0 or age %% 20 == 0 then
                    acted = acted + 1
                    step("key ENTER (scene)")
                    key(m, 10)
                end
            elseif m.exitCallback ~= nil then
                if acted == 0 and m.audioStarted then
                    acted = 1
                    local d = audio.duration()
                    if d ~= nil and d > 1.5 then
                        step("seek fin de piste")
                        audio.pause(); audio.seek(math.floor(d - 1)); audio.play()
                    end
                elseif age >= 15 and age %% 15 == 0 then   -- ~1,5 s apres : fin simulee
                    step("fin de piste simulee")
                    m.audioFeedback("stop", audio.duration() or 1)
                end
            elseif age >= 10 and age %% 30 == 10 then
                back("(scene sans OK ni fin d'audio)")
            end
        end

        local function onTitle(m)
            if acted == 0 and age >= 5 and m.isAudioStarted and m.on_encoder_clicked then
                acted = 1
                step("saut du titre")
                m.on_encoder_clicked()
            elseif age >= 50 and age %% 50 == 0 then
                back("(titre)")
            end
        end

        lv.timer.new(function()
            if done then return end
            local m, name = Global.current_module, Global.current_module_name or "?"
            if gen ~= cur then
                emit("module=" .. name .. " prev_ticks=" .. age)
                cur, age, acted, plan = gen, 0, 0, nil
                return
            end
            age = age + 1
            if m == nil or age < 3 then return end
            if steps >= MAX_STEPS then finish("max_steps"); return end
            if noProg > 40 + nSeen then finish("stuck"); return end
            if name:find("list-choice", 1, true) == 1 then onList(m)
            elseif name:find("carousel", 1, true) == 1 or name:find("image-choice", 1, true) == 1 then
                onChoice(m)
            elseif name:find("audio-player", 1, true) == 1 then onAudio(m)
            elseif name == "title-card" then onTitle(m)
            elseif age %% 50 == 0 then back("(ecran " .. name .. ")") end
        end, 100, nil)
        emit("start seed=" .. SEED .. " max_steps=" .. MAX_STEPS)
    end
end
'''


class _Watcher(object):
    """Lit stdout/stderr pendant l'execution et decide quand arreter."""

    def __init__(self, cwd, stuck_s):
        self.out_p = os.path.join(cwd, "stdout.log")
        self.err_p = os.path.join(cwd, "stderr.log")
        self.pos = {self.out_p: 0, self.err_p: 0}
        self.buf = {self.out_p: b"", self.err_p: b""}
        self.stuck_s = stuck_s
        self.t_last = time.time()
        self.done_reason = None
        self.library = False
        self.error = False
        self.quiet = False

    def _lines(self, path):
        try:
            with open(path, "rb") as f:
                f.seek(self.pos[path])
                data = f.read()
        except OSError:
            return []
        self.pos[path] += len(data)
        data = self.buf[path] + data
        *lines, self.buf[path] = data.split(b"\n")
        return [l.decode("utf-8", "replace").rstrip("\r") for l in lines]

    def __call__(self):
        for line in self._lines(self.out_p):
            if line.startswith(TAG):
                self.t_last = time.time()
                if line.startswith(TAG + "done reason="):
                    self.done_reason = line[len(TAG + "done reason="):].split()[0]
            elif LUA_OUT_RE.search(line):
                self.error = True
        for line in self._lines(self.err_p):
            if "returning to story browser" in line:
                self.library = True
            elif ERR_RE.search(line):
                self.error = True
        if time.time() - self.t_last > self.stuck_s:
            self.quiet = True
        return bool(self.done_reason or self.library or self.error or self.quiet)


def explore(player_exe, story_path, timeout_s=120, seed=0, max_steps=200,
            stuck_s=30.0, back_every=8, strict=False, keep=False):
    """Explore une histoire (.plain ou .plain.pk) avec flam-player sans fenetre.
    stuck_s : delai sans aucune ligne du pilote => `stuck`. back_every : test du
    retour / menu M toutes les N nouvelles scenes audio (0 : jamais). strict : option
    --strict du player (require() limite a script/, comme le device)."""
    work = tempfile.mkdtemp(prefix="flam_explore_")
    try:
        plain = prepare(story_path, work)
        total = count_stages(plain)
        with open(os.path.join(plain, "main.lua"), "a", encoding="utf-8", newline="\n") as f:
            f.write(DRIVER % {"seed": int(seed), "max_steps": int(max_steps),
                                 "back_every": max(0, int(back_every))})
        watch = _Watcher(work, stuck_s)
        args = (["--strict"] if strict else []) + [plain]
        code, out, err, dt = run_player(player_exe, args, work, timeout_s, until=watch)
        watch()   # dernieres lignes
        return _result(code, out, err, dt, total, watch, work if keep else None)
    finally:
        if not keep:
            shutil.rmtree(work, ignore_errors=True)


def _result(code, out, err, dt, total, watch, workdir):
    tagged = [l.rstrip("\r")[len(TAG):] for l in out.splitlines() if l.startswith(TAG)]
    scenes = {t[len("scene="):] for t in tagged if t.startswith("scene=")}
    choices = {t.rsplit(" key=", 1)[-1] for t in tagged if t.startswith("choice action=")}
    steps = [t for t in tagged if t.startswith("step=")]
    ends = [t for t in tagged if t.startswith("end via=") and t != "end via=back"]
    reasons = []
    for l in out.splitlines():
        m = END_RE.search(l)
        if m and m.group(1).strip() not in reasons:
            reasons.append(m.group(1).strip())
    errors = [l.rstrip("\r") for l in err.splitlines() if ERR_RE.search(l)]
    errors += [l.rstrip("\r") for l in out.splitlines()
               if not l.startswith(TAG) and LUA_OUT_RE.search(l)]
    started = any(t.startswith("start ") for t in tagged)

    if code is not None:
        status = "crash"
        errors.append("flam-player arrete seul (code %s)" % code)
    elif errors:
        status = "lua_error"
    elif not started:
        status = "stuck"
        errors.append("pilote jamais demarre (setup() non atteint ?)")
    elif watch.done_reason == "stuck" or (watch.quiet and not watch.done_reason
                                          and not watch.library):
        status = "stuck"
    elif watch.done_reason or watch.library:
        status = "ok"
    else:
        status = "timeout"
    res = {
        "status": status,
        "errors": errors[:50],
        "scenes_visited": len(scenes),
        "scenes_total": total,
        "end_reached": bool(ends),
        "duration_s": round(dt, 1),
        "steps": len(steps),
        "ends": len(ends),
        "back_ends": tagged.count("end via=back"),
        "end_reasons": reasons,
        "choices_explored": len(choices),
        "back_tests": sum(1 for s in steps if " back (test)" in s),
        "menu_tests": sum(1 for s in steps if " menu M : " in s),
        "done_reason": watch.done_reason or ("library" if watch.library else None),
        "exit_code": code,
    }
    if workdir:
        res["workdir"] = workdir
    return res


def main(argv=None):
    ap = argparse.ArgumentParser(description="Explore une histoire FLAM avec flam-player sans fenetre")
    ap.add_argument("story", help="dossier .plain ou archive .plain.pk")
    ap.add_argument("--player", help="flam-player.exe ou son dossier (defaut : $FLAM_BUILD_DIR, puis build/)")
    ap.add_argument("--timeout", type=float, default=120, help="delai max en s (defaut 120)")
    ap.add_argument("--seed", type=int, default=0, help="graine des tirages (defaut 0)")
    ap.add_argument("--max-steps", type=int, default=200, help="actions max du pilote (defaut 200)")
    ap.add_argument("--stuck", type=float, default=30, help="s sans trace du pilote => stuck (defaut 30)")
    ap.add_argument("--back-every", type=int, default=8,
                    help="test retour / menu M toutes les N scenes audio, 0 = jamais (defaut 8)")
    ap.add_argument("--strict", action="store_true", help="flam-player --strict (require limite a script/)")
    ap.add_argument("--json", action="store_true", help="resultat en JSON")
    ap.add_argument("--keep", action="store_true", help="garder le dossier de travail (logs)")
    a = ap.parse_args(argv)
    exe = find_player(a.player)
    if exe is None:
        print("flam-player introuvable (--player ou FLAM_BUILD_DIR)", file=sys.stderr)
        return SKIP
    try:
        res = explore(exe, a.story, timeout_s=a.timeout, seed=a.seed, max_steps=a.max_steps,
                      stuck_s=a.stuck, back_every=a.back_every, strict=a.strict, keep=a.keep)
    except (OSError, ValueError, zipfile.BadZipFile) as e:
        print("ERREUR : %s" % e, file=sys.stderr)
        return 1
    if a.json:
        print(json.dumps(res, ensure_ascii=False, indent=2))
    else:
        print("%s : %s en %.1f s, %d actions" % (os.path.basename(a.story), res["status"],
                                                res["duration_s"], res["steps"]))
        print("  scenes %s/%s, choix essayes %d, fins %d (%s), tests retour %d / menu M %d"
              % (res["scenes_visited"], res["scenes_total"] if res["scenes_total"] is not None else "?",
                 res["choices_explored"], res["ends"], "; ".join(res["end_reasons"]) or "-",
                 res["back_tests"], res["menu_tests"]))
        print("  arret : %s" % (res["done_reason"] or "-"))
        for e in res["errors"]:
            print("  ! " + e)
        if res.get("workdir"):
            print("  dossier conserve : %s" % res["workdir"])
    return 0 if res["status"] == "ok" else 1


if __name__ == "__main__":
    sys.exit(main())
