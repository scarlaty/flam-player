--- This module display specific item
--- - Show name/icon/description
--- - Can play audio for : description or use interactions
--- - Can override "use" audio and replace it with another audio or callback
---@class detail
local detail = {}

--- Store styles for all widget
detail.styles = {}

--- Store all buttons and their events
detail.events = {}

--- Store elapsed time of audio
detail.elapsed_time = 0
--- Store progression step of audio during foward/backward track
detail.progress_step_millis = 10

--- Current pressed button reference (if use_button is pressed, will store use_button reference)
detail.played_button = nil
--- Store audio state ("play", "pause", "stop")
detail.audio_state = "stop"
detail.img = nil
detail.mask = nil

--- Init styles for widgets
---@return nil
function detail.initStyles()
    detail.styles.parent_style = lv.style.new()
    lv.style.set_bg_color(detail.styles.parent_style, lv.color.hex(0x000000))
    lv.style.set_bg_opa(detail.styles.parent_style, lv.OPA_COVER)

    detail.styles.vertical_cont_style = lv.style.new()
    lv.style.set_bg_color(detail.styles.vertical_cont_style, lv.color.hex(0xff0000))
    lv.style.set_bg_opa(detail.styles.vertical_cont_style, lv.OPA_TRANSP)
    lv.style.set_pad_row(detail.styles.vertical_cont_style, 8)

    detail.styles.vertical_cont_scrolled_style = lv.style.new()
    lv.style.set_bg_color(detail.styles.vertical_cont_scrolled_style, lv.color.hex(0xf9fafa))
    lv.style.set_width(detail.styles.vertical_cont_scrolled_style, 4)
    lv.style.set_radius(detail.styles.vertical_cont_scrolled_style, 3)
    lv.style.set_bg_opa(detail.styles.vertical_cont_scrolled_style, lv.OPA_COVER)

    detail.styles.title_style = lv.style.new()
    lv.style.set_text_color(detail.styles.title_style, lv.color.hex(0xf2f4f5))
    lv.style.set_text_font(detail.styles.title_style, lv.font.nunito_extrabold_20)
    lv.style.set_text_line_space(detail.styles.title_style, 5)

    detail.styles.description_style = lv.style.new()
    lv.style.set_pad_top(detail.styles.description_style, 4)
    lv.style.set_text_color(detail.styles.description_style, lv.color.hex(0xffffff))
    lv.style.set_text_font(detail.styles.description_style, lv.font.nunito_extrabold_14)
    lv.style.set_text_line_space(detail.styles.description_style, 4)
    lv.style.set_pad_bottom(detail.styles.description_style, 8)

    detail.styles.data_btn_style = lv.style.new()
    lv.style.set_border_width(detail.styles.data_btn_style, 1)
    lv.style.set_border_side(detail.styles.data_btn_style, lv.BORDER_SIDE_TOP)
    lv.style.set_border_color(detail.styles.data_btn_style, lv.color.hex(0xA6B1B5))
    lv.style.set_border_opa(detail.styles.data_btn_style, lv.OPA_COVER)
    lv.style.set_pad_top(detail.styles.data_btn_style, 8)
    lv.style.set_pad_right(detail.styles.data_btn_style, 8)
    lv.style.set_pad_bottom(detail.styles.data_btn_style, 12)
    lv.style.set_pad_left(detail.styles.data_btn_style, 4)

    detail.styles.data_label_style = lv.style.new()
    lv.style.set_text_color(detail.styles.data_label_style, lv.color.hex(0xffffff))
    lv.style.set_text_font(detail.styles.data_label_style, lv.font.nunito_bold_12)

    detail.styles.data_value_style = lv.style.new()
    lv.style.set_text_color(detail.styles.data_value_style, lv.color.hex(0xffffff))
    lv.style.set_text_font(detail.styles.data_value_style, lv.font.nunito_bold_14)
    lv.style.set_text_align(detail.styles.data_value_style, lv.TEXT_ALIGN_CENTER)

    detail.styles.h_cont_style = lv.style.new()
    lv.style.set_bg_color(detail.styles.h_cont_style, lv.color.hex(0xff0000))
    lv.style.set_bg_opa(detail.styles.h_cont_style, lv.OPA_TRANSP)
    lv.style.set_pad_column(detail.styles.h_cont_style, 8)

    detail.styles.slider_indicator_style = lv.style.new()
    lv.style.set_bg_opa(detail.styles.slider_indicator_style, lv.OPA_COVER)
    lv.style.set_bg_color(detail.styles.slider_indicator_style, lv.color.hex(0xFBBD2F))
    lv.style.set_radius(detail.styles.slider_indicator_style, 2)

    detail.styles.knob_style = lv.style.new()
    lv.style.set_bg_opa(detail.styles.knob_style, lv.OPA_TRANSP)
    lv.style.set_border_width(detail.styles.knob_style, 0)
    lv.style.set_pad_all(detail.styles.knob_style, 0)

    detail.styles.slider_main_style = lv.style.new()
    lv.style.set_bg_opa(detail.styles.slider_main_style, lv.OPA_COVER)
    lv.style.set_bg_color(detail.styles.slider_main_style, lv.color.hex(0xa6b1b5))
    lv.style.set_radius(detail.styles.slider_main_style, 2)
    lv.style.set_pad_ver(detail.styles.slider_main_style, 0)

    detail.styles.slider_bars_style = lv.style.new()
    lv.style.set_bg_opa(detail.styles.slider_bars_style, lv.OPA_COVER)
    lv.style.set_bg_color(detail.styles.slider_bars_style, lv.color.hex(0xa6b1b5))
    lv.style.set_radius(detail.styles.slider_bars_style, 2)
    lv.style.set_pad_ver(detail.styles.slider_bars_style, 0)
