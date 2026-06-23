---@class interactiveDialog
local interactiveDialog = {}
--- Stores all lvgl style rules
interactiveDialog.styles = {}
--- Keep pointer on registered events for cleaning
interactiveDialog.events = {}
interactiveDialog.cb = nil

interactiveDialog.answerContainer = nil

interactiveDialog.background = {}
interactiveDialog.background.path = nil
interactiveDialog.background.data = nil
interactiveDialog.background.object = nil

interactiveDialog.leftContent = {}
interactiveDialog.leftContent.path = nil
interactiveDialog.leftContent.data = nil
interactiveDialog.leftContent.object = nil

interactiveDialog.rightContent = {}
interactiveDialog.rightContent.path = nil
interactiveDialog.rightContent.data = nil
interactiveDialog.rightContent.object = nil

interactiveDialog.answers = {}
interactiveDialog.sceneState = {}
interactiveDialog.sceneState.backgroundState = true
interactiveDialog.sceneState.leftContentState = true
interactiveDialog.sceneState.rightContentState = true

interactiveDialog.selector = {}
interactiveDialog.selector.data = nil

interactiveDialog.steps = nil
interactiveDialog.stepIterator = 1;

interactiveDialog.ressources = {}
interactiveDialog.debug = nil

interactiveDialog.elapsed = 0
interactiveDialog.pause = false
interactiveDialog.seconds = -1
interactiveDialog.timer = nil
interactiveDialog.it = 1
interactiveDialog.working = false
interactiveDialog.stepMode = false
interactiveDialog.first = true

function interactiveDialog.initStyles()
    interactiveDialog.styles.imageContentActive = lv.style.new()
    lv.style.set_img_opa(interactiveDialog.styles.imageContentActive, 255)

    interactiveDialog.styles.imageContentInactive = lv.style.new()
    lv.style.set_img_opa(interactiveDialog.styles.imageContentInactive, 100)

    interactiveDialog.styles.answerLabel = lv.style.new()
    lv.style.set_text_color(interactiveDialog.styles.answerLabel, lv.color.hex(0xffffff))
    lv.style.set_text_font(interactiveDialog.styles.answerLabel, lv.font.nunito_extrabold_14)
    lv.style.set_text_line_space(interactiveDialog.styles.answerLabel, 4)
    lv.style.set_width(interactiveDialog.styles.answerLabel, 220)
    lv.style.set_text_align(interactiveDialog.styles.answerLabel, lv.TEXT_ALIGN_LEFT)

    interactiveDialog.styles.answerContainer = lv.style.new()

    lv.style.set_pad_row(interactiveDialog.styles.answerContainer, 4)
    lv.style.set_bg_opa(interactiveDialog.styles.answerContainer, lv.OPA_TRANSP)

    interactiveDialog.styles.answerButton = lv.style.new()
    lv.style.set_bg_opa(interactiveDialog.styles.answerButton, lv.OPA_COVER)
    lv.style.set_bg_color(interactiveDialog.styles.answerButton, lv.color.hex(0x202020))
    lv.style.set_radius(interactiveDialog.styles.answerButton, 16)
    lv.style.set_width(interactiveDialog.styles.answerButton, 288)
    lv.style.set_height(interactiveDialog.styles.answerButton, 60)
end

function interactiveDialog.clean()
    Global.requestAudioStop(true, true)
    if (interactiveDialog.timer ~= nil) then
        lv.timer.del(interactiveDialog.timer)
        interactiveDialog.timer = nil
    end
    for container, event in pairs(interactiveDialog.events) do
        if (event.clicked ~= nil) then lv.obj.remove_event_cb(container, event.clicked) end
        if (event.focused ~= nil) then lv.obj.remove_event_cb(container, event.focused) end
        if (event.defocused ~= nil) then lv.obj.remove_event_cb(container, event.defocused) end
    end

    interactiveDialog.debug = nil
    interactiveDialog.background = {}
    interactiveDialog.leftContent = {}
    interactiveDialog.rightContent = {}
    interactiveDialog.answers = {}
    interactiveDialog.sceneState = {}
    interactiveDialog.events = {}
    interactiveDialog.styles = {}
    interactiveDialog.ressources = {}
    interactiveDialog.steps = {}
    interactiveDialog.cb = nil

    print("Pouet !")
end

