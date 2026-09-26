-- test_audio_real.lua — vrai moteur audio (sdl_audio.c + minimp3), pilote
-- SDL "dummy". A lancer avec flam-test-audio (pas flam-test : stub).
-- Couvre : callbacks 'play'/'stop' (A05), duree mp3map, frequence du MP3
-- (F30, 44,1 / 48 / 32 kHz), 'stop' en fin de LECTURE (F40), seek borne
-- (F44), pause + seek (F44), stop differe unique, audio.load en echec
-- (-1 puis un seul 'stop', F09).
local H = require("audio_helpers")
require("test_helpers")
print("--- test_audio_real ---")

local DUR = 2.0
local files = {}
for _, sr in ipairs({ 44100, 48000, 32000 }) do
    local n = H.frames_for(DUR, sr)
    files[sr] = { path = H.make_mp3("flam_audio_" .. sr .. ".mp3", sr, n, true), dur = H.duration(n, sr) }
end
local NOMAP = H.make_mp3("flam_audio_nomap.mp3", 44100, H.frames_for(0.5, 44100), false)
local EMPTY = H.make_empty("flam_audio_empty.mp3")
local f44 = files[44100]

-- Joue un fichier jusqu'au 'stop' (max ~ 3x la duree)
local function play_to_end(path, dur)
    local rec = H.recorder()
    expect_eq(audio.load(0, path, rec.cb), 0, "load " .. path)
    audio.play()
    expect_eq(audio.get_status(), "play", "status apres play")
    local ok = H.tick_until(function() return rec.count("stop") > 0 end, math.floor(dur * 3 / 0.005) + 400)
    expect_true(ok, "pas de 'stop' en fin de fichier " .. path)
    test_tick(20)
    return rec
end

test("etat initial", function()
    expect_eq(audio.get_status(), "stop", "status")
    expect_eq(audio.duration(), 0, "duration")
end)