end

--- Clean and reset module
---@return nil
function detail.clean()
    Global.requestAudioStop(true, true)
    detail.played_button = nil
    for _, entry in pairs(detail.events) do
        lv.obj.remove_event_cb(entry.button.button, entry.clicked)
        Global.button.clean(entry.button)
    end
    Global.v_scroll.clean()

    Global.requestAudioStop()
    detail.audio_state = "stop"

    lv.group.remove_all_objs(document)
    lv.obj.clean(window)

    detail.styles = {}
    detail.events = {}
    detail.img = nil
    detail.mask = nil
end

--- Audio callback function
---@return nil
function detail.audioFeedback(state, second)
    if (detail.played_button ~= nil) then
        if (state == "play") then
            detail.elapsed_time = detail.elapsed_time + detail.progress_step_millis

            local progress = 0
            if (audio.duration() ~= nil and audio.duration() ~= 0) then
                progress = 255 * (second / audio.duration())
            end

            lv.style.set_bg_main_stop(detail.played_button.button_style, math.floor(progress))
            lv.style.set_bg_grad_stop(detail.played_button.button_style, math.floor(progress))
            lv.obj.invalidate(detail.played_button.button)

            if (detail.audio_state ~= state) then
                Global.button.set_icons(detail.played_button, "script/play-18x18-ui-1000.lif",
                    "script/play-18x18-ui-1000.lif")
            end
        elseif (state == "pause" and detail.audio_state ~= state) then
            Global.button.set_icons(detail.played_button, "script/pause-18x18-brand-primary-900.lif",
                "script/pause-18x18-brand-primary-900.lif")
        elseif (state == "stop" and detail.audio_state ~= state) then
            Global.button.set_theme(detail.played_button, Global.button_theme_default, "small")
            Global.button.set_icons(detail.played_button, "script/play-18x18-ui-000.lif", "script/play-18x18-ui-1000.lif")
            lv.obj.invalidate(detail.played_button.button)
            detail.played_button = nil
            detail.elapsed_time = 0
        end
        detail.audio_state = state
    end
end

