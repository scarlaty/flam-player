--- @class minimal
local minimal = {}

minimal.styles = {}
--- Keep pointer on registered events for cleaning
minimal.events = {}
-- keep pointer on registered animation for cleaning
minimal.animations = {}

minimal.texts = {}
minimal.images = {}
minimal.buttons = {}
minimal.parentContainer = nil

function minimal.initStyles()
    minimal.styles.parentStyle = lv.style.new()
    lv.style.set_bg_color(minimal.styles.parentStyle, lv.color.hex(0x000000))
    lv.style.set_bg_opa(minimal.styles.parentStyle, lv.OPA_COVER)

    minimal.styles.title = lv.style.new()
    lv.style.set_text_color(minimal.styles.title, lv.color.hex(0xefedea))
    lv.style.set_text_font(minimal.styles.title, lv.font.nunito_extrabold_20)
end
function minimal.clean()
    Global.requestAudioStop(true,true)
    for container, event in pairs( minimal.events) do
        if(event.clicked ~= nil)then  lv.obj.remove_event_cb(container, event.clicked)  end
    end

    for anim, animVar in pairs(minimal.animations) do
        lv.anim_var.del(animVar.var)
    end

    lv.group.remove_all_objs(document)
    lv.obj.clean(window)

    for i, style in pairs(minimal.styles) do
        lv.style.reset(style)
    end
    minimal.styles = {}
    minimal.texts = {}
    minimal.images = {}
    minimal.buttons = {}
    minimal.parentContainer = nil
end
function minimal.audioFeedback(state,seconds)
    if(state == "stop") then minimal.cb() end
end
function minimal.create(args)
    lv.group.set_editing(document, true)
    lv.obj.clean(window) -- Clean main window
    minimal.initStyles()

    minimal.parentContainer = lv.obj.new(window)
    lv.obj.remove_style_all(minimal.parentContainer)
    lv.obj.set_size(minimal.parentContainer, lv.obj.get_width(window), lv.obj.get_height(window))
    lv.obj.add_style(minimal.parentContainer, minimal.styles.parentStyle, lv.STATE_DEFAULT)

    minimal.texts.title = lv.label.new(minimal.parentContainer)
    lv.label.set_long_mode(minimal.texts.title, lv.LABEL_LONG_WRAP)
    
    lv.label.set_text(minimal.texts.title, "Minimal")
    lv.obj.add_style(minimal.texts.title, minimal.styles.title, lv.STATE_DEFAULT)
    lv.obj.set_width(minimal.texts.title, 210)
    lv.obj.set_align(minimal.texts.title,lv.ALIGN_TOP_MID)

    minimal.buttons.validate = Global.button.create(minimal.parentContainer,
        { text = "close", size = "large", theme = Global.button_theme_default })
    lv.obj.align( minimal.buttons.validate.button, lv.ALIGN_BOTTOM_MID, 0, -15)

    minimal.events[minimal.buttons.validate.button] = {}
    minimal.cb = args.cb
    minimal.events[minimal.buttons.validate.button].clicked  = lv.obj.add_event_cb(minimal.buttons.validate.button,minimal.cb, lv.EVENT_CLICKED)
    Global.requestAudioPlay({path = args.audio,AFCb = minimal.audioFeedback})
end


return minimal
