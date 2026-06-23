---@class listQuizz
local listQuizz = {}
--- Stores all lvgl style rules
listQuizz.styles = {}
--- Keep pointer on registered events for cleaning
listQuizz.events = {}

listQuizz.entries = {}
listQuizz.bottomBar = {}
listQuizz.selector = {}
listQuizz.animations = {}
listQuizz.checkBoxData = {}
listQuizz.answers = nil
listQuizz.defaultOutput = nil
listQuizz.maxAnswer = nil
listQuizz.minAnswer = nil
listQuizz.freeSelection = false
listQuizz.outTab = nil
listQuizz.FirstFocus = nil
listQuizz.selectedAnswers = {}

listQuizz.layoutData = {}
listQuizz.layoutData.entry = {}
listQuizz.layoutData.checkbox = {}
listQuizz.layoutData.checkbox.xOffset = 16
listQuizz.layoutData.checkbox.yOffset = 0
listQuizz.layoutData.checkbox.zoom = 256
listQuizz.layoutData.entryImage = {}
listQuizz.layoutData.entryImage.xOffset = -8
listQuizz.layoutData.entryImage.yOffset = 0
listQuizz.layoutData.entryImage.zoom = 256
listQuizz.layoutData.entryImage.animDuration = 500
listQuizz.layoutData.entryImage.animRotation = 80
listQuizz.layoutData.entryLabel = {}
listQuizz.layoutData.entryLabel.xOffset = 44
listQuizz.layoutData.entryLabel.yOffset = 0

function listQuizz.clean()
    for anim, animVar in pairs(listQuizz.animations) do
        lv.anim_var.del(animVar.var)
    end
    for container, event in pairs(listQuizz.events) do
        if (event.clicked ~= nil) then lv.obj.remove_event_cb(container, event.clicked) end
        if (event.focused ~= nil) then lv.obj.remove_event_cb(container, event.focused) end
        if (event.defocused ~= nil) then lv.obj.remove_event_cb(container, event.defocused) end
    end
    listQuizz.styles = {}
    listQuizz.events = {}
    listQuizz.entries = {}
    listQuizz.selector = {}
    listQuizz.layoutData = {}
    listQuizz.animations = {}
    listQuizz.checkBoxData = {}
    listQuizz.selectedAnswers = {}

    listQuizz.answers = nil
    listQuizz.defaultOutput = nil
    listQuizz.maxAnswer = nil
end

