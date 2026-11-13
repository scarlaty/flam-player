---@class titleScreen
local titleScreen = {}
titleScreen.styles = {}
titleScreen.img = nil
titleScreen.imgObject = nil
titleScreen.animations = {}
titleScreen.cb = nil
titleScreen.delayTimer = nil
titleScreen.animationDuration = 1000
titleScreen.audioPath = nil

function titleScreen.initStyles()
    titleScreen.styles.parent_style = lv.style.new()
    lv.style.set_bg_color(titleScreen.styles.parent_style, lv.color.hex(0x000000))
    lv.style.set_bg_opa(titleScreen.styles.parent_style, lv.OPA_COVER)

    titleScreen.styles.image_style = lv.style.new()
    lv.style.set_img_opa(titleScreen.styles.image_style, 0)
end

function titleScreen.removeAnimations()
    for anim, animVar in pairs(titleScreen.animations) do
        lv.anim_var.del(animVar.var)
    end
    titleScreen.animations = {}
end

function titleScreen.clean()
    lv.group.remove_all_objs(document)
    lv.obj.clean(window)

    titleScreen.removeAnimations()
    titleScreen.removeTimer()
    titleScreen.delayTimer = nil
    titleScreen.img        = nil
    titleScreen.imgObject  = nil
    titleScreen.audioPath  = nil
    titleScreen.styles     = {}
    titleScreen.cb         = nil
end

function titleScreen.removeTimer()
    if (titleScreen.delayTimer ~= nil) then
        lv.timer.del(titleScreen.delayTimer)
        titleScreen.delayTimer = nil
    end
end

function titleScreen.fadeAnimation(startVal, endVal, duration)
    titleScreen.removeAnimations()
    local fadeIn = lv.anim.new()
    titleScreen.animations[fadeIn] = {}
    titleScreen.animations[fadeIn].anim = fadeIn
    titleScreen.animations[fadeIn].var = lv.anim.set_var(fadeIn, titleScreen.imgObject)
    lv.anim.set_values(fadeIn, startVal, endVal)
    lv.anim.set_time(fadeIn, duration)
    lv.anim.set_exec_cb(fadeIn, function(var, val)
        lv.style.set_img_opa(titleScreen.styles.image_style, val)
        lv.obj.invalidate(titleScreen.imgObject)
    end)
    lv.anim.set_path_cb(fadeIn, lv.anim.path_ease_in_out)
    lv.anim.start(fadeIn)
end

function titleScreen.audioFeedback(audioState, seconds)
    if (audioState == "stop") then
        titleScreen.delayTimer = lv.timer.new(titleScreen.leaveCallback, titleScreen.animationDuration, nil)
        lv.timer.set_repeat_count(titleScreen.delayTimer, 1)
        titleScreen.fadeAnimation(255, 0, titleScreen.animationDuration)
    end
end

function titleScreen.startFadeInAnimation(animStart, animEnd)
    titleScreen.delayTimer = lv.timer.new(titleScreen.playAudio, titleScreen.animationDuration, nil)
    lv.timer.set_repeat_count(titleScreen.delayTimer, 1)
    titleScreen.fadeAnimation(animStart, animEnd, titleScreen.animationDuration)
end

function titleScreen.playAudio()
    Global.requestAudioPlay({ path = titleScreen.audioPath, AFCb = titleScreen.audioFeedback, priority = true })
    titleScreen.removeTimer()
end

function titleScreen.leaveCallback()
    titleScreen.removeTimer()
    titleScreen.cb()
end

function titleScreen.create(args)
    lv.obj.clean(window)
    titleScreen.initStyles()
    titleScreen.cb = args.cb
    local parent_container = lv.obj.new(window)
    lv.obj.remove_style_all(parent_container)
    lv.obj.set_size(parent_container, lv.obj.get_width(window), lv.obj.get_height(window))
    lv.obj.add_style(parent_container, titleScreen.styles.parent_style, lv.STATE_DEFAULT)

    if (args.anim_duration ~= nil) then
        titleScreen.animationDuration = args.anim_duration
    end

    if (args.image_path ~= nil) then
        local width, height
        titleScreen.img, width, height = Global.load_image(args.image_path)
        titleScreen.imgObject = lv.img.new(parent_container)
        lv.obj.remove_style_all(titleScreen.imgObject)
        lv.img.set_src(titleScreen.imgObject, titleScreen.img)
        lv.obj.set_size(titleScreen.imgObject, width, height)
        --lv.img.set_zoom(image,207) -- 256 = 100%
        lv.obj.align(titleScreen.imgObject, lv.ALIGN_CENTER, 0, 0)
        lv.obj.add_style(titleScreen.imgObject, titleScreen.styles.image_style, lv.STATE_DEFAULT)

        lv.group.add_obj(document, titleScreen.imgObject)
        lv.group.focus_obj(titleScreen.imgObject)
    end

    if (args.audio_path ~= nil) then
        titleScreen.audioPath = args.audio_path
        titleScreen.startFadeInAnimation(0, 255)
    end
end

return titleScreen
