require("test_helpers")
-- Mesure du registre Lua sur stress anim (10 rounds)
local function reg_stats()
  local r = debug.getregistry()
  local live, free, total = 0, 0, 0
  for k, v in pairs(r) do
    total = total + 1
    if math.type(k) == "integer" then
      if math.type(v) == "integer" then free = free + 1 else live = live + 1 end
    end
  end
  return live, free, total
end
local keep_obj = lv.obj.new()
local function run(n, mode)
  for i = 1, n do
    local a = lv.anim.new(); lv.anim.set_values(a, 0, 1); lv.anim.set_time(a, 1)
    if mode == 1 then
      lv.anim.set_exec_cb(a, function() end)
    elseif mode == 2 then
      -- closure qui capture l'anim elle-meme (cycle via le registre)
      lv.anim.set_exec_cb(a, function() local _ = a end)
    else
      local o = lv.obj.new(keep_obj)
      lv.anim.set_var(a, o)
      lv.anim.set_exec_cb(a, function(obj, v) local _ = a; pcall(lv.obj.set_x, obj, v) end)
      if i % 3 == 0 then lv.obj.del(o) end
    end
    lv.anim.start(a)
    if i % 1000 == 0 then test_tick(2) end
  end
  test_tick(10)
  lv.obj.clean(keep_obj)
  test_tick(2)
  collectgarbage("collect"); collectgarbage("collect")
end
collectgarbage("collect")
local l0 = reg_stats()
local first
test("registre Lua stable sur 10 rounds d'anims", function()
for round = 1, 10 do
  run(4000, 1); run(4000, 2); run(2000, 3)
  local live, free, total = reg_stats()
  print(string.format("round %2d : live=%d free=%d total=%d luaKB=%.0f lvgl_free=%d", round, live, free, total,
    collectgarbage("count"), lv.mem.monitor().free_size))
  if round == 2 then first = live end
  if round == 10 then
    expect_true(live <= first + 5, string.format("fuite registre : %d -> %d refs vivantes", first, live))
  end
end
end)
print("REG_OK live0=" .. l0)
