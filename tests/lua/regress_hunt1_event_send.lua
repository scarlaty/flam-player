-- regress_hunt1_event_send.lua — Non-regression hunt1 (bindings)
-- lv.event.send passait param NULL pour EVENT_KEY sans touche (lv_obj_event,
-- lv_slider_event lisent *(char *)param : crash), et acceptait des codes
-- dont le param est une structure LVGL (18 COVER_CHECK, 19
-- REFR_EXT_DRAW_SIZE...) : lecture/ecriture d'un uint32 comme structure.
-- Comportement attendu : EVENT_KEY sans touche = touche 0 ; codes hors
-- lv.EVENT_* exposes refuses par une erreur Lua.
-- ATTENDU : ECHOUE (crash) AVANT LE CORRECTIF.
-- Lance seul (processus separe) par test_run.bat.

require("test_helpers")
print("--- regress_hunt1_event_send ---")

test("EVENT_KEY sans touche sur lv.obj (scrollable)", function()
    local o = lv.obj.new(window)
    lv.event.send(o, lv.EVENT_KEY)
    lv.event.send(o, lv.EVENT_KEY, nil)
    test_tick(2)
end)

test("EVENT_KEY sans touche sur un slider", function()
    local s = lv.slider.new(window)
    lv.event.send(s, lv.EVENT_KEY)
    test_tick(2)
end)

test("EVENT_KEY sans touche sur un objet checkable", function()
    local b = lv.btn.new(window)
    lv.obj.add_flag(b, lv.OBJ_FLAG_CHECKABLE or 8)
    lv.event.send(b, lv.EVENT_KEY)
    test_tick(2)
end)

test("EVENT_KEY sans touche : get_key_value = 0", function()
    local o = lv.obj.new(window)
    local k
    lv.obj.add_event_cb(o, function(e) k = string.byte(lv.event.get_key_value(e)) end,
                        lv.EVENT_KEY)
    lv.event.send(o, lv.EVENT_KEY)
    expect_eq(k, nil, "touche par defaut (chaine vide)")
    lv.event.send(o, lv.EVENT_KEY, lv.KEY_RIGHT)
    expect_eq(k, lv.KEY_RIGHT, "touche transmise")
end)

test("codes a param structure refuses", function()
    local o = lv.obj.new(window)
    for _, code in ipairs({ 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 1000 }) do
        local ok = pcall(lv.event.send, o, code)
        expect_eq(ok, false, "code " .. code .. " accepte")
        ok = pcall(lv.event.send, o, code, 5)
        expect_eq(ok, false, "code " .. code .. " (param 5) accepte")
    end
    test_tick(2)
end)

test("codes lv.EVENT_* exposes toujours acceptes", function()
    local o = lv.obj.new(window)
    local seen = 0
    lv.obj.add_event_cb(o, function(e) seen = seen + 1 end, lv.EVENT_ALL or 0)
    for _, name in ipairs({ "EVENT_CLICKED", "EVENT_PRESSED", "EVENT_RELEASED",
                            "EVENT_FOCUSED", "EVENT_DEFOCUSED", "EVENT_KEY",
                            "EVENT_SCROLL_BEGIN", "EVENT_SCROLL_END",
                            "EVENT_VALUE_CHANGED", "EVENT_READY", "EVENT_CANCEL" }) do
        local ok, err = pcall(lv.event.send, o, lv[name], nil)
        expect_true(ok, name .. " refuse : " .. tostring(err))
        ok, err = pcall(lv.event.send, o, lv[name], 5)
        expect_true(ok, name .. " (param 5) refuse : " .. tostring(err))
    end
    expect_ge(seen, 22, "callbacks recus")
    test_tick(2)
end)
