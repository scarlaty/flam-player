-- ============================================================================
-- story.lua — BRANCH script (graphe de scenes) pour histoires TELMI.
--
-- Charge par Global.loadBranch("story") -> require("story"). Doit :
--   - retourner une table `story`
--   - exposer story.clear()
--   - exposer story[<nom>]() pour chaque noeud (appele par current_branch[fn]())
--
-- On rejoue le graphe TELMI (script/nodes.lua) :
--   - currentFunction sauve = id de stage (sN) OU id d'action (aN, pour un choix)
--     + state.current_kind ("s" scene / "a" action) : les ids TELMI sont du texte
--     libre, une scene et une action peuvent partager un id
--   - resume : story.resume() (main.lua LoadCurrentFunction) -> noeud type
--   - chaque noeud appelle Global.progression.setProgression{currentFunction, branch}
--     -> etat sauve = champs STANDARD (current_fun + currentBranchName).
-- ============================================================================

local N = require("nodes")
local story = {}

local BRANCH = "story"

-- forward declarations (les fonctions se referencent mutuellement)
local enterStage, showChoice, followTransition, enterAction, play, endStory, leaveStory

-- Compteur anti-boucle des noeuds sans audio. LOCAL au module (et non un champ
-- story._pass) : un champ absent de `story` serait resolu par __index, qui
-- renvoie une closure => "arithmetic on a function value".
-- Une boucle n'est declaree que si l'etat se repete : cle = stage + valeurs de
-- l'inventaire. Une chaine de logique bornee (compteur d'inventaire decremente
-- a chaque passage) n'est donc pas interrompue. MAX_REPEAT tolere quelques
-- repetitions (branche aleatoire index -1 qui finit par sortir) ; MAX_PASS
-- borne une chaine dont l'etat ne se repete jamais (compteur sans max) et la
-- profondeur de la recursion enterStage/followTransition/enterAction (Lua pur,
-- sans frontiere C, mais chaque niveau coute de la memoire sur le device).
local passCount = 0
local passSeen = {}
local MAX_PASS = 1000
local MAX_REPEAT = 30
-- ---------------------------------------------------------------------------
-- Controles TELMI d'une scene (stages[id].ctrl = control.{ok,home,autoplay})
-- ---------------------------------------------------------------------------
-- nodes.lua genere avant l'emission de `ctrl` : comportement historique (avance
-- seule a la fin de l'audio, OK = skip, pas de home).
local LEGACY_CTRL = { ok = true, home = false, autoplay = true }
local function ctrlOf(st)
    return (st and st.ctrl) or LEGACY_CTRL
end
-- Noeud de passage : pas d'audio ET (autoplay, ou rien a afficher). Spec
-- TELMI : autoplay sans audio => ok execute immediatement. Une scene image
-- sans audio NON autoplay est affichee et attend OK.
local function isPassage(st)
    return (not st.audio) and (ctrlOf(st).autoplay or not st.image) and true or false
end


-- Noeud de JEU quitte par le bouton retour (transition home) : { id, kind, index }.
-- Pose au 1er appui sur retour depuis un noeud de jeu ; garde tant que le
-- joueur reste dans le "menu retour" de l'histoire (convention TELMI
-- backAction -> backStage -> backChildAction, avec ses options "Reprendre le
-- jeu ?", "Quitter le jeu ?", indices...). Effacee des que le joueur s'arrete
-- de nouveau sur un noeud de jeu (gameNodes).
-- Tant qu'elle est posee, une action vide ou une OPTION de choix sans suite
-- (ok=nil, "Quitter le jeu ?") n'est PAS une fin d'histoire mais "Quitter" :
-- retour au menu Start SANS effacer la progression ni l'inventaire
-- (leaveStory), la reprise ramene sur ce noeud.
-- Persistee dans `state` (state.home_origin) : une reprise ou une relance
-- pendant que le menu retour est affiche (veille, menu M) la retrouve.
local function resetPass()
    passCount = 0
    passSeen = {}
