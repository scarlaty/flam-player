-- regress_hunt1_engine.lua — Non-regression hunt1 (moteur telmi2flam)
--   A. scene image seule sans audio, ctrl.ok=false, autoplay=false : l'image
--      doit etre rendue visible (ecran noir avant correctif)
--   B. reprise sur un noeud TELMI nomme "clear" (vrai champ story.clear appele,
--      ecran noir) ou "__start" (redemarrage au lieu de reprendre)
--   C. scene et action de meme id : la reprise sur le choix doit reafficher
--      le choix, pas jouer la scene
--   D. reprise sur un choix : la preselection TELMI (index) est conservee
-- Harnais repris de tests/engine/test_story_f27.lua (mocks lv/firmware, VRAIS
-- global.lua, progressionManager.lua, audio-player_1_0_0.lua, story.lua, main.lua).
-- Lance seul (processus separe) par test_run.bat | lua54 <ce fichier>
-- VRAIS global.lua, progressionManager.lua et audio-player_1_0_0.lua.
-- Mocks : lv (no-op generique + timers/slider), audio, firmware (state,
-- progression, back_callback, goto_library, window/document/screen) et modules
-- d'UI de choix (list-choice, carousel, image-choice, title-card).
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
-- ---------------------------------------------------------------------------
-- A. scene image seule, sans audio, ok=false, autoplay=false
-- ---------------------------------------------------------------------------
local function bgShown()
  local ap = AP(); if not ap then return false end
  local bg = ap.styles.background
  return ncalls("style", "set_img_opa", function(a) return a[1] == bg and a[2] == 255 end) > 0
end
for _, withHome in ipairs({ true, false }) do
  local nA = { start = { action = "a0", index = 0 }, totalChapters = 2,
    stages = { s0 = stage({ image = "s0.lif", ok = { action = "a1", index = 0 },
                            home = withHome and { action = "a1", index = 0 } or nil,
                            ctrl = { ok = false, home = withHome, autoplay = false } }),
               s1 = stage({ audio = "s1.mp3", image = "s1.lif" }) },
    actions = { a0 = { { stage = "s0" } }, a1 = { { stage = "s1" } } } }
  boot(nA); menuPick()
  local tag = withHome and "(home)" or "(sans home)"
  check("A" .. tag .. ": scene s0 dans audio-player", AP() ~= nil and state.current_fun == "s0")
  check("A" .. tag .. ": image de fond visible (img_opa 255)", bgShown())
  check("A" .. tag .. ": pas d'okTimer (ctrl.ok=false)", AP() and AP().okTimer == nil)
end

-- ---------------------------------------------------------------------------
-- B. noeuds nommes "clear" et "__start"
-- ---------------------------------------------------------------------------
local nB = { start = { action = "a0", index = 0 }, totalChapters = 3,
  stages = { clear = stage({ audio = "c.mp3", image = "c.lif", ok = { action = "a1", index = 0 } }),
             __start = stage({ audio = "st.mp3", image = "st.lif", ok = { action = "a2", index = 0 } }),
             s2 = stage({ audio = "s2.mp3", image = "s2.lif" }) },
  actions = { a0 = { { stage = "clear" } }, a1 = { { stage = "__start" } }, a2 = { { stage = "s2" } } } }
boot(nB); menuPick()
check("B: 1er passage sur la scene 'clear'", state.current_fun == "clear" and AP() ~= nil)
boot(nB, true)
check("B: menu = Reprendre", menuLabel() == "Reprendre l'histoire", menuLabel())
menuPick()
check("B: reprise sur 'clear' -> audio-player c.mp3", AP() ~= nil and state.current_fun == "clear"
      and Global.audioDelayPath == "c.mp3", tostring(Global.current_module_name))
audioEnd()
check("B: scene '__start' jouee", state.current_fun == "__start" and Global.audioDelayPath == "st.mp3")
boot(nB, true); menuPick()
check("B: reprise sur la scene '__start' (pas de redemarrage)", AP() ~= nil and state.current_fun == "__start"
      and Global.audioDelayPath == "st.mp3", tostring(state.current_fun))