--- Common fuction for clicked button, play audio
---@return nil
function detail.buttonClicked(button_clicked, audio_path)
    if (detail.played_button ~= button_clicked) then
        --- Stop and reset button
        if (detail.audio_state ~= "stop") then
            Global.requestAudioStop(true, true)
        end

        if (detail.played_button ~= nil) then
            Global.button.set_theme(detail.played_button, Global.button_theme_default, "small")
            Global.button.set_icons(detail.played_button, "script/play-18x18-ui-000.lif", "script/play-18x18-ui-1000.lif")
            lv.obj.invalidate(detail.played_button.button)
        end

        --- Start play
        detail.elapsed_time = 0
        detail.audio_state = "stop"

        Global.requestAudioPlay({ path = audio_path, AFCb = detail.audioFeedback, priority = true, showPause = false })
        detail.played_button = button_clicked
        Global.button.set_theme(button_clicked, Global.button_theme_interactive, "small")
    end
end

-- Add data widget
---@return nil
function detail.addData(data, data_container, row_height, title)
    if (data.label == nil or data.value == nil or data.max == nil or data.type == nil) then
        print("detail.lua:192: warning: Bad format in data for item " .. title)
    else
        -- Not a real button but needed for scroll focus
        local data_button = lv.btn.new(data_container)
        lv.obj.remove_style_all(data_button)
        lv.group.remove_obj(data_button)
        lv.obj.set_width(data_button, 219)
        lv.obj.set_height(data_button, row_height)
        lv.obj.set_scroll_snap_y(data_button, lv.SCROLL_SNAP_END)
        lv.obj.add_style(data_button, detail.styles.data_btn_style, lv.STATE_DEFAULT)

        -- Init some containers for layout porpuse
        local h_container = lv.obj.new(data_button)
        lv.obj.remove_style_all(h_container)
        lv.obj.set_flex_flow(h_container, lv.FLEX_FLOW_ROW)
        lv.obj.set_width(h_container, 219)
        lv.obj.set_height(h_container, row_height)
        lv.obj.add_style(h_container, detail.styles.h_cont_style, lv.STATE_DEFAULT)
        lv.obj.set_style_pad_column(h_container, 8, lv.STATE_DEFAULT)

        local v_container = lv.obj.new(h_container)
        lv.obj.remove_style_all(v_container)
        lv.obj.set_flex_flow(v_container, lv.FLEX_FLOW_COLUMN)
        lv.obj.set_width(v_container, 150)
        lv.obj.set_height(v_container, 30)
        lv.obj.set_style_pad_row(v_container, 4, lv.STATE_DEFAULT)

        -- Init data label
        local data_label = lv.label.new(v_container)
        lv.obj.remove_style_all(data_label)
        lv.label.set_text(data_label, data.label)
        lv.obj.add_style(data_label, detail.styles.data_label_style, lv.STATE_DEFAULT)

        -- Init line/bars widget (depend of type value)
        if (data.type == nil or data.type == "line") then
            -- Add to state.invertory.objectName.data :
            -- {
            --     label = "Crédits",
            --     value = 30,
            --     max = 100,
            --     type = "line",
            --     color = 0xFFFFFF
            -- }

            local slider = detail.addLine(v_container, 150, detail.styles.slider_main_style, data.color)
            lv.slider.set_range(slider, 0, data.max)
            lv.slider.set_value(slider, data.value, lv.ANIM_OFF)
        elseif (data.type ~= nil and data.type == "bars") then
            -- Add to state.invertory.objectName.data :
            -- {
            --     label = "Niveau",
            --     value = 1,
            --     max = 5,
            --     type = "Bars",
            --     color = 0xFFFFFF
            -- }

            -- Bars type = many line type, init container to store all lines
            local bars_container = lv.obj.new(v_container)
            lv.obj.remove_style_all(bars_container)
            lv.obj.set_flex_flow(bars_container, lv.FLEX_FLOW_ROW)
            lv.obj.set_width(bars_container, 150)
            lv.obj.set_height(bars_container, row_height)
            lv.obj.add_style(bars_container, detail.styles.h_cont_style, lv.STATE_DEFAULT)
            lv.obj.set_style_pad_column(bars_container, 3, lv.STATE_DEFAULT)

            local barWidth = (150 - 3 * (data.max - 1)) // data.max
            if (barWidth < 1) then barWidth = 1 end

            -- Create all lines needed
            -- WARNING ! More than 11 bars will not be display correctly (not enough width)
            for i = 1, data.max, 1 do
                local slider = detail.addLine(bars_container, barWidth, detail.styles.slider_bars_style, data.color)
                lv.slider.set_range(slider, 0, 1)
                if (i <= data.value) then
                    lv.slider.set_value(slider, 1, lv.ANIM_OFF)
                else
                    lv.slider.set_value(slider, 0, lv.ANIM_OFF)
                end
            end
        else
            print("detail.lua:268: warning: Bad format (must be \"line\" or \"bar\") in data.type for item " ..
            title .. " data " .. data.label)
        end

        -- Init label for data value
        local text_container = lv.obj.new(h_container)
        lv.obj.remove_style_all(text_container)
        lv.obj.set_flex_flow(text_container, lv.FLEX_FLOW_ROW)
        lv.obj.set_width(text_container, 50)
        lv.obj.set_height(text_container, 50)
        lv.obj.set_flex_align(text_container, lv.FLEX_ALIGN_CENTER, lv.FLEX_ALIGN_CENTER, lv.FLEX_ALIGN_CENTER)

        local data_value_label = lv.label.new(text_container)
        lv.obj.remove_style_all(data_value_label)
        lv.obj.set_width(data_value_label, 50)
        lv.obj.set_height(data_value_label, 30)
        -- For line value is printed as "N"
        if (data.type == nil or data.type == "line") then
            if (data.override ~= nil) then
                lv.label.set_text(data_value_label, data.override)
            else
                lv.label.set_text(data_value_label, data.value .. data.unit)
            end

            -- For bars value is printed as "N/MAX"
            -- WARNING ! More than 2 digits will not be display correctly
        elseif (data.type ~= nil and data.type == "bars") then
            lv.label.set_text(data_value_label, data.value .. "/" .. data.max .. data.unit)
        end
        lv.obj.add_style(data_value_label, detail.styles.data_value_style, lv.STATE_DEFAULT)
    end
