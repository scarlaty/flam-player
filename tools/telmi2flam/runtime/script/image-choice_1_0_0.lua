--- This module il used to render a list with sound over, sound intro and image
---@class imageChoice
local imageChoice = {}
--- Stores all lvgl style rules
imageChoice.styles = {}
--- Keep pointer on registered events for cleaning
imageChoice.events = {}
-- keep pointer on registered animation for cleaning
imageChoice.animations = {}
-- store images data
imageChoice.answers = {}
imageChoice.answerIterator = 1
imageChoice.parentContainer = nil
imageChoice.renderedImage = nil
imageChoice.renderedLabel = nil
imageChoice.renderedData = nil

imageChoice.keyEvent = nil
imageChoice.inputProcessTimer = nil
imageChoice.tick = 0

function imageChoice.initStyle()
    imageChoice.styles.imageLabel = lv.style.new()
    lv.style.set_bg_color(imageChoice.styles.imageLabel, lv.color.black())
    lv.style.set_bg_opa(imageChoice.styles.imageLabel, lv.OPA_COVER)
    lv.style.set_text_color(imageChoice.styles.imageLabel, lv.color.hex(0xefedea))
    lv.style.set_text_font(imageChoice.styles.imageLabel, lv.font.nunito_extrabold_16)
    lv.style.set_pad_top(imageChoice.styles.imageLabel, 4)
    lv.style.set_pad_right(imageChoice.styles.imageLabel, 8)
    lv.style.set_pad_bottom(imageChoice.styles.imageLabel, 4)
    lv.style.set_pad_left(imageChoice.styles.imageLabel, 8)
    lv.style.set_radius(imageChoice.styles.imageLabel, 5)

    imageChoice.styles.parentContainer = lv.style.new()
    lv.style.set_pad_hor(imageChoice.styles.parentContainer, 0)
    lv.style.set_pad_ver(imageChoice.styles.parentContainer, 0)
    lv.style.set_bg_color(imageChoice.styles.parentContainer, lv.color.hex(0x000000))
    lv.style.set_bg_opa(imageChoice.styles.parentContainer, lv.OPA_COVER)
end

function imageChoice.clean()
    Global.requestAudioStop(true, true)
    if (imageChoice.inputProcessTimer ~= nil) then
        lv.timer.del(imageChoice.inputProcessTimer)
        imageChoice.inputProcessTimer = nil
    end
    for container, event in pairs(imageChoice.events) do
        lv.obj.remove_event_cb(container, event.key)
    end
    for anim, animVar in pairs(imageChoice.animations) do
        lv.anim_var.del(animVar.var)
    end

    lv.group.set_editing(document, false)
    lv.group.remove_all_objs(document)
    lv.obj.clean(window)

    imageChoice.answerIterator = 1
    imageChoice.parentContainer = nil
    imageChoice.renderedImage = nil
    imageChoice.renderedLabel = nil
    imageChoice.keyEvent = nil
    imageChoice.renderedData = nil
    imageChoice.styles = {}
    imageChoice.events = {}
    imageChoice.animations = {}
    imageChoice.answers = {}
end

function imageChoice.diplayAnswers()
    lv.group.add_obj(document, imageChoice.parentContainer)
    local width
    imageChoice.renderedData, width, _ = Global.load_image(imageChoice.answers[imageChoice.answerIterator].img)
    lv.img.set_src(imageChoice.renderedImage, imageChoice.renderedData)
    lv.obj.remove_style_all(imageChoice.renderedImage)
    lv.obj.align(imageChoice.renderedImage, lv.ALIGN_TOP_MID, 0, 0)
    lv.obj.set_size(imageChoice.renderedImage, width, Global.visual_height)
    Global.requestAudioPlay({ path = imageChoice.answers[imageChoice.answerIterator].audio })

    if (imageChoice.answers[imageChoice.answerIterator].label ~= nil) then
        imageChoice.renderedLabel = lv.label.new(imageChoice.parentContainer)
        lv.obj.remove_style_all(imageChoice.renderedLabel)
        lv.obj.set_width(imageChoice.renderedLabel, 296)
        lv.label.set_long_mode(imageChoice.renderedLabel, lv.LABEL_LONG_SCROLL_CIRCULAR)
        lv.label.set_text(imageChoice.renderedLabel, imageChoice.answers[imageChoice.answerIterator].label)
        lv.obj.clear_flag(imageChoice.renderedLabel, lv.OBJ_FLAG_SCROLLABLE)
        lv.obj.clear_flag(imageChoice.renderedLabel, lv.OBJ_FLAG_CLICK_FOCUSABLE)
        lv.obj.align(imageChoice.renderedLabel, lv.ALIGN_BOTTOM_MID, 0, -4)
        lv.obj.set_style_text_align(imageChoice.renderedLabel, lv.TEXT_ALIGN_CENTER, 0)
        lv.obj.add_style(imageChoice.renderedLabel, imageChoice.styles.imageLabel, lv.STATE_DEFAULT)

        local labelAnimation = lv.anim.new()
        imageChoice.animations[labelAnimation] = {}
        imageChoice.animations[labelAnimation].var = lv.anim.set_var(labelAnimation, nil)
        imageChoice.animations[labelAnimation].duration = 300
        lv.anim.set_values(labelAnimation, 0, -4)
        lv.anim.set_time(labelAnimation, imageChoice.animations[labelAnimation].duration)
        lv.anim.set_exec_cb(labelAnimation, function(_, val)
            lv.obj.set_y(imageChoice.renderedLabel, val)
        end)
        lv.anim.set_path_cb(labelAnimation, lv.anim.path_ease_in_out)
        lv.anim.start(labelAnimation)
    end
