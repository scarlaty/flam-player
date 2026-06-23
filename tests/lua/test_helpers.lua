-- test_helpers.lua — Minimal test framework for flam-player bindings

TEST_PASS = 0
TEST_FAIL = 0

function test(name, fn)
    local ok, err = pcall(fn)
    if ok then
        TEST_PASS = TEST_PASS + 1
        io.write(string.format("  [PASS] %s\n", name))
    else
        TEST_FAIL = TEST_FAIL + 1
        io.write(string.format("  [FAIL] %s: %s\n", name, tostring(err)))
    end
end

function expect_eq(a, b, msg)
    if a ~= b then
        error(string.format("%s: expected %s, got %s",
              msg or "mismatch", tostring(b), tostring(a)), 2)
    end
end

function expect_neq(a, b, msg)
    if a == b then
        error(string.format("%s: expected not %s",
              msg or "unexpected equal", tostring(a)), 2)
    end
end

function expect_true(v, msg)
    if not v then error(msg or "expected true, got falsy", 2) end
end

function expect_nil(v, msg)
    if v ~= nil then
        error(string.format("%s: expected nil, got %s",
              msg or "not nil", tostring(v)), 2)
    end
end

function expect_type(v, t, msg)
    if type(v) ~= t then
        error(string.format("%s: expected type %s, got %s",
              msg or "type mismatch", t, type(v)), 2)
    end
end

function expect_error(fn, msg)
    local ok, _ = pcall(fn)
    if ok then error(msg or "expected error but succeeded", 2) end
end

function expect_ge(a, b, msg)
    if a < b then
        error(string.format("%s: expected %s >= %s",
              msg or "not >=", tostring(a), tostring(b)), 2)
    end
end
