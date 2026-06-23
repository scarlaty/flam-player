--- This module il used to render a list with sound over, sound intro and image 
---@class listChoice
local listChoice = {}
--- Stores all lvgl style rules
listChoice.styles = {}
--- Keep pointer on registered events for cleaning
listChoice.events = {}
--- Pointer on answerContainer ( used by addEntry function)
listChoice.answerContainer = nil
-- Pointer on currently focused Item
listChoice.focusedEntry = nil
--- Answers properties array
listChoice.answers = {}
-- keep pointer on registered animation for cleaning
listChoice.animations = {}
-- store images data
listChoice.images = {}
-- pointer on scroll event callbak
listChoice.scrollEvent = nil
-- animation timer for slower compute rate on animation
listChoice.animationTimer = nil
-- flaf used by computeAnimation function
listChoice.animationWaiting = false
listChoice.haveTitle = false
listChoice.forcePlay = true
function listChoice.initSyles()

    listChoice.styles.parentStyle = lv.style.new()
    lv.style.set_pad_hor(listChoice.styles.parentStyle, 16)
    lv.style.set_pad_ver(listChoice.styles.parentStyle, 0)
    lv.style.set_bg_color(listChoice.styles.parentStyle, lv.color.hex(0x000000))
    lv.style.set_bg_opa(listChoice.styles.parentStyle, lv.OPA_COVER)
    lv.style.set_pad_top(listChoice.styles.parentStyle, 16)
    lv.style.set_pad_row(listChoice.styles.parentStyle, 16)

    listChoice.styles.titleLabelStyle = lv.style.new()
    lv.style.set_text_color(listChoice.styles.titleLabelStyle, lv.color.hex(0xffffff))
    lv.style.set_text_font(listChoice.styles.titleLabelStyle, lv.font.nunito_bold_12)

    listChoice.styles.answerContainerStyle = lv.style.new()
    lv.style.set_bg_color(listChoice.styles.answerContainerStyle, lv.color.hex(0x0000ff))

    listChoice.styles.answerStyle = lv.style.new()
    lv.style.set_bg_color( listChoice.styles.answerStyle, lv.color.black())
    lv.style.set_text_color( listChoice.styles.answerStyle, lv.color.hex(0xefedea))
    lv.style.set_text_font( listChoice.styles.answerStyle, lv.font.nunito_extrabold_20)
    lv.style.set_text_opa( listChoice.styles.answerStyle, 127)

    listChoice.styles.answerStyleFocused = lv.style.new()
    lv.style.set_text_color( listChoice.styles.answerStyleFocused, lv.color.hex(0xffffff))

    listChoice.styles.answerImages = lv.style.new()

    listChoice.styles.sliderMain = lv.style.new()
    lv.style.set_bg_opa(listChoice.styles.sliderMain , lv.OPA_COVER)
    lv.style.set_bg_color(listChoice.styles.sliderMain , lv.color.hex(0xa79f8e))
    lv.style.set_radius(listChoice.styles.sliderMain , 2)
    lv.style.set_pad_ver(listChoice.styles.sliderMain , 0)

    listChoice.styles.sliderIndicator = lv.style.new()
    lv.style.set_bg_opa(listChoice.styles.sliderIndicator, lv.OPA_COVER)
    lv.style.set_bg_color(listChoice.styles.sliderIndicator, lv.color.hex(0xfbbd2a))
    lv.style.set_radius(listChoice.styles.sliderIndicator, 2)

    listChoice.styles.knob = lv.style.new()
    lv.style.set_bg_opa(listChoice.styles.knob, lv.OPA_TRANSP)
    lv.style.set_border_width(listChoice.styles.knob, 0)
    lv.style.set_pad_all(listChoice.styles.knob, 0)

    listChoice.styles.sliderLabel = lv.style.new()
    lv.style.set_bg_color(listChoice.styles.sliderLabel, lv.color.black())
    lv.style.set_text_color(listChoice.styles.sliderLabel, lv.color.hex(0xf2f4f5))
    lv.style.set_text_font(listChoice.styles.sliderLabel, lv.font.nunito_extrabold_12)

