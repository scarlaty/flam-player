-- test_audio_global.lua — runtime global.lua (telmi2flam) sur le VRAI moteur
-- audio (flam-test-audio) : contrat audio.load partage C / Lua (F09).
--   - fichier absent : audio.load -> -1, global.lua notifie UN 'stop', le
--     'stop' differe du C est ignore, pas de reessai toutes les 500 ms
--   - fichier valide : 'play' puis un seul 'stop' de fin via requestAudioPlay
local H = require("audio_helpers")
package.path = H.root .. "/tools/telmi2flam/runtime/script/?.lua;" .. package.path
require("test_helpers")
print("--- test_audio_global ---")

state = { visited_funs = {}, current_fun = "f1" }
screen = screen or { set_state = function() end }
Global = require("global")
Global.init()

local P = H.make_mp3("flam_audio_global.mp3", 44100, H.frames_for(1.0, 44100), true)

-- compte les audio.load reels
local realLoad, loads = audio.load, 0
audio.load = function(...) loads = loads + 1; return realLoad(...) end

test("F09 fichier absent : un seul 'stop', pas de reessai", function()
    loads = 0
    local got = {}
    Global.requestAudioPlay({ path = "flam_audio_absent_xyz.mp3",
        AFCb = function(s) got[#got + 1] = s end, priority = true })
    H.tick_until(function() return #got > 0 end, 300)   -- timer 500 ms
    expect_eq(loads, 1, "un audio.load")
    expect_eq(#got, 1, "un evenement")
    expect_eq(got[1], "stop", "'stop'")
    test_tick(300)                                        -- ~1,5 s
    expect_eq(loads, 1, "pas de reessai")
    expect_eq(#got, 1, "pas de 2e 'stop' (differe du C ignore)")
    expect_nil(Global.audioDelayPath, "requete videe")
    expect_eq(audio.get_status(), "stop", "status")
end)

test("fichier valide apres echec : 'play' puis un seul 'stop'", function()
    loads = 0
    local got = {}
    Global.requestAudioPlay({ path = P, AFCb = function(s) got[#got + 1] = s end, priority = true })
    local function nstop() local n = 0; for _, s in ipairs(got) do if s == "stop" then n = n + 1 end end; return n end
    expect_true(H.tick_until(function() return nstop() > 0 end, 1200), "'stop' de fin")
    test_tick(20)
    expect_eq(loads, 1, "un audio.load")
    expect_eq(got[1], "play", "1er evenement 'play'")
    expect_eq(nstop(), 1, "un seul 'stop'")
end)

if Global.audioDelayTimer then lv.timer.del(Global.audioDelayTimer); Global.audioDelayTimer = nil end
audio.load = realLoad
H.cleanup()
