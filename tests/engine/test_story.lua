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

-- ---------------------------------------------------------------------------
-- Menu retour TELMI (backAction -> backStage -> backChildAction NON vide) :
-- "Quitter le jeu ?" (ok=nil) garde progression et inventaire (Calico Jack)
-- ---------------------------------------------------------------------------
local HOME = { action = "backAction", index = 0 }
local CTRL = { ok = true, home = true, autoplay = true }
local CTRLW = { ok = true, home = true, autoplay = false }
local function nodesMenu()
    return { inventory = { { name = "boussole", init = 0, max = 1 } }, start = { action = "a0" },
        stages = {
            s0 = { audio = "s0.mp3", ctrl = CTRL, home = HOME, ok = { action = "hub" } },
            take = { audio = "take.mp3", ctrl = CTRLW, home = HOME, items = { { type = 2, item = 0, number = 1 } },
                     ok = { action = "hub" } },
            look = { audio = "look.mp3", ctrl = CTRLW, home = HOME, ok = { action = "end" } },
            fin = { audio = "fin.mp3", ctrl = CTRL, home = HOME },
            backStage = { ctrl = { ok = true, home = false, autoplay = true }, home = HOME,
                          ok = { action = "backChildAction" } },
            resume = { audio = "resume.mp3", ctrl = CTRLW, home = HOME, ok = { action = "hub" } },
            hint = { audio = "hint.mp3", ctrl = CTRLW, home = HOME, ok = { action = "hints" } },
            hintScene = { audio = "hint1.mp3", ctrl = CTRL, home = HOME, ok = { action = "backAction" } },
            quit = { audio = "quit.mp3", ctrl = CTRLW, home = HOME },
        },
        actions = { a0 = { { stage = "s0" } }, hub = { { stage = "take" }, { stage = "look" } },
                    ["end"] = { { stage = "fin" } }, hints = { { stage = "hintScene" } },
                    backAction = { { stage = "backStage" } },
                    backChildAction = { { stage = "hint" }, { stage = "resume" }, { stage = "quit" } } },
        totalChapters = 5 }
end
local function opt(audio)
    for _, c in ipairs(lastModule.args.choices) do if c.audio == audio then return c end end
    error("option " .. audio .. " absente")
end
local function toHubWithItem(nodes)
    boot(nodes); menuPick(); audioEnd()          -- s0 -> hub
    opt("take.mp3").cb()                         -- option take (boussole=1) -> hub
