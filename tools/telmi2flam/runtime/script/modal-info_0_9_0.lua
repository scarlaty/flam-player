local modal_info = {}

--local global = require("global")
local button = require("button")
local button_theme_default = require("button-theme-default")
local button_theme_interactive = require("button-theme-interactive")

local styles

local parent_container

local title_button
local title_label

local text_container

local message_button
local primary_button
local secondary_button
local image
local image_data
local image_mask
local image_mask_data

local button_clicked_callback
local secondary_button_clicked_callback

local stop_progress = false
local progress_stopped = false

local elapsed_time = 0
local jump_to_next_delay = 0

local progress_step_millis = 100

local callback_primary
local callback_data_primary

local callback_secondary
local callback_data_secondary


local secondary_btn_focused_cb

local audio_state
local elapsed_time = 0
local progress_step_millis = 10

local audio_duration = 1

local exiting = false

local autoSkipTimer = nil
local autoSkipDuration = nil
local autoSkipFunction = nil

function modal_info.clean()
    if (button_clicked_callback ~= nil) then
        lv.obj.remove_event_cb(primary_button.button, button_clicked_callback)
        button_clicked_callback = nil
    end

    if (secondary_btn_focused_cb ~= nil) then
        lv.obj.remove_event_cb(secondary_button.button, secondary_btn_focused_cb)
        secondary_btn_focused_cb = nil
    end

    if (secondary_button_clicked_callback ~= nil) then
        lv.obj.remove_event_cb(secondary_button.button, secondary_button_clicked_callback)
        secondary_button_clicked_callback = nil
    end

    if (autoSkipTimer ~= nil) then
        lv.timer.del(autoSkipTimer)
        autoSkipTimer = nil
    end

    if (audio.get_status() ~= "stop") then
        exiting = true
        audio.stop()
    end

    button.clean(primary_button)
    button.clean(secondary_button)
    lv.obj.del(message_button)

    image = nil
    primary_button = nil
    secondary_button = nil
    message_button = nil

    image_data = nil
    image_mask = nil
    image_mask_data = nil

    parent_container = nil
    title_button = nil
    title_label = nil

    text_container = nil

    stop_progress = true

    -- Object memory is managed by lvgl, then ask lvgl to destroy it
    lv.group.remove_all_objs(document)
    lv.obj.clean(window)

    styles = nil
end

local function exit_primary()
    if (audio.get_status() == "play") then
        audio.stop()
    end
    callback_primary(callback_data_primary)
end

local function exit_secondary()
    if (audio.get_status() == "play") then
        audio.stop()
    end
    callback_secondary(callback_data_secondary)
end

local function button_clicked_cb(event)
    exit_primary()
end

local function secondary_button_clicked_cb(event)
    exit_secondary()
end

local function autoSkipCallback()
    if (audio.get_status() == "play") then
        audio.stop()
    end
    autoSkipFunction()
end

local function audio_feedback(state, second)
    if (exiting == true) then return end

    if (state == "stop") then
        if (autoSkipDuration ~= nil) then
            autoSkipTimer = lv.timer.new(autoSkipCallback, autoSkipDuration, nil)
        end
    end

    if (stop_progress == true) then
        if (progress_stopped == false) then
            progress_stopped = true
        end

        return
    end

    if (state == "play") then
        elapsed_time = elapsed_time + progress_step_millis

        lv.style.set_bg_main_stop(primary_button.button_style, 1)
        lv.style.set_bg_grad_stop(primary_button.button_style, 1)
        lv.obj.invalidate(primary_button.button)
    elseif (state == "stop") then
        if (exiting == true) then return end -- CAN THIS HAPPEN?

        if (audio_state ~= state) then
            if (primary_button ~= nil) then
                button.set_theme(primary_button, button_theme_default, "large")
                lv.obj.invalidate(primary_button.button)
            end
        end
    end

    audio_state = state
end


local function on_secondary_btn_focused(event)
    stop_progress = true

    button.set_theme(primary_button, button_theme_default, large)
    lv.obj.invalidate(primary_button.button)
end


