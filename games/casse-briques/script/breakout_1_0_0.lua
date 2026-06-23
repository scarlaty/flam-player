-- ============================================================================
-- breakout_1_0_0.lua — Casse-briques en MODULE (create/clean), charge via
-- Global.load_module. OBLIGATOIRE pour le device : le firmware ne rend/route que
-- le module courant (cf. DEVICE_VS_SIM.md #3). Une branche qui dessine en lv.*
-- marche au sim mais pas sur device. Pattern calque sur image-choice/carousel.
--
-- Adapte du tuto LOVE2D (320x212) : raquette PAS-A-PAS molette, grille briques,
-- balle (rebonds murs/raquette/briques), score, vies, win/lose. ENTER = lancer/rejouer.
-- ============================================================================

local breakout = {}

local W = Global.visual_width    -- 320
local H = Global.visual_height   -- 212

-- Geometrie
local PADDLE_W, PADDLE_H = 52, 8
local PADDLE_Y           = H - 14
local BALL               = 8
local STEP               = 18              -- deplacement raquette par clic
local COLS, ROWS         = 8, 4
local GAP, BRICK_TOP     = 2, 20
local BRICK_W            = math.floor((W - (COLS + 1) * GAP) / COLS)  -- 37
local BRICK_H            = 12
local BRICK_X0           = math.floor((W - (COLS * BRICK_W + (COLS - 1) * GAP)) / 2)
local SPEED              = 3

local ROW_COLORS = { 0xe6332a, 0xf18d05, 0xfbbd2a, 0x4caf50 }

-- --- Verrou d'acces (controle parental) ---------------------------------------
-- Code = suite de directions molette UNIQUEMENT (pas de bouton centre sur le
-- device). 19 = DROITE, 20 = GAUCHE. Validation AUTO par match de prefixe : la
-- bonne touche avance, une mauvaise remet a zero (repli sur la 1re si elle
-- correspond), sequence complete => deverrouille. Modifiable ici.
local LOCK_CODE = { 19, 19, 20, 20, 19, 20 }   -- D D G G D G  (-> -> <- <- -> <-)

-- Objets / etat (reinitialises a chaque require : load_module fait
-- package.loaded[name]=nil au nettoyage -> module recharge a neuf).
breakout.styles = {}
breakout.events = {}
local container, paddle, ball, scoreLbl, msgLbl, loopTimer
local bricks = {}
local exitCb
local paddleX
local bx, by, vx, vy
local lives, score, bricksLeft, running

-- Verrou
local phase            -- "lock" | "play"
local lockIdx          -- longueur du prefixe correct saisi
local lockDots = {}    -- temoins de progression
local lockTitle, lockHint

-- ---------------------------------------------------------------------------
function breakout.clean()
    if loopTimer ~= nil then lv.timer.del(loopTimer); loopTimer = nil end
    for obj, ev in pairs(breakout.events) do
        lv.obj.remove_event_cb(obj, ev.key)
    end
    Global.requestAudioStop(true, true)
    lv.group.set_editing(document, false)
    lv.group.remove_all_objs(document)
    lv.obj.clean(window)
    container, paddle, ball, scoreLbl, msgLbl = nil, nil, nil, nil, nil
    lockTitle, lockHint = nil, nil
    lockDots = {}
    bricks = {}
    breakout.styles = {}
    breakout.events = {}
end

-- ---------------------------------------------------------------------------
local function initStyles()
    local S = breakout.styles
    S.bg = lv.style.new()
    lv.style.set_bg_color(S.bg, lv.color.black())
    lv.style.set_bg_opa(S.bg, lv.OPA_COVER)
    lv.style.set_pad_all(S.bg, 0)

    S.paddle = lv.style.new()
    lv.style.set_bg_color(S.paddle, lv.color.hex(0xefedea))
    lv.style.set_bg_opa(S.paddle, lv.OPA_COVER)
    lv.style.set_radius(S.paddle, 3)

    S.ball = lv.style.new()
    lv.style.set_bg_color(S.ball, lv.color.hex(0xfbbd2a))
    lv.style.set_bg_opa(S.ball, lv.OPA_COVER)
    lv.style.set_radius(S.ball, BALL)

    S.text = lv.style.new()
    lv.style.set_text_color(S.text, lv.color.hex(0xefedea))
    lv.style.set_text_font(S.text, lv.font.nunito_extrabold_12)

    S.msg = lv.style.new()
    lv.style.set_bg_color(S.msg, lv.color.black())
    lv.style.set_bg_opa(S.msg, lv.OPA_COVER)
    lv.style.set_text_color(S.msg, lv.color.hex(0xffffff))
    lv.style.set_text_font(S.msg, lv.font.nunito_extrabold_16)
    lv.style.set_pad_all(S.msg, 6)
    lv.style.set_radius(S.msg, 6)

    S.lockTitle = lv.style.new()
    lv.style.set_text_color(S.lockTitle, lv.color.hex(0xffffff))
    lv.style.set_text_font(S.lockTitle, lv.font.nunito_extrabold_16)

    for r = 1, ROWS do
        S["brick" .. r] = lv.style.new()
        lv.style.set_bg_color(S["brick" .. r], lv.color.hex(ROW_COLORS[r] or 0x888888))
        lv.style.set_bg_opa(S["brick" .. r], lv.OPA_COVER)
        lv.style.set_radius(S["brick" .. r], 2)
    end
end

local function overlap(ax, ay, aw, ah, bx2, by2, bw, bh)
    return ax < bx2 + bw and ax + aw > bx2 and ay < by2 + bh and ay + ah > by2
end

local function resetBall()
    bx = math.floor(paddleX + PADDLE_W / 2 - BALL / 2)
    by = PADDLE_Y - BALL - 2
    vx = (math.floor(paddleX) % 2 == 0) and (SPEED - 1) or -(SPEED - 1)
    vy = -SPEED
end

local function buildBricks()
    bricksLeft = 0
    for r = 1, ROWS do
        for c = 1, COLS do
            local x = BRICK_X0 + (c - 1) * (BRICK_W + GAP)
            local y = BRICK_TOP + (r - 1) * (BRICK_H + GAP)
            local b = lv.obj.new(container)
            lv.obj.remove_style_all(b)
            lv.obj.set_size(b, BRICK_W, BRICK_H)
            lv.obj.add_style(b, breakout.styles["brick" .. r], lv.STATE_DEFAULT)
            lv.obj.set_pos(b, x, y)
            bricks[#bricks + 1] = { obj = b, x = x, y = y, alive = true }
            bricksLeft = bricksLeft + 1
        end
    end
end

local function setMessage(txt)
    if txt == nil then
        lv.obj.add_flag(msgLbl, lv.OBJ_FLAG_HIDDEN)
    else
        lv.label.set_text(msgLbl, txt)
        lv.obj.clear_flag(msgLbl, lv.OBJ_FLAG_HIDDEN)
        lv.obj.align(msgLbl, lv.ALIGN_CENTER, 0, 0)
    end
end

local function refreshScore()
    lv.label.set_text(scoreLbl, "Score " .. score .. "   Vies " .. lives)
end

-- ---------------------------------------------------------------------------
local function tick()
    if not running then return end

    bx = bx + vx
    by = by + vy

    if bx <= 0 then bx = 0; vx = -vx end
    if bx >= W - BALL then bx = W - BALL; vx = -vx end
    if by <= 0 then by = 0; vy = -vy end

    if by >= H - BALL then
        lives = lives - 1
        refreshScore()
        if lives <= 0 then
            running = false
            setMessage("Perdu !  ENTER pour rejouer")
            return
        end
        resetBall()
        lv.obj.set_pos(ball, bx, by)
        return
    end

    if vy > 0 and overlap(bx, by, BALL, BALL, paddleX, PADDLE_Y, PADDLE_W, PADDLE_H) then
        vy = -vy
        by = PADDLE_Y - BALL
        local hit = (bx + BALL / 2) - (paddleX + PADDLE_W / 2)
        vx = math.floor(hit / (PADDLE_W / 2) * SPEED)
        if vx == 0 then vx = (hit < 0) and -1 or 1 end
    end

    for _, b in ipairs(bricks) do
        if b.alive and overlap(bx, by, BALL, BALL, b.x, b.y, BRICK_W, BRICK_H) then
            b.alive = false
            lv.obj.add_flag(b.obj, lv.OBJ_FLAG_HIDDEN)
            bricksLeft = bricksLeft - 1
            score = score + 10
            refreshScore()
            vy = -vy
            if bricksLeft <= 0 then
                running = false
                setMessage("Gagne !  ENTER pour rejouer")
            end
            break
        end
    end

    lv.obj.set_pos(ball, bx, by)
end

-- ---------------------------------------------------------------------------
local function newGame()
    paddleX = math.floor((W - PADDLE_W) / 2)
    lives, score = 3, 0
    for _, b in ipairs(bricks) do lv.obj.del(b.obj) end
    bricks = {}
    buildBricks()
    lv.obj.set_pos(paddle, paddleX, PADDLE_Y)
    resetBall()
    lv.obj.set_pos(ball, bx, by)
    refreshScore()
    setMessage(nil)
    running = true
end

-- Forward declarations (references croisees lock <-> jeu).
local buildGame, unlock, relock

-- Saisie pendant le JEU (b = octet de la touche).
local function onKeyGame(b)
    if b == 20 then
        paddleX = paddleX - STEP
        if paddleX < 0 then paddleX = 0 end
        lv.obj.set_pos(paddle, paddleX, PADDLE_Y)
        if not running and lives > 0 and bricksLeft > 0 then
            bx = math.floor(paddleX + PADDLE_W / 2 - BALL / 2)
            lv.obj.set_pos(ball, bx, by)
        end
    elseif b == 19 then
        paddleX = paddleX + STEP
        if paddleX > W - PADDLE_W then paddleX = W - PADDLE_W end
        lv.obj.set_pos(paddle, paddleX, PADDLE_Y)
        if not running and lives > 0 and bricksLeft > 0 then
            bx = math.floor(paddleX + PADDLE_W / 2 - BALL / 2)
            lv.obj.set_pos(ball, bx, by)
        end
    elseif b == 10 then
        if not running then
            if lives <= 0 or bricksLeft <= 0 then
                relock()          -- fin de partie : re-saisir le code pour rejouer
            else
                running = true
                setMessage(nil)
            end
        end
    end
end

-- Temoins de progression du code (remplis = prefixe correct saisi).
local function updateLockDots()
    for i, d in ipairs(lockDots) do
        local on = (i <= lockIdx)
        lv.obj.set_style_bg_color(d, lv.color.hex(on and 0xfbbd2a or 0x555049), lv.STATE_DEFAULT)
    end
end

-- Saisie pendant le VERROU : directions seulement, match de prefixe.
local function onKeyLock(b)
    if b ~= 19 and b ~= 20 then return end
    if b == LOCK_CODE[lockIdx + 1] then
        lockIdx = lockIdx + 1
    elseif b == LOCK_CODE[1] then
        lockIdx = 1               -- repli : la touche amorce un nouveau code
    else
        lockIdx = 0
    end
    updateLockDots()
    if lockIdx >= #LOCK_CODE then unlock() end
end

-- Dispatcher d'entrees selon la phase.
local function onKey(e)
    local k = lv.event.get_key_value(e)
    if k == nil then return end
    local b = string.byte(k)
    if phase == "lock" then onKeyLock(b) else onKeyGame(b) end
end

-- Ecran verrou (code molette uniquement).
local function buildLock()
    lockIdx = 0
    lockDots = {}

    lockTitle = lv.label.new(container)
    lv.obj.remove_style_all(lockTitle)
    lv.obj.add_style(lockTitle, breakout.styles.lockTitle, lv.STATE_DEFAULT)
    lv.label.set_text(lockTitle, "Acces parent")
    lv.obj.align(lockTitle, lv.ALIGN_CENTER, 0, -34)

    lockHint = lv.label.new(container)
    lv.obj.remove_style_all(lockHint)
    lv.obj.add_style(lockHint, breakout.styles.text, lv.STATE_DEFAULT)
    lv.label.set_text(lockHint, "Entre le code (molette)")
    lv.obj.align(lockHint, lv.ALIGN_CENTER, 0, -12)

    local n = #LOCK_CODE
    local D, GAPD = 14, 8
    local total = n * D + (n - 1) * GAPD
    local x0 = math.floor((W - total) / 2)
    local y = math.floor(H / 2 + 12)
    for i = 1, n do
        local d = lv.obj.new(container)
        lv.obj.remove_style_all(d)
        lv.obj.set_size(d, D, D)
        lv.obj.set_style_bg_opa(d, lv.OPA_COVER, lv.STATE_DEFAULT)
        lv.obj.set_style_radius(d, 7, lv.STATE_DEFAULT)
        lv.obj.set_pos(d, x0 + (i - 1) * (D + GAPD), y)
        lockDots[i] = d
    end
    updateLockDots()
end

-- Construit le JEU (apres deverrouillage).
buildGame = function()
    paddle = lv.obj.new(container)
    lv.obj.remove_style_all(paddle)
    lv.obj.set_size(paddle, PADDLE_W, PADDLE_H)
    lv.obj.add_style(paddle, breakout.styles.paddle, lv.STATE_DEFAULT)

    ball = lv.obj.new(container)
    lv.obj.remove_style_all(ball)
    lv.obj.set_size(ball, BALL, BALL)
    lv.obj.add_style(ball, breakout.styles.ball, lv.STATE_DEFAULT)

    scoreLbl = lv.label.new(container)
    lv.obj.remove_style_all(scoreLbl)
    lv.obj.add_style(scoreLbl, breakout.styles.text, lv.STATE_DEFAULT)
    lv.obj.align(scoreLbl, lv.ALIGN_TOP_LEFT, 4, 2)

    msgLbl = lv.label.new(container)
    lv.obj.remove_style_all(msgLbl)
    lv.obj.add_style(msgLbl, breakout.styles.msg, lv.STATE_DEFAULT)
    lv.label.set_text(msgLbl, "")
    lv.obj.add_flag(msgLbl, lv.OBJ_FLAG_HIDDEN)

    newGame()
    running = false
    setMessage("ENTER pour lancer")
    loopTimer = lv.timer.new(tick, 25, nil)
end

-- Deverrouille : retire l'ecran verrou et lance le jeu.
unlock = function()
    if lockTitle then lv.obj.del(lockTitle); lockTitle = nil end
    if lockHint then lv.obj.del(lockHint); lockHint = nil end
    for _, d in ipairs(lockDots) do lv.obj.del(d) end
    lockDots = {}
    phase = "play"
    buildGame()
end

-- Re-verrouille apres une partie : retire le jeu et redemande le code.
relock = function()
    if loopTimer then lv.timer.del(loopTimer); loopTimer = nil end
    for _, b in ipairs(bricks) do lv.obj.del(b.obj) end
    bricks = {}
    if paddle then lv.obj.del(paddle); paddle = nil end
    if ball then lv.obj.del(ball); ball = nil end
    if scoreLbl then lv.obj.del(scoreLbl); scoreLbl = nil end
    if msgLbl then lv.obj.del(msgLbl); msgLbl = nil end
    phase = "lock"
    buildLock()
end

-- ---------------------------------------------------------------------------
function breakout.create(args)
    args = args or {}
    exitCb = args.exitCb

    lv.obj.clean(window)
    lv.group.set_editing(document, true)
    initStyles()

    container = lv.obj.new(window)
    lv.obj.remove_style_all(container)
    lv.obj.set_size(container, W, H)
    lv.obj.add_style(container, breakout.styles.bg, lv.STATE_DEFAULT)
    breakout.events[container] = {}
    breakout.events[container].key = lv.obj.add_event_cb(container, onKey, lv.EVENT_KEY)
    lv.group.add_obj(document, container)

    -- Demarre VERROUILLE : il faut le code molette pour acceder au jeu.
    phase = "lock"
    buildLock()
end

return breakout