end
local nm2 = nodesMenu()
toHubWithItem(nm2)
check("MENU objet ramasse", state.inv[1].value == 1 and lastModule.name == "carousel")
back_callback()
check("MENU retour -> menu interne", lastModule.name == "carousel" and #lastModule.args.choices == 3)
opt("quit.mp3").cb()
check("MENU Quitter -> menu Start Reprendre", lastModule.name == "list-choice"
      and menuLabel() == "Reprendre l'histoire", menuLabel())
check("MENU Quitter garde l'inventaire", state.inv and state.inv[1].value == 1)
check("MENU Quitter : position restauree sur le hub", state.current_fun == "hub" and state.current_kind == "a",
      tostring(state.current_fun))
check("MENU Quitter sans goto_library", #log == 0, table.concat(log, ","))
boot(nm2, true); menuPick()
check("MENU reprise sur le hub avec l'objet", lastModule.name == "carousel" and state.inv[1].value == 1)
-- indice (scene audio du menu) puis Quitter : origine de jeu conservee
toHubWithItem(nm2)
back_callback(); opt("hint.mp3").cb()               -- option indice -> scene hint1
check("MENU indice joue une scene", lastModule.name == "audio-player" and lastModule.args.audio_path == "hint1.mp3")
audioEnd()                                          -- hint1 -> backAction -> menu
check("MENU indice -> retour au menu interne", lastModule.name == "carousel" and #lastModule.args.choices == 3)
opt("quit.mp3").cb()
check("MENU indice puis Quitter : Reprendre + objet", menuLabel() == "Reprendre l'histoire"
      and state.inv[1].value == 1 and state.current_fun == "hub", tostring(state.current_fun))
-- retour au jeu par "Reprendre le jeu" : une vraie fin efface de nouveau tout
toHubWithItem(nm2)
back_callback(); opt("resume.mp3").cb()              -- Reprendre le jeu -> hub
check("MENU Reprendre -> hub", lastModule.name == "carousel" and #lastModule.args.choices == 2)
opt("look.mp3").cb()                                 -- look -> end -> scene fin
audioEnd()                                           -- fin (ok=nil) : vraie fin
check("MENU vraie fin apres Reprendre : remise a zero", menuLabel() == "Demarrer l'histoire"
      and state.inv == nil, menuLabel())
-- vraie fin sans passer par le menu : inchangee
boot(nm2); menuPick(); audioEnd()
opt("look.mp3").cb(); audioEnd()
check("MENU vraie fin directe : remise a zero", menuLabel() == "Demarrer l'histoire" and state.inv == nil)

-- scene finale atteinte par une transition home (pas une option de menu) :
-- reste une vraie fin (test_explore test_sous_menu_sorti_par_home)
boot({ inventory = { { name = "x", init = 0 } }, start = { action = "a0" },
       stages = { s0 = { audio = "s0.mp3", ctrl = CTRL, ok = { action = "m" } },
                  c1 = { audio = "c1.mp3", ctrl = CTRLW, home = { action = "ah" }, ok = { action = "m" } },
                  c2 = { audio = "c2.mp3", ctrl = CTRLW, home = { action = "ah" }, ok = { action = "m" } },
                  h = { audio = "h.mp3", ctrl = CTRL } },
       actions = { a0 = { { stage = "s0" } }, m = { { stage = "c1" }, { stage = "c2" } }, ah = { { stage = "h" } } },
       totalChapters = 4 })
menuPick(); audioEnd(); back_callback()
check("HOME scene finale jouee", lastModule.name == "audio-player" and lastModule.args.audio_path == "h.mp3")
audioEnd()
check("HOME scene finale = vraie fin", menuLabel() == "Demarrer l'histoire" and state.inv == nil, menuLabel())

-- ---------------------------------------------------------------------------
-- Reprise : inventaire sauve illisible (absent, cles texte) -> garde-fou
-- ---------------------------------------------------------------------------
toHubWithItem(nm2)
back_callback(); opt("quit.mp3").cb()
state.inv = { ["1"] = { value = 1, max = 1 } }       -- cles devenues texte
boot(nm2, true); menuPick()
check("INV cles texte relues", type(state.inv[1]) == "table" and state.inv[1].value == 1)
state.inv = nil                                      -- inventaire perdu par la sauvegarde
boot(nm2, true); menuPick()
check("INV absent -> reinitialise", type(state.inv) == "table" and state.inv[1].value == 0)
opt("take.mp3").cb()
check("INV objet ramasse apres reinitialisation", state.inv[1].value == 1)

-- ---------------------------------------------------------------------------
-- Revue : reprise sur le menu retour, #44 avec backStage atteint par ok,
-- menu atteint par ok, inventaire agrandi par une mise a jour du pack
-- ---------------------------------------------------------------------------
-- reprise (relance) pendant que le menu retour est affiche, puis Quitter
toHubWithItem(nm2)
back_callback()
check("REV menu affiche, origine sauvee", state.home_origin and state.home_origin.id == "hub")
boot(nm2, true); menuPick()                          -- relance -> Reprendre
check("REV reprise sur le menu retour", lastModule.name == "carousel" and #lastModule.args.choices == 3)
opt("quit.mp3").cb()
check("REV reprise menu puis Quitter : objet garde", menuLabel() == "Reprendre l'histoire"
      and state.inv[1].value == 1 and state.current_fun == "hub", tostring(state.current_fun))
-- meme chose via le menu M (LateralResume), sans relance
toHubWithItem(nm2)
back_callback()
LateralResume()
check("REV menu M -> menu retour", lastModule.name == "carousel" and #lastModule.args.choices == 3)
opt("quit.mp3").cb()
check("REV menu M puis Quitter : objet garde", menuLabel() == "Reprendre l'histoire" and state.inv[1].value == 1)
-- origine sauvee invalide (noeud disparu) : ignoree, Quitter = fin
toHubWithItem(nm2)
back_callback()
state.home_origin = { id = "disparu", kind = "s" }
boot(nm2, true); menuPick(); opt("quit.mp3").cb()
check("REV origine invalide ignoree", menuLabel() == "Demarrer l'histoire", menuLabel())
-- reprise sur un noeud de jeu : origine perimee effacee
toHubWithItem(nm2)
state.home_origin = { id = "hub", kind = "a" }
boot(nm2, true); menuPick()
check("REV reprise sur le jeu efface l'origine", state.home_origin == nil)

-- #44 : backChildAction vide, backStage AUSSI atteint par ok depuis le jeu
local function nodes44()
    return { inventory = { { name = "x", init = 0, max = 1 } }, start = { action = "a0" },
        stages = { s0 = { audio = "s0.mp3", ctrl = CTRLW, home = HOME, items = { { type = 2, item = 0, number = 1 } },
                          ok = { action = "a1" } },
                   s1 = { audio = "s1.mp3", ctrl = CTRLW, home = HOME, ok = { action = "backAction" } },
                   backStage = { ctrl = { ok = true, home = false, autoplay = true }, ok = { action = "backChildAction" } } },
        actions = { a0 = { { stage = "s0" } }, a1 = { { stage = "s1" } }, backAction = { { stage = "backStage" } },
                    backChildAction = {} }, totalChapters = 2 }
end
boot(nodes44()); menuPick()
check("REV44 objet pose", state.inv[1].value == 1)
back_callback()
check("REV44 retour -> menu Start sans effacer", menuLabel() == "Reprendre l'histoire" and state.inv[1].value == 1
      and state.current_fun == "s0", menuLabel())

-- menu retour AUSSI atteint par ok (scene "info" -> backAction) : toujours menu
local nmInfo = nodesMenu()
nmInfo.stages.info = { audio = "info.mp3", ctrl = CTRLW, home = HOME, ok = { action = "backAction" } }
nmInfo.actions.hub[#nmInfo.actions.hub + 1] = { stage = "info" }
toHubWithItem(nmInfo)
back_callback(); opt("quit.mp3").cb()
check("REV menu atteint aussi par ok : Quitter garde l'objet", menuLabel() == "Reprendre l'histoire"
      and state.inv[1].value == 1, menuLabel())

-- depart = scene de passage de routage, cible d'une home "menu principal" :
-- l'action de depart reste un noeud de jeu (#44 garde la progression)
boot({ inventory = { { name = "x", init = 0, max = 1 } }, start = { action = "a0" },
       stages = { route = { ctrl = { ok = true, home = false, autoplay = true }, ok = { action = "hub" } },
                  h1 = { audio = "h1.mp3", ctrl = CTRLW, home = { action = "a0" }, items = { { type = 2, item = 0, number = 1 } },
                         ok = { action = "a2" } },
                  h2 = { audio = "h2.mp3", ctrl = CTRLW, home = HOME, ok = { action = "hub" } },
                  backStage = { ctrl = { ok = true, home = false, autoplay = true }, ok = { action = "backChildAction" } } },
       actions = { a0 = { { stage = "route" } }, hub = { { stage = "h1" } }, a2 = { { stage = "h2" } },
                   backAction = { { stage = "backStage" } }, backChildAction = {} }, totalChapters = 2 })
menuPick(); lastModule.args.okCallback()             -- h1 (objet) -> h2
back_callback()                                      -- h2 : home -> backAction -> vide
check("REV depart en passage : retour garde la progression", menuLabel() == "Reprendre l'histoire"
      and state.inv[1].value == 1 and state.current_fun == "h2", menuLabel())

-- forme "Pere Noel" : backStage repris dans une action de jeu (a109, apres une
-- scene info) ; le menu interne reste un menu, Quitter garde l'objet
local nmPN = nodesMenu()
nmPN.stages.info = { audio = "info.mp3", ctrl = CTRLW, home = HOME, ok = { action = "a109" } }
nmPN.actions.a109 = { { stage = "backStage" } }
nmPN.actions.hub[#nmPN.actions.hub + 1] = { stage = "info" }
toHubWithItem(nmPN)
back_callback(); opt("quit.mp3").cb()
check("REV forme Pere Noel : Quitter garde l'objet", menuLabel() == "Reprendre l'histoire"
      and state.inv[1].value == 1, menuLabel())

-- pack mis a jour avec un objet de plus : objets ramasses gardes
toHubWithItem(nm2)
back_callback(); opt("quit.mp3").cb()
local nmBig = nodesMenu()
nmBig.inventory[2] = { name = "pelle", init = 2, max = 3 }
boot(nmBig, true); menuPick()
check("REV inventaire agrandi : objet garde + nouveau a init", state.inv[1].value == 1 and state.inv[2].value == 2,
      tostring(state.inv[2] and state.inv[2].value))

-- ---------------------------------------------------------------------------
-- CHAP : livres a chapitres (role "chapter" / "title", opt-in, launcher) :
-- chapitre quitte par Home repris a sa position ; sans role, rien ne change.
-- ---------------------------------------------------------------------------
-- Livre du launcher : e1 -> e2 -> annonce n3 -> e3 -> menu m0 ; home d'un
-- chapitre -> son option ; options sans home (retour = menu Start).
local CTRLP = { ok = true, home = true, autoplay = true }
local CTRLO = { ok = true, home = false, autoplay = false }
local function bookNodes(withRole)
    local ch = withRole and "chapter" or nil
    local ti = withRole and "title" or nil
    return { start = { action = "begin" }, totalChapters = 3,
        stages = {
            start = { ok = { action = "p1" }, ctrl = { ok = false, home = false, autoplay = true } },
            e1 = { image = "e1.lif", audio = "e1.mp3", ok = { action = "p2" }, home = { action = "m0", index = 0 },
                   ctrl = CTRLP, role = ch },
            e2 = { image = "e2.lif", audio = "e2.mp3", ok = { action = "p3" }, home = { action = "m0", index = 1 },
                   ctrl = CTRLP, role = ch },
            n3 = { image = "e3.lif", audio = "t3.mp3", ok = { action = "q3" }, home = { action = "m0", index = 2 },
                   ctrl = CTRLP, role = ti },
            e3 = { image = "e3.lif", audio = "e3.mp3", ok = { action = "m0" }, home = { action = "m0", index = 2 },
                   ctrl = CTRLP, role = ch },
            o1 = { image = "e1.lif", audio = "t1.mp3", ok = { action = "p1" }, ctrl = CTRLO, text = "C1" },
            o2 = { image = "e2.lif", audio = "t2.mp3", ok = { action = "p2" }, ctrl = CTRLO, text = "C2" },
            o3 = { image = "e3.lif", audio = "t3.mp3", ok = { action = "q3" }, ctrl = CTRLO, text = "C3" },
        },
        actions = { begin = { { stage = "start" } }, p1 = { { stage = "e1" } }, p2 = { { stage = "e2" } },
                    p3 = { { stage = "n3" } }, q3 = { { stage = "e3" } },
                    m0 = { { stage = "o1" }, { stage = "o2" }, { stage = "o3" } } } }
end
local function seek(id) return state.visited_funs[id] and state.visited_funs[id].seekposition end
-- Demarre le livre et quitte le chapitre 2 par Home a la position 42 (posee
-- chaque seconde par audio-player sur device).
local function leaveChapter2(nodes)
    boot(nodes); menuPick(); audioEnd()
    state.visited_funs.e2.seekposition = 42
    back_callback()
end
local bookR, bookN = bookNodes(true), bookNodes(false)

leaveChapter2(bookR)
check("CHAP Home -> menu des chapitres focalise sur C2", lastModule.name == "carousel" and lastModule.args.index == 2,
      lastModule.name)
check("CHAP position du chapitre quitte gardee", seek("e2") == 42, seek("e2"))
check("CHAP chapitre quitte retenu", state.chapter_left == "e2", state.chapter_left)
back_callback()   -- retour depuis le menu : menu Start (sortie de l'histoire)
check("CHAP retour du menu -> menu Start, Reprendre", lastModule.name == "list-choice"
      and menuLabel() == "Reprendre l'histoire", menuLabel())
check("CHAP pas de goto_library", #log == 0, table.concat(log, ","))
boot(bookR, true); menuPick()   -- relance, Reprendre l'histoire
check("CHAP Reprendre -> chapitre quitte", lastModule.name == "audio-player"
      and lastModule.args.audio_path == "e2.mp3", lastModule.args.audio_path)
check("CHAP Reprendre -> a sa position", seek("e2") == 42 and state.current_fun == "e2", seek("e2"))
check("CHAP Reprendre -> chapitre quitte consomme", state.chapter_left == nil, state.chapter_left)
audioEnd()   -- la suite continue normalement (annonce de C3)
check("CHAP la suite continue", lastModule.args.audio_path == "t3.mp3", lastModule.args.audio_path)

-- veille dans le menu des chapitres : meme reprise
leaveChapter2(bookR)
boot(bookR, true); menuPick()
check("CHAP veille dans le menu -> chapitre quitte a sa position", lastModule.args.audio_path == "e2.mp3"
      and seek("e2") == 42, seek("e2"))

-- valider dans le menu le chapitre qu'on vient de quitter : reprise a la position
leaveChapter2(bookR)
lastModule.args.choices[2].cb()
check("CHAP menu -> chapitre quitte a sa position", lastModule.args.audio_path == "e2.mp3" and seek("e2") == 42,
      seek("e2"))
check("CHAP menu -> chapitre quitte consomme", state.chapter_left == nil)

-- un autre chapitre repart du debut ; le chapitre quitte aussi ensuite
leaveChapter2(bookR)
state.visited_funs.e1.seekposition = 10
lastModule.args.choices[1].cb()
check("CHAP menu -> autre chapitre depuis le debut", lastModule.args.audio_path == "e1.mp3" and seek("e1") == nil,
      seek("e1"))
check("CHAP autre chapitre : plus de chapitre quitte", state.chapter_left == nil)
audioEnd()
check("CHAP chapitre quitte rejoint par la suite : depuis le debut", lastModule.args.audio_path == "e2.mp3"
      and seek("e2") == nil, seek("e2"))

-- Demarrer efface le chapitre quitte
leaveChapter2(bookR)
Global.current_branch["__start"]()
check("CHAP Demarrer : plus de chapitre quitte", state.chapter_left == nil
      and lastModule.args.audio_path == "e1.mp3", state.chapter_left)

-- jauge : l'annonce (role "title") n'est pas un chapitre
boot(bookR); menuPick(); audioEnd(); audioEnd()
check("CHAP annonce jouee", lastModule.args.audio_path == "t3.mp3" and state.current_fun == "n3")
check("CHAP annonce hors jauge", #store.chaps == 2, #store.chaps)
audioEnd()
check("CHAP 3 chapitres = jauge pleine", #store.chaps == 3 and Global.progression.getProgressionValue() == 100,
      #store.chaps)
-- chapitre quitte qui n'existe plus (pack mis a jour) : reprise normale
leaveChapter2(bookR)
local bookR2 = bookNodes(true)
bookR2.stages.e2.role = nil
back_callback()
boot(bookR2, true); menuPick()
check("CHAP chapitre quitte devenu inconnu : reprise du noeud sauve", lastModule.name == "carousel"
      and state.chapter_left == nil, lastModule.name)

-- Non-regression : meme graphe SANS role = comportement historique
leaveChapter2(bookN)
check("CHAP sans role : position effacee par Home", seek("e2") == nil, seek("e2"))
check("CHAP sans role : pas de chapitre quitte", state.chapter_left == nil)
lastModule.args.choices[2].cb()
check("CHAP sans role : chapitre repris du debut", lastModule.args.audio_path == "e2.mp3" and seek("e2") == nil)
leaveChapter2(bookN)
back_callback()
boot(bookN, true); menuPick()
check("CHAP sans role : Reprendre -> menu des chapitres", lastModule.name == "carousel", lastModule.name)
boot(bookN); menuPick(); audioEnd(); audioEnd()
check("CHAP sans role : annonce comptee comme chapitre", #store.chaps == 3, #store.chaps)

-- flam-test appelle setup() s'il existe apres le chargement : main.lua l'a
-- defini, on le retire pour ne pas relancer l'histoire hors test.
setup = nil

print(string.format("Results: %d passed, %d failed", TEST_PASS, TEST_FAIL))
if STANDALONE then os.exit(TEST_FAIL == 0 and TEST_PASS > 0 and 0 or 1) end
