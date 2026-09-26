-- regress_hunt1_del_in_delete.lua — Non-regression hunt1 (bindings)
-- lv.obj.del(target) dans un handler LV_EVENT_DELETE : LVGL envoie DELETE
-- avant de poser being_deleted, lv_obj_del relancait DELETE sans fin jusqu'au
-- debordement de la pile C (0xC00000FD, processus tue).
-- Comportement attendu : del/clean d'un objet deja en cours de suppression
-- sont des no-op ; handler appele une fois.
-- ATTENDU : ECHOUE (crash) AVANT LE CORRECTIF.
-- Lance seul (processus separe) par test_run.bat.

require("test_helpers")
print("--- regress_hunt1_del_in_delete ---")

test("del(target) dans son handler DELETE", function()
    local o = lv.obj.new(window)
    lv.obj.new(o)
    local n = 0
    lv.obj.add_event_cb(o, function(e)
        n = n + 1
        lv.obj.del(lv.event.get_target(e))
        lv.obj.clean(lv.event.get_target(e))
    end, lv.EVENT_DELETE)
    lv.obj.del(o)
    expect_eq(n, 1, "handler DELETE")
    expect_error(function() lv.obj.get_width(o) end, "objet encore valide")
    for i = 1, 50 do lv.label.new(window) end
    test_tick(5)
end)

test("del du parent depuis le handler DELETE d'un enfant", function()
    local p = lv.obj.new(window)
    local c = lv.obj.new(p)
    local n = 0
    lv.obj.add_event_cb(c, function(e)
        n = n + 1
        lv.obj.del(p)
        lv.obj.del(c)
    end, lv.EVENT_DELETE)
    lv.obj.del(p)
    expect_eq(n, 1, "handler DELETE enfant")
    expect_error(function() lv.obj.get_width(p) end, "parent encore valide")
    for i = 1, 50 do lv.label.new(window) end
    test_tick(5)
end)

test("del d'un autre objet depuis un handler DELETE reste possible", function()
    local o = lv.obj.new(window)
    local other = lv.obj.new(window)
    lv.obj.add_event_cb(o, function(e) lv.obj.del(other) end, lv.EVENT_DELETE)
    lv.obj.del(o)
    expect_error(function() lv.obj.get_width(other) end, "autre objet non supprime")
    test_tick(5)
end)