function listQuizz.initSyles()
    listQuizz.styles.titleLabelStyle = lv.style.new()
    lv.style.set_text_color(listQuizz.styles.titleLabelStyle, lv.color.hex(0xffffff))
    lv.style.set_text_font(listQuizz.styles.titleLabelStyle, lv.font.nunito_extrabold_16)
    lv.style.set_text_line_space(listQuizz.styles.titleLabelStyle, 4)
    lv.style.set_text_align(listQuizz.styles.titleLabelStyle, lv.TEXT_ALIGN_CENTER)

    listQuizz.styles.subtitleLabel = lv.style.new()
    lv.style.set_text_color(listQuizz.styles.subtitleLabel, lv.color.hex(0xA6B1B5))
    lv.style.set_text_font(listQuizz.styles.subtitleLabel, lv.font.nunito_extrabold_12)
    lv.style.set_text_line_space(listQuizz.styles.subtitleLabel, 4)
    lv.style.set_text_align(listQuizz.styles.subtitleLabel, lv.TEXT_ALIGN_CENTER)

    listQuizz.styles.entryLabel = lv.style.new()
    lv.style.set_text_color(listQuizz.styles.entryLabel, lv.color.hex(0xffffff))
    lv.style.set_text_font(listQuizz.styles.entryLabel, lv.font.nunito_extrabold_20)
    lv.style.set_text_line_space(listQuizz.styles.entryLabel, 4)
    lv.style.set_width(listQuizz.styles.entryLabel, 146)
    lv.style.set_text_align(listQuizz.styles.entryLabel, lv.TEXT_ALIGN_LEFT)

    listQuizz.styles.entryLabelSelected = lv.style.new()
    lv.style.set_text_color(listQuizz.styles.entryLabelSelected, lv.color.hex(0xFBBD2F))
    lv.style.set_text_font(listQuizz.styles.entryLabelSelected, lv.font.nunito_extrabold_20)
    lv.style.set_text_line_space(listQuizz.styles.entryLabelSelected, 4)
    lv.style.set_width(listQuizz.styles.entryLabelSelected, 146)
    lv.style.set_text_align(listQuizz.styles.entryLabelSelected, lv.TEXT_ALIGN_LEFT)


    listQuizz.styles.entryLabelGreyedOut = lv.style.new()
    lv.style.set_text_color(listQuizz.styles.entryLabelGreyedOut, lv.color.hex(0x5F6769))
    lv.style.set_text_font(listQuizz.styles.entryLabelGreyedOut, lv.font.nunito_extrabold_20)
    lv.style.set_text_line_space(listQuizz.styles.entryLabelGreyedOut, 4)
    lv.style.set_width(listQuizz.styles.entryLabelGreyedOut, 146)
    lv.style.set_text_align(listQuizz.styles.entryLabelGreyedOut, lv.TEXT_ALIGN_LEFT)

    listQuizz.styles.entryContainer = lv.style.new()
    lv.style.set_bg_opa(listQuizz.styles.entryContainer, lv.OPA_COVER)
    lv.style.set_bg_color(listQuizz.styles.entryContainer, lv.color.hex(0x202020))
    lv.style.set_radius(listQuizz.styles.entryContainer, 8)
    lv.style.set_pad_ver(listQuizz.styles.entryContainer, 0)
    lv.style.set_width(listQuizz.styles.entryContainer, 296)
    lv.style.set_height(listQuizz.styles.entryContainer, 102)

    listQuizz.styles.entryContainerChecked = lv.style.new()
    lv.style.set_bg_opa(listQuizz.styles.entryContainerChecked, lv.OPA_COVER)
    lv.style.set_bg_color(listQuizz.styles.entryContainerChecked, lv.color.hex(0x2B1E00))
    lv.style.set_radius(listQuizz.styles.entryContainerChecked, 8)
    lv.style.set_pad_ver(listQuizz.styles.entryContainerChecked, 0)
    lv.style.set_width(listQuizz.styles.entryContainerChecked, 296)
    lv.style.set_height(listQuizz.styles.entryContainerChecked, 102)


    listQuizz.styles.buttonDefault = lv.style.new()
    lv.style.set_bg_color(listQuizz.styles.buttonDefault, lv.color.hex(0xffffff))
    lv.style.set_bg_opa(listQuizz.styles.buttonDefault, lv.OPA_TRANSP)
    lv.style.set_border_width(listQuizz.styles.buttonDefault, 2)
    lv.style.set_border_color(listQuizz.styles.buttonDefault, lv.color.hex(0xffffff))
    lv.style.set_border_opa(listQuizz.styles.buttonDefault, lv.OPA_COVER)
    lv.style.set_radius(listQuizz.styles.buttonDefault, 17)
    lv.style.set_text_color(listQuizz.styles.buttonDefault, lv.color.hex(0xffffff))
    lv.style.set_pad_top(listQuizz.styles.buttonDefault, 3)
    lv.style.set_pad_right(listQuizz.styles.buttonDefault, 10)
    lv.style.set_pad_bottom(listQuizz.styles.buttonDefault, 3)
    lv.style.set_pad_left(listQuizz.styles.buttonDefault, 10)
    lv.style.set_text_font(listQuizz.styles.buttonDefault, lv.font.nunito_extrabold_16)

    listQuizz.styles.buttonFocus = lv.style.new()
    lv.style.set_bg_opa(listQuizz.styles.buttonFocus, lv.OPA_COVER)
    lv.style.set_bg_color(listQuizz.styles.buttonFocus, lv.color.hex(0xFFFFFF))
    lv.style.set_border_color(listQuizz.styles.buttonFocus, lv.color.hex(0xFFFFFF))
    lv.style.set_text_color(listQuizz.styles.buttonFocus, lv.color.hex(0x000000))

    listQuizz.styles.buttonPressed = lv.style.new()
    lv.style.set_bg_opa(listQuizz.styles.buttonPressed, lv.OPA_COVER)
    lv.style.set_bg_color(listQuizz.styles.buttonPressed, lv.color.hex(0xFBBD2F))
    lv.style.set_border_color(listQuizz.styles.buttonPressed, lv.color.hex(0xFBBD2F))
    lv.style.set_text_color(listQuizz.styles.buttonPressed, lv.color.hex(0x4F3B0C))

    listQuizz.styles.buttonDisabled = lv.style.new()
    lv.style.set_bg_opa(listQuizz.styles.buttonDisabled, lv.OPA_COVER)
    lv.style.set_bg_color(listQuizz.styles.buttonDisabled, lv.color.hex(0x000000))
    lv.style.set_border_color(listQuizz.styles.buttonDisabled, lv.color.hex(0xC8D1D4))
    lv.style.set_text_color(listQuizz.styles.buttonDisabled, lv.color.hex(0xC8D1D4))

    listQuizz.styles.titleContainer = lv.style.new()
    lv.style.set_bg_opa(listQuizz.styles.titleContainer, lv.OPA_COVER)
    lv.style.set_bg_color(listQuizz.styles.titleContainer, lv.color.hex(0x000000))
    lv.style.set_radius(listQuizz.styles.titleContainer, 8)
    lv.style.set_pad_row(listQuizz.styles.titleContainer, 15)
    lv.style.set_pad_top(listQuizz.styles.titleContainer, 15)

    listQuizz.styles.answerContainer = lv.style.new()
    lv.style.set_pad_row(listQuizz.styles.answerContainer, 12)
    lv.style.set_bg_opa(listQuizz.styles.answerContainer, lv.OPA_COVER)
    lv.style.set_bg_color(listQuizz.styles.answerContainer, lv.color.hex(0x000000))

    listQuizz.styles.bottomBarContainer = lv.style.new()
    lv.style.set_pad_row(listQuizz.styles.bottomBarContainer, 10)
    lv.style.set_bg_opa(listQuizz.styles.bottomBarContainer, lv.OPA_COVER)
    lv.style.set_bg_color(listQuizz.styles.bottomBarContainer, lv.color.hex(0x000000))
    lv.style.set_border_width(listQuizz.styles.bottomBarContainer, 1)
    lv.style.set_border_color(listQuizz.styles.bottomBarContainer, lv.color.hex(0x242424))
    lv.style.set_border_side(listQuizz.styles.bottomBarContainer, lv.BORDER_SIDE_TOP);

    listQuizz.styles.entryImg = lv.style.new()
    lv.style.set_img_opa(listQuizz.styles.entryImg, 255)

    listQuizz.styles.entryImgGreyedOut = lv.style.new()
    lv.style.set_img_opa(listQuizz.styles.entryImgGreyedOut, 100)

    listQuizz.styles.bottomIcon = lv.style.new()
    lv.style.set_img_recolor(listQuizz.styles.bottomIcon, lv.color.make(255, 0, 0))
    lv.style.set_img_opa(listQuizz.styles.bottomIcon, 255)
    lv.style.set_bg_opa(listQuizz.styles.bottomIcon, lv.OPA_COVER)
    lv.style.set_bg_color(listQuizz.styles.bottomIcon, lv.color.hex(0xffffff))
end

