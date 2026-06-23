-- ============================================================================
-- main.lua — Bootstrap du jeu casse-briques (contenu Lua FLAM, 0 firmware).
-- Le firmware appelle setup() apres avoir charge main.lua (meme convention que
-- les histoires). On boote directement sur la branche "game" (pas de nodes).
-- ============================================================================

-- NB : Global est une GLOBALE (pas de `local`), comme dans les histoires : les
-- modules/branches y accedent par le nom `Global`.
function setup()
    Global = require("global")
    Global.init()
    Global.isMultiBranches = true

    -- Jeu sans sauvegarde de progression : retour (ESC / back) -> bibliotheque.
    -- Pas de setProgression => isStoryStarted reste faux => pas de relance figee
    -- (issue #1 ne s'applique qu'aux histoires avec .prog).
    Global.setBackToLibrary()

    -- Le jeu est un MODULE (pas une branche) : sur device le firmware ne rend/route
    -- QUE le module courant charge par load_module (cf. DEVICE_VS_SIM.md #3).
    Global.load_module("breakout", "1_0_0").create({})
end
