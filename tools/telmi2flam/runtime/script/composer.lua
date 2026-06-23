---@class composer
local composer = {}
composer.styles = {}
--- Keep pointer on registered events for cleaning
composer.events = {}
composer.interractiveObjects = {}
composer.passiveObjects = {}
composer.parentContainer = nil
composer.label = nil
composer.selector = 1
composer.current = 1
composer.enabled = false


function composer.clean()
    for container, event in pairs(composer.events) do
        lv.obj.remove_event_cb(container, event.key)
    end
    composer.styles = {}
    composer.events = {}
    composer.interractiveObjects = {}
    composer.passiveObjects = {}
    composer.parentContainer = nil
    composer.selector = 1
    composer.current = 1
end

function composer.create(args)
    lv.group.set_editing(document, true)

    lv.group.set_wrap(document, false)
    lv.obj.clean(window)
    composer.initStyles()
    composer.parentContainer = lv.obj.new(window)
    lv.obj.remove_style_all(composer.parentContainer)
    lv.obj.set_size(composer.parentContainer, lv.obj.get_width(window), lv.obj.get_height(window))
    print(lv.obj.get_width(window))
    print(lv.obj.get_height(window))

    composer.renderScene(args.objects)

    lv.group.remove_all_objs(document)
    lv.group.add_obj(document, composer.parentContainer)
    composer.events[composer.parentContainer] = {}
    composer.events[composer.parentContainer].key = lv.obj.add_event_cb(composer.parentContainer,
        composer.keyPressed, lv.EVENT_KEY)

    if (args.audioIntro ~= nil) then
        Global.requestAudioPlay({ path = args.audioIntro, priority = true, AFCb = composer.introCb })
    else
        composer.updateSelected()
        composer.enabled = true
    end
end

function composer.introCb(audioState, seconds)
    if (audioState == "stop") then
        composer.updateSelected()
        composer.enabled = true
    end
end

function composer.renderScene(objects)
    for i, Object in ipairs(objects) do
        if (Object.show ~= false) then
            composer.addObject(Object)
        end
    end
    composer.renderLabel()
end

function composer.renderLabel()
    composer.label = lv.label.new(composer.parentContainer)
    lv.obj.remove_style_all(composer.label)
    lv.obj.set_width(composer.label, 296)
    lv.label.set_long_mode(composer.label, lv.LABEL_LONG_SCROLL_CIRCULAR)
    lv.obj.align(composer.label, lv.ALIGN_BOTTOM_MID, 0, -4)
    lv.obj.set_style_text_align(composer.label, lv.TEXT_ALIGN_CENTER, 0)
    lv.obj.add_style(composer.label, composer.styles.label, lv.STATE_DEFAULT)
    lv.label.set_text(composer.label, " This is a stupidly long Place holder and some for good measure")
    lv.obj.add_flag(composer.label, lv.OBJ_FLAG_HIDDEN)
end

function composer.updateSelected()
    if (# composer.interractiveObjects > 0) then
        composer.interractiveObjects[composer.current].data = Global.load_image(composer.interractiveObjects
            [composer.current].img)
        composer.interractiveObjects[composer.selector].data = Global.load_image(composer.interractiveObjects
            [composer.selector].imgSelected)
        lv.img.set_src(composer.interractiveObjects[composer.current].obj,
            composer.interractiveObjects[composer.current].data)
        lv.img.set_src(composer.interractiveObjects[composer.selector].obj,
            composer.interractiveObjects[composer.selector].data)
        composer.current = composer.selector

        if (composer.interractiveObjects[composer.selector].label ~= nil) then
            lv.label.set_text(composer.label, composer.interractiveObjects[composer.selector].label)
            lv.obj.clear_flag(composer.label, lv.OBJ_FLAG_HIDDEN)
        else
            lv.obj.add_flag(composer.label, lv.OBJ_FLAG_HIDDEN)
        end

        Global.requestAudioPlay({
            path = composer.interractiveObjects[composer.current].audio,
            priority = true
        })
    end
end

function composer.encoderLeft()
    if (composer.selector == 1) then
        composer.selector = #composer.interractiveObjects
    else
        composer.selector = composer.selector - 1
    end
end

function composer.encoderRight()
    if (composer.selector >= #composer.interractiveObjects) then
        composer.selector = 1
    else
        composer.selector = composer.selector + 1
    end
end

function composer.encoderClick()
    if (composer.interractiveObjects[composer.current].cb ~= nil) then
        composer.interractiveObjects[composer.current].cb()
    end
end

function composer.keyPressed(e)
    local key = string.byte(lv.event.get_key_value(e))
    if (#composer.interractiveObjects > 0 and composer.enabled == true) then
        if (key == 19) then
            composer.encoderRight()
            composer.updateSelected()
        end
        if (key == 20) then
            composer.encoderLeft()
            composer.updateSelected()
        end
        if (key == 10) then composer.encoderClick() end
    end
end

function composer.addObject(object)
    local sceneObject = {}
    sceneObject.obj = lv.img.new(composer.parentContainer)
    sceneObject.data = Global.load_image(object.img)
    sceneObject.img = object.img
    sceneObject.imgSelected = object.imgSelected
    sceneObject.label = object.label
    sceneObject.audio = object.audio
    sceneObject.cb = object.cb

    lv.obj.align(sceneObject.obj, lv.ALIGN_TOP_LEFT, object.x, object.y)
    lv.img.set_src(sceneObject.obj, sceneObject.data)
    if (object.imgSelected ~= nil) then
        table.insert(composer.interractiveObjects, sceneObject)
    else
        table.insert(composer.passiveObjects, sceneObject)
    end
end

function composer.initStyles()
    composer.styles.label = lv.style.new()
    lv.style.set_bg_color(composer.styles.label, lv.color.black())
    lv.style.set_bg_opa(composer.styles.label, lv.OPA_COVER)
    lv.style.set_text_color(composer.styles.label, lv.color.hex(0xefedea))
    lv.style.set_text_font(composer.styles.label, lv.font.nunito_extrabold_16)
    lv.style.set_pad_top(composer.styles.label, 4)
    lv.style.set_pad_right(composer.styles.label, 8)
    lv.style.set_pad_bottom(composer.styles.label, 4)
    lv.style.set_pad_left(composer.styles.label, 8)
    lv.style.set_radius(composer.styles.label, 5)
end

return composer