-- save d'une ancienne version (sans current_kind) : reprise par nom, sans planter
state.current_fun, state.current_kind = "clear", nil
boot(nB, true); menuPick()
check("B: save sans current_kind sur 'clear' -> scene clear", AP() ~= nil and Global.audioDelayPath == "c.mp3")
-- noeud inconnu : redemarrage depuis start
state.current_fun = "zz"; state.current_kind = "s"
boot(nB, true); menuPick()
check("B: noeud inconnu -> redemarrage", state.current_fun == "clear" and Global.audioDelayPath == "c.mp3")

-- ---------------------------------------------------------------------------
-- C. scene et action de meme id ("y")
-- ---------------------------------------------------------------------------
local nC = { start = { action = "x", index = 0 }, totalChapters = 2,
  stages = { x = stage({ audio = "x.mp3", image = "x.lif", ok = { action = "y", index = 0 } }),
             y = stage({ audio = "y.mp3", image = "y.lif", ctrl = MANUAL }) },
  actions = { x = { { stage = "x" } }, y = { { stage = "y" }, { stage = "x" } } } }
boot(nC); menuPick(); audioEnd()
check("C: choix 'y' affiche (carrousel)", lastModule and lastModule.name == "carousel" and state.current_fun == "y",
      lastModule and lastModule.name)
boot(nC, true); menuPick()
check("C: reprise sur l'action 'y' -> carrousel (pas la scene y)",
      lastModule and lastModule.name == "carousel" and AP() == nil and state.current_fun == "y",
      tostring(Global.current_module_name))

-- ---------------------------------------------------------------------------
-- D. preselection du choix conservee a la reprise
-- ---------------------------------------------------------------------------
local nD = { start = { action = "a0", index = 0 }, totalChapters = 4,
  stages = { s0 = stage({ audio = "s0.mp3", image = "s0.lif", ok = { action = "a1", index = 1 } }),
             o1 = stage({ audio = "o1.mp3", ctrl = MANUAL }), o2 = stage({ audio = "o2.mp3", ctrl = MANUAL }),
             o3 = stage({ audio = "o3.mp3", ctrl = MANUAL }) },
  actions = { a0 = { { stage = "s0" } }, a1 = { { stage = "o1" }, { stage = "o2" }, { stage = "o3" } } } }
boot(nD); menuPick(); audioEnd()
check("D: 1er affichage, option 2 preselectionnee", lastModule.name == "carousel" and lastModule.mod.answerIterator == 2
      and lastModule.args.index == 2, lastModule.mod.answerIterator)
boot(nD, true); menuPick()
check("D: reprise, option 2 toujours preselectionnee", lastModule.name == "carousel" and lastModule.mod.answerIterator == 2
      and lastModule.args.index == 2, lastModule.mod.answerIterator)
-- index -1 (tirage) : la reprise garde l'option tiree, sans nouveau tirage
nD.stages.s0.ok = { action = "a1", index = -1 }
boot(nD); menuPick(); randomRet = 3; audioEnd()
check("D: tirage -1 -> option 3", lastModule.name == "carousel" and lastModule.mod.answerIterator == 3)
boot(nD, true); menuPick()
check("D: reprise apres tirage -> option 3, sans nouveau tirage", lastModule.name == "carousel"
      and lastModule.mod.answerIterator == 3 and #randomArgs == 0, #randomArgs)
-- fin d'histoire : type et index remis a zero
boot({ start = { action = "a0", index = 0 }, totalChapters = 1,
  stages = { s0 = stage({ audio = "s0.mp3" }) }, actions = { a0 = { { stage = "s0" } } } })
menuPick(); audioEnd()
check("fin: current_kind et choice_index remis a nil", state.current_kind == nil and state.choice_index == nil)

print(string.format("\n%d/%d OK", count - fails, count))
-- flam-test appelle setup() s'il existe apres le chargement : on le retire
setup = nil
if STANDALONE then os.exit(fails == 0 and 0 or 1) end
