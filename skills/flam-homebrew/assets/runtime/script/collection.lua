local collection = {}

collection.args = nil
collection.styles = {}
collection.items = {}

function collection.clean()
    for _, item in ipairs(collection.items) do
        if (item.focused_cb ~= nil) then
            lv.obj.remove_event_cb(item.container, item.focused_cb)
        end

        if (item.defocused_cb ~= nil) then
            lv.obj.remove_event_cb(item.container, item.defocused_cb)
        end

        if (item.clicked_cb ~= nil) then
            lv.obj.remove_event_cb(item.container, item.clicked_cb)
        end
    end

    Global.requestAudioStop()

    Global.v_scroll.clean()
    lv.group.remove_all_objs(document)
    lv.obj.clean(window)

    collection.styles = {}
    collection.items = {}
    collection.args = nil
end

function collection.focusFirstItem()
    for i, v in ipairs(collection.items) do
        if (i == 1) then
            Global.v_scroll.focus_obj(v.container)
        end
    end
end

-- Init styles for widgets
function collection.initStyles()
    collection.styles.parent_style = lv.style.new()
    lv.style.set_bg_color(collection.styles.parent_style, lv.color.hex(0x000000))
    lv.style.set_bg_opa(collection.styles.parent_style, lv.OPA_COVER)
    lv.style.set_pad_top(collection.styles.parent_style, 12)
    lv.style.set_pad_right(collection.styles.parent_style, 12)
    lv.style.set_pad_bottom(collection.styles.parent_style, 0)
    lv.style.set_pad_left(collection.styles.parent_style, 12)

    collection.styles.item_cont_style = lv.style.new()
    lv.style.set_pad_top(collection.styles.item_cont_style, 0)
    lv.style.set_pad_right(collection.styles.item_cont_style, 10)
    lv.style.set_pad_bottom(collection.styles.item_cont_style, 0)
    lv.style.set_pad_left(collection.styles.item_cont_style, 10)

    collection.styles.item_title_style = lv.style.new()
    lv.style.set_bg_color(collection.styles.item_title_style, lv.color.hex(0xffffff))
    lv.style.set_bg_opa(collection.styles.item_title_style, lv.OPA_TRANSP)
    lv.style.set_radius(collection.styles.item_title_style, 2)
    lv.style.set_pad_top(collection.styles.item_title_style, 2)
    lv.style.set_pad_right(collection.styles.item_title_style, 4)
    lv.style.set_pad_bottom(collection.styles.item_title_style, 2)
    lv.style.set_pad_left(collection.styles.item_title_style, 4)
    lv.style.set_text_color(collection.styles.item_title_style, lv.color.hex(0xffffff))
    lv.style.set_text_font(collection.styles.item_title_style, lv.font.nunito_bold_12)
    lv.style.set_text_line_space(collection.styles.item_title_style, 4)
    lv.style.set_text_align(collection.styles.item_title_style, lv.TEXT_ALIGN_CENTER)

    collection.styles.item_title_locked_style = lv.style.new()
    lv.style.set_text_color(collection.styles.item_title_locked_style, lv.color.hex(0xB8C4C8))

    collection.styles.items_style = lv.style.new()

    collection.styles.title_style = lv.style.new()
    lv.style.set_text_color(collection.styles.title_style, lv.color.hex(0xf2f4f5))
    lv.style.set_text_font(collection.styles.title_style, lv.font.nunito_extrabold_16)
    lv.style.set_text_line_space(collection.styles.title_style, 4)
    lv.style.set_text_align(collection.styles.title_style, lv.TEXT_ALIGN_CENTER)

    collection.styles.empty_style = lv.style.new()
    lv.style.set_pad_top(collection.styles.empty_style, 50)
    lv.style.set_text_color(collection.styles.empty_style, lv.color.hex(0xf1f3f4))
    lv.style.set_text_font(collection.styles.empty_style, lv.font.nunito_extrabold_14)
    lv.style.set_text_line_space(collection.styles.empty_style, 4)
    lv.style.set_text_align(collection.styles.empty_style, lv.TEXT_ALIGN_CENTER)
end

-- @param args:table
--        {
--          item:table item from inventory,
--          callback_previous:function function to call when back button is pressed
--        }
function collection.showDetail(args)
    Global.load_module("detail", "").create(args)
end

