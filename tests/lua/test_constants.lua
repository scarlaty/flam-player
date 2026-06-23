require("test_helpers")
print("--- test_constants ---")

-- Verify all constants exist and are numbers

local constants = {
    -- Events
    "EVENT_CLICKED", "EVENT_PRESSED", "EVENT_RELEASED",
    "EVENT_FOCUSED", "EVENT_DEFOCUSED", "EVENT_KEY",
    "EVENT_SCROLL_END", "EVENT_VALUE_CHANGED", "EVENT_DELETE",
    "EVENT_READY", "EVENT_CANCEL", "EVENT_SCROLL_BEGIN",
    -- Keys
    "KEY_LEFT", "KEY_RIGHT", "KEY_ENTER", "KEY_ESC",
    "KEY_UP", "KEY_DOWN", "KEY_NEXT", "KEY_PREV",
    -- Alignment
    "ALIGN_DEFAULT", "ALIGN_TOP_LEFT", "ALIGN_TOP_MID", "ALIGN_TOP_RIGHT",
    "ALIGN_BOTTOM_LEFT", "ALIGN_BOTTOM_MID", "ALIGN_BOTTOM_RIGHT",
    "ALIGN_LEFT_MID", "ALIGN_RIGHT_MID", "ALIGN_CENTER",
    -- Flex
    "FLEX_FLOW_ROW", "FLEX_FLOW_COLUMN", "FLEX_FLOW_ROW_WRAP", "FLEX_FLOW_COLUMN_WRAP",
    "FLEX_ALIGN_START", "FLEX_ALIGN_END", "FLEX_ALIGN_CENTER",
    "FLEX_ALIGN_SPACE_EVENLY", "FLEX_ALIGN_SPACE_AROUND", "FLEX_ALIGN_SPACE_BETWEEN",
    -- States
    "STATE_DEFAULT", "STATE_FOCUSED", "STATE_PRESSED", "STATE_CHECKED",
    "STATE_DISABLED", "STATE_FOCUS_KEY",
    "STATE_USER_1", "STATE_USER_2", "STATE_USER_3", "STATE_USER_4",
    -- Opacity
    "OPA_TRANSP", "OPA_COVER",
    "OPA_10", "OPA_20", "OPA_30", "OPA_40", "OPA_50",
    "OPA_60", "OPA_70", "OPA_80", "OPA_90",
    -- Parts
    "PART_MAIN", "PART_SCROLLBAR", "PART_INDICATOR", "PART_KNOB",
    -- Flags
    "OBJ_FLAG_HIDDEN", "OBJ_FLAG_CLICKABLE", "OBJ_FLAG_SCROLLABLE",
    "OBJ_FLAG_CLICK_FOCUSABLE", "OBJ_FLAG_FLOATING",
    "OBJ_FLAG_SCROLL_ONE", "OBJ_FLAG_SNAPPABLE",
    -- Animation
    "ANIM_ON", "ANIM_OFF", "ANIM_REPEAT_INFINITE",
    -- Gradient
    "GRAD_DIR_NONE", "GRAD_DIR_VER", "GRAD_DIR_HOR",
    -- Dither
    "DITHER_NONE", "DITHER_ORDERED", "DITHER_ERR_DIFF",
    -- Border
    "BORDER_SIDE_NONE", "BORDER_SIDE_BOTTOM", "BORDER_SIDE_TOP",
    "BORDER_SIDE_LEFT", "BORDER_SIDE_RIGHT", "BORDER_SIDE_FULL",
    -- Scroll snap
    "SCROLL_SNAP_NONE", "SCROLL_SNAP_START", "SCROLL_SNAP_END", "SCROLL_SNAP_CENTER",
    -- Radius
    "RADIUS_CIRCLE",
    -- Label long mode
    "LABEL_LONG_WRAP", "LABEL_LONG_DOT", "LABEL_LONG_SCROLL",
    "LABEL_LONG_SCROLL_CIRCULAR", "LABEL_LONG_CLIP",
    -- Text align
    "TEXT_ALIGN_LEFT", "TEXT_ALIGN_CENTER", "TEXT_ALIGN_RIGHT", "TEXT_ALIGN_AUTO",
    -- Size
    "SIZE_CONTENT", "PCT_100",
}

for _, name in ipairs(constants) do
    test("lv." .. name .. " exists", function()
        local v = lv[name]
        expect_true(v ~= nil, name .. " is nil")
        expect_type(v, "number", name)
    end)
end

-- Font constants
local fonts = {
    "nunito_bold_12", "nunito_bold_16", "nunito_bold_20",
    "nunito_extrabold_12", "nunito_extrabold_14",
    "nunito_extrabold_16", "nunito_extrabold_20",
}

for _, name in ipairs(fonts) do
    test("lv.font." .. name .. " exists", function()
        expect_true(lv.font[name] ~= nil, name .. " is nil")
    end)
end

-- Animation path constants
local paths = {
    "path_ease_in_out", "path_linear", "path_ease_out",
    "path_ease_in", "path_overshoot", "path_bounce",
}

for _, name in ipairs(paths) do
    test("lv.anim." .. name .. " exists", function()
        expect_true(lv.anim[name] ~= nil, name .. " is nil")
    end)
end
