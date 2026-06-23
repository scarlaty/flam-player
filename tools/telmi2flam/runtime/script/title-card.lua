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
        --lv.anim_var.del(titleCard.opacityAnim)
        titleCard.opacityAnimVar = nil
        titleCard.opacityAnim = nil
    end

    if (titleCard.translateAnim ~= nil) then
        lv.anim_var.del(titleCard.translateAnimVar)
        --lv.anim_var.del(titleCard.translateAnim)
        titleCard.translateAnimVar = nil
        titleCard.translateAnim = nil
    end
    if (titleCard.closeAnim ~= nil) then
        lv.anim_var.del(titleCard.closeAnimVar)
        --lv.anim_var.del(titleCard.closeAnim)
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
        back_callback()
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
        back_callback()
        -- if( titleCard.closeAnim == nil) then
        --     titleCard.closeAnim = lv.anim.new()
        --     titleCard.closeAnimVar = lv.anim.set_var(titleCard.closeAnim, nil)
        --     lv.anim.set_values(titleCard.closeAnim, 255, 0)
        --     lv.anim.set_time(titleCard.closeAnim, 1000)
        --     lv.anim.set_exec_cb(titleCard.closeAnim, titleCard.close_transition)
        --     lv.anim.set_path_cb(titleCard.closeAnim, lv.anim.path_ease_in_out)
        --     lv.anim.start(titleCard.closeAnim)
        -- end
    end
end

function titleCard.on_encoder_clicked()
    if( titleCard.isAudioStarted) then
        Global.requestAudioStop(true)
    end
end

function titleCard.display(args)

    titleCard.callback = args.cb

    lv.obj.clean(window)

    lv.group.set_editing(document, false)
    lv.group.set_wrap(document, false)
    titleCard.initSyles()

    --init parent container
    titleCard.parentContainer = require("v-container").create(window, 8, true)
    lv.obj.remove_style_all(titleCard.parentContainer)
    lv.obj.set_size(titleCard.parentContainer, lv.obj.get_width(window), lv.obj.get_height(window))
    lv.obj.set_flex_flow(titleCard.parentContainer, lv.FLEX_FLOW_COLUMN)
    lv.obj.add_style(titleCard.parentContainer, titleCard.styles.parentContainer, lv.STATE_DEFAULT)


    titleCard.event_clicked_cb = lv.obj.add_event_cb(titleCard.parentContainer, titleCard.on_encoder_clicked, lv.EVENT_CLICKED)


    titleCard.subtitleLabel = lv.label.new(titleCard.parentContainer)
    lv.obj.remove_style_all(titleCard.subtitleLabel)
    lv.obj.set_width(titleCard.subtitleLabel, 214)
    lv.label.set_text(titleCard.subtitleLabel, args.subtitle)
    lv.obj.add_style(titleCard.subtitleLabel, titleCard.styles.subtitle, lv.STATE_DEFAULT)

    titleCard.titleLabel = lv.label.new(titleCard.parentContainer)
    lv.obj.remove_style_all(titleCard.titleLabel)
    lv.obj.set_width(titleCard.titleLabel, 214)
    lv.label.set_text(titleCard.titleLabel, args.title)
    lv.obj.add_style(titleCard.titleLabel, titleCard.styles.title, lv.STATE_DEFAULT)

    titleCard.image = lv.img.new(window)
    lv.obj.remove_style_all(titleCard.image)
    titleCard.img = Global.load_image(args.img)
    lv.img.set_src(titleCard.image, titleCard.img)
    lv.obj.add_flag(titleCard.image, lv.OBJ_FLAG_FLOATING)
    --lv.obj.set_size(titleCard.image, 89, 120)
    lv.obj.set_style_translate_x(titleCard.image, 231, lv.STATE_DEFAULT)
    lv.obj.set_style_translate_y(titleCard.image, 92, lv.STATE_DEFAULT)


    titleCard.opacityAnim= lv.anim.new()
    titleCard.opacityAnimVar = lv.anim.set_var(titleCard.opacityAnim, nil)
    lv.anim.set_values(titleCard.opacityAnim, 0, 255)
    lv.anim.set_time(titleCard.opacityAnim, 500)
    lv.anim.set_exec_cb(titleCard.opacityAnim, titleCard.text_opacity_transition)
    lv.anim.set_path_cb(titleCard.opacityAnim, lv.anim.path_ease_in_out)
    lv.anim.start(titleCard.opacityAnim)

    titleCard.translateAnim = lv.anim.new()
    titleCard.translateAnimVar = lv.anim.set_var(titleCard.translateAnim, nil)
    lv.anim.set_values(titleCard.translateAnim, 320, Global.visual_width - 116)
    lv.anim.set_time(titleCard.translateAnim, 1500)
    lv.anim.set_exec_cb(titleCard.translateAnim, titleCard.translate_img)
    lv.anim.set_path_cb(titleCard.translateAnim, lv.anim.path_ease_in_out)
    lv.anim.start(titleCard.translateAnim)

    
    Global.requestAudioPlay({path = args.audio,AFCb = titleCard.audio_feedback})

    --lv_anim_set_playback_delay(&a, delay);
    args= nil
end



return titleCard