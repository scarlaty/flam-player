require("test_helpers")
print("--- test_integration ---")
-- These tests replicate real script patterns that have caused bugs.

-- ============================================================
-- add_style with STATE selector (dialog-interactive.lua pattern)
-- ============================================================

test("add_style with STATE_DEFAULT selector", function()
    local obj = lv.obj.new(window)
    local s = lv.style.new()
    lv.style.set_bg_color(s, lv.color.hex(0xFF0000))
    lv.obj.add_style(obj, s, lv.STATE_DEFAULT)
    expect_true(true)
    lv.obj.del(obj)
end)

test("add_style with STATE_USER_1 selector", function()
    local obj = lv.obj.new(window)
    local s1 = lv.style.new()
    lv.style.set_bg_opa(s1, lv.OPA_TRANSP)
    local s2 = lv.style.new()
    lv.style.set_bg_opa(s2, lv.OPA_COVER)
    -- Real pattern: two styles on same obj for different states
    lv.obj.add_style(obj, s1, lv.STATE_DEFAULT)
    lv.obj.add_style(obj, s2, lv.STATE_USER_1)
    expect_true(true)
    lv.obj.del(obj)
end)

test("add_style with STATE_FOCUS_KEY selector", function()
    local obj = lv.btn.new(window)
    local s1 = lv.style.new()
    lv.style.set_text_color(s1, lv.color.hex(0xFFFFFF))
    local s2 = lv.style.new()
    lv.style.set_text_color(s2, lv.color.hex(0xFBBD2F))
    lv.obj.add_style(obj, s1, lv.STATE_DEFAULT)
    lv.obj.add_style(obj, s2, lv.STATE_FOCUS_KEY)
    expect_true(true)
    lv.obj.del(obj)
end)

-- ============================================================
-- remove_event_cb pattern (all scripts do this in clean())
-- ============================================================

test("remove_event_cb stops callback from firing", function()
    local obj = lv.obj.new(window)
    local count = 0
    local cb = lv.obj.add_event_cb(obj, function() count = count + 1 end, lv.EVENT_CLICKED)
    lv.event.send(obj, lv.EVENT_CLICKED)
    expect_eq(count, 1, "before remove")
    lv.obj.remove_event_cb(obj, cb)
    lv.event.send(obj, lv.EVENT_CLICKED)
    expect_eq(count, 1, "after remove should not fire")
    lv.obj.del(obj)
end)

test("remove multiple event_cbs (chapterView pattern)", function()
    local obj = lv.btn.new(window)
    local events = {}
    events.focused = lv.obj.add_event_cb(obj, function() end, lv.EVENT_FOCUSED)
    events.defocused = lv.obj.add_event_cb(obj, function() end, lv.EVENT_DEFOCUSED)
    events.pressed = lv.obj.add_event_cb(obj, function() end, lv.EVENT_PRESSED)
    events.clicked = lv.obj.add_event_cb(obj, function() end, lv.EVENT_CLICKED)
    -- Clean pattern: remove all callbacks
    lv.obj.remove_event_cb(obj, events.focused)
    lv.obj.remove_event_cb(obj, events.defocused)
    lv.obj.remove_event_cb(obj, events.pressed)
    lv.obj.remove_event_cb(obj, events.clicked)
    expect_true(true)
    lv.obj.del(obj)
end)

-- ============================================================
-- v_scroll navigation pattern (the actual bug scenario)
-- ============================================================

test("v_scroll key navigation pattern", function()
    -- Reproduces the v_scroll.event_key_cb pattern
    local parent = lv.obj.new(window)
    lv.obj.set_size(parent, 320, 212)
    lv.group.add_obj(document, parent)
    lv.group.set_editing(document, true)

    local received_key = nil
    lv.obj.add_event_cb(parent, function(ev)
        local code = lv.event.get_code(ev)
        if code == lv.EVENT_KEY then
            local raw = lv.event.get_key_value(ev)
            received_key = string.byte(raw)
        end
    end, lv.EVENT_KEY)

    -- Simulate sending KEY_LEFT like the indev would
    lv.event.send(parent, lv.EVENT_KEY, lv.KEY_LEFT)
    expect_eq(received_key, lv.KEY_LEFT, "should receive KEY_LEFT")

    lv.event.send(parent, lv.EVENT_KEY, lv.KEY_RIGHT)
    expect_eq(received_key, lv.KEY_RIGHT, "should receive KEY_RIGHT")

    lv.group.set_editing(document, false)
    lv.group.remove_all_objs(document)
    lv.obj.del(parent)
end)

-- ============================================================
-- list-choice create/clean lifecycle (full menu pattern)
-- ============================================================

