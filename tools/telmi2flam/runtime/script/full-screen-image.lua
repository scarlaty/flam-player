---@class fullScreen
local fullScreenImage = {}
fullScreenImage.styles = {}
fullScreenImage.img = nil

function fullScreenImage.initStyles()
  fullScreenImage.styles.parent_style = lv.style.new()
  lv.style.set_bg_color(fullScreenImage.styles.parent_style, lv.color.hex(0x000000))
  lv.style.set_bg_opa(fullScreenImage.styles.parent_style, lv.OPA_COVER)

  fullScreenImage.styles.image_style = lv.style.new()
  lv.style.set_img_opa(fullScreenImage.styles.image_style, 255)
end

function fullScreenImage.clean()
    lv.group.remove_all_objs(document)
    lv.obj.clean(window)

    for i, style in pairs(fullScreenImage.styles) do
        lv.style.reset(style)
    end
    fullScreenImage.img = nil
    fullScreenImage.styles = {}
end

function fullScreenImage.display(args) 
    if(args.cb ~= nil) then
        fullScreenImage.setBackBehavior(args.cb)
    end
    lv.obj.clean(window)
    fullScreenImage.initStyles()

    local parent_container = lv.obj.new(window)
    lv.obj.remove_style_all(parent_container)
    lv.obj.set_size(parent_container, lv.obj.get_width(window), lv.obj.get_height(window))
    lv.obj.add_style(parent_container, fullScreenImage.styles.parent_style, lv.STATE_DEFAULT)

    if (args.image_path ~= nil) then
        local width, height
        fullScreenImage.img, width, height = Global.load_image(args.image_path)
        local image = lv.img.new(parent_container)
        lv.obj.remove_style_all(image)
        lv.img.set_src(image, fullScreenImage.img)
        lv.obj.set_size(image, width, height)
        --lv.img.set_zoom(image,207) -- 256 = 100%
        lv.obj.align(image, lv.ALIGN_CENTER, 0, 0)
        lv.obj.add_style(image, fullScreenImage.styles.image_style, lv.STATE_DEFAULT)

        lv.group.add_obj(document, image)
        lv.group.focus_obj(image) 
    end
end

function fullScreenImage.setBackBehavior(backCallback)
    Global.setBackBehavior(backCallback)
end

return fullScreenImage
