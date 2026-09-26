-- ============================================================================
--  POC : Ecriture via dangling pointer — corruption d'objet  (F03, lot L2)
--
--  Scenario :
--    1. Creer un spy label, le supprimer
--    2. Un autre label "victime" reutilise le meme bloc
--    3. Ecrire via le spy → le texte de la victime changeait
--    4. La victime n'a jamais ete touchee directement
--
--  TEST : lecture et ecriture via le spy doivent lever une erreur Lua
--  propre et la victime doit garder son texte.
--  ATTENDU : ECHOUE (crash ou assertion) AVANT LE CORRECTIF L2.
--  Lance seul (processus separe) par test_run.bat.
-- ============================================================================

require("test_helpers")
print("--- poc_write_corruption ---")

test("ecriture via un label supprime", function()
    local spy = lv.label.new(window)
    lv.label.set_text(spy, ".")
    lv.obj.add_flag(spy, lv.OBJ_FLAG_HIDDEN)
    lv.obj.del(spy)

    local victim = lv.label.new(window)
    lv.label.set_text(victim, "Texte original du firmware")
    lv.obj.add_flag(victim, lv.OBJ_FLAG_HIDDEN)

    expect_error(function() lv.label.get_text(spy) end,
                 "get_text sur spy supprime")
    expect_error(function() lv.label.set_text(spy, "CORROMPU PAR UAF") end,
                 "set_text sur spy supprime")
    expect_error(function() lv.obj.add_flag(spy, lv.OBJ_FLAG_HIDDEN) end,
                 "add_flag sur spy supprime")

    expect_eq(lv.label.get_text(victim), "Texte original du firmware",
              "victime intacte")
end)
