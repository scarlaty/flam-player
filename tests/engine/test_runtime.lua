-- test_runtime.lua — modules runtime telmi2flam (runtime/script) sur le VRAI
-- LVGL de flam-test, touches envoyees par lv.event.send(EVENT_KEY) et timers
-- LVGL pompes par test_tick. Audio : table mock (retours du device).
-- Lancement : flam-test tests\engine\test_runtime.lua (suite test_run.bat).
--
-- Couvre F11 (ENTER consomme : carrousel et image-choice), F09 (audio.load en
-- echec : nil sur device, -1 sur simulateur -> un seul 'stop', pas de
-- reessai), F06 (sources des fleches du carrousel gardees puis liberees).

local src = debug.getinfo(1, "S").source
local HERE = src:sub(1, 1) == "@" and src:sub(2):match("^(.*)[/\\]") or "."
-- FLAM_RUNTIME_DIR : autre copie du runtime (ex. verifier que les tests echouent sur l'original)
local RUNTIME = os.getenv("FLAM_RUNTIME_DIR") or (HERE .. "/../../tools/telmi2flam/runtime/script/")
package.path = HERE .. "/../lua/?.lua;" .. RUNTIME .. "?.lua;" .. package.path

require("test_helpers")
print("--- test_runtime (modules telmi2flam, LVGL reel) ---")

local KEY_ENTER, KEY_NEXT, KEY_PREV = 10, 19, 20

state = { visited_funs = {}, current_fun = "f1" }
screen = screen or { set_state = function() end }

-- Audio mock : A.loadRet = retour de audio.load (nil = device, -1 = simulateur)
local A = { status = "stop", loadRet = 0, loads = 0, cb = nil, dur = 0 }
audio = {
    load = function(_, p, cb) A.loads = A.loads + 1; A.cb = cb; A.path = p; return A.loadRet end,
    play = function() A.status = "play" end, pause = function() A.status = "pause" end,
    stop = function() A.status = "stop" end, get_status = function() return A.status end,
    seek = function(v) A.lastSeek = v end, duration = function() return A.dur end,
}

Global = require("global")
Global.init()
-- Images : vraies LIF du runtime (runtime/img/script) si elles existent,
-- sinon source nil (set_src(nil) accepte)
local IMGDIR = HERE .. "/../../tools/telmi2flam/runtime/img/script/"
Global.load_image = function(p)
    local path = IMGDIR .. (p:match("[^/\\]+$") or p)
    local f = io.open(path, "rb")
    if not f then return nil, 18, 18 end
    f:close()
    local d = lv.img_src.load(path)
    return d, lv.img_src.get_width(d), lv.img_src.get_height(d)
end

local function key(obj, k)
    lv.event.send(obj, lv.EVENT_KEY, k)
end

-- ---------------------------------------------------------------------------
-- F11 / F06 : carrousel
-- ---------------------------------------------------------------------------
test("F11 carrousel : ENTER -> cb une seule fois (ENTER consomme)", function()
    local car = Global.load_module("carousel", "1_0_0")
    local n1, n2 = 0, 0
    car.create({ choices = {
        { img = "a.lif", audio = "a.mp3", label = "A", cb = function() n1 = n1 + 1 end },
        { img = "b.lif", audio = "b.mp3", label = "B", cb = function() n2 = n2 + 1 end },
        { img = "c.lif", audio = "c.mp3", label = "C" },
    } })
    expect_true(car.parentContainer ~= nil, "conteneur cree")
    test_tick(60)                     -- fenetre anti-rebond (~200 ms)
    key(car.parentContainer, KEY_ENTER)
    test_tick(80)                     -- ~8 passages du timer 100 ms
    expect_eq(n1, 1, "cb option 1 appele une fois")
    expect_nil(car.keyEvent, "keyEvent consomme")
    key(car.parentContainer, KEY_NEXT)
    test_tick(30)
    expect_eq(car.answerIterator, 2, "navigation vers l'option 2")
    key(car.parentContainer, KEY_ENTER)
    test_tick(80)
    expect_eq(n2, 1, "cb option 2 appele une fois")
    expect_eq(n1, 1, "option 1 pas rappelee")
    key(car.parentContainer, KEY_PREV); test_tick(30)
    key(car.parentContainer, KEY_PREV); test_tick(30)
    expect_eq(car.answerIterator, 3, "navigation circulaire (1 -> 3)")
    key(car.parentContainer, KEY_ENTER)   -- option sans cb : sans erreur
    test_tick(30)
    Global.cleanCurrentModule()
    test_tick(5)
end)

test("F06 carrousel : fleches LIF gardees (GC + rendu) puis liberees au clean", function()
    local car = Global.load_module("carousel", "1_0_0")
    car.create({ choices = { { img = "a.lif" }, { img = "b.lif" } } })
    expect_type(car.arrowLData, "userdata", "arrowLData chargee")
    expect_type(car.arrowRData, "userdata", "arrowRData chargee")
    collectgarbage("collect"); collectgarbage("collect")
    test_tick(20)                     -- rendu avec les fleches affichees
    expect_true(car.arrowLData ~= nil and car.arrowRData ~= nil, "fleches toujours referencees")
    Global.cleanCurrentModule()
    expect_nil(car.arrowLData, "arrowLData libere apres clean")
    expect_nil(car.arrowRData, "arrowRData libere apres clean")
    collectgarbage("collect")
    test_tick(10)
end)

-- ---------------------------------------------------------------------------
-- F11 : image-choice
-- ---------------------------------------------------------------------------
test("F11 image-choice : ENTER -> cb une seule fois", function()
    local ic = Global.load_module("image-choice", "1_0_0")
    local n2 = 0
    ic.create({ choices = {
        { img = "a.lif", audio = "a.mp3" },
        { img = "b.lif", audio = "b.mp3", cb = function() n2 = n2 + 1 end },
        { img = "c.lif", audio = "c.mp3" },
    } })
    test_tick(60)
    key(ic.parentContainer, KEY_NEXT)
    test_tick(30)
    expect_eq(ic.answerIterator, 2, "1re touche apres la fenetre traitee (F46)")
    key(ic.parentContainer, KEY_ENTER)
    test_tick(80)
    expect_eq(n2, 1, "cb appele une fois")
    expect_nil(ic.keyEvent, "keyEvent consomme")
    Global.cleanCurrentModule()
    test_tick(5)
end)

-- ---------------------------------------------------------------------------
-- F09 : audio.load en echec
-- ---------------------------------------------------------------------------
for _, ret in ipairs({ "nil", -1 }) do
    test("F09 audio.load en echec (" .. tostring(ret) .. ") : un seul 'stop', pas de reessai", function()
        A.loadRet = (ret ~= "nil") and ret or nil
        A.loads, A.status = 0, "stop"
        local got = {}
        Global.requestAudioPlay({ path = "absent.mp3", AFCb = function(s) got[#got + 1] = s end, priority = true })
        test_tick(140)                -- timer 500 ms de global.lua
        expect_eq(A.loads, 1, "un seul audio.load")
        expect_eq(#got, 1, "callback appele une fois")
        expect_eq(got[1], "stop", "avec 'stop'")
        expect_nil(Global.audioDelayPath, "requete videe")
        -- 'stop' differe emis ensuite par le C (simulateur) : ignore
        if A.cb then A.cb("stop", 0) end
        test_tick(240)                -- ~1,2 s : aucun reessai
        expect_eq(A.loads, 1, "toujours un seul audio.load")
        expect_eq(#got, 1, "pas de double 'stop'")
    end)
end

test("F09 succes apres echec : 'play' puis 'stop' de fin transmis", function()
    A.loadRet = 0; A.loads = 0
    local got = {}
    Global.requestAudioPlay({ path = "ok.mp3", AFCb = function(s) got[#got + 1] = s end, priority = true })
    test_tick(140)
    expect_eq(A.loads, 1, "load")
    expect_eq(A.status, "play", "lecture lancee")
    A.cb("play", 0)
    A.status = "stop"; A.cb("stop", 3)
    expect_eq(#got, 2, "2 evenements")
    expect_eq(got[2], "stop", "stop de fin transmis")
    Global.requestAudioStop(true, true)
end)

if Global.audioDelayTimer then lv.timer.del(Global.audioDelayTimer); Global.audioDelayTimer = nil end
print(string.format("Results: %d passed, %d failed", TEST_PASS, TEST_FAIL))
