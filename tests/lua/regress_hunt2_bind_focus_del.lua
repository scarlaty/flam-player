-- regress_hunt2_bind_focus_del.lua — Non-regression hunt2 (bindings)
-- Un handler LV_EVENT_DEFOCUSED qui supprimait (ou retirait du groupe)
-- l'objet qui allait recevoir le focus : lv_group_focus_obj et
-- focus_next_core ont deja mis de cote le noeud de liste de la cible et
-- font g->obj_focus = noeud libere, puis lv_event_send(*noeud) :
-- use-after-free (0xC0000005).
-- Comportement attendu : pendant DEFOCUSED, lv.obj.del/clean sont differes
-- au prochain lv_timer_handler ; lv.group.remove_obj/remove_all_objs (et
-- add_obj d'un objet deja dans un groupe) levent une erreur Lua.
-- ATTENDU : ECHOUE (crash) AVANT LE CORRECTIF.
-- Lance seul (processus separe) par test_run.bat.

require("test_helpers")
print("--- regress_hunt2_bind_focus_del ---")

local function three_buttons()
    local a, b, c = lv.btn.new(window), lv.btn.new(window), lv.btn.new(window)
    for _, o in ipairs({ a, b, c }) do lv.group.add_obj(document, o) end
    lv.group.focus_obj(a)
    return a, b, c
end

local function focused(o)
    return (lv.obj.get_state(o) & lv.STATE_FOCUSED) ~= 0
end

test("del de la future cible dans DEFOCUSED puis focus_obj", function()
    local a, b, c = three_buttons()
    local n = 0
    lv.obj.add_event_cb(a, function(e)
        n = n + 1
        lv.obj.del(b)
        -- differe : b reste utilisable pendant le handler
        expect_type(lv.obj.get_width(b), "number", "b supprime dans DEFOCUSED")
    end, lv.EVENT_DEFOCUSED)
    lv.group.focus_obj(b)
    expect_eq(n, 1, "handler DEFOCUSED")
    expect_true(focused(b), "b n'a pas recu le focus")
    test_tick(3)
    expect_error(function() lv.obj.get_width(b) end, "b non supprime apres tick")
    for i = 1, 50 do lv.label.new(window) end
    test_tick(3)
    lv.obj.del(a); lv.obj.del(c)
    test_tick(2)
end)

test("clean du parent de la cible dans DEFOCUSED", function()
    local a = lv.btn.new(window)
    local p = lv.obj.new(window)
    local b = lv.btn.new(p)
    lv.group.add_obj(document, a)
    lv.group.add_obj(document, b)
    lv.group.focus_obj(a)
    lv.obj.add_event_cb(a, function(e)
        lv.obj.clean(p)
        lv.btn.new(p)   -- cree apres clean : doit rester
    end, lv.EVENT_DEFOCUSED)
    lv.group.focus_obj(b)
    test_tick(3)
    expect_error(function() lv.obj.get_width(b) end, "b non supprime apres tick")
    expect_eq(lv.obj.get_child_cnt(p), 1, "enfants de p apres clean differe")
    lv.obj.del(p); lv.obj.del(a)
    test_tick(2)
end)

test("del de l'objet qui perd le focus dans son DEFOCUSED", function()
    local a, b, c = three_buttons()
    lv.obj.add_event_cb(a, function(e) lv.obj.del(lv.event.get_target(e)) end,
                        lv.EVENT_DEFOCUSED)
    lv.group.focus_obj(c)
    test_tick(3)
    expect_error(function() lv.obj.get_width(a) end, "a non supprime apres tick")
    lv.obj.del(b); lv.obj.del(c)
    test_tick(2)
end)

test("objet supprime avant la suppression differee (pas de double del)", function()
    local a, b, c = three_buttons()
    lv.obj.add_event_cb(a, function(e) lv.obj.del(b); lv.obj.del(b) end,
                        lv.EVENT_DEFOCUSED)
    lv.group.focus_obj(c)
    lv.obj.del(b)   -- hors DEFOCUSED : immediat, annule la suppression differee
    expect_error(function() lv.obj.get_width(b) end, "b non supprime")
    for i = 1, 50 do lv.label.new(window) end
    test_tick(3)
    lv.obj.del(a); lv.obj.del(c)
    test_tick(2)
end)

test("lv.group.remove_obj / remove_all_objs / add_obj refuses dans DEFOCUSED", function()
    local a, b, c = three_buttons()
    local errs = {}
    lv.obj.add_event_cb(a, function(e)
        errs[#errs + 1] = select(2, pcall(lv.group.remove_obj, b))
        errs[#errs + 1] = select(2, pcall(lv.group.remove_all_objs, document))
        errs[#errs + 1] = select(2, pcall(lv.group.add_obj, document, b))
    end, lv.EVENT_DEFOCUSED)
    lv.group.focus_obj(b)
    expect_eq(#errs, 3, "appels refuses")
    for i, err in ipairs(errs) do
        expect_true(tostring(err):find("DEFOCUSED", 1, true), "erreur " .. i .. " : " .. tostring(err))
    end
    expect_true(focused(b), "b n'a pas recu le focus")
    -- hors DEFOCUSED : toujours permis
    lv.group.remove_obj(c)
    lv.group.add_obj(document, c)
    test_tick(2)
    lv.obj.del(a); lv.obj.del(b); lv.obj.del(c)
    test_tick(2)
end)

test("hors DEFOCUSED : del immediat", function()
    local o = lv.obj.new(window)
    lv.obj.del(o)
    expect_error(function() lv.obj.get_width(o) end, "del non immediat")
end)
