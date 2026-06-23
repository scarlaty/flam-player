--- This module il used to render a list with sound over, sound intro and image 
---@class dialog
local dialog = {}
--- Stores all lvgl style rules
dialog.styles = {}
--- Keep pointer on registered events for cleaning
dialog.events = {}
dialog.primaryButton = nil
dialog.secondaryButton = nil

dialog.button = require("button")
dialog.default_btn_theme = require("button-theme-default")
dialog.img = nil

function dialog.initStyles()
    dialog.styles.parentContainer = lv.style.new()
    lv.style.set_bg_color(dialog.styles.parentContainer, lv.color.hex(0x000000))
    lv.style.set_bg_opa(dialog.styles.parentContainer, lv.OPA_COVER)
    lv.style.set_pad_top(dialog.styles.parentContainer, 7)
    lv.style.set_pad_right(dialog.styles.parentContainer, 35)
    lv.style.set_pad_bottom(dialog.styles.parentContainer, 37)
    lv.style.set_pad_left(dialog.styles.parentContainer, 35)

    dialog.styles.titleLabel = lv.style.new()
    lv.style.set_text_color(dialog.styles.titleLabel, lv.color.hex(0xffffff))
    lv.style.set_text_font(dialog.styles.titleLabel, lv.font.nunito_bold_12)
    lv.style.set_text_align(dialog.styles.titleLabel, lv.TEXT_ALIGN_CENTER)
    
    dialog.styles.dialogMessage = lv.style.new()
    lv.style.set_text_color( dialog.styles.dialogMessage, lv.color.hex(0xffffff))
    lv.style.set_text_font( dialog.styles.dialogMessage, lv.font.nunito_extrabold_16)
    lv.style.set_text_align( dialog.styles.dialogMessage, lv.TEXT_ALIGN_CENTER)
    
end
function dialog.clean(args)
    for container, event in pairs( dialog.events) do
        lv.obj.remove_event_cb(container, event.clicked)
    end

    if (dialog.primaryButton ~= nil) then
        dialog.button.clean(dialog.primaryButton)
    end

    if (dialog.secondaryButton ~= nil) then
        dialog.button.clean(dialog.secondaryButton)
    end

    lv.group.remove_all_objs(document)
    lv.obj.clean(window)
    dialog.events = {}
    dialog.img = nil
end

function dialog.create(args)
    lv.group.set_editing(document, false)
    lv.group.set_wrap(document, true)

    lv.group.remove_all_objs(document)
    lv.obj.clean(window)

    dialog.initStyles()

    local parentContainer = require("v-container").create(window, 0, false)
    lv.obj.set_size(parentContainer, lv.obj.get_width(window), lv.obj.get_height(window))
    lv.obj.set_flex_align(parentContainer, lv.FLEX_ALIGN_CENTER, lv.FLEX_ALIGN_CENTER, lv.FLEX_ALIGN_CENTER)
    lv.obj.add_style(parentContainer, dialog.styles.parentContainer, lv.STATE_DEFAULT)


    if (args.img ~= nil) then
        local img = lv.img.new(parentContainer)
        lv.obj.remove_style_all(img)
        dialog.img  = Global.load_image(args.img)
        lv.img.set_src(img, dialog.img )
       -- lv.obj.set_size(img, 58, 60)
    end

    local titleLabel = lv.label.new(parentContainer)
    lv.obj.remove_style_all(titleLabel)
    lv.label.set_text(titleLabel, args.title)
    lv.label.set_long_mode(titleLabel, lv.LABEL_LONG_WRAP)
    lv.obj.set_width(titleLabel, 250)
    lv.obj.add_style(titleLabel,dialog.styles.titleLabel, lv.STATE_DEFAULT)

    local dialogMessage = lv.label.new(parentContainer)
    lv.obj.remove_style_all(dialogMessage)
    lv.label.set_text(dialogMessage, args.message)
    lv.label.set_long_mode(dialogMessage, lv.LABEL_LONG_WRAP)
    lv.obj.set_width(dialogMessage, 250)
    lv.obj.add_style(dialogMessage,  dialog.styles.dialogMessage, lv.STATE_DEFAULT)

    local buttons_container = require("h-container").create(parentContainer, 12, false)
    lv.obj.set_width(buttons_container, lv.obj.get_width(window))
    lv.obj.set_height(buttons_container, 34)
    lv.obj.set_style_translate_y(buttons_container, 12, lv.STATE_DEFAULT)
    lv.obj.set_flex_align(buttons_container, lv.FLEX_ALIGN_CENTER, lv.FLEX_ALIGN_CENTER, lv.FLEX_ALIGN_CENTER)

    if (args.primary_button_text ~= nil) then

        dialog.primaryButton = dialog.button.create(buttons_container, { text = args.primary_button_text, size = "large", theme = dialog.default_btn_theme })

        if (args.primary_button_cb ~= nil) then

            dialog.events[dialog.primaryButton.button] = {}
            dialog.events[dialog.primaryButton.button].clicked = lv.obj.add_event_cb(dialog.primaryButton.button, function()

              args.primary_button_cb()

            end, lv.EVENT_CLICKED)
        end

    end
    if (args.secondary_button_text ~= nil) then

        dialog.secondaryButton = dialog.button.create(buttons_container, { text = args.secondary_button_text, size = "large", theme = dialog.default_btn_theme })

        if (args.secondary_button_cb ~= nil) then
            dialog.events[dialog.secondaryButton.button] = {}
            dialog.events[dialog.secondaryButton.button].clicked = lv.obj.add_event_cb(dialog.secondaryButton.button, function()
                args.secondary_button_cb()
            end, lv.EVENT_CLICKED)
        end

    end

end 
return dialog
