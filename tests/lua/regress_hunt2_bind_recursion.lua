-- regress_hunt2_bind_recursion.lua — Non-regression hunt2 (bindings)
-- Recursion Lua -> LVGL -> Lua sans borne : un handler qui relance son
-- propre evenement (lv.event.send dans VALUE_CHANGED, lv.group.focus_obj
-- dans DEFOCUSED) debordait la pile C (0xC00000FD) vers 150 niveaux, avant
-- la limite de Lua (200).
-- Comportement attendu : au-dela de 32 callbacks imbriques, le callback Lua
-- est ignore (log "[EVENT] recursion trop profonde") ; les callbacks
-- fonctionnent ensuite normalement.
-- ATTENDU : ECHOUE (crash) AVANT LE CORRECTIF.
-- Lance seul (processus separe) par test_run.bat.

require("test_helpers")
print("--- regress_hunt2_bind_recursion ---")

test("lv.event.send recursif dans VALUE_CHANGED", function()
    local o = lv.obj.new(window)
    local depth, max = 0, 0
    lv.obj.add_event_cb(o, function(e)
        depth = depth + 1
        if depth > max then max = depth end
        lv.event.send(o, lv.EVENT_VALUE_CHANGED)
        depth = depth - 1
    end, lv.EVENT_VALUE_CHANGED)
    lv.event.send(o, lv.EVENT_VALUE_CHANGED)
    expect_eq(depth, 0, "profondeur au retour")
    expect_true(max >= 8 and max <= 64, "profondeur max " .. max)
    lv.obj.del(o)
end)

test("focus_obj recursif dans DEFOCUSED", function()
    local a, b, c = lv.btn.new(window), lv.btn.new(window), lv.btn.new(window)
    for _, o in ipairs({ a, b, c }) do lv.group.add_obj(document, o) end
    lv.group.focus_obj(a)
    local n = 0
    lv.obj.add_event_cb(a, function(e)
        n = n + 1
        lv.group.focus_obj(c)
    end, lv.EVENT_DEFOCUSED)
    lv.group.focus_obj(b)
    expect_true(n >= 8 and n <= 64, "appels DEFOCUSED " .. n)
    test_tick(2)
    lv.obj.del(a); lv.obj.del(b); lv.obj.del(c)
    test_tick(2)
end)

test("callbacks normaux apres une recursion bornee", function()
    local o = lv.obj.new(window)
    local n = 0
    lv.obj.add_event_cb(o, function(e) n = n + 1 end, lv.EVENT_CLICKED)
    for i = 1, 3 do lv.event.send(o, lv.EVENT_CLICKED) end
    expect_eq(n, 3, "callbacks CLICKED")
    local fired = 0
    local t = lv.timer.new(function() fired = fired + 1 end, 1, 1)
    test_tick(5)
    expect_eq(fired, 1, "timer")
    lv.obj.del(o)
end)