function listQuizz.entryFocusedCallback(ev)
    local button = lv.event.get_target(ev)
    listQuizz.showSelector(listQuizz.entries[button].selectable)
    --print("Focused " .. lv.label.get_text(listQuizz.entries[button].label))


    if (listQuizz.entries[button].selectable) then
        if (listQuizz.entries[button].checkable and listQuizz.computeSelectedAmount() < listQuizz.maxAnswer) then
            listQuizz.enableSelector(true)
        elseif (listQuizz.entries[button].checkable and listQuizz.entries[button].checked and listQuizz.computeSelectedAmount() >= listQuizz.maxAnswer) then
            listQuizz.enableSelector(true)
        else
            listQuizz.enableSelector(false)
        end
        if (listQuizz.selector.enabled) then
            lv.obj.clear_state(listQuizz.entries[button].label, lv.STATE_USER_1)
            lv.obj.clear_state(listQuizz.entries[button].imageObject, lv.STATE_USER_1)
        else
            lv.obj.add_state(listQuizz.entries[button].label, lv.STATE_USER_1)
            lv.obj.add_state(listQuizz.entries[button].imageObject, lv.STATE_USER_1)
        end
    end

    if (listQuizz.entries[button].focusedAction ~= nil) then
        listQuizz.entries[button].focusedAction()
    end
end

function listQuizz.entryDefocusedCallback(ev)
    local button = lv.event.get_target(ev)
    --print("Defocused " .. lv.label.get_text(listQuizz.entries[button].label))
    if (listQuizz.entries[button].defocusedAction ~= nil) then
        listQuizz.entries[button].defocusedAction()
    end
end

function listQuizz.entryClickedCallback(ev)
    local button = lv.event.get_target(ev)
    --print("Clicked " .. lv.label.get_text(listQuizz.entries[button].label))
    if (listQuizz.entries[button].clickedAction ~= nil) then
        if (listQuizz.entries[button].selectable and listQuizz.selector.enabled) then
            listQuizz.entries[button].clickedAction()
        elseif (listQuizz.entries[button].selectable == false) then
            listQuizz.entries[button].clickedAction()
        end
    end
    listQuizz.updateBottomBar()
end

function listQuizz.addEvents(target)
    listQuizz.events[target] = {}
    listQuizz.events[target].focused = lv.obj.add_event_cb(target, listQuizz.entryFocusedCallback,
        lv.EVENT_FOCUSED)
    listQuizz.events[target].defocused = lv.obj.add_event_cb(target, listQuizz.entryDefocusedCallback,
        lv.EVENT_DEFOCUSED)
    listQuizz.events[target].clicked = lv.obj.add_event_cb(target, listQuizz.entryClickedCallback,
        lv.EVENT_CLICKED)
end

function listQuizz.buttonFocusedCallback(ev)
    --print("FOCUS Button")
    listQuizz.showSelector(false)

    if (listQuizz.bottomBar.buttonFocusAudio ~= nil) then
        Global.requestAudioPlay({ path = listQuizz.bottomBar.buttonFocusAudio, priority = true })
    else
        Global.requestAudioStop(true, true)
    end

    listQuizz.animateBottomBar(true)
end

function listQuizz.buttonDefocusedCallback(ev)
    --print("DEFOCUS Button")
    listQuizz.animateBottomBar(false)
end

function listQuizz.buttonClicked(ev)
    --print("Clicked Button")
    listQuizz.computeResults()
end

