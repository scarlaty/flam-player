--- This module il used to render a list with sound over, sound intro and image
---@class cirularListChoice
local circularListChoice = {}
circularListChoice.styles = {}
--- Keep pointer on registered events for cleaning
circularListChoice.events = {}
-- keep pointer on registered animation for cleaning
circularListChoice.animations = {}
-- store images data
circularListChoice.diceFaces = {}

circularListChoice.arc = nil
circularListChoice.arcPos = 211
circularListChoice.arcLen = 30
circularListChoice.arcIncr = 10
circularListChoice.arcSelLen = 30


circularListChoice.answerContainer = nil
circularListChoice.answers = {}

circularListChoice.image = nil
circularListChoice.imageData = nil

circularListChoice.snapTimmer = nil

function circularListChoice.initSyles()
    circularListChoice.styles.parentStyle = lv.style.new()


    circularListChoice.styles.arcMain = lv.style.new()
    lv.style.set_arc_color(circularListChoice.styles.arcMain,lv.color.hex(0x333333))
    lv.style.set_arc_width(circularListChoice.styles.arcMain,3)

    circularListChoice.styles.arcIndic = lv.style.new()
    lv.style.set_arc_color( circularListChoice.styles.arcIndic,lv.color.hex(0xffffff))
    lv.style.set_arc_width(circularListChoice.styles.arcIndic,3)

    circularListChoice.styles.arcKnob = lv.style.new()
    lv.style.set_bg_opa(circularListChoice.styles.arcKnob, lv.OPA_TRANSP)
    
    circularListChoice.styles.answerContainer = lv.style.new()
    lv.style.set_bg_opa( circularListChoice.styles.answerContainer, lv.OPA_TRANSP)

    circularListChoice.styles.answerStyle = lv.style.new()
    lv.style.set_bg_opa(circularListChoice.styles.answerStyle, lv.OPA_TRANSP)
    lv.style.set_text_color( circularListChoice.styles.answerStyle, lv.color.hex(0x333333))
    lv.style.set_text_font( circularListChoice.styles.answerStyle, lv.font.nunito_extrabold_20)


    circularListChoice.styles.answerStyleFocused = lv.style.new()
    lv.style.set_bg_opa(circularListChoice.styles.answerStyleFocused, lv.OPA_TRANSP)
    lv.style.set_text_color( circularListChoice.styles.answerStyleFocused, lv.color.hex(0xffffff))
    lv.style.set_text_font( circularListChoice.styles.answerStyleFocused, lv.font.nunito_extrabold_20)

    circularListChoice.styles.title = lv.style.new()
    lv.style.set_text_color(circularListChoice.styles.title, lv.color.hex(0xefedea))
    lv.style.set_text_font(circularListChoice.styles.title, lv.font.nunito_extrabold_14)
    lv.style.set_pad_top(circularListChoice.styles.title, 10)
    lv.style.set_pad_left(circularListChoice.styles.title, 10)
end

function circularListChoice.clear()

end
function circularListChoice.focusEntry(entry)
    lv.obj.remove_style_all(entry)
    lv.obj.add_style(entry,circularListChoice.styles.answerStyleFocused,lv.STATE_DEFAULT)
    circularListChoice.imageData = Global.load_image(circularListChoice.answers[entry].imagePath)
    lv.img.set_src(circularListChoice.image,circularListChoice.imageData)
    circularListChoice.answers[entry].focused = true
end
function circularListChoice.defocusEntry(entry)
    lv.obj.remove_style_all(entry)
    lv.obj.add_style(entry,circularListChoice.styles.answerStyle,lv.STATE_DEFAULT)
    circularListChoice.answers[entry].focused = false
end
function circularListChoice.snapArc()
    

    for entry, v in pairs( circularListChoice.answers) do
        if( circularListChoice.answers[entry].focused) then
            circularListChoice.arcPos = v.arcTrigger + math.floor(circularListChoice.arcLen/2) +5
            lv.arc.set_angles(circularListChoice.arc,circularListChoice.arcPos,circularListChoice.arcPos + circularListChoice.arcLen  )
           -- print("Snap on "..circularListChoice.answers[entry].label)
        end
    end

