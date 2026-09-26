-- regress_pcall_recursion.lua : recursion pcall en Lua pur.
-- Avant : pile C de 1 Mo en Debug -> 0xC00000FD (crash natif du player).
-- Apres : pile de 8 Mo (CMakeLists.txt), Lua s'arrete proprement
-- ("C stack overflow") et le runner survit.
require("test_helpers")

test("recursion pcall bornee sans crash natif", function()
    local depth = 0
    local function f()
        depth = depth + 1
        return pcall(f)
    end
    pcall(f)
    expect_ge(depth, 100, "profondeur atteinte")
end)
