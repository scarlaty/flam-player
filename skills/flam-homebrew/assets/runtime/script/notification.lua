--- @class notification
local notification = {}

notification.styles = {}
--- Keep pointer on registered events for cleaning
notification.events = {}

notification.texts = {}
notification.images = {}
notification.buttons = {}
notification.cb = nil
notification.entryContainer = nil
notification.parentContainer = nil

notification.sounds = {}
notification.soundIterator = 1
function notification.playListFeedback(state,seconds)
    if(state == "stop") then
        notification.soundIterator = notification.soundIterator + 1
        if(notification.soundIterator <= #notification.sounds )then
            Global.requestAudioPlay({ path = notification.sounds[notification.soundIterator], AFCb=notification.playListFeedback})
        end
    end
end
function notification.clean()
    for container, event in pairs( notification.events) do
        if(event.clicked ~= nil)then  lv.obj.remove_event_cb(container, event.clicked)  end
    end

    lv.obj.remove_style_all(notification.parentContainer)
    lv.obj.remove_style_all(notification.entryContainer)
    lv.obj.remove_style_all(notification.texts.subtitle)
    lv.obj.remove_style_all(notification.buttons.validate.button)
    for _, image in pairs(notification.images) do
        lv.obj.remove_style_all(image.object)
    end

    for i, style in pairs(notification.styles) do
        lv.style.reset(style)
    end
    notification.styles = {}
    notification.texts = {}
    notification.images = {}
    notification.buttons = {}

    notification.cb = nil
    notification.entryContainer = nil
    notification.parentContainer = nil

end
function notification.initStyles()
    notification.styles.parentStyle = lv.style.new()
    lv.style.set_bg_color(notification.styles.parentStyle, lv.color.hex(0x000000))
    lv.style.set_bg_opa(notification.styles.parentStyle, lv.OPA_COVER)

    notification.styles.textContainer = lv.style.new()
    lv.style.set_pad_top(notification.styles.textContainer,20)
    lv.style.set_pad_left(notification.styles.textContainer,32)

    notification.styles.image = lv.style.new()
    lv.style.set_img_opa(notification.styles.image ,255)

    notification.styles.title = lv.style.new()
    lv.style.set_text_color(notification.styles.title, lv.color.hex(0xefedea))
    lv.style.set_text_font(notification.styles.title, lv.font.nunito_extrabold_20)

    notification.styles.subtitle = lv.style.new()
    lv.style.set_text_color(notification.styles.subtitle, lv.color.hex(0xefedea))
    lv.style.set_text_font(notification.styles.subtitle, lv.font.nunito_extrabold_16)
    lv.style.set_pad_top(notification.styles.subtitle,5)
    lv.style.set_pad_bottom(notification.styles.subtitle,20)

    notification.styles.button = lv.style.new()
    lv.style.set_pad_top(notification.styles.button,0)
end
function notification.create(args)
    lv.group.set_editing(document, true)
    lv.obj.clean(window) -- Clean main window
    notification.initStyles()

    notification.parentContainer = lv.obj.new(window)
    lv.obj.remove_style_all(notification.parentContainer)
    lv.obj.set_size(notification.parentContainer, lv.obj.get_width(window), lv.obj.get_height(window))
    lv.obj.add_style(notification.parentContainer, notification.styles.parentStyle, lv.STATE_DEFAULT)


    notification.entryContainer = Global.v_container.create(notification.parentContainer, 7, true)
    lv.obj.remove_style_all(notification.entryContainer)
    lv.obj.set_style_pad_row(notification.entryContainer, 0, 0)
    lv.obj.set_style_pad_bottom(notification.entryContainer,0,lv.STATE_DEFAULT)
    lv.obj.set_size(notification.entryContainer,  lv.obj.get_width(window), lv.obj.get_height(window))
    lv.obj.set_flex_flow(notification.entryContainer, lv.FLEX_FLOW_COLUMN)
    lv.obj.add_style(notification.entryContainer, notification.styles.textContainer, lv.STATE_DEFAULT)

    notification.texts.title = lv.label.new(notification.entryContainer)
    lv.label.set_long_mode(notification.texts.title, lv.LABEL_LONG_WRAP)
    
    lv.label.set_text(notification.texts.title, args.title)
    lv.obj.add_style(notification.texts.title, notification.styles.title, lv.STATE_DEFAULT)
    lv.obj.set_width(notification.texts.title, 210)

    if(args.subtitle ~= nil) then
        notification.texts.subtitle = lv.label.new(notification.entryContainer)
        lv.label.set_long_mode(notification.texts.subtitle, lv.LABEL_LONG_WRAP)
        lv.label.set_text(notification.texts.subtitle, args.subtitle)
        lv.obj.add_style(notification.texts.subtitle, notification.styles.subtitle, lv.STATE_DEFAULT)
        lv.obj.set_width(notification.texts.subtitle, 210)
    else
        notification.texts.subtitle = lv.label.new(notification.entryContainer)
        lv.label.set_long_mode(notification.texts.subtitle, lv.LABEL_LONG_WRAP)
        lv.label.set_text(notification.texts.subtitle, " ")
        lv.obj.add_style(notification.texts.subtitle, notification.styles.title, lv.STATE_DEFAULT)
    end

    

    notification.buttons.validate = Global.button.create(notification.entryContainer,
        { text = args.buttonText, size = "large", theme = Global.button_theme_default })
    lv.obj.add_style(notification.buttons.validate.button, notification.styles.button, lv.STATE_DEFAULT)

    notification.cb = args.cb
    notification.events[notification.buttons.validate.button] = {}
    notification.events[notification.buttons.validate.button].clicked  = lv.obj.add_event_cb(notification.buttons.validate.button,notification.cb, lv.EVENT_CLICKED)

    for it,path in pairs(args.images) do
        print(path)
        notification.images[path] = {}
        notification.images[path].object = lv.img.new(notification.parentContainer)
        lv.obj.remove_style_all(notification.images[path].object)
        lv.obj.add_flag(notification.images[path].object, lv.OBJ_FLAG_FLOATING)
        lv.obj.align(notification.images[path].object, lv.ALIGN_BOTTOM_RIGHT, 0, 0)
        notification.images[path].data = Global.load_image(args.images[it])
        lv.img.set_src(notification.images[path].object, notification.images[path].data)
        lv.obj.add_style(notification.images[path].object, notification.styles.image, lv.STATE_DEFAULT)
    end

    lv.group.remove_obj(notification.entryContainer)
    lv.group.focus_obj(notification.buttons.validate.button)
    notification.sounds = args.audio
    Global.requestAudioPlay({ path = notification.sounds[1], AFCb = notification.playListFeedback, priority = true})
end

return notification
