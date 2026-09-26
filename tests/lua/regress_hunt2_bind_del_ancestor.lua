-- regress_hunt2_bind_del_ancestor.lua — Non-regression hunt2 (bindings)
-- lv.obj.del(p) (ou clean(p)) depuis le handler LV_EVENT_DELETE d'un
-- descendant x de p : lv_obj_del(p) liberait p et x pendant la suppression
-- de x, puis le lv_obj_del(x) exterieur relisait p (use-after-free,
-- 0xC0000005) ; clean(p) relancait DELETE de x sans fin (0xC00000FD).
-- Comportement attendu : suppression de l'ancetre differee au prochain
-- lv_timer_handler ; del d'un objet sans lien reste immediat.
-- ATTENDU : ECHOUE (crash) AVANT LE CORRECTIF.
-- Lance seul (processus separe) par test_run.bat.

require("test_helpers")
print("--- regress_hunt2_bind_del_ancestor ---")

test("del(parent) dans le DELETE d'un enfant supprime seul", function()
    local p = lv.obj.new(window)
    local x = lv.obj.new(p)
    local n = 0
    lv.obj.add_event_cb(x, function(e)
        n = n + 1
        lv.obj.del(p)
        lv.obj.del(x)
    end, lv.EVENT_DELETE)
    lv.obj.del(x)
    expect_eq(n, 1, "handler DELETE")
    expect_error(function() lv.obj.get_width(x) end, "x encore valide")
    expect_eq(lv.obj.get_child_cnt(p), 0, "enfants de p")
    test_tick(3)
    expect_error(function() lv.obj.get_width(p) end, "p non supprime apres tick")
    for i = 1, 50 do lv.label.new(window) end
    test_tick(3)
end)

test("del(grand-parent) dans le DELETE d'un petit-enfant", function()
    local g = lv.obj.new(window)
    local p = lv.obj.new(g)
    local x = lv.obj.new(p)
    lv.obj.add_event_cb(x, function(e) lv.obj.del(g) end, lv.EVENT_DELETE)
    lv.obj.del(p)
    expect_error(function() lv.obj.get_width(p) end, "p encore valide")
    test_tick(3)
    expect_error(function() lv.obj.get_width(g) end, "g non supprime apres tick")
    for i = 1, 50 do lv.label.new(window) end
    test_tick(3)
end)

test("clean(parent) dans le DELETE d'un enfant", function()
    local p = lv.obj.new(window)
    local x = lv.obj.new(p)
    local y = lv.obj.new(p)
    local n = 0
    lv.obj.add_event_cb(x, function(e) n = n + 1; lv.obj.clean(p) end, lv.EVENT_DELETE)
    lv.obj.del(x)
    expect_eq(n, 1, "handler DELETE (recursion ?)")
    test_tick(3)
    expect_error(function() lv.obj.get_width(y) end, "y non supprime apres tick")
    expect_eq(lv.obj.get_child_cnt(p), 0, "enfants de p")
    lv.obj.del(p)
    test_tick(2)
end)

test("clean(parent) dans le DELETE d'un enfant pendant clean(parent)", function()
    local p = lv.obj.new(window)
    local x = lv.obj.new(p)
    lv.obj.new(p)
    local n = 0
    lv.obj.add_event_cb(x, function(e) n = n + 1; lv.obj.clean(p) end, lv.EVENT_DELETE)
    lv.obj.clean(p)
    expect_eq(n, 1, "handler DELETE")
    test_tick(3)
    expect_eq(lv.obj.get_child_cnt(p), 0, "enfants de p")
    lv.obj.del(p)
    test_tick(2)
end)

test("del d'un objet sans lien dans un handler DELETE : immediat", function()
    local o = lv.obj.new(window)
    local other = lv.obj.new(window)
    lv.obj.add_event_cb(o, function(e) lv.obj.del(other) end, lv.EVENT_DELETE)
    lv.obj.del(o)
    expect_error(function() lv.obj.get_width(other) end, "autre objet non supprime")
    test_tick(2)
end)
