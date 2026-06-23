local btn = {}

function btn.clean(button)

    if (button ~= nil) then

        if (button.focus_cb ~= nil) then
            lv.obj.remove_event_cb(button.button, button.focus_cb)
            button.focus_cb = nil
        end

        if (button.defocus_cb ~= nil) then
            lv.obj.remove_event_cb(button.button, button.defocus_cb)
            button.defocus_cb = nil
        end

        if (button.img_data ~= nil) then
            button.img_data = nil
        end

        if (button.img_hover_data ~= nil) then
            button.img_hover_data = nil
        end

        if (button.button_style ~= nil) then
            lv.obj.remove_style(button.button, button.button_style, lv.STATE_DEFAULT)
            button.button_style = nil
        end

        if (button.button_focused_style ~= nil) then
            lv.obj.remove_style(button.button, button.button_focused_style, lv.STATE_FOCUSED)
            button.button_focused_style = nil
        end

        if (button.button_pressed_style ~= nil) then
            lv.obj.remove_style(button.button, button.button_pressed_style, lv.STATE_PRESSED)
            button.button_pressed_style = nil
        end

    end

end

function btn.set_theme(button, theme, size)

    btn.set_default_style(button, theme:get_default_style(size))
    btn.set_focused_style(button, theme:get_focused_style(size))
    btn.set_pressed_style(button, theme:get_pressed_style(size))

end

function btn.set_default_style(button, style)

    if (button.button_style ~= nil) then
        lv.obj.remove_style(button.button, button.button_style, lv.STATE_DEFAULT)
    end

    button.button_style = style
    lv.obj.add_style(button.button, button.button_style, lv.STATE_DEFAULT)

end

function btn.set_focused_style(button, style)

    if (button.button_focused_style ~= nil) then
        lv.obj.remove_style(button.button, button.button_focused_style, lv.STATE_FOCUSED)
    end

    button.button_focused_style = style
    lv.obj.add_style(button.button, button.button_focused_style, lv.STATE_FOCUSED)

end

function btn.set_pressed_style(button, style)

    if (button.button_pressed_style ~= nil) then
        lv.obj.remove_style(button.button, button.button_pressed_style, lv.STATE_PRESSED)
    end

    button.button_pressed_style = style
    lv.obj.add_style(button.button, button.button_pressed_style, lv.STATE_PRESSED)

end

local function bit_and(a, b)

    local result = 0
    local bitval = 1

    while a > 0 and b > 0 do
        if a % 2 == 1 and b % 2 == 1 then -- test the rightmost bits
            result = result + bitval      -- set the current bit
        end
        bitval = bitval * 2 -- shift left
        a = math.floor(a/2) -- shift right
        b = math.floor(b/2)
    end

    return result
end

local function bit_or(a, b)

    local result = 0
    local bitval = 1

    while a > 0 and b > 0 do
        if a % 2 == 1 and b % 2 == 1 then -- test the rightmost bits
            result = result + bitval      -- set the current bit
        end
        bitval = bitval * 2 -- shift left
        a = math.floor(a/2) -- shift right
        b = math.floor(b/2)
    end

    return result
end

function btn.set_icons(button, icon_path, icon_hover_path)

    local label_margin_left

    if (button.size == "small") then
        label_margin_left = 19
    else
        label_margin_left = 22
    end

    lv.obj.set_style_pad_left(button.label, label_margin_left, lv.STATE_DEFAULT)

    if (button.icon ~= nil) then
        lv.obj.del(button.icon)
    end

    button.icon = lv.img.new(button.button)
    button.img_data = Global.load_image(icon_path)
    lv.img.set_src(button.icon, button.img_data)
    lv.obj.align(button.icon, lv.ALIGN_LEFT_MID, 0, 0)

    if (icon_hover_path ~= nil) then

        button.img_hover_data = Global.load_image(icon_hover_path)

        if (button.focus_cb ~= nil) then
            lv.obj.remove_event_cb(button.button, button.focus_cb)
        end

        if (button.defocus_cb ~= nil) then
            lv.obj.remove_event_cb(button.button, button.defocus_cb)
        end

        button.focus_cb = lv.obj.add_event_cb(button.button, function(event)

            lv.img.set_src(button.icon, button.img_hover_data)

        end, lv.EVENT_FOCUSED)

        button.defocus_cb = lv.obj.add_event_cb(button.button, function(event)

            lv.img.set_src(button.icon, button.img_data)

        end, lv.EVENT_DEFOCUSED)

        if (bit_and(lv.obj.get_state(button.button), lv.STATE_FOCUSED) > 0) then
            lv.img.set_src(button.icon, button.img_hover_data)
        end

    end

    lv.obj.invalidate(button.button)

end

function btn.create(parent, args)

    local button = {}

    button.button = lv.btn.new(parent)
    lv.obj.remove_style_all(button.button)

    button.label = lv.label.new(button.button)
    lv.obj.remove_style_all(button.label)
    lv.label.set_text(button.label, args.text)

    button.size = args.size

    if (args.icon ~= nil) then

        btn.set_icons(button, args.icon, args.icon_hover)

    end

    btn.set_theme(button, args.theme, args.size)

    return button

end

return btn