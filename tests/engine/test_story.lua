-- test_story.lua — moteur d'histoire telmi2flam (engine/story.lua + main.lua)
-- avec le VRAI progressionManager et des mocks (modules d'ecran, progression
-- persistante, Global). Aucune API LVGL requise.
--
-- Lancement :
--   flam-test tests\engine\test_story.lua      (suite test_run.bat)
--   lua54 tests\engine\test_story.lua          (Lua 5.4 autonome)
--
-- Couvre F01, F08, F24, F25, F26, F12, F45 (+ reprise sur noeud inconnu).
-- F27 (ctrl autoplay/ok/home, index) : tests/engine/test_story_f27.lua.

local src = debug.getinfo(1, "S").source
local HERE = src:sub(1, 1) == "@" and src:sub(2):match("^(.*)[/\\]") or "."
local ROOT = HERE .. "/../.."
-- FLAM_ENGINE_DIR : autre copie du moteur (ex. verifier que les tests echouent sur l'original)
local ENGINE = os.getenv("FLAM_ENGINE_DIR") or (ROOT .. "/tools/telmi2flam/engine/")
local RUNTIME = ROOT .. "/tools/telmi2flam/runtime/script/"
package.path = HERE .. "/../lua/?.lua;" .. ENGINE .. "?.lua;" .. RUNTIME .. "?.lua;" .. package.path
local STANDALONE = (test_tick == nil)   -- pas lance par flam-test

require("test_helpers")
print("--- test_story (moteur telmi2flam) ---")

local function check(name, cond, info)
    test(name, function()
        if not cond then error(info ~= nil and tostring(info) or "condition fausse", 0) end
    end)
end

-- ---------------------------------------------------------------------------
-- Mocks firmware / Global
-- ---------------------------------------------------------------------------
local log, lastModule, audioStops
local store = {}
progression = {
    load = function(k) local t = store[k] or {}; local c = {}; for i, v in pairs(t) do c[i] = v end; return c end,
    save = function(k, v) store[k] = v end,
}
function goto_library() log[#log + 1] = "goto_library" end
back_callback = goto_library

local function newModule(name)
    return { answerIterator = 1,
             create = function(args) lastModule = { name = name, args = args } end,
             display = function(args) lastModule = { name = name, args = args } end,
             clean = function() end }
end

Global = {
    audioDuration = 0, current_module = nil, current_branch = nil,
    table_length = function(t) local n = 0; for _ in pairs(t) do n = n + 1 end; return n end,
    requestAudioStop = function() audioStops = audioStops + 1 end,
    cleanCurrentModule = function() Global.current_module = nil end,
    removeAudioFeedbackCB = function() end,
    load_module = function(name, v)
        local m = newModule(name); Global.current_module = m; return m
    end,
    setBackBehavior = function(b, a)
        back_callback = function()
            Global.requestAudioStop(true, true); Global.cleanCurrentModule()
            if a ~= nil then b(a) else b() end
        end
    end,
    setBackToLibrary = function() back_callback = function() goto_library() end end,
    setDefaultAudioPlayerCover = function() end,
    init = function() end,
    loadBranch = function(name)
        package.loaded[name] = nil
        Global.current_branch = require(name); Global.current_branch_name = name
    end,
}
local GlobalMock = Global
package.loaded["global"] = Global

-- Demarre main.lua sur un graphe `nodes` (keepState : relance sans effacer
-- la sauvegarde, comme un 2e lancement de l'histoire)
local function boot(nodes, keepState)
    Global = GlobalMock
    package.loaded["global"] = GlobalMock
    package.loaded["nodes"] = nodes
    package.loaded["story"] = nil
    package.loaded["progressionManager"] = nil
    Global.progression = require("progressionManager")
    if not keepState then state = {}; store = {} end
    state.visited_funs = state.visited_funs or {}
    log, lastModule, audioStops = {}, nil, 0
    context_menu = nil
    dofile(ENGINE .. "main.lua")
    setup()
end
local function menuLabel() return lastModule and lastModule.args.choices and lastModule.args.choices[1].label end
local function menuPick(i) lastModule.args.choices[i or 1].cb() end
local function audioEnd() lastModule.args.callback() end -- fin audio du audio-player

-- ---------------------------------------------------------------------------
-- F01 : 1er noeud sans audio (Demarrer)
-- ---------------------------------------------------------------------------
local nodesF01 = { start = { action = "a0" }, stages = { s0 = { ok = { action = "a1" } }, s1 = { audio = "s1.mp3" } },
                   actions = { a0 = { { stage = "s0" } }, a1 = { { stage = "s1" } } }, totalChapters = 1 }
boot(nodesF01)
check("F01 menu Start initial = Demarrer", menuLabel() == "Demarrer l'histoire", menuLabel())
local ok, err = pcall(menuPick)
check("F01 Demarrer sur noeud sans audio sans erreur", ok, err)
check("F01 arrive sur audio-player s1", lastModule.name == "audio-player" and lastModule.args.audio_path == "s1.mp3")
local st = Global.current_branch
check("F01 story._pass non resolu par __index", rawget(st, "_pass") == nil and st._pass == nil)
check("F01 cle inconnue -> nil", st.foo == nil and st.clear ~= nil)
check("F01 noeuds du graphe resolus", type(st.s0) == "function" and type(st.a1) == "function"
      and type(st["__start"]) == "function")

-- ---------------------------------------------------------------------------
-- F08 : fin d'histoire -> menu Start avec progression remise a zero
-- ---------------------------------------------------------------------------
boot({ start = { action = "a0" }, stages = { s0 = { audio = "fin.mp3" } }, actions = { a0 = { { stage = "s0" } } },
       inventory = { { name = "x", init = 0 } }, totalChapters = 1 })
menuPick()
check("F08 progression enregistree", state.current_fun == "s0" and #store.chaps == 1)
audioEnd()
check("F08 pas de goto_library depuis la scene", #log == 0, table.concat(log, ","))
check("F08 retour au menu Start (list-choice)", lastModule.name == "list-choice", lastModule.name)
check("F08 menu propose de nouveau Demarrer", menuLabel() == "Demarrer l'histoire", menuLabel())
check("F08 reset current_fun/visited/inv/chaps", state.current_fun == nil and next(state.visited_funs) == nil
      and state.inv == nil and #store.chaps == 0)
check("F08 audio stoppe", audioStops >= 1)
menuPick()
check("F08 on peut recommencer", lastModule.name == "audio-player" and state.current_fun == "s0")
-- retour depuis Start -> bibliotheque (seul goto_library legitime)
audioEnd(); back_callback()
check("F08 goto_library uniquement depuis Start", #log == 1 and log[1] == "goto_library", table.concat(log, ","))
-- garde-fous : action inconnue, stage inconnu
boot({ start = { action = "zz" }, stages = {}, actions = {}, totalChapters = 1 })
menuPick()
check("F08 action inconnue -> Start, pas goto_library", #log == 0 and lastModule.name == "list-choice")
boot({ start = { action = "a0" }, stages = {}, actions = { a0 = { { stage = "nope" } } }, totalChapters = 1 })
menuPick()
check("F08 stage inconnu -> Start, pas goto_library", #log == 0 and lastModule.name == "list-choice")

-- ---------------------------------------------------------------------------
-- F24 : reprise ne reapplique pas les items ni le reset
-- ---------------------------------------------------------------------------
local nodesF24 = { inventory = { { name = "cle", init = 0, max = 5 } }, start = { action = "a0" },
    stages = { s0 = { audio = "s0.mp3", items = { { type = 0, item = 0, number = 1 } }, ok = { action = "a1" } },
               s1 = { audio = "s1.mp3", reset = true } },
    actions = { a0 = { { stage = "s0" } }, a1 = { { stage = "s1" } } }, totalChapters = 2 }
boot(nodesF24); menuPick()
check("F24 items appliques au 1er passage", state.inv[1].value == 1)
boot(nodesF24, true)   -- relance : title-card absente -> Start
check("F24 menu = Reprendre", menuLabel() == "Reprendre l'histoire", menuLabel())
menuPick()
check("F24 reprise sur s0 sans re-appliquer", state.current_fun == "s0" and state.inv[1].value == 1, state.inv[1].value)
boot(nodesF24, true); menuPick(); check("F24 2e reprise idem", state.inv[1].value == 1)
-- apres la reprise, la suite applique normalement
state.inv[1].value = 3
audioEnd()   -- s1 reset=true
check("F24 scene suivante applique son reset", state.current_fun == "s1" and state.inv[1].value == 0)
state.inv[1].value = 4
boot(nodesF24, true); menuPick()
check("F24 reprise sur scene reset : pas de reset", state.inv[1].value == 4)

-- ---------------------------------------------------------------------------
-- F25 : boucle menu -> option -> backStage (sans audio) -> menu
-- ---------------------------------------------------------------------------
boot({ start = { action = "a0" }, stages = { s0 = { audio = "i.mp3", ok = { action = "m" } },
    o1 = { audio = "o1.mp3", ok = { action = "b" } }, o2 = { audio = "o2.mp3", ok = { action = "b" } },
    back = { ok = { action = "m" } } },
    actions = { a0 = { { stage = "s0" } }, m = { { stage = "o1" }, { stage = "o2" } }, b = { { stage = "back" } } },
    totalChapters = 3 })
menuPick(); audioEnd()
local n = 0
while lastModule.name == "carousel" and #log == 0 and n < 200 do
    n = n + 1
    menuPick(1)   -- choix o1 -> audio-player
    if lastModule.name == "audio-player" then audioEnd() end   -- fin o1 -> back -> m
end
check("F25 200 tours choix/scene/backStage sans sortie", n == 200 and lastModule.name == "carousel",
      n .. " " .. lastModule.name)
-- vraie boucle de noeuds sans audio -> sortie propre
boot({ start = { action = "a0" }, stages = { p = { ok = { action = "a0" } } }, actions = { a0 = { { stage = "p" } } },
       totalChapters = 1 })
ok, err = pcall(menuPick)
check("F25 boucle infinie sans audio -> sortie propre", ok and #log == 0 and lastModule.name == "list-choice"
      and menuLabel() == "Demarrer l'histoire", err)
-- double appel du cb (ENTER maintenu) : items appliques une seule fois
boot({ inventory = { { name = "or", init = 0 } }, start = { action = "a0" },
    stages = { s0 = { audio = "i.mp3", ok = { action = "m" } },
               o1 = { audio = "o1.mp3", items = { { type = 0, item = 0, number = 10 } }, ok = { action = "b" } },
               o2 = { audio = "o2.mp3" }, back = { ok = { action = "m" } } },
    actions = { a0 = { { stage = "s0" } }, m = { { stage = "o1" }, { stage = "o2" } }, b = { { stage = "back" } } },
    totalChapters = 3 })
menuPick(); audioEnd()
local cb = lastModule.args.choices[1].cb
for _ = 1, 5 do cb() end
check("F25 cb d'option idempotent (ENTER maintenu)", state.inv[1].value == 10, state.inv[1].value)

-- ---------------------------------------------------------------------------
-- F26 : filtrage par conditions
-- ---------------------------------------------------------------------------
local function nodesF26(m)
    return { inventory = { { name = "x", init = 0 } }, start = { action = "a0" },
        stages = { s0 = { audio = "i.mp3", ok = { action = "m" } }, o1 = { audio = "o1.mp3" },
                   o2 = { audio = "o2.mp3" }, o3 = { audio = "o3.mp3" } },
        actions = { a0 = { { stage = "s0" } }, m = m }, totalChapters = 4 }
end
local X_GT0 = { { cmp = 3, item = 0, num = 0 } }
boot(nodesF26({ { stage = "o1" }, { stage = "o2" }, { stage = "o3", cond = X_GT0 } }))
menuPick(); audioEnd()
check("F26 action mixte -> choix des 2 options visibles",
      lastModule.name == "carousel" and #lastModule.args.choices == 2, lastModule.name)
boot(nodesF26({ { stage = "o1", cond = X_GT0 }, { stage = "o2", cond = X_GT0 } }))
menuPick(); audioEnd()
check("F26 aucune option accessible -> sortie propre (pas o2)", lastModule.name == "list-choice" and #log == 0,
      lastModule.name)
boot(nodesF26({ { stage = "o1", cond = X_GT0 }, { stage = "o2" } }))
menuPick(); audioEnd()
check("F26 1 seule visible -> scene directe",
      lastModule.name == "audio-player" and lastModule.args.audio_path == "o2.mp3")
boot(nodesF26({ { stage = "o1", cond = { { cmp = 2, item = 0, num = 0 } } }, { stage = "o2", cond = X_GT0 } }))
menuPick(); audioEnd()
check("F26 branchement conditionnel (1ere vraie)", lastModule.args.audio_path == "o1.mp3")
-- reprise sur un choix : meme filtre
local nm = nodesF26({ { stage = "o1" }, { stage = "o2" }, { stage = "o3", cond = X_GT0 } })
boot(nm); menuPick(); audioEnd()
boot(nm, true); menuPick()
check("F26 reprise sur choix filtre", lastModule.name == "carousel" and #lastModule.args.choices == 2)

-- ---------------------------------------------------------------------------
-- F45 : option sans audio + preselection
-- ---------------------------------------------------------------------------
boot({ start = { action = "a0" }, stages = { s0 = { audio = "i.mp3", ok = { action = "m", index = 2 } },
       o1 = { audio = "o1.mp3" }, o2 = {}, o3 = { audio = "o3.mp3" } },
       actions = { a0 = { { stage = "s0" } }, m = { { stage = "o1" }, { stage = "o2" }, { stage = "o3" } } },
       totalChapters = 3 })
menuPick(); audioEnd()
check("F45 option sans audio -> silent.mp3", lastModule.args.choices[2].audio == "silent.mp3")
check("F45 preselection index=2 -> 3e option", Global.current_module.answerIterator == 3 and lastModule.args.index == 3)
boot({ start = { action = "a0" }, stages = { s0 = { audio = "i.mp3", ok = { action = "m", index = 7 } },
       o1 = { audio = "o1.mp3" }, o2 = { audio = "o2.mp3" } },
       actions = { a0 = { { stage = "s0" } }, m = { { stage = "o1" }, { stage = "o2" } } }, totalChapters = 3 })
menuPick(); audioEnd()
check("F45 index hors borne -> 1", Global.current_module.answerIterator == 1 and lastModule.args.index == 1)

-- ---------------------------------------------------------------------------
-- F12 : title-card sans title.mp3
-- ---------------------------------------------------------------------------
boot({ title = { image = "t.lif" }, start = { action = "a0" }, stages = { s0 = { audio = "s0.mp3" } },
       actions = { a0 = { { stage = "s0" } } }, totalChapters = 1 })
check("F12 title-card sans audio -> silent.mp3", lastModule.name == "title-card" and lastModule.args.audio == "silent.mp3")
-- la title-card avance sur back_callback (fin audio) -> menu Start
lastModule.args.cb()
check("F12 fin de title-card -> menu Start", lastModule.name == "list-choice" and menuLabel() == "Demarrer l'histoire")
boot({ title = { image = "t.lif", audio = "title.mp3" }, start = { action = "a0" }, stages = { s0 = { audio = "s0.mp3" } },
       actions = { a0 = { { stage = "s0" } } }, totalChapters = 1 })
check("F12 title-card avec audio conserve", lastModule.args.audio == "title.mp3")

-- ---------------------------------------------------------------------------
-- Reprise sur noeud inconnu (save d'un ancien pack)
-- ---------------------------------------------------------------------------
boot(nodesF01)
state.current_fun = "s999"; state.currentBranchName = "story"; state.visited_funs = { s999 = {} }
ok, err = pcall(LoadCurrentFunction)
check("Reprise noeud inconnu -> redemarrage", ok and lastModule.name == "audio-player", err)

-- flam-test appelle setup() s'il existe apres le chargement : main.lua l'a
-- defini, on le retire pour ne pas relancer l'histoire hors test.
setup = nil

print(string.format("Results: %d passed, %d failed", TEST_PASS, TEST_FAIL))
if STANDALONE then os.exit(TEST_FAIL == 0 and TEST_PASS > 0 and 0 or 1) end
