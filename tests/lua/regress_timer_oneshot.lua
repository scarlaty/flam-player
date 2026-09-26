-- regress_timer_oneshot.lua — Non-regression F04 (lot L3)
-- Un timer one-shot est supprime par LVGL apres son declenchement ; un
-- lv.timer.del ulterieur refaisait lv_timer_del sur un bloc libere :
-- liste des timers corrompue et lv_timer_handler bloque (hang).
-- Comportement sur attendu : del idempotent (no-op) ou erreur Lua propre,
-- et le systeme de timers reste fonctionnel.
-- ATTENDU : ECHOUE (TIMEOUT du runner) AVANT LE CORRECTIF L3.
-- Lance seul (processus separe) par test_run.bat.

require("test_helpers")
print("--- regress_timer_oneshot ---")

test("del d'un one-shot deja auto-supprime", function()
    local fired = 0
    local t = lv.timer.new(function() fired = fired + 1 end, 10)
    lv.timer.set_repeat_count(t, 1)
    test_tick(20)
    expect_eq(fired, 1, "one-shot declenche une seule fois")

    -- Reallouer le bloc libere par l'auto-suppression
    local fillers = {}
    for i = 1, 50 do fillers[i] = lv.timer.new(function() end, 100000) end

    pcall(lv.timer.del, t)
    test_tick(5)
    pcall(lv.timer.del, t)
    test_tick(5)

    -- Les timers doivent encore fonctionner
    local n = 0
    local t2 = lv.timer.new(function() n = n + 1 end, 10)
    lv.timer.set_repeat_count(t2, 1)
    test_tick(20)
    expect_eq(n, 1, "nouveau one-shot apres double del")

    for i = 1, #fillers do lv.timer.del(fillers[i]) end
end)

test("reset / set_repeat_count sur un one-shot termine", function()
    local t = lv.timer.new(function() end, 10)
    lv.timer.set_repeat_count(t, 1)
    test_tick(20)
    pcall(lv.timer.reset, t)
    pcall(lv.timer.set_repeat_count, t, 5)
    test_tick(10)
    expect_true(true)
end)