end
function listChoice.audioFeedback(state,second)
    if (state == "stop") then
        listChoice.enableSelection()
    end
end
function listChoice.enableSelection()    

    lv.img.set_src(listChoice.images.arrow.object,listChoice.images.arrow.data)
    local area = lv.area.new()
    local button = lv.obj.get_child(listChoice.answerContainer, 0)
    lv.obj.get_coords(button, area)
    local button_y1 = lv.area.get_y1(area)
    lv.obj.get_coords(listChoice.answerContainer, area)
    lv.obj.set_style_translate_y(listChoice.images.arrow.object, button_y1 - Global.header_height + listChoice.images.arrow.YOffset, lv.STATE_DEFAULT)

    local translateArrowAnimX = lv.anim.new()
    listChoice.animations[translateArrowAnimX] = {}
    listChoice.animations[translateArrowAnimX].var = lv.anim.set_var(translateArrowAnimX, nil)
    listChoice.animations[translateArrowAnimX].duration = 300
    lv.anim.set_values(translateArrowAnimX, -32, 12)
    lv.anim.set_time(translateArrowAnimX, listChoice.animations[translateArrowAnimX].duration)
    lv.anim.set_exec_cb(translateArrowAnimX, function(_, val)
        lv.obj.set_style_translate_x(listChoice.images.arrow.object, val, lv.STATE_DEFAULT)
    end)
    lv.anim.set_path_cb(translateArrowAnimX, lv.anim.path_ease_in_out)
    lv.anim.start(translateArrowAnimX)

    local answersOpacityAnim = lv.anim.new()
    listChoice.animations[answersOpacityAnim] = {}
    listChoice.animations[answersOpacityAnim].var = lv.anim.set_var(answersOpacityAnim, nil)
    listChoice.animations[answersOpacityAnim].duration = 300
    lv.anim.set_values(answersOpacityAnim, 127, 255)
    lv.anim.set_time(answersOpacityAnim, listChoice.animations[answersOpacityAnim].duration)
    lv.anim.set_exec_cb(answersOpacityAnim, function(_, val)
        lv.style.set_text_opa(listChoice.styles.answerStyle, val)
        lv.obj.invalidate(listChoice.answerContainer) -- mark for repaint
        if(val == 255) then
            for i = 0, lv.obj.get_child_cnt(listChoice.answerContainer) - 1 do
                lv.group.add_obj(document, lv.obj.get_child(listChoice.answerContainer, i))
            end
        end
    end)
    
    lv.anim.set_path_cb(answersOpacityAnim, lv.anim.path_ease_in_out)
    lv.anim.start(answersOpacityAnim)

end
function listChoice.clean()
    Global.requestAudioStop()
    for container, event in pairs( listChoice.events) do
        lv.obj.remove_event_cb(container, event.focused)
        lv.obj.remove_event_cb(container, event.clicked)
    end

    for anim, animVar in pairs( listChoice.animations) do
        --lv.anim_var.del(anim)
        lv.anim_var.del(animVar.var)
    end
    lv.obj.remove_event_cb(listChoice.answerContainer,listChoice.scrollEvent)
    listChoice.answers = {}
    listChoice.images = {}
    listChoice.animations ={}

    lv.timer.del(listChoice.animationTimer)

    lv.group.remove_all_objs(document)
    lv.obj.clean(window)

    listChoice.events = {}
    listChoice.styles = {}
end
function listChoice.onScrollEnd()
   if(listChoice.focusedEntry  ~= nil) then
    local button_y1 = listChoice.clampArrow(listChoice.focusedEntry)
    lv.obj.set_style_translate_y(listChoice.images.arrow.object, button_y1 - Global.header_height + listChoice.images.arrow.YOffset, lv.STATE_DEFAULT)
   end
end
function listChoice.clampArrow(button)
    local area = lv.area.new()
    lv.obj.get_coords(button, area)
    local button_y1 = lv.area.get_y1(area)
    if(listChoice.haveTitle) then
        if(button_y1 < 75) then button_y1 = 75 end
    else
        if(button_y1 < 44) then button_y1 = 44 end
    end
    if(button_y1 > 210) then button_y1 = 210 end
    return button_y1