function collection.createItem(items_container, item_data, hideUse)
    local item = {}

    -- Create item container
    local item_cont = Global.v_container.create(items_container, 0, true)
    lv.obj.set_flex_align(item_cont, lv.FLEX_ALIGN_CENTER, lv.FLEX_ALIGN_CENTER, lv.FLEX_ALIGN_START)
    lv.obj.add_style(item_cont, collection.styles.item_cont_style, lv.STATE_DEFAULT)
    Global.v_scroll.add_focusable_obj(item_cont)

    item.container = item_cont
    item.img = lv.img.new(item_cont)
    item.audio = item_data.audio
    item.detailImg = item_data.detailImg

    -- Add item title
    local item_title_label = lv.label.new(item_cont)
    lv.obj.remove_style_all(item_title_label)
    lv.obj.set_width(item_title_label, 122)
    lv.label.set_text(item_title_label, item_data.title)
    lv.label.set_long_mode(item_title_label, lv.LABEL_LONG_WRAP)
    lv.obj.add_style(item_title_label, collection.styles.item_title_style, lv.STATE_DEFAULT)
    -- If item is lock, load locked image, else load default image
    if (item_data.saved.locked ~= nil and item_data.saved.locked == true) then
        item.img_data = Global.load_image(item_data.img_locked)
        if item_data.audio_locked ~= nil then
            item.audio = item_data.audio_locked
        end
        --     local img_lock = lv.img.new(item.img)
        --     print("locked image loaded")
        --     item.lock_img_data = Global.load_image("script/lock.lif")
        --     lv.img.set_src(img_lock, item.lock_img_data)
        --     lv.obj.align(img_lock, lv.ALIGN_BOTTOM_RIGHT, 0, 0)
        --     lv.obj.add_style(item_title_label, collection.styles.item_title_locked_style, lv.STATE_DEFAULT)
    else
        item.img_data = Global.load_image(item_data.img)
    end

    lv.img.set_src(item.img, item.img_data)

    -- Init focus callback
    if (item_data.img_hovered == nil) then
        item_data.img_hovered = item_data.img
    end

    if (item_data.img_locked_hovered == nil) then
        item_data.img_locked_hovered = item_data.img_locked
    end

    item.focused_cb = lv.obj.add_event_cb(item_cont, function()
        if (item_data.saved.locked ~= nil and item_data.saved.locked == true) then
            item.img_data = Global.load_image(item_data.img_locked_hovered)
        else
            item.img_data = Global.load_image(item_data.img_hovered)
        end

        lv.img.set_src(item.img, item.img_data)
        lv.obj.set_style_text_color(item_title_label, lv.color.hex(0x000000), lv.STATE_DEFAULT)
        lv.obj.set_style_bg_opa(item_title_label, lv.OPA_COVER, lv.STATE_DEFAULT)

        -- If audio is set, play it on focus
        Global.requestAudioPlay({ path = item.audio, priority = true })
        collectgarbage("collect")
    end, lv.EVENT_FOCUSED)

    -- Init defocus callback
    item.defocused_cb = lv.obj.add_event_cb(item_cont, function()
        if (item_data.saved.locked ~= nil and item_data.saved.locked == true) then
            item.img_data = Global.load_image(item_data.img_locked)
            lv.obj.set_style_text_color(item_title_label, lv.color.hex(0xB8C4C8), lv.STATE_DEFAULT)
        else
            item.img_data = Global.load_image(item_data.img)
            lv.obj.set_style_text_color(item_title_label, lv.color.hex(0xffffff), lv.STATE_DEFAULT)
        end

        lv.img.set_src(item.img, item.img_data)
        lv.obj.set_style_bg_opa(item_title_label, lv.OPA_TRANSP, lv.STATE_DEFAULT)
    end, lv.EVENT_DEFOCUSED)

    -- Init click callback
    if (item_data.noDetail == nil or collection.args.override ~= nil) then
        item.clicked_cb = lv.obj.add_event_cb(item_cont, function()
            if (item_data.saved.locked ~= nil and item_data.saved.locked == true) then return end
            local found = false
            local cbArgs = {
                name = "collection",
                version = "",
                args = collection.args
            }

            if (collection.args.override ~= nil) then
                for _, o in pairs(collection.args.override) do
                    if (found == false and o.item ~= nil and o.item == item_data) then
                        if (o.overrideClick ~= nil) then
                            found = true
                            Global.setBackModule(cbArgs)
                            o.overrideClick(o.item)
                        elseif (o.overrideAudio ~= nil or o.overrideCallback ~= nil) then
                            found = true
                            collection.showDetail({
                                item = item_data,
                                backModule = cbArgs,
                                hideUse = hideUse,
                                overrideAudio =
                                    o.overrideAudio,
                                overrideCallback = o.overrideCallback
                            })
                            break
                        end
                    end
                end -- end for override
            end     -- end if override exists

            if (found == false) then
                collection.showDetail({ item = item_data, backModule = cbArgs, hideUse = hideUse })
            end
        end, lv.EVENT_CLICKED)
    end

    table.insert(collection.items, item)
