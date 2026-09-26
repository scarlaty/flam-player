-- regress_hunt1_anim_restart.lua — Non-regression hunt1 (bindings)
-- lv.anim.start(a) depuis l'exec_cb de la meme anim : lv_anim_start faisait
-- lv_anim_del(var, exec_cb) et liberait la copie LVGL qu'anim_timer relit au
-- retour de l'exec_cb (use-after-free puis double free). Masque par TLSF
-- (bloc reutilise aussitot) ; visible avec un tas CRT de debogage.
-- Comportement attendu : la relance fonctionne, l'anim finit par s'arreter
-- (lv.anim_var.del) et le userdata anim redevient collectable.
-- Lance seul (processus separe) par test_run.bat.

require("test_helpers")
print("--- regress_hunt1_anim_restart ---")

test("relance depuis l'exec_cb (early_apply = false)", function()
    local o = lv.obj.new(window)
    local a = lv.anim.new()
    lv.anim.set_var(a, o)
    lv.anim.set_values(a, 0, 100)
    lv.anim.set_time(a, 30)
    lv.anim.set_early_apply(a, false)
    local n = 0
    lv.anim.set_exec_cb(a, function(obj, v)
        n = n + 1
        if n < 50 then lv.anim.start(a) end
        lv.obj.set_x(obj, v)
    end)
    lv.anim.start(a)
    for i = 1, 100 do test_tick(1) end
    expect_ge(n, 10, "exec_cb apres relances")
    -- churn du tas
    for i = 1, 50 do lv.label.new(window) end
    test_tick(5)
end)

test("relance en boucle puis arret : userdata collectable", function()
    local o = lv.obj.new(window)
    local weak = setmetatable({}, { __mode = "k" })
    do
        local a = lv.anim.new()
        weak[a] = true
        lv.anim.set_var(a, o)
        lv.anim.set_values(a, 0, 10)
        lv.anim.set_time(a, 20)
        lv.anim.set_early_apply(a, false)
        local n = 0
        lv.anim.set_exec_cb(a, function(obj, v)
            n = n + 1
            if n < 20 then lv.anim.start(a) else lv.anim_var.del(a) end
        end)
        lv.anim.start(a)
    end
    for i = 1, 100 do test_tick(1) end
    collectgarbage("collect")
    collectgarbage("collect")
    expect_nil(next(weak), "anim toujours ancree apres arret")
end)

test("relance hors exec_cb : une seule copie active", function()
    local o = lv.obj.new(window)
    local a = lv.anim.new()
    lv.anim.set_var(a, o)
    lv.anim.set_values(a, 0, 100)
    lv.anim.set_time(a, 100)
    local last = {}
    lv.anim.set_exec_cb(a, function(obj, v) last[#last + 1] = v end)
    lv.anim.start(a)
    test_tick(5)
    lv.anim.start(a)
    lv.anim.start(a)
    last = {}
    test_tick(2)
    -- une seule copie vivante : une seule valeur par tick
    expect_true(#last <= 2, "copies multiples actives : " .. #last)
    for i = 1, 30 do test_tick(10) end
end)