end
function listChoice.addSlider(entryButton,percentage_value)
    lv.obj.set_flex_flow(entryButton, lv.FLEX_FLOW_COLUMN)
    lv.obj.set_style_pad_row(entryButton, 11, lv.STATE_DEFAULT)

    local slider_cont = lv.obj.new(entryButton)
    lv.obj.remove_style_all(slider_cont)
    lv.obj.set_flex_flow(slider_cont, lv.FLEX_FLOW_ROW)
    lv.obj.set_flex_align(slider_cont, lv.FLEX_ALIGN_START, lv.FLEX_ALIGN_CENTER, lv.FLEX_ALIGN_START)
    lv.obj.set_style_pad_column(slider_cont, 12, lv.STATE_DEFAULT)
    lv.obj.set_size(slider_cont, 142, 18)

    local slider = lv.slider.new(slider_cont)
    lv.obj.remove_style_all(slider)
    lv.group.remove_obj(slider)
    lv.obj.set_size(slider, 65, 4)

    lv.obj.add_style(slider, listChoice.styles.sliderMain , lv.PART_MAIN)
    lv.obj.add_style(slider, listChoice.styles.sliderIndicator, lv.PART_INDICATOR)
    lv.obj.add_style(slider, listChoice.styles.knob, lv.PART_KNOB)

    lv.slider.set_range(slider, 0, 100)
    lv.slider.set_value(slider, percentage_value, lv.ANIM_OFF)

    local slider_label = lv.label.new(slider_cont)
    lv.obj.remove_style_all(slider_label)
    lv.label.set_text(slider_label, percentage_value .. "%")
    lv.obj.add_style(slider_label, listChoice.styles.sliderLabel, lv.STATE_DEFAULT)

    lv.obj.set_style_translate_x(entryButton, 28, lv.STATE_DEFAULT)
end
function listChoice.entryFocusedCallback(ev)
    local button = lv.event.get_target(ev)
    listChoice.focusedEntry = button
    listChoice.animationWaiting = true

    local button_y1 = listChoice.clampArrow(button)
    lv.obj.set_style_translate_y(listChoice.images.arrow.object, button_y1 - Global.header_height + listChoice.images.arrow.YOffset, lv.STATE_DEFAULT)

    if(listChoice.answers[button] ~= nil and listChoice.answers[button].audioPath ~= nil) then
        Global.requestAudioPlay({path =listChoice.answers[button].audioPath, priority = listChoice.forcePlay})
    end
end
function listChoice.computeAnimation()
    if(listChoice.focusedEntry ~= nil and listChoice.animationWaiting == true) then
        listChoice.showIllustration(listChoice.focusedEntry)
        listChoice.animationWaiting = false
    end
end
function listChoice.showIllustration(button)

    for btn, answer in pairs(listChoice.answers) do
        if(button ~= btn) then
            lv.anim.set_values(answer.animOut, lv.obj.get_x(answer.imageObject), Global.visual_width)
            lv.anim.start(answer.animOut)
            
        else
            lv.anim.set_values(listChoice.answers[button].animIn, lv.obj.get_x(listChoice.answers[button].imageObject), Global.visual_width - 116)
            lv.anim.start(listChoice.answers[button].animIn)
        end
    end 

