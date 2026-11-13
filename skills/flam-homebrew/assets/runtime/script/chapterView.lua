--- This module il used to render chapter selection and select a chapter to visit
---@class chapterView
local chapterView = {}
--- Stores all lvgl style rules
chapterView.styles = {}
--- Keep pointer on registered events for cleaning
chapterView.events = {}
--- Store callback used on back button pressed ( need to be stored fore recreate function)
---@see recreate()
chapterView.backCallback = nil
chapterView.forcePlay = false
--- Init Styles for lvgl components
---@return nil
function chapterView.initStyles()
    -- < Components styles >
    chapterView.styles = {}

    chapterView.styles.parent_style = lv.style.new()
    lv.style.set_bg_color(chapterView.styles.parent_style, lv.color.hex(0x000000))
    lv.style.set_bg_opa(chapterView.styles.parent_style, lv.OPA_COVER)
    lv.style.set_pad_row(chapterView.styles.parent_style, 12)
    lv.style.set_pad_top(chapterView.styles.parent_style, 12)
    lv.style.set_pad_left(chapterView.styles.parent_style, 12)

    chapterView.styles.title_style = lv.style.new()
    lv.style.set_text_color(chapterView.styles.title_style, lv.color.hex(0xf2f4f5))
    lv.style.set_text_font(chapterView.styles.title_style, lv.font.nunito_extrabold_20)


    chapterView.styles.item_cont_style = lv.style.new()
    lv.style.set_text_color(chapterView.styles.item_cont_style, lv.color.hex(0xf2f4f5))
    lv.style.set_text_font(chapterView.styles.item_cont_style, lv.font.nunito_extrabold_16)

    chapterView.styles.knob_style = lv.style.new()
    lv.style.set_bg_opa(chapterView.styles.knob_style, lv.OPA_COVER)
    lv.style.set_bg_color(chapterView.styles.knob_style, lv.color.hex(0xffffff))
    lv.style.set_border_color(chapterView.styles.knob_style, lv.color.hex(0x000000))
    lv.style.set_border_width(chapterView.styles.knob_style, 1)
    lv.style.set_radius(chapterView.styles.knob_style, lv.RADIUS_CIRCLE)
    lv.style.set_pad_all(chapterView.styles.knob_style, 4)

    chapterView.styles.slider_style = lv.style.new()
    lv.style.set_bg_color(chapterView.styles.slider_style, lv.color.hex(0xF2F4F5))
    lv.style.set_bg_opa(chapterView.styles.slider_style, lv.OPA_COVER)
    lv.style.set_radius(chapterView.styles.slider_style, 0)
    lv.style.set_border_width(chapterView.styles.slider_style, 0)

    -- < end of  Components styles>
end

--- Compute LVGL events for each entry (EVENT_FOCUSED,EVENT_DEFOCUSED,EVENT_PRESSED)
--- @param ev any # lvgl event
---@return nil
function chapterView.entryCallbackEvent(ev)
    local entryContainer = lv.event.get_target(ev)
    local label = lv.obj.get_child(entryContainer, 1) -- childs are store in instanciation order
    local indicator_container = lv.obj.get_child(entryContainer, 0)
    local knob = lv.obj.get_child(indicator_container, 1)



    if lv.event.get_code(ev) == lv.EVENT_FOCUSED then
        lv.obj.set_style_text_color(label, lv.color.hex(0xFFFFFF), lv.STATE_DEFAULT)
        lv.obj.set_style_bg_color(knob, lv.color.hex(0xfbbd2a), lv.STATE_DEFAULT)
        if (chapterView.events[entryContainer].audio ~= nil) then
            Global.requestAudioPlay({ path = chapterView.events[entryContainer].audio, priority = chapterView.forcePlay })
        end
    elseif lv.event.get_code(ev) == lv.EVENT_DEFOCUSED then
        lv.obj.set_style_text_color(label, lv.color.hex(0xFFFFFF), lv.STATE_DEFAULT)
        lv.obj.set_style_bg_color(knob, lv.color.hex(0xffffff), lv.STATE_DEFAULT)
    elseif lv.event.get_code(ev) == lv.EVENT_PRESSED then
        lv.obj.set_style_text_color(label, lv.color.hex(0xFBBD2F), lv.STATE_DEFAULT)
    end
end

--- Clean ChapterView Elements
---@return nil
function chapterView.clean()
    for _, entry in pairs(chapterView.events) do
        lv.obj.remove_event_cb(entry.container, entry.focused)
        lv.obj.remove_event_cb(entry.container, entry.defocused)
        lv.obj.remove_event_cb(entry.container, entry.pressed)
        lv.obj.remove_event_cb(entry.container, entry.clicked)
    end

    lv.group.remove_all_objs(document)
    lv.obj.clean(window)

    chapterView.styles = {}
    chapterView.events = {}
end

