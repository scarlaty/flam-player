-- regress_hunt2_engine.lua — Non-regression hunt2 (moteur telmi2flam)
--   A. scene quittee par Home (transition home TELMI) puis re-entree par le
--      graphe : l'audio repart du debut (pas de seek sur la position sauvee).
--      La reprise ("Reprendre l'histoire") seek toujours sur la position.
--   B. garde anti-boucle des noeuds sans audio : une boucle d'inventaire
--      bornee de plus de 30 passages atteint la scene de fin ; une vraie
--      boucle (etat repete) et un compteur sans fin sortent proprement.
-- Harnais repris de regress_hunt1_engine.lua (mocks lv/firmware, VRAIS
-- global.lua, progressionManager.lua, audio-player_1_0_0.lua, story.lua, main.lua).
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
-- A. Home puis re-entree par le graphe
-- ---------------------------------------------------------------------------
local nA = { start = { action = "a0", index = 0 }, totalChapters = 3,
  stages = { s0 = stage({ audio = "s0.mp3", image = "s0.lif", ok = { action = "a2", index = 0 },
                          home = { action = "a1", index = 0 }, ctrl = { ok = true, home = true, autoplay = true } }),
             h = stage({ audio = "h.mp3", ok = { action = "a0", index = 0 } }),
             e = stage({ audio = "e.mp3" }) },
  actions = { a0 = { { stage = "s0" } }, a1 = { { stage = "h" } }, a2 = { { stage = "e" } } } }
boot(nA); menuPick()
audioStart()
A.cb("play", 5)
check("A: s0 en cours, position sauvee 5", state.current_fun == "s0" and state.visited_funs.s0
      and state.visited_funs.s0.seekposition == 5, state.visited_funs.s0 and state.visited_funs.s0.seekposition)
back_callback()   -- Home/ESC : transition home -> a1 (h)
check("A: Home -> scene h", state.current_fun == "h" and AP() ~= nil and Global.audioDelayPath == "h.mp3",
      tostring(state.current_fun))
check("A: position de s0 effacee par Home", state.visited_funs.s0.seekposition == nil,
      state.visited_funs.s0.seekposition)
audioEnd()        -- fin de h -> a0 -> s0 (entree normale par le graphe)
check("A: s0 re-entree par le graphe", state.current_fun == "s0" and Global.audioDelayPath == "s0.mp3",
      tostring(state.current_fun))
local seeks = A.seeks
audioStart()
check("A: re-entree -> pas de seek (audio depuis le debut)", A.seeks == seeks, A.seeks - seeks)
-- position residuelle (save d'une version precedente) : ignoree hors reprise
A.cb("play", 7)
state.visited_funs.s0.seekposition = 7
back_callback()
state.visited_funs.s0.seekposition = 7   -- simule une position restee en place
audioEnd()
seeks = A.seeks
audioStart()
check("A: position residuelle ignoree a l'entree par le graphe", A.seeks == seeks and state.current_fun == "s0",
      A.seeks - seeks)
-- reprise : le seek sur la position sauvee est conserve
A.cb("play", 6)
boot(nA, true)
check("A: menu = Reprendre", menuLabel() == "Reprendre l'histoire", menuLabel())
menuPick()
seeks = A.seeks
audioStart()
check("A: reprise -> seek sur la position sauvee", state.current_fun == "s0" and A.seeks == seeks + 1,
      A.seeks - seeks)

-- ---------------------------------------------------------------------------
-- B. boucles de noeuds sans audio
-- ---------------------------------------------------------------------------
local function counterStory(init, op, max)
  return { start = { action = "a0", index = 0 }, totalChapters = 2,
    stages = { s0 = stage({ audio = "s0.mp3", ok = { action = "chk", index = 0 } }),
               dec = stage({ items = { { type = op, item = 0, number = 1 } }, ok = { action = "chk", index = 0 } }),
               fin = stage({ audio = "fin.mp3" }) },
    actions = { a0 = { { stage = "s0" } },
                chk = { { stage = "dec", cond = { { cmp = 3, item = 0, num = 0 } } },
                        { stage = "fin", cond = { { cmp = 2, item = 0, num = 0 } } } } },
    inventory = { { name = "compteur", init = init, max = max or 0, display = 2 } } }
end
for _, n in ipairs({ 10, 31, 200 }) do
  boot(counterStory(n, 1)); menuPick(); audioEnd()
  check("B: compteur " .. n .. " -> scene fin", state.current_fun == "fin" and AP() ~= nil
        and Global.audioDelayPath == "fin.mp3" and state.inv and state.inv[1].value == 0,
        tostring(state.current_fun))
end
-- compteur qui croit sans fin (max 0) : sortie propre (menu Start, progression remise a zero)
boot(counterStory(1, 0)); menuPick()
local ok, err = pcall(audioEnd)
check("B: compteur sans fin -> pas d'erreur Lua", ok, err)
check("B: compteur sans fin -> fin d'histoire propre", state.current_fun == nil and state.inv == nil
      and lastModule and lastModule.name == "list-choice", tostring(state.current_fun))
-- vraie boucle (etat identique) p1 -> p2 -> p1
boot({ start = { action = "a0", index = 0 }, totalChapters = 1,
  stages = { s0 = stage({ audio = "s0.mp3", ok = { action = "b1", index = 0 } }),
             p1 = stage({ ok = { action = "b2", index = 0 } }), p2 = stage({ ok = { action = "b1", index = 0 } }) },
  actions = { a0 = { { stage = "s0" } }, b1 = { { stage = "p1" } }, b2 = { { stage = "p2" } } } })
menuPick()
ok, err = pcall(audioEnd)
check("B: boucle p1/p2 -> fin d'histoire propre", ok and state.current_fun == nil
      and lastModule and lastModule.name == "list-choice", err or tostring(state.current_fun))

print(string.format("\n%d/%d OK", count - fails, count))
-- flam-test appelle setup() s'il existe apres le chargement : on le retire
setup = nil
if STANDALONE then os.exit(fails == 0 and 0 or 1) end