end

-- Livres a chapitres (opt-in : champ `role` d'une scene, pose par le launcher ;
-- une histoire sans `role` garde EXACTEMENT le comportement historique) :
--   role = "chapter" : chapitre. Quitte par Home, sa position audio (seekposition,
--     ecrite chaque seconde par audio-player) est GARDEE et il devient le
--     "chapitre quitte" (state.chapter_left, persiste comme home_origin) :
--     "Reprendre l'histoire" (menu Start, apres sortie ou veille dans le menu des
--     chapitres) le reprend a cette position, et y revenir par le menu des
--     chapitres aussi. Tout autre chapitre repart du debut.
--   role = "title"   : titre lu (option affichee seule, annonce d'un chapitre) :
--     position seule, pas un chapitre de la jauge (totalChapters l'exclut aussi).
local function roleOf(st)
    return st and st.role
end
local function getHomeOrigin() return state.home_origin end
local function setHomeOrigin(o) state.home_origin = o end

-- Noeuds de jeu : atteignables depuis l'action de depart en suivant les
-- transitions ok et les entrees d'action (conditions ignorees), JAMAIS les
-- transitions home. Le reste (menu retour, indices) n'est atteint que par le
-- bouton retour. Les scenes de PASSAGE contenues dans une action cible d'une
-- transition home (backStage de backAction) sont l'entree du menu retour :
-- elles ne sont pas parcourues, meme quand une scene de jeu y mene par ok
-- (scene "info" qui renvoie au menu, backStage repris dans une autre action).
-- Les scenes de l'action de depart ne sont jamais bloquees. Une cible home
-- qui joue une scene (hub) reste un noeud de jeu. Calcule une fois, O(noeuds).
local gameNodes = nil
local function computeGameNodes()
    local startAction = N.start and N.start.action
    local startStages = {}
    for _, e in ipairs(N.actions[startAction] or {}) do
        if e.stage ~= nil then startStages[e.stage] = true end
    end
    local blocked, homeTargets = {}, {}
    for _, st in pairs(N.stages) do
        local h = st.home and st.home.action
        if h ~= nil then homeTargets[h] = true end
    end
    for h in pairs(homeTargets) do
        for _, e in ipairs(N.actions[h] or {}) do
            local sid = e.stage
            local st = sid ~= nil and N.stages[sid]
            if st and not startStages[sid] and isPassage(st) then blocked[sid] = true end
        end
    end
    local seen = { s = {}, a = {} }
    local todo = {}
    if startAction ~= nil then todo[1] = startAction end
    while #todo > 0 do
        local aid = table.remove(todo)
        if aid ~= nil and not seen.a[aid] then
            seen.a[aid] = true
            for _, e in ipairs(N.actions[aid] or {}) do
                local sid = e.stage
                if sid ~= nil and not seen.s[sid] and not blocked[sid] then
                    seen.s[sid] = true
                    local st = N.stages[sid]
                    if st and st.ok and st.ok.action ~= nil then todo[#todo + 1] = st.ok.action end
                end
            end
        end
    end
    return seen
end
local function isGameNode(kind, id)
    if not gameNodes then gameNodes = computeGameNodes() end
    return gameNodes[kind][id] == true
end
-- Le joueur s'arrete sur un noeud (scene affichee ou choix) : s'il s'agit d'un
-- noeud de jeu, le contexte "menu retour" est termine. Jamais appele pour un
-- noeud de passage (backStage peut etre aussi atteint par ok depuis le jeu).
local function enterNode(kind, id)
    if getHomeOrigin() and isGameNode(kind, id) then setHomeOrigin(nil) end
end
-- Appui sur retour depuis le noeud { id, kind, index } : seul un appui depuis
-- un noeud de jeu fixe l'origine ; un appui dans le menu retour la garde.
local function markHome(origin)
    if getHomeOrigin() == nil and isGameNode(origin.kind, origin.id) then
        setHomeOrigin(origin)
    end
end
-- Reprise : origine sauvee gardee seulement si elle designe un noeud de jeu
-- existant ET que le noeud repris n'est pas un noeud de jeu (menu retour).
local function validateHomeOrigin(kind, id)
    local o = getHomeOrigin()
    if type(o) ~= "table" or (kind and isGameNode(kind, id))
        or not ((o.kind == "s" and N.stages[o.id]) or (o.kind == "a" and N.actions[o.id]))
        or not isGameNode(o.kind, o.id) then
        setHomeOrigin(nil)
    end
end

local function passKey(id)
    local k = tostring(id)
    if state.inv then
        for i = 1, #state.inv do k = k .. "|" .. tostring(state.inv[i].value) end
    end
    return k
end

-- Transition home d'une scene (ctrl.home ET home defini), sinon nil : le retour
-- garde alors le comportement pose par setProgression (menu Start). Une home qui
-- ramene sur le noeud courant (meme action, ou action contenant cette scene) est
-- ignoree : sinon le bouton retour ne permettrait plus de sortir de l'histoire.
local function homeOf(st, selfStage, selfAction)
    if not (st and st.home and ctrlOf(st).home) then return nil end
    local a = st.home.action
    if a == selfAction then return nil end
    local list = N.actions[a]
    if not list then return nil end
    for _, e in ipairs(list) do
        if e.stage == selfStage then return nil end
    end
    return st.home
end

-- ---------------------------------------------------------------------------
-- Sauvegarde de progression
-- ---------------------------------------------------------------------------
-- Enregistre la position courante ET un "chapitre" (ischapter=true), comme les
-- histoires officielles : alimente la jauge du menu Reprendre
-- (`getProgressionValue() = #chapitres/totalChapters`). NB : un `.prog` a 0 n'est
-- PAS la cause de l'ecran noir a la relance (piste ecartee ; la cause etait la
-- sortie brutale goto_library depuis une scene : Bug C, DEVICE_VS_SIM.md §10).
-- Label unique par scene => un chapitre par scene visitee (dedup par label).
-- N'est appele QUE pour les scenes AVEC audio (= vrais chapitres / contenu
-- ecoute). Les choix et les noeuds de passage n'appellent pas saveProgress :
-- ainsi #chapitres <= nb de scenes-audio = totalChapters, donc .prog in [0,100].
-- Type du noeud courant, pose APRES setProgression (qui ecrit current_fun).
-- choiceIndex : index TELMI (0-based, liste NON filtree) de l'option
-- preselectionnee d'un choix, restaure a la reprise.
local function setKind(kind, choiceIndex)
    state.current_kind = kind
    state.choice_index = choiceIndex
end

local function saveProgress(fn, img, audio)
    Global.progression.setProgression({
        currentFunction = fn,
        branch = BRANCH,
        ischapter = true,
        chapterData = {
            label = fn,
            cb = fn,
            branch = BRANCH,
            img = img or "empty.lif",
            audio = audio or "silent.mp3",
        },
    })
    setKind("s", nil)
end

-- ---------------------------------------------------------------------------
-- Inventaire (persiste dans `state`)
-- ---------------------------------------------------------------------------
local function initInventory()
    -- Pas de table vide `inv = {}` persistee : precaution heritee du debug de
    -- l'ecran noir a la relance (hypothese NON confirmee, la cause etait Bug C,
    -- DEVICE_VS_SIM.md §10). Sans cout : si l'histoire n'a pas d'inventaire,
    -- `state.inv` reste nil et tous les acces sont gardes par `state.inv and ...`.
    if N.inventory and #N.inventory > 0 then
        state.inv = {}
        for i, item in ipairs(N.inventory) do
            state.inv[i] = { value = item.init or 0, max = item.max or 0 }
        end
    else
        state.inv = nil
    end