---@alias FOL_enum
---| "first"
---| "last"
---| "both"
--- Add entry to chapter list
---@param entry {cb : string,img : string, inventory : table,label : string, audio:string, branch?:string} # entry parameters
---@param parentContainer  any # parent LVGL container
---@param FOL? FOL_enum  # entry position ( used for specific rendering)
---@return nil
function chapterView.addEntry(entry, parentContainer, FOL)
    local base_height = 68
    local entryContainer = require("h-container").create(parentContainer, 7, true)
    chapterView.events[entryContainer] = {}
    chapterView.events[entryContainer].container = entryContainer
    lv.obj.set_flex_align(entryContainer, lv.FLEX_ALIGN_START, lv.FLEX_ALIGN_CENTER, lv.FLEX_ALIGN_CENTER)
    lv.obj.set_width(entryContainer, 292)
    lv.obj.set_height(entryContainer, 68)
    lv.obj.add_style(entryContainer, chapterView.styles.item_cont_style, lv.STATE_DEFAULT)

    local indicator_container = lv.obj.new(entryContainer)
    lv.obj.remove_style_all(indicator_container)
    lv.obj.set_width(indicator_container, 12)
    lv.obj.set_height(indicator_container, 68)

    local entrySlider = lv.obj.new(indicator_container)
    lv.obj.remove_style_all(entrySlider)
    lv.obj.set_width(entrySlider, 4)
    lv.obj.set_align(entrySlider, lv.ALIGN_CENTER)
    lv.obj.add_style(entrySlider, chapterView.styles.slider_style, lv.STATE_DEFAULT)
    lv.obj.set_height(entrySlider, base_height)

    local entryKnob = lv.obj.new(indicator_container)
    lv.obj.remove_style_all(entryKnob)
    lv.obj.set_size(entryKnob, 12, 12)
    lv.obj.set_align(entryKnob, lv.ALIGN_CENTER)
    lv.obj.add_style(entryKnob, chapterView.styles.knob_style, lv.STATE_DEFAULT)

    if (FOL ~= nil) then
        if (FOL == "first") then
            lv.obj.set_height(entrySlider, base_height - 2)
            local topCorner = lv.obj.new(indicator_container)
            lv.obj.remove_style_all(topCorner)
            lv.obj.set_width(topCorner, 4)
            lv.obj.set_height(topCorner, 4)
            lv.obj.set_align(topCorner, lv.ALIGN_TOP_MID)
            lv.obj.add_style(topCorner, chapterView.styles.slider_style, lv.STATE_DEFAULT)
            lv.obj.set_style_radius(topCorner, 2, lv.STATE_DEFAULT)

            lv.obj.set_align(entrySlider, lv.ALIGN_BOTTOM_MID)
        elseif (FOL == "last") then
            lv.obj.set_height(entrySlider, base_height - 2)
            local BottomCorner = lv.obj.new(indicator_container)
            lv.obj.remove_style_all(BottomCorner)
            lv.obj.set_width(BottomCorner, 4)
            lv.obj.set_height(BottomCorner, 4)
            lv.obj.set_align(BottomCorner, lv.ALIGN_BOTTOM_MID)
            lv.obj.add_style(BottomCorner, chapterView.styles.slider_style, lv.STATE_DEFAULT)
            lv.obj.set_style_radius(BottomCorner, 2, lv.STATE_DEFAULT)

            lv.obj.set_align(entrySlider, lv.ALIGN_TOP_MID)
        elseif (FOL == "both") then
            lv.obj.set_height(entrySlider, base_height - 2)

            local topCorner = lv.obj.new(indicator_container)
            lv.obj.remove_style_all(topCorner)
            lv.obj.set_width(topCorner, 4)
            lv.obj.set_height(topCorner, 4)
            lv.obj.set_align(topCorner, lv.ALIGN_TOP_MID)
            lv.obj.add_style(topCorner, chapterView.styles.slider_style, lv.STATE_DEFAULT)
            lv.obj.set_style_radius(topCorner, 2, lv.STATE_DEFAULT)

            local BottomCorner = lv.obj.new(indicator_container)
            lv.obj.remove_style_all(BottomCorner)
            lv.obj.set_width(BottomCorner, 4)
            lv.obj.set_height(BottomCorner, 4)
            lv.obj.set_align(BottomCorner, lv.ALIGN_BOTTOM_MID)
            lv.obj.add_style(BottomCorner, chapterView.styles.slider_style, lv.STATE_DEFAULT)
            lv.obj.set_style_radius(BottomCorner, 2, lv.STATE_DEFAULT)


            lv.obj.set_align(entrySlider, lv.ALIGN_CENTER)
        end
    end

    -- draw chapter label
    local label = lv.label.new(entryContainer)
    lv.label.set_long_mode(label, lv.LABEL_LONG_WRAP)
    lv.label.set_text(label, entry.label)
    lv.obj.set_width(label, 196)

    -- draw chpater image if any
    if (entry.img ~= nil) then
        local img = lv.img.new(entryContainer)
        lv.obj.remove_style_all(img)
        chapterView.events[entryContainer].imgData = Global.load_image(entry.img)
        lv.img.set_src(img, chapterView.events[entryContainer].imgData)
        lv.obj.set_style_pad_left(img, 8, lv.STATE_DEFAULT)
    end
    if (entry.audio ~= nil) then
        chapterView.events[entryContainer].audio = entry.audio
    end
    -- bind callback if any
    if (entry.cb ~= nil) then
        local entryCallback = function()
            if (entry.inventory ~= nil) then
                -- state.inventory = nil
                -- state.inventory = {}
                for k, v in pairs(entry.inventory) do
                    for l, j in pairs(v.saved) do
                        state.inventory[k].saved[l] = j
                    end
                    --state.inventory[k] = v
                end
            end
            state.visited_funs = {}
            state.registeredChoices = {}
            if(Global.isMultiBranches) then
                Global.loadBranch(entry.branch)
                Global.current_branch[entry.cb]()
            else
                _G[entry.cb]() 
            end
                 
        end

        chapterView.events[entryContainer].clicked = lv.obj.add_event_cb(entryContainer, function()
            Global.setBackBehavior(Global.progression.displayChapters)
            Global.load_module("dialog", "1_0_0").create({
                img = entry.img,
                title = entry.label,
                message = "Reprendre l'histoire à partir de ce chapitre ?",
                primary_button_text = "Valider",
                primary_button_cb = entryCallback,
                secondary_button_text = "Annuler",
                secondary_button_cb = Global.progression.displayChapters
            })
        end, lv.EVENT_CLICKED)
    end
    -- event biding
    chapterView.events[entryContainer].focused = lv.obj.add_event_cb(entryContainer, chapterView.entryCallbackEvent,
        lv.EVENT_FOCUSED)
    chapterView.events[entryContainer].defocused = lv.obj.add_event_cb(entryContainer, chapterView.entryCallbackEvent,
        lv.EVENT_DEFOCUSED)
    chapterView.events[entryContainer].pressed = lv.obj.add_event_cb(entryContainer, chapterView.entryCallbackEvent,
        lv.EVENT_PRESSED)