end
function circularListChoice.processArcPosition()
    for entry, v in pairs( circularListChoice.answers) do
        if(circularListChoice.arcPos > v.arcTrigger and circularListChoice.arcPos <= v.arcTrigger + circularListChoice.arcSelLen) then
            circularListChoice.focusEntry(entry)
            local area = lv.area.new()
            --local button = lv.obj.get_child(circularListChoice.answerContainer, 0)
            lv.obj.get_coords(entry, area)
            local button_y1 = lv.area.get_y1(area)
            lv.obj.get_coords(circularListChoice.answerContainer, area)
            local Ypos = (Global.visual_height/2) - button_y1
            print("Raw pos : "..button_y1)
            print("Centered pos: "..Ypos)
            print("current pos "..v.arcTrigger + math.floor(circularListChoice.arcLen/2) +5)
            print("arcSin "..math.asin(Ypos/135).." ".. (math.asin(Ypos/135)*135) + 250)
        else
            circularListChoice.defocusEntry(entry)
        end
        
    end
end
function circularListChoice.keyPressed(e)
    lv.timer.reset(circularListChoice.snapTimmer)

    if (string.byte(lv.event.get_key_value(e)) == 19 )then
        if(circularListChoice.arcPos < 350 ) then circularListChoice.arcPos = circularListChoice.arcPos + circularListChoice.arcIncr  end
        
    elseif (string.byte(lv.event.get_key_value(e)) == 20 ) then
        if(circularListChoice.arcPos > 160 ) then circularListChoice.arcPos = circularListChoice.arcPos - circularListChoice.arcIncr  end
    end
    if(circularListChoice.arcPos < 211) then circularListChoice.arcPos = 211 end
    if(circularListChoice.arcPos > 300) then circularListChoice.arcPos = 300 end
    lv.arc.set_angles(circularListChoice.arc,circularListChoice.arcPos,circularListChoice.arcPos + circularListChoice.arcLen  )
    --print(circularListChoice.arcPos)
    circularListChoice.processArcPosition()
end

function circularListChoice.entryFocusedCallback(ev)
    local button = lv.event.get_target(ev)
    if(circularListChoice.answers[button].audioPath ~= nil) then
        Global.requestAudioPlay({path =circularListChoice.answers[button].audioPath})
    end
end
function circularListChoice.addEntry(entryLabel,entryImagePath,entryAudiopath,entryCallback,arcTrigger,progressionData)
    local entryButton = lv.btn.new(circularListChoice.answerContainer)
    lv.obj.remove_style_all(entryButton)
    lv.obj.add_style(entryButton,circularListChoice.styles.answerStyle,lv.STATE_DEFAULT)
    --lv.obj.add_style(entryButton,circularListChoice.styles.answerStyleFocused,lv.STATE_FOCUS_KEY)

    circularListChoice.answers[entryButton] = {}
    circularListChoice.answers[entryButton].imagePath = entryImagePath;
    circularListChoice.answers[entryButton].audioPath = entryAudiopath;
    circularListChoice.answers[entryButton].label = entryLabel;
    circularListChoice.answers[entryButton].cb = entryCallback;
    circularListChoice.answers[entryButton].arcTrigger = arcTrigger
    circularListChoice.answers[entryButton].focused = false
    

    local label = lv.label.new( entryButton)
    lv.obj.remove_style_all(label)
    --lv.obj.set_width(label, lv.obj.get_width(circularListChoice.answerContainer) - 32) -- ??????
    lv.obj.set_width(label, 219)
    lv.label.set_long_mode(label, lv.LABEL_LONG_WRAP)
    lv.label.set_text(label, circularListChoice.answers[entryButton].label)
    
    lv.group.remove_obj(entryButton)
    lv.group.focus_obj(entryButton)
    print(entryLabel.." added")
end