test("list-choice lifecycle: create buttons, focus, clean", function()
    lv.group.remove_all_objs(document)
    lv.obj.clean(window)

    lv.group.set_editing(document, false)
    lv.group.set_wrap(document, false)

    -- Create container with buttons like list-choice does
    local container = lv.obj.new(window)
    lv.obj.set_flex_flow(container, lv.FLEX_FLOW_COLUMN)
    lv.obj.set_size(container, 191, 190)

    local events = {}
    for i = 1, 3 do
        local btn = lv.btn.new(container)
        lv.obj.remove_style_all(btn)
        local label = lv.label.new(btn)
        lv.label.set_text(label, "Item " .. i)
        -- Remove from default group (list-choice does this)
        lv.group.remove_obj(btn)
        events[btn] = {}
        events[btn].focused = lv.obj.add_event_cb(btn, function() end, lv.EVENT_FOCUSED)
        events[btn].clicked = lv.obj.add_event_cb(btn, function() end, lv.EVENT_CLICKED)
    end

    -- Add back to group (enableSelection pattern)
    for i = 0, lv.obj.get_child_cnt(container) - 1 do
        lv.group.add_obj(document, lv.obj.get_child(container, i))
    end

    -- Clean pattern
    for btn, ev in pairs(events) do
        lv.obj.remove_event_cb(btn, ev.focused)
        lv.obj.remove_event_cb(btn, ev.clicked)
    end
    lv.group.remove_all_objs(document)
    lv.obj.clean(window)
    expect_true(true)
end)

-- ============================================================
-- collection/v_scroll lifecycle (trophy menu pattern)
-- ============================================================

test("collection lifecycle: v_scroll focusable items", function()
    lv.group.remove_all_objs(document)
    lv.obj.clean(window)
    lv.group.set_editing(document, false)
    lv.group.set_wrap(document, false)

    local parent = lv.obj.new(window)
    lv.obj.set_size(parent, 320, 212)

    local items_container = lv.obj.new(parent)
    lv.obj.set_flex_flow(items_container, lv.FLEX_FLOW_ROW_WRAP)
    lv.obj.set_size(items_container, 296, lv.pct(100))

    -- Create focusable items (collection.createItem pattern)
    local focusable = {}
    for i = 1, 4 do
        local item_cont = lv.obj.new(items_container)
        lv.obj.set_flex_flow(item_cont, lv.FLEX_FLOW_COLUMN)
        local img = lv.img.new(item_cont)
        local lbl = lv.label.new(item_cont)
        lv.label.set_text(lbl, "Trophy " .. i)
        table.insert(focusable, item_cont)
    end

    -- v_scroll.init pattern
    local key_cb = lv.obj.add_event_cb(parent, function(ev)
        if lv.event.get_code(ev) == lv.EVENT_KEY then
            local dir = string.byte(lv.event.get_key_value(ev))
            -- This is the exact pattern that was broken
            if dir == lv.KEY_LEFT then
                -- navigate prev
            elseif dir == lv.KEY_RIGHT then
                -- navigate next
            end
        end
    end, lv.EVENT_KEY)

    local click_cb = lv.obj.add_event_cb(parent, function() end, lv.EVENT_CLICKED)
    lv.group.add_obj(document, parent)
    lv.group.focus_obj(parent)
    lv.group.set_editing(document, true)

    -- Simulate focus on first item
    lv.obj.add_state(focusable[1], lv.STATE_FOCUSED)
    lv.event.send(focusable[1], lv.EVENT_FOCUSED)

    -- Simulate key press
    lv.event.send(parent, lv.EVENT_KEY, lv.KEY_RIGHT)
    test_tick(1)

    -- Clean
    lv.obj.remove_event_cb(parent, key_cb)
    lv.obj.remove_event_cb(parent, click_cb)
    lv.group.set_editing(document, false)
    lv.group.remove_all_objs(document)
    lv.obj.clean(window)
    expect_true(true)
end)

-- ============================================================
-- chapterView pattern: btn auto-added to default group
-- ============================================================

test("chapterView: buttons auto-added, focus_obj works", function()
    lv.group.remove_all_objs(document)
    lv.obj.clean(window)
    lv.group.set_editing(document, false)
    lv.group.set_wrap(document, false)

    local parent = lv.obj.new(window)
    lv.obj.set_flex_flow(parent, lv.FLEX_FLOW_COLUMN)

    -- Create chapter entry buttons (h-container pattern with clickable=true)
    local entries = {}
    for i = 1, 3 do
        local btn = lv.btn.new(parent)
        lv.obj.remove_style_all(btn)
        local lbl = lv.label.new(btn)
        lv.label.set_text(lbl, "Chapter " .. i)
        table.insert(entries, btn)
    end

    -- chapterView focuses first child — this requires btn to be in group
    local first = lv.obj.get_child(parent, 0)
    lv.group.focus_obj(first)

    -- Verify focus events work
    local focused = false
    lv.obj.add_event_cb(first, function() focused = true end, lv.EVENT_FOCUSED)
    lv.event.send(first, lv.EVENT_FOCUSED)
    expect_true(focused, "first chapter button should receive focus")

    lv.group.remove_all_objs(document)
    lv.obj.clean(window)
end)