function listQuizz.addEntry(parentContainer, entry, id)
    local entryButton = lv.btn.new(parentContainer)

    if (id == 1) then listQuizz.FirstFocus = entryButton end

    listQuizz.entries[entryButton] = {}

    lv.obj.remove_style_all(entryButton)

    listQuizz.entries[entryButton].checkbox = {}
    listQuizz.entries[entryButton].checkbox = lv.img.new(entryButton)
    lv.obj.align(listQuizz.entries[entryButton].checkbox, lv.ALIGN_LEFT_MID, listQuizz.layoutData.checkbox.xOffset,
        listQuizz.layoutData.checkbox.yOffset)

    lv.obj.add_style(entryButton, listQuizz.styles.entryContainer, lv.STATE_DEFAULT)
    lv.obj.add_style(entryButton, listQuizz.styles.entryContainerChecked, lv.STATE_USER_1)
    if (entry.checkbox == 0) then
        lv.img.set_src(listQuizz.entries[entryButton].checkbox, listQuizz.checkBoxData.uncheckedDisabled)
        listQuizz.entries[entryButton].checkable = false
        listQuizz.entries[entryButton].checked = false
    elseif (entry.checkbox == 1) then
        lv.img.set_src(listQuizz.entries[entryButton].checkbox, listQuizz.checkBoxData.checkedDisabled)
        listQuizz.entries[entryButton].checkable = false
        listQuizz.entries[entryButton].checked = true
    elseif (entry.checkbox == 2) then
        lv.img.set_src(listQuizz.entries[entryButton].checkbox, listQuizz.checkBoxData.unchecked)
        listQuizz.entries[entryButton].checkable = true
        listQuizz.entries[entryButton].checked = false
    elseif (entry.checkbox == 3) then
        lv.img.set_src(listQuizz.entries[entryButton].checkbox, listQuizz.checkBoxData.checked)
        listQuizz.entries[entryButton].checkable = true
        listQuizz.entries[entryButton].checked = true
    end
    lv.img.set_zoom(listQuizz.entries[entryButton].checkbox, listQuizz.layoutData.checkbox.zoom)


    listQuizz.entries[entryButton].label = lv.label.new(entryButton)
    lv.obj.remove_style_all(listQuizz.entries[entryButton].label)
    lv.obj.align(listQuizz.entries[entryButton].label, lv.ALIGN_LEFT_MID, listQuizz.layoutData.entryLabel.xOffset,
        listQuizz.layoutData.entryLabel.yOffset)

    lv.label.set_long_mode(listQuizz.entries[entryButton].label, lv.LABEL_LONG_WRAP)
    lv.label.set_text(listQuizz.entries[entryButton].label, entry.label)

    lv.group.remove_obj(listQuizz.entries[entryButton].label)

    lv.obj.add_style(listQuizz.entries[entryButton].label, listQuizz.styles.entryLabel, lv.STATE_DEFAULT)
    lv.obj.add_style(listQuizz.entries[entryButton].label, listQuizz.styles.entryLabelGreyedOut, lv.STATE_USER_1)
    lv.obj.add_style(listQuizz.entries[entryButton].label, listQuizz.styles.entryLabelSelected, lv.STATE_USER_2)


    listQuizz.entries[entryButton].imageObject = {}
    listQuizz.entries[entryButton].imageObject = lv.img.new(entryButton)
    lv.obj.align(listQuizz.entries[entryButton].imageObject, lv.ALIGN_RIGHT_MID, listQuizz.layoutData.entryImage.xOffset,
        listQuizz.layoutData.entryImage.yOffset)
    listQuizz.entries[entryButton].imgData = Global.load_image(entry.img)
    lv.img.set_src(listQuizz.entries[entryButton].imageObject, listQuizz.entries[entryButton].imgData)
    lv.img.set_zoom(listQuizz.entries[entryButton].imageObject, listQuizz.layoutData.entryImage.zoom)
    lv.obj.add_style(listQuizz.entries[entryButton].imageObject, listQuizz.styles.entryImg, lv.STATE_DEFAULT)
    lv.obj.add_style(listQuizz.entries[entryButton].imageObject, listQuizz.styles.entryImgGreyedOut, lv.STATE_USER_1)



    if (listQuizz.entries[entryButton].checkable == false) then
        lv.obj.add_state(listQuizz.entries[entryButton].label, lv.STATE_USER_1)
        lv.obj.add_state(listQuizz.entries[entryButton].imageObject, lv.STATE_USER_1)
    end

    if (listQuizz.entries[entryButton].checked) then
        lv.obj.add_state(entryButton, lv.STATE_USER_1)
        lv.obj.add_state(listQuizz.entries[entryButton].label, lv.STATE_USER_2)
    end



    listQuizz.entries[entryButton].focusedAction = function()
        Global.requestAudioPlay({ path = entry.audioFocus, priority = true })
        if (listQuizz.animations[entryButton] ~= nil) then
            lv.anim_var.del(listQuizz.animations[entryButton].var)
            listQuizz.animations[entryButton] = nil
        end
        if (listQuizz.entries[entryButton].checkable == true and listQuizz.selector.enabled) then
            local entryAnimation = lv.anim.new()
            listQuizz.animations[entryButton] = {}
            listQuizz.animations[entryButton].anim = entryAnimation
            listQuizz.animations[entryButton].var = lv.anim.set_var(entryAnimation,
                listQuizz.selector.imageObject)
            lv.anim.set_values(entryAnimation, 1, listQuizz.layoutData.entryImage.animRotation)
            lv.anim.set_time(entryAnimation, listQuizz.layoutData.entryImage.animDuration)
            lv.anim.set_playback_time(entryAnimation, listQuizz.layoutData.entryImage.animDuration);
            lv.anim.set_repeat_count(entryAnimation, lv.ANIM_REPEAT_INFINITE);
            lv.anim.set_early_apply(entryAnimation, true);
            lv.anim.set_exec_cb(entryAnimation, function(var, val)
                lv.img.set_angle(listQuizz.entries[entryButton].imageObject, val)
                lv.obj.invalidate(listQuizz.entries[entryButton].imageObject)
            end)

            lv.anim.set_path_cb(entryAnimation, lv.anim.path_ease_in_out)
            lv.anim.start(entryAnimation)
        end
    end
    listQuizz.entries[entryButton].defocusedAction = function()
        if (listQuizz.animations[entryButton] ~= nil) then
            lv.anim_var.del(listQuizz.animations[entryButton].var)
            listQuizz.animations[entryButton] = nil
        end
    end

    listQuizz.entries[entryButton].clickedAction = function()
        if (listQuizz.entries[entryButton].checkable) then
            if (listQuizz.entries[entryButton].checked) then
                lv.img.set_src(listQuizz.entries[entryButton].checkbox, listQuizz.checkBoxData.unchecked)
                lv.obj.clear_state(entryButton, lv.STATE_USER_1)
                lv.obj.clear_state(listQuizz.entries[entryButton].label, lv.STATE_USER_2)
                listQuizz.entries[entryButton].checked = false
                listQuizz.removeSelectedEntry(listQuizz.entries[entryButton])
            else
                lv.img.set_src(listQuizz.entries[entryButton].checkbox, listQuizz.checkBoxData.checked)
                lv.obj.add_state(listQuizz.entries[entryButton].label, lv.STATE_USER_2)
                lv.obj.add_state(entryButton, lv.STATE_USER_1)
                listQuizz.entries[entryButton].checked = true
                listQuizz.addSelectedEntry(listQuizz.entries[entryButton])
            end
            if (listQuizz.computeSelectedAmount() >= listQuizz.minAnswer) then
                lv.obj.clear_state(listQuizz.bottomBar.button, lv.STATE_USER_1)
                lv.group.add_obj(document, listQuizz.bottomBar.button)
            else
                lv.obj.add_state(listQuizz.bottomBar.button, lv.STATE_USER_1)
                lv.group.remove_obj(listQuizz.bottomBar.button)
            end
        end
    end

    listQuizz.entries[entryButton].selectable = true
    listQuizz.entries[entryButton].order = id
    listQuizz.addEvents(entryButton)
end

