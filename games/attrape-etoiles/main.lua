-- ============================================================================
-- main.lua — Bootstrap d'un homebrew FLAM (genere par new_project.py).
-- Le firmware charge ce fichier puis appelle setup().
--
-- Regles device (ne pas casser) :
--   * aucun require() au top-level : le searcher n'existe qu'apres le chargement
--     de main.lua -> tous les require sont DANS setup() ;
--   * Global est une GLOBALE (pas de `local`) : les modules y accedent par nom ;
--   * le jeu est un MODULE charge par Global.load_module : le firmware ne rend
--     et ne route les touches que pour le module courant.
-- ============================================================================

function setup()
    Global = require("global")
    Global.init()
    Global.isMultiBranches = true

    -- Retour (bouton retour du device / Echap au simulateur) -> bibliotheque.
    Global.setBackToLibrary()

    Global.load_module("etoiles", "1_0_0").create({})
end