function interactiveDialog.audioCallback(state, seconds)
    if (state == "stop") then
        interactiveDialog.cb()
    else
        interactiveDialog.seconds = seconds
        if (state == "pause") then
            interactiveDialog.pause = -1
        end

        if (interactiveDialog.timer == nil and interactiveDialog.stepMode == true) then
            print("Create Timmer")
            interactiveDialog.timer = lv.timer.new(interactiveDialog.timerCallback, 100, nil)
        end
    end
end

function interactiveDialog.timerCallback()
    if (interactiveDialog.pause == false) then
        interactiveDialog.elapsed = interactiveDialog.elapsed + .1
        if (interactiveDialog.debug ~= nil) then
            lv.label.set_text(interactiveDialog.debug, interactiveDialog.seconds .. " " .. interactiveDialog.elapsed)
        end
        if (interactiveDialog.steps ~= nil) then
            local pos = 0
            while interactiveDialog.it <= #interactiveDialog.steps and interactiveDialog.steps[interactiveDialog.it].timeCode < interactiveDialog.elapsed do
                interactiveDialog.it = interactiveDialog.it + 1
                pos = pos + 1
            end
            if (interactiveDialog.it > 1) then
                if (interactiveDialog.steps[interactiveDialog.it - 1].consumed == false and interactiveDialog.working == false) then
                    interactiveDialog.renderStep(interactiveDialog.steps[interactiveDialog.it - 1])
                else
                    interactiveDialog.steps[interactiveDialog.it - 1].consumed = true
                end
            end
        end
    end
end

function interactiveDialog.renderStep(step)
    interactiveDialog.working = true
    step.action()
    interactiveDialog.working = false
end

function interactiveDialog.computeContentState(StateOverride)
    if (interactiveDialog.sceneState.backgroundState) then
        lv.obj.add_state(interactiveDialog.background.object, lv.STATE_USER_1)
    else
        lv.obj.clear_state(interactiveDialog.background.object, lv.STATE_USER_1)
    end
    if (interactiveDialog.sceneState.leftContentState) then
        lv.obj.add_state(interactiveDialog.leftContent.object, lv.STATE_USER_1)
    else
        lv.obj.clear_state(interactiveDialog.leftContent.object, lv.STATE_USER_1)
    end
    if (interactiveDialog.sceneState.rightContentState) then
        lv.obj.add_state(interactiveDialog.rightContent.object, lv.STATE_USER_1)
    else
        lv.obj.clear_state(interactiveDialog.rightContent.object, lv.STATE_USER_1)
    end
    if (StateOverride ~= nil) then
        if (StateOverride.backgroundState ~= nil) then
            if (StateOverride.backgroundState) then
                lv.obj.add_state(interactiveDialog.background.object, lv.STATE_USER_1)
            else
                lv.obj.clear_state(interactiveDialog.background.object, lv.STATE_USER_1)
            end
        end
        if (StateOverride.leftContentState ~= nil) then
            if (StateOverride.leftContentState) then
                lv.obj.add_state(interactiveDialog.leftContent.object, lv.STATE_USER_1)
            else
                lv.obj.clear_state(interactiveDialog.leftContent.object, lv.STATE_USER_1)
            end
        end
        if (StateOverride.rightContentState ~= nil) then
            if (StateOverride.rightContentState) then
                lv.obj.add_state(interactiveDialog.rightContent.object, lv.STATE_USER_1)
            else
                lv.obj.clear_state(interactiveDialog.rightContent.object, lv.STATE_USER_1)
            end
        end
    end
end

function interactiveDialog.answerFocusedCallback(ev)
    local answerButton = lv.event.get_target(ev)
    if (interactiveDialog.answers[answerButton].focusedAction ~= nil) then
        interactiveDialog.answers[answerButton].focusedAction()
    end
end

function interactiveDialog.answerDefocusedCallback(ev)
    local answerButton = lv.event.get_target(ev)
end

function interactiveDialog.answerClickedCallback(ev)
    local answerButton = lv.event.get_target(ev)
    if (interactiveDialog.answers[answerButton].focusedAction ~= nil) then
        interactiveDialog.answers[answerButton].clickedAction()
    end
end

function interactiveDialog.addEvents(target)
    interactiveDialog.events[target] = {}
    interactiveDialog.events[target].focused = lv.obj.add_event_cb(target, interactiveDialog.answerFocusedCallback,
        lv.EVENT_FOCUSED)
    interactiveDialog.events[target].defocused = lv.obj.add_event_cb(target, interactiveDialog.answerDefocusedCallback,
        lv.EVENT_DEFOCUSED)
    interactiveDialog.events[target].clicked = lv.obj.add_event_cb(target, interactiveDialog.answerClickedCallback,
        lv.EVENT_CLICKED)