function modal_info.create(args)
    styles = {}

    exiting = false

    stop_progress = false
    progress_stopped = false

    lv.group.set_editing(document, false)

    if (args.autoSkipData ~= nil) then
        autoSkipDuration = args.autoSkipData.autoSkipDelay
        autoSkipFunction = args.autoSkipData.autoSkipCb
    end

    callback_primary = args.primary_button_cb
    callback_data_primary = args.primary_button_cb_data
    callback_secondary = args.secondary_button_cb
    callback_data_secondary = args.secondary_button_cb_data
    elapsed_time = 0

    lv.obj.clean(window)

    parent_container = lv.obj.new(window)
    lv.obj.remove_style_all(parent_container)
    lv.obj.set_size(parent_container, lv.obj.get_width(window), lv.obj.get_height(window))

    styles.parent_style = lv.style.new()
    lv.style.set_bg_color(styles.parent_style, lv.color.hex(0x000000))
    lv.style.set_bg_opa(styles.parent_style, lv.OPA_COVER)

    lv.obj.add_style(parent_container, styles.parent_style, lv.STATE_DEFAULT)

    if (args.image_path ~= nil) then
        image = lv.img.new(parent_container)
        image_data = Global.load_image(args.image_path)
        lv.obj.add_flag(image, lv.OBJ_FLAG_FLOATING)
        lv.obj.remove_style_all(image)
        lv.img.set_src(image, image_data)
        lv.obj.set_size(image, 154, 213)
        lv.obj.align(image, lv.ALIGN_TOP_LEFT, 0, 0)

        image_mask = lv.img.new(parent_container)
        image_mask_data = Global.load_image("script/mask-154x213.lif")
        lv.obj.add_flag(image_mask, lv.OBJ_FLAG_FLOATING)
        lv.obj.remove_style_all(image_mask)
        lv.img.set_src(image_mask, image_mask_data)
        lv.obj.set_size(image_mask, 154, 213)
        lv.obj.align(image_mask, lv.ALIGN_TOP_LEFT, 0, 0)
    end

    styles.button_focused_style = lv.style.new()

    styles.vertical_cont_style = lv.style.new()
    lv.style.set_bg_color(styles.vertical_cont_style, lv.color.hex(0xff0000))
    lv.style.set_bg_opa(styles.vertical_cont_style, lv.OPA_TRANSP)
    lv.style.set_pad_row(styles.vertical_cont_style, 8)

    styles.vertical_cont_scrolled_style = lv.style.new()
    lv.style.set_bg_color(styles.vertical_cont_scrolled_style, lv.color.hex(0xf9fafa))
    lv.style.set_width(styles.vertical_cont_scrolled_style, 4)
    lv.style.set_radius(styles.vertical_cont_scrolled_style, 3)
    lv.style.set_bg_opa(styles.vertical_cont_scrolled_style, lv.OPA_COVER)

    text_container = lv.obj.new(parent_container)
    lv.obj.remove_style_all(text_container)
    lv.obj.set_pos(text_container, 89, 18)
    lv.obj.set_size(text_container, 227, lv.obj.get_height(window) - 13)
    lv.obj.set_flex_flow(text_container, lv.FLEX_FLOW_COLUMN)
    lv.obj.set_style_pad_bottom(text_container, 34, lv.STATE_DEFAULT)
    lv.obj.add_style(text_container,styles.vertical_cont_style,lv.STATE_DEFAULT)
    lv.obj.add_style(text_container,styles.vertical_cont_scrolled_style, lv.PART_SCROLLBAR);

    if (args.title ~= nil) then
        title_button = lv.btn.new(text_container)
        lv.obj.remove_style_all(title_button)

        title_label = lv.label.new(title_button)
        lv.obj.remove_style_all(title_label)
        lv.label.set_text(title_label, args.title)
        lv.obj.set_width(title_label, 219)

        styles.title_style = lv.style.new()
        lv.style.set_text_color(styles.title_style, lv.color.hex(0xffffff))
        lv.style.set_text_font(styles.title_style, lv.font.nunito_bold_20)

        lv.obj.add_style(title_label, styles.title_style, lv.STATE_DEFAULT)


        lv.obj.add_style(title_button, styles.button_focused_style, lv.PART_MAIN | lv.STATE_FOCUSED)
    end

    if (args.message ~= nil) then
        message_button = lv.btn.new(text_container)
        lv.obj.remove_style_all(message_button)

        lv.group.remove_obj(message_button)

        local message_label = lv.label.new(message_button)
        lv.obj.remove_style_all(message_label)
        lv.label.set_text(message_label, args.message)
        lv.obj.set_width(message_label, 219)

        styles.message_style = lv.style.new()

        lv.style.set_bg_color(styles.message_style, lv.color.hex(0x000000))
        lv.style.set_bg_opa(styles.message_style, lv.OPA_TRANSP)
        lv.style.set_text_color(styles.message_style, lv.color.hex(0xffffff))
        lv.style.set_text_font(styles.message_style, lv.font.nunito_bold_16)

        lv.obj.add_style(message_label, styles.message_style, lv.STATE_DEFAULT)

        lv.obj.add_style(message_button, styles.button_focused_style, lv.PART_MAIN | lv.STATE_FOCUSED)
    end

    primary_button = button.create(text_container,
        { text = args.primary_button_text, size = "large", theme = button_theme_default })

    lv.group.remove_obj(primary_button.button)
    lv.obj.set_style_translate_y(primary_button.button, 20, lv.STATE_DEFAULT)

    if (args.secondary_button_text ~= nil) then
        secondary_button = button.create(text_container,
            { text = args.secondary_button_text, size = "large", theme = button_theme_default })

        lv.group.remove_obj(secondary_button.button)
        lv.obj.set_scroll_snap_y(secondary_button.button, lv.SCROLL_SNAP_END)

        secondary_btn_focused_cb = lv.obj.add_event_cb(secondary_button.button, on_secondary_btn_focused,
            lv.EVENT_FOCUSED)

        lv.obj.set_style_translate_y(secondary_button.button, 25, lv.STATE_DEFAULT)
    end

    button_clicked_callback = lv.obj.add_event_cb(primary_button.button, button_clicked_cb, lv.EVENT_CLICKED)

    if (secondary_button ~= nil) then
        secondary_button_clicked_callback = lv.obj.add_event_cb(secondary_button.button, secondary_button_clicked_cb,
            lv.EVENT_CLICKED)
    end

    lv.group.add_obj(document, primary_button.button)

    if (secondary_button ~= nil) then
        lv.group.add_obj(document, secondary_button.button)
    end

    if (args.audio ~= nil) then
        Global.requestAudioPlay({ path = args.audio, AFCb = audio_feedback, priority = true, showPause = true })
        elapsed_time = 0
        --button.set_theme(primary_button, button_theme_interactive, "large")
    else
        print("Unable to play audio: " .. args.audio)
    end
end

return modal_info
