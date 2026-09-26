-- ============================================================================
--  POC : Lecture de donnees residuelles en RAM via Use-After-Free  (F03, L2)
-- ============================================================================
--
--  Apres del(), la RAM n'est pas nettoyee et le dangling pointer permet de
--  lire ce qu'un AUTRE objet a ecrit dans la meme zone (ici des donnees
--  simulant l'activite du firmware).
--
--  TEST : get_text sur le label supprime doit lever une erreur Lua propre,
--  jamais renvoyer le texte residuel ni celui d'un autre label.
--  ATTENDU : ECHOUE (crash ou assertion) AVANT LE CORRECTIF L2.
--  Lance seul (processus separe) par test_run.bat.
--
-- ============================================================================

require("test_helpers")
print("--- poc_residual_read ---")

test("lecture via un label espion supprime", function()
    -- 1. Label espion, puis suppression
    local spy = lv.label.new(window)
    lv.label.set_text(spy, "AAA")
    lv.obj.add_flag(spy, lv.OBJ_FLAG_HIDDEN)
    lv.obj.del(spy)

    -- 2. Activite simulee du firmware : l'allocateur reutilise les blocs
    local internal_data = {
        "BLE:paired=A4:C1:38:FF",
        "wifi_psk=M0nR3seau!",
        "user_token=9f3a7c1b2e",
        "device_key=HMAC-SHA256",
        "fw_build=2024.3.15-rc2",
        "battery_sn=LiPo-04812",
        "crash_log=0xDEADBEEF",
        "heap_free=14832 bytes",
    }
    local internal_labels = {}
    for i, text in ipairs(internal_data) do
        internal_labels[i] = lv.label.new(window)
        lv.label.set_text(internal_labels[i], text)
        lv.obj.add_flag(internal_labels[i], lv.OBJ_FLAG_HIDDEN)
    end

    -- 3. Lecture via le dangling pointer : erreur Lua attendue
    local ok, leaked = pcall(lv.label.get_text, spy)
    if ok then
        error("get_text(spy) apres del a renvoye \"" .. tostring(leaked) .. "\"")
    end

    -- 4. Les labels firmware sont intacts
    for i, text in ipairs(internal_data) do
        expect_eq(lv.label.get_text(internal_labels[i]), text, "label firmware #" .. i)
    end
end)
