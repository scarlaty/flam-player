-- regress_hunt3_del_remove_cb.lua
-- Un handler LV_EVENT_DELETE qui modifie la liste d'evenements de l'objet
-- en cours de suppression (lv.obj.remove_event_cb de son propre callback,
-- lv.anim.set_var sur la cible) decalait la liste parcourue par index :
-- obj_final_cb etait saute, le handle restait valide sur l'objet libere
-- (lecture/ecriture apres liberation, 0xC0000005).
require("test_helpers")

local function expect_dead(o, what)
    local ok = pcall(lv.obj.get_width, o)
    expect_true(not ok, what .. " : le handle d'un objet supprime doit etre invalide")
end

test("DELETE qui retire son propre callback", function()
    local o = lv.obj.new(window)
    local id, n = nil, 0
    id = lv.obj.add_event_cb(o, function(e)
        n = n + 1
        lv.obj.remove_event_cb(o, id)
    end, lv.EVENT_DELETE)
    lv.obj.del(o)
    test_tick(2)
    expect_eq(n, 1, "handler DELETE appele une fois")
    expect_dead(o, "remove_event_cb")
    -- Ecritures via le handle perime : erreur, pas de corruption
    expect_true(not pcall(lv.obj.set_size, o, 10, 10), "set_size refuse")
    lv.obj.remove_event_cb(o, id)   -- no-op
end)

test("DELETE qui retire le callback d'un autre handler DELETE", function()
    local o = lv.obj.new(window)
    local a_calls, b_calls = 0, 0
    local id_b
    lv.obj.add_event_cb(o, function(e)
        a_calls = a_calls + 1
        lv.obj.remove_event_cb(o, id_b)
    end, lv.EVENT_DELETE)
    id_b = lv.obj.add_event_cb(o, function(e) b_calls = b_calls + 1 end, lv.EVENT_DELETE)
    lv.obj.del(o)
    test_tick(2)
    expect_eq(a_calls, 1, "handler A")
    expect_eq(b_calls, 0, "handler B retire avant son tour")
    expect_dead(o, "remove_event_cb d'un autre handler")
end)

test("DELETE qui retire un callback CLICKED puis suite normale", function()
    local o = lv.obj.new(window)
    local clicks = 0
    local id_c = lv.obj.add_event_cb(o, function() clicks = clicks + 1 end, lv.EVENT_CLICKED)
    lv.obj.add_event_cb(o, function(e) lv.obj.remove_event_cb(o, id_c) end, lv.EVENT_DELETE)
    lv.event.send(o, lv.EVENT_CLICKED)
    expect_eq(clicks, 1, "clic avant suppression")
    lv.obj.del(o)
    test_tick(2)
    expect_dead(o, "remove CLICKED dans DELETE")
end)

test("DELETE qui appelle lv.anim.set_var sur la cible", function()
    local o = lv.obj.new(window)
    local a = lv.anim.new()
    lv.anim.set_var(a, o)
    lv.obj.add_event_cb(o, function(e) lv.anim.set_var(a, o) end, lv.EVENT_DELETE)
    lv.obj.del(o)
    test_tick(2)
    expect_dead(o, "set_var")
end)

test("set_var repete : anim toujours arretee a la suppression", function()
    local o = lv.obj.new(window)
    local a = lv.anim.new()
    local calls = 0
    lv.anim.set_var(a, o)
    lv.anim.set_var(a, o)
    lv.anim.set_values(a, 0, 100)
    lv.anim.set_time(a, 1000)
    lv.anim.set_exec_cb(a, function(v, x) calls = calls + 1 end)
    lv.anim.start(a)
    test_tick(3)
    lv.obj.del(o)
    local before = calls
    test_tick(5)
    expect_eq(calls, before, "exec_cb apres suppression de la var")
    expect_dead(o, "set_var repete")
end)

test("suppression d'un ancetre apres DELETE nettoye", function()
    -- g_del_stack ne doit plus garder l'objet : la suppression du parent
    -- est immediate (pas differee)
    local p = lv.obj.new(window)
    local c = lv.obj.new(p)
    local id
    id = lv.obj.add_event_cb(c, function() lv.obj.remove_event_cb(c, id) end, lv.EVENT_DELETE)
    lv.obj.del(c)
    lv.obj.del(p)
    expect_dead(p, "parent supprime immediatement")
end)