end
function listChoice.addEntry(entryLabel,entryImagePath,entryAudiopath,entryCallback,progressionData)
    if type(entryCallback) ~= "function" or type(entryLabel) ~= "string" or type(entryImagePath) ~= "string" or type(entryAudiopath) ~= "string" then
        print("Invalid entryCallback => skip")
        print("   entryLabel=" .. tostring(entryLabel))
        print("   entryImagePath=" .. tostring(entryImagePath))
        print("   entryAudiopath=" .. tostring(entryAudiopath))
        return
    end

    local entryButton = lv.btn.new(listChoice.answerContainer)
    lv.obj.remove_style_all(entryButton)
    lv.obj.add_style(entryButton,listChoice.styles.answerStyle,lv.STATE_DEFAULT)
    lv.obj.add_style(entryButton,listChoice.styles.answerStyleFocused,lv.STATE_FOCUS_KEY)

    listChoice.events[entryButton] = {}
    listChoice.events[entryButton].focused = lv.obj.add_event_cb(entryButton,listChoice.entryFocusedCallback,lv.EVENT_FOCUSED)
    listChoice.events[entryButton].clicked = lv.obj.add_event_cb(entryButton, function()
        entryCallback()
    end,lv.EVENT_CLICKED)

    listChoice.answers[entryButton] = {}
    listChoice.answers[entryButton].imagePath = entryImagePath;
    listChoice.answers[entryButton].audioPath = entryAudiopath;
    listChoice.answers[entryButton].label = entryLabel;

    
    listChoice.answers[entryButton].imageObject = lv.img.new(window)
    lv.obj.remove_style_all(listChoice.answers[entryButton].imageObject)
    lv.obj.add_flag(listChoice.answers[entryButton].imageObject, lv.OBJ_FLAG_FLOATING)
    lv.obj.align(listChoice.answers[entryButton].imageObject, lv.ALIGN_TOP_LEFT, Global.visual_width, 92)
    lv.obj.set_size(listChoice.answers[entryButton].imageObject, 116, 120)

    if(entryImagePath ~= nil ) then
        listChoice.answers[entryButton].imageData,listChoice.answers[entryButton].imageWidth,listChoice.answers[entryButton].imageHeight = Global.load_image(listChoice.answers[entryButton].imagePath) 
        lv.img.set_src(listChoice.answers[entryButton].imageObject, listChoice.answers[entryButton].imageData)
    end

    local slideInAnimation = lv.anim.new()  
    listChoice.animations[slideInAnimation] = {}
    listChoice.animations[slideInAnimation].var = lv.anim.set_var(slideInAnimation, listChoice.answers[entryButton].imageObject)
    listChoice.animations[slideInAnimation].duration = 200
    lv.anim.set_values(slideInAnimation, lv.obj.get_x(listChoice.answers[entryButton].imageObject), Global.visual_width - 116)
    lv.anim.set_time(slideInAnimation, listChoice.animations[slideInAnimation].duration)
    lv.anim.set_exec_cb(slideInAnimation, function(_, val)
        lv.obj.set_x(listChoice.answers[entryButton].imageObject, val)
    end)
    lv.anim.set_path_cb(slideInAnimation, lv.anim.path_ease_in_out)
    listChoice.answers[entryButton].animIn = slideInAnimation


    local slideOutAnimation = lv.anim.new()
    listChoice.animations[slideOutAnimation] = {}
    listChoice.animations[slideOutAnimation].var = lv.anim.set_var(slideOutAnimation, listChoice.answers[entryButton].imageObject)
    listChoice.animations[slideOutAnimation].duration = 200
    lv.anim.set_values(slideOutAnimation, lv.obj.get_x(listChoice.answers[entryButton].imageObject), Global.visual_width)
    lv.anim.set_time(slideOutAnimation, listChoice.animations[slideOutAnimation].duration)
    lv.anim.set_exec_cb(slideOutAnimation, function(_, val)
        lv.obj.set_x(listChoice.answers[entryButton].imageObject, val)
    end)
    lv.anim.set_path_cb(slideOutAnimation, lv.anim.path_ease_in_out)
    listChoice.answers[entryButton].animOut = slideOutAnimation

    

    local label = lv.label.new(entryButton)
    lv.obj.set_width(label, lv.obj.get_width(listChoice.answerContainer) - 32) -- ??????
    lv.label.set_long_mode(label, lv.LABEL_LONG_WRAP)
    lv.label.set_text(label, listChoice.answers[entryButton].label)
    lv.obj.align(label, lv.ALIGN_TOP_LEFT, 28, 0)
    if(progressionData  ~= nil) then
        listChoice.addSlider(entryButton,progressionData.percentage_value)
    end
    lv.group.remove_obj(entryButton)