function listQuizz.drawTitle(parentContainer, title)
    local entryButton = lv.btn.new(parentContainer)

    listQuizz.entries[entryButton] = {}
    lv.obj.remove_style_all(entryButton)
    lv.obj.set_width(entryButton, lv.obj.get_width(parentContainer) - 1)
    lv.obj.set_height(entryButton, lv.obj.get_height(parentContainer) - 1)
    lv.obj.add_flag(entryButton, lv.OBJ_FLAG_SNAPPABLE)

    listQuizz.entries[entryButton].containter = lv.obj.new(entryButton)
    lv.obj.remove_style_all(listQuizz.entries[entryButton].containter)
    lv.obj.align(listQuizz.entries[entryButton].containter, lv.ALIGN_CENTER, 0, 0)
    lv.obj.set_width(listQuizz.entries[entryButton].containter, lv.obj.get_width(parentContainer) - 101)
    lv.obj.set_flex_flow(listQuizz.entries[entryButton].containter, lv.FLEX_FLOW_COLUMN)
    lv.obj.set_flex_align(listQuizz.entries[entryButton].containter, lv.FLEX_ALIGN_START, lv.FLEX_ALIGN_CENTER,
        lv.FLEX_ALIGN_CENTER)
    lv.obj.add_style(listQuizz.entries[entryButton].containter, listQuizz.styles.titleContainer, lv.STATE_DEFAULT)



    listQuizz.entries[entryButton].label = lv.label.new(listQuizz.entries[entryButton].containter)
    lv.obj.remove_style_all(listQuizz.entries[entryButton].label)
    lv.obj.set_style_text_align(listQuizz.entries[entryButton].label, lv.TEXT_ALIGN_CENTER, 0);
    lv.label.set_long_mode(listQuizz.entries[entryButton].label, lv.LABEL_LONG_WRAP)
    lv.label.set_text(listQuizz.entries[entryButton].label, title.text)
    lv.group.remove_obj(listQuizz.entries[entryButton].label)
    lv.obj.set_width(listQuizz.entries[entryButton].label, lv.obj.get_width(parentContainer) - 101)
    lv.obj.add_style(listQuizz.entries[entryButton].label, listQuizz.styles.titleLabelStyle, lv.STATE_DEFAULT)

    if (title.subtitle ~= nil) then
        listQuizz.entries[entryButton].subtitle = lv.label.new(listQuizz.entries[entryButton].containter)
        lv.obj.remove_style_all(listQuizz.entries[entryButton].subtitle)
        lv.obj.set_style_text_align(listQuizz.entries[entryButton].subtitle, lv.TEXT_ALIGN_CENTER, 0);
        lv.label.set_long_mode(listQuizz.entries[entryButton].subtitle, lv.LABEL_LONG_WRAP)
        lv.label.set_text(listQuizz.entries[entryButton].subtitle, title.subtitle)
        lv.group.remove_obj(listQuizz.entries[entryButton].subtitle)
        lv.obj.set_width(listQuizz.entries[entryButton].subtitle, lv.obj.get_width(parentContainer) - 101)
        lv.obj.add_style(listQuizz.entries[entryButton].subtitle, listQuizz.styles.subtitleLabel, lv.STATE_DEFAULT)
    end

    listQuizz.entries[entryButton].arrow = lv.img.new(listQuizz.entries[entryButton].containter)
    lv.obj.remove_style_all(listQuizz.entries[entryButton].arrow)
    lv.img.set_src(listQuizz.entries[entryButton].arrow, listQuizz.selector.dataEnabled)
    lv.img.set_zoom(listQuizz.entries[entryButton].arrow, 256)
    lv.img.set_angle(listQuizz.entries[entryButton].arrow, -450)

    local titleArrowAnimation = lv.anim.new()
    listQuizz.animations[titleArrowAnimation] = {}
    listQuizz.animations[titleArrowAnimation].anim = titleArrowAnimation
    listQuizz.animations[titleArrowAnimation].var = lv.anim.set_var(titleArrowAnimation,
        listQuizz.selector.lowerLeftCorner)
    lv.anim.set_values(titleArrowAnimation, 0, 5)
    lv.anim.set_time(titleArrowAnimation, 500)
    lv.anim.set_playback_time(titleArrowAnimation, 500);
    lv.anim.set_repeat_count(titleArrowAnimation, lv.ANIM_REPEAT_INFINITE);
    lv.anim.set_early_apply(titleArrowAnimation, true);

    lv.anim.set_exec_cb(titleArrowAnimation, function(var, val)
        lv.obj.set_style_translate_y(listQuizz.entries[entryButton].arrow, val, lv.STATE_DEFAULT)
        lv.obj.invalidate(listQuizz.selector.lowerLeftCorner)
    end)

    lv.anim.set_path_cb(titleArrowAnimation, lv.anim.path_ease_in_out)
    lv.anim.start(titleArrowAnimation)

    listQuizz.entries[entryButton].focusedAction = function()
        Global.requestAudioPlay({ path = title.audioFocusTitle, priority = true })
        lv.obj.clear_flag(listQuizz.entries[entryButton].arrow, lv.OBJ_FLAG_HIDDEN)
    end
    listQuizz.entries[entryButton].defocusedAction = function()
        lv.obj.add_flag(listQuizz.entries[entryButton].arrow, lv.OBJ_FLAG_HIDDEN)
    end
    listQuizz.entries[entryButton].clickedAction = function()
        lv.group.focus_obj(listQuizz.FirstFocus)
        lv.obj.update_snap(parentContainer, lv.ANIM_ON);
    end

    listQuizz.entries[entryButton].selectable = false
    listQuizz.addEvents(entryButton)
end