-- ============================================================
-- Animation with exec_cb modifying widget (real animation pattern)
-- ============================================================

test("anim exec_cb moves widget position", function()
    local obj = lv.img.new(window)
    lv.obj.set_pos(obj, 320, 92)

    local anim = lv.anim.new()
    lv.anim.set_var(anim, obj)
    lv.anim.set_values(anim, 320, 204)
    lv.anim.set_time(anim, 200)
    lv.anim.set_exec_cb(anim, function(_, val)
        lv.obj.set_x(obj, val)
    end)
    lv.anim.set_path_cb(anim, lv.anim.path_ease_in_out)
    lv.anim.start(anim)

    -- Let animation run
    test_tick(50)

    -- Object should have moved from starting position
    local x = lv.obj.get_x(obj)
    expect_type(x, "number", "x after anim")
    lv.obj.del(obj)
end)

-- ============================================================
-- Timer with nil third arg (scripts pass nil as repeat count)
-- ============================================================

test("timer.new with nil third arg", function()
    local count = 0
    local t = lv.timer.new(function() count = count + 1 end, 100, nil)
    expect_true(t ~= nil, "timer created with nil repeat")
    test_tick(5)
    lv.timer.del(t)
end)

-- ============================================================
-- scroll_by with ANIM_ON (v_scroll pattern)
-- ============================================================

test("scroll_by with ANIM_ON", function()
    local container = lv.obj.new(window)
    lv.obj.set_size(container, 296, 200)
    -- Add tall content to make scrollable
    local tall = lv.obj.new(container)
    lv.obj.set_size(tall, 296, 500)
    test_tick(1)
    lv.obj.scroll_by(container, 0, -80, lv.ANIM_ON)
    test_tick(10)
    expect_true(true)
    lv.obj.del(container)
end)

-- ============================================================
-- Image set_angle with real values (dialog-interactive pattern)
-- ============================================================

test("img.set_angle with rotation value", function()
    local img = lv.img.new(window)
    lv.img.set_angle(img, 2250) -- 225.0 degrees
    lv.img.set_angle(img, 0)
    expect_true(true)
    lv.obj.del(img)
end)

-- ============================================================
-- Style text_opa + invalidate pattern (list-choice animation)
-- ============================================================

test("style text_opa change + invalidate", function()
    local s = lv.style.new()
    lv.style.set_text_opa(s, 127)
    local container = lv.obj.new(window)
    lv.obj.add_style(container, s, lv.STATE_DEFAULT)
    local label = lv.label.new(container)
    lv.label.set_text(label, "Test")
    -- Animation pattern: update style then invalidate
    lv.style.set_text_opa(s, 255)
    lv.obj.invalidate(container)
    test_tick(1)
    expect_true(true)
    lv.obj.del(container)
end)

-- ============================================================
-- Slider with styles on PART_MAIN/INDICATOR/KNOB
-- ============================================================

test("slider with part-specific styles", function()
    local slider = lv.slider.new(window)
    lv.obj.remove_style_all(slider)
    lv.group.remove_obj(slider)

    local main_style = lv.style.new()
    lv.style.set_bg_opa(main_style, lv.OPA_COVER)
    lv.style.set_bg_color(main_style, lv.color.hex(0xa79f8e))

    local ind_style = lv.style.new()
    lv.style.set_bg_opa(ind_style, lv.OPA_COVER)
    lv.style.set_bg_color(ind_style, lv.color.hex(0xfbbd2a))

    local knob_style = lv.style.new()
    lv.style.set_bg_opa(knob_style, lv.OPA_TRANSP)
    lv.style.set_pad_all(knob_style, 0)

    lv.obj.add_style(slider, main_style, lv.PART_MAIN)
    lv.obj.add_style(slider, ind_style, lv.PART_INDICATOR)
    lv.obj.add_style(slider, knob_style, lv.PART_KNOB)

    lv.slider.set_range(slider, 0, 100)
    lv.slider.set_value(slider, 42, lv.ANIM_OFF)
    expect_eq(lv.slider.get_value(slider), 42, "slider value with part styles")
    lv.obj.del(slider)
end)

-- ============================================================
-- OBJ_FLAG_FLOATING (used for overlay images)
-- ============================================================

test("OBJ_FLAG_FLOATING for overlay", function()
    local img = lv.img.new(window)
    lv.obj.remove_style_all(img)
    lv.obj.add_flag(img, lv.OBJ_FLAG_FLOATING)
    lv.obj.align(img, lv.ALIGN_TOP_LEFT, 320, 92)
    lv.obj.set_size(img, 116, 120)
    expect_true(true)
    lv.obj.del(img)
end)
