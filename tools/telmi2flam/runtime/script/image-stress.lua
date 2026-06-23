--- This module il used to render a list with sound over, sound intro and image
---@class imageStress
local imageStress = {}
imageStress.styles = {}
-- load tick rate in ms
imageStress.tickRate = 50
imageStress.updateTimer = nil
imageStress.imgIt = 1
imageStress.image = nil
imageStress.renderedImg = nil

function imageStress.initSyles()
    imageStress.styles.parentStyle = lv.style.new()
    imageStress.styles.images = lv.style.new()

    imageStress.styles.annoucement = lv.style.new()
    lv.style.set_text_color(imageStress.styles.annoucement, lv.color.hex(0xefedea))
    lv.style.set_text_font(imageStress.styles.annoucement, lv.font.nunito_extrabold_20)
end

function imageStress.clean()
    if (imageStress.updateTimer ~= nil) then
        lv.timer.del(imageStress.updateTimer)
    end
    imageStress.styles = {}
    imageStress.renderedImg = nil
end

function imageStress.updateImage()
    if (imageStress.imgIt > 12) then imageStress.imgIt = 1 end
    imageStress.renderedImg = Global.load_image("Nyan_Cat" .. imageStress.imgIt .. ".lif")
    lv.img.set_src(imageStress.image, imageStress.renderedImg)
    imageStress.imgIt = imageStress.imgIt + 1
end

function imageStress.audioFeedback(state, second)
    if (state == "stop") then
        Global.requestAudioPlay({ path = "audio-stress.mp3", AFCb = imageStress.audioFeedback })
    end
end

function imageStress.create(args)
    lv.group.set_editing(document, true)
    lv.group.set_wrap(document, false)
    lv.obj.clean(window) -- Clean main window
    --lv.timer.del(Global.screenShutdownTimer )
    imageStress.initSyles()

    local parentContainer = lv.obj.new(window)
    lv.obj.remove_style_all(parentContainer)
    lv.obj.set_size(parentContainer, lv.obj.get_width(window), lv.obj.get_height(window))
    lv.obj.add_style(parentContainer, imageStress.styles.parentStyle, lv.STATE_DEFAULT)

    imageStress.image = lv.img.new(parentContainer)
    lv.obj.remove_style_all(imageStress.image)
    lv.obj.add_flag(imageStress.image, lv.OBJ_FLAG_FLOATING)
    lv.obj.align(imageStress.image, lv.ALIGN_TOP_MID, 0, 0)
    lv.obj.set_size(imageStress.image, Global.visual_width, Global.visual_height)

    local titleLabel = lv.label.new(parentContainer)
    lv.obj.remove_style_all(titleLabel)
    lv.obj.set_width(titleLabel, Global.visual_width)
    lv.label.set_text(titleLabel, "Kaboom ?")
    lv.obj.align(titleLabel, lv.ALIGN_CENTER, 105, -70)
    lv.obj.add_style(titleLabel, imageStress.styles.annoucement, lv.STATE_DEFAULT)

    imageStress.updateTimer = lv.timer.new(imageStress.updateImage, imageStress.tickRate, nil)
    Global.requestAudioPlay({ path = "audio-stress.mp3", AFCb = imageStress.audioFeedback })
end

return imageStress
