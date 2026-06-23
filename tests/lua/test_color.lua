require("test_helpers")
print("--- test_color ---")

test("color.hex returns integer", function()
    local c = lv.color.hex(0xFF0000)
    expect_type(c, "number", "color type")
end)

test("color.make returns integer", function()
    local c = lv.color.make(255, 128, 0)
    expect_type(c, "number", "color type")
end)

test("color.black", function()
    local c = lv.color.black()
    expect_type(c, "number", "color type")
end)

test("color.white", function()
    local c = lv.color.white()
    expect_type(c, "number", "color type")
end)

test("color.hex(0) == color.black()", function()
    expect_eq(lv.color.hex(0x000000), lv.color.black(), "black match")
end)

test("color.hex(0xFFFFFF) == color.white()", function()
    expect_eq(lv.color.hex(0xFFFFFF), lv.color.white(), "white match")
end)

test("lv.pct returns integer", function()
    local p = lv.pct(50)
    expect_type(p, "number", "pct type")
end)

test("lv.pct(100) == lv.PCT_100", function()
    expect_eq(lv.pct(100), lv.PCT_100, "PCT_100")
end)
