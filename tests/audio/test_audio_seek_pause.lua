-- test_audio_seek_pause.lua — seek mp3map exact et callback 'pause' (vrai
-- moteur audio, flam-test-audio, pilote SDL "dummy").
-- B3 : seek(k) atteint la trame la plus proche de k (et non l'entree de la
--      table qui precede), seek(0) / cible avant entries[0] repart du debut
--      reel des donnees (id3_offset), horloge rapportee au Lua recalee sur
--      la position retenue ('stop' = duree du fichier).
-- B8 : 'pause' emis au passage en pause puis chaque seconde, 'play' a la
--      reprise, touche P (sdl_audio_toggle_pause, test_audio_toggle_pause).
local H = require("audio_helpers")
require("test_helpers")
print("--- test_audio_seek_pause ---")

local SR = 44100
local FB = 417                               -- octets / trame (128 kbit/s)
local FRAME_S = H.SAMPLES_PER_FRAME / SR     -- 26,1 ms
local UPF = H.SAMPLES_PER_FRAME * H.UNITS_PER_S // SR
local N = 77                                 -- ~2,01 s
local DUR = H.duration(N, SR)
local ID3 = 400                              -- taille du tag ID3 en tete

-- MP3 facon paquet Lunii/Telmi : tag ID3 en tete, table .mp3map avec une
-- entree toutes les 38 trames (~1 s) et SANS entree pour t=0.
local LUNII = "flam_audio_lunii.mp3"
do
    local sz = ID3 - 10                      -- taille syncsafe du corps
    local hdr = "ID3" .. string.char(4, 0, 0,
        (sz >> 21) & 0x7F, (sz >> 14) & 0x7F, (sz >> 7) & 0x7F, sz & 0x7F)
    local frame = string.char(0xFF, 0xFB, 0x90, 0x00) .. string.rep("\0", FB - 4)
    local f = assert(io.open(LUNII, "wb"))
    f:write(hdr .. string.rep("\0", sz) .. string.rep(frame, N))
    f:close()
    local parts = { string.pack("<I4I4I4", N * UPF, ID3, 0) }
    for i = 38, N - 1, 38 do
        parts[#parts + 1] = string.pack("<I4I4", ID3 + i * FB, i * UPF)
    end
    local m = assert(io.open(LUNII .. "map", "wb"))
    m:write(table.concat(parts))
    m:close()
end

local STD = H.make_mp3("flam_audio_sp.mp3", SR, H.frames_for(2.0, SR), true)
local STD_DUR = H.duration(H.frames_for(2.0, SR), SR)

-- Position retenue par seek(x) (en pause) : le 'stop' differe porte le
-- temps courant, soit la position de depart de la lecture.
local function seek_position(path, x)
    local rec = H.recorder()
    expect_eq(audio.load(0, path, rec.cb), 0, "load")
    audio.play()
    test_tick(10)
    audio.pause()
    audio.seek(x)
    audio.stop()
    test_tick(3)
    expect_eq(rec.count("stop"), 1, "stop differe")
    return rec.last("stop").t
end

-- seek(x) puis lecture jusqu'a la fin : retourne t('stop'). Si l'octet de
-- depart et l'horloge concordent, t('stop') = duree du fichier.
local function seek_play_to_end(path, x, dur)
    local rec = H.recorder()
    expect_eq(audio.load(0, path, rec.cb), 0, "load")
    audio.play()
    test_tick(10)
    audio.pause()
    audio.seek(x)
    audio.play()
    local ok = H.tick_until(function() return rec.count("stop") > 0 end,
        math.floor((dur - x) * 3 / 0.005) + 400)
    expect_true(ok, "stop apres seek(" .. x .. ")")
    test_tick(5)
    expect_eq(rec.count("stop"), 1, "un seul stop")
    return rec.last("stop").t
end

-- ---------------------------------------------------------------- B3 ----

test("seek(0) avant entries[0] : debut reel des donnees (id3_offset) (B3)", function()
    local t0 = seek_position(LUNII, 0)
    expect_true(H.near(t0, 0, 0.001), "position apres seek(0) : " .. tostring(t0))
    -- avant correctif : lecture depuis entries[0] (~0,99 s) avec une
    -- horloge a 0 => 'stop' a ~1,02 s au lieu de ~2,01 s
    local t = seek_play_to_end(LUNII, 0, DUR)
    expect_true(H.near(t, DUR, FRAME_S), "t('stop') " .. tostring(t) .. " ~ " .. DUR)
end)

for _, x in ipairs({ 0.5, 1.0, 1.5 }) do
    test("seek(" .. x .. ") : trame la plus proche, horloge recalee (B3)", function()
        local p = seek_position(LUNII, x)
        expect_true(H.near(p, x, FRAME_S / 2 + 0.001),
            "position apres seek(" .. x .. ") : " .. tostring(p))
        -- frontiere de trame exacte
        local k = p / FRAME_S
        expect_true(math.abs(k - math.floor(k + 0.5)) < 0.01, "frontiere de trame : " .. k)
        local t = seek_play_to_end(LUNII, x, DUR)
        expect_true(H.near(t, DUR, FRAME_S), "t('stop') " .. tostring(t) .. " ~ " .. DUR)
    end)
end

test("seek(1.0) : la trame la plus proche (0,993 s), pas la suivante", function()
    local p = seek_position(LUNII, 1.0)
    expect_true(H.near(p, 38 * FRAME_S, 0.001), "position " .. tostring(p))
end)

-- ---------------------------------------------------------------- B8 ----

test("audio.pause() : 'pause' emis au tick suivant, position figee (B8)", function()
    local rec = H.recorder()
    expect_eq(audio.load(0, STD, rec.cb), 0, "load")
    audio.play()
    test_tick(40)
    audio.pause()
    expect_eq(rec.count("pause"), 0, "pas d'emission pendant audio.pause()")
    test_tick(2)
    expect_eq(rec.count("pause"), 1, "'pause' emis")
    expect_eq(audio.get_status(), "pause", "status")
    local tp = rec.last("pause").t
    expect_true(tp > 0 and tp < STD_DUR, "t('pause') " .. tostring(tp))
    -- ~2,3 s de pause : 'pause' repete chaque seconde, aucun 'play'
    local n_play = rec.count("play")
    local t_start = test_ms()
    while test_ms() - t_start < 2300 do test_tick(4) end
    expect_ge(rec.count("pause"), 3, "'pause' chaque seconde")
    expect_true(rec.count("pause") <= 4, "pas plus d'un 'pause' par seconde : " .. rec.count("pause"))
    expect_eq(rec.count("play"), n_play, "pas de 'play' en pause")
    expect_true(H.near(rec.last("pause").t, tp, 0.001), "position figee en pause")
    -- reprise : 'play' des le tick suivant, puis fin normale
    audio.play()
    test_tick(2)
    expect_eq(rec.events[#rec.events].s, "play", "'play' a la reprise")
    expect_true(H.near(rec.last("play").t, tp, 0.05), "reprise a la position")
    expect_true(H.tick_until(function() return rec.count("stop") > 0 end, 800), "stop")
    test_tick(20)
    expect_eq(rec.count("stop"), 1, "un seul stop")
    local n_pause = rec.count("pause")
    test_tick(250)
    expect_eq(rec.count("pause"), n_pause, "plus de 'pause' apres la fin")
end)

test("pause -> seek -> play dans le meme tick : pas de 'pause' (runtime)", function()
    local rec = H.recorder()
    expect_eq(audio.load(0, STD, rec.cb), 0, "load")
    audio.play()
    test_tick(10)
    audio.pause(); audio.seek(1.0); audio.play()
    test_tick(20)
    expect_eq(rec.count("pause"), 0, "aucun 'pause'")
    audio.stop(); test_tick(3)
end)

test("audio.stop() en pause : 'stop' et plus de 'pause'", function()
    local rec = H.recorder()
    expect_eq(audio.load(0, STD, rec.cb), 0, "load")
    audio.play()
    test_tick(10)
    audio.pause()
    test_tick(2)
    audio.stop()
    test_tick(3)
    expect_eq(rec.count("stop"), 1, "stop")
    local n = rec.count("pause")
    test_tick(250)
    expect_eq(rec.count("pause"), n, "plus de 'pause'")
    expect_eq(audio.get_status(), "stop", "status")
end)

test("touche P (sdl_audio_toggle_pause) : pause puis reprise (B8)", function()
    expect_eq(type(test_audio_toggle_pause), "function", "binding de test")
    local rec = H.recorder()
    expect_eq(audio.load(0, STD, rec.cb), 0, "load")
    audio.play()
    test_tick(20)
    test_audio_toggle_pause()
    expect_eq(audio.get_status(), "pause", "status apres P")
    test_tick(2)
    expect_eq(rec.count("pause"), 1, "'pause' emis")
    test_audio_toggle_pause()
    expect_eq(audio.get_status(), "play", "status apres 2e P")
    test_tick(2)
    expect_eq(rec.events[#rec.events].s, "play", "'play' a la reprise")
    expect_true(H.tick_until(function() return rec.count("stop") > 0 end, 800), "stop")
    test_tick(5)
    -- sans son : sans effet
    local n = #rec.events
    test_audio_toggle_pause()
    expect_eq(audio.get_status(), "stop", "P sans son : reste stop")
    test_tick(250)
    expect_eq(#rec.events, n, "P sans son : aucun evenement")
end)

test("touche P avant tout chargement valide : sans effet", function()
    local rec = H.recorder()
    expect_eq(audio.load(0, "flam_audio_absent_xyz.mp3", rec.cb), -1, "load absent")
    test_tick(3)
    test_audio_toggle_pause()
    expect_eq(audio.get_status(), "stop", "status")
    test_tick(250)
    expect_eq(rec.count("pause"), 0, "aucun 'pause'")
    expect_eq(rec.count("stop"), 1, "seul le 'stop' du load en echec")
end)

os.remove(LUNII)
os.remove(LUNII .. "map")
H.cleanup()
print(string.format("Results: %d passed, %d failed", TEST_PASS, TEST_FAIL))