for _, sr in ipairs({ 44100, 48000, 32000 }) do
    test("lecture complete " .. sr .. " Hz : duree, 'play' puis un seul 'stop' (F30/F40)", function()
        local f = files[sr]
        local rec = play_to_end(f.path, f.dur)
        expect_true(H.near(audio.duration(), f.dur, 0.01),
            "duration() " .. tostring(audio.duration()) .. " ~ " .. f.dur)
        expect_ge(rec.count("play"), 1, "au moins un 'play'")
        expect_eq(rec.events[1].s, "play", "1er evenement")
        expect_true(H.near(rec.events[1].t, 0, 0.1), "1er 'play' a t~0 : " .. tostring(rec.events[1].t))
        expect_eq(rec.count("stop"), 1, "un seul 'stop'")
        expect_eq(rec.events[#rec.events].s, "stop", "'stop' en dernier")
        -- temps au 'stop' = temps JOUE a 44,1 kHz apres conversion : un MP3
        -- 48 kHz non converti donnerait +8,8 %, 32 kHz -27 %
        local t = rec.last("stop").t
        expect_true(H.near(t, f.dur, f.dur * 0.04), "t('stop') " .. tostring(t) .. " ~ " .. f.dur)
        -- 'play' monotones et bornes par la duree
        local prev = -1
        for _, e in ipairs(rec.events) do
            if e.s == "play" then
                expect_true(e.t >= prev and e.t <= f.dur + 0.1, "t('play') " .. e.t)
                prev = e.t
            end
        end
        expect_eq(audio.get_status(), "stop", "status en fin")
    end)
end

test("sans mp3map : duree 0, seek ignore, lecture jusqu'au 'stop'", function()
    local rec = H.recorder()
    expect_eq(audio.load(0, NOMAP, rec.cb), 0, "load")
    expect_eq(audio.duration(), 0, "duration sans mp3map")
    audio.seek(10)
    audio.play()
    expect_true(H.tick_until(function() return rec.count("stop") > 0 end, 600), "stop")
    test_tick(10)
    expect_eq(rec.count("stop"), 1, "un seul stop")
end)

-- Seek puis stop : le 'stop' differe porte le temps courant => position bornee
local function seek_then_stop(value)
    local rec = H.recorder()
    expect_eq(audio.load(0, f44.path, rec.cb), 0, "load")
    audio.play()
    test_tick(10)
    audio.seek(value)
    test_tick(4)
    audio.stop()
    test_tick(4)
    expect_eq(rec.count("stop"), 1, "stop differe unique")
    return rec.last("stop").t
end

test("seek negatif borne a 0 (F44)", function()
    local t = seek_then_stop(-5)
    expect_true(t >= 0 and t < 0.3, "t apres seek(-5) : " .. tostring(t))
end)

test("seek NaN borne a 0 (F44)", function()
    local t = seek_then_stop(0 / 0)
    expect_true(t >= 0 and t < 0.3, "t apres seek(nan) : " .. tostring(t))
end)

test("seek au milieu", function()
    local t = seek_then_stop(1.0)
    expect_true(t >= 0.95 and t < 1.3, "t apres seek(1.0) : " .. tostring(t))
end)

test("seek au-dela de la fin borne a la duree (F44)", function()
    local rec = H.recorder()
    expect_eq(audio.load(0, f44.path, rec.cb), 0, "load")
    audio.play()
    test_tick(5)
    audio.seek(1e9)
    expect_true(H.tick_until(function() return rec.count("stop") > 0 end, 400), "stop rapide apres seek fin")
    local t = rec.last("stop").t
    expect_true(t >= f44.dur - 0.1 and t <= f44.dur + 0.3, "t('stop') " .. tostring(t) .. " ~ " .. f44.dur)
    for _, e in ipairs(rec.events) do
        expect_true(e.t <= f44.dur + 0.3, e.s .. " t=" .. tostring(e.t) .. " hors duree")
    end
    test_tick(10)
    expect_eq(rec.count("stop"), 1, "un seul stop")
end)

test("pause + seek : reste en pause, reprise a la position (F44)", function()
    local rec = H.recorder()
    expect_eq(audio.load(0, f44.path, rec.cb), 0, "load")
    audio.play()
    test_tick(20)
    audio.pause()
    expect_eq(audio.get_status(), "pause", "status pause")
    audio.seek(1.5)
    expect_eq(audio.get_status(), "pause", "seek pendant la pause ne relance pas")
    test_tick(60)
    expect_eq(audio.get_status(), "pause", "toujours en pause")
    expect_eq(rec.count("stop"), 0, "pas de stop pendant la pause")
    audio.play()
    expect_eq(audio.get_status(), "play", "reprise")
    -- il reste ~0,5 s : 'stop' bien avant la duree complete
    local ok = H.tick_until(function() return rec.count("stop") > 0 end, 400)
    expect_true(ok, "stop apres reprise")
    local t = rec.last("stop").t
    expect_true(H.near(t, f44.dur, 0.15), "t('stop') " .. tostring(t))
    test_tick(10)
    expect_eq(rec.count("stop"), 1, "un seul stop")
end)

test("audio.stop() : un seul 'stop', differe au tick (pas dans l'appel)", function()
    local rec = H.recorder()
    expect_eq(audio.load(0, f44.path, rec.cb), 0, "load")
    audio.play()
    test_tick(10)
    audio.stop()
    expect_eq(rec.count("stop"), 0, "stop non emis pendant audio.stop()")
    expect_eq(audio.get_status(), "stop", "status")
    test_tick(3)
    expect_eq(rec.count("stop"), 1, "stop emis au tick")
    audio.stop()   -- deja arrete : pas de nouveau 'stop'
    test_tick(20)
    expect_eq(rec.count("stop"), 1, "un seul stop")
end)

test("load pendant une lecture : l'ancien callback n'est plus appele", function()
    local r1, r2 = H.recorder(), H.recorder()
    expect_eq(audio.load(0, f44.path, r1.cb), 0, "load 1")
    audio.play()
    test_tick(10)
    audio.stop()             -- 'stop' differe en attente pour r1...
    expect_eq(audio.load(0, f44.path, r2.cb), 0, "load 2")   -- ...annule par le load
    local n1 = #r1.events
    audio.play()
    test_tick(30)
    expect_eq(#r1.events, n1, "ancien callback silencieux")
    expect_ge(r2.count("play"), 1, "nouveau callback recoit 'play'")
    audio.stop(); test_tick(3)
end)

test("audio.load fichier absent : -1 puis un seul 'stop' (F09)", function()
    local rec = H.recorder()
    local r = audio.load(0, "flam_audio_absent_xyz.mp3", rec.cb)
    expect_eq(r, -1, "retour de load")
    expect_eq(audio.get_status(), "stop", "status")
    expect_eq(rec.count("stop"), 0, "stop differe (pas dans l'appel)")
    test_tick(3)
    expect_eq(rec.count("stop"), 1, "un 'stop'")
    audio.play()   -- rien a jouer
    expect_eq(audio.get_status(), "stop", "play sans piste")
    test_tick(30)
    expect_eq(rec.count("stop"), 1, "un seul 'stop'")
    expect_eq(#rec.events, 1, "aucun autre evenement")
end)

test("audio.load fichier vide : -1 puis un seul 'stop'", function()
    local rec = H.recorder()
    expect_eq(audio.load(0, EMPTY, rec.cb), -1, "retour")
    test_tick(10)
    expect_eq(rec.count("stop"), 1, "un 'stop'")
end)

test("audio.load absent sans callback : -1 sans erreur", function()
    expect_eq(audio.load(0, "flam_audio_absent_xyz.mp3"), -1, "retour")
    test_tick(5)
    expect_eq(audio.get_status(), "stop", "status")
end)

test("arguments invalides", function()
    expect_error(function() audio.load() end, "load sans argument")
    expect_error(function() audio.seek("x") end, "seek non numerique")
end)

H.cleanup()
print(string.format("Results: %d passed, %d failed", TEST_PASS, TEST_FAIL))
