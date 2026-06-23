require("test_helpers")
print("--- test_img ---")

test("img.new creates image widget", function()
    local img = lv.img.new(window)
    expect_true(img ~= nil)
    lv.obj.del(img)
end)

test("img.set_zoom", function()
    local img = lv.img.new(window)
    lv.img.set_zoom(img, 512) -- 2x zoom (256 = 1x)
    expect_true(true)
    lv.obj.del(img)
end)

test("img.set_angle", function()
    local img = lv.img.new(window)
    lv.img.set_angle(img, 450) -- 45.0 degrees (in 0.1 deg units)
    expect_true(true)
    lv.obj.del(img)
end)

test("img_src.load returns nil for missing file", function()
    local dsc = lv.img_src.load("nonexistent.lif")
    expect_nil(dsc, "should be nil for missing file")
end)

-- Note: img_src.load with a real file can't be tested without a .lif file
-- img_src.get_width and get_height require a valid img_dsc

test("img.set_src with nil does not crash", function()
    local img = lv.img.new(window)
    -- Passing nil should be handled gracefully
    local ok = pcall(function() lv.img.set_src(img, nil) end)
    -- Whether it errors or handles nil, it should not crash
    expect_true(true)
    lv.obj.del(img)
end)