end

-- Reprise : inventaire relu de la sauvegarde. Garde-fou si la sauvegarde ne
-- l'a pas rendu tel quel (absent, cles devenues texte, taille differente
-- apres une mise a jour du pack) : sans lui, applyStageItems ignorerait en
-- silence tout objet ramasse ensuite (slot nil).
local function ensureInventory()
    if not (N.inventory and #N.inventory > 0) then return end
    local inv = state.inv
    if type(inv) ~= "table" then
        print("story.lua: inventaire sauve absent, reinitialise")
        initInventory()
        return
    end
    -- Fusion slot par slot : un slot illisible ou ajoute par une mise a jour du
    -- pack prend sa valeur initiale, les objets deja ramasses sont gardes.
    local fixed = {}
    for i, def in ipairs(N.inventory) do
        local slot = inv[i]
        if slot == nil then slot = inv[tostring(i)] end
        local v = type(slot) == "table" and tonumber(slot.value) or nil
        if v == nil then
            print("story.lua: objet " .. i .. " absent de la sauvegarde, valeur initiale")
            fixed[i] = { value = def.init or 0, max = def.max or 0 }
        else
            fixed[i] = { value = v, max = def.max or tonumber(slot.max) or 0 }
        end
    end
    state.inv = fixed
end

local function applyStageItems(stage)
    if not stage then return end
    if stage.reset then initInventory() end
    if stage.items then
        for _, op in ipairs(stage.items) do
            local slot = state.inv and state.inv[(op.item or 0) + 1]
            if slot then
                local n
                if op.playingTime then
                    -- Temps audio ecoule : Global.audioDuration tenu a jour par
                    -- les callbacks du module audio-player via global.audioFeedback.
                    n = math.floor(Global.audioDuration or 0)
                elseif op.assignItem ~= nil then
                    local src = state.inv and state.inv[op.assignItem + 1]
                    n = src and src.value or 0
                else
                    n = op.number or 0
                end
                local v, t = slot.value, op.type or 0
                if     t == 0 then v = v + n
                elseif t == 1 then v = v - n
                elseif t == 2 then v = n
                elseif t == 3 then v = v * n
                elseif t == 4 then if n ~= 0 then v = math.floor(v / n) end
                elseif t == 5 then if n ~= 0 then v = v % n end end
                if slot.max and slot.max > 0 and v > slot.max then v = slot.max end
                if v < 0 then v = 0 end
                slot.value = v
            end
        end
    end
end

local function evalCond(c)
    local slot = state.inv and state.inv[(c.item or 0) + 1]
    local v = slot and slot.value or 0
    local n
    if c.itemB ~= nil then
        local slotB = state.inv and state.inv[c.itemB + 1]
        n = slotB and slotB.value or 0
    else
        n = c.num or 0
    end
    local cmp = c.cmp or 2
    if     cmp == 0 then return v <  n
    elseif cmp == 1 then return v <= n
    elseif cmp == 2 then return v == n
    elseif cmp == 3 then return v >  n
    elseif cmp == 4 then return v >= n
    else                return v ~= n end
end

-- Une entree d'action est accessible si toutes ses conditions passent.
local function isVisible(e)
    if not e.cond then return true end
    for _, c in ipairs(e.cond) do
        if not evalCond(c) then return false end
    end
    return true
end

-- ---------------------------------------------------------------------------
-- Fin d'histoire / garde-fous
-- ---------------------------------------------------------------------------
-- Sortie PROPRE vers le menu Start, jamais goto_library() depuis une scene
-- active (Bug C, DEVICE_VS_SIM.md §10). La progression est remise a zero pour
-- que le menu propose de nouveau "Demarrer l'histoire".
function endStory(reason)
    print("story.lua: fin d'histoire : " .. (reason or "fin normale"))
    resetPass()
    setHomeOrigin(nil)
    Global.progression.resetProgress()   -- chaps = {}
    state.current_fun = nil
    state.currentBranchName = nil
    state.visited_funs = {}
    state.inv = nil
    state.chapter_left = nil
    setKind(nil, nil)
    progress = 0
    -- back_callback = requestAudioStop + cleanCurrentModule + Start
    -- (backBehavior = Start, pose par progression.create dans main.lua)
    Global.progression.restoreBackBehavior()
    back_callback()
end

-- Sortie depuis le menu retour (action vide, ou option sans suite ok=nil type
-- "Quitter le jeu ?") : retour au menu Start avec la progression du noeud de
-- jeu quitte (menu "Reprendre"). Le menu retour a deplace la position sauvee :
-- on la restaure sur ce noeud. Contrairement a endStory, rien n'est remis a
-- zero (inventaire compris).
function leaveStory(origin)
    print("story.lua: sortie par le menu retour : retour au menu Start")
    resetPass()
    setHomeOrigin(nil)
    Global.progression.setProgression({ currentFunction = origin.id, branch = BRANCH })
    setKind(origin.kind, origin.index)
    Global.progression.restoreBackBehavior()
    back_callback()
end

-- ---------------------------------------------------------------------------
-- Rendu d'un stage (scene image + audio)
-- ---------------------------------------------------------------------------
-- isResume : reprise (LoadCurrentFunction). Les ops inventaire / reset de la
-- scene ont deja ete appliquees (et sauvees) au 1er passage : ne pas les rejouer.
function enterStage(id, isResume)
    local st = N.stages[id]
    if not st then endStory("stage inconnu " .. tostring(id)); return end
    if not isResume then applyStageItems(st) end
    local ctrl = ctrlOf(st)
    -- Noeud de passage (isPassage) : enchaine sans lecteur audio (plus bas).
    local passage = isPassage(st)
    local role = roleOf(st)
    if st.audio then
        if role == "title" then
            -- titre lu (role "title") : position seule, pas un chapitre (jauge)
            Global.progression.setProgression({ currentFunction = id, branch = BRANCH })
            setKind("s", nil)
        else
            -- scene avec audio = un VRAI chapitre (contenu ecoute) => alimente .prog
            saveProgress(id, st.image, st.audio)
        end
        -- Entree par le graphe (pas une reprise) : l'audio repart du debut. La
        -- position sauvee (seekposition, lue par audio-player au 1er 'play') ne
        -- sert qu'a la reprise ; une sortie par Home (transition home) la laisse
        -- en place, sinon une re-entree reprendrait l'audio en cours de route.
        -- Exception (role "chapter") : le chapitre quitte par Home reprend a sa
        -- position quand on y revient.
        local keep = role == "chapter" and state.chapter_left == id
        if not isResume and not keep and state.visited_funs and state.visited_funs[id] then
            state.visited_funs[id].seekposition = nil
        end
        if role == "chapter" then state.chapter_left = nil end
    else
        -- noeud de passage (ex. backStage) : pur routage, PAS un chapitre.
        -- Position seule (isStoryStarted reste vrai => plancher .prog>=1 assure
        -- par le wrapper getProgressionValue dans main.lua).
        Global.progression.setProgression({ currentFunction = id, branch = BRANCH })
        setKind("s", nil)
    end
    -- NE PAS forcer setBackBehavior(goto_library) : on laisse le comportement
    -- pose par setProgression (retour -> menu Start), exactement comme les
    -- histoires officielles. Sortir BRUTALEMENT d'une scene active vers la
    -- bibliotheque (goto_library) est le chemin qui, sur device, corrompt l'etat
    -- et fige le lancement suivant en ecran noir (Bug C, DEVICE_VS_SIM.md §10).
    -- L'officiel fait : scene -> (retour) menu Start -> (retour) bibliotheque.
    if passage then
        -- noeud de passage : enchaine sans lecteur audio
        passCount = passCount + 1
        local key = passKey(id)
        passSeen[key] = (passSeen[key] or 0) + 1
        if passCount > MAX_PASS or passSeen[key] > MAX_REPEAT then
            endStory("boucle de noeuds sans audio (stage " .. tostring(id) .. ")")
            return
        end
        followTransition(st.ok)
        return
    end
    resetPass()
    enterNode("s", id)
    -- Home TELMI (X/Y) = bouton retour : transition home si ctrl.home, sinon
    -- menu Start (pose par setProgression ci-dessus). Reste DANS l'histoire :
    -- ce n'est pas la sortie goto_library du Bug C.
    local home = homeOf(st, id, nil)
    if home then
        Global.setBackBehavior(function()
            resetPass()
            if role == "chapter" then
                -- chapitre quitte par Home : position gardee, reprise possible
                state.chapter_left = id
            elseif state.visited_funs and state.visited_funs[id] then
                -- scene quittee par Home : pas de reprise au milieu de cet audio
                -- (comme OK, processOk dans audio-player)
                state.visited_funs[id].seekposition = nil
            end
            markHome({ id = id, kind = "s" })
            followTransition(home)
        end)
    end
    local function goOk()
        resetPass()
        followTransition(st.ok)
    end
    Global.load_module("audio-player", "1_0_0").create({
        audio_path = st.audio,   -- nil : scene image seule (attend OK)
        image_background_path = st.image or "empty.lif",
        image_path = "empty.lif",
        image_foreground_path = "empty.lif",
        song_name = "",
        -- autoplay : ok a la fin de l'audio ; sinon on reste sur l'image
        callback = ctrl.autoplay and goOk or nil,
        -- ctrl.ok : OK (ENTER) pendant ou apres l'audio => ok (skip)
        okCallback = ctrl.ok and goOk or nil,
    })
end

-- ---------------------------------------------------------------------------
-- Choix multi-options
-- ---------------------------------------------------------------------------
-- list : entrees DEJA filtrees par conditions (enterAction) ; sel : option
-- preselectionnee (1-based dans list) ; selIndex : index TELMI de cette option
-- (0-based, liste NON filtree), sauve pour la reprise.
function showChoice(list, actionId, sel, selIndex)
    -- Position SEULE, PAS un chapitre : seules les scenes comptent dans la jauge.
    -- Sinon #chapitres (scenes + choix) depasserait totalChapters (= nb de scenes)
    -- et .prog grimperait > 100 (hors plage observee sur device : 0..100).
    -- setProgression passe quand meme isStoryStarted a true => le plancher du
    -- wrapper getProgressionValue garantit .prog >= 1 si on quitte sur un choix.
    Global.progression.setProgression({ currentFunction = actionId, branch = BRANCH })
    setKind("a", selIndex)
    resetPass()   -- un choix attend l'utilisateur : fin de la chaine de passage
    enterNode("a", actionId)
    local choices = {}
    for i, e in ipairs(list) do
        local opt = N.stages[e.stage] or {}
        local chosen = false
        choices[#choices + 1] = {
            img = opt.image or "empty.lif",
            -- Option sans audio : silence, sinon l'audio de l'option precedente
            -- continue de jouer.
            audio = opt.audio or "silent.mp3",
            -- Label : texte notes.json (resolu au build dans stages[id].text)
            -- sinon "Choix N" (N = position dans CE choix ; non resoluble au
            -- build car une scene peut etre cible de plusieurs actions).
            label = opt.text or ("Choix " .. i),
            cb = function()
                -- garde : un 2e appel (ENTER maintenu) re-appliquerait les items
                if chosen then return end
                chosen = true
                resetPass()
                applyStageItems(opt)
                -- Option sans suite (ok=null) choisie dans le menu retour
                -- ("Quitter le jeu ?") : retour au menu Start en gardant
                -- progression et inventaire. Une SCENE sans suite reste une
                -- vraie fin (endStory), meme atteinte par une transition home.
                if opt.ok == nil and getHomeOrigin() then
                    leaveStory(getHomeOrigin())
                    return
                end
                followTransition(opt.ok)
            end,
        }
    end
    -- Selecteur choisi a la conversion (--selector). Defaut : carrousel.
    -- Idem : pas de setBackBehavior(goto_library). Retour -> menu Start (officiel).
    local module = (N.selector == "image") and "image-choice" or "carousel"
    if type(sel) ~= "number" or sel < 1 or sel > #choices then sel = 1 end
    local m = Global.load_module(module, "1_0_0")
    -- Preselection : pas d'argument dedie dans les modules, create() part de
    -- answerIterator (1 par defaut). `index` est passe en plus pour un module
    -- qui le lirait.
    if type(m.answerIterator) == "number" then m.answerIterator = sel end
    -- Home TELMI de l'option focalisee (retour). Le focus est lu AVANT le
    -- cleanCurrentModule de back_callback (clean remet answerIterator a 1).
    local homes, anyHome = {}, false
    for i, e in ipairs(list) do
        homes[i] = homeOf(N.stages[e.stage], e.stage, actionId)
        if homes[i] then anyHome = true end
    end
    if anyHome then
        local focused = sel
        Global.setBackBehavior(function()
            local h = homes[focused]
            if h then
                resetPass()
                markHome({ id = actionId, kind = "a", index = selIndex })
                followTransition(h)
            elseif Global.progression.backButtonBehavior then
                Global.progression.backButtonBehavior()   -- menu Start
            end
        end)
        local backCb = back_callback
        back_callback = function()
            if type(m.answerIterator) == "number" then focused = m.answerIterator end
            backCb()
        end
    end
    m.create({
        choices = choices,
        index = sel,
        exitCb = function() back_callback() end,
        skipIfLastChoice = false,
        -- title_audio : aucune source audio dediee au prompt dans les donnees
        -- TELMI (la scene source a deja joue son audio, pas de TTS pour le
        -- texte notes.json) => nil. Le module degrade sur l'audio du focus.
    })
end

-- ---------------------------------------------------------------------------
-- Resolution d'une transition {action, index|indexItem}
-- ---------------------------------------------------------------------------
function followTransition(trans)
    if not trans then endStory(); return end   -- ok=null : fin d'histoire
    local list = N.actions[trans.action]
    if list and #list == 0 and getHomeOrigin() then
        leaveStory(getHomeOrigin())   -- backChildAction vide atteinte par Home
        return
    end
    if not list or #list == 0 then
        endStory("action inconnue ou vide " .. tostring(trans.action))
        return
    end

    -- indexItem : spec TELMI, equivalent a `index` dont la valeur est celle
    -- d'un item d'inventaire (0-based, bornee a la liste ; jamais -1 = tirage).
    -- Memes regles que index (enterAction) : filtre des conditions, cible
    -- autoplay jouee directement, sinon choix preselectionne sur cette option.
    local index = trans.index
    if trans.indexItem ~= nil then
        local slot = state.inv and state.inv[trans.indexItem + 1]
        local v = math.floor(tonumber(slot and slot.value) or 0)
        index = math.max(0, math.min(v, #list - 1))
    end

    enterAction(trans.action, index)
end

-- Entree dans une action : seules les entrees dont les conditions passent sont
-- proposees. 1 -> scene directe, plusieurs -> choix, aucune -> sortie propre.
-- index : option preselectionnee (TELMI 0-based, dans la liste NON filtree) ;
-- -1 = tirage aleatoire parmi les entrees visibles.
-- isResume : reprise sur un choix sauve => toujours le choix (pas de raccourci
-- autoplay, pas de tirage ; index = option preselectionnee sauvee).
function enterAction(actionId, index, isResume)
    local list = N.actions[actionId]
    if not list or #list == 0 then
        endStory("action inconnue ou vide " .. tostring(actionId))
        return
    end
    local visible, visIdx, sel = {}, {}, 1
    for i, e in ipairs(list) do
        if isVisible(e) then
            visible[#visible + 1] = e
            visIdx[#visible] = i - 1
            if type(index) == "number" and i == index + 1 then sel = #visible end
        end
    end
    if #visible == 0 then
        endStory("aucune option accessible (action " .. tostring(actionId) .. ")")
        return
    end
    if index == -1 and not isResume then sel = math.random(#visible) end
    if #visible == 1 then
        enterStage(visible[1].stage)
        return
    end
    -- Scene cible autoplay : spec TELMI, gauche/droite desactives => pas de
    -- molette, la scene est jouee directement (branche aleatoire si index -1).
    local target = N.stages[visible[sel].stage]
    if not isResume and target and target.ctrl and target.ctrl.autoplay then
        enterStage(visible[sel].stage)
        return
    end
    showChoice(visible, actionId, sel, visIdx[sel])
end

-- ---------------------------------------------------------------------------
-- Point d'entree par nom de noeud (appele par Global.current_branch[name]())
-- ---------------------------------------------------------------------------
function play(name)
    resetPass()
    if name == "__start" then
        setHomeOrigin(nil)
        state.chapter_left = nil
        initInventory()           -- nouveau depart : inventaire neuf
        followTransition(N.start)
        return
    end
    ensureInventory()
    validateHomeOrigin(N.stages[name] and "s" or "a", name)
    if N.stages[name] then
        enterStage(name, true)    -- reprise sur une scene (items deja appliques)
    elseif N.actions[name] then
        enterAction(name, nil, true)   -- reprise sur un choix (memes filtres)
    else
        endStory("noeud inconnu " .. tostring(name))
    end
end

function story.clear()
    resetPass()
end

-- Reprise (main.lua LoadCurrentFunction) : rejoue le noeud sauve SELON SON TYPE.
-- Ne passe pas par story[<id>] : un id TELMI egal a un champ du module ("clear",
-- "resume") ou a "__start" serait mal resolu, et une scene et une action de
-- meme id seraient confondues. Type absent (save d'une ancienne version du
-- pack) ou incoherent : scene puis action, comme avant. Noeud absent du
-- graphe : redemarrage.
function story.resume()
    local id, kind = state.current_fun, state.current_kind
    resetPass()
    -- Livre (role "chapter") : chapitre quitte par Home puis sortie ou veille dans
    -- le menu des chapitres : on le reprend, a sa position, au lieu du menu.
    local left = state.chapter_left
    if left ~= nil then
        if roleOf(N.stages[left]) == "chapter" then
            ensureInventory()
            validateHomeOrigin("s", left)
            enterStage(left, true)
            return
        end
        state.chapter_left = nil   -- sauvegarde d'une autre version du pack
    end
    if id ~= nil then ensureInventory() end
    -- Reprise sur le menu retour (veille, menu M) : origine sauvee gardee.
    validateHomeOrigin(id ~= nil and (kind or (N.stages[id] and "s" or "a")) or nil, id)
    if id == nil then
        -- redemarrage (plus bas)
    elseif kind == "s" and N.stages[id] then
        enterStage(id, true)
        return
    elseif kind == "a" and N.actions[id] then
        enterAction(id, state.choice_index, true)
        return
    end
    if id ~= nil and N.stages[id] then
        enterStage(id, true)
    elseif id ~= nil and N.actions[id] then
        enterAction(id, nil, true)
    else
        print("story.lua: noeud de reprise inconnu " .. tostring(id) .. ", redemarrage")
        play("__start")
    end
end

-- current_branch[<name>]() -> play(name), UNIQUEMENT pour les noeuds du graphe
-- (sN, aN, __start) : toute autre cle reste nil.
setmetatable(story, {
    __index = function(_, name)
        if name == "__start" or N.stages[name] or N.actions[name] then
            return function() play(name) end
        end
    end,
})

return story
