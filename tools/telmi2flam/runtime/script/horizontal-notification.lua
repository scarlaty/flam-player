--- @class horizontalNotification
local horizontalNotification = {}

horizontalNotification.styles = {}
--- Keep pointer on registered events for cleaning
horizontalNotification.events = {}
-- keep pointer on registered animation for cleaning
horizontalNotification.animations = {}

horizontalNotification.texts = {}
horizontalNotification.images = {}
horizontalNotification.buttons = {}
horizontalNotification.cb = nil
horizontalNotification.hasAudioFadeout = false
horizontalNotification.leaveTimer = nil

function horizontalNotification.initStyles()
    horizontalNotification.styles.parentStyle = lv.style.new()
    lv.style.set_bg_color(horizontalNotification.styles.parentStyle, lv.color.hex(0x000000))
    lv.style.set_bg_opa(horizontalNotification.styles.parentStyle, lv.OPA_COVER)

    horizontalNotification.styles.image = lv.style.new()
    lv.style.set_img_opa(horizontalNotification.styles.image, 255)
    lv.style.set_pad_top(horizontalNotification.styles.image, 8)

    horizontalNotification.styles.upTitle = lv.style.new()
    lv.style.set_text_color(horizontalNotification.styles.upTitle, lv.color.hex(0xA6B1B5))
    lv.style.set_text_font(horizontalNotification.styles.upTitle, lv.font.nunito_bold_12)
    lv.style.set_pad_top(horizontalNotification.styles.upTitle, 12)
    -- lv.style.set_pad_bottom(horizontalNotification.styles.upTitle, 20)

    horizontalNotification.styles.title = lv.style.new()
    lv.style.set_text_color(horizontalNotification.styles.title, lv.color.hex(0xF2F4F5))
    lv.style.set_text_font(horizontalNotification.styles.title, lv.font.nunito_extrabold_16)
    lv.style.set_pad_top(horizontalNotification.styles.title, 4)
end

function horizontalNotification.removeAnimations()
    for anim, animVar in pairs(horizontalNotification.animations) do
        lv.anim_var.del(animVar.var)
    end
end

function horizontalNotification.clean()
    for container, event in pairs(horizontalNotification.events) do
        if (event.clicked ~= nil) then lv.obj.remove_event_cb(container, event.clicked) end
    end

    horizontalNotification.removeAnimations()

    if (horizontalNotification.leaveTimer ~= nil) then
        lv.timer.del(horizontalNotification.leaveTimer)
        horizontalNotification.leaveTimer = nil
    end

    lv.group.set_editing(document, false)
    lv.group.remove_all_objs(document)
    lv.obj.clean(window)

    horizontalNotification.styles = {}
    horizontalNotification.texts = {}
    horizontalNotification.images = {}
    horizontalNotification.buttons = {}
    horizontalNotification.events = {}
    horizontalNotification.animations = {}
    horizontalNotification.cb = nil
end

function horizontalNotification.pulseAnimation()
    horizontalNotification.removeAnimations()
    local pulseAnimation = lv.anim.new()
    horizontalNotification.animations[pulseAnimation] = {}
    horizontalNotification.animations[pulseAnimation].anim = pulseAnimation
    horizontalNotification.animations[pulseAnimation].var = lv.anim.set_var(pulseAnimation,
        horizontalNotification.images.centralImage.object)
    lv.anim.set_values(pulseAnimation, 0, 10)
    lv.anim.set_time(pulseAnimation, 500)
    lv.anim.set_playback_time(pulseAnimation, 500);
    lv.anim.set_repeat_count(pulseAnimation, lv.ANIM_REPEAT_INFINITE);
    lv.anim.set_early_apply(pulseAnimation, true);

    lv.anim.set_exec_cb(pulseAnimation, function(var, val)
        lv.img.set_zoom(horizontalNotification.images.centralImage.object, 245 + val) -- 256 = 100%
    end)
    lv.anim.set_path_cb(pulseAnimation, lv.anim.path_ease_in_out)
    lv.anim.start(pulseAnimation)
