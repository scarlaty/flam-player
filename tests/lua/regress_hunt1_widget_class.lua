-- regress_hunt1_widget_class.lua — Non-regression hunt1 (bindings)
-- Les fonctions propres a un widget (lv.label/slider/arc/img) convertissaient
-- n'importe quel LvObj en lv_label_t, lv_slider_t... : sur un bouton, lecture
-- et ecriture hors de l'allocation (crash 0xC0000005, parfois au rendu).
-- Comportement attendu : erreur Lua "lv.<widget> attendu", tas intact.
-- ATTENDU : ECHOUE (crash) AVANT LE CORRECTIF.
-- Lance seul (processus separe) par test_run.bat.

require("test_helpers")
print("--- regress_hunt1_widget_class ---")

local function churn()
    for i = 1, 20 do
        local l = lv.label.new(window)
        lv.label.set_text(l, string.rep("y", 40))
    end
    test_tick(5)
end

local cases = {
    { "label.set_text",      function(x) lv.label.set_text(x, "hello") end },
    { "label.get_text",      function(x) return lv.label.get_text(x) end },
    { "label.set_long_mode", function(x) lv.label.set_long_mode(x, lv.LABEL_LONG_SCROLL) end },
    { "slider.set_value",    function(x) lv.slider.set_value(x, 5) end },
    { "slider.set_range",    function(x) lv.slider.set_range(x, 0, 10) end },
    { "slider.get_value",    function(x) return lv.slider.get_value(x) end },
    { "arc.set_angles",      function(x) lv.arc.set_angles(x, 10, 90) end },
    { "arc.set_rotation",    function(x) lv.arc.set_rotation(x, 90) end },
    { "img.set_zoom",        function(x) lv.img.set_zoom(x, 300) end },
    { "img.set_angle",       function(x) lv.img.set_angle(x, 450) end },
    { "img.set_src",         function(x) lv.img.set_src(x, nil) end },
}

for _, c in ipairs(cases) do
    test(c[1] .. " sur un bouton leve une erreur Lua", function()
        local x = lv.btn.new(window)
        for i = 1, 20 do lv.obj.new(window) end   -- voisins dans le tas LVGL
        local ok, err = pcall(c[2], x)
        expect_eq(ok, false, c[1] .. " accepte sur un lv.btn")
        expect_true(tostring(err):find("attendu", 1, true), "message : " .. tostring(err))
        churn()
    end)
end

test("les bons widgets restent acceptes", function()
    local l = lv.label.new(window)
    lv.label.set_text(l, "ok")
    expect_eq(lv.label.get_text(l), "ok", "label.get_text")
    lv.label.set_long_mode(l, lv.LABEL_LONG_CLIP)
    local s = lv.slider.new(window)
    lv.slider.set_range(s, 0, 10)
    lv.slider.set_value(s, 7)
    expect_eq(lv.slider.get_value(s), 7, "slider.get_value")
    local a = lv.arc.new(window)
    lv.arc.set_angles(a, 10, 90)
    lv.arc.set_rotation(a, 90)
    local i = lv.img.new(window)
    lv.img.set_zoom(i, 300)
    lv.img.set_angle(i, 450)
    test_tick(5)
end)
