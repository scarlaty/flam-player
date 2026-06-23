--- This module il used to render a list with sound over, sound intro and image
---@class diceRoll
local diceRoll = {}
diceRoll.styles = {}
--- Keep pointer on registered events for cleaning
diceRoll.events = {}
-- keep pointer on registered animation for cleaning
diceRoll.animations = {}
-- store images data
diceRoll.diceFaces = {}

diceRoll.dice = nil
diceRoll.diceData = nil
diceRoll.bonus = nil
diceRoll.bonusData = nil
diceRoll.parentContainer = nil
diceRoll.victory = false
diceRoll.victoryTitle = nil
diceRoll.defeatTitle = nil
diceRoll.result = nil
diceRoll.resultDice = nil
diceRoll.resultAudio = nil
diceRoll.rollAudio = nil
diceRoll.rolled = false
diceRoll.exit = nil

function diceRoll.initSyles()
    diceRoll.styles.parentStyle = lv.style.new()

    lv.style.set_bg_color(diceRoll.styles.parentStyle, lv.color.hex(0x000000))
    lv.style.set_bg_opa(diceRoll.styles.parentStyle, lv.OPA_COVER)

    diceRoll.styles.image = lv.style.new()
    lv.style.set_img_opa(diceRoll.styles.image ,255)

    diceRoll.styles.bonusImage = lv.style.new()
    lv.style.set_img_opa(diceRoll.styles.bonusImage ,255)


    diceRoll.styles.title = lv.style.new()
    lv.style.set_text_color(diceRoll.styles.title, lv.color.hex(0xefedea))
    lv.style.set_text_font(diceRoll.styles.title, lv.font.nunito_extrabold_20)
    lv.style.set_pad_top(diceRoll.styles.title, 16)

    diceRoll.styles.subtitle = lv.style.new()
    lv.style.set_text_color(diceRoll.styles.subtitle, lv.color.hex(0xefedea))
    lv.style.set_text_font(diceRoll.styles.subtitle, lv.font.nunito_extrabold_14)
    lv.style.set_pad_top(diceRoll.styles.subtitle, 45)
end

function diceRoll.clean()
    print("Cleaning dice roll")
    for anim, animVar in pairs(diceRoll.animations) do
        --lv.anim_var.del(anim)
        lv.anim_var.del(animVar.var)
    end
    diceRoll.diceFaces = {}
    diceRoll.dice = nil
end
function diceRoll.showResult()
    Global.requestAudioPlay({path = diceRoll.resultAudio})
    
    lv.img.set_src(diceRoll.dice, diceRoll.resultDice)
    if(diceRoll.victory) then
        lv.label.set_text(diceRoll.titleLabel, diceRoll.victoryTitle)
        lv.label.set_text(diceRoll.subTitleLabel, "tu as fait "..diceRoll.result)
    else
        lv.label.set_text(diceRoll.titleLabel, diceRoll.defeatTitle)
        lv.label.set_text(diceRoll.subTitleLabel, "tu as fait "..diceRoll.result)
    end
    lv.obj.clear_flag(diceRoll.bonus,lv.OBJ_FLAG_HIDDEN)
end
function diceRoll.keyPressed(e)
    if (string.byte(lv.event.get_key_value(e)) == 10 and diceRoll.rolled == false) then
        diceRoll.rolled = true
        
        for anim, animVar in pairs(diceRoll.animations) do
            --lv.anim_var.del(anim)
            lv.anim_var.del(animVar.var)
        end
        Global.requestAudioPlay({path = diceRoll.rollAudio, priority = true})
        local shuffleAnimation = lv.anim.new()
        diceRoll.animations[shuffleAnimation] = {}
        diceRoll.animations[shuffleAnimation].anim = shuffleAnimation
        diceRoll.animations[shuffleAnimation].var = lv.anim.set_var(shuffleAnimation, diceRoll.dice)
        lv.anim.set_values(shuffleAnimation, 255, 0)
        lv.anim.set_time(shuffleAnimation, 500)
        lv.anim.set_playback_time(shuffleAnimation, 500);
        --lv.anim.set_repeat_count(shuffleAnimation, 100);
        --lv.anim.set_early_apply(shuffleAnimation, true);

        lv.anim.set_exec_cb(shuffleAnimation, function(var, val)
            lv.style.set_img_opa(diceRoll.styles.image ,val)    
            lv.style.set_img_opa(diceRoll.styles.bonusImage ,val)
            lv.style.set_text_opa(diceRoll.styles.subtitle,val)
            lv.style.set_text_opa(diceRoll.styles.title,val)
            lv.obj.invalidate(diceRoll.parentContainer)
            if(val == 0) then diceRoll.showResult() end
        end)
        lv.anim.set_path_cb(shuffleAnimation, lv.anim.path_ease_in_out)
        lv.anim.start(shuffleAnimation)
    elseif (string.byte(lv.event.get_key_value(e)) == 10 and diceRoll.rolled == true) then
        diceRoll.exit()
    end
end

