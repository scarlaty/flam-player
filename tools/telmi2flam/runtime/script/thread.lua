---@class thread
local thread = {}

thread.characters = {}
thread.styles = {}
thread.events = {}
thread.lines = {}
thread.animations = {}
thread.linescroll = 0
thread.exitCb = nil
thread.lineProgress = 1
thread.bufferedScroll = 0
thread.scrollLenght = 170
thread.lineJump = 6
thread.scrolling = false
thread.clickActive = true
thread.pos = 0
thread.linescrolled = 0
thread.lineSize = 24
thread.headerSize = 30
thread.scrollCallback = nil

function thread.clean()
    for anim, animVar in pairs(thread.animations) do
        lv.anim_var.del(animVar.var)
    end
    for container, event in pairs(thread.events) do
        if (event.focused ~= nil) then lv.obj.remove_event_cb(container, event.focused) end
        if (event.defocused ~= nil) then lv.obj.remove_event_cb(container, event.defocused) end
        if (event.clicked ~= nil) then lv.obj.remove_event_cb(container, event.clicked) end
        if (event.key ~= nil) then lv.obj.remove_event_cb(container, event.key) end
    end
    thread.characters = {}
    thread.styles = {}
    thread.events = {}
    thread.lines = {}
end

function thread.initStyles()
    thread.styles.answerContainer = lv.style.new()
    lv.style.set_pad_row(thread.styles.answerContainer, 25)
    lv.style.set_bg_opa(thread.styles.answerContainer, lv.OPA_COVER)
    lv.style.set_bg_color(thread.styles.answerContainer, lv.color.hex(0x000000))

    thread.styles.lineLabelPlayable = lv.style.new()
    lv.style.set_bg_opa(thread.styles.lineLabelPlayable, lv.OPA_TRANSP)
    lv.style.set_text_color(thread.styles.lineLabelPlayable, lv.color.hex(0xffffff))
    lv.style.set_text_font(thread.styles.lineLabelPlayable, lv.font.nunito_extrabold_16)
    lv.style.set_text_line_space(thread.styles.lineLabelPlayable, 4)
    lv.style.set_text_align(thread.styles.lineLabelPlayable, lv.TEXT_ALIGN_LEFT)
    lv.style.set_width(thread.styles.lineLabelPlayable, 220)

    thread.styles.lineTitle = lv.style.new()
    lv.style.set_bg_opa(thread.styles.lineTitle, lv.OPA_TRANSP)
    lv.style.set_text_color(thread.styles.lineTitle, lv.color.hex(0xffffff))
    lv.style.set_text_font(thread.styles.lineTitle, lv.font.nunito_extrabold_16)
    lv.style.set_text_line_space(thread.styles.lineTitle, 4)
    lv.style.set_text_align(thread.styles.lineTitle, lv.TEXT_ALIGN_LEFT)
    lv.style.set_width(thread.styles.lineTitle, 220)

    thread.styles.lineContainer = lv.style.new()
    lv.style.set_bg_opa(thread.styles.lineContainer, lv.OPA_TRANSP)
    lv.style.set_bg_color(thread.styles.lineContainer, lv.color.hex(0x202020))
    lv.style.set_radius(thread.styles.lineContainer, 8)
    lv.style.set_pad_ver(thread.styles.lineContainer, 0)

    thread.styles.lineContainerGreyedOut = lv.style.new()
    lv.style.set_text_color(thread.styles.lineContainerGreyedOut, lv.color.hex(0x5F6769))

    thread.styles.titleGrey = lv.style.new()
    lv.style.set_text_opa(thread.styles.titleGrey, 180)

    thread.styles.entryImage = lv.style.new()
    lv.style.set_img_opa(thread.styles.entryImage, 255)

    thread.styles.entryImageGrey = lv.style.new()
    lv.style.set_img_opa(thread.styles.entryImageGrey, 100)

    thread.styles.lineHeader = {}
    thread.styles.lineHeader[1] = lv.style.new()
    lv.style.set_img_recolor(thread.styles.lineHeader[1], lv.color.hex(0xF8D651))
    lv.style.set_img_recolor_opa(thread.styles.lineHeader[1], 255)
    lv.style.set_bg_opa(thread.styles.lineHeader[1], lv.OPA_TRANSP)
    lv.style.set_text_color(thread.styles.lineHeader[1], lv.color.hex(0xF8D651))
    lv.style.set_text_font(thread.styles.lineHeader[1], lv.font.nunito_extrabold_16)
    lv.style.set_text_line_space(thread.styles.lineHeader[1], 4)
    lv.style.set_text_align(thread.styles.lineHeader[1], lv.TEXT_ALIGN_LEFT)

    thread.styles.lineHeader[2] = lv.style.new()
    lv.style.set_img_recolor(thread.styles.lineHeader[2], lv.color.hex(0xDA9DFF))
    lv.style.set_img_recolor_opa(thread.styles.lineHeader[2], 255)
    lv.style.set_bg_opa(thread.styles.lineHeader[2], lv.OPA_TRANSP)
    lv.style.set_text_color(thread.styles.lineHeader[2], lv.color.hex(0xDA9DFF))
    lv.style.set_text_font(thread.styles.lineHeader[2], lv.font.nunito_extrabold_16)
    lv.style.set_text_line_space(thread.styles.lineHeader[2], 4)
    lv.style.set_text_align(thread.styles.lineHeader[2], lv.TEXT_ALIGN_LEFT)

    thread.styles.lineHeader[3] = lv.style.new()
    lv.style.set_img_recolor(thread.styles.lineHeader[3], lv.color.hex(0x84E3FC))
    lv.style.set_img_recolor_opa(thread.styles.lineHeader[3], 255)
    lv.style.set_bg_opa(thread.styles.lineHeader[3], lv.OPA_TRANSP)
    lv.style.set_text_color(thread.styles.lineHeader[3], lv.color.hex(0x84E3FC))
    lv.style.set_text_font(thread.styles.lineHeader[3], lv.font.nunito_extrabold_16)
    lv.style.set_text_line_space(thread.styles.lineHeader[3], 4)
    lv.style.set_text_align(thread.styles.lineHeader[3], lv.TEXT_ALIGN_LEFT)

    thread.styles.lineHeader[4] = lv.style.new()
    lv.style.set_img_recolor(thread.styles.lineHeader[4], lv.color.hex(0xFF993A))
    lv.style.set_img_recolor_opa(thread.styles.lineHeader[4], 255)
    lv.style.set_bg_opa(thread.styles.lineHeader[4], lv.OPA_TRANSP)
    lv.style.set_text_color(thread.styles.lineHeader[4], lv.color.hex(0xFF993A))
    lv.style.set_text_font(thread.styles.lineHeader[4], lv.font.nunito_extrabold_16)
    lv.style.set_text_line_space(thread.styles.lineHeader[4], 4)
    lv.style.set_text_align(thread.styles.lineHeader[4], lv.TEXT_ALIGN_LEFT)

    thread.styles.lineHeader[5] = lv.style.new()
    lv.style.set_img_recolor(thread.styles.lineHeader[5], lv.color.hex(0x6CD86B))
    lv.style.set_img_recolor_opa(thread.styles.lineHeader[5], 255)
    lv.style.set_bg_opa(thread.styles.lineHeader[5], lv.OPA_TRANSP)
    lv.style.set_text_color(thread.styles.lineHeader[5], lv.color.hex(0x6CD86B))
    lv.style.set_text_font(thread.styles.lineHeader[5], lv.font.nunito_extrabold_16)
    lv.style.set_text_line_space(thread.styles.lineHeader[5], 4)
    lv.style.set_text_align(thread.styles.lineHeader[5], lv.TEXT_ALIGN_LEFT)

    thread.styles.lineHeader[6] = lv.style.new()
    lv.style.set_img_recolor(thread.styles.lineHeader[6], lv.color.hex(0xD0AC8A))
    lv.style.set_img_recolor_opa(thread.styles.lineHeader[6], 255)
    lv.style.set_bg_opa(thread.styles.lineHeader[6], lv.OPA_TRANSP)
    lv.style.set_text_color(thread.styles.lineHeader[6], lv.color.hex(0xD0AC8A))
    lv.style.set_text_font(thread.styles.lineHeader[6], lv.font.nunito_extrabold_16)
    lv.style.set_text_line_space(thread.styles.lineHeader[6], 4)
    lv.style.set_text_align(thread.styles.lineHeader[6], lv.TEXT_ALIGN_LEFT)

    thread.styles.lineHeader[7] = lv.style.new()
    lv.style.set_img_recolor(thread.styles.lineHeader[7], lv.color.hex(0x788DFF))
    lv.style.set_img_recolor_opa(thread.styles.lineHeader[7], 255)
    lv.style.set_bg_opa(thread.styles.lineHeader[7], lv.OPA_TRANSP)
    lv.style.set_text_color(thread.styles.lineHeader[7], lv.color.hex(0x788DFF))
    lv.style.set_text_font(thread.styles.lineHeader[7], lv.font.nunito_extrabold_16)
    lv.style.set_text_line_space(thread.styles.lineHeader[7], 4)
    lv.style.set_text_align(thread.styles.lineHeader[7], lv.TEXT_ALIGN_LEFT)

    thread.styles.lineHeader[8] = lv.style.new()
    lv.style.set_img_recolor(thread.styles.lineHeader[8], lv.color.hex(0xFF5468))
    lv.style.set_img_recolor_opa(thread.styles.lineHeader[8], 255)
    lv.style.set_bg_opa(thread.styles.lineHeader[8], lv.OPA_TRANSP)
    lv.style.set_text_color(thread.styles.lineHeader[8], lv.color.hex(0xFF5468))
    lv.style.set_text_font(thread.styles.lineHeader[8], lv.font.nunito_extrabold_16)
    lv.style.set_text_line_space(thread.styles.lineHeader[8], 4)
    lv.style.set_text_align(thread.styles.lineHeader[8], lv.TEXT_ALIGN_LEFT)

    thread.styles.arrowActive = lv.style.new()
    lv.style.set_img_opa(thread.styles.arrowActive, 255)

    thread.styles.arrowInactive = lv.style.new()
    lv.style.set_img_opa(thread.styles.arrowInactive, 0)
