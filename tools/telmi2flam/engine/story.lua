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
--   - resume : Global.current_branch[state.current_fun]() -> play(current_fun)
--   - chaque noeud appelle Global.progression.setProgression{currentFunction, branch}
--     -> etat sauve = champs STANDARD (current_fun + currentBranchName), comme Cluedo.
-- ============================================================================

local N = require("nodes")
local story = {}

local BRANCH = "story"

-- forward declarations (les fonctions se referencent mutuellement)
local enterStage, showChoice, followTransition, play

-- ---------------------------------------------------------------------------
-- Sauvegarde de progression
-- ---------------------------------------------------------------------------
-- Enregistre la position courante ET un "chapitre" (ischapter=true). C'est
-- INDISPENSABLE sur device : `getProgressionValue() = #chapitres/totalChapters`,
-- et le firmware Lunii FIGE l'ecran (noir, sans input) au 2e lancement d'une
-- histoire commencee dont le `.prog` vaut 0 (issue #1). Les histoires officielles
-- enregistrent toutes des chapitres -> .prog > 0 ; on fait pareil ici.
-- Label unique par scene => un chapitre par scene visitee (dedup par label).
-- N'est appele QUE pour les scenes AVEC audio (= vrais chapitres / contenu
-- ecoute). Les choix et les noeuds de passage n'appellent pas saveProgress :
-- ainsi #chapitres <= nb de scenes-audio = totalChapters, donc .prog in [0,100].
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
end

-- ---------------------------------------------------------------------------
-- Inventaire (persiste dans `state`)
-- ---------------------------------------------------------------------------
local function initInventory()
    -- Ne JAMAIS persister une table Lua vide `inv = {}`. Sur device, la
    -- deserialisation d'une table vide dans `{uuid}.save` est suspectee de
    -- corrompre l'etat runtime au 2e lancement (ecran noir fige, issue #1).
    -- Si l'histoire n'a pas d'inventaire, on laisse `state.inv` a nil ; tous
    -- les acces sont deja gardes par `state.inv and ...`.
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

local function pickByConditions(list)
    for _, e in ipairs(list) do
        if not e.cond then return e end
        local all = true
        for _, c in ipairs(e.cond) do
            if not evalCond(c) then all = false; break end
        end
        if all then return e end
    end
    return list[#list]
end

-- ---------------------------------------------------------------------------
-- Rendu d'un stage (scene image + audio)
-- ---------------------------------------------------------------------------
function enterStage(id)
    local st = N.stages[id]
    if not st then goto_library(); return end
    applyStageItems(st)
    if st.audio then
        -- scene avec audio = un VRAI chapitre (contenu ecoute) => alimente .prog
        saveProgress(id, st.image, st.audio)
    else
        -- noeud de passage (ex. backStage) : pur routage, PAS un chapitre.
        -- Position seule (isStoryStarted reste vrai => plancher .prog>=1 assure
        -- par le wrapper getProgressionValue dans main.lua).
        Global.progression.setProgression({ currentFunction = id, branch = BRANCH })
    end
    -- NE PAS forcer setBackBehavior(goto_library) : on laisse le comportement
    -- pose par setProgression (retour -> menu Start), exactement comme les
    -- histoires officielles. Sortir BRUTALEMENT d'une scene active vers la
    -- bibliotheque (goto_library) est le chemin qui, sur device, corrompt l'etat
    -- et fige le lancement suivant en ecran noir (issue #1). L'officiel fait :
    -- scene -> (retour) menu Start -> (retour) bibliotheque.
    if not st.audio then
        -- noeud de passage : enchaine sans lecteur audio
        story._pass = (story._pass or 0) + 1
        if story._pass > 30 then story._pass = 0; goto_library(); return end
        followTransition(st.ok)
        return
    end
    story._pass = 0
    Global.load_module("audio-player", "1_0_0").create({
        audio_path = st.audio,
        image_background_path = st.image,
        image_path = "empty.lif",
        image_foreground_path = "empty.lif",
        song_name = "",
        callback = function() followTransition(st.ok) end,
    })
end

-- ---------------------------------------------------------------------------
-- Choix multi-options
-- ---------------------------------------------------------------------------
function showChoice(list, actionId)
    -- Position SEULE, PAS un chapitre : seules les scenes comptent dans la jauge.
    -- Sinon #chapitres (scenes + choix) depasserait totalChapters (= nb de scenes)
    -- et .prog grimperait > 100 (hors plage observee sur device : 0..100).
    -- setProgression passe quand meme isStoryStarted a true => le plancher du
    -- wrapper getProgressionValue garantit .prog >= 1 si on quitte sur un choix.
    Global.progression.setProgression({ currentFunction = actionId, branch = BRANCH })
    local choices = {}
    for i, e in ipairs(list) do
        local opt = N.stages[e.stage] or {}
        choices[#choices + 1] = {
            img = opt.image or "empty.lif",
            audio = opt.audio,
            -- Label : texte notes.json (resolu au build dans stages[id].text)
            -- sinon "Choix N" (N = position dans CE choix ; non resoluble au
            -- build car une scene peut etre cible de plusieurs actions).
            label = opt.text or ("Choix " .. i),
            cb = function()
                applyStageItems(opt)
                followTransition(opt.ok)
            end,
        }
    end
    -- Selecteur choisi a la conversion (--selector). Defaut : carrousel.
    -- Idem : pas de setBackBehavior(goto_library). Retour -> menu Start (officiel).
    local module = (N.selector == "image") and "image-choice" or "carousel"
    Global.load_module(module, "1_0_0").create({
        choices = choices,
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
    if not trans then goto_library(); return end
    local list = N.actions[trans.action]
    if not list or #list == 0 then goto_library(); return end

    -- indexItem : l'index du stage dans l'action est la valeur d'un item inventaire
    if trans.indexItem ~= nil then
        local slot = state.inv and state.inv[trans.indexItem + 1]
        local idx = (slot and slot.value or 0) + 1  -- TELMI 0-based -> Lua 1-based
        local e = list[idx] or list[#list]           -- clamp sur le dernier si hors borne
        enterStage(e.stage)
        return
    end

    local hasCond = false
    for _, e in ipairs(list) do if e.cond then hasCond = true; break end end

    if #list == 1 then
        enterStage(list[1].stage)
    elseif hasCond then
        enterStage(pickByConditions(list).stage)
    else
        showChoice(list, trans.action)
    end
end

-- ---------------------------------------------------------------------------
-- Point d'entree par nom de noeud (appele par Global.current_branch[name]())
-- ---------------------------------------------------------------------------
function play(name)
    if name == "__start" then
        initInventory()           -- nouveau depart : inventaire neuf
        followTransition(N.start)
    elseif N.stages[name] then
        enterStage(name)          -- reprise sur une scene
    elseif N.actions[name] then
        showChoice(N.actions[name], name)  -- reprise sur un choix
    else
        goto_library()
    end
end

function story.clear()
    story._pass = 0
end

-- current_branch[<name>]() -> play(name) (sN, aN, __start)
setmetatable(story, {
    __index = function(_, name)
        return function() play(name) end
    end,
})

return story