end

function horizontalNotification.rotateAnimation()
    horizontalNotification.removeAnimations()
    local rotateAnimation = lv.anim.new()
    horizontalNotification.animations[rotateAnimation] = {}
    horizontalNotification.animations[rotateAnimation].anim = rotateAnimation
    horizontalNotification.animations[rotateAnimation].var = lv.anim.set_var(rotateAnimation,
        horizontalNotification.images.centralImage.object)
    lv.anim.set_values(rotateAnimation, 1, 80)
    lv.anim.set_time(rotateAnimation, 500)
    lv.anim.set_playback_time(rotateAnimation, 500);
    lv.anim.set_repeat_count(rotateAnimation, lv.ANIM_REPEAT_INFINITE);
    lv.anim.set_early_apply(rotateAnimation, true);

    lv.anim.set_exec_cb(rotateAnimation, function(var, val)
        lv.img.set_angle(horizontalNotification.images.centralImage.object, val)
    end)
    lv.anim.set_path_cb(rotateAnimation, lv.anim.path_ease_in_out)
    lv.anim.start(rotateAnimation)
end

function horizontalNotification.fadeOutAnimation(animDuration)
    horizontalNotification.removeAnimations()

    local fadeOut = lv.anim.new()
    horizontalNotification.animations[fadeOut] = {}
    horizontalNotification.animations[fadeOut].anim = fadeOut
    horizontalNotification.animations[fadeOut].var = lv.anim.set_var(fadeOut, horizontalNotification.texts.title)
    lv.anim.set_values(fadeOut, 255, 0)
    lv.anim.set_time(fadeOut, animDuration)
    lv.anim.set_exec_cb(fadeOut, function(var, val)
        lv.style.set_img_opa(horizontalNotification.styles.image, val)
        lv.style.set_text_opa(horizontalNotification.styles.upTitle, val)
        lv.style.set_text_opa(horizontalNotification.styles.title, val)
        if (horizontalNotification.images.centralImage ~= nil) then
            lv.obj.invalidate(horizontalNotification
                .images.centralImage.object)
        end
        if (horizontalNotification.texts.upTitle ~= nil) then lv.obj.invalidate(horizontalNotification.texts.upTitle) end
        lv.obj.invalidate(horizontalNotification.texts.title)
    end)
    lv.anim.set_path_cb(fadeOut, lv.anim.path_ease_in_out)
    lv.anim.start(fadeOut)
    lv.obj.add_flag(horizontalNotification.buttons.validate.button, lv.OBJ_FLAG_HIDDEN)
end

function horizontalNotification.audioFeedback(state, seconds)
    if (state == "stop") then horizontalNotification.cb() end
end

