--- @class imageButton
local imageButton = {}

imageButton.styles = {}
--- Keep pointer on registered events for cleaning
imageButton.events = {}
-- keep pointer on registered animation for cleaning
imageButton.animations = {}

imageButton.texts = {}
imageButton.images = {}
imageButton.buttons = {}
imageButton.cb = nil
imageButton.hasAudioFadeout = false
imageButton.leaveTimer = nil

function imageButton.initStyles()
    imageButton.styles.parentStyle = lv.style.new()
    lv.style.set_bg_color(imageButton.styles.parentStyle, lv.color.hex(0x000000))
    lv.style.set_bg_opa(imageButton.styles.parentStyle, lv.OPA_COVER)

    imageButton.styles.image = lv.style.new()
    lv.style.set_img_opa(imageButton.styles.image ,255)

    imageButton.styles.bonusImage = lv.style.new()
    lv.style.set_img_opa(imageButton.styles.bonusImage ,255)
end

function imageButton.removeAnimations()
    for anim, animVar in pairs(imageButton.animations) do
        lv.anim_var.del(animVar.var)
    end
end

function imageButton.clean()
    for container, event in pairs( imageButton.events) do
        if(event.clicked ~= nil)then  lv.obj.remove_event_cb(container, event.clicked)  end
    end

    imageButton.removeAnimations()

    if (imageButton.leaveTimer ~= nil) then
        lv.timer.del(imageButton.leaveTimer)
        imageButton.leaveTimer = nil
    end

    -- lv.obj.remove_style_all(imageButton.images.centralDice.object)
    -- lv.obj.remove_style_all(imageButton.parentContainer)
    lv.group.set_editing(document, false)
    lv.group.remove_all_objs(document)
    lv.obj.clean(window)
    
    imageButton.styles = {}
    imageButton.texts = {}
    imageButton.images = {}
    imageButton.buttons = {}
    imageButton.events = {}
    imageButton.animations = {}
    imageButton.cb = nil

end
function imageButton.pulseAnimation()
    imageButton.removeAnimations()
    local pulseAnimation = lv.anim.new()
    imageButton.animations[pulseAnimation] = {}
    imageButton.animations[pulseAnimation].anim = pulseAnimation
    imageButton.animations[pulseAnimation].var = lv.anim.set_var(pulseAnimation,  imageButton.images.centralDice.object)
    lv.anim.set_values(pulseAnimation, 0, 10)
    lv.anim.set_time(pulseAnimation, 500)
    lv.anim.set_playback_time(pulseAnimation, 500);
    lv.anim.set_repeat_count(pulseAnimation, 100);
    lv.anim.set_early_apply(pulseAnimation, true);

    lv.anim.set_exec_cb(pulseAnimation, function(var, val)
        lv.img.set_zoom( imageButton.images.centralDice.object, 245 + val) -- 256 = 100%
    end)
    lv.anim.set_path_cb(pulseAnimation, lv.anim.path_ease_in_out)
    lv.anim.start(pulseAnimation)
end
function imageButton.fadeOutAnimation(animDuration)
    imageButton.removeAnimations()

    local fadeOut = lv.anim.new()
    imageButton.animations[fadeOut] = {}
    imageButton.animations[fadeOut].anim = fadeOut
    imageButton.animations[fadeOut].var = lv.anim.set_var(fadeOut,  imageButton.images.centralDice.object)
    lv.anim.set_values(fadeOut, 255, 0)
    lv.anim.set_time(fadeOut, animDuration)
    lv.anim.set_exec_cb(fadeOut, function(var, val)
        lv.style.set_img_opa(imageButton.styles.image ,val)
        lv.obj.invalidate(imageButton.images.centralDice.object)
    end)
    lv.anim.set_path_cb(fadeOut, lv.anim.path_ease_in_out)
    lv.anim.start(fadeOut)
    lv.obj.add_flag(imageButton.buttons.validate.button,lv.OBJ_FLAG_HIDDEN)
end
function imageButton.audioFeedback(state,seconds)
    if(state == "stop") then imageButton.cb() end
end
function imageButton.create(args)
    lv.group.set_editing(document, true)
    lv.obj.clean(window) -- Clean main window
    imageButton.initStyles()

    imageButton.parentContainer = lv.obj.new(window)
    lv.obj.remove_style_all(imageButton.parentContainer)
    lv.obj.set_size(imageButton.parentContainer, lv.obj.get_width(window), lv.obj.get_height(window))
    lv.obj.add_style(imageButton.parentContainer, imageButton.styles.parentStyle, lv.STATE_DEFAULT)

    imageButton.images.centralDice = {}
    imageButton.images.centralDice.object = lv.img.new(imageButton.parentContainer)
    lv.obj.remove_style_all(imageButton.images.centralDice.object)
    lv.obj.add_flag(imageButton.images.centralDice.object, lv.OBJ_FLAG_FLOATING)
    lv.obj.align(imageButton.images.centralDice.object, lv.ALIGN_CENTER, 0, 0)
    --lv.obj.set_size(imageButton.images.centralDice.object, 150, 150)
    imageButton.images.centralDice.data = Global.load_image(args.image)
    lv.img.set_src(imageButton.images.centralDice.object, imageButton.images.centralDice.data)
    lv.obj.add_style(imageButton.images.centralDice.object, imageButton.styles.image, lv.STATE_DEFAULT)
    imageButton.pulseAnimation()



    imageButton.buttons.validate = Global.button.create(imageButton.parentContainer,
        { text = args.buttonText, size = "large", theme = Global.button_theme_default })
    lv.obj.align( imageButton.buttons.validate.button, lv.ALIGN_BOTTOM_MID, 0, -15)

    imageButton.events[imageButton.buttons.validate.button] = {}
    imageButton.cb = args.cb
    imageButton.events[imageButton.buttons.validate.button].clicked  = lv.obj.add_event_cb(imageButton.buttons.validate.button,function()
        if(args.audioButton ~= nil) then
            Global.requestAudioPlay({path = args.audioButton, AFCb = imageButton.audioFeedback, priority = true})
            imageButton.fadeOutAnimation(500)
        else
            imageButton.leaveTimer = lv.timer.new(imageButton.cb, 500, nil)
            lv.timer.set_repeat_count(imageButton.leaveTimer, 1)
            imageButton.fadeOutAnimation(500)
        end
       
        for container, event in pairs( imageButton.events) do
            if(event.clicked ~= nil)then  lv.obj.remove_event_cb(container, event.clicked)  end
        end
        imageButton.events = {}

        end, lv.EVENT_CLICKED)


    if(args.audio ~= nil) then
        Global.requestAudioPlay({path = args.audio, priority = true})
    end
end

return imageButton
