-- regress_anim_del.lua — Non-regression F05 (lot L3)
-- La var LVGL de l'anim est le userdata anim et non l'objet : lv.obj.del
-- n'arretait pas l'animation et exec_cb ecrivait dans l'objet libere.
-- Comportement sur attendu : apres del, exec_cb n'est plus appele, ou
-- l'objet recu leve une erreur Lua propre (handle invalide).
-- ATTENDU : ECHOUE (crash ou assertion) AVANT LE CORRECTIF L2/L3.
-- Lance seul (processus separe) par test_run.bat.

require("test_helpers")
print("--- regress_anim_del ---")

test("anim infinie puis del de l'objet anime", function()
    local o = lv.obj.new(window)
    local deleted = false
    local unsafe_calls = 0

    local a = lv.anim.new()
    lv.anim.set_var(a, o)
    lv.anim.set_values(a, 0, 100)
    lv.anim.set_time(a, 50)
    lv.anim.set_repeat_count(a, lv.ANIM_REPEAT_INFINITE)
    lv.anim.set_exec_cb(a, function(obj, v)
        if deleted then
            -- Ne doit pas arriver ; si ca arrive, l'objet doit etre invalide
            if pcall(lv.obj.set_style_translate_y, obj, v, 0) then
                unsafe_calls = unsafe_calls + 1
            end
        else
            lv.obj.set_style_translate_y(obj, v, 0)
        end
    end)
    lv.anim.start(a)
    test_tick(5)

    lv.obj.del(o)
    deleted = true

    -- Churn du tas pour reutiliser le bloc libere
    for i = 1, 200 do
        local l = lv.label.new(window)
        lv.label.set_text(l, string.rep("X", 64))
    end

    test_tick(100)
    expect_eq(unsafe_calls, 0, "exec_cb sur objet supprime")
end)
