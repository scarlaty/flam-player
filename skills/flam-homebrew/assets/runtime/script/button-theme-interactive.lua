local theme = require("button-theme"):new()

function theme:get_default_style(size)

    local button_style = lv.style.new()

    lv.style.set_border_width(button_style, 2)
    lv.style.set_border_opa(button_style, lv.OPA_TRANSP)
    lv.style.set_bg_opa(button_style, lv.OPA_COVER)
    lv.style.set_radius(button_style, 16)
    lv.style.set_text_color(button_style, lv.color.hex(0x000000))
    lv.style.set_bg_grad_dir(button_style, lv.GRAD_DIR_HOR)
    lv.style.set_bg_dither_mode(button_style, lv.DITHER_NONE)
    lv.style.set_bg_main_stop(button_style, 0)
    lv.style.set_bg_grad_stop(button_style, 0)
    lv.style.set_bg_color(button_style, lv.color.hex(0xfbbd2f))
    lv.style.set_bg_grad_color(button_style, lv.color.hex(0xffffff))

    if (size == "small") then

        lv.style.set_pad_top(button_style, 2)
        lv.style.set_pad_right(button_style, 8)
        lv.style.set_pad_bottom(button_style, 2)
        lv.style.set_pad_left(button_style, 8)

        lv.style.set_text_font(button_style, lv.font.nunito_extrabold_12)
    else

        lv.style.set_pad_top(button_style, 3)
        lv.style.set_pad_right(button_style, 10)
        lv.style.set_pad_bottom(button_style, 3)
        lv.style.set_pad_left(button_style, 10)

        lv.style.set_text_font(button_style, lv.font.nunito_extrabold_16)
    end

    return button_style

end

function theme:get_focused_style(size)

    local button_focused_style = lv.style.new()

    --lv.style.set_bg_opa(button_focused_style, lv.OPA_COVER)
    --lv.style.set_bg_color(button_focused_style, lv.color.hex(0xffffff))
    --lv.style.set_text_color(button_focused_style, lv.color.hex(0x000000))

    return button_focused_style

end

function theme:get_pressed_style(size)

    local button_pressed_style = lv.style.new()

    --lv.style.set_bg_opa(button_pressed_style, lv.OPA_COVER)
    --lv.style.set_bg_color(button_pressed_style, lv.color.hex(0xFBBD2F))
    --lv.style.set_border_color(button_pressed_style, lv.color.hex(0xFBBD2F))
    --lv.style.set_text_color(button_pressed_style, lv.color.hex(0x4F3B0C))

    return button_pressed_style

end

return theme