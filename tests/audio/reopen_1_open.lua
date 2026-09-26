-- reopen_1_open.lua — issue #1 / F07, 1re partie (flam-test-audio).
-- Laisse une piste chargee avec callback ET un 'stop' differe en attente,
-- puis le runner ferme le lua_State (sdl_audio_stop_all + lua_close, comme le
-- retour bibliotheque de main.c). reopen_2_reuse.lua, lance JUSTE APRES dans
-- le meme processus, verifie que rien de cet etat ne touche le nouveau.
local H = require("audio_helpers")
require("test_helpers")
print("--- reopen_1_open ---")

local P = H.make_mp3("flam_audio_reopen.mp3", 44100, H.frames_for(3.0, 44100), true)

test("etat laisse ouvert : callback enregistre + 'stop' differe en attente", function()
    local calls = 0
    local cb = function() calls = calls + 1 end
    expect_eq(audio.load(0, P, cb), 0, "load")
    audio.play()
    test_tick(20)
    expect_ge(calls, 1, "'play' recu")
    -- le callback est bien dans le registre de CET etat
    local found
    for k, v in pairs(debug.getregistry()) do
        if v == cb and math.type(k) == "integer" then found = k end
    end
    expect_true(found ~= nil, "ref du callback dans le registre")
    print("  (ref du callback dans ce registre : " .. tostring(found) .. ")")
    audio.stop()    -- 'stop' differe NON pompe avant la fermeture
    expect_eq(audio.get_status(), "stop", "status")
    -- relance une lecture pour fermer l'etat en cours de lecture
    expect_eq(audio.load(0, P, cb), 0, "reload")
    audio.play()
    test_tick(5)
    audio.stop()
end)
-- Pas de cleanup : le fichier sert encore a reopen_2 (qui le supprime)