function diceRoll.create(args)
    lv.group.set_editing(document, true)
    lv.obj.clean(window) -- Clean main window
    diceRoll.initSyles()

    diceRoll.result = math.random(args.maxValue)
    diceRoll.resultAudio = args.audio.defeatAudio
    diceRoll.exit = args.cbLoose
    if (diceRoll.result + args.bonus>= args.difficulty) then 
        diceRoll.victory = true
        diceRoll.exit = args.cbWin
        diceRoll.resultAudio = args.audio.victoryAudio
    end


    diceRoll.victoryTitle = args.texts.resolveTitle.victoryTitle
    diceRoll.defeatTitle = args.texts.resolveTitle.defeatTitle
    diceRoll.resultDice = Global.load_image(args.diceFaces[diceRoll.result])
    diceRoll.result = diceRoll.result + args.bonus
    diceRoll.rollAudio = args.audio.shuffleAudio

    diceRoll.parentContainer = lv.obj.new(window)
    lv.obj.remove_style_all(diceRoll.parentContainer)
    lv.obj.set_size(diceRoll.parentContainer, lv.obj.get_width(window), lv.obj.get_height(window))
    lv.obj.add_style(diceRoll.parentContainer, diceRoll.styles.parentStyle, lv.STATE_DEFAULT)

    diceRoll.titleLabel = lv.label.new(diceRoll.parentContainer)
    lv.obj.remove_style_all(diceRoll.titleLabel)
    lv.obj.set_width(diceRoll.titleLabel, Global.visual_width)
    lv.label.set_text(diceRoll.titleLabel, args.texts.introTitle)
    lv.obj.align(diceRoll.titleLabel, lv.ALIGN_TOP_MID, 0, 0)
    lv.obj.set_style_text_align(diceRoll.titleLabel, lv.TEXT_ALIGN_CENTER, 0)
    lv.obj.set_width(diceRoll.titleLabel, Global.screen_width - 20);
    lv.obj.add_style(diceRoll.titleLabel, diceRoll.styles.title, lv.STATE_DEFAULT)

    diceRoll.subTitleLabel = lv.label.new(diceRoll.parentContainer)
    lv.obj.remove_style_all(diceRoll.subTitleLabel)
    lv.obj.set_width(diceRoll.subTitleLabel, Global.visual_width)
    lv.label.set_text(diceRoll.subTitleLabel, args.texts.introSubTitle)
    lv.obj.align(diceRoll.subTitleLabel, lv.ALIGN_TOP_MID, 0, 0)
    lv.obj.set_style_text_align(diceRoll.subTitleLabel, lv.TEXT_ALIGN_CENTER, 0)
    lv.obj.set_width(diceRoll.subTitleLabel, Global.screen_width - 20);
    lv.obj.add_style(diceRoll.subTitleLabel, diceRoll.styles.subtitle, lv.STATE_DEFAULT)

    diceRoll.dice = lv.img.new(diceRoll.parentContainer)
    lv.obj.remove_style_all(diceRoll.dice)
    lv.obj.add_flag(diceRoll.dice, lv.OBJ_FLAG_FLOATING)
    lv.obj.align(diceRoll.dice, lv.ALIGN_TOP_MID, 0, 60)
    lv.obj.set_size(diceRoll.dice, 150, 150)
    diceRoll.diceData = Global.load_image(args.introImage)
    lv.img.set_src(diceRoll.dice, diceRoll.diceData)
    lv.obj.add_style(diceRoll.dice, diceRoll.styles.image, lv.STATE_DEFAULT)

    diceRoll.bonus = lv.img.new(diceRoll.parentContainer)
    lv.obj.remove_style_all(diceRoll.bonus)
    lv.obj.add_flag(diceRoll.bonus, lv.OBJ_FLAG_FLOATING)
    lv.obj.align(diceRoll.bonus, lv.ALIGN_TOP_MID, 60, 50)
    lv.obj.set_size(diceRoll.bonus, 70, 90)
    diceRoll.bonusData = Global.load_image(args.diceDecoration[args.bonus])
    lv.img.set_src(diceRoll.bonus, diceRoll.bonusData)
    lv.obj.add_style(diceRoll.bonus, diceRoll.styles.bonusImage, lv.STATE_DEFAULT)
    lv.obj.add_flag(diceRoll.bonus,lv.OBJ_FLAG_HIDDEN)

    local pulseAnimation = lv.anim.new()
    diceRoll.animations[pulseAnimation] = {}
    diceRoll.animations[pulseAnimation].anim = pulseAnimation
    diceRoll.animations[pulseAnimation].var = lv.anim.set_var(pulseAnimation, diceRoll.dice)
    lv.anim.set_values(pulseAnimation, 0, 10)
    lv.anim.set_time(pulseAnimation, 500)
    lv.anim.set_playback_time(pulseAnimation, 500);
    lv.anim.set_repeat_count(pulseAnimation, 100);
    lv.anim.set_early_apply(pulseAnimation, true);

    lv.anim.set_exec_cb(pulseAnimation, function(var, val)
        lv.img.set_zoom(diceRoll.dice, 245 + val) -- 256 = 100%
    end)
    lv.anim.set_path_cb(pulseAnimation, lv.anim.path_ease_in_out)
    lv.anim.start(pulseAnimation)

    diceRoll.events[diceRoll.parentContainer] = {}
    diceRoll.events[diceRoll.parentContainer].key = lv.obj.add_event_cb(diceRoll.parentContainer, diceRoll.keyPressed,
        lv.EVENT_KEY)
    lv.group.add_obj(document, diceRoll.parentContainer)
    Global.requestAudioPlay({path = args.audio.introAudio})
end

return diceRoll