end

function interactiveDialog.renderScene(parentContainer)
    interactiveDialog.background.object = lv.img.new(parentContainer)
    if (interactiveDialog.background.path ~= nil) then
        interactiveDialog.background.data = Global.load_image(interactiveDialog.background.path)
    end
    lv.img.set_src(interactiveDialog.background.object, interactiveDialog.background.data)
    lv.obj.add_style(interactiveDialog.background.object, interactiveDialog.styles.imageContentInactive, lv
        .STATE_DEFAULT)
    lv.obj.add_style(interactiveDialog.background.object, interactiveDialog.styles.imageContentActive, lv.STATE_USER_1) --STATE_USER_1 => active
    lv.obj.align(interactiveDialog.background.object, lv.ALIGN_CENTER, 0, 0);

    interactiveDialog.leftContent.object = lv.img.new(parentContainer)
    if (interactiveDialog.leftContent.path ~= nil) then
        interactiveDialog.leftContent.data = Global.load_image(interactiveDialog.leftContent.path)
    end
    lv.img.set_src(interactiveDialog.leftContent.object, interactiveDialog.leftContent.data)
    lv.obj.add_style(interactiveDialog.leftContent.object, interactiveDialog.styles.imageContentInactive, lv
        .STATE_DEFAULT)
    lv.obj.add_style(interactiveDialog.leftContent.object, interactiveDialog.styles.imageContentActive, lv.STATE_USER_1)
    lv.obj.align(interactiveDialog.leftContent.object, lv.ALIGN_LEFT_MID, 0, 0);

    interactiveDialog.rightContent.object = lv.img.new(parentContainer)
    if (interactiveDialog.leftContent.path ~= nil) then
        interactiveDialog.rightContent.data = Global.load_image(interactiveDialog.rightContent.path)
    end

    lv.img.set_src(interactiveDialog.rightContent.object, interactiveDialog.rightContent.data)
    lv.obj.add_style(interactiveDialog.rightContent.object, interactiveDialog.styles.imageContentInactive, lv
        .STATE_DEFAULT)
    lv.obj.add_style(interactiveDialog.rightContent.object, interactiveDialog.styles.imageContentActive, lv.STATE_USER_1)
    lv.obj.align(interactiveDialog.rightContent.object, lv.ALIGN_RIGHT_MID, 0, 0);

    interactiveDialog.computeContentState()
end

function interactiveDialog.addAnswerEntry(container, entry, pos)
    local answerButton = lv.btn.new(container)
    lv.obj.remove_style_all(answerButton)
    lv.obj.add_style(answerButton, interactiveDialog.styles.answerButton, lv.STATE_DEFAULT)

    interactiveDialog.answers[answerButton] = {}
    interactiveDialog.answers[answerButton].img = Global.load_image(entry.image)


    interactiveDialog.answers[answerButton].labelObject = lv.label.new(answerButton)
    lv.obj.remove_style_all(interactiveDialog.answers[answerButton].labelObject)
    lv.obj.add_style(interactiveDialog.answers[answerButton].labelObject, interactiveDialog.styles.answerLabel, lv
        .STATE_DEFAULT)
    lv.label.set_text(interactiveDialog.answers[answerButton].labelObject, entry.label)
    lv.obj.align(interactiveDialog.answers[answerButton].labelObject, lv.ALIGN_LEFT_MID, 24, 0)

    interactiveDialog.answers[answerButton].imageObject = lv.img.new(answerButton)
    interactiveDialog.answers[answerButton].data = Global.load_image(entry.image)
    lv.img.set_src(interactiveDialog.answers[answerButton].imageObject, interactiveDialog.answers[answerButton].data)
    lv.obj.align(interactiveDialog.answers[answerButton].imageObject, lv.ALIGN_RIGHT_MID, -10, 0)

    interactiveDialog.answers[answerButton].audio = entry.audio

    if (entry.scene.leftContent ~= nil) then
        interactiveDialog.answers[answerButton].leftContentOverride = Global.load_image(entry.scene.leftContent)
    end
    if (entry.scene.rightContent ~= nil) then
        interactiveDialog.answers[answerButton].rightContentOverride = Global.load_image(entry.scene.rightContent)
    end
    if (entry.scene.background) then
        interactiveDialog.answers[answerButton].backgroundOverride = Global.load_image(entry.scene.background)
    end


    interactiveDialog.answers[answerButton].sceneState = {}
    interactiveDialog.answers[answerButton].sceneState.backgroundState = entry.scene.backgroundState
    interactiveDialog.answers[answerButton].sceneState.leftContentState = entry.scene.leftContentState
    interactiveDialog.answers[answerButton].sceneState.rightContentState = entry.scene.rightContentState

    interactiveDialog.answers[answerButton].focusedAction = function()
        Global.requestAudioPlay({ path = interactiveDialog.answers[answerButton].audio, priority = true })
        if (interactiveDialog.answers[answerButton].leftContentOverride ~= nil) then
            lv.img.set_src(interactiveDialog.leftContent.object, interactiveDialog.answers[answerButton]
                .leftContentOverride)
        else
            lv.img.set_src(interactiveDialog.leftContent.object, interactiveDialog.leftContent.data)
        end
        if (interactiveDialog.answers[answerButton].rightContentOverride ~= nil) then
            lv.img.set_src(interactiveDialog.rightContent.object, interactiveDialog.answers[answerButton]
                .rightContentOverride)
        else
            lv.img.set_src(interactiveDialog.rightContent.object, interactiveDialog.rightContent.data)
        end
        if (interactiveDialog.answers[answerButton].backgroundOverride ~= nil) then
            lv.img.set_src(interactiveDialog.background.object, interactiveDialog.answers[answerButton]
                .backgroundOverride)
        else
            lv.img.set_src(interactiveDialog.background.object, interactiveDialog.background.data)
        end


        interactiveDialog.computeContentState(interactiveDialog.answers[answerButton].sceneState)
    end
    interactiveDialog.answers[answerButton].defocusedAction = function()
    end
    interactiveDialog.answers[answerButton].clickedAction = function()
        entry.cb()
    end

    interactiveDialog.addEvents(answerButton)
    if (interactiveDialog.first) then
        lv.group.focus_obj(answerButton)
        interactiveDialog.first = false
    end