end

-- @param args:table
--  {
--      title:string page title,
--      audio:string (optional) path to audio
--      items:table collection items,
--      [
--          title:string item title
--          audio:string (optional) path to audio for this item
--          img:string item img path
--          img_hover:string item img path when element is focused
--          hidden:bool item is hidden (example: item not discover yet)
--          locked:bool (optional) item is locked (example: achievements)
--          img_locked:string item img path when element is locked
--          img_locked_hovered:string item img path when element is locked and focused
--      ]
--      back_cb:function function to call when back button is pressed
--  }
function collection.create(args)
    collection.styles = {}
    collection.items = {}
    collection.args = args

    local hideUse = true;
    if (args.hideUse ~= nil) then hideUse = args.hideUse end

    lv.group.remove_all_objs(document)
    lv.obj.clean(window)

    lv.group.set_editing(document, false)
    lv.group.set_wrap(document, false)

    collection.initStyles()

    -- Init parent container
    local parent_container = Global.v_container.create(window, 16, false)
    lv.obj.set_size(parent_container, lv.obj.get_width(window), lv.obj.get_height(window))
    lv.obj.set_flex_align(parent_container, lv.FLEX_ALIGN_START, lv.FLEX_ALIGN_CENTER, lv.FLEX_ALIGN_START)
    lv.obj.add_style(parent_container, collection.styles.parent_style, lv.STATE_DEFAULT)

    -- Init items container
    local items_container = lv.obj.new(parent_container)
    lv.obj.remove_style_all(items_container)
    lv.obj.set_width(items_container, 296)
    lv.obj.set_height(items_container, lv.pct(100))
    lv.obj.set_flex_flow(items_container, lv.FLEX_FLOW_ROW_WRAP)
    lv.obj.set_flex_align(items_container, lv.FLEX_ALIGN_START, lv.FLEX_ALIGN_START, lv.FLEX_ALIGN_START)
    lv.obj.set_style_pad_row(items_container, 12, lv.STATE_DEFAULT)
    lv.obj.add_style(items_container, collection.styles.items_style, lv.STATE_DEFAULT)

    -- Init title label
    local title_label = lv.label.new(items_container)
    lv.obj.remove_style_all(title_label)
    lv.obj.set_width(title_label, 296)
    lv.label.set_text(title_label, args.title)
    lv.obj.add_style(title_label, collection.styles.title_style, lv.STATE_DEFAULT)

    -- Init items grid
    if (args.items ~= nil) then
        for _, v in ipairs(args.items) do
            if ((v.saved.hidden == nil or (v.saved.hidden ~= nil and v.saved.hidden == false)) and (args.group == nil or (v.group ~= nil and args.group == v.group))) then
                collection.createItem(items_container, v, hideUse)
            end
        end
    end

    -- If no item, show text for empty collection
    if (#collection.items == 0) then
        local empty_label = lv.label.new(items_container)
        lv.obj.remove_style_all(empty_label)
        lv.obj.set_width(empty_label, 296)
        local text = "Empty"
        if (args.empty_text ~= nil) then text = args.empty_text end
        lv.label.set_text(empty_label, text)
        lv.obj.add_style(empty_label, collection.styles.empty_style, lv.STATE_DEFAULT)
        lv.obj.align(empty_label, lv.ALIGN_BOTTOM_MID, 0, 0)
    end

    Global.v_scroll.init(parent_container, items_container)

    -- Play audio if set
    if (args.audio ~= nil) then
        Global.requestAudioPlay({ path = args.audio })
    else
        collection.focusFirstItem()
    end

    -- Init back callback
    if (collection.args.back_cb ~= nil) then
        Global.setBackBehavior(collection.args.back_cb)
    else
        print("ERROR: No callback set for back button")
    end
end

function collection.recreate(args)
    Global.load_module("collection", "").create(args)
end

return collection
