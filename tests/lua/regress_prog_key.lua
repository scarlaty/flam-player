require("test_helpers")
print("--- regress_prog_key ---")
local bad = { "../x", "..\\x", "a/b", "a\\b", "C:x", "a:b", "", "a.b", "a b",
              "a\0b", string.rep("k", 65), "..", "/abs" }
for _, k in ipairs(bad) do
    test("save refuse " .. string.format("%q", k), function()
        local ok, err = pcall(progression.save, k, { v = 1 })
        expect_true(not ok, "save doit echouer")
        expect_true(tostring(err):find("cle de progression invalide", 1, true), tostring(err))
    end)
    test("load refuse " .. string.format("%q", k), function()
        local ok = pcall(progression.load, k)
        expect_true(not ok, "load doit echouer")
    end)
end
for _, k in ipairs({ "chaps", "A-z_09", string.rep("k", 64) }) do
    test("cle valide " .. k, function()
        progression.save(k, { v = 1 })
        expect_type(progression.load(k), "table", "load")
    end)
end