function listQuizz.drawSelector()
    listQuizz.selector.dataEnabled = Global.load_image("script/corner.lif")
    listQuizz.selector.dataDisabled = Global.load_image("script/cornerDisabled.lif")
    listQuizz.selector.lowerLeftCorner = lv.img.new(window)
    lv.obj.remove_style_all(listQuizz.selector.lowerLeftCorner)
    lv.obj.add_flag(listQuizz.selector.lowerLeftCorner, lv.OBJ_FLAG_FLOATING)
    lv.obj.align(listQuizz.selector.lowerLeftCorner, lv.ALIGN_CENTER, -140, 20)
    lv.img.set_src(listQuizz.selector.lowerLeftCorner, listQuizz.selector.dataEnabled)
    lv.img.set_zoom(listQuizz.selector.lowerLeftCorner, 256)

    listQuizz.selector.upperLeftCorner = lv.img.new(window)
    lv.obj.remove_style_all(listQuizz.selector.upperLeftCorner)
    lv.obj.add_flag(listQuizz.selector.upperLeftCorner, lv.OBJ_FLAG_FLOATING)
    lv.obj.align(listQuizz.selector.upperLeftCorner, lv.ALIGN_CENTER, -140, -68)
    lv.img.set_src(listQuizz.selector.upperLeftCorner, listQuizz.selector.dataEnabled)
    lv.img.set_zoom(listQuizz.selector.upperLeftCorner, 256)
    lv.img.set_angle(listQuizz.selector.upperLeftCorner, 900)

    listQuizz.selector.upperRightCorner = lv.img.new(window)
    lv.obj.remove_style_all(listQuizz.selector.upperRightCorner)
    lv.obj.add_flag(listQuizz.selector.upperRightCorner, lv.OBJ_FLAG_FLOATING)
    lv.obj.align(listQuizz.selector.upperRightCorner, lv.ALIGN_CENTER, 140, -68)
    lv.img.set_src(listQuizz.selector.upperRightCorner, listQuizz.selector.dataEnabled)
    lv.img.set_zoom(listQuizz.selector.upperRightCorner, 256)
    lv.img.set_angle(listQuizz.selector.upperRightCorner, 1800)

    listQuizz.selector.lowerRightCorner = lv.img.new(window)
    lv.obj.remove_style_all(listQuizz.selector.lowerRightCorner)
    lv.obj.add_flag(listQuizz.selector.lowerRightCorner, lv.OBJ_FLAG_FLOATING)
    lv.obj.align(listQuizz.selector.lowerRightCorner, lv.ALIGN_CENTER, 140, 20)
    lv.img.set_src(listQuizz.selector.lowerRightCorner, listQuizz.selector.dataEnabled)
    lv.img.set_zoom(listQuizz.selector.lowerRightCorner, 256)
    lv.img.set_angle(listQuizz.selector.lowerRightCorner, -900)

    local selectorAnimation = lv.anim.new()
    listQuizz.animations[selectorAnimation] = {}
    listQuizz.animations[selectorAnimation].anim = selectorAnimation
    listQuizz.animations[selectorAnimation].var = lv.anim.set_var(selectorAnimation,
        listQuizz.selector.lowerLeftCorner)
    lv.anim.set_values(selectorAnimation, 0, -4)
    lv.anim.set_time(selectorAnimation, 500)
    lv.anim.set_playback_time(selectorAnimation, 500);
    lv.anim.set_repeat_count(selectorAnimation, lv.ANIM_REPEAT_INFINITE);
    lv.anim.set_early_apply(selectorAnimation, true);

    lv.anim.set_exec_cb(selectorAnimation, function(var, val)
        lv.obj.set_style_translate_y(listQuizz.selector.lowerLeftCorner, -val, lv.STATE_DEFAULT)
        lv.obj.set_style_translate_x(listQuizz.selector.lowerLeftCorner, val, lv.STATE_DEFAULT)
        lv.obj.invalidate(listQuizz.selector.lowerLeftCorner)

        lv.obj.set_style_translate_y(listQuizz.selector.lowerRightCorner, -val, lv.STATE_DEFAULT)
        lv.obj.set_style_translate_x(listQuizz.selector.lowerRightCorner, -val, lv.STATE_DEFAULT)
        lv.obj.invalidate(listQuizz.selector.lowerRightCorner)

        lv.obj.set_style_translate_y(listQuizz.selector.upperLeftCorner, val, lv.STATE_DEFAULT)
        lv.obj.set_style_translate_x(listQuizz.selector.upperLeftCorner, val, lv.STATE_DEFAULT)
        lv.obj.invalidate(listQuizz.selector.upperLeftCorner)

        lv.obj.set_style_translate_y(listQuizz.selector.upperRightCorner, val, lv.STATE_DEFAULT)
        lv.obj.set_style_translate_x(listQuizz.selector.upperRightCorner, -val, lv.STATE_DEFAULT)
        lv.obj.invalidate(listQuizz.selector.upperRightCorner)
    end)

    lv.anim.set_path_cb(selectorAnimation, lv.anim.path_ease_in_out)
    lv.anim.start(selectorAnimation)

    listQuizz.showSelector(false)
    listQuizz.selector.enabled = true
end

function listQuizz.showSelector(onOff)
    if (onOff) then
        lv.obj.clear_flag(listQuizz.selector.lowerLeftCorner, lv.OBJ_FLAG_HIDDEN)
        lv.obj.clear_flag(listQuizz.selector.lowerRightCorner, lv.OBJ_FLAG_HIDDEN)
        lv.obj.clear_flag(listQuizz.selector.upperLeftCorner, lv.OBJ_FLAG_HIDDEN)
        lv.obj.clear_flag(listQuizz.selector.upperRightCorner, lv.OBJ_FLAG_HIDDEN)
    else
        lv.obj.add_flag(listQuizz.selector.lowerLeftCorner, lv.OBJ_FLAG_HIDDEN)
        lv.obj.add_flag(listQuizz.selector.lowerRightCorner, lv.OBJ_FLAG_HIDDEN)
        lv.obj.add_flag(listQuizz.selector.upperLeftCorner, lv.OBJ_FLAG_HIDDEN)
        lv.obj.add_flag(listQuizz.selector.upperRightCorner, lv.OBJ_FLAG_HIDDEN)
    end
