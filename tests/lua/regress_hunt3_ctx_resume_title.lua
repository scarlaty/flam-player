-- regress_hunt3_ctx_resume_title.lua — Non-regression hunt3 (moteur telmi2flam)
-- Menu contextuel "Reprendre l'histoire" (LateralResume) pendant la title-card :
-- titleCard.clean() ne stoppe pas l'audio, image-choice demande l'audio de
-- l'option SANS priorite -> global.audioDelayerCallback attendait la fin de
-- title.mp3 (qui jouait par-dessus le choix).
-- ATTENDU : ECHOUE AVANT LE CORRECTIF (LateralResume coupe l'audio en tete).
-- Harnais repris de regress_hunt2_engine.lua (mocks lv/firmware, VRAIS
-- global.lua, progressionManager.lua, title-card.lua, image-choice_1_0_0.lua,
-- story.lua, main.lua).
-- Lance seul (processus separe) par test_run.bat | lua54 <ce fichier>
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

-- ---------------------------------------------------------------------------
-- Boot / helpers
-- ---------------------------------------------------------------------------
local realRandom = math.random
local randomArgs = {}
local randomRet = nil   -- nil = vrai math.random

local function boot(nodes, keepState)
  for _, k in ipairs({ "nodes", "story", "global", "progressionManager", "audio-player_1_0_0",
                      "image-choice_1_0_0", "title-card" }) do
    package.loaded[k] = nil
  end
  package.loaded["nodes"] = nodes
  if not keepState then state = {}; store = {} end
  log, lastModule, calls, timers = {}, nil, {}, {}
  A.status, A.cb, A.loads, A.seeks = "stop", nil, {}, 0
  randomArgs, randomRet = {}, nil
  ctxEntries = nil
  context_menu = { set_entries = function(e) ctxEntries = e end }
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


local AUTO = { ok = true, home = false, autoplay = true }
local MANUAL = { ok = true, home = false, autoplay = false }
local function stage(t) t.ctrl = t.ctrl or AUTO; return t end
local nodes = { start = { action = "a0", index = 0 }, totalChapters = 3, selector = "image",
  title = { image = "title.lif", audio = "title.mp3" }, meta = { title = "T" },
  stages = { o1 = stage({ audio = "o1.mp3", image = "o1.lif", ctrl = MANUAL, ok = { action = "a1", index = 0 } }),
             o2 = stage({ audio = "o2.mp3", image = "o2.lif", ctrl = MANUAL, ok = { action = "a1", index = 0 } }),
             e = stage({ audio = "e.mp3", image = "e.lif" }) },
  actions = { a0 = { { stage = "o1" }, { stage = "o2" } }, a1 = { { stage = "e" } } } }

local function ctxResume()
  local cb
  for _, e in ipairs(ctxEntries or {}) do
    if e.title == "Reprendre l'histoire" then cb = e.cb end
  end
  check("menu contextuel : entree Reprendre", cb ~= nil)
  if cb then cb() end
end
local function lastLoad() return A.loads[#A.loads] end

-- 1. histoire non commencee (sauvegarde vide) : LateralResume -> LoadStartFunction
local ok, err = pcall(function()
  boot(nodes)
  audioStart()
  check("1: title.mp3 en lecture", lastLoad() == "title.mp3" and A.status == "play", lastLoad())
  ctxResume()
  check("1: module image-choice", Global.current_module_name == "image-choice_1_0_0",
        tostring(Global.current_module_name))
  check("1: title.mp3 coupe", A.status == "stop", A.status)
  tick(Global.audioDelayTimer)
  check("1: audio de l'option 1 charge au 1er tick", lastLoad() == "o1.mp3" and A.status == "play",
        lastLoad())
end)
check("1: pas d'erreur Lua", ok, err)

-- 2. histoire commencee (choix a0 sauve par le cas 1) : LateralResume ->
--    LoadCurrentFunction -> image-choice repris
ok, err = pcall(function()
  check("2: choix a0 sauve", state.current_fun == "a0" and Global.progression.isStoryStarted(),
        tostring(state.current_fun))
  boot(nodes, true)
  audioStart()
  check("2: title.mp3 en lecture", lastLoad() == "title.mp3" and A.status == "play", lastLoad())
  ctxResume()
  check("2: module image-choice", Global.current_module_name == "image-choice_1_0_0",
        tostring(Global.current_module_name))
  check("2: title.mp3 coupe a la reprise", A.status == "stop", A.status)
  tick(Global.audioDelayTimer)
  check("2: audio de l'option 1 charge au 1er tick", lastLoad() == "o1.mp3" and A.status == "play",
        lastLoad())
  -- choix d'une option : l'histoire continue normalement (scene e)
  Global.current_module.answers[1].cb()
  audioStart()
  check("2: option choisie -> scene e", state.current_fun == "e" and lastLoad() == "e.mp3",
        tostring(state.current_fun))
end)
check("2: pas d'erreur Lua", ok, err)

print(string.format("\n%d/%d OK", count - fails, count))
-- flam-test appelle setup() s'il existe apres le chargement : on le retire
setup = nil
if STANDALONE then os.exit(fails == 0 and 0 or 1) end
