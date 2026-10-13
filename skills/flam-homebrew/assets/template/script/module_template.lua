-- ============================================================================
-- __MODULE_____VERSION__.lua — Module de jeu FLAM (gabarit new_project.py).
-- Charge par Global.load_module("__MODULE__", "__VERSION__") depuis main.lua.
--
-- Contrat module : create(args) construit l'ecran, clean() detruit TOUT ce que
-- create a cree (timers, callbacks, objets). Global.cleanCurrentModule appelle
-- clean() puis vide package.loaded : le module est recharge a neuf ensuite,
-- donc l'etat vit dans des upvalues reinitialisees au chargement.
--
-- Demo : un carre deplace a la molette (gauche/droite), le clic molette
-- (ENTER) change sa couleur, une balle rebondit via un timer de 25 ms.
--
-- Entrees (verifie sur la Flam) : la molette est un ENCODEUR LVGL. Gauche /
-- droite arrivent en EVENT_KEY (20 / 19) ; le clic molette n'arrive PAS en
-- EVENT_KEY 10 mais en EVENT_CLICKED. On ecoute donc les deux, et un meme
-- appui ne compte qu'une fois.
-- ============================================================================

local game = {}

local W = Global.visual_width    -- 320
local H = Global.visual_height   -- 212 (sous le bandeau de 28 px)

local KEY_RIGHT, KEY_LEFT, KEY_ENTER = 19, 20, 10
local STEP = 16
local SIZE = 24
local BALL = 10
local COLORS = { 0xe6332a, 0xf18d05, 0xfbbd2a, 0x4caf50, 0x2196f3 }

game.styles = {}
game.events = {}
local container, player, ball, label, loopTimer
local px, colorIdx, bx, by, vx, vy, hits
local ticks, lastEnter = 0, -100

-- ---------------------------------------------------------------------------
function game.clean()
    if loopTimer ~= nil then lv.timer.del(loopTimer); loopTimer = nil end
    for obj, ev in pairs(game.events) do
        for _, cb in pairs(ev) do lv.obj.remove_event_cb(obj, cb) end
    end
    Global.requestAudioStop(true, true)
    lv.group.set_editing(document, false)
    lv.group.remove_all_objs(document)
    lv.obj.clean(window)
    container, player, ball, label = nil, nil, nil, nil
    game.styles = {}
    game.events = {}
end

-- ---------------------------------------------------------------------------
local function initStyles()
    local S = game.styles
    S.bg = lv.style.new()
    lv.style.set_bg_color(S.bg, lv.color.black())
    lv.style.set_bg_opa(S.bg, lv.OPA_COVER)
    lv.style.set_pad_all(S.bg, 0)

    S.player = lv.style.new()
    lv.style.set_bg_opa(S.player, lv.OPA_COVER)
    lv.style.set_radius(S.player, 4)

    S.ball = lv.style.new()
    lv.style.set_bg_color(S.ball, lv.color.hex(0xffffff))
    lv.style.set_bg_opa(S.ball, lv.OPA_COVER)
    lv.style.set_radius(S.ball, BALL)

    S.text = lv.style.new()
    lv.style.set_text_color(S.text, lv.color.hex(0xefedea))
    lv.style.set_text_font(S.text, lv.font.nunito_extrabold_16)
end

local function refresh()
    lv.obj.set_style_bg_color(player, lv.color.hex(COLORS[colorIdx]), lv.STATE_DEFAULT)
    lv.obj.set_pos(player, px, H - SIZE - 8)
    lv.label.set_text(label, "Rebonds : " .. hits)
end

local function tick()
    ticks = ticks + 1
    local before = hits
    bx = bx + vx
    by = by + vy
    if bx <= 0 then bx = 0; vx = -vx; hits = hits + 1 end
    if bx >= W - BALL then bx = W - BALL; vx = -vx; hits = hits + 1 end
    if by <= 24 then by = 24; vy = -vy; hits = hits + 1 end
    if by >= H - BALL then by = H - BALL; vy = -vy; hits = hits + 1 end
    lv.obj.set_pos(ball, bx, by)
    if hits ~= before then   -- texte mis a jour seulement s'il change
        lv.label.set_text(label, "Rebonds : " .. hits)
    end
end

local function action(b)
    if b == KEY_ENTER then
        if ticks - lastEnter < 4 then return end   -- un appui = une fois
        lastEnter = ticks
    end
    if b == KEY_LEFT then
        px = math.max(0, px - STEP)
    elseif b == KEY_RIGHT then
        px = math.min(W - SIZE, px + STEP)
    elseif b == KEY_ENTER then
        colorIdx = colorIdx % #COLORS + 1
    end
    refresh()
end

local function onKey(e)                    -- molette gauche / droite
    local k = lv.event.get_key_value(e)
    if k == nil then return end
    action(string.byte(k))
end

local function onClick(e)                  -- clic molette (ENTER)
    action(KEY_ENTER)
end

-- ---------------------------------------------------------------------------
function game.create(args)
    args = args or {}
    if loopTimer ~= nil then lv.timer.del(loopTimer); loopTimer = nil end
    px, colorIdx, hits = math.floor((W - SIZE) / 2), 1, 0
    bx, by, vx, vy = 40, 40, 3, 2
    ticks, lastEnter = 0, -100

    lv.obj.clean(window)
    lv.group.set_editing(document, true)   -- molette -> EVENT_KEY, clic -> EVENT_CLICKED
    initStyles()

    container = lv.obj.new(window)
    lv.obj.remove_style_all(container)
    lv.obj.set_size(container, W, H)
    lv.obj.add_style(container, game.styles.bg, lv.STATE_DEFAULT)
    -- pas de defilement si un objet deborde (sprite qui entre/sort de l'ecran)
    lv.obj.clear_flag(container, lv.OBJ_FLAG_SCROLLABLE)
    game.events[container] = {}
    game.events[container].key = lv.obj.add_event_cb(container, onKey, lv.EVENT_KEY)
    game.events[container].click = lv.obj.add_event_cb(container, onClick, lv.EVENT_CLICKED)
    lv.group.add_obj(document, container)

    player = lv.obj.new(container)
    lv.obj.remove_style_all(player)
    lv.obj.set_size(player, SIZE, SIZE)
    lv.obj.add_style(player, game.styles.player, lv.STATE_DEFAULT)

    ball = lv.obj.new(container)
    lv.obj.remove_style_all(ball)
    lv.obj.set_size(ball, BALL, BALL)
    lv.obj.add_style(ball, game.styles.ball, lv.STATE_DEFAULT)

    -- Ordre de creation = ordre d'affichage : le dernier cree est dessine au-dessus.
    -- Le HUD (texte) est donc cree APRES les objets du jeu.
    label = lv.label.new(container)
    lv.obj.remove_style_all(label)
    lv.obj.add_style(label, game.styles.text, lv.STATE_DEFAULT)
    lv.obj.align(label, lv.ALIGN_TOP_LEFT, 6, 2)

    refresh()
    loopTimer = lv.timer.new(tick, 25, nil)
end

return game
