-- regress_hunt1_typeconf.lua — Non-regression hunt1 (bindings)
-- set_style_text_font / style.set_text_font / anim.set_path_cb prenaient
-- n'importe quel light userdata : une path function comme police, une
-- police comme path function => LVGL appelait des pointeurs bidon (crash).
-- Comportement attendu : erreur Lua ; nil reste ignore ; les vraies
-- polices / paths restent acceptees.
-- ATTENDU : ECHOUE (crash) AVANT LE CORRECTIF.
-- Lance seul (processus separe) par test_run.bat.

require("test_helpers")
print("--- regress_hunt1_typeconf ---")

test("path function refusee comme police (obj)", function()
    local l = lv.label.new(window)
    lv.label.set_text(l, "abc")
    expect_error(function() lv.obj.set_style_text_font(l, lv.anim.path_linear, 0) end,
                 "path acceptee comme police")
    expect_error(function() lv.obj.set_style_text_font(l, l, 0) end,
                 "LvObj accepte comme police")
    test_tick(5)
end)

test("path function refusee comme police (style)", function()
    local s = lv.style.new()
    expect_error(function() lv.style.set_text_font(s, lv.anim.path_bounce) end,
                 "path acceptee comme police")
    local l = lv.label.new(window)
    lv.label.set_text(l, "abc")
    lv.obj.add_style(l, s, 0)
    test_tick(5)
end)

test("police refusee comme path d'anim", function()
    local o = lv.obj.new(window)
    local a = lv.anim.new()
    lv.anim.set_var(a, o)
    lv.anim.set_values(a, 0, 10)
    lv.anim.set_time(a, 30)
    lv.anim.set_exec_cb(a, function(obj, v) lv.obj.set_x(obj, v) end)
    expect_error(function() lv.anim.set_path_cb(a, lv.font.nunito_bold_12) end,
                 "police acceptee comme path")
    lv.anim.start(a)
    test_tick(20)
end)

test("usages valides inchanges", function()
    local l = lv.label.new(window)
    lv.label.set_text(l, "abc")
    lv.obj.set_style_text_font(l, lv.font.nunito_extrabold_16, 0)
    lv.obj.set_style_text_font(l, nil, 0)              -- ignore
    local s = lv.style.new()
    lv.style.set_text_font(s, lv.font.nunito_bold_20)
    lv.style.set_text_font(s, nil)                     -- ignore
    lv.obj.add_style(l, s, 0)
    local a = lv.anim.new()
    for _, n in ipairs({ "path_ease_in_out", "path_linear", "path_ease_out",
                         "path_ease_in", "path_overshoot", "path_bounce" }) do
        lv.anim.set_path_cb(a, lv.anim[n])
    end
    lv.anim.set_path_cb(a, nil)                        -- ignore
    test_tick(5)
end)
