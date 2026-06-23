local dialog = {}

local button = require("button")
local default_btn_theme = require("button-theme-default")

local styles

local parent_container

local img
local img_data

local title_label

local title_message

local buttons_container
local primary_button
local secondary_button
local go_back_cb

local primary_btn_clicked_cb
local secondary_btn_clicked_cb

local function clean()

    if (primary_btn_clicked_cb ~= nil) then
        lv.obj.remove_event_cb(primary_button.button, primary_btn_clicked_cb)
        primary_btn_clicked_cb = nil
    end

    if (secondary_btn_clicked_cb ~= nil) then
        lv.obj.remove_event_cb(secondary_button.button, secondary_btn_clicked_cb)
        secondary_btn_clicked_cb = nil
    end

    if (primary_button ~= nil) then
        button.clean(primary_button)
    end

    if (secondary_button ~= nil) then
        button.clean(secondary_button)
    end

    img_data = nil

    lv.group.remove_all_objs(document)
    lv.obj.clean(window)

    styles = {}
end

local function go_back_callback()
    if type(go_back_cb) == "function" then go_back_cb() end
end

function dialog.create(args)

    styles = {}

    go_back_cb = args.secondary_button_cb

    lv.group.remove_all_objs(document)
    lv.obj.clean(window)

    lv.group.set_editing(document, false)
    lv.group.set_wrap(document, true)

    --init parent container
    parent_container = require("v-container").create(window, 0, false)
    lv.obj.set_size(parent_container, lv.obj.get_width(window), lv.obj.get_height(window))
    lv.obj.set_flex_align(parent_container, lv.FLEX_ALIGN_CENTER, lv.FLEX_ALIGN_CENTER, lv.FLEX_ALIGN_CENTER)

    styles.parent_style = lv.style.new()
    lv.style.set_bg_color(styles.parent_style, lv.color.hex(0x000000))
    lv.style.set_bg_opa(styles.parent_style, lv.OPA_COVER)

    lv.style.set_pad_top(styles.parent_style, 7)
    lv.style.set_pad_right(styles.parent_style, 35)
    lv.style.set_pad_bottom(styles.parent_style, 37)
    lv.style.set_pad_left(styles.parent_style, 35)

    lv.obj.add_style(parent_container, styles.parent_style, lv.STATE_DEFAULT)

    if (args.img ~= nil) then
        img = lv.img.new(parent_container)
        lv.obj.remove_style_all(img)
        img_data, _, _ = Global.load_image(args.img)
        lv.img.set_src(img, img_data)
        lv.obj.set_size(img, 58, 60)
    end

    title_label = lv.label.new(parent_container)
    lv.obj.remove_style_all(title_label)
    lv.label.set_text(title_label, args.title)
    lv.label.set_long_mode(title_label, lv.LABEL_LONG_WRAP)
    lv.obj.set_width(title_label, 250)

    styles.title_style = lv.style.new()
    lv.style.set_text_color(styles.title_style, lv.color.hex(0xffffff))
    lv.style.set_text_font(styles.title_style, lv.font.nunito_bold_12)
    lv.style.set_text_align(styles.title_style, lv.TEXT_ALIGN_CENTER)
    lv.obj.add_style(title_label, styles.title_style, lv.STATE_DEFAULT)

    title_message = lv.label.new(parent_container)
    lv.obj.remove_style_all(title_message)
    lv.label.set_text(title_message, args.message)
    lv.label.set_long_mode(title_message, lv.LABEL_LONG_WRAP)
    lv.obj.set_width(title_message, 250)

    styles.message_style = lv.style.new()
    lv.style.set_text_color(styles.message_style, lv.color.hex(0xffffff))
    lv.style.set_text_font(styles.message_style, lv.font.nunito_extrabold_16)
    lv.style.set_text_align(styles.message_style, lv.TEXT_ALIGN_CENTER)
    lv.obj.add_style(title_message, styles.message_style, lv.STATE_DEFAULT)

    buttons_container = require("h-container").create(parent_container, 12, false)
    lv.obj.set_width(buttons_container, lv.obj.get_width(window))
    lv.obj.set_height(buttons_container, 34)
    lv.obj.set_style_translate_y(buttons_container, 12, lv.STATE_DEFAULT)
    lv.obj.set_flex_align(buttons_container, lv.FLEX_ALIGN_CENTER, lv.FLEX_ALIGN_CENTER, lv.FLEX_ALIGN_CENTER)

    if (args.primary_button_text ~= nil) then

        primary_button = button.create(buttons_container, { text = args.primary_button_text, size = "large", theme = default_btn_theme })

        if (args.primary_button_cb ~= nil) then

            primary_btn_clicked_cb = lv.obj.add_event_cb(primary_button.button, function()

              args.primary_button_cb()

            end, lv.EVENT_CLICKED)
        end

    end

    if (args.secondary_button_text ~= nil) then

        secondary_button = button.create(buttons_container, { text = args.secondary_button_text, size = "large", theme = default_btn_theme })

        if (go_back_cb ~= nil) then

            back_callback = go_back_callback

            secondary_btn_clicked_cb = lv.obj.add_event_cb(secondary_button.button, go_back_callback, lv.EVENT_CLICKED)
        end

    end



end

return dialog
