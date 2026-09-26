-- reopen_2_reuse.lua — issue #1 / F07, 2e partie (flam-test-audio), a lancer
-- JUSTE APRES reopen_1_open.lua dans le meme processus.
-- Avant correctif : callback_ref et pending_stop_cb survivaient a lua_close ;
-- le pump appelait registry[ancienne ref] du NOUVEL etat, et audio.load
-- faisait luaL_unref(ancienne ref) => un slot du nouveau registre ecrase
-- (mauvais callback, histoire figee a la relance).
-- Detection : chaque slot libre du registre recoit une sentinelle ; aucune ne
-- doit etre appelee ni remplacee.
local H = require("audio_helpers")
require("test_helpers")
print("--- reopen_2_reuse ---")

local P = "flam_audio_reopen.mp3"
local reg = debug.getregistry()
local hits = 0
local sentinel = function() hits = hits + 1 end
local filled = {}
-- slots 1..3 : thread principal, globals, freelist de luaL_ref (a ne pas toucher)
for k = 4, 1024 do
    if reg[k] == nil then reg[k] = sentinel; filled[#filled + 1] = k end
end

local function sentinels_intact()
    for _, k in ipairs(filled) do
        if reg[k] ~= sentinel then return false, k end
    end
    return true
end

test("nouvel etat : audio a l'arret, aucune trace de l'ancien", function()
    expect_eq(audio.get_status(), "stop", "status")
    expect_eq(audio.duration(), 0, "duration remise a zero")
end)

test("pump sans load : ni 'stop' differe ni callback de l'ancien etat", function()
    test_tick(20)
    expect_eq(hits, 0, "sentinelle appelee (ancienne ref utilisee)")
    local ok, k = sentinels_intact()
    expect_true(ok, "slot " .. tostring(k) .. " du registre modifie")
end)

test("audio.load dans le nouvel etat : pas de luaL_unref de l'ancienne ref", function()
    local rec = H.recorder()
    expect_eq(audio.load(0, P, rec.cb), 0, "load")
    local ok, k = sentinels_intact()
    expect_true(ok, "slot " .. tostring(k) .. " du registre ecrase par luaL_unref")
    audio.play()
    expect_true(H.tick_until(function() return rec.count("play") > 0 end, 100), "'play' vers le NOUVEAU callback")
    audio.stop()
    test_tick(3)
    expect_eq(rec.count("stop"), 1, "un seul 'stop' vers le nouveau callback")
    expect_eq(hits, 0, "aucune sentinelle appelee")
    ok, k = sentinels_intact()
    expect_true(ok, "slot " .. tostring(k) .. " modifie")
end)

test("audio.load en echec dans le nouvel etat : -1 et un seul 'stop'", function()
    local rec = H.recorder()
    expect_eq(audio.load(0, "flam_audio_absent_xyz.mp3", rec.cb), -1, "retour")
    test_tick(10)
    expect_eq(rec.count("stop"), 1, "un 'stop'")
    expect_eq(hits, 0, "aucune sentinelle appelee")
end)

for _, k in ipairs(filled) do if reg[k] == sentinel then reg[k] = nil end end
os.remove(P); os.remove(P .. "map")
