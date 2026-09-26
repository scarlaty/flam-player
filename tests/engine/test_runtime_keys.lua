-- test_runtime_keys.lua — modules runtime telmi2flam (carrousel, image-choice)
-- sur le VRAI LVGL de flam-test : touches rapprochees (rapport Maxicours).
-- Touches par lv.event.send(EVENT_KEY), timers LVGL pompes par test_tick,
-- audio = table mock (retours du device), horloge murale test_ms.
-- Lancement : flam-test tests\engine\test_runtime_keys.lua (suite test_run.bat).
--
-- B6 : navigation rapide (< 500 ms entre appuis) : l'audio de l'option quittee
--      est coupe des le changement de focus, seul le chargement du nouvel audio
--      reste differe (un seul load, celui de l'option finale).
-- B7 : OK puis Home/ESC a < 100 ms : l'ENTER en attente (keyEvent differe du
--      timer 100 ms) est traite AVANT le retour, qui s'applique au nouvel ecran.

local src = debug.getinfo(1, "S").source
local HERE = src:sub(1, 1) == "@" and src:sub(2):match("^(.*)[/\\]") or "."
-- FLAM_RUNTIME_DIR : autre copie du runtime (ex. verifier que les tests echouent sur l'original)
local RUNTIME = os.getenv("FLAM_RUNTIME_DIR") or (HERE .. "/../../tools/telmi2flam/runtime/script/")
package.path = HERE .. "/../lua/?.lua;" .. RUNTIME .. "?.lua;" .. package.path

require("test_helpers")
print("--- test_runtime_keys (B6 audio carrousel, B7 OK puis Home) ---")

local KEY_ENTER, KEY_NEXT, KEY_PREV = 10, 19, 20

state = { visited_funs = {}, current_fun = "f1" }
screen = screen or { set_state = function() end }
goto_library = goto_library or function() end

-- Audio mock : journal des load/stop
local A = { status = "stop", loads = {}, stops = 0, cb = nil }
audio = {
    load = function(_, p, cb) A.loads[#A.loads + 1] = p; A.cb = cb; A.path = p; return 0 end,
    play = function() A.status = "play" end, pause = function() A.status = "pause" end,
    stop = function() A.stops = A.stops + 1; A.status = "stop" end,
    get_status = function() return A.status end,
    seek = function() end, duration = function() return 0 end,
}
local function resetAudio()
    A.status, A.loads, A.stops, A.cb, A.path = "stop", {}, 0, nil, nil
end

Global = require("global")
Global.init()
Global.load_image = function() return nil, 18, 18 end   -- pas d'image : repli

local function key(obj, k)
    lv.event.send(obj, lv.EVENT_KEY, k)
end

-- Pompe les timers jusqu'a cond() ou timeout (ms). Renvoie true si cond() vraie.
local function waitFor(cond, timeoutMs)
    local t0 = test_ms()
    while not cond() do
        if test_ms() - t0 > timeoutMs then return false end
        test_tick(1)
    end
    return true
end

-- Pompe les timers pendant ms millisecondes (horloge murale)
local function waitMs(ms)
    local t0 = test_ms()
    while test_ms() - t0 < ms do test_tick(1) end
end

local function choices(n, withCb)
    local t = {}
    for i = 1, n do
        t[i] = { img = "o" .. i .. ".lif", audio = "o" .. i .. ".mp3", label = "O" .. i }
        if withCb then t[i].cb = withCb(i) end
    end
    return t
end

-- Module ouvert, audio de l'option 1 en lecture, fenetre anti-rebond passee
local function openModule(name, n, withCb)
    resetAudio()
    local m = Global.load_module(name, "1_0_0")
    m.create({ choices = choices(n, withCb) })
    expect_true(waitFor(function() return A.status == "play" end, 2000), name .. " : audio option 1 lance")
    expect_eq(A.path, "o1.mp3", name .. " : audio de l'option 1")
    waitMs(250)                                  -- fenetre anti-rebond (tick > 1)
    return m
end

-- ---------------------------------------------------------------------------
-- B6 : navigation rapide
-- ---------------------------------------------------------------------------
for _, name in ipairs({ "carousel", "image-choice" }) do
    test("B6 " .. name .. " : navigation rapide -> audio quitte coupe tout de suite, un seul load", function()
        local m = openModule(name, 6)
        local loads0 = #A.loads
        -- 4 x Droite, ~150 ms entre appuis (< 500 ms du delai audio)
        for i = 1, 4 do
            key(m.parentContainer, KEY_NEXT)
            local tKey = test_ms()
            expect_true(waitFor(function() return m.answerIterator == i + 1 end, 1000),
                "appui " .. i .. " traite")
            local dt = test_ms() - tKey
            -- audio de l'option quittee coupe au traitement de la touche (timer
            -- 100 ms), pas apres le delai de 500 ms
            expect_eq(A.status, "stop", "appui " .. i .. " : audio quitte coupe (" .. dt .. " ms)")
            expect_eq(#A.loads, loads0, "appui " .. i .. " : pas de load pendant la rafale")
            waitMs(150 - math.min(dt, 150))
        end
        -- apres la rafale : un seul load, celui de l'option focalisee (5)
        expect_true(waitFor(function() return #A.loads > loads0 end, 2000), "nouvel audio charge")
        waitMs(1200)                             -- pas de 2e lecture
        expect_eq(#A.loads, loads0 + 1, "un seul load apres la rafale")
        expect_eq(A.path, "o5.mp3", "audio de l'option focalisee")
        expect_eq(A.status, "play", "en lecture")
        Global.cleanCurrentModule()
        test_tick(5)
    end)
end

test("B6 carrousel : 'stop' de l'audio coupe (title_audio) ne relance rien", function()
    resetAudio()
    local car = Global.load_module("carousel", "1_0_0")
    car.create({ choices = choices(3), title_audio = "titre.mp3" })
    expect_true(waitFor(function() return A.status == "play" end, 2000), "title_audio lance")
    expect_eq(A.path, "titre.mp3", "title_audio")
    waitMs(250)
    local titleCb = A.cb
    key(car.parentContainer, KEY_NEXT)
    expect_true(waitFor(function() return car.answerIterator == 2 end, 1000), "appui traite")
    expect_eq(A.status, "stop", "title_audio coupe tout de suite")
    titleCb("stop", 1)                           -- 'stop' emis ensuite par le C
    waitMs(1200)
    expect_eq(#A.loads, 2, "title_audio puis option 2 seulement")
    expect_eq(A.path, "o2.mp3", "audio de l'option 2")
    Global.cleanCurrentModule()
    test_tick(5)
end)

-- ---------------------------------------------------------------------------
-- B7 : OK puis Home avant le passage du timer 100 ms
-- ---------------------------------------------------------------------------
for _, name in ipairs({ "carousel", "image-choice" }) do
    test("B7 " .. name .. " : OK puis Home immediat -> OK traite d'abord, Home sur le nouvel ecran", function()
        local log = {}
        Global.setBackBehavior(function() log[#log + 1] = "back-choix" end)
        local m = openModule(name, 3, function(i)
            return function()
                log[#log + 1] = "ok" .. i
                -- nouvel ecran (scene) : il pose son propre retour
                Global.cleanCurrentModule()
                Global.setBackBehavior(function() log[#log + 1] = "back-scene" end)
            end
        end)
        key(m.parentContainer, KEY_ENTER)
        back_callback()                          -- Home/ESC < 100 ms apres OK
        expect_eq(table.concat(log, ","), "ok1,back-scene", "ordre OK puis Home respecte")
        test_tick(30)
        expect_eq(table.concat(log, ","), "ok1,back-scene", "ENTER pas rejoue")
        Global.cleanCurrentModule()
        test_tick(5)
    end)

    test("B7 " .. name .. " : OK dont l'ecran garde le meme retour -> OK puis ce retour", function()
        local log = {}
        Global.setBackBehavior(function() log[#log + 1] = "back" end)
        local m = openModule(name, 3, function(i)
            return function() log[#log + 1] = "ok" .. i end
        end)
        key(m.parentContainer, KEY_ENTER)
        back_callback()
        expect_eq(table.concat(log, ","), "ok1,back", "OK puis retour (une fois)")
        test_tick(5)
    end)

    test("B7 " .. name .. " : Droite en attente puis Home -> retour seul (touche ignoree)", function()
        local log = {}
        Global.setBackBehavior(function() log[#log + 1] = "back" end)
        local m = openModule(name, 3, function(i)
            return function() log[#log + 1] = "ok" .. i end
        end)
        key(m.parentContainer, KEY_NEXT)
        back_callback()
        expect_eq(table.concat(log, ","), "back", "retour seul")
        test_tick(5)
    end)

    test("B7 " .. name .. " : ENTER dans la fenetre anti-rebond puis Home -> retour seul", function()
        local log = {}
        Global.setBackBehavior(function() log[#log + 1] = "back" end)
        resetAudio()
        local m = Global.load_module(name, "1_0_0")
        m.create({ choices = choices(3, function(i)
            return function() log[#log + 1] = "ok" .. i end
        end) })
        key(m.parentContainer, KEY_ENTER)        -- clic qui a ouvert le choix
        back_callback()
        expect_eq(table.concat(log, ","), "back", "ENTER de la fenetre ignore")
        test_tick(5)
    end)
end

test("B7 temoin : Home sans touche en attente -> retour normal", function()
    local log = {}
    Global.setBackBehavior(function() log[#log + 1] = "back" end)
    local m = openModule("carousel", 3, function(i)
        return function() log[#log + 1] = "ok" .. i end
    end)
    back_callback()
    expect_eq(table.concat(log, ","), "back", "retour seul")
    expect_nil(Global.current_module, "module nettoye")
    test_tick(5)
end)

if Global.audioDelayTimer then lv.timer.del(Global.audioDelayTimer); Global.audioDelayTimer = nil end
print(string.format("Results: %d passed, %d failed", TEST_PASS, TEST_FAIL))
