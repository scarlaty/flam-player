-- ============================================================================
-- story.lua — BRANCH script (style Lunii Studio) pour histoires TELMI.
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
--     -> etat sauve = champs STANDARD (current_fun + currentBranchName), comme Cluedo.
-- ============================================================================

local N = require("nodes")
local story = {}

local BRANCH = "story"

-- forward declarations (les fonctions se referencent mutuellement)
local enterStage, showChoice, followTransition, enterAction, play, endStory

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
local function resetPass()
    passCount = 0
    passSeen = {}
end
local function passKey(id)
    local k = tostring(id)
    if state.inv then
        for i = 1, #state.inv do k = k .. "|" .. tostring(state.inv[i].value) end
    end
    return k
end

-- ---------------------------------------------------------------------------
-- Controles TELMI d'une scene (stages[id].ctrl = control.{ok,home,autoplay})
-- ---------------------------------------------------------------------------
-- nodes.lua genere avant l'emission de `ctrl` : comportement historique (avance
-- seule a la fin de l'audio, OK = skip, pas de home).
local LEGACY_CTRL = { ok = true, home = false, autoplay = true }
local function ctrlOf(st)
    return (st and st.ctrl) or LEGACY_CTRL
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
    Global.progression.resetProgress()   -- chaps = {}
    state.current_fun = nil
    state.currentBranchName = nil
    state.visited_funs = {}
    state.inv = nil
    setKind(nil, nil)
    progress = 0
    -- back_callback = requestAudioStop + cleanCurrentModule + Start
    -- (backBehavior = Start, pose par progression.create dans main.lua)
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
    -- Noeud de passage : pas d'audio ET (autoplay, ou rien a afficher). Spec
    -- TELMI : autoplay sans audio => ok execute immediatement. Une scene image
    -- sans audio NON autoplay est affichee et attend OK (plus bas).
    local passage = (not st.audio) and (ctrl.autoplay or not st.image)
    if st.audio then
        -- scene avec audio = un VRAI chapitre (contenu ecoute) => alimente .prog
        saveProgress(id, st.image, st.audio)
        -- Entree par le graphe (pas une reprise) : l'audio repart du debut. La
        -- position sauvee (seekposition, lue par audio-player au 1er 'play') ne
        -- sert qu'a la reprise ; une sortie par Home (transition home) la laisse
        -- en place, sinon une re-entree reprendrait l'audio en cours de route.
        if not isResume and state.visited_funs and state.visited_funs[id] then
            state.visited_funs[id].seekposition = nil
        end
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
    -- Home TELMI (X/Y) = bouton retour : transition home si ctrl.home, sinon
    -- menu Start (pose par setProgression ci-dessus). Reste DANS l'histoire :
    -- ce n'est pas la sortie goto_library du Bug C.
    local home = homeOf(st, id, nil)
    if home then
        Global.setBackBehavior(function()
            resetPass()
            -- scene quittee par Home : pas de reprise au milieu de cet audio
            -- (comme OK, processOk dans audio-player)
            if state.visited_funs and state.visited_funs[id] then
                state.visited_funs[id].seekposition = nil
            end
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
    if not list or #list == 0 then
        endStory("action inconnue ou vide " .. tostring(trans.action))
        return
    end

    -- indexItem : l'index du stage dans l'action est la valeur d'un item inventaire
    if trans.indexItem ~= nil then
        local slot = state.inv and state.inv[trans.indexItem + 1]
        local idx = (slot and slot.value or 0) + 1  -- TELMI 0-based -> Lua 1-based
        local e = list[idx] or list[#list]           -- clamp sur le dernier si hors borne
        enterStage(e.stage)
        return
    end

    enterAction(trans.action, trans.index)
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
        initInventory()           -- nouveau depart : inventaire neuf
        followTransition(N.start)
    elseif N.stages[name] then
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
