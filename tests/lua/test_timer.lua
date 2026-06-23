require("test_helpers")
print("--- test_timer ---")

test("timer.new creates timer", function()
    local t = lv.timer.new(function() end, 100)
    expect_true(t ~= nil)
    lv.timer.del(t)
end)

test("timer fires callback", function()
    local fired = false
    local t = lv.timer.new(function() fired = true end, 1)
    lv.timer.set_repeat_count(t, 1)
    -- Advance LVGL timers
    test_tick(5)
    expect_true(fired, "timer should fire")
end)

test("timer.reset", function()
    local count = 0
    local t = lv.timer.new(function() count = count + 1 end, 10000)
    lv.timer.reset(t)
    expect_true(true)
    lv.timer.del(t)
end)

test("timer.set_repeat_count", function()
    local count = 0
    local t = lv.timer.new(function() count = count + 1 end, 1)
    lv.timer.set_repeat_count(t, 3)
    -- Tick multiple times with small delays to let LVGL process
    for i = 1, 50 do test_tick(1) end
    expect_true(count >= 1, "timer should fire at least once")
end)

test("timer.del does not crash", function()
    local t = lv.timer.new(function() end, 100)
    lv.timer.del(t)
    expect_true(true)
end)