end

function listQuizz.enableSelector(onOff)
    if (onOff and listQuizz.selector.enabled == false) then
        lv.img.set_src(listQuizz.selector.lowerLeftCorner, listQuizz.selector.dataEnabled)
        lv.img.set_src(listQuizz.selector.lowerRightCorner, listQuizz.selector.dataEnabled)
        lv.img.set_src(listQuizz.selector.upperLeftCorner, listQuizz.selector.dataEnabled)
        lv.img.set_src(listQuizz.selector.upperRightCorner, listQuizz.selector.dataEnabled)
        listQuizz.selector.enabled = true
    elseif (onOff == false and listQuizz.selector.enabled) then
        lv.img.set_src(listQuizz.selector.lowerLeftCorner, listQuizz.selector.dataDisabled)
        lv.img.set_src(listQuizz.selector.lowerRightCorner, listQuizz.selector.dataDisabled)
        lv.img.set_src(listQuizz.selector.upperLeftCorner, listQuizz.selector.dataDisabled)
        lv.img.set_src(listQuizz.selector.upperRightCorner, listQuizz.selector.dataDisabled)
        listQuizz.selector.enabled = false
    end
end

function listQuizz.loadBoxes()
    listQuizz.checkBoxData.uncheckedDisabled = Global.load_image("script/uncheckDisabled.lif")
    listQuizz.checkBoxData.checkedDisabled = Global.load_image("script/checkDisabled.lif")
    listQuizz.checkBoxData.unchecked = Global.load_image("script/uncheck.lif")
    listQuizz.checkBoxData.checked = Global.load_image("script/check.lif")
end

function listQuizz.addSelectedEntry(entry)
    table.insert(listQuizz.selectedAnswers, entry)
end

function listQuizz.removeSelectedEntry(entry)
    for pos, selected in ipairs(listQuizz.selectedAnswers) do
        if (selected.order == entry.order) then
            table.remove(listQuizz.selectedAnswers, pos)
        end
    end
end

function listQuizz.computeSelectedAmount()
    return #listQuizz.selectedAnswers
end

function listQuizz.drawBottomBar(parent, answerAmount, buttonParams)
    if (listQuizz.freeSelection) then
        listQuizz.bottomBar.defaultImageData = Global.load_image("script/empty.lif")
    else
        listQuizz.bottomBar.defaultImageData = Global.load_image("script/picto_empty.lif")
    end
    listQuizz.bottomBar.icons = {}
    for i = 0, answerAmount - 1 do
        table.insert(listQuizz.bottomBar.icons, lv.img.new(parent))
        if (i == 0) then
            lv.obj.align(listQuizz.bottomBar.icons[i + 1], lv.ALIGN_LEFT_MID, 12, 0)
        else
            lv.obj.align(listQuizz.bottomBar.icons[i + 1], lv.ALIGN_LEFT_MID, 12 + i * 40, 0)
        end

        lv.img.set_src(listQuizz.bottomBar.icons[i + 1], listQuizz.bottomBar.defaultImageData)
        lv.img.set_zoom(listQuizz.bottomBar.icons[i + 1], 256)
    end

    listQuizz.bottomBar.button = lv.btn.new(parent)
    lv.obj.remove_style_all(listQuizz.bottomBar.button)
    listQuizz.bottomBar.label = lv.label.new(listQuizz.bottomBar.button)
    lv.obj.remove_style_all(listQuizz.bottomBar.label)
    lv.label.set_text(listQuizz.bottomBar.label, buttonParams.text)

    lv.obj.align(listQuizz.bottomBar.button, lv.ALIGN_RIGHT_MID, -12, 0)

    lv.obj.add_style(listQuizz.bottomBar.button, listQuizz.styles.buttonDefault, lv.STATE_DEFAULT)
    lv.obj.add_style(listQuizz.bottomBar.button, listQuizz.styles.buttonFocus, lv.STATE_FOCUSED)
    lv.obj.add_style(listQuizz.bottomBar.button, listQuizz.styles.buttonPressed, lv.STATE_PRESSED)
    lv.obj.add_style(listQuizz.bottomBar.button, listQuizz.styles.buttonDisabled, lv.STATE_USER_1)

    listQuizz.bottomBar.buttonFocusAudio = buttonParams.audioFocus
    listQuizz.events[listQuizz.bottomBar.button] = {}
    listQuizz.events[listQuizz.bottomBar.button].focused = lv.obj.add_event_cb(listQuizz.bottomBar.button,
        listQuizz.buttonFocusedCallback,
        lv.EVENT_FOCUSED)
    listQuizz.events[listQuizz.bottomBar.button].defocused = lv.obj.add_event_cb(listQuizz.bottomBar.button,
        listQuizz.buttonDefocusedCallback,
        lv.EVENT_DEFOCUSED)
    listQuizz.events[listQuizz.bottomBar.button].clicked = lv.obj.add_event_cb(listQuizz.bottomBar.button,
        listQuizz.buttonClicked,
        lv.EVENT_CLICKED)
    if (listQuizz.computeSelectedAmount() < listQuizz.minAnswer) then
        lv.obj.add_state(listQuizz.bottomBar.button, lv.STATE_USER_1)
        lv.group.remove_obj(listQuizz.bottomBar.button)
    end
end

