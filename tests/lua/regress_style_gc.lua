-- regress_style_gc.lua — Non-regression F02 (lot L2)
-- Un style Lua collecte par le GC alors qu'il est encore attache a un
-- objet : __gc fait lv_style_reset, l'objet garde un pointeur pendant,
-- use-after-free au rendu et a la suppression.
-- Comportement sur attendu : le style reste vivant tant qu'il est attache.
-- ATTENDU : ECHOUE (crash) AVANT LE CORRECTIF L2.
-- Lance seul (processus separe) par test_run.bat.

require("test_helpers")
print("--- regress_style_gc ---")

test("style collecte alors qu'il est attache", function()
    local objs = {}
    for i = 1, 200 do
        local o = lv.obj.new(window)
        objs[i] = o
        do
            local s = lv.style.new()
            lv.style.set_bg_color(s, lv.color.hex(0xFF0000))
            lv.style.set_bg_opa(s, lv.OPA_COVER)
            lv.style.set_radius(s, 5)
            lv.obj.add_style(o, s, 0)
        end
    end
    collectgarbage("collect")
    collectgarbage("collect")

    -- Churn du tas pour reutiliser les blocs des styles
    local junk = {}
    for i = 1, 400 do
        junk[i] = lv.style.new()
        lv.style.set_pad_all(junk[i], i)
        lv.style.set_border_width(junk[i], i)
    end

    test_tick(10)   -- rendu
    for i = 1, 200 do lv.obj.del(objs[i]) end
    test_tick(3)
    expect_true(true)
end)