end

function interactiveDialog.registerSteps(order, step)
    interactiveDialog.steps[order] = {}
    interactiveDialog.steps[order].timeCode = step.timeCode
    if (step.scene.leftContent ~= nil) then
        interactiveDialog.steps[order].leftContentOverride = Global.load_image(step.scene.leftContent)
    end
    if (step.scene.rightContent ~= nil) then
        interactiveDialog.steps[order].rightContentOverride = Global.load_image(step.scene.rightContent)
    end
    if (step.scene.background) then
        interactiveDialog.steps[order].backgroundOverride = Global.load_image(step.scene.background)
    end

    if (step.scene.leftRessource ~= nil) then
        interactiveDialog.steps[order].leftRessource = step.scene.leftRessource
    end
    if (step.scene.rightRessource ~= nil) then
        interactiveDialog.steps[order].rightRessource = step.scene.rightRessource
    end
    if (step.scene.backRessource ~= nil) then
        interactiveDialog.steps[order].backRessource = step.scene.backRessource
    end

    interactiveDialog.steps[order].sceneState = {}
    interactiveDialog.steps[order].sceneState.backgroundState = step.scene.backgroundState
    interactiveDialog.steps[order].sceneState.leftContentState = step.scene.leftContentState
    interactiveDialog.steps[order].sceneState.rightContentState = step.scene.rightContentState
    interactiveDialog.steps[order].consumed = false
    interactiveDialog.steps[order].action = function()
        interactiveDialog.steps[order].consumed = true
        if (interactiveDialog.steps[order].leftContentOverride ~= nil) then
            lv.img.set_src(interactiveDialog.leftContent.object, interactiveDialog.steps[order]
                .leftContentOverride)
        else
            lv.img.set_src(interactiveDialog.leftContent.object, interactiveDialog.leftContent.data)
        end
        if (interactiveDialog.steps[order].rightContentOverride ~= nil) then
            lv.img.set_src(interactiveDialog.rightContent.object, interactiveDialog.steps[order]
                .rightContentOverride)
        else
            lv.img.set_src(interactiveDialog.rightContent.object, interactiveDialog.rightContent.data)
        end
        if (interactiveDialog.steps[order].backgroundOverride ~= nil) then
            lv.img.set_src(interactiveDialog.background.object, interactiveDialog.steps[order]
                .backgroundOverride)
        else
            lv.img.set_src(interactiveDialog.background.object, interactiveDialog.background.data)
        end

        if (interactiveDialog.steps[order].leftRessource ~= nil) then
            lv.img.set_src(interactiveDialog.leftContent.object,
                interactiveDialog.ressources[interactiveDialog.steps[order].leftRessource])
        else
            lv.img.set_src(interactiveDialog.leftContent.object, interactiveDialog.leftContent.data)
        end
        if (interactiveDialog.steps[order].rightRessource ~= nil) then
            lv.img.set_src(interactiveDialog.rightContent.object,
                interactiveDialog.ressources[interactiveDialog.steps[order].rightRessource])
        else
            lv.img.set_src(interactiveDialog.rightContent.object, interactiveDialog.rightContent.data)
        end
        if (interactiveDialog.steps[order].backRessource ~= nil) then
            lv.img.set_src(interactiveDialog.background.object,
                interactiveDialog.ressources[interactiveDialog.steps[order].backRessource])
        else
            lv.img.set_src(interactiveDialog.background.object, interactiveDialog.background.data)
        end


        interactiveDialog.computeContentState(interactiveDialog.steps[order].sceneState)
    end
