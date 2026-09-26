-- regress_heap_exhaust.lua — X1a : epuisement du tas LVGL
-- Avant : lv_obj_create plantait (~1700 enfants, lv_mem_realloc NULL non
-- teste dans lv_obj_class.c). Apres : erreur Lua "tas LVGL epuise",
-- l'etat reste utilisable et le tas est rendu apres suppression.

require("test_helpers")

local function fill(fn, max)
    local n, err = 0, nil
    for i = 1, max do
        local ok, e = pcall(fn, i)
        if not ok then err = e; break end
        n = i
    end
    return n, err
end

local function is_heap_err(e)
    return type(e) == "string" and e:find("tas LVGL epuise", 1, true) ~= nil
end

test("lv.obj.new jusqu'a l'erreur propre (enfants d'un meme parent)", function()
    local free0 = lv.mem.monitor().free_size
    local root = lv.obj.new()
    local keep = {}
    local n, err = fill(function(i) keep[i] = lv.obj.new(root) end, 100000)
    expect_true(n > 100, "trop peu d'objets crees : " .. n)
    expect_true(is_heap_err(err), "erreur attendue 'tas LVGL epuise', obtenu " .. tostring(err))
    print(string.format("  %d objets avant erreur : %s", n, err))
    -- Etat encore utilisable a tas presque plein : getters, del
    expect_eq(lv.obj.get_child_cnt(root), n, "child_cnt")
    lv.obj.del(root)
    keep = nil
    test_tick(3)
    collectgarbage("collect"); collectgarbage("collect")
    test_tick(3)
    local free1 = lv.mem.monitor().free_size
    expect_true(free1 >= free0 * 9 // 10, string.format("tas non rendu : %d / %d", free1, free0))
    -- Et on peut recreer
    local o = lv.obj.new()
    expect_true(o ~= nil, "creation apres liberation")
    lv.obj.del(o)
end)

test("widgets varies + styles + events + anims jusqu'a l'erreur", function()
    local root = lv.obj.new()
    local keep = {}
    local kinds = { "btn", "label", "img", "slider", "arc" }
    local n, err = fill(function(i)
        local k = kinds[i % #kinds + 1]
        local o = lv[k].new(root)
        keep[#keep + 1] = o
        if k == "label" then lv.label.set_text(o, string.rep("x", 40)) end
        local s = lv.style.new()
        lv.style.set_bg_color(s, lv.color.hex(i))
        lv.style.set_radius(s, i % 20)
        lv.obj.add_style(o, s, 0)
        keep[#keep + 1] = s
        lv.obj.set_size(o, 5, 5)
        lv.obj.add_event_cb(o, function() end, lv.EVENT_CLICKED)
        if i % 7 == 0 then
            local a = lv.anim.new()
            lv.anim.set_var(a, o)
            lv.anim.set_values(a, 0, 10)
            lv.anim.set_time(a, 1000)
            lv.anim.set_exec_cb(a, function(obj, v) end)
            lv.anim.start(a)
        end
        if i % 11 == 0 then
            keep[#keep + 1] = lv.timer.new(function() end, 1000)
        end
    end, 100000)
    expect_true(n > 50, "trop peu d'iterations : " .. n)
    expect_true(is_heap_err(err), "erreur attendue 'tas LVGL epuise', obtenu " .. tostring(err))
    print(string.format("  %d iterations avant erreur : %s", n, err))
    -- Style/texte a tas plein : erreur propre, pas de crash
    -- keep[1] : label (i = 1 -> kinds[2])
    local ok, e = pcall(lv.label.set_text, keep[1], string.rep("y", 200000))
    expect_true(not ok and is_heap_err(e), "set_text 200 KB a tas plein : " .. tostring(e))
    for _, v in ipairs(keep) do lv.timer.del(v) end   -- no-op hors timers
    lv.obj.del(root)
    keep = nil
    test_tick(3)
    collectgarbage("collect"); collectgarbage("collect")
    test_tick(3)
    local o = lv.label.new()
    lv.label.set_text(o, "ok")
    expect_eq(lv.label.get_text(o), "ok", "label apres liberation")
    lv.obj.del(o)
end)

test("timers puis anims jusqu'a l'erreur propre", function()
    local root = lv.obj.new()
    local keep = {}
    fill(function(i) keep[i] = lv.obj.new(root) end, 100000)
    local timers = {}
    local n, err = fill(function(i) timers[i] = lv.timer.new(function() end, 100000) end, 100000)
    expect_true(is_heap_err(err), "timer.new : " .. tostring(err))
    print(string.format("  +%d timers avant erreur", n))
    local a = lv.anim.new()
    lv.anim.set_values(a, 0, 1); lv.anim.set_time(a, 10)
    lv.anim.set_exec_cb(a, function() end)
    local ok, e = pcall(lv.anim.start, a)
    expect_true(not ok and is_heap_err(e), "anim.start a tas plein : " .. tostring(e))
    ok, e = pcall(lv.obj.set_x, keep[1], 3)
    expect_true(ok or is_heap_err(e), "set_x a tas plein : " .. tostring(e))
    ok, e = pcall(lv.obj.add_event_cb, keep[1], function() end, lv.EVENT_CLICKED)
    expect_true(ok or is_heap_err(e), "add_event_cb a tas plein : " .. tostring(e))
    for _, t in ipairs(timers) do lv.timer.del(t) end
    lv.obj.del(root)
    keep = nil
    test_tick(3)
    ok, e = pcall(lv.anim.start, a)
    expect_true(ok, "anim.start apres liberation : " .. tostring(e))
    test_tick(5)
end)
