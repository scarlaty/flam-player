-- regress_event_delete.lua — Non-regression lot R0 (lv.event.send + DELETE)
-- Un LV_EVENT_DELETE simule par lv.event.send declenchait obj_final_cb
-- (lua_lv_obj.c) sur un objet encore vivant : userdata invalide, cbd
-- liberes, styles desancres puis collectes -> use-after-free au rendu.
-- Comportement attendu : lv.event.send refuse EVENT_DELETE (erreur Lua),
-- l'objet reste pleinement utilisable, seul lv.obj.del le supprime.
-- Lance seul (processus separe) par test_run.bat.

require("test_helpers")
print("--- regress_event_delete ---")

test("lv.event.send(EVENT_DELETE) leve une erreur Lua", function()
    local o = lv.obj.new(window)
    local ok, err = pcall(lv.event.send, o, lv.EVENT_DELETE)
    expect_eq(ok, false, "send DELETE accepte")
    expect_true(tostring(err):find("DELETE", 1, true) ~= nil,
                "message d'erreur inattendu : " .. tostring(err))
    lv.obj.del(o)
end)

test("objet intact apres un DELETE simule refuse", function()
    local o = lv.obj.new(window)
    lv.obj.set_size(o, 40, 30)

    local s = lv.style.new()
    lv.style.set_bg_color(s, lv.color.hex(0x123456))
    lv.style.set_bg_opa(s, lv.OPA_COVER)
    lv.obj.add_style(o, s, 0)
    s = nil

    local clicks, deletes = 0, 0
    lv.obj.add_event_cb(o, function(e) clicks = clicks + 1 end, lv.EVENT_CLICKED)
    lv.obj.add_event_cb(o, function(e) deletes = deletes + 1 end, lv.EVENT_DELETE)

    pcall(lv.event.send, o, lv.EVENT_DELETE)
    expect_eq(deletes, 0, "handler DELETE Lua appele par un DELETE simule")

    -- Le style ne doit pas avoir ete desancre : GC + rendu
    collectgarbage("collect")
    collectgarbage("collect")
    test_tick(5)

    -- Userdata toujours valide, callbacks toujours en place
    expect_eq(lv.obj.get_width(o), 40, "largeur apres DELETE simule")
    lv.event.send(o, lv.EVENT_CLICKED)
    expect_eq(clicks, 1, "callback CLICKED perdu apres DELETE simule")

    -- La vraie suppression fonctionne et appelle le handler une fois
    lv.obj.del(o)
    expect_eq(deletes, 1, "handler DELETE sur lv.obj.del")
    expect_error(function() lv.obj.get_width(o) end,
                 "acces accepte apres lv.obj.del")
    test_tick(5)
end)

test("DELETE simule refuse depuis un handler DELETE", function()
    local o = lv.obj.new(window)
    local inner_ok
    lv.obj.add_event_cb(o, function(e)
        inner_ok = pcall(lv.event.send, lv.event.get_target(e) or o, lv.EVENT_DELETE)
    end, lv.EVENT_DELETE)
    lv.obj.del(o)
    expect_eq(inner_ok, false, "send DELETE accepte pendant la suppression")
    test_tick(5)
end)
