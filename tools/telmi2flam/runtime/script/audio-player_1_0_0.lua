--- This module il used to render a list with sound over, sound intro and image
---@class audioPlayer
local audioPlayer = {}
--- Stores all lvgl style rules
audioPlayer.styles = {}
--- Keep ref on registered events for cleaning
audioPlayer.events = {}
-- keep ref on registered animation for cleaning
audioPlayer.animations = {}
-- keep ref on registered images to avoir garbage collection
audioPlayer.images = {}
-- ref on title lVGL label object
audioPlayer.titleLabel = nil
-- LVGL timmer, used to detect end of interraction and swap audioPlayer views
audioPlayer.inactivity_timer = nil
-- LVGL main container
audioPlayer.parentContainer = nil
-- boolean for view state (small/large view)
audioPlayer.isLargeView = true
-- ref on lvgl slider object
audioPlayer.slider = nil
-- store pause state
audioPlayer.paused = false
-- Boolean initalized at false until first audio-feedback trigger ( used to enable user interaction)
audioPlayer.audioStarted = false
-- Amount of seconds to seek at each step
audioPlayer.seekingSpeed = 1
-- store last seek direction for seek speed reset
audioPlayer.lastSeekDirection = 0
-- store seek value between slider and audioFeedback (used to synchronize UI and player)
-- nil = aucun seek en attente (0 est une position valide : retour au debut)
audioPlayer.seekValue = nil
-- Store interraction state ( seeking mode on/off)
audioPlayer.seeking = false
-- Store exit callback from args.callback
audioPlayer.exitCallback = nil
-- ref on player cover
audioPlayer.imgPlayer = nil
-- ref on player foreground
audioPlayer.imgForeground = nil
-- ref on player background
audioPlayer.imgBackground = nil
-- store args.ignoreSeek ( used to disable seek restore)
audioPlayer.ignoreSeekSave = nil
-- [telmi2flam] args.okCallback : appele sur OK (ENTER), pendant ou apres l'audio
-- (skip, ctrl.ok TELMI). nil = ENTER ignore.
audioPlayer.okCallback = nil
-- ENTER recu, traite par okTimer et jamais dans l'evenement KEY : le callback
-- recharge un module (lv.obj.clean(window)) et supprimerait l'objet en cours de
-- dispatch (meme principe que carousel.inputProcessTimer)
audioPlayer.okRequested = false
audioPlayer.okTimer = nil
-- ticks de okTimer depuis create (anti-rebond d'entree, ~200 ms)
audioPlayer.okTick = 0
-- true des que la sortie est declenchee (fin d'audio ou OK) : garde anti double appel
audioPlayer.exited = false
-- true apres le 'stop' de fin quand args.callback est nil : on reste sur l'image
-- (scene TELMI non-autoplay) en attendant OK
audioPlayer.ended = false

--- initialize lvgl styles
---@return nil
function audioPlayer.initStyles()
    audioPlayer.styles.parentContainer = lv.style.new()
    lv.style.set_bg_color(audioPlayer.styles.parentContainer, lv.color.hex(0x000000))
    lv.style.set_bg_opa(audioPlayer.styles.parentContainer, lv.OPA_COVER)

    audioPlayer.styles.background = lv.style.new()
    lv.style.set_img_opa(audioPlayer.styles.background, 0)

    audioPlayer.styles.foreground = lv.style.new()
    lv.style.set_img_opa(audioPlayer.styles.foreground, 0)

    audioPlayer.styles.titleLabel = lv.style.new()
    lv.style.set_text_color(audioPlayer.styles.titleLabel, lv.color.hex(0xffffff))
    lv.style.set_text_font(audioPlayer.styles.titleLabel, lv.font.nunito_bold_16)
    lv.style.set_text_opa(audioPlayer.styles.titleLabel, 0)

    audioPlayer.styles.sliderMain = lv.style.new()
    lv.style.set_bg_opa(audioPlayer.styles.sliderMain, 0)
    lv.style.set_bg_color(audioPlayer.styles.sliderMain, lv.color.hex(0x908977))
    lv.style.set_radius(audioPlayer.styles.sliderMain, 2)
    lv.style.set_pad_ver(audioPlayer.styles.sliderMain, 0)

    audioPlayer.styles.sliderIndicator = lv.style.new()
    lv.style.set_bg_opa(audioPlayer.styles.sliderIndicator, 0)
    lv.style.set_bg_color(audioPlayer.styles.sliderIndicator, lv.color.hex(0xfaf9f8))
    lv.style.set_radius(audioPlayer.styles.sliderIndicator, 2)

    audioPlayer.styles.sliderKnob = lv.style.new()
    lv.style.set_bg_opa(audioPlayer.styles.sliderKnob, 0)
    lv.style.set_bg_color(audioPlayer.styles.sliderKnob, lv.color.hex(0xfbbd2a))
    lv.style.set_border_color(audioPlayer.styles.sliderKnob, lv.color.hex(0x000000))
    lv.style.set_border_width(audioPlayer.styles.sliderKnob, 1)
    lv.style.set_radius(audioPlayer.styles.sliderKnob, lv.RADIUS_CIRCLE)
    lv.style.set_pad_all(audioPlayer.styles.sliderKnob, 4)
end

--- Clean module variable and remove lvgl binds
---@return nil
function audioPlayer.clean()
    Global.requestAudioStop(true, true)
    for container, event in pairs(audioPlayer.events) do
        lv.obj.remove_event_cb(container, event.key)
    end
    for anim, animVar in pairs(audioPlayer.animations) do
        --lv.anim_var.del(anim)
        lv.anim_var.del(animVar.var)
    end
    if (audioPlayer.inactivity_timer ~= nil) then
        lv.timer.del(audioPlayer.inactivity_timer)
        audioPlayer.inactivity_timer = nil
    end
    if (audioPlayer.okTimer ~= nil) then
        lv.timer.del(audioPlayer.okTimer)
        audioPlayer.okTimer = nil
    end
    audioPlayer.okCallback = nil
    audioPlayer.okRequested = false

    lv.group.set_editing(document, false)
    lv.group.remove_all_objs(document)
    lv.obj.clean(window)

    audioPlayer.styles = {}
    audioPlayer.events = {}
    audioPlayer.animations = {}
    audioPlayer.images = {}
    audioPlayer.seekRecorder = nil
    audioPlayer.imgBackground = nil
    audioPlayer.imgForeground = nil
    audioPlayer.imgPlayer = nil
end

--- Callback for inactivity_timer ( terminate seek phase and restore largePlayer after user input )
---@return nil
function audioPlayer.inactivityCb()
    if (audioPlayer.paused == false) then
        audioPlayer.showLargePlayer()
    end
    -- audio termine (attente OK) : pas de seek, pause/seek/play relancerait la lecture
    if (audioPlayer.seekValue ~= nil and audioPlayer.ended == false) then
        -- [telmi2flam] Sur device, audio.seek() PENDANT la lecture gele l'appareil
        -- (ecran noir, lv_timer_handler mort - issue #1). Sequence sure, validee
        -- sur device : pause -> seek -> reprise (uniquement si on jouait).
        local wasPlaying = (audioPlayer.paused == false)
        audio.pause()
        audio.seek(audioPlayer.seekValue)
        if (wasPlaying) then audio.play() end
        print("audio-player_1_0_0.lua: info:  Seeking ...")
        audioPlayer.seekValue = nil
        audioPlayer.seeking = false
    end
end

--- Callback for global.audioFeedback is called each seconds by firmware using global, Allow audio state following
---@return nil
function audioPlayer.audioFeedback(audioState, seconds)
    local duration = audio.duration()
    if (duration ~= nil and duration ~= 0) then
        -- range >= 1 : un audio de moins d'1 s donnerait floor=0 => slider de range 0
        -- (division par zero dans lv_bar sur le LVGL non patche du device)
        lv.slider.set_range(audioPlayer.slider, 0, (duration >= 1) and math.floor(duration) or 1)
    end

    if (audioPlayer.audioStarted == false) then
        audioPlayer.audioStarted = true

        -- pas de reprise de position sur un "stop" (fin immediate ou audio.load en echec)
        if (audioState ~= "stop" and (audioPlayer.ignoreSeekSave == nil or audioPlayer.ignoreSeekSave == false)) then
            if (state.visited_funs[state.current_fun] ~= nil) then
                if (state.visited_funs[state.current_fun].seekposition ~= nil) then
                    -- [telmi2flam] pause -> seek -> play : seek en lecture gele le
                    -- device (issue #1). C'est ce seek de reprise qui figeait l'ecran
                    -- noir au 2e lancement (reprise au milieu d'une scene audio).
                    audio.pause()
                    audio.seek(state.visited_funs[state.current_fun].seekposition)
                    audio.play()
                    print("audio-player_1_0_0.lua: info:  Seeking saved seekPosition...")
                end
            end
        end
    end
    if (audioPlayer.ignoreSeekSave == nil or audioPlayer.ignoreSeekSave == false) then
        if (state.visited_funs[state.current_fun] ~= nil) then
            state.visited_funs[state.current_fun].seekposition = seconds
        end
    end

    if (audioPlayer.seeking == false) then
        lv.slider.set_value(audioPlayer.slider, math.floor(seconds), lv.ANIM_OFF)
    end


    if (audioState == "pause") then
        audioPlayer.paused = true
        audioPlayer.showMiniPlayer()
    else
        audioPlayer.paused = false
    end

    if (audioState == "stop") then
        -- Fin de lecture -> on passe au noeud suivant (selection / scene). On NE
        -- filtre PLUS sur `seconds` : le device emet le "stop" de fin avec
        -- seconds ~= 0, et l'ancienne garde anti-racing (ignorer seconds<0.5)
        -- mangeait ce "stop" legitime => l'histoire ne passait pas a la selection.
        -- Le "racing" d'origine venait du seek-en-lecture (corrige : pause/seek/play).
        if (state.visited_funs[state.current_fun] ~= nil) then
            state.visited_funs[state.current_fun].seekposition = nil
        end
        if (audioPlayer.exitCallback ~= nil) then
            audioPlayer.leave(audioPlayer.exitCallback)
        else
            -- [telmi2flam] pas de callback de fin (non-autoplay) : on reste sur
            -- l'image, seek desactive, OK (okCallback) fait avancer
            audioPlayer.ended = true
            audioPlayer.seeking = false
            audioPlayer.seekValue = nil
        end
    end
end

--- [telmi2flam] Sortie unique du module (fin d'audio ou OK) : garde anti double appel
---@return nil
function audioPlayer.leave(cb)
    if (audioPlayer.exited or cb == nil) then
        return
    end
    audioPlayer.exited = true
    cb()
end

--- [telmi2flam] Timer okTimer : traite l'ENTER memorise par keyPressed
---@return nil
function audioPlayer.processOk()
    -- anti-rebond d'entree (~200 ms) : ignore l'ENTER qui a valide l'ecran precedent
    if (audioPlayer.okTick <= 1) then
        audioPlayer.okTick = audioPlayer.okTick + 1
        audioPlayer.okRequested = false
        return
    end
    if (audioPlayer.okRequested) then
        audioPlayer.okRequested = false
        -- scene quittee : pas de reprise au milieu de cet audio
        if (state.visited_funs ~= nil and state.visited_funs[state.current_fun] ~= nil) then
            state.visited_funs[state.current_fun].seekposition = nil
        end
        audioPlayer.leave(audioPlayer.okCallback)
    end
end

--- Borne une position de seek entre 0 et la fin de l'audio (si la duree est connue)
---@return number
local function clampSeek(value)
    local duration = audio.duration()
    if (value < 0) then
        value = 0
    end
    if (duration ~= nil and duration > 0 and value > math.floor(duration)) then
        value = math.floor(duration)
    end
    return value
end

--- KeyPressed event computing
---@return nil
function audioPlayer.keyPressed(e)
    -- [telmi2flam] OK (ENTER) : memorise, traite par okTimer (voir processOk)
    if (string.byte(lv.event.get_key_value(e)) == 10) then
        if (audioPlayer.okCallback ~= nil) then
            audioPlayer.okRequested = true
        end
        return
    end
    if (audioPlayer.audioStarted and audioPlayer.ended == false) then
        local wasSeeking = false
        if (audioPlayer.isLargeView) then
            audioPlayer.showMiniPlayer()
            audioPlayer.isLargeView = false
        else
            wasSeeking = true
        end
        if (wasSeeking and audioPlayer.lastSeekDirection == string.byte(lv.event.get_key_value(e))) then
            audioPlayer.seekingSpeed = audioPlayer.seekingSpeed + 1
        else
            audioPlayer.seekingSpeed = 1
        end

        if (string.byte(lv.event.get_key_value(e)) == 19) then
            audioPlayer.seekValue = clampSeek(lv.slider.get_value(audioPlayer.slider) + audioPlayer.seekingSpeed)
            lv.slider.set_value(audioPlayer.slider, audioPlayer.seekValue, lv.ANIM_OFF)
            audioPlayer.seeking = true
            audioPlayer.lastSeekDirection = 19
        end
        if (string.byte(lv.event.get_key_value(e)) == 20) then
            audioPlayer.seekValue = clampSeek(lv.slider.get_value(audioPlayer.slider) - audioPlayer.seekingSpeed)
            lv.slider.set_value(audioPlayer.slider, audioPlayer.seekValue, lv.ANIM_OFF)
            audioPlayer.seeking = true
            audioPlayer.lastSeekDirection = 20
        end

        lv.timer.reset(audioPlayer.inactivity_timer)
    end
end

--- update view to show Large player
---@return nil
function audioPlayer.showLargePlayer()
    lv.style.set_img_opa(audioPlayer.styles.background, 255)
    lv.style.set_img_opa(audioPlayer.styles.foreground, 255)
    lv.style.set_text_opa(audioPlayer.styles.titleLabel, 255)
    lv.style.set_bg_opa(audioPlayer.styles.sliderMain, 0)
    lv.style.set_bg_opa(audioPlayer.styles.sliderIndicator, 0)
    lv.style.set_bg_opa(audioPlayer.styles.sliderKnob, 0)
    lv.style.set_border_width(audioPlayer.styles.sliderKnob, 0)
    lv.obj.invalidate(audioPlayer.images.background)
    lv.obj.invalidate(audioPlayer.images.foreground)
    lv.obj.invalidate(audioPlayer.titleLabel)
    lv.obj.invalidate(audioPlayer.slider)
    lv.style.set_text_font(audioPlayer.styles.titleLabel, lv.font.nunito_bold_20)
    lv.obj.set_y(audioPlayer.titleLabel, 175)
    lv.img.set_zoom(audioPlayer.images.cover, 256)
    audioPlayer.isLargeView = true
end

--- update view to show Mini player
---@return nil
function audioPlayer.showMiniPlayer()
    lv.style.set_img_opa(audioPlayer.styles.background, 0)
    lv.style.set_img_opa(audioPlayer.styles.foreground, 0)
    lv.style.set_text_opa(audioPlayer.styles.titleLabel, 255)
    lv.style.set_bg_opa(audioPlayer.styles.sliderMain, 255)
    lv.style.set_bg_opa(audioPlayer.styles.sliderIndicator, 255)
    lv.style.set_bg_opa(audioPlayer.styles.sliderKnob, 255)
    lv.style.set_border_width(audioPlayer.styles.sliderKnob, 1)
    lv.obj.invalidate(audioPlayer.images.background)
    lv.obj.invalidate(audioPlayer.images.foreground)
    lv.obj.invalidate(audioPlayer.titleLabel)
    lv.style.set_text_font(audioPlayer.styles.titleLabel, lv.font.nunito_bold_16)
    lv.obj.set_y(audioPlayer.titleLabel, 148)
    lv.img.set_zoom(audioPlayer.images.cover, 207)
    audioPlayer.isLargeView = false
end

--- init function for audio-Player
---@return nil
function audioPlayer.create(args)
    lv.group.set_editing(document, true)
    lv.obj.clean(window)
    audioPlayer.initStyles()
    audioPlayer.exitCallback = args.callback
    audioPlayer.okCallback = args.okCallback
    audioPlayer.okRequested = false
    audioPlayer.okTick = 0
    audioPlayer.exited = false
    audioPlayer.ended = false
    audioPlayer.ignoreSeekSave = args.ignoreSeek
    audioPlayer.parentContainer = lv.obj.new(window)
    lv.obj.remove_style_all(audioPlayer.parentContainer)
    lv.obj.set_size(audioPlayer.parentContainer, lv.obj.get_width(window), lv.obj.get_height(window))
    lv.obj.add_style(audioPlayer.parentContainer, audioPlayer.styles.parentContainer, lv.STATE_DEFAULT)

    local image_path = Global.apDefaultCover
    local image_background_path = Global.apDefaultBGCover
    local image_foreground_path = Global.apDefaultFGCover

    if (args.image_path ~= nil) then
        image_path = args.image_path
    end
    if (args.image_background_path ~= nil) then
        image_background_path = args.image_background_path
    end
    if (args.image_foreground_path ~= nil) then
        image_foreground_path = args.image_foreground_path
    end

    audioPlayer.images.background = lv.img.new(audioPlayer.parentContainer)
    lv.obj.remove_style_all(audioPlayer.images.background)
    audioPlayer.imgBackground = Global.load_image(image_background_path)
    lv.img.set_src(audioPlayer.images.background, audioPlayer.imgBackground)
    lv.obj.align(audioPlayer.images.background, lv.ALIGN_TOP_MID, 0, 0)
    lv.obj.set_size(audioPlayer.images.background, Global.visual_width, Global.visual_height)
    lv.obj.add_style(audioPlayer.images.background, audioPlayer.styles.background, lv.STATE_DEFAULT)

    audioPlayer.images.cover = lv.img.new(audioPlayer.parentContainer)
    lv.obj.remove_style_all(audioPlayer.images.cover)
    audioPlayer.imgPlayer = Global.load_image(image_path)
    lv.img.set_src(audioPlayer.images.cover, audioPlayer.imgPlayer)
    lv.obj.align(audioPlayer.images.cover, lv.ALIGN_TOP_MID, 0, 8)

    audioPlayer.images.foreground = lv.img.new(audioPlayer.parentContainer)
    lv.obj.remove_style_all(audioPlayer.images.foreground)
    audioPlayer.imgForeground = Global.load_image(image_foreground_path)
    lv.img.set_src(audioPlayer.images.foreground, audioPlayer.imgForeground)
    lv.obj.align(audioPlayer.images.foreground, lv.ALIGN_TOP_MID, 0, 0)
    lv.obj.set_size(audioPlayer.images.foreground, Global.visual_width, Global.visual_height)
    lv.obj.add_style(audioPlayer.images.foreground, audioPlayer.styles.foreground, lv.STATE_DEFAULT)

    audioPlayer.titleLabel = lv.label.new(audioPlayer.parentContainer)
    lv.obj.remove_style_all(audioPlayer.titleLabel)
    lv.obj.set_width(audioPlayer.titleLabel, 296)
    lv.label.set_long_mode(audioPlayer.titleLabel, lv.LABEL_LONG_SCROLL_CIRCULAR)
    lv.label.set_text(audioPlayer.titleLabel, args.song_name)
    lv.obj.clear_flag(audioPlayer.titleLabel, lv.OBJ_FLAG_SCROLLABLE)
    lv.obj.clear_flag(audioPlayer.titleLabel, lv.OBJ_FLAG_CLICK_FOCUSABLE)
    lv.obj.set_style_text_align(audioPlayer.titleLabel, lv.TEXT_ALIGN_CENTER, 0)
    lv.obj.align(audioPlayer.titleLabel, lv.ALIGN_TOP_MID, 0, 148)
    lv.obj.add_style(audioPlayer.titleLabel, audioPlayer.styles.titleLabel, lv.STATE_DEFAULT)

    audioPlayer.slider = lv.slider.new(audioPlayer.parentContainer)
    lv.obj.remove_style_all(audioPlayer.slider)
    lv.obj.set_size(audioPlayer.slider, 214, 4)
    lv.obj.align(audioPlayer.slider, lv.ALIGN_TOP_MID, 0, 181)
    lv.obj.add_style(audioPlayer.slider, audioPlayer.styles.sliderMain, lv.PART_MAIN)
    lv.obj.add_style(audioPlayer.slider, audioPlayer.styles.sliderIndicator, lv.PART_INDICATOR)
    lv.obj.add_style(audioPlayer.slider, audioPlayer.styles.sliderKnob, lv.PART_KNOB)
    lv.group.remove_obj(audioPlayer.slider)

    audioPlayer.events[audioPlayer.parentContainer] = {}
    audioPlayer.events[audioPlayer.parentContainer].key = lv.obj.add_event_cb(audioPlayer.parentContainer,
        audioPlayer.keyPressed, lv.EVENT_KEY)

    lv.group.add_obj(document, audioPlayer.parentContainer)
    if (args.audio_path ~= nil) then
        audioPlayer.inactivity_timer = lv.timer.new(audioPlayer.inactivityCb, 1000, nil)
        Global.requestAudioPlay({ path = args.audio_path, AFCb = audioPlayer.audioFeedback, priority = true })
        audioPlayer.showLargePlayer()
    else
        -- [telmi2flam] scene image sans audio (non-autoplay) : image visible
        -- (opa 0 par defaut) et attente de OK, ou de Home seul si ctrl.ok=false
        -- (sans okCallback : l'image doit quand meme s'afficher, sinon ecran noir)
        audioPlayer.showLargePlayer()
    end
    if (audioPlayer.okCallback ~= nil) then
        audioPlayer.okTimer = lv.timer.new(audioPlayer.processOk, 100, nil)
    end
end

return audioPlayer
