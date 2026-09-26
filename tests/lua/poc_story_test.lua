-- poc_story_test.lua — Espions supprimes lus depuis setup()  (F03, lot L2)
-- Meme scenario que poc_residual_read, mais depuis setup() comme une
-- histoire FLAM (le runner appelle setup() apres le chargement).
-- TEST : lire un label supprime doit lever une erreur Lua propre.
-- ATTENDU : ECHOUE (crash ou assertion) AVANT LE CORRECTIF L2.
-- Lance seul (processus separe) par test_run.bat.

require("test_helpers")
print("--- poc_story_test ---")

function setup()
    local box = lv.obj.new(window)
    lv.obj.set_size(box, 290, 120)
    local btn = lv.btn.new(window)
    lv.obj.set_size(btn, 120, 35)

    -- 5 espions supprimes
    local NUM = 5
    local spies = {}
    for i = 1, NUM do
        spies[i] = lv.label.new(window)
        lv.label.set_text(spies[i], ".")
        lv.obj.add_flag(spies[i], lv.OBJ_FLAG_HIDDEN)
    end
    for i = 1, NUM do
        lv.obj.del(spies[i])
    end

    -- 10 labels (plus que les 5 espions) reutilisent les blocs
    local labels = {}
    local texts = {
        "Le Petit Prince", "Chapitre I", "Saint-Exupery",
        "Suite >", "Chapitre II", "Page 1",
        "Il etait une fois", "Un aviateur", "Le desert",
        "Une rose",
    }
    for i = 1, 10 do
        labels[i] = lv.label.new(window)
        lv.label.set_text(labels[i], texts[i])
        lv.obj.add_flag(labels[i], lv.OBJ_FLAG_HIDDEN)
    end

    for _, i in ipairs({ 1, 3, 5 }) do
        test("get_text sur espion supprime #" .. i, function()
            expect_error(function() lv.label.get_text(spies[i]) end)
        end)
    end

    test("labels de l'histoire intacts", function()
        for i = 1, 10 do
            expect_eq(lv.label.get_text(labels[i]), texts[i], "label #" .. i)
        end
    end)
end
