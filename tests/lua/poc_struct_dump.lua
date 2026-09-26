-- ============================================================================
--  POC : Dump de la RAM via multiples dangling pointers  (F03, lot L2)
-- ============================================================================
--
--  Strategie : creer N labels espions, les supprimer, laisser le firmware
--  reutiliser la memoire, puis lire via get_text + get_state +
--  get_child_cnt + get_scroll_y sur chaque espion.
--
--  TEST : chaque lecture via un espion supprime doit lever une erreur Lua
--  propre (aucune donnee lue).
--  ATTENDU : ECHOUE (crash ou assertion) AVANT LE CORRECTIF L2.
--  Lance seul (processus separe) par test_run.bat.
--
-- ============================================================================

require("test_helpers")
print("--- poc_struct_dump ---")

test("dump via 20 espions supprimes", function()
    local NUM = 20
    local spies = {}
    for i = 1, NUM do
        local s = lv.label.new(window)
        lv.label.set_text(s, ".")
        lv.obj.add_flag(s, lv.OBJ_FLAG_HIDDEN)
        spies[i] = s
    end
    for i = 1, NUM do
        lv.obj.del(spies[i])
    end

    -- Activite firmware : donnees variees dans des labels
    local fw_labels = {}
    for i = 1, NUM do
        fw_labels[i] = lv.label.new(window)
        lv.label.set_text(fw_labels[i], string.format("Secret-%02d: 0x%08X", i, i * 2654435761 % 4294967296))
        lv.obj.add_flag(fw_labels[i], lv.OBJ_FLAG_HIDDEN)
    end

    local readers = {
        { "get_text",      lv.label.get_text },
        { "get_state",     lv.obj.get_state },
        { "get_child_cnt", lv.obj.get_child_cnt },
        { "get_scroll_y",  lv.obj.get_scroll_y },
    }
    local leaks = {}
    for i = 1, NUM do
        for _, r in ipairs(readers) do
            local ok, v = pcall(r[2], spies[i])
            if ok then
                leaks[#leaks + 1] = string.format("[%d] %s=%s", i, r[1], tostring(v))
            end
        end
    end
    if #leaks > 0 then
        error(#leaks .. " lectures via espions supprimes : " ..
              table.concat(leaks, ", ", 1, math.min(#leaks, 5)))
    end
end)
