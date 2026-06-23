require("test_helpers")
print("--- test_firmware ---")

test("state global exists and is table", function()
    expect_type(state, "table", "state type")
end)

test("state can store and retrieve values", function()
    state.test_key = 42
    expect_eq(state.test_key, 42, "state value")
    state.test_key = nil
end)

test("progression global exists and is table", function()
    expect_type(progression, "table", "progression type")
end)

test("progression.save and load", function()
    expect_true(progression.save ~= nil, "save exists")
    expect_true(progression.load ~= nil, "load exists")
end)

test("context_menu global exists", function()
    expect_type(context_menu, "table", "context_menu type")
end)

test("context_menu.set_entries", function()
    expect_true(context_menu.set_entries ~= nil, "set_entries exists")
    -- Set entries with a table
    context_menu.set_entries({
        { title = "Test", cb = function() end }
    })
    expect_true(true)
end)

test("goto_library exists", function()
    expect_type(goto_library, "function", "goto_library type")
end)

test("back_callback exists", function()
    expect_type(back_callback, "function", "back_callback type")
end)

test("screen global exists", function()
    expect_type(screen, "table", "screen type")
end)

test("screen stub functions", function()
    screen.set_state("on")
    screen.wake_up()
    screen.set_brightness(100)
    screen.on_state_changed()
    expect_true(true)
end)

test("progress global exists", function()
    expect_true(progress ~= nil, "progress exists")
    expect_type(progress, "number", "progress type")
end)