end

function thread.loadCharacters(characters)
    for i, v in ipairs(characters) do
        table.insert(thread.characters, v)
        characters[i].imgData = Global.load_image(characters[i].image)
    end
end

function thread.lineReadCallback(state, seconds)
    if (state == "stop") then
        if (thread.scrolling == true) then
            thread.scrollCallback = function() thread.autoNext() end
        else
            thread.autoNext()
        end
    end
end

function thread.insertLine(parent, line, pos)
    local entryButton = lv.btn.new(parent)
    line.entry = entryButton
    lv.obj.remove_style_all(entryButton)
    lv.obj.add_style(entryButton, thread.styles.lineContainer, lv.STATE_DEFAULT)
    lv.obj.set_width(entryButton, lv.obj.get_width(window))

    line.lineImage = lv.img.new(entryButton)
    lv.img.set_src(line.lineImage, thread.characters[line.id].imgData)
    lv.img.set_zoom(line.lineImage, 256)
    lv.obj.align(line.lineImage, lv.ALIGN_TOP_LEFT, 0, 0)
    lv.obj.add_style(line.lineImage, thread.styles.entryImage, lv.STATE_DEFAULT)
    lv.obj.add_style(line.lineImage, thread.styles.entryImageGrey, lv.STATE_USER_1)



    line.len = string.len(line.text) / thread.lineSize -- Amount of lines in text

    line.lineIcon = lv.img.new(entryButton)
    if (thread.characters[line.id].playable) then
        lv.img.set_src(line.lineIcon, thread.playerIcon)
    else
        lv.img.set_src(line.lineIcon, thread.flamIcon)
    end
    lv.img.set_zoom(line.lineIcon, 256)
    lv.obj.align(line.lineIcon, lv.ALIGN_TOP_LEFT, 60, 0)

    lv.obj.add_style(line.lineIcon, thread.styles.lineHeader[line.id], lv.STATE_DEFAULT)
    lv.obj.add_style(line.lineIcon, thread.styles.entryImageGrey, lv.STATE_USER_1)


    line.Title = lv.label.new(entryButton)
    lv.label.set_text(line.Title, thread.characters[line.id].name)
    lv.obj.align(line.Title, lv.ALIGN_TOP_LEFT, 100, 0)
    lv.obj.add_style(line.Title, thread.styles.lineHeader[line.id], lv.STATE_DEFAULT)
    lv.obj.add_style(line.Title, thread.styles.titleGrey, lv.STATE_USER_1)

    line.textLabel = lv.label.new(entryButton)
    lv.obj.remove_style_all(line.textLabel)
    lv.label.set_long_mode(line.textLabel, lv.LABEL_LONG_WRAP)
    lv.label.set_text(line.textLabel, line.text)

    lv.obj.align(line.textLabel, lv.ALIGN_TOP_LEFT, 60, thread.headerSize)
    lv.obj.add_style(line.textLabel, thread.styles.lineLabelPlayable, lv.STATE_DEFAULT)
    lv.obj.add_style(line.textLabel, thread.styles.lineContainerGreyedOut, lv.STATE_USER_1)

    if (pos ~= 1) then
        lv.obj.add_state(line.textLabel, lv.STATE_USER_1)
        lv.obj.add_state(line.lineImage, lv.STATE_USER_1)
        lv.obj.add_state(line.Title, lv.STATE_USER_1)
        lv.obj.add_state(line.lineIcon, lv.STATE_USER_1)
    end