end

--- Create Chapter list from stored back parameters
---@return nil
function chapterView.recreate()
    chapterView.create(chapterView.backCallback)
end

--- Create Chapter list
---@param go_back_cb function
---@return nil
function chapterView.create(go_back_cb,titleLabel,forcePlay)
    chapterView.backCallback = go_back_cb
    Global.setBackBehavior(go_back_cb)

    lv.group.remove_all_objs(document)
    lv.obj.clean(window)

    lv.group.set_editing(document, false)
    lv.group.set_wrap(document, false)

    chapterView.initStyles()

    -- < Component Definition >
    local parent_container = lv.obj.new(window)
    lv.obj.remove_style_all(parent_container)
    lv.obj.set_size(parent_container, lv.obj.get_width(window), lv.obj.get_height(window))
    lv.obj.set_flex_flow(parent_container, lv.FLEX_FLOW_COLUMN)
    lv.obj.add_style(parent_container, chapterView.styles.parent_style, lv.STATE_DEFAULT)

    -- local title_button = lv.btn.new(parent_container)
    -- lv.obj.remove_style_all(title_button)

    local title_label = lv.label.new(parent_container)
    lv.obj.remove_style_all(title_label)
    if(titleLabel ~= nil) then
        lv.label.set_text(title_label, titleLabel)
    else
        lv.label.set_text(title_label, "Chapitres écoutés")
    end
    
    if(forcePlay ~= nil) then
        chapterView.forcePlay = forcePlay
    end

    lv.obj.add_style(title_label, chapterView.styles.title_style, lv.STATE_DEFAULT)

    local answers_container = lv.obj.new(parent_container)
    lv.obj.remove_style_all(answers_container)
    lv.obj.set_width(answers_container, 193)
    lv.obj.set_size(answers_container, 292, lv.obj.get_height(window))
    lv.obj.set_flex_flow(answers_container, lv.FLEX_FLOW_COLUMN)
    lv.obj.set_align(answers_container, lv.ALIGN_TOP_MID)

    -- < end of Component Definition >

    -- insert entries
    local orderedChapters = {}
    local loadedProgress = progression.load("chaps")
    if(loadedProgress ~= nil) then
        if(#loadedProgress ~= 0 and loadedProgress[1].order ~= nil) then
            for i = 1, (Global.progression.totalChapters) do
                for key, entry in pairs(loadedProgress) do
                    if (entry.order == i) then
                        table.insert(orderedChapters, entry)
                        table.remove(loadedProgress, key)
                    end
                end
            end
        else
            orderedChapters = loadedProgress
        end
    end
    local chapterLenght = Global.table_length(loadedProgress )
    loadedProgress = nil

    for i, entry in pairs(orderedChapters) do
        if (i == 1) then
            if (chapterLenght == 1) then
                chapterView.addEntry(entry, answers_container, "both")
            else
                chapterView.addEntry(entry, answers_container, "first")
            end
        else
            if (i == chapterLenght) then
                chapterView.addEntry(entry, answers_container, "last")
            else
                chapterView.addEntry(entry, answers_container)
            end
        end
    end
    -- focus on first item
    if (lv.obj.get_child_cnt(answers_container) > 0) then
        local first_answer = lv.obj.get_child(answers_container, 0)
        lv.group.focus_obj(first_answer)
    end
end

return chapterView