function circularListChoice.create(args)
    lv.group.set_editing(document, true)
    lv.obj.clean(window) -- Clean main window
    circularListChoice.initSyles()
    circularListChoice.parentContainer = lv.obj.new(window)
    lv.obj.remove_style_all(circularListChoice.parentContainer)
    lv.obj.set_size(circularListChoice.parentContainer, lv.obj.get_width(window), lv.obj.get_height(window))
    lv.obj.add_style(circularListChoice.parentContainer, circularListChoice.styles.parentStyle, lv.STATE_DEFAULT)

    circularListChoice.image = lv.img.new(window)
    lv.obj.remove_style_all(circularListChoice.image)
    lv.obj.align(circularListChoice.image , lv.ALIGN_LEFT_MID, -20, 15)
    --lv.obj.set_size(circularListChoice.image , 80, 80)
    lv.img.set_zoom(circularListChoice.image,190)
    lv.img.set_src(circularListChoice.image,nil)

    circularListChoice.arc = lv.arc.new(circularListChoice.parentContainer)
    lv.obj.align(circularListChoice.arc, lv.ALIGN_LEFT_MID, -40, 10)
    lv.obj.add_style(circularListChoice.arc, circularListChoice.styles.arcMain , lv.PART_MAIN)
    lv.obj.add_style(circularListChoice.arc, circularListChoice.styles.arcIndic, lv.PART_INDICATOR)
    lv.obj.add_style(circularListChoice.arc, circularListChoice.styles.arcKnob, lv.PART_KNOB)
    lv.arc.set_angles(circularListChoice.arc,circularListChoice.arcPos,circularListChoice.arcPos + circularListChoice.arcLen  )
    lv.obj.set_size(circularListChoice.arc,135,135)
    lv.arc.set_rotation(circularListChoice.arc,90)

    circularListChoice.events[circularListChoice.parentContainer] = {}
    circularListChoice.events[circularListChoice.parentContainer].key = lv.obj.add_event_cb(circularListChoice.parentContainer, circularListChoice.keyPressed,
        lv.EVENT_KEY)
    lv.group.add_obj(document, circularListChoice.parentContainer)

    local tileLabel = lv.label.new(circularListChoice.parentContainer)
    lv.obj.remove_style_all(tileLabel)
    lv.obj.set_width(tileLabel, 290)
    lv.label.set_long_mode(tileLabel, lv.LABEL_LONG_WRAP)
    lv.label.set_text(tileLabel, args.title)
    lv.obj.add_style(tileLabel, circularListChoice.styles.title, lv.STATE_DEFAULT)

    circularListChoice.answerContainer = lv.obj.new(circularListChoice.parentContainer)
    lv.obj.remove_style_all(circularListChoice.answerContainer)
    lv.obj.set_style_pad_row(circularListChoice.answerContainer, 19, 0)
    lv.obj.set_style_pad_bottom(circularListChoice.answerContainer,10,lv.STATE_DEFAULT)
    lv.obj.set_flex_flow(circularListChoice.answerContainer, lv.FLEX_FLOW_COLUMN)
    lv.obj.set_size(circularListChoice.answerContainer, 220, 150)
    lv.obj.add_style(circularListChoice.answerContainer, circularListChoice.styles.answerContainer, lv.STATE_DEFAULT)
    lv.obj.align(circularListChoice.answerContainer, lv.ALIGN_RIGHT_MID, 0, 13)

    
    

    local entryFound = false
    for i, v in ipairs(args.choices) do
        if v.show ~= false then
            circularListChoice.addEntry(v.label, v.img, v.audio, v.cb,210 + (i*circularListChoice.arcSelLen - circularListChoice.arcSelLen),v.slider)
            entryFound = true
        end
    end

    circularListChoice.snapTimmer = lv.timer.new(circularListChoice.snapArc, 100, nil)
    circularListChoice.processArcPosition()
    
    --lv.obj.scroll_by( circularListChoice.answerContainer, 0, -40, lv.ANIM_ON)
end

return circularListChoice