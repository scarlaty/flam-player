---@class imagePanoramic
local imagePanoramic = {}
imagePanoramic.styles = {}
imagePanoramic.events = {}
imagePanoramic.img = nil
imagePanoramic.imgObject = nil
imagePanoramic.mask = nil
imagePanoramic.maskObject = nil
imagePanoramic.cb = nil
imagePanoramic.parentContainer = nil
imagePanoramic.imageContainer = nil
imagePanoramic.imgxOffset = 0
imagePanoramic.imgWidth = nil
imagePanoramic.stepSize = 10

function imagePanoramic.initStyles()
    imagePanoramic.styles.parentContainer = lv.style.new()
    lv.style.set_bg_color(imagePanoramic.styles.parentContainer, lv.color.hex(0x000000))
    lv.style.set_bg_opa(imagePanoramic.styles.parentContainer, lv.OPA_COVER)

    imagePanoramic.styles.img = lv.style.new()
    --lv.style.set_pad_left(imagePanoramic.styles.img, -100)
    -- lv.style.set_img_opa(imagePanoramic.styles.img, 1)

    imagePanoramic.styles.mask = lv.style.new()
    -- lv.style.set_img_opa(imagePanoramic.styles.mask, 1)
end

function imagePanoramic.clean()
    for container, event in pairs(imagePanoramic.events) do
        lv.obj.remove_event_cb(container, event.key)
    end
    lv.group.set_editing(document, false)
    lv.group.remove_all_objs(document)
    lv.obj.clean(window)

    imagePanoramic.styles = {}
    imagePanoramic.events = {}
    imagePanoramic.img = nil
    imagePanoramic.imgObject = nil
    imagePanoramic.mask = nil
    imagePanoramic.maskObject = nil
    imagePanoramic.cb = nil
    imagePanoramic.parentContainer = nil
    imagePanoramic.interactionEnabled = false
end

function imagePanoramic.keyPressed(e)
    --print(string.byte(lv.event.get_key_value(e)))
    if (imagePanoramic.interactionEnabled) then
        if (string.byte(lv.event.get_key_value(e)) == 10) then
            Global.requestAudioStop(true, true)
            imagePanoramic.cb()
        else
            if (string.byte(lv.event.get_key_value(e)) == 20) then
                if (imagePanoramic.imgxOffset <= imagePanoramic.stepSize * -1) then
                    imagePanoramic.imgxOffset = imagePanoramic.imgxOffset + imagePanoramic.stepSize
                end
            elseif (string.byte(lv.event.get_key_value(e)) == 19) then
                if (imagePanoramic.imgxOffset > ((imagePanoramic.imgWidth - lv.obj.get_width(window)) * -1) - imagePanoramic.stepSize) then
                    imagePanoramic.imgxOffset = imagePanoramic.imgxOffset - imagePanoramic.stepSize
                end
            end
            -- print(imagePanoramic.imgxOffset)
            lv.obj.set_style_translate_x(imagePanoramic.imgObject, imagePanoramic.imgxOffset, lv.STATE_DEFAULT)
            lv.obj.invalidate(imagePanoramic.imgObject)
        end
    end
end

function imagePanoramic.audioCallback(state, seconds)
    if (state == "stop") then
        imagePanoramic.interactionEnabled = true
    end
end

function imagePanoramic.create(args)
    lv.group.set_editing(document, true)
    lv.obj.clean(window)
    imagePanoramic.initStyles()
    if (args.cb ~= nil) then imagePanoramic.cb = args.cb else print("No CB found") end
    if (args.stepSize ~= nil) then imagePanoramic.stepSize = args.stepSize end
    if (args.startPos ~= nil) then imagePanoramic.imgxOffset = args.startPos * -1 end
    if (args.interactionEnabled ~= nil) then imagePanoramic.interactionEnabled = args.interactionEnabled end
    imagePanoramic.parentContainer = lv.obj.new(window)
    lv.obj.remove_style_all(imagePanoramic.parentContainer)
    lv.obj.set_size(imagePanoramic.parentContainer, lv.obj.get_width(window), lv.obj.get_height(window))
    lv.obj.add_style(imagePanoramic.parentContainer, imagePanoramic.styles.parentContainer, lv.STATE_DEFAULT)

    if (args.image_path ~= nil) then
        local imgHeight
        imagePanoramic.img, imagePanoramic.imgWidth, imgHeight = Global.load_image(args.image_path)
        imagePanoramic.imgObject = lv.img.new(imagePanoramic.parentContainer)
        lv.obj.remove_style_all(imagePanoramic.imgObject)
        lv.obj.add_flag(imagePanoramic.imgObject, lv.OBJ_FLAG_FLOATING)
        lv.img.set_src(imagePanoramic.imgObject, imagePanoramic.img)
        lv.obj.set_size(imagePanoramic.imgObject, imagePanoramic.imgWidth, imgHeight)
        lv.obj.align(imagePanoramic.imgObject, lv.ALIGN_TOP_LEFT, 0, 0)
        lv.obj.add_style(imagePanoramic.imgObject, imagePanoramic.styles.img, lv.STATE_DEFAULT)

        lv.group.remove_obj(imagePanoramic.imgObject)
    else
        print("no image found")
    end
    if (args.mask_path ~= nil) then
        local width, height
        imagePanoramic.mask, width, height = Global.load_image(args.mask_path)
        imagePanoramic.maskObject = lv.img.new(imagePanoramic.parentContainer)
        lv.obj.remove_style_all(imagePanoramic.maskObject)
        lv.img.set_src(imagePanoramic.maskObject, imagePanoramic.mask)
        lv.obj.set_size(imagePanoramic.maskObject, width, height)
        lv.obj.align(imagePanoramic.maskObject, lv.ALIGN_TOP_LEFT, 0, 0)
        lv.obj.add_style(imagePanoramic.maskObject, imagePanoramic.styles.mask, lv.STATE_DEFAULT)
        lv.obj.set_style_translate_x(imagePanoramic.imgObject, imagePanoramic.imgxOffset, lv.STATE_DEFAULT)
        lv.group.remove_obj(imagePanoramic.maskObject)
    end
    lv.group.remove_all_objs(document)
    lv.group.add_obj(document, imagePanoramic.parentContainer)
    imagePanoramic.events[imagePanoramic.parentContainer] = {}
    imagePanoramic.events[imagePanoramic.parentContainer].key = lv.obj.add_event_cb(imagePanoramic.parentContainer,
        imagePanoramic.keyPressed, lv.EVENT_KEY)
    if (args.audio_path ~= nil) then
        Global.requestAudioPlay({ path = args.audio_path, AFCb = imagePanoramic.audioCallback, priority = true })
    else
        imagePanoramic.interactionEnabled = true
    end
end

return imagePanoramic