end

function thread.scrollStart()
    thread.scrolling = true
end

function thread.scrollEnd()
    thread.scrolling = false
    if (thread.scrollCallback ~= nil) then
        thread.scrollCallback()
        thread.scrollCallback = nil
        print("scroll callback used")
    end
end

function thread.scrollNext()
    if (thread.bufferedScroll ~= 0) then
        if (thread.bufferedScroll < thread.lineJump) then
            lv.obj.scroll_by(thread.AnswersContainer, 0,
                -thread.lineSize * thread.bufferedScroll,
                lv.ANIM_ON)
            thread.linescrolled = thread.linescrolled + thread.bufferedScroll
            thread.bufferedScroll = 0
        else
            lv.obj.scroll_by(thread.AnswersContainer, 0,
                -thread.lineSize * thread.lineJump,
                lv.ANIM_ON)
            thread.linescrolled = thread.linescrolled + thread.lineJump
            thread.bufferedScroll = thread.bufferedScroll - thread.lineJump
        end
        thread.refreshFocus()
    end
end

function thread.scrollBack()
    if (thread.linescrolled > 3) then
        lv.obj.scroll_by(thread.AnswersContainer, 0,
            thread.lineSize * 3,
            lv.ANIM_ON)
        thread.linescrolled = thread.linescrolled - 3
        thread.bufferedScroll = thread.bufferedScroll + 3
    elseif (thread.linescrolled > 0) then
        lv.obj.scroll_by(thread.AnswersContainer, 0,
            thread.lineSize,
            lv.ANIM_ON)
        thread.linescrolled = thread.linescrolled - 1
        thread.bufferedScroll = thread.bufferedScroll + 1
    end
    thread.refreshFocus()
