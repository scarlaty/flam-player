-- regress_hunt3_close_a.lua (+ regress_hunt3_close_b.lua)
-- Un __gc execute pendant lua_close cree un timer, une anim et un objet
-- avec handler : leurs userdata ne sont jamais finalises (Lua 5.4 ignore
-- les nouveaux finaliseurs pendant la fermeture), LVGL gardait des
-- pointeurs vers l'etat libere et lv_timer_handler plantait ensuite
-- (0xC0000005). Isole, ce fichier passe toujours ; le cas se verifie en
-- enchainant les deux fichiers dans le meme processus :
--   flam-test regress_hunt3_close_a.lua regress_hunt3_close_b.lua
require("test_helpers")

KEEP = setmetatable({}, {__gc = function()
    lv.timer.new(function() io.write("timer apres lua_close\n") end, 5)
    local a = lv.anim.new()
    lv.anim.set_values(a, 0, 100)
    lv.anim.set_time(a, 1000)
    lv.anim.set_exec_cb(a, function(v, x) end)
    lv.anim.start(a)
    local o = lv.obj.new(nil)                      -- ecran hors lv_obj_clean
    lv.obj.add_event_cb(o, function() end, lv.EVENT_DELETE)
    local c = lv.obj.new(o)
    lv.obj.add_event_cb(c, function() end, lv.EVENT_CLICKED)
    CLOSE_SCREEN = o
end})

test("finaliseur arme pour lua_close", function()
    expect_true(getmetatable(KEEP).__gc ~= nil)
end)
