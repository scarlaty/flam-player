-- ============================================================================
--  POC : Fuite memoire via Use-After-Free  (F03, lot L2)
-- ============================================================================
--
--  SCENARIO
--  --------
--  Un ecran "Parametres" affiche des donnees sensibles (mot de passe WiFi,
--  PIN...). Quand l'ecran est ferme, les labels sont supprimes. Un script qui
--  a garde les references Lua pouvait RELIRE ces donnees depuis la RAM, ou
--  celles d'un label qui reutilise le bloc (lecture cross-objet).
--
--  TEST : get_text sur la reference volee doit lever une erreur Lua propre.
--  ATTENDU : ECHOUE (crash ou assertion) AVANT LE CORRECTIF L2.
--  Lance seul (processus separe) par test_run.bat.
--
-- ============================================================================

require("test_helpers")
print("--- poc_memory_leak ---")

test("relecture d'un label secret supprime", function()
    local secret_text = "Pass: Xk9#mP2$vL"
    local secret_label = lv.label.new(window)
    lv.label.set_text(secret_label, secret_text)
    lv.obj.align(secret_label, lv.ALIGN_TOP_MID, 0, 35)
    local stolen_ref = secret_label

    lv.obj.del(secret_label)

    -- Label de meme taille cree juste apres : reutilise le bloc libere
    local replacement = lv.label.new(window)
    lv.label.set_text(replacement, "Bienvenue dans le jeu !")

    local ok, leaked = pcall(lv.label.get_text, stolen_ref)
    if ok then
        error("get_text(stolen_ref) apres del a renvoye \"" .. tostring(leaked) .. "\"")
    end
    expect_eq(lv.label.get_text(replacement), "Bienvenue dans le jeu !",
              "label de remplacement intact")
end)