end

function thread.clicked()
    if (thread.clickActive == true and thread.scrolling == false) then
        thread.nextLine()
    end
end

function thread.nextLine()
    thread.linescrolled = 0
    if (thread.pos < #thread.lines) then
        thread.pos = thread.pos + 1
        if (lv.obj.get_height(thread.lines[thread.pos].entry) < thread.scrollLenght) then
            lv.obj.scroll_by(thread.AnswersContainer, 0,
                -(lv.obj.get_height(thread.lines[thread.pos].entry) + 25),
                lv.ANIM_ON)
        else
            thread.bufferedScroll = math.ceil((lv.obj.get_height(thread.lines[thread.pos].entry) - (thread.headerSize - 4)) /
                thread.lineSize)
            lv.obj.scroll_by(thread.AnswersContainer, 0,
                -21 - (thread.headerSize + thread.lineJump * thread.lineSize),
                lv.ANIM_ON)
            thread.bufferedScroll = thread.bufferedScroll - thread.lineJump
        end
        thread.refreshFocus()
        thread.processStep(thread.pos)
    else
        thread.exitCb()
    end
end

function thread.refreshFocus()
    for i, v in ipairs(thread.lines) do
        if (i == thread.pos) then
            lv.obj.clear_state(v.textLabel, lv.STATE_USER_1)
            lv.obj.clear_state(v.lineImage, lv.STATE_USER_1)
            lv.obj.clear_state(v.Title, lv.STATE_USER_1)
            lv.obj.clear_state(v.lineIcon, lv.STATE_USER_1)
        else
            lv.obj.add_state(v.textLabel, lv.STATE_USER_1)
            lv.obj.add_state(v.lineImage, lv.STATE_USER_1)
            lv.obj.add_state(v.Title, lv.STATE_USER_1)
            lv.obj.add_state(v.lineIcon, lv.STATE_USER_1)
        end
    end
    if (thread.bufferedScroll == 0 and thread.characters[thread.lines[thread.pos].id].playable) then
        lv.obj.clear_flag(thread.arrowImage, lv.OBJ_FLAG_HIDDEN)
        lv.obj.invalidate(thread.arrowImage)
        thread.clickActive = true
    else
        lv.obj.add_flag(thread.arrowImage, lv.OBJ_FLAG_HIDDEN)
        lv.obj.invalidate(thread.arrowImage)
        thread.clickActive = false
    end
end

function thread.processStep(pos)
    if (thread.characters[thread.lines[pos].id].playable == true) then
    else
        thread.clickActive = false
        Global.requestAudioPlay({ path = thread.lines[pos].audio, priority = true, AFCb = thread.lineReadCallback })
    end
end

function thread.autoNext()
    if (thread.bufferedScroll == 0) then
        thread.nextLine()
    else
        if (thread.pos < #thread.lines) then
            thread.pos = thread.pos + 1
            if (lv.obj.get_height(thread.lines[thread.pos].entry) < thread.scrollLenght) then
                lv.obj.scroll_by(thread.AnswersContainer, 0,
                    -(lv.obj.get_height(thread.lines[thread.pos].entry)) - (thread.lineSize * thread.bufferedScroll) -
                    25
                    ,
                    lv.ANIM_ON)
                thread.bufferedScroll = 0
            else
                lv.obj.scroll_by(thread.AnswersContainer, 0,
                    -(thread.lineSize * (thread.bufferedScroll) + 21 + (thread.headerSize + thread.lineJump * thread.lineSize)),
                    lv.ANIM_ON)
                thread.bufferedScroll = math.ceil((lv.obj.get_height(thread.lines[thread.pos].entry) - (thread.headerSize - 4)) /
                    thread.lineSize)
                thread.bufferedScroll = thread.bufferedScroll - thread.lineJump
            end
            thread.refreshFocus()
            thread.processStep(thread.pos)
        end
    end
end

function thread.keyPressed(e)
    if (thread.scrolling) then
    else
        if (string.byte(lv.event.get_key_value(e)) == 19) then
            thread.scrollNext()
        elseif (string.byte(lv.event.get_key_value(e)) == 20) then
            thread.scrollBack()
        end
    end
end

function thread.createArrow()
    thread.arrowImage = lv.img.new(window)
    lv.img.set_src(thread.arrowImage, thread.arrowIcon)
    lv.img.set_zoom(thread.arrowImage, 256)
    lv.img.set_angle(thread.arrowImage, -450)
    lv.obj.align(thread.arrowImage, lv.ALIGN_BOTTOM_RIGHT, -30, -10)
    lv.obj.add_style(thread.arrowImage, thread.styles.entryImage, lv.STATE_DEFAULT)
    lv.obj.add_style(thread.arrowImage, thread.styles.entryImageGrey, lv.STATE_USER_1)
    local arrowAnimation = lv.anim.new()
    thread.animations[window] = {}
    thread.animations[window].anim = arrowAnimation
    thread.animations[window].var = lv.anim.set_var(arrowAnimation,
        thread.arrowImage)
    lv.anim.set_values(arrowAnimation, 0, 5)
    lv.anim.set_time(arrowAnimation, 5)
    lv.anim.set_playback_time(arrowAnimation, 500);
    lv.anim.set_repeat_count(arrowAnimation, lv.ANIM_REPEAT_INFINITE);
    lv.anim.set_early_apply(arrowAnimation, true);
    lv.anim.set_exec_cb(arrowAnimation, function(var, val)
        lv.obj.set_style_translate_y(thread.arrowImage, val, lv.STATE_DEFAULT)
        lv.obj.invalidate(thread.arrowImage)
    end)

    lv.anim.set_path_cb(arrowAnimation, lv.anim.path_ease_in_out)
    lv.anim.start(arrowAnimation)
    lv.obj.add_flag(thread.arrowImage, lv.OBJ_FLAG_HIDDEN)
end

function thread.create(args)
    lv.group.set_editing(document, true)
    lv.group.set_wrap(document, false)
    thread.initStyles()
    thread.loadCharacters(args.characters)
    thread.playerIcon = Global.load_image("script/player_icon.lif")
    thread.flamIcon = Global.load_image("script/flam_icon.lif")
    thread.arrowIcon = Global.load_image("script/corner.lif")
    thread.exitCb = args.exitCb
    thread.lines = args.lines


    thread.AnswersContainer = lv.obj.new(window)
    lv.obj.remove_style_all(thread.AnswersContainer)
    lv.obj.set_size(thread.AnswersContainer, lv.obj.get_width(window) - thread.lineSize, lv.obj.get_height(window))
    lv.obj.set_flex_flow(thread.AnswersContainer, lv.FLEX_FLOW_COLUMN)
    lv.obj.add_style(thread.AnswersContainer, thread.styles.answerContainer, lv.STATE_DEFAULT)
    lv.obj.align(thread.AnswersContainer, lv.ALIGN_CENTER, 0, 0)
    lv.obj.clear_flag(thread.AnswersContainer, lv.OBJ_FLAG_SCROLLABLE)




    for i, v in ipairs(thread.lines) do
        thread.insertLine(thread.AnswersContainer, v, i)
    end

    thread.createArrow()
    if (thread.lines[1].playable and lv.obj.get_height(thread.lines[1].entry < thread.scrollLenght)) then
        lv.obj.clear_flag(thread.arrowImage, lv.OBJ_FLAG_HIDDEN)
    end
    lv.obj.scroll_by(thread.AnswersContainer, 0,
        lv.obj.get_height(window),
        lv.ANIM_OFF)

    thread.autoNext()

    thread.events[thread.AnswersContainer] = {}
    thread.events[thread.AnswersContainer].clicked = lv.obj.add_event_cb(thread.AnswersContainer,
        thread.clicked, lv.EVENT_CLICKED)
    thread.events[thread.AnswersContainer].key = lv.obj.add_event_cb(thread.AnswersContainer,
        thread.keyPressed, lv.EVENT_KEY)
    thread.events[thread.AnswersContainer].scrollStart = lv.obj.add_event_cb(thread.AnswersContainer,
        thread.scrollStart, lv.EVENT_SCROLL_BEGIN)
    thread.events[thread.AnswersContainer].scrollEnd = lv.obj.add_event_cb(thread.AnswersContainer,
        thread.scrollEnd, lv.EVENT_SCROLL_END)

    lv.group.remove_all_objs(document)
    lv.group.add_obj(document, thread.AnswersContainer)
end

return thread
