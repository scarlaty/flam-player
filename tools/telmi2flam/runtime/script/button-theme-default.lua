local theme = require("button-theme"):new()

function theme:get_default_style(size)

    local style = lv.style.new()

    lv.style.set_bg_color(style, lv.color.hex(0xffffff))
    lv.style.set_bg_opa(style, lv.OPA_TRANSP)
    lv.style.set_border_width(style, 2)
    lv.style.set_border_color(style, lv.color.hex(0xffffff))
    lv.style.set_border_opa(style, lv.OPA_COVER)
    lv.style.set_radius(style, 17)

    lv.style.set_text_color(style, lv.color.hex(0xffffff))

    if (size == "small") then

        lv.style.set_pad_top(style, 2)
        lv.style.set_pad_right(style, 8)
        lv.style.set_pad_bottom(style, 2)
        lv.style.set_pad_left(style, 8)

        lv.style.set_text_font(style, lv.font.nunito_extrabold_12)
    else

        lv.style.set_pad_top(style, 3)
        lv.style.set_pad_right(style, 10)
        lv.style.set_pad_bottom(style, 3)
        lv.style.set_pad_left(style, 10)

        lv.style.set_text_font(style, lv.font.nunito_extrabold_16)
    end

    return style

end

function theme:get_focused_style(size)

    local style = lv.style.new()

    lv.style.set_bg_opa(style, lv.OPA_COVER)
    lv.style.set_bg_color(style, lv.color.hex(0xFFFFFF))
    lv.style.set_border_color(style, lv.color.hex(0xFFFFFF))
    lv.style.set_text_color(style, lv.color.hex(0x000000))

    return style

end

function theme:get_pressed_style(size)

    local style = lv.style.new()

    lv.style.set_bg_opa(style, lv.OPA_COVER)
    lv.style.set_bg_color(style, lv.color.hex(0xFBBD2F))
    lv.style.set_border_color(style, lv.color.hex(0xFBBD2F))
    lv.style.set_text_color(style, lv.color.hex(0x4F3B0C))

    return style

end

return theme