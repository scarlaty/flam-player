-- ==========================================================================
-- fuzz_lvgl.lua — Fuzzer intensif pour les bindings LVGL/Lua
-- Usage : charger ce script au demarrage de l'emulateur FLAM compile avec ASan
-- Chaque categorie de test est isolee dans sa propre fonction.
-- Les messages de log permettent de correler un crash ASan avec le scenario.
-- ==========================================================================

local LOG_PREFIX = "[FUZZ]"

local function log(msg)
    print(LOG_PREFIX .. " " .. msg)
end

-- helper : appel protege qui capture les erreurs Lua sans arreter le fuzzer
local function try(description, fn)
    if description then
        log("  -> " .. description)
    end
    local ok, err = pcall(fn)
    if not ok then
        log("  !! Lua error: " .. tostring(err))
    end
end

-- =========================================================================
-- 1. DEPASSEMENT DE TAMPON TEXTUEL  (Buffer Overflow)
-- =========================================================================
function test_buffer_overflow()
    log("========== TEST 1 : Buffer Overflow (label set_text) ==========")

    local parent = lv.obj.new()
    lv.obj.set_size(parent, 200, 200)

    -- 1a. Chaines de taille exponentielle : 1 Ko -> 1 Mo
    log("[1a] Chaines de taille exponentielle")
    local sizes = { 1024, 4096, 16384, 32768, 65536 }
    for _, sz in ipairs(sizes) do
        try(string.format("set_text avec %d octets de 'A'", sz), function()
            local lbl = lv.label.new(parent)
            lv.label.set_long_mode(lbl, lv.LABEL_LONG_CLIP) -- eviter le calcul de wrapping
            lv.label.set_text(lbl, string.rep("A", sz))
            lv.obj.del(lbl)
        end)
    end

    -- 1b. Caracteres de controle non-ASCII (0x01-0x1F, 0x7F-0xFF)
    log("[1b] Chaines de caracteres de controle")
    local ctrl = {}
    for i = 1, 255 do
        ctrl[#ctrl + 1] = string.char(i)
    end
    local ctrl_str = table.concat(ctrl)
    for _, n in ipairs({10, 50, 100}) do
        try(string.format("set_text avec %d x bloc de controle (255 chars)", n), function()
            local lbl = lv.label.new(parent)
            lv.label.set_long_mode(lbl, lv.LABEL_LONG_CLIP)
            lv.label.set_text(lbl, string.rep(ctrl_str, n))
            lv.obj.del(lbl)
        end)
    end

    -- 1c. Sequences Unicode exotiques (multi-octets, emoji, surrogates invalides)
    log("[1c] Sequences Unicode exotiques")
    local unicode_payloads = {
        -- Emoji compose sequence
        string.rep("\xF0\x9F\x91\xA8\xE2\x80\x8D\xF0\x9F\x91\xA9\xE2\x80\x8D\xF0\x9F\x91\xA7\xE2\x80\x8D\xF0\x9F\x91\xA6", 100),
        -- Caracteres de combinaison empiles (zalgo)
        string.rep("A\xCC\x80\xCC\x81\xCC\x82\xCC\x83\xCC\x84\xCC\x85\xCC\x86\xCC\x87\xCC\x88\xCC\x89", 100),
        -- Sequences UTF-8 invalides / tronquees
        string.rep("\xC0\xAF", 1000),           -- overlong encoding
        string.rep("\xED\xA0\x80", 1000),        -- surrogate halves (invalide)
        string.rep("\xF4\x90\x80\x80", 1000),    -- au-dela de U+10FFFF
        string.rep("\x80\x81\x82\x83", 1000),    -- continuation bytes orphelins
        string.rep("\xFE\xFF", 1000),             -- octets jamais valides en UTF-8
        -- Melange de texte valide et d'octets tronques
        string.rep("Hello\xC3", 500),             -- sequence 2-byte tronquee
        string.rep("\xF0\x9F", 500),               -- sequence 4-byte tronquee a 2
    }
    for i, payload in ipairs(unicode_payloads) do
        try(string.format("Unicode payload #%d (%d octets)", i, #payload), function()
            local lbl = lv.label.new(parent)
            lv.label.set_long_mode(lbl, lv.LABEL_LONG_CLIP)
            lv.label.set_text(lbl, payload)
            lv.obj.del(lbl)
        end)
    end

    -- 1d. Sequences de formatage (si LVGL recoloring est actif : #RRGGBB text#)
    log("[1d] Sequences de formatage LVGL")
    local fmt_payloads = {
        string.rep("#FF0000 red# normal ", 1000),
        string.rep("#", 10000),
        string.rep("#ZZZZZZ bad#", 1000),
        "#" .. string.rep("F", 10000) .. "#",
        string.rep("\n", 10000),
        string.rep("\t", 10000),
    }
    for i, payload in ipairs(fmt_payloads) do
        try(string.format("Format payload #%d (%d octets)", i, #payload), function()
            local lbl = lv.label.new(parent)
            lv.label.set_long_mode(lbl, lv.LABEL_LONG_CLIP)
            lv.label.set_text(lbl, payload)
            lv.obj.del(lbl)
        end)
    end

    -- 1e. Tester differents long_mode avec de longues chaines
    log("[1e] Differents long_mode avec chaine de 100Ko")
    local long_str = string.rep("Bonjour le monde! ", 200)  -- ~3.6 Ko
    local modes = {
        { "WRAP",            lv.LABEL_LONG_WRAP },
        { "DOT",             lv.LABEL_LONG_DOT },
        { "SCROLL",          lv.LABEL_LONG_SCROLL },
        { "SCROLL_CIRCULAR", lv.LABEL_LONG_SCROLL_CIRCULAR },
        { "CLIP",            lv.LABEL_LONG_CLIP },
    }
    for _, m in ipairs(modes) do
        try(string.format("long_mode %s avec 108 Ko", m[1]), function()
            local lbl = lv.label.new(parent)
            lv.label.set_long_mode(lbl, m[2])
            lv.obj.set_width(lbl, 50) -- forcer le wrapping
            lv.label.set_text(lbl, long_str)
            lv.obj.invalidate(lbl)
            lv.obj.del(lbl)
        end)
    end

    -- 1f. Chaine vide et chaine nulle
    log("[1f] Cas limites : chaine vide")
    try("set_text chaine vide", function()
        local lbl = lv.label.new(parent)
        lv.label.set_text(lbl, "")
        lv.obj.invalidate(lbl)
        lv.label.set_text(lbl, "ok")
        lv.label.set_text(lbl, "")
        lv.obj.del(lbl)
    end)

    lv.obj.del(parent)
    log("========== TEST 1 TERMINE ==========\n")
end

-- =========================================================================
-- 2. GESTION DE POINTEURS INVALIDES  (Use-After-Free)
-- =========================================================================
function test_use_after_free()
    log("========== TEST 2 : Use-After-Free ==========")

    -- 2a. Arbre d'objets avec styles et evenements, suppression du parent
    log("[2a] Arbre d'objets : creation")
    local parent = lv.obj.new()
    lv.obj.set_size(parent, 300, 300)

    local children = {}
    local styles = {}
    local anims = {}
    local NUM_CHILDREN = 50

    for i = 1, NUM_CHILDREN do
        local btn = lv.btn.new(parent)
        lv.obj.set_size(btn, 40, 30)
        lv.obj.set_pos(btn, (i % 10) * 42, math.floor(i / 10) * 32)

        -- Label enfant du bouton
        local lbl = lv.label.new(btn)
        lv.label.set_text(lbl, string.format("B%d", i))

        -- Style unique par bouton
        local s = lv.style.new()
        lv.style.set_bg_color(s, lv.color.hex(i * 12345))
        lv.style.set_bg_opa(s, lv.OPA_COVER)
        lv.style.set_radius(s, i % 20)
        lv.style.set_border_width(s, i % 5)
        lv.style.set_border_color(s, lv.color.hex(0xFF0000 + i * 256))
        lv.obj.add_style(btn, s, 0)

        -- Callback d'evenement
        lv.obj.add_event_cb(btn, function(e)
            -- ce callback ne devrait jamais etre appele apres del
        end, lv.EVENT_CLICKED)

        -- Animation active
        local a = lv.anim.new()
        lv.anim.set_var(a, btn)
        lv.anim.set_values(a, 0, 100)
        lv.anim.set_time(a, 500 + i * 10)
        lv.anim.set_repeat_count(a, lv.ANIM_REPEAT_INFINITE)
        lv.anim.set_playback_time(a, 500)
        lv.anim.set_exec_cb(a, function(obj, val)
            -- tenter de manipuler l'objet pendant l'animation
            lv.obj.set_style_translate_y(obj, val - 50, 0)
        end)
        lv.anim.set_path_cb(a, lv.anim.path_ease_in_out)
        lv.anim.start(a)

        children[i] = btn
        styles[i] = s
        anims[i] = a
    end

    log("[2a] Suppression brutale du parent (50 enfants + anims actives)")
    lv.obj.del(parent)

    -- 2b. Acces aux references apres suppression
    log("[2b] Acces aux anciennes references Lua apres del")
    for i = 1, NUM_CHILDREN do
        try(string.format("Acces enfant #%d apres del parent", i), function()
            lv.obj.get_width(children[i])
        end)
        try(string.format("set_size enfant #%d apres del parent", i), function()
            lv.obj.set_size(children[i], 100, 100)
        end)
        try(string.format("add_style enfant #%d apres del parent", i), function()
            lv.obj.add_style(children[i], styles[i], 0)
        end)
        try(string.format("del enfant #%d (double free)", i), function()
            lv.obj.del(children[i])
        end)
    end

    -- 2c. Manipulation de styles orphelins
    log("[2c] Manipulation de styles apres suppression des objets")
    for i = 1, NUM_CHILDREN do
        try(string.format("Modification style orphelin #%d", i), function()
            lv.style.set_bg_color(styles[i], lv.color.hex(0x00FF00))
            lv.style.set_radius(styles[i], 999)
        end)
    end

    -- 2d. Suppression enfant puis parent : ordre inverse
    log("[2d] Suppression enfant-par-enfant puis parent")
    local parent2 = lv.obj.new()
    local kids2 = {}
    for i = 1, 30 do
        kids2[i] = lv.btn.new(parent2)
        lv.label.new(kids2[i])
    end
    -- Supprimer les enfants d'abord
    for i = 1, 30 do
        try(string.format("del enfant #%d", i), function()
            lv.obj.del(kids2[i])
        end)
    end
    -- Puis le parent (qui n'a plus d'enfants)
    try("del parent apres suppression de tous les enfants", function()
        lv.obj.del(parent2)
    end)
    -- Re-tenter d'acceder
    try("acces parent2 apres del", function()
        lv.obj.get_width(parent2)
    end)

    -- 2e. clean() puis acces aux anciens enfants
    log("[2e] clean() puis acces")
    local parent3 = lv.obj.new()
    local kids3 = {}
    for i = 1, 20 do
        kids3[i] = lv.obj.new(parent3)
    end
    lv.obj.clean(parent3)
    for i = 1, 20 do
        try(string.format("acces enfant #%d apres clean(parent)", i), function()
            lv.obj.set_size(kids3[i], 10, 10)
        end)
    end
    lv.obj.del(parent3)

    -- 2f. Suppression pendant iteration get_child
    log("[2f] Suppression pendant iteration get_child")
    local parent4 = lv.obj.new()
    for i = 1, 20 do
        lv.btn.new(parent4)
    end
    try("del children pendant iteration", function()
        local cnt = lv.obj.get_child_cnt(parent4)
        for i = 0, cnt - 1 do
            local c = lv.obj.get_child(parent4, i)
            if c then
                lv.obj.del(c)
            end
        end
    end)
    try("del parent4 final", function()
        lv.obj.del(parent4)
    end)

    log("========== TEST 2 TERMINE ==========\n")
end

-- =========================================================================
-- 3. DEPASSEMENT D'ENTIERS  (Integer Overflow via coordonnees extremes)
-- =========================================================================
function test_integer_overflow()
    log("========== TEST 3 : Integer Overflow ==========")

    local parent = lv.obj.new()
    lv.obj.set_size(parent, 400, 400)

    -- Valeurs critiques a tester
    local critical_values = {
        0,
        1,
        -1,
        127,
        128,
        255,
        256,
        -128,
        -129,
        32767,
        32768,
        -32768,
        -32769,
        65535,
        65536,
        -65536,
        2147483647,    -- INT32_MAX
        -2147483648,   -- INT32_MIN
        2147483646,    -- INT32_MAX - 1
        -2147483647,   -- INT32_MIN + 1
    }

    -- 3a. set_width / set_height avec valeurs critiques
    log("[3a] set_width / set_height avec valeurs critiques")
    for _, v in ipairs(critical_values) do
        try(string.format("set_size(%d, %d)", v, v), function()
            local obj = lv.obj.new(parent)
            lv.obj.set_width(obj, v)
            lv.obj.set_height(obj, v)
            lv.obj.invalidate(obj)
            local w = lv.obj.get_width(obj)
            local h = lv.obj.get_height(obj)
            lv.obj.del(obj)
        end)
    end

    -- 3b. set_pos / set_x / set_y avec valeurs critiques
    log("[3b] Positions extremes")
    for _, v in ipairs(critical_values) do
        try(string.format("set_pos(%d, %d)", v, v), function()
            local obj = lv.obj.new(parent)
            lv.obj.set_pos(obj, v, v)
            lv.obj.invalidate(obj)
            lv.obj.del(obj)
        end)
    end

    -- 3c. Combinaisons taille + position extremes
    log("[3c] Combinaisons taille + position extremes")
    local combos = {
        { 2147483647, 2147483647, -2147483648, -2147483648 },
        { 0, 0, 2147483647, 2147483647 },
        { 65535, 65535, -32768, -32768 },
        { 1, 1, 2147483647, 2147483647 },
        { -1, -1, -1, -1 },
    }
    for i, c in ipairs(combos) do
        try(string.format("combo #%d: size(%d,%d) pos(%d,%d)", i, c[1], c[2], c[3], c[4]), function()
            local obj = lv.obj.new(parent)
            lv.obj.set_size(obj, c[1], c[2])
            lv.obj.set_pos(obj, c[3], c[4])
            lv.obj.invalidate(obj)
            lv.obj.del(obj)
        end)
    end

    -- 3d. Zoom avec valeurs critiques (img.set_zoom, normal = 256)
    log("[3d] img.set_zoom avec valeurs critiques")
    local zoom_values = { 0, 1, 2, 128, 255, 256, 512, 1024, 32767, 65535, -1, -256, 2147483647, -2147483648 }
    for _, z in ipairs(zoom_values) do
        try(string.format("img.set_zoom(%d)", z), function()
            local img = lv.img.new(parent)
            lv.img.set_zoom(img, z)
            lv.obj.invalidate(img)
            lv.obj.del(img)
        end)
    end

    -- 3e. Rotation d'image avec valeurs critiques
    log("[3e] img.set_angle avec valeurs critiques")
    local angle_values = { 0, 1, -1, 3600, -3600, 36000, 360000, 2147483647, -2147483648 }
    for _, a in ipairs(angle_values) do
        try(string.format("img.set_angle(%d)", a), function()
            local img = lv.img.new(parent)
            lv.img.set_angle(img, a)
            lv.obj.invalidate(img)
            lv.obj.del(img)
        end)
    end

    -- 3f. Padding et border avec valeurs critiques
    log("[3f] Padding / border / radius extremes")
    for _, v in ipairs(critical_values) do
        try(string.format("style extremes avec %d", v), function()
            local obj = lv.obj.new(parent)
            local s = lv.style.new()
            lv.style.set_pad_all(s, v)
            lv.style.set_border_width(s, v)
            lv.style.set_radius(s, v)
            lv.obj.add_style(obj, s, 0)
            lv.obj.invalidate(obj)
            lv.obj.del(obj)
        end)
    end

    -- 3g. Slider avec range et valeurs critiques
    log("[3g] Slider range et valeurs critiques")
    local range_combos = {
        { -2147483648, 2147483647 },
        { 0, 0 },
        { -1, -1 },
        { 2147483647, -2147483648 },  -- min > max
        { 0, 1 },
        { -32768, 32767 },
    }
    for i, r in ipairs(range_combos) do
        try(string.format("slider range(%d, %d)", r[1], r[2]), function()
            local sl = lv.slider.new(parent)
            lv.slider.set_range(sl, r[1], r[2])
            -- Tenter de mettre des valeurs hors range
            lv.slider.set_value(sl, 2147483647, lv.ANIM_OFF)
            lv.slider.set_value(sl, -2147483648, lv.ANIM_OFF)
            lv.slider.set_value(sl, 0, lv.ANIM_OFF)
            lv.obj.del(sl)
        end)
    end

    -- 3h. Arc angles extremes
    log("[3h] Arc angles extremes")
    local arc_combos = {
        { 0, 360 },
        { -360, 720 },
        { 2147483647, -2147483648 },
        { 0, 0 },
        { 65535, 65535 },
    }
    for i, ac in ipairs(arc_combos) do
        try(string.format("arc.set_angles(%d, %d)", ac[1], ac[2]), function()
            local a = lv.arc.new(parent)
            lv.arc.set_angles(a, ac[1], ac[2])
            lv.arc.set_rotation(a, 2147483647)
            lv.obj.invalidate(a)
            lv.obj.del(a)
        end)
    end

    -- 3i. scroll_by / scroll_to avec valeurs extremes
    log("[3i] Scroll avec valeurs extremes")
    for _, v in ipairs(critical_values) do
        try(string.format("scroll_to(%d, %d)", v, v), function()
            lv.obj.scroll_to(parent, v, v, lv.ANIM_OFF)
        end)
    end

    -- 3j. align avec offsets extremes
    log("[3j] Align avec offsets extremes")
    local aligns = {
        lv.ALIGN_CENTER, lv.ALIGN_TOP_LEFT, lv.ALIGN_BOTTOM_RIGHT,
    }
    for _, al in ipairs(aligns) do
        for _, v in ipairs({ 2147483647, -2147483648, 0, 65535, -32768 }) do
            try(string.format("align(%d, offset=%d)", al, v), function()
                local obj = lv.obj.new(parent)
                lv.obj.align(obj, al, v, v)
                lv.obj.invalidate(obj)
                lv.obj.del(obj)
            end)
        end
    end

    -- 3k. lv.pct() avec valeurs critiques
    log("[3k] lv.pct() avec valeurs critiques")
    local pct_values = { 0, 1, 50, 100, 200, 1000, -1, -100, 32767, 65535, 2147483647, -2147483648 }
    for _, p in ipairs(pct_values) do
        try(string.format("set_width(pct(%d))", p), function()
            local obj = lv.obj.new(parent)
            lv.obj.set_width(obj, lv.pct(p))
            lv.obj.set_height(obj, lv.pct(p))
            lv.obj.invalidate(obj)
            lv.obj.del(obj)
        end)
    end

    -- 3l. Animations avec valeurs extremes
    log("[3l] Animations avec valeurs extremes dans set_values")
    local anim_combos = {
        { 0, 2147483647 },
        { -2147483648, 2147483647 },
        { 2147483647, -2147483648 },
        { 0, 0 },
    }
    for i, ac in ipairs(anim_combos) do
        try(string.format("anim set_values(%d, %d)", ac[1], ac[2]), function()
            local obj = lv.obj.new(parent)
            local a = lv.anim.new()
            lv.anim.set_var(a, obj)
            lv.anim.set_values(a, ac[1], ac[2])
            lv.anim.set_time(a, 1) -- 1 ms pour finir vite
            lv.anim.set_exec_cb(a, function(o, val)
                lv.obj.set_x(o, val)
            end)
            lv.anim.start(a)
            -- on ne supprime pas immediatement : laisser l'anim se terminer
        end)
    end

    -- 3m. set_time / set_delay avec valeurs critiques
    log("[3m] Anim timing extreme")
    local time_values = { 0, 1, -1, 2147483647, -2147483648 }
    for _, t in ipairs(time_values) do
        try(string.format("anim set_time(%d)", t), function()
            local obj = lv.obj.new(parent)
            local a = lv.anim.new()
            lv.anim.set_var(a, obj)
            lv.anim.set_values(a, 0, 100)
            lv.anim.set_time(a, t)
            lv.anim.set_delay(a, t)
            lv.anim.set_exec_cb(a, function(o, val) end)
            lv.anim.start(a)
        end)
    end

    lv.obj.del(parent)
    log("========== TEST 3 TERMINE ==========\n")
end

-- =========================================================================
-- 4. EPUISEMENT DES RESSOURCES  (Stress Test / Heap Exhaustion)
-- =========================================================================
function test_heap_exhaustion()
    log("========== TEST 4 : Heap Exhaustion ==========")

    -- 4a. Instanciation massive sans suppression
    log("[4a] Creation de 2000 objets sans suppression")
    local root = lv.obj.new()
    lv.obj.set_size(root, 10, 10)
    local mass_objects = {}

    for i = 1, 2000 do
        local ok, obj = pcall(lv.obj.new, root)
        if not ok then
            log(string.format("  !! Echec creation objet #%d: %s", i, tostring(obj)))
            break
        end
        mass_objects[i] = obj

        -- Varier les types d'objets pour diversifier l'allocation
        if i % 4 == 0 then
            pcall(function()
                local lbl = lv.label.new(obj)
                lv.label.set_text(lbl, string.format("Label %d avec du texte", i))
            end)
        elseif i % 4 == 1 then
            pcall(function()
                lv.btn.new(obj)
            end)
        elseif i % 4 == 2 then
            pcall(function()
                local s = lv.style.new()
                lv.style.set_bg_color(s, lv.color.hex(i * 7))
                lv.style.set_bg_opa(s, lv.OPA_COVER)
                lv.obj.add_style(obj, s, 0)
            end)
        end

        if i % 500 == 0 then
            log(string.format("  ... %d objets crees", i))
        end
    end

    log("[4a] Tentative de suppression massive")
    try("del root (2000+ objets)", function()
        lv.obj.del(root)
    end)

    -- 4b. Cycles creation/suppression rapides
    log("[4b] 500 cycles creation/suppression rapides")
    for i = 1, 500 do
        try(i % 100 == 0 and string.format("cycle %d/500", i) or nil, function()
            local o = lv.obj.new()
            local lbl = lv.label.new(o)
            lv.label.set_text(lbl, "ephemere")
            local s = lv.style.new()
            lv.style.set_bg_color(s, lv.color.hex(0xFF00FF))
            lv.obj.add_style(o, s, 0)
            lv.obj.del(o)
        end)
        -- ne pas spammer les logs pour les cycles intermediaires
    end

    -- 4c. Styles massifs
    log("[4c] Creation de 1000 styles")
    local mass_styles = {}
    for i = 1, 1000 do
        local ok, s = pcall(lv.style.new)
        if not ok then
            log(string.format("  !! Echec creation style #%d: %s", i, tostring(s)))
            break
        end
        mass_styles[i] = s
        pcall(function()
            lv.style.set_bg_color(s, lv.color.hex(i))
            lv.style.set_radius(s, i % 100)
            lv.style.set_border_width(s, i % 10)
            lv.style.set_pad_all(s, i % 50)
        end)
        if i % 500 == 0 then
            log(string.format("  ... %d styles crees", i))
        end
    end

    -- 4d. Timers massifs
    log("[4d] Creation de 500 timers")
    local mass_timers = {}
    for i = 1, 500 do
        local ok, t = pcall(lv.timer.new, function() end, 10)
        if not ok then
            log(string.format("  !! Echec creation timer #%d: %s", i, tostring(t)))
            break
        end
        mass_timers[i] = t
    end
    log("[4d] Suppression des timers")
    for i = 1, #mass_timers do
        pcall(function() lv.timer.del(mass_timers[i]) end)
    end

    -- 4e. Animations massives
    log("[4e] Creation de 500 animations")
    local anim_parent = lv.obj.new()
    for i = 1, 500 do
        pcall(function()
            local obj = lv.obj.new(anim_parent)
            local a = lv.anim.new()
            lv.anim.set_var(a, obj)
            lv.anim.set_values(a, 0, 100)
            lv.anim.set_time(a, 100)
            lv.anim.set_repeat_count(a, lv.ANIM_REPEAT_INFINITE)
            lv.anim.set_exec_cb(a, function(o, v)
                pcall(lv.obj.set_x, o, v)
            end)
            lv.anim.start(a)
        end)
        if i % 100 == 0 then
            log(string.format("  ... %d animations actives", i))
        end
    end
    try("del anim_parent (500 anims)", function()
        lv.obj.del(anim_parent)
    end)

    -- 4f. Arbre profondement imbrique
    log("[4f] Arbre imbrique sur 200 niveaux de profondeur")
    try("deep nesting", function()
        local current = lv.obj.new()
        local root_deep = current
        for i = 1, 200 do
            local child = lv.obj.new(current)
            lv.obj.set_size(child, 5, 5)
            current = child
        end
        -- Placer un label au fond de l'arbre
        local deep_label = lv.label.new(current)
        lv.label.set_text(deep_label, "fond de l'arbre")
        lv.obj.invalidate(root_deep)
        lv.obj.del(root_deep)
    end)

    log("========== TEST 4 TERMINE ==========\n")
end

-- =========================================================================
-- EXECUTION PRINCIPALE
-- =========================================================================
log("================================================================")
log(" DEBUT DU FUZZING LVGL — " .. os.date("%Y-%m-%d %H:%M:%S"))
log(" Lua " .. _VERSION)
log("================================================================\n")

local tests = {
    { "Buffer Overflow",    test_buffer_overflow },
    { "Use-After-Free",     test_use_after_free },
    { "Integer Overflow",   test_integer_overflow },
    { "Heap Exhaustion",    test_heap_exhaustion },
}

local results = {}
for _, t in ipairs(tests) do
    log(">>> Lancement : " .. t[1])
    local start = os.clock()
    local ok, err = pcall(t[2])
    local elapsed = os.clock() - start
    if ok then
        results[#results + 1] = string.format("  [OK]   %-25s (%.3fs)", t[1], elapsed)
    else
        results[#results + 1] = string.format("  [FAIL] %-25s (%.3fs) — %s", t[1], elapsed, tostring(err))
    end
end

log("================================================================")
log(" RESUME DU FUZZING")
log("================================================================")
for _, r in ipairs(results) do
    log(r)
end
log("================================================================")
log(" FIN DU FUZZING")
log("================================================================")
