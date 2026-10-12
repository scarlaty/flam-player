-- ============================================================================
-- etoiles_1_0_0.lua — "Attrape-étoiles" pour la Flam.
-- Un panier en bas, deplace a la molette ; des etoiles tombent.
-- +1 point par etoile attrapee, 3 etoiles ratees = perdu, ENTER pour rejouer
-- (la molette relance aussi apres 1 s, le device n'ayant pas de bouton central).
-- ============================================================================

local game = {}

local W = Global.visual_width    -- 320
local H = Global.visual_height   -- 212

local KEY_RIGHT, KEY_LEFT, KEY_ENTER = 19, 20, 10
local STEP = 20                  -- px par cran de molette
local BASKET_W, BASKET_H = 48, 20
local BASKET_Y = H - BASKET_H - 4
local STAR = 24
local NSTARS = 4                 -- pool d'etoiles (objets crees une fois)
local MAX_MISS = 3
local TICK_MS = 25
local RESTART_DELAY = 40         -- ticks (1 s) avant que la molette puisse relancer

game.styles = {}
game.events = {}
game.images = {}

local container, basket, hud, overlay, overlayLabel, loopTimer
local stars = {}                 -- { obj, x, y, v, active }
local bx, score, misses, phase, spawnIn, overTicks

-- ---------------------------------------------------------------------------
function game.clean()
    if loopTimer ~= nil then lv.timer.del(loopTimer); loopTimer = nil end
    for obj, ev in pairs(game.events) do
        lv.obj.remove_event_cb(obj, ev.key)
    end
    Global.requestAudioStop(true, true)
    lv.group.set_editing(document, false)
    lv.group.remove_all_objs(document)
    lv.obj.clean(window)
    container, basket, hud, overlay, overlayLabel = nil, nil, nil, nil, nil
    stars = {}
    game.styles = {}
    game.events = {}
    game.images = {}
end

-- ---------------------------------------------------------------------------
local function initStyles()
    local S = game.styles
    S.bg = lv.style.new()
    lv.style.set_bg_color(S.bg, lv.color.hex(0x141846))
    lv.style.set_bg_opa(S.bg, lv.OPA_COVER)
    lv.style.set_pad_all(S.bg, 0)

    S.text = lv.style.new()
    lv.style.set_text_color(S.text, lv.color.hex(0xefedea))
    lv.style.set_text_font(S.text, lv.font.nunito_extrabold_16)

    S.panel = lv.style.new()
    lv.style.set_bg_color(S.panel, lv.color.hex(0x000000))
    lv.style.set_bg_opa(S.panel, 200)
    lv.style.set_radius(S.panel, 10)
    lv.style.set_border_color(S.panel, lv.color.hex(0xfbbd2a))
    lv.style.set_border_width(S.panel, 2)

    S.big = lv.style.new()
    lv.style.set_text_color(S.big, lv.color.hex(0xfbbd2a))
    lv.style.set_text_font(S.big, lv.font.nunito_extrabold_20)
    lv.style.set_text_align(S.big, lv.TEXT_ALIGN_CENTER)
end

local function updateHud()
    local lives = ""
    for i = 1, MAX_MISS do
        lives = lives .. (i <= misses and "x " or "o ")
    end
    lv.label.set_text(hud, "Score : " .. score .. "      Ratées : " .. lives)
end

local function placeBasket()
    lv.obj.set_pos(basket, bx, BASKET_Y)
end

local function spawn(s)
    s.x = math.random(0, W - STAR)
    s.y = -STAR - math.random(0, 30)
    s.v = 2 + math.min(score, 30) / 10 + math.random(0, 10) / 10
    s.active = true
    lv.obj.set_pos(s.obj, s.x, math.floor(s.y))
    lv.obj.clear_flag(s.obj, lv.OBJ_FLAG_HIDDEN)
end

local function retire(s)
    s.active = false
    lv.obj.add_flag(s.obj, lv.OBJ_FLAG_HIDDEN)
end

local function gameOver()
    phase = "over"
    overTicks = 0
    for _, s in ipairs(stars) do retire(s) end
    lv.label.set_text(overlayLabel, "Perdu !\nScore : " .. score .. "\n\nENTER pour rejouer")
    lv.obj.clear_flag(overlay, lv.OBJ_FLAG_HIDDEN)
end

local function startGame()
    score, misses = 0, 0
    bx = math.floor((W - BASKET_W) / 2)
    spawnIn = 20
    for _, s in ipairs(stars) do retire(s) end
    lv.obj.add_flag(overlay, lv.OBJ_FLAG_HIDDEN)
    placeBasket()
    updateHud()
    phase = "play"
end

local function tick()
    if phase == "over" then
        overTicks = overTicks + 1
        return
    end
    -- apparition : intervalle qui raccourcit avec le score
    spawnIn = spawnIn - 1
    if spawnIn <= 0 then
        for _, s in ipairs(stars) do
            if not s.active then spawn(s); break end
        end
        spawnIn = math.max(22, 60 - score * 2)
    end
    for _, s in ipairs(stars) do
        if s.active then
            s.y = s.y + s.v
            local cx = s.x + STAR / 2
            if s.y + STAR >= BASKET_Y + 6 and s.y + STAR <= BASKET_Y + BASKET_H + 6
                    and cx >= bx and cx <= bx + BASKET_W then
                retire(s)
                score = score + 1
                updateHud()
            elseif s.y >= H then
                retire(s)
                misses = misses + 1
                updateHud()
                if misses >= MAX_MISS then gameOver(); return end
            else
                lv.obj.set_pos(s.obj, s.x, math.floor(s.y))
            end
        end
    end
end

local function onKey(e)
    local k = lv.event.get_key_value(e)
    if k == nil then return end
    local b = string.byte(k)
    if phase == "over" then
        if b == KEY_ENTER or ((b == KEY_LEFT or b == KEY_RIGHT) and overTicks >= RESTART_DELAY) then
            startGame()
        end
        return
    end
    if b == KEY_LEFT then
        bx = math.max(0, bx - STEP)
        placeBasket()
    elseif b == KEY_RIGHT then
        bx = math.min(W - BASKET_W, bx + STEP)
        placeBasket()
    end
end

-- ---------------------------------------------------------------------------
function game.create(args)
    args = args or {}
    if loopTimer ~= nil then lv.timer.del(loopTimer); loopTimer = nil end
    lv.obj.clean(window)
    lv.group.set_editing(document, true)
    initStyles()

    game.images.star = Global.load_image("star.lif")
    game.images.basket = Global.load_image("basket.lif")

    container = lv.obj.new(window)
    lv.obj.remove_style_all(container)
    lv.obj.set_size(container, W, H)
    lv.obj.add_style(container, game.styles.bg, lv.STATE_DEFAULT)
    lv.obj.clear_flag(container, lv.OBJ_FLAG_SCROLLABLE)
    game.events[container] = {}
    game.events[container].key = lv.obj.add_event_cb(container, onKey, lv.EVENT_KEY)
    lv.group.add_obj(document, container)

    stars = {}
    for i = 1, NSTARS do
        local o = lv.img.new(container)
        lv.obj.remove_style_all(o)
        lv.img.set_src(o, game.images.star)
        lv.obj.add_flag(o, lv.OBJ_FLAG_HIDDEN)
        stars[i] = { obj = o, x = 0, y = 0, v = 0, active = false }
    end

    basket = lv.img.new(container)
    lv.obj.remove_style_all(basket)
    lv.img.set_src(basket, game.images.basket)

    -- HUD apres les etoiles : dessine par-dessus
    hud = lv.label.new(container)
    lv.obj.remove_style_all(hud)
    lv.obj.add_style(hud, game.styles.text, lv.STATE_DEFAULT)
    lv.obj.align(hud, lv.ALIGN_TOP_LEFT, 6, 2)

    overlay = lv.obj.new(container)
    lv.obj.remove_style_all(overlay)
    lv.obj.set_size(overlay, 220, 120)
    lv.obj.add_style(overlay, game.styles.panel, lv.STATE_DEFAULT)
    lv.obj.align(overlay, lv.ALIGN_CENTER, 0, 0)
    overlayLabel = lv.label.new(overlay)
    lv.obj.remove_style_all(overlayLabel)
    lv.obj.add_style(overlayLabel, game.styles.big, lv.STATE_DEFAULT)
    lv.obj.set_width(overlayLabel, 210)
    lv.label.set_long_mode(overlayLabel, lv.LABEL_LONG_WRAP)
    lv.obj.align(overlayLabel, lv.ALIGN_CENTER, 0, 0)

    startGame()
    loopTimer = lv.timer.new(tick, TICK_MS, nil)
end

return game