function listQuizz.updateBottomBar()
    --if (listQuizz.computeSelectedAmount() > listQuizz.maxAnswer) then return end

    for i, icon in ipairs(listQuizz.bottomBar.icons) do
        lv.obj.set_style_translate_x(icon, 0, lv.STATE_DEFAULT)
        lv.img.set_src(icon, listQuizz.bottomBar.defaultImageData)
        lv.img.set_zoom(icon, 256)
        lv.obj.invalidate(icon)
    end

    for pos, selectedAnswer in ipairs(listQuizz.selectedAnswers) do
        lv.img.set_src(listQuizz.bottomBar.icons[pos], selectedAnswer.imgData)
        lv.img.set_zoom(listQuizz.bottomBar.icons[pos], 95)
        lv.obj.set_style_translate_x(listQuizz.bottomBar.icons[pos], -28, lv.STATE_DEFAULT)
        lv.obj.invalidate(selectedAnswer.imageObject)
    end
end

function listQuizz.animateBottomBar(onOff)
    if (listQuizz.animations[listQuizz.bottomBar] ~= nil) then
        lv.anim_var.del(listQuizz.animations[listQuizz.bottomBar].var)
        listQuizz.animations[listQuizz.bottomBar] = nil
    end
    if (onOff) then
        local bottomBarAnimation = lv.anim.new()
        listQuizz.animations[listQuizz.bottomBar] = {}
        listQuizz.animations[listQuizz.bottomBar].anim = bottomBarAnimation
        listQuizz.animations[listQuizz.bottomBar].var = lv.anim.set_var(bottomBarAnimation,
            listQuizz.bottomBar)
        lv.anim.set_values(bottomBarAnimation, 0, 80)
        lv.anim.set_time(bottomBarAnimation, 500)
        lv.anim.set_playback_time(bottomBarAnimation, 500);
        lv.anim.set_repeat_count(bottomBarAnimation, lv.ANIM_REPEAT_INFINITE);
        lv.anim.set_early_apply(bottomBarAnimation, true);
        lv.anim.set_exec_cb(bottomBarAnimation, function(var, val)
            for i, icon in ipairs(listQuizz.bottomBar.icons) do
                lv.img.set_angle(icon, val)
                lv.obj.invalidate(icon)
            end
        end)

        lv.anim.set_path_cb(bottomBarAnimation, lv.anim.path_ease_in_out)
        lv.anim.start(bottomBarAnimation)
    else
        for i, icon in ipairs(listQuizz.bottomBar.icons) do
            lv.img.set_angle(icon, 0)
            lv.obj.invalidate(icon)
        end
    end
end

function listQuizz.computeResults()
    local entryOrdered = Global.sortCollectionTable(listQuizz.entries)
    if (listQuizz.computeSelectedAmount() < listQuizz.minAnswer) then
        print("Not enough answers selected")
        return
    end

    for k in pairs(listQuizz.outTab) do
        listQuizz.outTab[k] = nil
    end

    for _, entry in ipairs(entryOrdered) do
        local data = { label = lv.label.get_text(entry.label), value = entry.checked }
        table.insert(listQuizz.outTab, data)
    end

    for _, out in ipairs(listQuizz.answers) do
        local outFlag = true
        for it, entry in ipairs(entryOrdered) do
            if (out.data[it] ~= entry.checked) then
                outFlag = false
                break
            end
        end
        if (outFlag) then
            --print("Out condition matched")
            out.cb()
            return
        end
    end

    listQuizz.defaultOutput()
end

function listQuizz.create(args)
    lv.group.set_editing(document, false)
    lv.group.set_wrap(document, false)
    lv.obj.clean(window) -- Clean main window

    if (args.MaximumAnswerAmount ~= nil) then
        listQuizz.maxAnswer = args.MaximumAnswerAmount
    else
        listQuizz.maxAnswer = 5
        listQuizz.freeSelection = true
    end

    listQuizz.minAnswer = args.minimumAnswerAmount
    listQuizz.answers = args.answers
    listQuizz.defaultOutput = args.defaultOutput
    listQuizz.outTab = args.outputTab
    listQuizz.initSyles()
    listQuizz.loadBoxes()
    local parentContainer = lv.obj.new(window)
    lv.obj.remove_style_all(parentContainer)
    lv.obj.set_size(parentContainer, lv.obj.get_width(window), lv.obj.get_height(window))
    lv.obj.set_flex_flow(parentContainer, lv.FLEX_FLOW_COLUMN)

    local AnswersContainer = lv.obj.new(parentContainer)
    lv.obj.remove_style_all(AnswersContainer)
    lv.obj.set_size(AnswersContainer, lv.obj.get_width(window), lv.obj.get_height(window) - 48)
    lv.obj.set_scroll_snap_y(AnswersContainer, lv.SCROLL_SNAP_CENTER)
    lv.obj.set_flex_flow(AnswersContainer, lv.FLEX_FLOW_COLUMN)
    lv.obj.set_flex_align(AnswersContainer, lv.FLEX_ALIGN_START, lv.FLEX_ALIGN_CENTER, lv.FLEX_ALIGN_CENTER)
    lv.obj.add_style(AnswersContainer, listQuizz.styles.answerContainer, lv.STATE_DEFAULT)

    listQuizz.drawSelector()
    listQuizz.drawTitle(AnswersContainer, args.title)

    -- Draw Entries
    for i, v in ipairs(args.choices) do
        listQuizz.addEntry(AnswersContainer, v, i)
    end

    lv.obj.update_snap(AnswersContainer, lv.ANIM_ON);

    local bottomBarContainer = lv.obj.new(parentContainer)
    lv.obj.remove_style_all(bottomBarContainer)
    lv.obj.set_size(bottomBarContainer, lv.obj.get_width(window), 48)
    lv.obj.add_style(bottomBarContainer, listQuizz.styles.bottomBarContainer, lv.STATE_DEFAULT)

    listQuizz.drawBottomBar(bottomBarContainer, listQuizz.maxAnswer, args.validation)
    listQuizz.updateBottomBar()

    Global.requestAudioPlay({ path = args.title.audioFocusTitle, priority = true })
end

return listQuizz
