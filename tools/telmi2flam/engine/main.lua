-- ============================================================================
-- main.lua — Bootstrap FLAM pour histoires converties depuis TELMI.
--
-- Reproduit le flux des histoires Lunii officielles (Cluedo) :
--   setup() -> title-card -> Start() (menu Demarrer / Reprendre / Inventaire)
--   -> loadBranch("story") -> current_branch[<noeud>]()
--
-- La logique de scenes est dans le branch script/story.lua. La reprise utilise
-- les champs STANDARD (state.current_fun + state.currentBranchName) sauves par
-- Global.progression.setProgression, exactement comme les histoires officielles.
-- ============================================================================

-- Pas de require() au top-level (le firmware installe le searcher apres le
-- chargement de main.lua). N est requis dans setup().
local N

-- Retourne true si au moins un item inventaire est visible (display != 2).
local function hasVisibleInventory()
    if not N.inventory then return false end
    for _, def in ipairs(N.inventory) do
        if (def.display or 0) ~= 2 then return true end
    end
    return false
end

-- Ecran inventaire : liste les items visibles et non-vides avec leur valeur courante.
-- backCb est appele quand l'utilisateur selectionne un item ou revient en arriere.
function ShowInventory(backCb)
    if not hasVisibleInventory() then backCb(); return end
    local entries = {}
    for i, def in ipairs(N.inventory) do
        local mode = def.display or 0
        if mode ~= 2 then
            local slot = state.inv and state.inv[i] or { value = def.init or 0 }
            local val  = slot.value or 0
            -- Filtrage : count a zero ET init=0 = jamais acquis, on masque.
            -- Gauges (mode=1) et items avec init>0 : toujours visibles.
            local init = def.init or 0
            if val > 0 or mode == 1 or init > 0 then
                local entry = {
                    img   = def.image or "empty.lif",
                    audio = "silent.mp3",
                    cb    = backCb,
                }
                if mode == 1 then
                    -- Gauge : barre de progression + label simple
                    entry.label = def.name or "?"
                    local pct = 0
                    if def.max and def.max > 0 then
                        pct = math.min(100, math.floor(val / def.max * 100))
                    end
                    entry.slider = { percentage_value = pct }
                else
                    -- Compteur : valeur/max dans le label
                    if def.max and def.max > 0 then
                        entry.label = (def.name or "?") .. " : " .. val .. "/" .. def.max
                    else
                        entry.label = (def.name or "?") .. " : " .. val
                    end
                end
                table.insert(entries, entry)
            end
        end
    end
    -- Inventaire vide : afficher un message plutot que de revenir silencieusement
    if #entries == 0 then
        entries[1] = {
            label = "Inventaire vide, revenez plus tard !",
            img   = "empty.lif",
            audio = "silent.mp3",
            cb    = backCb,
        }
    end
    Global.setBackBehavior(backCb)
    Global.load_module("list-choice", "1_0_0").create({ choices = entries })
end

function IntroCard()
    local meta = N.meta or {}
    if N.title and N.title.image then
        Global.load_module("title-card", "").display({
            title = meta.title or "",
            subtitle = meta.subtitle or "",
            audio = N.title.audio,
            img = N.title.image,
            cb = Start,
        })
    else
        Start()
    end
end

function Start()
    local items = {}
    if Global.progression.isStoryStarted() then
        items[#items + 1] = {
            label = "Reprendre l'histoire",
            cb = LoadCurrentFunction,
            img = "empty.lif",
            audio = "silent.mp3",
            slider = { percentage_value = Global.progression.getProgressionValue() },
        }
    else
        items[#items + 1] = {
            label = "Demarrer l'histoire",
            cb = LoadStartFunction,
            img = "empty.lif",
            audio = "silent.mp3",
        }
    end
    if hasVisibleInventory() then
        items[#items + 1] = {
            label = "Inventaire",
            cb = function() ShowInventory(Start) end,
            img = "empty.lif",
            audio = "silent.mp3",
        }
    end
    Global.setBackToLibrary()
    Global.load_module("list-choice", "1_0_0").create({ choices = items })
end

-- Nouveau depart : charge le branch et joue le noeud de demarrage.
function LoadStartFunction()
    Global.loadBranch("story")
    Global.current_branch["__start"]()
end

-- Reprise : recharge le branch sauve et rejoue le noeud courant (comme Cluedo).
function LoadCurrentFunction()
    Global.loadBranch(state.currentBranchName)
    Global.current_branch[state.current_fun]()
end

-- Entree du menu contextuel M : reprend si l'histoire est commencee, sinon demarre.
function LateralResume()
    if Global.progression.isStoryStarted() then
        LoadCurrentFunction()
    else
        LoadStartFunction()
    end
end

function setup()
    N = require("nodes")
    Global = require("global")
    Global.init()
    -- Les noeuds vivent dans la branche "story" (loadBranch + current_branch[fn]),
    -- donc l'histoire est "multi-branches" du point de vue du moteur.
    Global.isMultiBranches = true

    -- RISQUE DEVICE (issue #1) : le firmware Lunii FIGE l'ecran (noir, sans input)
    -- au 2e lancement d'une histoire commencee dont le `.prog` vaut 0.
    -- Or `.prog` = getProgressionValue() = floor(#chapitres / totalChapters * 100).
    -- Avec le VRAI totalChapters (= nb de scenes, souvent > 100), les premieres scenes
    -- donnent floor(<1%) = 0  =>  .prog = 0  =>  freeze a la relance.
    -- Choix : garder le vrai totalChapters (jauge de progression fidele) et PLANCHER
    -- le resultat a 1 des que l'histoire est commencee. Ainsi `.prog` ne tombe jamais
    -- a 0 (device OK) sans fausser la progression affichee. Le wrapper est idempotent.
    local _getProgressionValue = Global.progression.getProgressionValue
    Global.progression.getProgressionValue = function(...)
        local v = _getProgressionValue(...)
        if v > 100 then v = 100 end   -- jamais > 100 (plage observee sur device : 0..100)
        if Global.progression.isStoryStarted() and v < 1 then v = 1 end
        return v
    end

    -- backBehavior = Start : la title-card avance en appelant back_callback(),
    -- progression.create installe back_callback = (cleanup + Start).
    Global.progression.create({ totalChapters = N.totalChapters or 1, backBehavior = Start })
    Global.setDefaultAudioPlayerCover("empty.lif", "empty.lif", "empty.lif")
    IntroCard()

    -- Menu contextuel M (touche M / appui long sur device) : uniquement "Reprendre".
    -- IMPORTANT (issue #1) : au 2e lancement SOFT d'une histoire convertie, l'ecran
    -- reste NOIR (bug firmware). Le menu contextuel firmware reconstruit l'ecran au
    -- niveau C : "Reprendre l'histoire" est la SEULE facon de revenir sans hard reboot.
    if context_menu ~= nil then
        context_menu.set_entries({
            { title = "Reprendre l'histoire", cb = LateralResume },
        })
    end
end