end

function imageChoice.audioFeedback(state, second)
    if (state == "stop") then
        imageChoice.diplayAnswers()
    end
end

function imageChoice.processKeyEvent()
    if (imageChoice.tick <= 1 and imageChoice.keyEvent ~= nil) then
        imageChoice.keyEvent = nil
        imageChoice.tick = imageChoice.tick + 1
    end
    if (imageChoice.keyEvent ~= nil) then
        if (string.byte(imageChoice.keyEvent) == 20 or string.byte(imageChoice.keyEvent) == 19) then
            if (string.byte(imageChoice.keyEvent) == 20) then
                imageChoice.answerIterator = imageChoice.answerIterator - 1
                if (imageChoice.answerIterator < 1) then
                    imageChoice.answerIterator = #imageChoice.answers
                end
            else
                imageChoice.answerIterator = imageChoice.answerIterator + 1
                if (imageChoice.answerIterator > #imageChoice.answers) then
                    imageChoice.answerIterator = 1
                end
            end
        end
        if (string.byte(imageChoice.keyEvent) == 10) then
            if (imageChoice.answers[imageChoice.answerIterator].cb ~= nil) then
                imageChoice.answers[imageChoice.answerIterator].cb()
            end
        else
            if (imageChoice.answers[imageChoice.answerIterator].data ~= nil) then
                lv.img.set_src(imageChoice.renderedImage, imageChoice.answers[imageChoice.answerIterator].data)
                lv.obj.remove_style_all(imageChoice.renderedImage)
                lv.obj.align(imageChoice.renderedImage, lv.ALIGN_TOP_MID, 0, 0)
                lv.obj.set_size(imageChoice.renderedImage, imageChoice.answers[imageChoice.answerIterator].width,
                    Global.visual_height)
            else
                local width
                imageChoice.renderedData, width, _ = Global.load_image(imageChoice.answers[imageChoice.answerIterator]
                    .img)
                lv.img.set_src(imageChoice.renderedImage, imageChoice.renderedData)
                lv.obj.remove_style_all(imageChoice.renderedImage)
                lv.obj.align(imageChoice.renderedImage, lv.ALIGN_TOP_MID, 0, 0)
                lv.obj.set_size(imageChoice.renderedImage, width, Global.visual_height)
            end
            if (imageChoice.renderedLabel ~= nil) then
                if (imageChoice.answers[imageChoice.answerIterator].label ~= nil) then
                    lv.label.set_text(imageChoice.renderedLabel, imageChoice.answers[imageChoice.answerIterator].label)
                else
                    lv.label.set_text(imageChoice.renderedLabel, " ")
                end
            end

            local priority = true
            if (imageChoice.answers[imageChoice.answerIterator].priority ~= nil) then
                priority = imageChoice.answers[imageChoice.answerIterator].priority
            end
            Global.requestAudioPlay({ path = imageChoice.answers[imageChoice.answerIterator].audio, priority = priority })
            imageChoice.keyEvent = nil
        end
    end
end

function imageChoice.keyPressed(e)
    lv.timer.reset(imageChoice.inputProcessTimer)
    imageChoice.keyEvent = lv.event.get_key_value(e)
end

function imageChoice.create(args)
    lv.obj.clean(window)
    lv.group.set_editing(document, true)
    imageChoice.initStyle()

    --init parent container
    imageChoice.parentContainer = lv.obj.new(window)
    lv.obj.remove_style_all(imageChoice.parentContainer)
    lv.obj.set_size(imageChoice.parentContainer, lv.obj.get_width(window), lv.obj.get_height(window))
    lv.obj.add_style(imageChoice.parentContainer, imageChoice.styles.parentContainer, lv.STATE_DEFAULT)

    imageChoice.events[imageChoice.parentContainer] = {}
    imageChoice.events[imageChoice.parentContainer].key = lv.obj.add_event_cb(imageChoice.parentContainer,
        imageChoice.keyPressed, lv.EVENT_KEY)

    local i = 1
    local lastCb = nil
    for it, v in ipairs(args.choices) do
        if v.show ~= false then
            table.insert(imageChoice.answers, v)
            if (args.preload == true) then
                imageChoice.answers[i].data, imageChoice.answers[i].width, _ = Global.load_image(imageChoice.answers[i]
                    .img)
            end
            i = i + 1
            lastCb = v.cb
        end
    end

    if (#imageChoice.answers == 0) then
        return args.exitCb()
    elseif #imageChoice.answers == 1 and lastCb ~= nil and args.skipIfLastChoice and args.skipIfLastChoice == true then
        return lastCb()
    end

    local width, height

    if (args.title_image ~= nil) then
        imageChoice.renderedData, width, height = Global.load_image(args.title_image)
    else
        imageChoice.renderedData, width, height = Global.load_image(imageChoice.answers[imageChoice.answerIterator].img)
    end

    imageChoice.renderedImage = lv.img.new(imageChoice.parentContainer)
    lv.img.set_src(imageChoice.renderedImage, imageChoice.renderedData)
    lv.obj.remove_style_all(imageChoice.renderedImage)
    lv.obj.align(imageChoice.renderedImage, lv.ALIGN_TOP_MID, 0, 0)
    lv.obj.set_size(imageChoice.renderedImage, width, Global.visual_height)
    if (args.title_audio ~= nil) then
        Global.requestAudioPlay({ path = args.title_audio, AFCb = imageChoice.audioFeedback })
    else
        imageChoice.diplayAnswers()
    end
    imageChoice.inputProcessTimer = lv.timer.new(imageChoice.processKeyEvent, 100, nil)
end

return imageChoice