end

-- Add data line widget
-- @param   parent: reference parent widget
--          width: int widget width in pixel
--          style: reference style to use (slider_main_style or slider_bars_style)
---@return nil
function detail.addLine(parent, width, style, colorOverride)
    local slider = lv.slider.new(parent)
    lv.obj.remove_style_all(slider)
    lv.obj.set_size(slider, width, 8)
    lv.obj.add_style(slider, style, lv.PART_MAIN)
    lv.obj.add_style(slider, detail.styles.slider_indicator_style, lv.PART_INDICATOR)
    lv.obj.add_style(slider, detail.styles.knob_style, lv.PART_KNOB)

    if (colorOverride ~= nil) then
        lv.obj.set_style_bg_color(slider, lv.color.hex(colorOverride), lv.PART_INDICATOR)
    end
    return slider
end

-- @param args:table
--        {
--          item:table item from inventory
--          [
--              title:string item title,
--              description:string descrition text,
--              audio_description:string (optional) path to audio when description button is pressed,
--              audio_use:string (optional) path to audio when use button is pressed,
--              img:string item img path,
--              data:table (optional) item data use for display more informations like levels/money/etc
--              [
--                  label:string data name
--                  value:int data value
--                  max:int data max value
--                  type:string type of display ("line" or "bars")
--                  unit:string to add at the end of the value
--              ]
--          ],
--          callback_previous:function function to call when back button is pressed,
--          callback_args:table (optional) data used by callback_previous if needed,
--          overrideAudio:string (optional) audio path that will override item.audio_use (exclude overrideCallback),
--          overrideCallback:function (optional) function to call when use button is pressed (exclude by overrideAudio)
--        }
---@return nil
function detail.create(args)
    if (args == nil) then print("detail.lua:329: error: No args !") end
    if (args.item == nil) then print("detail.lua:330: error: No item set in args !") end

    local data = args.item
    local hideUse = true
    if (args.hideUse ~= nil) then hideUse = args.hideUse end

    detail.events = {}
    detail.styles = {}

    local audio_use_path = data.audio_use
    if (args.overrideAudio ~= nil) then
        audio_use_path = args.overrideAudio
    end
    local audio_description_path = data.audio_description

    lv.group.remove_all_objs(document)
    lv.obj.clean(window)

    lv.group.set_wrap(document, false)

    detail.initStyles()

    -- Init main container
    local parent_container = lv.obj.new(window)
    lv.obj.remove_style_all(parent_container)
    lv.obj.set_size(parent_container, lv.obj.get_width(window), lv.obj.get_height(window))
    lv.obj.add_style(parent_container, detail.styles.parent_style, lv.STATE_DEFAULT)

    -- Image back
    if (data.detailImg ~= nil) then
        local img = lv.img.new(parent_container)
        lv.obj.remove_style_all(img)
        lv.obj.add_flag(img, lv.OBJ_FLAG_FLOATING)
        lv.obj.align(img, lv.ALIGN_TOP_LEFT, 0, 0)
        local width, height
        detail.img, width, height = Global.load_image(data.detailImg)
        lv.img.set_src(img, detail.img)
        lv.obj.set_size(img, width, height)

        local image_mask = lv.img.new(parent_container)
        detail.mask = Global.load_image("script/mask-154x213.lif")
        lv.obj.add_flag(image_mask, lv.OBJ_FLAG_FLOATING)
        lv.obj.remove_style_all(image_mask)
        lv.img.set_src(image_mask, detail.mask)
        lv.obj.align(image_mask, lv.ALIGN_TOP_LEFT, 0, 0)
        lv.obj.set_size(image_mask, 154, 213)
    else
        print("detail.lua:376: warning: No image \"detailImg\" are set for item " .. data.title .. "!")
    end

    -- Init item container (text + buttons + data)
    local vertical_container = lv.obj.new(parent_container)
    lv.obj.remove_style_all(vertical_container)
    lv.obj.set_flex_flow(vertical_container, lv.FLEX_FLOW_COLUMN)
    lv.obj.set_size(vertical_container, 227, lv.obj.get_height(window) - 13)
    lv.obj.set_pos(vertical_container, 89, 13)
    lv.obj.add_style(vertical_container, detail.styles.vertical_cont_style, lv.STATE_DEFAULT)
    lv.obj.add_style(vertical_container, detail.styles.vertical_cont_scrolled_style, lv.PART_SCROLLBAR);

    -- Init title label
    local title_label = lv.label.new(vertical_container)
    lv.obj.remove_style_all(title_label)
    lv.obj.set_width(title_label, 219)
    lv.label.set_long_mode(title_label, lv.LABEL_LONG_WRAP)
    if (data.title ~= nil) then lv.label.set_text(title_label, data.title) else print(
        "detail.lua:393: warning: No title for item ?") end
    lv.group.focus_obj(title_label)
    lv.obj.add_style(title_label, detail.styles.title_style, lv.STATE_DEFAULT)

    -- Init button fullscreen, need detailImg or img
    if (data.hasFullscreen ~= nil and data.hasFullscreen) then
        -- Init fullscreen button
        local fullscreen_button = Global.button.create(vertical_container,
            { text = "Voir", size = "small", theme = Global.button_theme_default })
        Global.v_scroll.add_focusable_obj(fullscreen_button.button)

        detail.events[fullscreen_button] = {}
        detail.events[fullscreen_button].button = fullscreen_button

        -- Init clicked callback event (play audio)
        detail.events[fullscreen_button].clicked = lv.obj.add_event_cb(fullscreen_button.button, function()
            local cbArgs = {
                name = "detail",
                version = "",
                args = args
            }
            Global.setBackModule(cbArgs)

            local argsFullscreen = {}
            if (data.fullscreenImage ~= nil) then
                argsFullscreen.image_path = data.fullscreenImage
            else
                argsFullscreen.image_path = data.detailImg
            end
            Global.load_module("full-screen-image", "").display(argsFullscreen)
        end, lv.EVENT_CLICKED)
    end

    -- Init use button, need audio_use path or overrideCallback function
    if ((hideUse == false) and (audio_use_path ~= nil or args.overrideCallback ~= nil)) then
        -- Init use button
        local use_button = Global.button.create(vertical_container,
            {
                text = "Utiliser",
                size = "small",
                icon = "script/play-18x18-ui-000.lif",
                icon_hover = "script/play-18x18-ui-1000.lif",
                theme = Global.button_theme_default
            })

        Global.v_scroll.add_focusable_obj(use_button.button)

        detail.events[use_button] = {}
        detail.events[use_button].button = use_button

        -- If overrideCallback is set, call function on click
        if (args.overrideCallback ~= nil) then
            -- Init clicked callback event (clean view and call function)
            detail.events[use_button].clicked = lv.obj.add_event_cb(use_button.button, function()
                local call = args.overrideCallback
                Global.cleanCurrentModule()
                call()
            end, lv.EVENT_CLICKED)

            -- If audio_use is set, play audio
        elseif (audio_use_path ~= nil) then
            -- Init clicked callback event (play audio)
            detail.events[use_button].clicked = lv.obj.add_event_cb(use_button.button, function()
                detail.buttonClicked(use_button, audio_use_path)
            end, lv.EVENT_CLICKED)
        end
    elseif ((hideUse == false) and (audio_use_path == nil and data.overrideCallback == nil)) then
        print("detail.lua:453: warning: Use can't be initialize, audio_use or overrideCallback are missing for item " ..
        data.title)
    end

    -- Init description label
    if (data.description ~= nil) then
        local description_label = lv.label.new(vertical_container)
        lv.obj.remove_style_all(description_label)
        lv.obj.set_width(description_label, 219)
        lv.label.set_long_mode(description_label, lv.LABEL_LONG_WRAP)
        lv.label.set_text(description_label, data.description)
        lv.group.remove_obj(description_label)
        lv.obj.add_style(description_label, detail.styles.description_style, lv.STATE_DEFAULT)
    end

    -- Init audio description button
    if (data.audio_description ~= nil) then
        -- Init button
        local description_button = Global.button.create(vertical_container,
            {
                text = "Écouter la description",
                size = "small",
                icon = "script/play-18x18-ui-000.lif",
                icon_hover = "script/play-18x18-ui-1000.lif",
                theme = Global.button_theme_default
            })

        Global.v_scroll.add_focusable_obj(description_button.button)

        detail.events[description_button] = {}
        detail.events[description_button].button = description_button

        -- Init clicked callback event (play audio)
        detail.events[description_button].clicked = lv.obj.add_event_cb(description_button.button, function()
            detail.buttonClicked(description_button, audio_description_path)
        end, lv.EVENT_CLICKED)
    end

    -- Init data widgets
    if (data.data ~= nil) then
        local row_height = 50

        -- Init data container
        local data_container = lv.obj.new(vertical_container)
        lv.obj.remove_style_all(data_container)
        lv.obj.set_style_pad_top(data_container, 4, lv.STATE_DEFAULT)
        lv.obj.set_flex_flow(data_container, lv.FLEX_FLOW_COLUMN)
        lv.obj.set_width(data_container, 227)
        lv.obj.set_height(data_container, #data.data * row_height)

        -- Foreach data in item
        for _, v in ipairs(data.data) do
            detail.addData(v, data_container, row_height, data.title)
        end -- end for
    end     -- end if data

    Global.v_scroll.init(parent_container, vertical_container)

    if (args.backModule ~= nil) then
        Global.setBackModule(args.backModule)
    elseif (args.backBehavior ~= nil) then
        Global.setBackBehavior(args.backBehavior)
    end
end

return detail
