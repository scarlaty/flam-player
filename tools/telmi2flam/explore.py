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
  - choix (carrousel / image-choice) : option jamais essayee, puis la moins
    essayee dans l'etat d'inventaire courant, puis la plus proche de nouveau
    dans le graphe, puis la moins essayee ; egalites tirees au sort selon `seed` ;
  - scene audio : OK (ENTER) si la scene l'accepte, sinon saut en fin de piste
    (pause/seek/play) puis, au besoin, fin de piste simulee ;
  - toutes les `back_every` nouvelles scenes audio, en alternance :
    back_callback() puis "Reprendre", ou l'entree du menu contextuel (M) ;
  - noeud repasse en boucle sans nouveaute : sortie par sa transition home (#42).
Arret : toutes les options rencontrees essayees et aucune transition ok connue
non suivie (scene suivante jamais atteinte) apres une fin (`complete`,
puis goto_library depuis le menu Start) ; une transition ok / home conditionnelle ou
indexItem vers une scene jamais vue (#76) fait redemarrer l'histoire (le choix prefere
les options jamais essayees dans l'etat d'inventaire courant), au plus 3 fois sans
aucune nouveaute, puis `conditions` (et non `complete`), boucle ou plus rien n'est a essayer
(`exhausted` ; sans fin jouee, home conditionnelle et indexItem ne comptent pas, #142),
`max_steps` actions, pas de nouvelle scene / option / fin pendant
40 + nb de scenes vues actions (`loop` si l'histoire avance encore, sinon
`unresponsive`), sortie vers la bibliotheque (anciens moteurs), erreur, crash
ou delai.
Le pilote trace sur stdout des lignes `[EXPLORE] ...` (scene=, choice, end,
step=, done) ; ce module les analyse pendant l'execution.

Resultat de explore() (dict) :
  status          ok | lua_error | crash | timeout | loop | stuck
  done_reason     cause de l'arret, toujours presente : complete, exhausted (boucle
                  sans erreur ou plus rien n'est a essayer), conditions (apres une fin,
                  des scenes restent derriere une condition / un indexItem que 3
                  redemarrages n'ont pas satisfait : couverture partielle, #76),
                  max_steps, library
                  (status ok) ; loop (plus rien de nouveau, l'histoire tourne encore :
                  status loop) ; unresponsive (ecran qui ne repond plus), no_trace,
                  no_start (stuck) ;
                  timeout ; error (lua_error) ; exit (player sorti seul :
                  lua_error si une erreur Lua a ete vue avant, sinon crash)
  errors          lignes d'erreur (stderr : Erreur Lua, callback error, assert,
                  traceback... ; stdout : attempt to / stack traceback)
  scenes_visited  nb de stages distincts reellement atteints ; scenes (leurs
                  identifiants, pour fusionner plusieurs parcours) ; scenes_total :
                  nb de stages de nodes.lua (None si illisible) ; coverage
  end_reached     au moins une fin d'histoire jouee
  duration_s      duree de l'execution du player
  seed, steps, ends, back_ends (fins causees par un test de retour, non
  comptees), end_reasons, choices_explored, back_tests, menu_tests,
  dead_end_backs / dead_end_scenes (retours de secours sur un ecran sans
  action possible), home_escapes (sorties de boucle par home), exit_code,
  cond_frontier (scenes visitees dont une transition dependant de l'etat mene a une
  scene jamais atteinte, a l'arret ; None sans trace `done`), blocked (au plus 100 :
  "<scene>><cible>", suffixe "@home" pour une transition home ; ce que `conditions`
  n'a pas pu atteindre, #142), workdir (si keep).
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
# Erreur Lua identifiee (distingue lua_error de crash quand le player sort)
LUA_ERR_RE = re.compile(r"Erreur Lua|Erreur dans setup|callback error|attempt to|"
                        r"stack traceback|watchdog")
END_RE = re.compile(r"fin d'histoire : (.*)$")
TAG = "[EXPLORE] "
# Statut d'une raison d'arret du pilote (les autres : ok). `stuck` : raison des
# traces d'avant #42 (boucle et ecran fige confondus).
DONE_STATUS = {"loop": "loop", "unresponsive": "stuck", "stuck": "stuck"}


def run_player(exe, args, cwd, timeout, until=None, grace=2.0):
    """Lance flam-player sans fenetre ; until() -> True arrete l'attente (le
    player tourne encore `grace` s, pour detecter un crash). stdout/stderr vont
    dans cwd/stdout.log et cwd/stderr.log. Retourne (code de sortie, ou None si
    le player a ete arrete, stdout, stderr, duree). Le player est toujours
    arrete en sortie, meme sur exception (until(), Ctrl+C) : pas d'orphelin."""
    env = dict(os.environ)
    env.setdefault("SDL_VIDEODRIVER", "dummy")
    env.setdefault("SDL_AUDIODRIVER", "dummy")
    # le pilote video dummy n'a pas de rendu accelere : forcer le rendu
    # logiciel (sinon SDL_CreateRenderer(ACCELERATED) echoue)
    env.setdefault("SDL_RENDER_DRIVER", "software")
    # pas de ressource partagee entre players simultanes : captures d'ecran dans le dossier
    # de travail (la capture auto par defaut ecrit dans un chemin global), sauvegardes
    # relatives au cwd, aucun port
    env["FLAM_SCREENSHOT_AUTO_MS"] = "0"
    env["FLAM_SCREENSHOT"] = os.path.join(cwd, "screenshot.bmp")
    out_p, err_p = os.path.join(cwd, "stdout.log"), os.path.join(cwd, "stderr.log")
    with open(out_p, "wb") as fo, open(err_p, "wb") as fe:
        t0 = time.time()
        p = subprocess.Popen([exe] + args, cwd=cwd, stdout=fo, stderr=fe, env=env)
        code = None
        try:
            while time.time() - t0 < timeout:
                code = p.poll()
                if code is not None:
                    break
                if until is not None and until():
                    time.sleep(grace)   # laisser la suite se faire (retour bibliotheque, crash ?)
                    code = p.poll()
                    break
                time.sleep(0.2)
        finally:
            if p.poll() is None:
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


# Frontiere de l'exploration (Lua pur : testable avec lupa, sans player).
# newFrontier(N, seen, shown) : N = nodes.lua charge, seen = scenes visitees
# (id -> true), shown = actions de choix deja affichees (id d'action -> true).
FRONTIER_LUA = r'''
local function newFrontier(N, seen, shown)
    local F = {}
    local function unseen(e) return N.stages and N.stages[e.stage] and not seen[e.stage] end
    -- Transition home utilisable (memes regles que homeOf de story.lua :
    -- ctrl.home, home definie, ne ramenant pas sur le noeud courant)
    function F.hasHome(sid, aid)
        local st = N.stages and N.stages[sid]
        if not (st and st.home and st.ctrl and st.ctrl.home) then return false end
        local a = st.home.action
        if a == aid then return false end
        local list = N.actions and N.actions[a]
        if not list then return false end
        for _, e in ipairs(list) do
            if e.stage == sid then return false end
        end
        return true
    end
    -- Transitions ok connues mais jamais suivies : pour chaque scene visitee,
    -- l'une des entrees sans condition de son action pas encore atteinte (sauf si
    -- c'est un choix deja affiche : suivi par `pending`). Un test de
    -- retour peut couper un chemin lineaire : sans ce compte, `complete` serait
    -- declare alors que la suite du chemin n'a pas ete jouee (#45). Les
    -- transitions indexItem / conditionnelles dependent de l'etat : voir condFrontier.
    function F.frontier()
        local n = 0
        for id in pairs(seen) do
            local st = N.stages and N.stages[id]
            local tr = st and st.ok
            local list = tr and tr.indexItem == nil and N.actions and N.actions[tr.action]
            -- choix deja affiche : ses options sont suivies par `pending`
            if list and not (#list > 1 and shown[tr.action]) then
                for _, e in ipairs(list) do
                    if not e.cond and unseen(e) then n = n + 1; break end
                end
            end
        end
        return n
    end
    -- Home d'une scene menant a une scene jamais atteinte (entree sans condition)
    function F.homeNew(sid, aid)
        if not F.hasHome(sid, aid) then return false end
        for _, e in ipairs(N.actions[N.stages[sid].home.action]) do
            if not e.cond and unseen(e) then return true end
        end
        return false
    end
    -- Scenes visitees dont la home mene a du nouveau (entree sans condition) :
    -- rien n'est epuise tant qu'il en reste (le pilote les tente en boucle)
    function F.homeFrontier()
        local n = 0
        for id in pairs(seen) do
            if F.homeNew(id, nil) then n = n + 1 end
        end
        return n
    end
    -- Une transition (ok ou home) dependant de l'etat vers du nouveau : entree
    -- conditionnelle vers une scene jamais atteinte, ou transition indexItem (l'objet
    -- designe l'entree) dont une entree n'a jamais ete atteinte. Un autre etat
    -- d'inventaire peut y mener (hubs a etat, #76) ; les entrees de home comptent
    -- aussi (#87), si la home est utilisable. Mode `strict` (histoire sans fin
    -- jouee, #142) : seules les entrees conditionnelles de la transition ok comptent
    -- (comportement d'avant #76 pour une histoire cyclique : exhausted et non loop).
    local function stateful(tr, list, strict)
        for _, e in ipairs(list or {}) do
            if unseen(e) and (e.cond or (not strict and tr.indexItem ~= nil)) then return e end
        end
        return nil
    end
    -- Scenes visitees dont une transition ok ou home dependant de l'etat mene a une
    -- scene jamais atteinte : liste de { src = scene, dst = cible, home = bool }
    local function condEntries(strict)
        local out = {}
        for id in pairs(seen) do
            local st = N.stages and N.stages[id]
            local tr = st and st.ok
            local e = tr and stateful(tr, N.actions and N.actions[tr.action], strict)
            local viaHome = false
            if not e and not strict and st and st.home and F.hasHome(id, nil) then
                e = stateful(st.home, N.actions[st.home.action], false)
                viaHome = e ~= nil
            end
            if e then out[#out + 1] = { src = tostring(id), dst = tostring(e.stage), home = viaHome } end
        end
        return out
    end
    function F.condFrontier(strict) return #condEntries(strict) end
    -- Scenes bloquees (visitees, derriere une condition / un indexItem) et leur
    -- cible : "<scene>><cible>" ("@home" ajoute si c'est la transition home),
    -- tries ; au plus `max` (defaut 100)
    function F.condBlocked(max)
        local t = {}
        for _, e in ipairs(condEntries(false)) do
            t[#t + 1] = e.src .. ">" .. e.dst .. (e.home and "@home" or "")
        end
        table.sort(t)
        for i = (max or 100) + 1, #t do t[i] = nil end
        return t
    end
    -- Decision au menu Start apres une fin : reprendre (nil), `complete`, ou
    -- `conditions` (#76). Reprise tant qu'une option / transition ok sans condition
    -- reste a jouer. Si seules des transitions dependant de l'etat menent a du
    -- nouveau, l'histoire est redemarree (le choix prefere les options jamais
    -- essayees dans l'etat d'inventaire courant) : au plus COND_RESTARTS reprises
    -- sans aucune nouveaute (mark = scenes vues + options decouvertes), puis
    -- `conditions` : conditions jamais satisfaites par l'exploration.
    local COND_RESTARTS = 3
    local blocked, lastMark = 0, nil
    function F.restartVerdict(pending, mark)
        if pending > 0 or F.frontier() > 0 then blocked, lastMark = 0, nil; return nil end
        if F.condFrontier() == 0 then return "complete" end
        if lastMark == mark then blocked = blocked + 1 else blocked, lastMark = 1, mark end
        if blocked > COND_RESTARTS then return "conditions" end
        return nil
    end
    return F
end
'''


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

    %(frontier)s

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

        -- Boucles (#42) : noeuds (scenes, "a:<action>" pour un choix) entres depuis
        -- le dernier progres. Un noeud entre 3 fois sans rien de nouveau = cycle :
        -- le pilote tente alors la transition home de ce noeud, seule sortie des
        -- sous-menus et des histoires cycliques ; des 2 entrees pour un choix dont
        -- toutes les options sont essayees alors qu'il reste des options jamais
        -- essayees ailleurs (remonter vers elles). Au plus 3 sorties par noeud entre
        -- deux progres (escN), espacees (escAt). nLoop = noeuds distincts : une
        -- boucle fait bouger l'histoire (>= 2), une scene qui ne repond plus non
        -- (arret "unresponsive" au lieu de "loop").
        local loopSeen, nLoop, maxLoop, escN, escAt = {}, 0, 0, {}, {}
        local lastFresh = 0      -- dernier choix d'une option jamais essayee dans cet etat
        local function resetLoop() loopSeen, nLoop, maxLoop, escN, escAt = {}, 0, 0, {}, {} end
        local function progress() noProg = 0; resetLoop() end
        local function entered(id)
            local n = (loopSeen[id] or 0) + 1
            loopSeen[id] = n
            if n == 1 then nLoop = nLoop + 1 end
            if n > maxLoop then maxLoop = n end
        end
        -- Etat d'inventaire significatif : valeurs des objets lus par une condition
        -- ou un indexItem. Une option deja essayee peut mener ailleurs dans un autre
        -- etat (hubs a etat) : le choix prefere les options jamais essayees dans
        -- l'etat courant. Chaine vide si l'histoire n'a pas de condition.
        local sigItems = {}
        do
            local set = {}
            local function add(i)
                if type(i) == "number" and not set[i] then set[i] = true; sigItems[#sigItems + 1] = i end
            end
            for _, list in pairs(N.actions or {}) do
                for _, e in ipairs(list) do
                    for _, c in ipairs(e.cond or {}) do add(c.item or 0); add(c.itemB) end
                end
            end
            for _, st in pairs(N.stages or {}) do
                if st.ok then add(st.ok.indexItem) end
                if st.home then add(st.home.indexItem) end
            end
            table.sort(sigItems)
        end
        local function invSig()
            if #sigItems == 0 or not state.inv then return "" end
            local t = {}
            for i, it in ipairs(sigItems) do
                local s = state.inv[it + 1]
                t[i] = tostring(s and s.value or 0)
            end
            return table.concat(t, ",")
        end
        local stateCount = {}    -- essais par (option, etat d'inventaire)
        local escapes = 0
        local shown = {}     -- actions de choix deja affichees (cle = id d'action)
        -- Frontiere (transitions ok / home / conditionnelles vers du nouveau) :
        -- voir FRONTIER_LUA (testable sans player)
        local F = newFrontier(N, seen, shown)
        local hasHome, frontier, homeNew = F.hasHome, F.frontier, F.homeNew
        local condFrontier, homeFrontier = F.condFrontier, F.homeFrontier
        local nOptKeys = 0       -- options et (option, etat d'inventaire) distincts vus (nouveaute)
        local function finish(why)
            done = true
            emit("done reason=" .. why .. " steps=" .. steps .. " visited=" .. nSeen
                 .. " ends=" .. ends .. " pending=" .. pending .. " frontier=" .. frontier()
                 .. " cond=" .. condFrontier() .. " escapes=" .. escapes
                 .. " blocked=" .. table.concat(F.condBlocked(), ","))
        end
        -- Reste-t-il a reprendre ailleurs (option jamais essayee, transition ok
        -- jamais suivie) ? Alors un cycle se quitte par toute home (remonter).
        local function wantUp() return pending > 0 or frontier() > 0 end
        -- nav = true : touche de navigation (PREV/NEXT) dans un choix, non
        -- comptee dans noProg (seul le nombre de touches borne la navigation,
        -- cf. plan.nav) : un grand carrousel ne doit pas declencher "stuck".
        local function step(what, nav)
            steps = steps + 1
            if not nav then noProg = noProg + 1 end
            emit("step=" .. steps .. " " .. what)
        end
        local function visit(id)
            if not seen[id] then seen[id] = true; nSeen = nSeen + 1; progress() end
            entered(tostring(id))
            emit("scene=" .. tostring(id))
        end
        -- Option validee (ENTER envoye) : comptee (option essayee, scene de
        -- l'option visitee) seulement quand la transition a vraiment lieu, au
        -- noeud suivant (setProgression) ou a la fin d'histoire ; un ENTER
        -- avale ou un retour l'annule.
        local pendingOpt = nil
        local function confirmOpt()
            local o = pendingOpt
            if o == nil then return end
            pendingOpt = nil
            if optCount[o.key] == 0 then pending = pending - 1; progress() end
            optCount[o.key] = optCount[o.key] + 1
            if stateCount[o.skey] == nil then nOptKeys = nOptKeys + 1 end
            stateCount[o.skey] = (stateCount[o.skey] or 0) + 1
            emit("choice action=" .. tostring(o.aid) .. " option=" .. o.k .. "/" .. o.n
                 .. " key=" .. o.key)
            if o.stage then visit(o.stage) end
        end

        -- Scenes : toute entree dans un noeud passe par setProgression
        local _setProg = P.setProgression
        P.setProgression = function(d)
            confirmOpt()
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
            confirmOpt()
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
        -- Distance (transitions ok, graphe statique) d'une scene a du nouveau : une
        -- scene jamais atteinte ou une option affichee jamais essayee. nil si rien
        -- a moins de MAXD transitions. Guide le choix dans un hub deja essaye vers
        -- la branche qui mene a du nouveau, au lieu d'un tirage au sort (#42).
        local MAXD = 60
        local function newDist(sid)
            if sid == nil or not seen[sid] then return 0 end
            local dist, q, h = { [sid] = 0 }, { sid }, 1
            while h <= #q do
                local s = q[h]; h = h + 1
                local d = dist[s]
                if d >= MAXD then break end
                local tr = N.stages[s] and N.stages[s].ok
                local list = tr and N.actions and N.actions[tr.action]
                for _, e in ipairs(list or {}) do
                    local t = e.stage
                    if N.stages[t] then
                        if not seen[t] or (#list > 1 and optCount[tostring(tr.action)
                                .. "#s:" .. tostring(t)] == 0) then
                            return d + 1
                        end
                        if dist[t] == nil then dist[t] = d + 1; q[#q + 1] = t end
                    end
                end
            end
            return nil
        end
        -- Ordre des options : jamais essayee, puis la moins essayee dans l'etat
        -- d'inventaire courant, puis la plus proche de nouveau (newDist : departage
        -- les options d'un hub deja essayees sans figer le pilote sur une branche
        -- dont la nouveaute est gardee par une condition), puis la moins essayee
        -- tout court ; egalites tirees au sort (seed)
        local function choose(aid, n)
            shown[aid] = true
            local vis = visibleOf(aid)
            if #vis ~= n then vis = {} end   -- moteur different : cle par position
            local sig = invSig()
            local keys, skeys, best, cands = {}, {}, nil, {}
            for k = 1, n do
                local st = vis[k] and vis[k].stage
                keys[k] = tostring(aid) .. "#" .. (st and ("s:" .. tostring(st)) or tostring(k))
                skeys[k] = keys[k] .. "@" .. sig
                if optCount[keys[k]] == nil then optCount[keys[k]] = 0; pending = pending + 1; nOptKeys = nOptKeys + 1 end
                local c, sc = optCount[keys[k]], stateCount[skeys[k]] or 0
                local r = { c == 0 and 0 or 1, sc, (st and newDist(st)) or MAXD + 1, c }
                local cmp = 0
                if best then
                    for i = 1, 4 do
                        if r[i] ~= best[i] then cmp = r[i] < best[i] and -1 or 1; break end
                    end
                end
                if best == nil or cmp < 0 then best, cands = r, { k }
                elseif cmp == 0 then cands[#cands + 1] = k end
            end
            local k = cands[math.random(#cands)]
            -- option deja essayee mais jamais dans cet etat : nouveaute possible
            if best[2] == 0 and best[4] > 0 then lastFresh = steps end
            return { k = k, n = n, keys = keys, skeys = skeys, vis = vis, aid = aid, nav = 0 }
        end

        local cur, age, acted, plan = -1, 0, 0, nil
        local function key(m, code) lv.event.send(m.parentContainer, lv.EVENT_KEY, code) end
        -- deadEnd : sortie de secours (ecran sans action possible), tracee
        -- "deadend" avec le noeud courant pour reperer les impasses
        local function back(why, deadEnd)
            step("back " .. why)
            if deadEnd then
                emit("deadend at=" .. tostring(state.current_fun) .. " why=" .. why)
            end
            pendingOpt = nil
            inBack = true
            local ok, e = pcall(back_callback)
            inBack = false
            if not ok then error(e, 0) end
        end
        -- Sortie d'un cycle par la transition home du noeud courant (#42)
        -- gap : entrees du noeud requises depuis le dernier progres / la derniere sortie
        local function escape(id, gap)
            local n = loopSeen[id] or 0
            if n - (escAt[id] or 0) < gap or (escN[id] or 0) >= 3 then return false end
            escAt[id], escN[id] = n, (escN[id] or 0) + 1
            escapes = escapes + 1
            emit("escape at=" .. id)
            back("(home, boucle)")
            return true
        end

        local function onList(m)
            if acted > 0 and age %% 30 ~= 0 then return end
            local btn = m.answerContainer and lv.obj.get_child(m.answerContainer, 0)
            if btn == nil then return end
            local label = m.answers and m.answers[btn] and m.answers[btn].label or "?"
            if label == "Demarrer l'histoire" and ends > 0 then
                -- une decision par presentation du menu (pas a chaque relance du clic)
                local verdict = acted == 0 and F.restartVerdict(pending, nSeen + nOptKeys)
                if verdict then
                    finish(verdict)
                    goto_library()          -- retour bibliotheque (lua_close)
                    return
                end
                noProg = 0                  -- redemarrage voulu : prefixe deja connu
                resetLoop()
            end
            acted = acted + 1
            step("click " .. label)
            lv.event.send(btn, lv.EVENT_CLICKED)
        end

        local function onChoice(m)
            local n = m.answers and #m.answers or 0
            if n == 0 or m.parentContainer == nil then return end
            if plan == nil then
                local aid = tostring(state.current_fun)
                entered("a:" .. aid)
                -- cycle : sortie par la home de l'option focalisee, si elle en a une
                -- (rien a reprendre ailleurs : seulement vers une scene nouvelle)
                local vis = visibleOf(state.current_fun)
                local it = type(m.answerIterator) == "number" and m.answerIterator or 1
                local fs = #vis == n and vis[it] and vis[it].stage
                local up = fs and wantUp()
                local can = fs and (up and hasHome(fs, state.current_fun)
                                    or homeNew(fs, state.current_fun))
                local gap = 3
                if can and escape("a:" .. aid, gap) then
                    plan = { escaped = true }
                    return
                end
                plan = choose(state.current_fun, n)
                emit("choice? action=" .. tostring(plan.aid) .. " options=" .. n
                     .. " it=" .. tostring(m.answerIterator) .. " k=" .. plan.k)
            end
            if plan.escaped or age %% 2 ~= 0 then return end
            if plan.sent then
                if age %% 20 == 0 then step("key ENTER (relance)"); key(m, 10) end
                return
            end
            local it = m.answerIterator
            if type(it) == "number" and it ~= plan.k then
                if plan.nav < 2 * n + 4 then
                    plan.nav = plan.nav + 1
                    local fwd
                    if n >= 3 then
                        -- carrousel circulaire : sens le plus court
                        fwd = ((plan.k - it) %% n) <= n - ((plan.k - it) %% n)
                    else
                        -- 2 options : navigation bornee (pas de bouclage)
                        fwd = plan.k > it
                    end
                    if fwd then step("key NEXT", true); key(m, 19)
                    else step("key PREV", true); key(m, 20) end
                    return
                end
                plan.k = it                 -- navigation sans effet : option affichee
            end
            plan.sent = true
            local k = plan.k
            pendingOpt = { key = plan.keys[k], skey = plan.skeys[k], aid = plan.aid, k = k, n = plan.n,
                           stage = plan.vis[k] and plan.vis[k].stage }
            step("key ENTER (option " .. k .. "/" .. plan.n .. ")")
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
            if acted == 0 then
                local sid = state.current_fun
                local up = (loopSeen[tostring(sid)] or 0) >= 3 and wantUp()
                local can = up and hasHome(sid, nil) or homeNew(sid, nil)
                if can and escape(tostring(sid), 3) then acted = 1; return end
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
                back("(scene sans OK ni fin d'audio)", true)
            end
        end

        local function onTitle(m)
            if acted == 0 and age >= 5 and m.isAudioStarted and m.on_encoder_clicked then
                acted = 1
                step("saut du titre")
                m.on_encoder_clicked()
            elseif age >= 50 and age %% 50 == 0 then
                back("(titre)", true)
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
            -- cycle confirme, plus rien a essayer (options, transitions ok, homes vers
            -- du nouveau, entrees conditionnelles vers du nouveau, options jamais
            -- essayees dans l'etat d'inventaire) : epuise. Sans fin jouee (histoire
            -- cyclique), home conditionnelle et indexItem ne comptent pas (#142)
            if noProg > 10 and pending == 0 and maxLoop >= 3 and steps - lastFresh > 10
                    and frontier() == 0 and homeFrontier() == 0 and condFrontier(ends == 0) == 0 then
                finish("exhausted"); return
            end
            -- sans progres : boucle si l'histoire avance encore (>= 2 noeuds
            -- distincts depuis le dernier progres), sinon ecran qui ne repond plus
            if noProg > 40 + nSeen then
                finish(nLoop >= 2 and "loop" or "unresponsive"); return
            end
            if name:find("list-choice", 1, true) == 1 then onList(m)
            elseif name:find("carousel", 1, true) == 1 or name:find("image-choice", 1, true) == 1 then
                onChoice(m)
            elseif name:find("audio-player", 1, true) == 1 then onAudio(m)
            elseif name == "title-card" then onTitle(m)
            elseif age %% 50 == 0 then back("(ecran " .. name .. ")", true) end
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
        self.done_line = ""
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
                    self.done_line = line[len(TAG + "done reason="):]
                    self.done_reason = self.done_line.split()[0]
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
            f.write(DRIVER % {"frontier": FRONTIER_LUA, "seed": int(seed), "max_steps": int(max_steps),
                                 "back_every": max(0, int(back_every))})
        watch = _Watcher(work, stuck_s)
        args = (["--strict"] if strict else []) + [plain]
        code, out, err, dt = run_player(player_exe, args, work, timeout_s, until=watch)
        watch()   # dernieres lignes
        return _result(code, out, err, dt, total, watch, int(seed), work if keep else None)
    finally:
        if not keep:
            shutil.rmtree(work, ignore_errors=True)


def _result(code, out, err, dt, total, watch, seed, workdir):
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
    cond_n, blocked = None, []
    mc = re.search(r" cond=(\d+)", watch.done_line)
    if mc:
        cond_n = int(mc.group(1))
    mb = re.search(r" blocked=(.*)$", watch.done_line)
    if mb and mb.group(1):
        blocked = mb.group(1).split(",")
    started = any(t.startswith("start ") for t in tagged)
    dead = [t[len("deadend at="):] for t in tagged if t.startswith("deadend at=")]
    lua_seen = any(LUA_ERR_RE.search(e) for e in errors)

    # status + done_reason (pourquoi l'exploration s'est arretee), par priorite
    if code is not None:
        # sortie du player : erreur Lua vue avant => lua_error, sinon crash
        status = "lua_error" if lua_seen else "crash"
        done_reason = "exit"
        errors.append("flam-player arrete seul (code %s)" % code)
    elif errors:
        status, done_reason = "lua_error", "error"
    elif not started:
        status, done_reason = "stuck", "no_start"
        errors.append("pilote jamais demarre (setup() non atteint ?)")
    elif watch.done_reason:
        done_reason = watch.done_reason
        status = DONE_STATUS.get(done_reason, "ok")
    elif watch.library:
        status, done_reason = "ok", "library"
    elif watch.quiet:
        status, done_reason = "stuck", "no_trace"
    else:
        status, done_reason = "timeout", "timeout"
    res = {
        "status": status,
        "done_reason": done_reason,
        "errors": errors if len(errors) <= 200 else errors[:100] + errors[-100:],
        "scenes_visited": len(scenes),
        "scenes_total": total,
        "scenes": sorted(scenes),
        "coverage": round(len(scenes) / total, 3) if total else None,
        "end_reached": bool(ends),
        "duration_s": round(dt, 1),
        "seed": seed,
        "steps": len(steps),
        "ends": len(ends),
        "back_ends": tagged.count("end via=back"),
        "end_reasons": reasons,
        "choices_explored": len(choices),
        "back_tests": sum(1 for s in steps if " back (test)" in s),
        "menu_tests": sum(1 for s in steps if " menu M : " in s),
        "dead_end_backs": len(dead),
        "dead_end_scenes": sorted(set(dead)),
        "home_escapes": sum(1 for t in tagged if t.startswith("escape at=")),
        "cond_frontier": cond_n,
        "blocked": blocked,
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
        print("  arret : %s, couverture %s, seed %d" % (
            res["done_reason"],
            "?" if res["coverage"] is None else "%d %%" % round(100 * res["coverage"]),
            res["seed"]))
        if res["dead_end_backs"]:
            print("  impasses (retour de secours) : %d (%s)" % (
                res["dead_end_backs"], "; ".join(res["dead_end_scenes"])))
        for e in res["errors"]:
            print("  ! " + e)
        if res.get("workdir"):
            print("  dossier conserve : %s" % res["workdir"])
    return 0 if res["status"] == "ok" else 1


if __name__ == "__main__":
    sys.exit(main())
