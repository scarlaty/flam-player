-- test_story_f27.lua — F27 (ctrl autoplay/ok/home, index, -1 aleatoire) du
-- moteur telmi2flam. Importe du harnais du lot X3 (harness_f27.lua, sha1 7e86f36195b3)
-- avec chemins relatifs et compteurs TEST_PASS/TEST_FAIL.
-- Lancement : flam-test tests\engine\test_story_f27.lua | lua54 <ce fichier>
-- Harnais F27 : moteur telmi2flam (engine/main.lua + engine/story.lua) avec les
-- VRAIS global.lua, progressionManager.lua et audio-player_1_0_0.lua.
-- Mocks : lv (no-op generique + timers/slider), audio, firmware (state,
-- progression, back_callback, goto_library, window/document/screen) et modules
-- d'UI de choix (list-choice, carousel, image-choice, title-card).
-- Lancer : lua54.exe harness_f27.lua [repertoire telmi2flam]
local _src = debug.getinfo(1, "S").source
local HERE = _src:sub(1, 1) == "@" and _src:sub(2):match("^(.*)[/\\]") or "."
-- FLAM_TELMI_DIR : autre copie de tools/telmi2flam (ex. version d'origine)
local ROOT = os.getenv("FLAM_TELMI_DIR") or (HERE .. "/../../tools/telmi2flam/")
local STANDALONE = (test_tick == nil)   -- pas lance par flam-test
local ENGINE = ROOT .. "engine/"
local RUNTIME = ROOT .. "runtime/script/"
package.path = ENGINE .. "?.lua;" .. RUNTIME .. "?.lua;" .. package.path

local fails, count = 0, 0
TEST_PASS, TEST_FAIL = 0, 0
local function check(name, cond, info)
  count = count + 1
  if cond then TEST_PASS = TEST_PASS + 1 else TEST_FAIL = TEST_FAIL + 1 end
  if cond then print("[OK]   " .. name)
  else fails = fails + 1; print("[FAIL] " .. name .. (info ~= nil and ("  -> " .. tostring(info)) or "")) end
end

-- ---------------------------------------------------------------------------
-- Mock lv
-- ---------------------------------------------------------------------------
local calls = {}
local timers = {}
local objId = 0
local sliderVal, sliderMax = {}, {}
local special = {
  obj = { new = function() objId = objId + 1; return { id = objId } end,
          get_width = function() return 320 end, get_height = function() return 212 end },
  img = { new = function() objId = objId + 1; return { id = objId, img = true } end },
  label = { new = function() objId = objId + 1; return { id = objId } end },
  slider = {
    new = function() objId = objId + 1; local o = { id = objId }; sliderVal[o] = 0; sliderMax[o] = 100; return o end,
    set_range = function(o, a, b) sliderMax[o] = b end,
    set_value = function(o, v) sliderVal[o] = v end,
    get_value = function(o) return sliderVal[o] or 0 end,
  },
  img_src = { load = function(p) return { src = p } end, get_width = function() return 18 end, get_height = function() return 18 end },
  timer = {
    new = function(cb, period) local t = { cb = cb, period = period }; timers[#timers + 1] = t; return t end,
    del = function(t) t.dead = true end,
  },
  event = { get_key_value = function(e) return e end },
  style = { new = function() objId = objId + 1; return { style = objId } end },
  anim = { new = function() return {} end, set_var = function() return {} end },
}
lv = setmetatable({}, { __index = function(t, mod)
  if type(mod) == "string" and mod:match("^[A-Z_]+$") then return 0 end
  local m = setmetatable({}, { __index = function(mt, fn)
    local sp = special[mod] and special[mod][fn]
    local f = function(...)
      calls[#calls + 1] = { mod = mod, fn = fn, args = { ... } }
      if sp then return sp(...) end
    end
    rawset(mt, fn, f); return f
  end })
  rawset(t, mod, m); return m
end })
local function tick(t) if t and not t.dead then t.cb() end end
local function ncalls(mod, fn, pred)
  local n = 0
  for _, c in ipairs(calls) do
    if c.mod == mod and c.fn == fn and (pred == nil or pred(c.args)) then n = n + 1 end
  end
  return n
end

-- ---------------------------------------------------------------------------
-- Mock firmware / audio
-- ---------------------------------------------------------------------------
window, document = { id = "window" }, { id = "document" }
screen = { set_state = function() end }
local A = { status = "stop", cb = nil, loads = {}, seeks = 0 }
audio = {
  load = function(_, p, cb) A.loads[#A.loads + 1] = p; A.cb = cb; return 0 end,
  play = function() A.status = "play" end, pause = function() A.status = "pause" end,
  stop = function() A.status = "stop" end, get_status = function() return A.status end,
  seek = function() A.seeks = A.seeks + 1 end, duration = function() return 10 end,
}
local store = {}
progression = {
  load = function(k) local t = store[k] or {}; local c = {}; for i, v in pairs(t) do c[i] = v end; return c end,
  save = function(k, v) store[k] = v end,
}
local log = {}
function goto_library() log[#log + 1] = "goto_library" end
back_callback = goto_library

-- Modules d'UI remplaces (package.preload persiste : chaque require apres le
-- package.loaded[...] = nil de cleanCurrentModule redonne un module neuf)
local lastModule
local function mockModule(name)
  return function()
    local m = { answerIterator = 1 }
    m.create = function(args) lastModule = { name = name, args = args, mod = m } end
    m.display = m.create
    m.clean = function() m.answerIterator = 1; m.cleaned = true end
    return m
  end
end
package.preload["list-choice_1_0_0"] = mockModule("list-choice")
package.preload["carousel_1_0_0"] = mockModule("carousel")
package.preload["image-choice_1_0_0"] = mockModule("image-choice")
package.preload["title-card"] = mockModule("title-card")

-- ---------------------------------------------------------------------------
-- Boot / helpers
-- ---------------------------------------------------------------------------
local realRandom = math.random
local randomArgs = {}
local randomRet = nil   -- nil = vrai math.random

local function boot(nodes, keepState)
  for _, k in ipairs({ "nodes", "story", "global", "progressionManager", "audio-player_1_0_0" }) do
    package.loaded[k] = nil
  end
  package.loaded["nodes"] = nodes
  if not keepState then state = {}; store = {} end
  log, lastModule, calls, timers = {}, nil, {}, {}
  A.status, A.cb, A.loads, A.seeks = "stop", nil, {}, 0
  randomArgs, randomRet = {}, nil
  context_menu = nil
  back_callback = goto_library
  dofile(ENGINE .. "main.lua")
  setup()
end
math.random = function(n, ...)
  randomArgs[#randomArgs + 1] = n
  if randomRet ~= nil then return randomRet end
  return realRandom(n, ...)
end

local function AP() return (Global.current_module_name == "audio-player_1_0_0") and Global.current_module or nil end
local function menuLabel() return lastModule and lastModule.args.choices and lastModule.args.choices[1].label end
-- Start (list-choice) : 1ere entree = Demarrer / Reprendre
local function menuPick(i) lastModule.args.choices[i or 1].cb() end
-- lance l'audio demande (timer de global.lua) puis 1er feedback 'play'
local function audioStart()
  tick(Global.audioDelayTimer)
  if A.cb then A.cb("play", 1) end
end
local function audioEnd()
  audioStart()
  A.status = "stop"
  if A.cb then A.cb("stop", 3) end
end
local function key(k) AP().keyPressed(string.char(k)) end
local function tickOk(n) local ap = AP(); for _ = 1, (n or 3) do if AP() == ap then tick(ap.okTimer) end end end
-- OK apres l'anti-rebond d'entree : ticks jusqu'a okTick > 1, ENTER, 1 tick
local function pressOk()
  local ap = AP()
  while ap.okTick <= 1 do tick(ap.okTimer) end
  key(10); tick(ap.okTimer); return ap
end
local function curAudio() local ap = AP(); return ap and Global.audioDelayPath end

-- Constructeurs de noeuds
local AUTO = { ok = true, home = false, autoplay = true }
local MANUAL = { ok = true, home = false, autoplay = false }
local function stage(t) t.ctrl = t.ctrl or AUTO; return t end

-- ---------------------------------------------------------------------------
-- 1. autoplay on : fin d'audio => scene suivante
-- ---------------------------------------------------------------------------
local nLin = { start = { action = "a0", index = 0 }, totalChapters = 2,
  stages = { s0 = stage({ audio = "s0.mp3", image = "s0.lif", ok = { action = "a1", index = 0 } }),
             s1 = stage({ audio = "s1.mp3", image = "s1.lif" }) },
  actions = { a0 = { { stage = "s0" } }, a1 = { { stage = "s1" } } } }
boot(nLin); menuPick()
check("autoplay: scene s0 dans audio-player", AP() and Global.audioDelayPath == "s0.mp3" and state.current_fun == "s0")
check("autoplay: callback de fin pose", AP() and AP().exitCallback ~= nil)
audioEnd()
check("autoplay: fin d'audio -> s1", state.current_fun == "s1" and Global.audioDelayPath == "s1.mp3")
audioEnd()
check("fin d'histoire (ok nil) -> menu Start 'Demarrer'", lastModule.name == "list-choice" and menuLabel() == "Demarrer l'histoire"
      and state.current_fun == nil and #log == 0, menuLabel())
check("fin d'histoire : chapitres remis a zero", #(store.chaps or {}) == 0)

-- ---------------------------------------------------------------------------
-- 2. ctrl.ok pendant l'audio (skip) + anti-rebond + anti double appel
-- ---------------------------------------------------------------------------
boot(nLin); menuPick(); audioStart()
local ap0 = AP()
check("skip: okTimer cree (ctrl.ok)", ap0.okTimer ~= nil and ap0.okTimer.period == 100)
key(10); tick(ap0.okTimer); tick(ap0.okTimer)
check("skip: ENTER ignore pendant l'anti-rebond (2 ticks)", state.current_fun == "s0" and ap0.okRequested == false)
key(10); key(10)
check("skip: ENTER non traite dans l'evenement KEY", state.current_fun == "s0" and AP() == ap0)
tick(ap0.okTimer)
check("skip: ENTER -> s1 via okTimer", state.current_fun == "s1" and Global.audioDelayPath == "s1.mp3")
check("skip: okTimer de s0 supprime au clean", ap0.okTimer == nil)
check("skip: garde exited posee", ap0.exited == true)
local visits = 0
for _, c in ipairs(calls) do if c.mod == "img_src" and c.fn == "load" and c.args[1] == "s1.lif" then visits = visits + 1 end end
check("skip: une seule creation de s1", visits == 1, visits)
-- leave() rappele (stop tardif + ENTER) : aucune seconde transition
local s1ap = AP()
s1ap.leave(s1ap.exitCallback); local after1 = state.current_fun
s1ap.leave(s1ap.exitCallback); s1ap.leave(s1ap.okCallback)
check("anti double appel: leave idempotent", after1 == nil and lastModule.name == "list-choice" and #log == 0)

-- ENTER pendant la phase de lecture ne passe PAS en mini-player / seek
boot(nLin); menuPick(); audioStart()
local nMini = ncalls("img", "set_zoom", function(a) return a[2] == 207 end)
key(10)
check("ENTER ne declenche pas le mini-player", ncalls("img", "set_zoom", function(a) return a[2] == 207 end) == nMini
      and AP().seeking == false)

-- ---------------------------------------------------------------------------
-- 3. ctrl.ok = false : ENTER ignore, l'autoplay avance
-- ---------------------------------------------------------------------------
local nNoOk = { start = { action = "a0" }, totalChapters = 2,
  stages = { s0 = stage({ audio = "s0.mp3", ok = { action = "a1" }, ctrl = { ok = false, home = false, autoplay = true } }),
             s1 = stage({ audio = "s1.mp3" }) },
  actions = { a0 = { { stage = "s0" } }, a1 = { { stage = "s1" } } } }
boot(nNoOk); menuPick(); audioStart()
check("ok=false: pas d'okTimer", AP().okTimer == nil and AP().okCallback == nil)
key(10); key(10)
check("ok=false: ENTER ignore", state.current_fun == "s0")
audioEnd()
check("ok=false: fin d'audio -> s1", state.current_fun == "s1")

-- ---------------------------------------------------------------------------
-- 4. autoplay off + audio : fin d'audio => on reste, OK => suite
-- ---------------------------------------------------------------------------
local nMan = { start = { action = "a0" }, totalChapters = 2,
  stages = { s0 = stage({ audio = "s0.mp3", image = "s0.lif", ok = { action = "a1" }, ctrl = MANUAL }),
             s1 = stage({ audio = "s1.mp3" }) },
  actions = { a0 = { { stage = "s0" } }, a1 = { { stage = "s1" } } } }
boot(nMan); menuPick()
check("non-autoplay: pas de callback de fin", AP().exitCallback == nil and AP().okCallback ~= nil)
local apM = AP()
tickOk(3)   -- anti-rebond ecoule
audioEnd()
check("non-autoplay: fin d'audio -> reste sur s0", AP() == apM and state.current_fun == "s0" and apM.ended == true)
check("non-autoplay: seekposition effacee a la fin", state.visited_funs.s0.seekposition == nil)
-- fleches apres la fin : pas de seek (pause/seek/play relancerait l'audio)
key(19); tick(apM.inactivity_timer)
check("non-autoplay: fleche apres la fin sans seek", A.seeks == 0 and apM.seekValue == nil and A.status == "stop")
key(10); tick(apM.okTimer)
check("non-autoplay: OK apres la fin -> s1", state.current_fun == "s1")

-- non-autoplay + ok=false : bloque sur l'image, retour -> menu Start
local nStuck = { start = { action = "a0" }, totalChapters = 2,
  stages = { s0 = stage({ audio = "s0.mp3", ok = { action = "a1" }, ctrl = { ok = false, home = false, autoplay = false } }),
             s1 = stage({ audio = "s1.mp3" }) },
  actions = { a0 = { { stage = "s0" } }, a1 = { { stage = "s1" } } } }
boot(nStuck); menuPick(); audioEnd(); key(10)
check("non-autoplay ok=false: reste sur s0", state.current_fun == "s0" and AP() ~= nil)
back_callback()
check("non-autoplay ok=false: retour -> menu Reprendre", lastModule.name == "list-choice" and menuLabel() == "Reprendre l'histoire", menuLabel())

-- ---------------------------------------------------------------------------
-- 5. scene image sans audio
-- ---------------------------------------------------------------------------
local nImg = { start = { action = "a0" }, totalChapters = 1,
  stages = { i0 = stage({ image = "i0.lif", ok = { action = "a1" }, ctrl = MANUAL }),
             s1 = stage({ audio = "s1.mp3" }) },
  actions = { a0 = { { stage = "i0" } }, a1 = { { stage = "s1" } } } }
boot(nImg); menuPick()
local apI = AP()
check("image seule non-autoplay: audio-player sans audio", apI ~= nil and Global.audioDelayPath == nil and #A.loads == 0)
check("image seule: image de fond chargee", ncalls("img_src", "load", function(a) return a[1] == "i0.lif" end) == 1)
check("image seule: image rendue visible (showLargePlayer)",
      ncalls("style", "set_img_opa", function(a) return a[1] == apI.styles.background and a[2] == 255 end) == 1)
check("image seule: position sauvee (pas un chapitre)", state.current_fun == "i0" and #(store.chaps or {}) == 0)
pressOk()
check("image seule: OK -> s1", state.current_fun == "s1" and Global.audioDelayPath == "s1.mp3")
-- image seule autoplay => noeud de passage immediat
local nImgAuto = { start = { action = "a0" }, totalChapters = 1,
  stages = { i0 = stage({ image = "i0.lif", ok = { action = "a1" } }), s1 = stage({ audio = "s1.mp3" }) },
  actions = { a0 = { { stage = "i0" } }, a1 = { { stage = "s1" } } } }
boot(nImgAuto); menuPick()
check("image seule autoplay: passage direct -> s1", state.current_fun == "s1")
-- ni image ni audio, non-autoplay => passage (rien a afficher)
local nVoid = { start = { action = "a0" }, totalChapters = 1,
  stages = { v0 = stage({ ok = { action = "a1" }, ctrl = MANUAL }), s1 = stage({ audio = "s1.mp3" }) },
  actions = { a0 = { { stage = "v0" } }, a1 = { { stage = "s1" } } } }
boot(nVoid); menuPick()
check("ni image ni audio non-autoplay: passage -> s1", state.current_fun == "s1")
-- image seule en fin d'histoire (ok nil) : OK -> fin
local nImgEnd = { start = { action = "a0" }, totalChapters = 1,
  stages = { i0 = stage({ image = "i0.lif", ctrl = MANUAL }) }, actions = { a0 = { { stage = "i0" } } } }
boot(nImgEnd); menuPick(); pressOk()
check("image seule finale: OK -> fin d'histoire", AP() == nil and state.current_fun == nil and lastModule.name == "list-choice" and menuLabel() == "Demarrer l'histoire")

-- ---------------------------------------------------------------------------
-- 6. Home (bouton retour)
-- ---------------------------------------------------------------------------
local function nHome(ctrlHome, homeAction)
  return { start = { action = "a0" }, totalChapters = 3,
    stages = { s0 = stage({ audio = "s0.mp3", ok = { action = "a1" }, home = homeAction and { action = homeAction, index = 0 } or nil,
                            ctrl = { ok = true, home = ctrlHome, autoplay = true } }),
               s1 = stage({ audio = "s1.mp3" }), h = stage({ audio = "h.mp3" }) },
    actions = { a0 = { { stage = "s0" } }, a1 = { { stage = "s1" } }, ah = { { stage = "h" } } } }
end
boot(nHome(true, "ah")); menuPick(); audioStart()
back_callback()
check("home: ctrl.home + home -> scene home", state.current_fun == "h" and Global.audioDelayPath == "h.mp3")
check("home: retour depuis la scene home -> menu Start", (function() back_callback(); return lastModule.name == "list-choice" end)())
boot(nHome(false, "ah")); menuPick()
back_callback()
check("home: ctrl.home=false -> menu Start", lastModule.name == "list-choice" and menuLabel() == "Reprendre l'histoire")
boot(nHome(true, nil)); menuPick()
back_callback()
check("home: ctrl.home sans transition -> menu Start", lastModule.name == "list-choice")
boot(nHome(true, "a0")); menuPick()
back_callback()
check("home: boucle sur soi ignoree -> menu Start", lastModule.name == "list-choice")
-- la home ne fuit pas sur la scene suivante (setProgression la remet a Start)
boot(nHome(true, "ah")); menuPick(); audioEnd()
back_callback()
check("home: non heritee par la scene suivante", lastModule.name == "list-choice" and #log == 0)

-- ---------------------------------------------------------------------------
-- 7. Choix (molette), home de l'option focalisee
-- ---------------------------------------------------------------------------
local OPT = { ok = true, home = true, autoplay = false }
local nChoice = { start = { action = "a0" }, totalChapters = 5,
  stages = { s0 = stage({ audio = "s0.mp3", ok = { action = "m", index = 0 } }),
             o1 = stage({ audio = "o1.mp3", ok = { action = "a1" }, home = { action = "ah" }, ctrl = OPT }),
             o2 = stage({ audio = "o2.mp3", ok = { action = "a1" }, home = { action = "ah2" }, ctrl = OPT }),
             s1 = stage({ audio = "s1.mp3" }), h = stage({ audio = "h.mp3" }), h2 = stage({ audio = "h2.mp3" }) },
  actions = { a0 = { { stage = "s0" } }, m = { { stage = "o1" }, { stage = "o2" } }, a1 = { { stage = "s1" } },
              ah = { { stage = "h" } }, ah2 = { { stage = "h2" } } } }
boot(nChoice); menuPick(); audioEnd()
check("choix: carrousel non-autoplay", lastModule.name == "carousel" and #lastModule.args.choices == 2 and state.current_fun == "m")
lastModule.mod.answerIterator = 2
back_callback()
check("choix: retour -> home de l'option focalisee (o2)", state.current_fun == "h2", state.current_fun)
boot(nChoice); menuPick(); audioEnd()
back_callback()
check("choix: retour -> home de l'option 1", state.current_fun == "h")
boot(nChoice); menuPick(); audioEnd()
lastModule.args.choices[2].cb()
check("choix: validation option 2 -> s1", state.current_fun == "s1")
-- choix sans home : back_callback reste celui de setProgression (Start)
local nChoiceNoHome = { start = { action = "a0" }, totalChapters = 3,
  stages = { s0 = stage({ audio = "s0.mp3", ok = { action = "m" } }),
             o1 = stage({ audio = "o1.mp3", ctrl = MANUAL }), o2 = stage({ audio = "o2.mp3", ctrl = MANUAL }) },
  actions = { a0 = { { stage = "s0" } }, m = { { stage = "o1" }, { stage = "o2" } } } }
boot(nChoiceNoHome); menuPick(); audioEnd()
back_callback()
check("choix sans home: retour -> menu Start", lastModule.name == "list-choice" and menuLabel() == "Reprendre l'histoire")

-- ---------------------------------------------------------------------------
-- 8. index -1 (aleatoire) et cibles autoplay
-- ---------------------------------------------------------------------------
local function nRand(ctrlOpt, index, cond)
  return { inventory = { { name = "x", init = 0 } }, start = { action = "a0" }, totalChapters = 5,
    stages = { s0 = stage({ audio = "s0.mp3", ok = { action = "m", index = index } }),
               r1 = stage({ audio = "r1.mp3", ctrl = ctrlOpt }), r2 = stage({ audio = "r2.mp3", ctrl = ctrlOpt }),
               r3 = stage({ audio = "r3.mp3", ctrl = ctrlOpt }), r4 = stage({ audio = "r4.mp3", ctrl = ctrlOpt }) },
    actions = { a0 = { { stage = "s0" } },
                m = { { stage = "r1" }, { stage = "r2", cond = cond }, { stage = "r3" }, { stage = "r4" } } } }
end
local HIDDEN = { { cmp = 3, item = 0, num = 0 } }   -- x > 0 : faux
boot(nRand(AUTO, -1, HIDDEN)); menuPick(); randomRet = 3; audioEnd()
check("random: math.random sur les entrees visibles (3)", randomArgs[1] == 3, randomArgs[1])
check("random: 3e visible (r4, r2 masquee) jouee directement", state.current_fun == "r4" and Global.audioDelayPath == "r4.mp3", state.current_fun)
local seen = {}
for _ = 1, 60 do
  boot(nRand(AUTO, -1, HIDDEN)); menuPick(); audioEnd(); seen[state.current_fun] = true
end
check("random reel: jamais l'entree masquee, plusieurs branches", not seen.r2 and seen.r1 and seen.r3 and seen.r4)
boot(nRand(MANUAL, -1, nil)); menuPick(); randomRet = 2; audioEnd()
check("random non-autoplay: choix avec preselection tiree", lastModule.name == "carousel" and lastModule.args.index == 2
      and lastModule.mod.answerIterator == 2 and randomArgs[1] == 4)
boot(nRand(AUTO, 2, nil)); menuPick(); audioEnd()
check("index 2 cible autoplay -> scene directe r3 (pas de molette)", state.current_fun == "r3" and #randomArgs == 0)
boot(nRand(MANUAL, 1, nil)); menuPick(); audioEnd()
check("index 1 non-autoplay -> choix preselection 2", lastModule.name == "carousel" and lastModule.args.index == 2)
-- start avec index -1
local nStartRand = { start = { action = "a0", index = -1 }, totalChapters = 2,
  stages = { r1 = stage({ audio = "r1.mp3" }), r2 = stage({ audio = "r2.mp3" }) },
  actions = { a0 = { { stage = "r1" }, { stage = "r2" } } } }
boot(nStartRand); randomRet = 2; menuPick()
check("startAction index -1 -> tirage", state.current_fun == "r2")

-- ---------------------------------------------------------------------------
-- 9. Reprise
-- ---------------------------------------------------------------------------
local nRes = { inventory = { { name = "or", init = 0, max = 10 } }, start = { action = "a0" }, totalChapters = 3,
  stages = { s0 = stage({ audio = "s0.mp3", ok = { action = "a1" }, items = { { type = 0, item = 0, number = 2 } }, ctrl = MANUAL }),
             s1 = stage({ audio = "s1.mp3", ok = { action = "m", index = -1 } }),
             r1 = stage({ audio = "r1.mp3" }), r2 = stage({ audio = "r2.mp3" }) },
  actions = { a0 = { { stage = "s0" } }, a1 = { { stage = "s1" } }, m = { { stage = "r1" }, { stage = "r2" } } } }
boot(nRes); menuPick()
check("reprise: items appliques au 1er passage", state.inv[1].value == 2)
boot(nRes, true)
check("reprise: menu 'Reprendre'", menuLabel() == "Reprendre l'histoire")
menuPick()
check("reprise: scene non-autoplay rejouee sans re-appliquer items", state.current_fun == "s0" and state.inv[1].value == 2
      and AP() and AP().exitCallback == nil and AP().okCallback ~= nil)
pressOk()
check("reprise: OK -> s1", state.current_fun == "s1")
-- reprise sur un choix : jamais de raccourci autoplay ni de tirage
state.current_fun = "m"
boot(nRes, true); menuPick()
check("reprise sur choix (options autoplay, index -1) -> choix", lastModule.name == "carousel" and #randomArgs == 0)

-- ---------------------------------------------------------------------------
-- 10. Conditions, inventaire, indexItem
-- ---------------------------------------------------------------------------
local nInv = { inventory = { { name = "cle", init = 0, max = 3 }, { name = "sel", init = 1 } }, start = { action = "a0" }, totalChapters = 4,
  stages = { s0 = stage({ audio = "s0.mp3", ok = { action = "a1" }, items = { { type = 0, item = 0, number = 5 }, { type = 2, item = 1, assignItem = 0 } } }),
             s1 = stage({ audio = "s1.mp3", ok = { action = "m", indexItem = 1 } }),
             c0 = stage({ audio = "c0.mp3" }), c1 = stage({ audio = "c1.mp3" }), c2 = stage({ audio = "c2.mp3" }), c3 = stage({ audio = "c3.mp3" }) },
  actions = { a0 = { { stage = "s0" } },
              a1 = { { stage = "s1", cond = { { cmp = 4, item = 0, num = 3 } } }, { stage = "c0" } },
              m = { { stage = "c0" }, { stage = "c1" }, { stage = "c2" }, { stage = "c3" } } } }
boot(nInv); menuPick()
check("inventaire: += borne au max (3)", state.inv[1].value == 3, state.inv[1].value)
check("inventaire: assignItem", state.inv[2].value == 3)
audioEnd()
check("conditions: cle>=3 -> s1 (seule visible)", state.current_fun == "s1")
audioEnd()
check("indexItem: sel=3 -> c3", state.current_fun == "c3", state.current_fun)
local nCond = { inventory = { { name = "x", init = 0 } }, start = { action = "a0" }, totalChapters = 3,
  stages = { s0 = stage({ audio = "s0.mp3", ok = { action = "m" } }),
             o1 = stage({ audio = "o1.mp3", ctrl = MANUAL }), o2 = stage({ audio = "o2.mp3", ctrl = MANUAL }), o3 = stage({ audio = "o3.mp3", ctrl = MANUAL }) },
  actions = { a0 = { { stage = "s0" } }, m = { { stage = "o1" }, { stage = "o2", cond = HIDDEN }, { stage = "o3" } } } }
boot(nCond); menuPick(); audioEnd()
check("conditions: choix filtre (2 options)", lastModule.name == "carousel" and #lastModule.args.choices == 2)
local nNone = { inventory = { { name = "x", init = 0 } }, start = { action = "a0" }, totalChapters = 1,
  stages = { s0 = stage({ audio = "s0.mp3", ok = { action = "m" } }), o1 = stage({ audio = "o1.mp3" }) },
  actions = { a0 = { { stage = "s0" } }, m = { { stage = "o1", cond = HIDDEN } } } }
boot(nNone); menuPick(); audioEnd()
check("conditions: aucune option -> sortie propre", lastModule.name == "list-choice" and #log == 0)

-- ---------------------------------------------------------------------------
-- 11. nodes.lua ancien (sans ctrl) : comportement historique
-- ---------------------------------------------------------------------------
local nLegacy = { start = { action = "a0" }, totalChapters = 2,
  stages = { s0 = { audio = "s0.mp3", image = "i.lif", ok = { action = "a1" }, home = { action = "a1" } },
             p = { image = "p.lif", ok = { action = "a2" } }, s2 = { audio = "s2.mp3" } },
  actions = { a0 = { { stage = "s0" } }, a1 = { { stage = "p" } }, a2 = { { stage = "s2" } } } }
boot(nLegacy); menuPick()
check("legacy: autoplay + skip, pas de home", AP().exitCallback ~= nil and AP().okCallback ~= nil)
audioEnd()
check("legacy: image sans audio = passage", state.current_fun == "s2")

-- ---------------------------------------------------------------------------
-- 12. Nettoyage audio-player
-- ---------------------------------------------------------------------------
boot(nMan); menuPick()
local apC = AP(); local tmr = apC.okTimer
back_callback()
check("clean: okTimer supprime", tmr.dead == true and apC.okTimer == nil and apC.okCallback == nil)
check("clean: inactivity_timer supprime", apC.inactivity_timer == nil)

print(string.format("\n%d/%d OK", count - fails, count))
-- flam-test appelle setup() s'il existe apres le chargement : on le retire
setup = nil
if STANDALONE then os.exit(fails == 0 and 0 or 1) end