function horizontalNotification.create(args)
    lv.group.set_editing(document, true)
    lv.obj.clean(window) -- Clean main window
    horizontalNotification.initStyles()


    horizontalNotification.parentContainer = lv.obj.new(window)
    lv.obj.remove_style_all(horizontalNotification.parentContainer)
    lv.obj.set_size(horizontalNotification.parentContainer, lv.obj.get_width(window), lv.obj.get_height(window) - 40)
    lv.obj.set_flex_flow(horizontalNotification.parentContainer, lv.FLEX_FLOW_COLUMN)
    if (args.image ~= nil) then
        lv.obj.set_flex_align(horizontalNotification.parentContainer, lv.FLEX_ALIGN_START, lv.FLEX_ALIGN_CENTER,
            lv.FLEX_ALIGN_CENTER)
    else
        lv.obj.set_flex_align(horizontalNotification.parentContainer, lv.FLEX_ALIGN_CENTER, lv.FLEX_ALIGN_CENTER,
            lv.FLEX_ALIGN_CENTER)
    end

    lv.obj.add_style(horizontalNotification.parentContainer, horizontalNotification.styles.parentStyle, lv.STATE_DEFAULT)

    if (args.image ~= nil) then
        horizontalNotification.images.centralImage = {}
        horizontalNotification.images.centralImage.object = lv.img.new(horizontalNotification.parentContainer)
        lv.obj.remove_style_all(horizontalNotification.images.centralImage.object)
        horizontalNotification.images.centralImage.data = Global.load_image(args.image)
        lv.img.set_src(horizontalNotification.images.centralImage.object, horizontalNotification.images.centralImage
            .data)
        lv.obj.add_style(horizontalNotification.images.centralImage.object, horizontalNotification.styles.image,
            lv.STATE_DEFAULT)
        if (args.anim == 1) then
            horizontalNotification.rotateAnimation()
        elseif (args.anim == 2) then
            horizontalNotification.pulseAnimation()
        end
    end

    if (args.upText ~= nil) then
        horizontalNotification.texts.upTitle = lv.label.new(horizontalNotification.parentContainer)
        lv.obj.set_style_text_align(horizontalNotification.texts.upTitle, lv.TEXT_ALIGN_CENTER, 0);
        lv.obj.set_width(horizontalNotification.texts.upTitle, lv.obj.get_width(window) - 20)
        lv.label.set_long_mode(horizontalNotification.texts.upTitle, lv.LABEL_LONG_WRAP)
        lv.label.set_text(horizontalNotification.texts.upTitle, args.upText)
        lv.obj.add_style(horizontalNotification.texts.upTitle, horizontalNotification.styles.upTitle, lv.STATE_DEFAULT)
    end


    horizontalNotification.texts.title = lv.label.new(horizontalNotification.parentContainer)
    lv.obj.set_style_text_align(horizontalNotification.texts.title, lv.TEXT_ALIGN_CENTER, 0);
    lv.obj.set_width(horizontalNotification.texts.title, lv.obj.get_width(window) - 20)
    lv.label.set_long_mode(horizontalNotification.texts.title, lv.LABEL_LONG_WRAP)

    lv.label.set_text(horizontalNotification.texts.title, args.title)
    lv.obj.add_style(horizontalNotification.texts.title, horizontalNotification.styles.title, lv.STATE_DEFAULT)


    horizontalNotification.buttons.validate = Global.button.create(
        window,
        { text = args.buttonText, size = "large", theme = Global.button_theme_default })
    lv.obj.add_flag(horizontalNotification.buttons.validate.button, lv.OBJ_FLAG_FLOATING)
    lv.obj.align(horizontalNotification.buttons.validate.button, lv.ALIGN_BOTTOM_MID, 0, -10)
    if (args.audioOutDuration == nil) then args.audioOutDuration = 200 end
    horizontalNotification.events[horizontalNotification.buttons.validate.button]         = {}
    horizontalNotification.cb                                                             = args.cb
    horizontalNotification.events[horizontalNotification.buttons.validate.button].clicked = lv.obj.add_event_cb(
        horizontalNotification.buttons.validate.button, function()
            if (args.audioButton ~= nil) then
                Global.requestAudioPlay({ path = args.audioButton, AFCb = horizontalNotification.audioFeedback, priority = true })
                horizontalNotification.fadeOutAnimation(args.audioOutDuration)
            else
                horizontalNotification.leaveTimer = lv.timer.new(horizontalNotification.cb, args.audioOutDuration, nil)
                lv.timer.set_repeat_count(horizontalNotification.leaveTimer, 1)
                horizontalNotification.fadeOutAnimation(args.audioOutDuration)
            end

            for container, event in pairs(horizontalNotification.events) do
                if (event.clicked ~= nil) then lv.obj.remove_event_cb(container, event.clicked) end
            end
            horizontalNotification.events = {}
        end, lv.EVENT_CLICKED)


    if (args.audio ~= nil) then
        Global.requestAudioPlay({ path = args.audio, priority = true })
    end
end

return horizontalNotification
