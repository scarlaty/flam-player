require("test_helpers")
print("--- test_event ---")

test("event.send fires callback", function()
    local obj = lv.obj.new(window)
    local fired = false
    lv.obj.add_event_cb(obj, function(e)
        fired = true
    end, lv.EVENT_CLICKED)
    lv.event.send(obj, lv.EVENT_CLICKED)
    expect_true(fired, "callback should fire")
    lv.obj.del(obj)
end)

test("event.get_code in callback", function()
    local obj = lv.obj.new(window)
    local code = nil
    lv.obj.add_event_cb(obj, function(e)
        code = lv.event.get_code(e)
    end, lv.EVENT_CLICKED)
    lv.event.send(obj, lv.EVENT_CLICKED)
    expect_eq(code, lv.EVENT_CLICKED, "event code")
    lv.obj.del(obj)
end)

test("event.get_target in callback", function()
    local obj = lv.obj.new(window)
    local target = nil
    lv.obj.add_event_cb(obj, function(e)
        target = lv.event.get_target(e)
    end, lv.EVENT_CLICKED)
    lv.event.send(obj, lv.EVENT_CLICKED)
    expect_eq(target, obj, "event target should be obj")
    lv.obj.del(obj)
end)

test("multiple event callbacks on same obj", function()
    local obj = lv.obj.new(window)
    local count = 0
    lv.obj.add_event_cb(obj, function(e) count = count + 1 end, lv.EVENT_CLICKED)
    lv.obj.add_event_cb(obj, function(e) count = count + 10 end, lv.EVENT_CLICKED)
    lv.event.send(obj, lv.EVENT_CLICKED)
    expect_eq(count, 11, "both callbacks fired")
    lv.obj.del(obj)
end)

test("event callback with different event codes", function()
    local obj = lv.obj.new(window)
    local clicked = false
    local pressed = false
    lv.obj.add_event_cb(obj, function(e) clicked = true end, lv.EVENT_CLICKED)
    lv.obj.add_event_cb(obj, function(e) pressed = true end, lv.EVENT_PRESSED)
    lv.event.send(obj, lv.EVENT_CLICKED)
    expect_true(clicked, "clicked fired")
    expect_true(not pressed, "pressed not fired")
    lv.obj.del(obj)
end)

test("event.get_key_value returns string for string.byte()", function()
    local obj = lv.obj.new(window)
    local key_val = nil
    lv.obj.add_event_cb(obj, function(e)
        key_val = lv.event.get_key_value(e)
    end, lv.EVENT_KEY)
    -- Send KEY event with KEY_LEFT as param
    lv.event.send(obj, lv.EVENT_KEY, lv.KEY_LEFT)
    expect_type(key_val, "string", "get_key_value should return string")
    expect_eq(string.byte(key_val), lv.KEY_LEFT,
        "string.byte(get_key_value) should equal KEY_LEFT")
end)

test("event.get_key_value works for KEY_RIGHT", function()
    local obj = lv.obj.new(window)
    local key_val = nil
    lv.obj.add_event_cb(obj, function(e)
        key_val = lv.event.get_key_value(e)
    end, lv.EVENT_KEY)
    lv.event.send(obj, lv.EVENT_KEY, lv.KEY_RIGHT)
    expect_eq(string.byte(key_val), lv.KEY_RIGHT,
        "string.byte(get_key_value) should equal KEY_RIGHT")
    lv.obj.del(obj)
end)

test("event callback error does not crash", function()
    local obj = lv.obj.new(window)
    lv.obj.add_event_cb(obj, function(e)
        error("intentional error in callback")
    end, lv.EVENT_CLICKED)
    -- Should not crash, error is caught internally
    lv.event.send(obj, lv.EVENT_CLICKED)
    expect_true(true)
    lv.obj.del(obj)
end)