end

function interactiveDialog.create(args)
    lv.group.set_editing(document, false)
    lv.group.set_wrap(document, false)
    interactiveDialog.initStyles()

    local parentContainer = lv.obj.new(window)
    lv.obj.remove_style_all(parentContainer)
    lv.obj.set_size(parentContainer, lv.obj.get_width(window), lv.obj.get_height(window))

    interactiveDialog.cb = args.scene.cb

    interactiveDialog.background.path = args.scene.background
    interactiveDialog.leftContent.path = args.scene.leftContent
    interactiveDialog.rightContent.path = args.scene.rightContent

    if (args.scene.backgroundState ~= nil) then
        interactiveDialog.sceneState.backgroundState = args.scene.backgroundState
    end
    if (args.scene.leftContentState ~= nil) then
        interactiveDialog.sceneState.leftContentState = args.scene.leftContentState
    end
    if (args.scene.rightContentState ~= nil) then
        interactiveDialog.sceneState.rightContentState = args.scene.rightContentState
    end

    local nbEntry = -1
    interactiveDialog.renderScene(parentContainer)

    if (args.answers ~= nil) then
        local answerContainer = lv.obj.new(parentContainer)
        lv.obj.remove_style_all(answerContainer)
        lv.obj.set_size(answerContainer, lv.obj.get_width(window), math.floor(lv.obj.get_height(window) * .37))
        lv.obj.align(answerContainer, lv.ALIGN_BOTTOM_MID, 0, 0)
        lv.obj.add_style(answerContainer, interactiveDialog.styles.answerContainer, lv.STATE_DEFAULT)

        lv.obj.set_scroll_snap_y(answerContainer, lv.SCROLL_SNAP_START)
        lv.obj.set_flex_flow(answerContainer, lv.FLEX_FLOW_COLUMN)
        lv.obj.set_flex_align(answerContainer, lv.FLEX_ALIGN_START, lv.FLEX_ALIGN_CENTER, lv.FLEX_ALIGN_CENTER)

        interactiveDialog.selector.data = Global.load_image("script/corner.lif")
        local selector = lv.img.new(parentContainer)
        lv.img.set_src(selector, interactiveDialog.selector.data)
        lv.img.set_angle(selector, 2250)
        lv.obj.align(selector, lv.ALIGN_BOTTOM_LEFT, 0, -35)

        nbEntry = 0
        for i, v in pairs(args.answers) do
            if (v.show == nil or v.show == true) then
                interactiveDialog.addAnswerEntry(answerContainer, v, i)
                nbEntry = nbEntry + 1
            end
        end

        lv.obj.update_snap(answerContainer, lv.ANIM_ON);
    elseif (args.steps ~= nil) then
        interactiveDialog.stepMode = true
        for i, ressource in pairs(args.ressources) do
            interactiveDialog.ressources[i] = Global.load_image(ressource.path)
        end
        interactiveDialog.steps = {}
        for order, step in pairs(args.steps) do
            interactiveDialog.registerSteps(order, step)
        end
    end
    -- interactiveDialog.debug = lv.label.new(parentContainer)
    -- lv.label.set_text(interactiveDialog.debug, "Debug")
    if (args.scene.audio ~= nil) then
        Global.requestAudioPlay({ path = args.scene.audio, AFCb = interactiveDialog.audioCallback, priority = true })
    end
    if (nbEntry == 0) then
        print("POUET")
        args.exitcb()
    end
end

return interactiveDialog
