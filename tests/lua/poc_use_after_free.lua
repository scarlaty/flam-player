-- ============================================================================
--  POC : Use-After-Free dans les bindings LVGL/Lua  (F03, lot L2)
-- ============================================================================
--
--  CONTEXTE
--  --------
--  Decalage entre la duree de vie d'un objet cote C (geree par LVGL) et sa
--  reference cote Lua (geree par le GC) : lua_lv_check_obj() renvoie le
--  pointeur meme apres lv_obj_del(), et l_obj_del() ne met pas *ud = NULL.
--  Toute utilisation ulterieure de la variable Lua est un Use-After-Free
--  (CWE-416) : lecture de donnees d'un autre objet, ecriture dans le tas.
--
--  ANALOGIE : on garde la cle de la chambre 42 apres la demolition de la
--  chambre ; y retourner, c'est tomber dans le vide.
--
--  TEST
--  ----
--  Ce fichier n'est plus une demo : il verifie le comportement SUR attendu,
--  a savoir qu'un acces a un objet supprime leve une erreur Lua propre
--  (expect_error) au lieu de lire/ecrire la memoire liberee.
--  ATTENDU : ECHOUE (crash ou assertion) AVANT LE CORRECTIF L2.
--  Lance seul (processus separe) par test_run.bat.
--
-- ============================================================================

require("test_helpers")
print("--- poc_use_after_free ---")

test("acces a un bouton supprime apres heap spray", function()
    -- 1. Objet valide
    local btn = lv.btn.new(window)
    lv.obj.set_size(btn, 100, 40)
    lv.obj.set_pos(btn, 50, 50)
    local label = lv.label.new(btn)
    lv.label.set_text(label, "Cliquez-moi")
    expect_eq(lv.obj.get_width(btn), 100, "largeur avant del")

    -- 2. Suppression cote C, la reference Lua reste
    lv.obj.del(btn)

    -- 3. Heap spray : objets de taille similaire pour reutiliser le bloc
    local spray = {}
    for i = 1, 30 do
        spray[i] = lv.obj.new(window)
        lv.obj.set_size(spray[i], 10, 10)
    end

    -- 4. Lecture et ecriture sur l'objet supprime : erreur Lua attendue
    expect_error(function() lv.obj.get_width(btn) end,
                 "get_width sur objet supprime")
    expect_error(function() lv.obj.set_size(btn, 999, 999) end,
                 "set_size sur objet supprime")
    expect_error(function() lv.label.get_text(label) end,
                 "get_text sur enfant d'un objet supprime")

    -- 5. Les objets du spray ne doivent pas avoir ete touches
    for i = 1, #spray do
        expect_eq(lv.obj.get_width(spray[i]), 10, "spray intact #" .. i)
    end
end)
