-- regress_hunt2_bind_event_code.lua — Non-regression hunt2 (bindings)
-- lv.obj.add_event_cb acceptait n'importe quel code (DRAW_MAIN 21,
-- COVER_CHECK 18...) et un handler LV_EVENT_ALL (0) recevait les codes
-- internes du rendu : lv.obj.del(o) y liberait l'objet en cours de dessin
-- ("_lv_inv_area: detected modifying dirty areas in render", 0xC0000005).
-- Comportement attendu : codes hors lv.EVENT_* refuses par une erreur Lua ;
-- 0 (tous) accepte mais le callback Lua ne voit que les lv.EVENT_* exposes.
-- ATTENDU : ECHOUE (crash) AVANT LE CORRECTIF.
-- Lance seul (processus separe) par test_run.bat.

require("test_helpers")
print("--- regress_hunt2_bind_event_code ---")

local exposed = {}
for _, name in ipairs({ "EVENT_CLICKED", "EVENT_PRESSED", "EVENT_RELEASED",
                        "EVENT_FOCUSED", "EVENT_DEFOCUSED", "EVENT_KEY",
                        "EVENT_SCROLL_BEGIN", "EVENT_SCROLL_END",
                        "EVENT_VALUE_CHANGED", "EVENT_DELETE",
                        "EVENT_READY", "EVENT_CANCEL" }) do
    exposed[lv[name]] = name
end

test("codes internes refuses par add_event_cb", function()
    local o = lv.obj.new(window)
    for _, code in ipairs({ 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 40, 1000, -1, 0x80 }) do
        local ok = pcall(lv.obj.add_event_cb, o, function() end, code)
        expect_eq(ok, false, "code " .. code .. " accepte")
    end
    lv.obj.del(o)
end)

test("codes lv.EVENT_* exposes acceptes", function()
    local o = lv.obj.new(window)
    for code, name in pairs(exposed) do
        local ok, err = pcall(lv.obj.add_event_cb, o, function() end, code)
        expect_true(ok, name .. " refuse : " .. tostring(err))
    end
    lv.obj.del(o)
    test_tick(2)
end)

test("handler 0 (tous) : pas de code de rendu, del sans crash", function()
    local o = lv.obj.new(window)
    lv.obj.set_size(o, 50, 50)
    local bad, calls = nil, 0
    lv.obj.add_event_cb(o, function(e)
        calls = calls + 1
        local code = lv.event.get_code(e)
        if not exposed[code] then bad = code end
        if code ~= lv.EVENT_DELETE then lv.obj.del(o) end
    end, 0)
    lv.obj.invalidate(o)
    test_tick(5)
    expect_nil(bad, "code interne transmis a Lua")
    -- toujours vivant : aucun evenement expose pendant le rendu
    expect_type(lv.obj.get_width(o), "number", "objet supprime pendant le rendu")
    lv.event.send(o, lv.EVENT_CLICKED)
    expect_error(function() lv.obj.get_width(o) end, "del sur CLICKED")
    for i = 1, 50 do lv.label.new(window) end
    test_tick(3)
end)