end
function listChoice.create(args)

    lv.group.set_editing(document, false)
    lv.group.set_wrap(document, false)
    lv.obj.clean(window) -- Clean main window
    listChoice.initSyles()
    local exitCallback = args.exitCb
    local parentContainer = lv.obj.new(window)
    lv.obj.remove_style_all(parentContainer)
    lv.obj.set_flex_flow(parentContainer, lv.FLEX_FLOW_COLUMN)
    lv.obj.set_size(parentContainer, lv.obj.get_width(window), lv.obj.get_height(window))
    lv.obj.add_style(parentContainer, listChoice.styles.parentStyle, lv.STATE_DEFAULT)

    if(args.title ~= nil) then
        local tileLabel = lv.label.new(parentContainer)
        lv.obj.remove_style_all(tileLabel)
        lv.obj.set_width(tileLabel, 290)
        lv.label.set_long_mode(tileLabel, lv.LABEL_LONG_WRAP)
        lv.label.set_text(tileLabel, args.title)
        lv.obj.add_style(tileLabel,  listChoice.styles.titleLabelStyle, lv.STATE_DEFAULT)
        listChoice.haveTitle = true
    end
    listChoice.answerContainer = lv.obj.new(parentContainer)
    lv.obj.remove_style_all(listChoice.answerContainer)
    lv.obj.set_style_pad_row(listChoice.answerContainer, 16, 0)
    lv.obj.set_style_pad_bottom(listChoice.answerContainer,50,lv.STATE_DEFAULT)
    lv.obj.set_flex_flow(listChoice.answerContainer, lv.FLEX_FLOW_COLUMN)
    if(args.title ~= nil) then
        lv.obj.set_size(listChoice.answerContainer, 191, 168)
    else
        lv.obj.set_size(listChoice.answerContainer, 191, 190)
    end
    lv.obj.add_style(listChoice.answerContainer, listChoice.styles.answerContainerStyle, lv.STATE_DEFAULT)
    listChoice.scrollEvent = lv.obj.add_event_cb(listChoice.answerContainer, listChoice.onScrollEnd, lv.EVENT_SCROLL_END)

    local arrowData, arrowWidth, arrowHeight = Global.load_image("script/arrow-right-ui-000.lif")
    listChoice.images.arrow = {}
    listChoice.images.arrow.data = arrowData
    listChoice.images.arrow.object = lv.img.new(window)
    listChoice.images.arrow.YOffset = 3
    lv.obj.remove_style_all(listChoice.images.arrow.object)
    lv.obj.add_flag(listChoice.images.arrow.object, lv.OBJ_FLAG_FLOATING)
    lv.obj.set_size(listChoice.images.arrow.object, arrowWidth, arrowHeight)
    lv.img.set_src(listChoice.images.arrow.object, nil)
    lv.obj.set_style_translate_x(listChoice.images.arrow.object, -32, lv.STATE_DEFAULT)
    lv.obj.set_style_translate_y(listChoice.images.arrow.object, -50, lv.STATE_DEFAULT)


    local nbEntry = 0
    local lastCb = nil
    for _, v in ipairs(args.choices) do
        if v.show ~= false then
            listChoice.addEntry(v.label, v.img, v.audio, v.cb,v.slider)
            nbEntry = nbEntry + 1
            lastCb = v.cb
        end
    end

    lv.obj.scroll_to(listChoice.answerContainer, 0, 0, lv.ANIM_OFF)
    listChoice.animationTimer = lv.timer.new(listChoice.computeAnimation, 250, nil)

    -- If only one entry found, do callback of the entry
    if(nbEntry == 1 and lastCb ~= nil and args.skipIfLastChoice ~= nil and args.skipIfLastChoice == true) then
        return lastCb()
    end

    if(args.exitCb ~= nil and nbEntry == 0) then
        return args.exitCb()
    end
    if(args.forcePlay ~= nil) then
        listChoice.forcePlay = args.forcePlay
    end
    if (args.title_audio ~= nil) then
        Global.requestAudioPlay({path = args.title_audio,AFCb = listChoice.audioFeedback,priority = true})
    else
        listChoice.enableSelection()
    end

end

return listChoice