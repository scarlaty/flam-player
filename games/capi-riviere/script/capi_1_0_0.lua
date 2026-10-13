-- ============================================================================
-- capi_1_0_0.lua — « Capi et la rivière » pour la Flam.
-- Course-esquive vue de dessus : Capi le capybara descend la rivière sur sa
-- bouée. Molette = gauche/droite, ENTER = saut (aussi : 3 crans rapides).
-- Rochers, troncs, nénuphars : plouf, une bouée en moins (3). Mandarines :
-- points, combo si on les enchaîne. Fin douce « Bravo ! », on peut rejouer.
--
-- Un seul module, trois phases : "title" -> "play" -> "end".
-- Retour : depuis le jeu ou la fin -> écran titre ; depuis le titre ->
-- bibliothèque (on ne sort jamais vers la bibliothèque en pleine partie).
--
-- Audio : le device ne joue qu'une piste à la fois, et Global.requestAudioPlay
-- attend 500 ms. Le module pilote donc audio.* lui-même (mêmes appels que
-- global.lua) depuis la boucle de 25 ms. Constaté sur la Flam : arrêter une
-- piste en cours (stop ou pause) fige l'image un instant, et charger sans
-- arrêter gèle l'appareil. Donc PENDANT LA PARTIE, pas de musique et jamais
-- d'arrêt : un bruitage attend la fin naturelle du précédent (ou est oublié
-- s'il attend trop, sauf le plouf). Musique seulement au titre et à la fin,
-- où un arrêt ne se voit pas. Un jeton (sndId) ignore les "stop" des pistes
-- coupées volontairement.
-- ============================================================================

local game = {}

local W = Global.visual_width    -- 320
local H = Global.visual_height   -- 212

local KEY_RIGHT, KEY_LEFT, KEY_ENTER = 19, 20, 10
local TICK_MS = 25

local RIVER_L, RIVER_R = 46, 274       -- eau navigable (berges du décor)
local CAPI = 44                        -- sprite capi.png 44x44
local CAPI_Y = H - CAPI - 6
local STEP = 24                        -- px par cran de molette
local JUMP_T = 30                      -- durée du saut (ticks)
local JUMP_COOL = 16                   -- délai avant un nouveau saut
local BURST_N, BURST_T = 3, 10         -- 3 crans en 10 ticks (0,25 s) = saut
local INVUL_T = 80                     -- invulnérable 2 s après un plouf
local LIVES = 3
local COMBO_T = 160                    -- 4 s pour enchaîner une mandarine
local SPEED0, SPEED_MAX, ACCEL = 1.6, 4.6, 0.0012
local LEVEL2, LEVEL3 = 2.6, 3.6
local START_DELAY = 20                 -- ticks avant qu'un cran lance la partie
local RESTART_DELAY = 60               -- idem pour rejouer

local NOBS, NMAND, NRIP, NPLANT = 6, 4, 5, 4
local OBS_TYPES = {
    { img = "rock.lif", w = 36, h = 30, ix = 6, iy = 6 },
    { img = "log.lif",  w = 72, h = 24, ix = 6, iy = 5 },
    { img = "lily.lif", w = 36, h = 34, ix = 7, iy = 7 },
}
local MAND = 22

game.styles = {}
game.events = {}
game.images = {}

-- objets
local container, capiImg, shadowImg, splashImg, popLabel
local scorePanel, scoreLabel, msgLabel
local lifeImgs = {}
local endPanel, endLabel, endHint
local titleLayer, titleHint
local obstacles, mandarines, ripples, plants = {}, {}, {}, {}
local loopTimer

-- état
local phase, ticks, phaseTicks
local capiX, targetX, lives, score, gobbed, combo, lastGob
local speed, level, obsIn, mandIn, trailX, trailLeft
local jumpT, jumpCool, invulT, gobT, splashT, popT, msgT, endT
local burst = {}
local alive = false

-- audio
local MUSIC = "@music"                 -- demande « segment de musique courant »
local NCHUNK = 4                       -- musiques en 4 segments <base>_1..4.mp3
local sndId, sndPending, sndStart, stopping, stopTick = 0, nil, 0, false, 0
local sndPrio, sndAsked = 0, 0         -- priorité et tick de la demande en attente
local SFX_WAIT = 16                    -- un bruitage attend au plus 0,4 s
local curIsMusic = false
local musicBase, musicChunk, musicFails = nil, 1, 0

-- ---------------------------------------------------------------------------
-- Audio : une piste à la fois, demandes traitées dans la boucle.
-- La musique (titre, fin) est découpée en segments de 2 mesures enchaînés
-- (pas d'audio.seek). En partie, rien n'est jamais coupé (voir l'en-tête).
-- ---------------------------------------------------------------------------
local function setMusic(base)
    musicBase, musicChunk, musicFails = base, 1, 0
    curIsMusic = false                 -- la piste en cours n'avance pas ce morceau
end

local function playMusic(base)
    setMusic(base)
    sndPending = MUSIC
end

-- prio : 1 glop/saut, 2 combo/palier, 3 plouf (jamais oublié), 9 fanfare
local function playSfx(path, prio)
    prio = prio or 1
    if sndPending ~= nil and sndPending ~= MUSIC and sndPrio > prio then return end
    sndPending, sndPrio, sndAsked = path, prio, ticks
end

local function stopAudio()
    sndPending, musicBase, curIsMusic, stopping = nil, nil, false, false
    if audio.get_status() ~= "stop" then
        sndId = sndId + 1              -- le "stop" qui suit est ignoré
        stopping, stopTick = true, ticks
        audio.stop()
    end
end

local function nextChunk()
    musicChunk = musicChunk % NCHUNK + 1
end

local function onAudio(id, isMusic, state)
    if not alive or id ~= sndId then return end
    if state == "play" then
        if isMusic then musicFails = 0 end
        return
    end
    if state ~= "stop" then return end
    curIsMusic = false
    if isMusic then
        if ticks - sndStart < 8 then
            musicFails = musicFails + 1    -- arrêt immédiat = échec (piège 8)
            if musicFails >= 3 then musicBase = nil end
        end
        nextChunk()
    end
    if musicBase ~= nil and sndPending == nil then sndPending = MUSIC end
end

local function audioPump()
    if sndPending == nil then return end
    local st = audio.get_status()
    local waitSfx = phase == "play" or (phase == "end" and not curIsMusic)
    if (st == "play" or st == "pause") and waitSfx and not stopping then
        -- en partie (et sur « Bravo ! » tant qu'un bruitage finit) : on
        -- n'interrompt jamais un bruitage (à-coup / gel sur la Flam)
        local waited = ticks - sndAsked
        if (sndPrio < 3 and waited > SFX_WAIT) or (sndPrio == 3 and waited > 40) then
            sndPending = nil           -- trop tard, on l'oublie
        end
        return
    end
    if st == "play" or st == "pause" then
        if not stopping or ticks - stopTick > 20 then   -- réessai si l'arrêt traîne
            stopping = true
            stopTick = ticks
            sndId = sndId + 1          -- le "stop" qui suit est ignoré
            if curIsMusic then nextChunk(); curIsMusic = false end
            audio.stop()
        end
        return                         -- charger au tick suivant
    elseif st ~= "stop" then
        return
    end
    -- garde : jamais deux chargements rapprochés (si le statut tardait à
    -- passer à "play", un load par-dessus une lecture gèlerait la Flam)
    if ticks - sndStart < 2 then return end
    stopping = false
    local path, isMusic = sndPending, sndPending == MUSIC
    sndPending = nil
    if isMusic then
        if musicBase == nil then return end
        path = musicBase .. "_" .. musicChunk .. ".mp3"
    end
    sndId = sndId + 1
    local id = sndId
    if audio.load(0, path, function(state, s) onAudio(id, isMusic, state) end) == 0 then
        audio.play()
        sndStart = ticks
        curIsMusic = isMusic
    elseif isMusic then
        musicBase = nil                -- fichier illisible : on n'insiste pas
    end
end

-- ---------------------------------------------------------------------------
function game.clean()
    alive = false
    if loopTimer ~= nil then lv.timer.del(loopTimer); loopTimer = nil end
    for obj, ev in pairs(game.events) do
        for _, cb in pairs(ev) do lv.obj.remove_event_cb(obj, cb) end
    end
    sndPending, musicBase, curIsMusic, stopping = nil, nil, false, false
    sndId = sndId + 1
    Global.requestAudioStop(true, true)
    lv.group.set_editing(document, false)
    lv.group.remove_all_objs(document)
    lv.obj.clean(window)
    container, capiImg, shadowImg, splashImg, popLabel = nil, nil, nil, nil, nil
    scorePanel, scoreLabel, msgLabel = nil, nil, nil
    endPanel, endLabel, endHint, titleLayer, titleHint = nil, nil, nil, nil, nil
    lifeImgs, obstacles, mandarines, ripples, plants = {}, {}, {}, {}, {}
    game.styles = {}
    game.events = {}
    game.images = {}
end

-- ---------------------------------------------------------------------------
local function initStyles()
    local S = game.styles
    S.panel = lv.style.new()
    lv.style.set_bg_color(S.panel, lv.color.hex(0x1d4f6b))
    lv.style.set_bg_opa(S.panel, 170)
    lv.style.set_radius(S.panel, 10)
    lv.style.set_pad_all(S.panel, 0)

    S.hud = lv.style.new()
    lv.style.set_text_color(S.hud, lv.color.hex(0xffffff))
    lv.style.set_text_font(S.hud, lv.font.nunito_extrabold_16)

    S.pop = lv.style.new()
    lv.style.set_bg_color(S.pop, lv.color.hex(0x1d4f6b))
    lv.style.set_bg_opa(S.pop, 170)
    lv.style.set_radius(S.pop, 8)
    lv.style.set_pad_hor(S.pop, 6)
    lv.style.set_pad_ver(S.pop, 1)
    lv.style.set_text_color(S.pop, lv.color.hex(0xfff2a8))
    lv.style.set_text_font(S.pop, lv.font.nunito_extrabold_16)
    lv.style.set_text_align(S.pop, lv.TEXT_ALIGN_CENTER)

    S.msg = lv.style.new()
    lv.style.set_text_color(S.msg, lv.color.hex(0xffffff))
    lv.style.set_text_font(S.msg, lv.font.nunito_extrabold_20)
    lv.style.set_text_align(S.msg, lv.TEXT_ALIGN_CENTER)
    lv.style.set_bg_color(S.msg, lv.color.hex(0xff8a3d))
    lv.style.set_bg_opa(S.msg, 220)
    lv.style.set_radius(S.msg, 12)
    lv.style.set_pad_hor(S.msg, 12)
    lv.style.set_pad_ver(S.msg, 4)

    S.card = lv.style.new()
    lv.style.set_bg_color(S.card, lv.color.hex(0xfff7e6))
    lv.style.set_bg_opa(S.card, 240)
    lv.style.set_radius(S.card, 16)
    lv.style.set_border_color(S.card, lv.color.hex(0xff9a2e))
    lv.style.set_border_width(S.card, 3)

    S.big = lv.style.new()
    lv.style.set_text_color(S.big, lv.color.hex(0xe0601a))
    lv.style.set_text_font(S.big, lv.font.nunito_extrabold_20)
    lv.style.set_text_align(S.big, lv.TEXT_ALIGN_CENTER)

    S.body = lv.style.new()
    lv.style.set_text_color(S.body, lv.color.hex(0x3d2a1e))
    lv.style.set_text_font(S.body, lv.font.nunito_bold_16)
    lv.style.set_text_align(S.body, lv.TEXT_ALIGN_CENTER)

    S.hint = lv.style.new()
    lv.style.set_text_color(S.hint, lv.color.hex(0x1d4f6b))
    lv.style.set_text_font(S.hint, lv.font.nunito_extrabold_14)
    lv.style.set_text_align(S.hint, lv.TEXT_ALIGN_CENTER)
end

local function newImg(parent, data)
    local o = lv.img.new(parent)
    lv.obj.remove_style_all(o)
    lv.img.set_src(o, data)
    return o
end

local function newLabel(parent, style, text)
    local l = lv.label.new(parent)
    lv.obj.remove_style_all(l)
    lv.obj.add_style(l, style, lv.STATE_DEFAULT)
    lv.label.set_text(l, text)
    return l
end

local function hide(o) lv.obj.add_flag(o, lv.OBJ_FLAG_HIDDEN) end
local function show(o) lv.obj.clear_flag(o, lv.OBJ_FLAG_HIDDEN) end

-- ---------------------------------------------------------------------------
local function updateScore()
    lv.label.set_text(scoreLabel, "Score : " .. score)
end

local function updateLives()
    for i = 1, LIVES do
        lv.obj.set_style_img_opa(lifeImgs[i], i <= lives and lv.OPA_COVER or 70, lv.STATE_DEFAULT)
    end
end

local function showMsg(text, t)
    lv.label.set_text(msgLabel, text)
    lv.obj.align(msgLabel, lv.ALIGN_CENTER, 0, -30)
    show(msgLabel)
    msgT = t
end

local function showPop(text, x, y)
    lv.label.set_text(popLabel, text)
    lv.obj.set_pos(popLabel, x, y)
    show(popLabel)
    popT = 30
end

local function capiSrc(name)
    lv.img.set_src(capiImg, game.images[name])
end

local function overlaps(ax, ay, aw, ah, bx, by, bw, bh)
    return ax < bx + bw and bx < ax + aw and ay < by + bh and by < ay + ah
end

-- ---------------------------------------------------------------------------
-- Apparitions
-- ---------------------------------------------------------------------------
local function nearMandarine(x, w)
    -- une mandarine qui vient d'apparaître ne doit pas cacher un obstacle
    for _, m in ipairs(mandarines) do
        if m.active and m.y < 40 and overlaps(x - 8, -50, w + 16, 90, m.x, m.y, MAND, MAND) then
            return true
        end
    end
    return false
end

-- renvoie false si l'apparition est à retenter au tick suivant
local function spawnObstacle()
    for _, o in ipairs(obstacles) do
        if not o.active then
            local t = OBS_TYPES[math.random(1, #OBS_TYPES)]
            local x = math.random(RIVER_L, RIVER_R - t.w)
            local tries = 1
            while nearMandarine(x, t.w) do
                if tries >= 3 then return false end
                x = math.random(RIVER_L, RIVER_R - t.w)
                tries = tries + 1
            end
            o.t = t
            lv.img.set_src(o.obj, game.images[t.img])
            o.x = x
            o.y = -t.h
            o.active = true
            lv.obj.set_pos(o.obj, o.x, o.y)
            show(o.obj)
            return true
        end
    end
    return true                        -- pool plein : on attend le prochain tour
end

local function freeSpot(x)
    -- pas de mandarine posée sur un obstacle qui vient d'apparaître
    for _, o in ipairs(obstacles) do
        if o.active and o.y < 30 and overlaps(x - 6, -MAND - 6, MAND + 12, MAND + 40,
                                               o.x, o.y, o.t.w, o.t.h) then
            return false
        end
    end
    return true
end

local function spawnMandarine()
    if trailLeft <= 0 then
        trailX = math.random(RIVER_L + 4, RIVER_R - MAND - 4)
        trailLeft = math.random(1, 3)
    end
    if not freeSpot(trailX) then trailLeft = 0; return end
    for _, m in ipairs(mandarines) do
        if not m.active then
            m.x, m.y, m.active = trailX, -MAND, true
            lv.obj.set_pos(m.obj, m.x, m.y)
            show(m.obj)
            trailLeft = trailLeft - 1
            return
        end
    end
end

-- ---------------------------------------------------------------------------
-- Phases
-- ---------------------------------------------------------------------------
local function setPhase(p)
    phase = p
    phaseTicks = 0
end

local function backToTitle()
    Global.load_module("capi", "1_0_0").create({})
end

local function showTitle()
    setPhase("title")
    hide(endPanel)
    back_callback = function()
        sndPending, musicBase, curIsMusic = nil, nil, false
        sndId = sndId + 1              -- le "stop" qui suit est ignoré
        Global.requestAudioStop(true, true)
        goto_library()
    end
    playMusic("title")
end

local function startGame()
    setPhase("play")
    if titleLayer ~= nil then
        lv.obj.del(titleLayer)         -- l'image titre n'est plus utile
        titleLayer, titleHint = nil, nil
        game.images["title.lif"] = nil
    end
    hide(endPanel)
    lives, score, gobbed, combo, lastGob = LIVES, 0, 0, 0, -1000
    speed, level = SPEED0, 1
    obsIn, mandIn, trailLeft, trailX = 60, 30, 0, W / 2
    jumpT, jumpCool, invulT, gobT, splashT, popT, msgT = 0, 0, 0, 0, 0, 0, 0
    capiX = math.floor((W - CAPI) / 2)
    targetX = capiX
    for _, o in ipairs(obstacles) do o.active = false; hide(o.obj) end
    for _, m in ipairs(mandarines) do m.active = false; hide(m.obj) end
    hide(splashImg); hide(popLabel); hide(shadowImg)
    capiSrc("capi.lif")
    lv.obj.set_pos(capiImg, capiX, CAPI_Y)
    show(capiImg)
    updateScore()
    updateLives()
    showMsg("C'est parti !", 50)
    Global.setBackBehavior(backToTitle)
    stopAudio()
end

local function finishGame()
    setPhase("end")
    endT = 0
    local best = 0
    if type(Global.capiBest) == "number" then best = Global.capiBest end
    local txt = "Score : " .. score .. "\nMandarines : " .. gobbed
    if score > best then
        Global.capiBest = score
        if score > 0 then txt = txt .. "\nNouveau record !" end
    else
        txt = txt .. "\nRecord : " .. best
    end
    lv.label.set_text(endLabel, txt)
    jumpT, gobT = 0, 0
    hide(shadowImg)
    capiSrc("capi.lif")
    lv.obj.set_pos(capiImg, math.floor(capiX), CAPI_Y)
    show(capiImg)
    for _, o in ipairs(obstacles) do o.active = false; hide(o.obj) end
    for _, m in ipairs(mandarines) do m.active = false; hide(m.obj) end
    hide(endHint)
    hide(msgLabel)
    hide(popLabel)
    show(endPanel)
    setMusic("title")                  -- après la fanfare, musique douce
    playSfx("fanfare.mp3", 9)
end

-- ---------------------------------------------------------------------------
-- Actions
-- ---------------------------------------------------------------------------
local function jump()
    if jumpT > 0 or jumpCool > 0 then return end
    jumpT = JUMP_T
    capiSrc("capi_jump.lif")
    show(shadowImg)
    playSfx("hop" .. level .. ".mp3")
end

local function plouf()
    lives = lives - 1
    combo = 0
    invulT = INVUL_T
    splashT = 24
    lv.obj.set_pos(splashImg, math.floor(capiX) - 6, CAPI_Y + 2)
    show(splashImg)
    showPop("Plouf !", math.floor(capiX), CAPI_Y - 24)
    updateLives()
    playSfx("splash.mp3", 3)
end

local function gob(m)
    m.active = false
    hide(m.obj)
    if ticks - lastGob <= COMBO_T then combo = combo + 1 else combo = 1 end
    lastGob = ticks
    gobbed = gobbed + 1
    local pts = 10 * math.min(combo, 5)
    score = score + pts
    updateScore()
    gobT = 8
    if jumpT == 0 then capiSrc("capi_gob.lif") end
    if combo >= 2 then
        showPop("Combo x" .. combo .. " !", math.floor(capiX) - 20, CAPI_Y - 24)
    else
        showPop("+" .. pts, math.floor(capiX) + 8, CAPI_Y - 24)
    end
    if combo >= 3 then playSfx("combo.mp3", 2) else playSfx("glop" .. level .. ".mp3") end
end

-- ---------------------------------------------------------------------------
-- Boucle
-- ---------------------------------------------------------------------------
local function scrollDecor(v)
    for _, r in ipairs(ripples) do
        r.y = r.y + v
        if r.y > H then
            r.y = r.y - H - 20
            r.x = math.random(RIVER_L, RIVER_R - 24)
        end
        lv.obj.set_pos(r.obj, r.x, math.floor(r.y))
    end
    for _, p in ipairs(plants) do
        p.y = p.y + v
        if p.y > H then p.y = p.y - H - 40 end
        lv.obj.set_pos(p.obj, p.x, math.floor(p.y))
    end
end

local function tickPlay()
    -- vitesse et paliers (musique et bruitages plus vifs)
    if speed < SPEED_MAX then speed = math.min(SPEED_MAX, speed + ACCEL) end
    local lvl = speed >= LEVEL3 and 3 or (speed >= LEVEL2 and 2 or 1)
    if lvl ~= level then
        level = lvl
        showMsg("Plus vite !", 60)
        playSfx("combo.mp3", 2)
    end

    scrollDecor(speed)

    -- Capi glisse vers sa cible (molette)
    local dx = targetX - capiX
    if math.abs(dx) < 0.6 then capiX = targetX else capiX = capiX + dx * 0.35 end
    local cx = math.floor(capiX)

    -- saut
    local lift = 0
    if jumpT > 0 then
        jumpT = jumpT - 1
        lift = math.floor(14 * math.sin(math.pi * (JUMP_T - jumpT) / JUMP_T))
        if jumpT == 0 then
            jumpCool = JUMP_COOL
            gobT = 0
            hide(shadowImg)
            capiSrc("capi.lif")
        end
    elseif jumpCool > 0 then
        jumpCool = jumpCool - 1
    end

    if invulT > 0 then invulT = invulT - 1 end
    if jumpT > 0 then
        lv.obj.set_pos(shadowImg, cx + 2, CAPI_Y + 32)
        lv.obj.set_pos(capiImg, cx - 6, CAPI_Y - 6 - lift)
        show(capiImg)
    else
        if gobT > 0 then
            gobT = gobT - 1
            if gobT == 0 then capiSrc("capi.lif") end
        end
        lv.obj.set_pos(capiImg, cx, CAPI_Y)
        -- clignote pendant l'invulnérabilité
        if invulT > 0 and (invulT // 4) % 2 == 1 then hide(capiImg) else show(capiImg) end
    end

    -- hitbox indulgente
    local hx, hy, hw, hh = cx + 9, CAPI_Y + 8, CAPI - 18, CAPI - 14

    -- obstacles
    obsIn = obsIn - speed
    if obsIn <= 0 and spawnObstacle() then
        obsIn = math.random(95, 140) - level * 12
    end
    for _, o in ipairs(obstacles) do
        if o.active then
            o.y = o.y + speed
            if o.y >= H then
                o.active = false
                hide(o.obj)
            else
                lv.obj.set_pos(o.obj, o.x, math.floor(o.y))
                local t = o.t
                if jumpT == 0 and invulT == 0 and lives > 0
                        and overlaps(hx, hy, hw, hh, o.x + t.ix, o.y + t.iy,
                                     t.w - 2 * t.ix, t.h - 2 * t.iy) then
                    plouf()
                end
            end
        end
    end

    -- mandarines
    mandIn = mandIn - speed
    if mandIn <= 0 then
        spawnMandarine()
        mandIn = trailLeft > 0 and 34 or math.random(70, 130)
    end
    for _, m in ipairs(mandarines) do
        if m.active then
            m.y = m.y + speed
            if m.y >= H then
                m.active = false
                hide(m.obj)
            elseif overlaps(cx + 4, CAPI_Y - lift, CAPI - 8, CAPI, m.x, m.y, MAND, MAND) then
                gob(m)
            else
                lv.obj.set_pos(m.obj, m.x, math.floor(m.y))
            end
        end
    end

    -- effets
    if splashT > 0 then
        splashT = splashT - 1
        if splashT == 0 then hide(splashImg) end
    end
    if popT > 0 then
        popT = popT - 1
        lv.obj.set_y(popLabel, CAPI_Y - 24 - (30 - popT))
        if popT == 0 then hide(popLabel) end
    end
    if msgT > 0 then
        msgT = msgT - 1
        if msgT == 0 then hide(msgLabel) end
    end

    -- plus de bouée : on laisse finir l'éclaboussure puis « Bravo ! »
    if lives <= 0 and splashT == 0 then finishGame() end
end

local function tick()
    ticks = ticks + 1
    phaseTicks = phaseTicks + 1
    if phase == "play" then
        tickPlay()
    elseif phase == "title" then
        scrollDecor(0)
    elseif phase == "end" then
        scrollDecor(1)
        endT = endT + 1
        if endT == RESTART_DELAY then show(endHint) end
    end
    audioPump()
end

-- ---------------------------------------------------------------------------
local lastEnter = -100

local function action(b)
    if b == KEY_ENTER then
        -- un même appui ne compte qu'une fois (EVENT_KEY et EVENT_CLICKED)
        if ticks - lastEnter < 4 then return end
        lastEnter = ticks
    end
    if phase == "title" then
        if (b == KEY_ENTER or b == KEY_LEFT or b == KEY_RIGHT) and phaseTicks >= START_DELAY then
            startGame()
        end
        return
    elseif phase == "end" then
        if (b == KEY_ENTER or b == KEY_LEFT or b == KEY_RIGHT) and endT >= RESTART_DELAY then
            startGame()
        end
        return
    elseif phase ~= "play" then
        return
    end

    if b == KEY_ENTER then
        jump()
        return
    end
    if b == KEY_LEFT then
        targetX = math.max(RIVER_L - 6, targetX - STEP)
    elseif b == KEY_RIGHT then
        targetX = math.min(RIVER_R - CAPI + 6, targetX + STEP)
    else
        return
    end
    -- un grand coup de molette (3 crans très rapides) fait aussi sauter
    table.insert(burst, ticks)
    if #burst > BURST_N then table.remove(burst, 1) end
    if #burst == BURST_N and burst[BURST_N] - burst[1] <= BURST_T then
        burst = {}
        jump()
    end
end

local function onKey(e)
    local k = lv.event.get_key_value(e)
    if k == nil then return end
    action(string.byte(k))
end

-- La molette est un encodeur LVGL : l'appui (ENTER) n'arrive pas en
-- EVENT_KEY mais en PRESSED/CLICKED sur l'objet focalisé.
local function onClick(e)
    action(KEY_ENTER)
end

-- ---------------------------------------------------------------------------
function game.create(args)
    args = args or {}
    if loopTimer ~= nil then lv.timer.del(loopTimer); loopTimer = nil end
    lv.obj.clean(window)
    lv.group.set_editing(document, true)
    game.events = {}
    initStyles()
    alive = true
    ticks, phaseTicks, endT = 0, 0, 0
    lastEnter = -100
    burst = {}

    local I = game.images
    for _, name in ipairs({ "bg.lif", "title.lif", "capi.lif", "capi_gob.lif",
                            "capi_jump.lif", "shadow.lif", "rock.lif", "log.lif",
                            "lily.lif", "mandarine.lif", "splash.lif", "ripple.lif",
                            "fern.lif", "flower.lif", "life.lif" }) do
        I[name] = Global.load_image(name)
    end

    container = lv.obj.new(window)
    lv.obj.remove_style_all(container)
    lv.obj.set_size(container, W, H)
    lv.obj.clear_flag(container, lv.OBJ_FLAG_SCROLLABLE)
    game.events[container] = {}
    game.events[container].key = lv.obj.add_event_cb(container, onKey, lv.EVENT_KEY)
    game.events[container].click = lv.obj.add_event_cb(container, onClick, lv.EVENT_CLICKED)
    lv.group.add_obj(document, container)

    -- décor (dessous) -> sprites -> HUD -> panneaux (dessus)
    newImg(container, I["bg.lif"])
    plants = {}
    for i = 1, NPLANT do
        local left = (i % 2 == 1)
        local p = { x = left and math.random(0, 10) or math.random(W - 40, W - 30),
                    y = (i - 1) * (H + 40) / NPLANT - 40 }
        p.obj = newImg(container, I[(i % 3 == 0) and "flower.lif" or "fern.lif"])
        lv.obj.set_pos(p.obj, p.x, math.floor(p.y))
        plants[i] = p
    end
    ripples = {}
    for i = 1, NRIP do
        local r = { x = math.random(RIVER_L, RIVER_R - 24), y = (i - 1) * (H + 20) / NRIP - 20 }
        r.obj = newImg(container, I["ripple.lif"])
        lv.obj.set_pos(r.obj, r.x, math.floor(r.y))
        ripples[i] = r
    end

    obstacles = {}
    for i = 1, NOBS do
        local o = newImg(container, I["rock.lif"])
        hide(o)
        obstacles[i] = { obj = o, t = OBS_TYPES[1], x = 0, y = 0, active = false }
    end
    mandarines = {}
    for i = 1, NMAND do
        local o = newImg(container, I["mandarine.lif"])
        hide(o)
        mandarines[i] = { obj = o, x = 0, y = 0, active = false }
    end

    shadowImg = newImg(container, I["shadow.lif"]); hide(shadowImg)
    splashImg = newImg(container, I["splash.lif"]); hide(splashImg)
    capiImg = newImg(container, I["capi.lif"])
    lv.obj.set_pos(capiImg, math.floor((W - CAPI) / 2), CAPI_Y)

    popLabel = newLabel(container, game.styles.pop, "")
    hide(popLabel)

    scorePanel = lv.obj.new(container)
    lv.obj.remove_style_all(scorePanel)
    lv.obj.add_style(scorePanel, game.styles.panel, lv.STATE_DEFAULT)
    lv.obj.set_size(scorePanel, 112, 26)
    lv.obj.set_pos(scorePanel, 4, 4)
    scoreLabel = newLabel(scorePanel, game.styles.hud, "Score : 0")
    lv.obj.align(scoreLabel, lv.ALIGN_LEFT_MID, 8, 0)

    local lifePanel = lv.obj.new(container)
    lv.obj.remove_style_all(lifePanel)
    lv.obj.add_style(lifePanel, game.styles.panel, lv.STATE_DEFAULT)
    lv.obj.set_size(lifePanel, 72, 26)
    lv.obj.set_pos(lifePanel, W - 76, 4)
    lifeImgs = {}
    for i = 1, LIVES do
        lifeImgs[i] = newImg(lifePanel, I["life.lif"])
        lv.obj.set_pos(lifeImgs[i], 6 + (i - 1) * 22, 5)
    end

    msgLabel = newLabel(container, game.styles.msg, "")
    hide(msgLabel)

    -- fin de partie
    endPanel = lv.obj.new(container)
    lv.obj.remove_style_all(endPanel)
    lv.obj.add_style(endPanel, game.styles.card, lv.STATE_DEFAULT)
    lv.obj.set_size(endPanel, 240, 168)
    lv.obj.align(endPanel, lv.ALIGN_CENTER, 0, 0)
    local bravo = newLabel(endPanel, game.styles.big, "Bravo !")
    lv.obj.align(bravo, lv.ALIGN_TOP_MID, 0, 10)
    endLabel = newLabel(endPanel, game.styles.body, "")
    lv.obj.set_width(endLabel, 224)
    lv.label.set_long_mode(endLabel, lv.LABEL_LONG_WRAP)
    lv.obj.align(endLabel, lv.ALIGN_CENTER, 0, 0)
    endHint = newLabel(endPanel, game.styles.hint, "Tourne la molette pour rejouer")
    lv.obj.set_width(endHint, 224)
    lv.label.set_long_mode(endHint, lv.LABEL_LONG_WRAP)
    lv.obj.align(endHint, lv.ALIGN_BOTTOM_MID, 0, -10)
    hide(endPanel)

    -- écran titre (au-dessus de tout)
    titleLayer = lv.obj.new(container)
    lv.obj.remove_style_all(titleLayer)
    lv.obj.set_size(titleLayer, W, H)
    lv.obj.clear_flag(titleLayer, lv.OBJ_FLAG_SCROLLABLE)
    newImg(titleLayer, I["title.lif"])
    local tp = lv.obj.new(titleLayer)
    lv.obj.remove_style_all(tp)
    lv.obj.add_style(tp, game.styles.card, lv.STATE_DEFAULT)
    lv.obj.set_size(tp, 230, 40)
    lv.obj.align(tp, lv.ALIGN_TOP_MID, 0, 8)
    local tl = newLabel(tp, game.styles.big, "Capi et la rivière")
    lv.obj.align(tl, lv.ALIGN_CENTER, 0, 0)
    local hp = lv.obj.new(titleLayer)
    lv.obj.remove_style_all(hp)
    lv.obj.add_style(hp, game.styles.card, lv.STATE_DEFAULT)
    lv.obj.set_size(hp, 260, 44)
    lv.obj.align(hp, lv.ALIGN_BOTTOM_MID, 0, -6)
    titleHint = newLabel(hp, game.styles.hint, "Tourne la molette pour partir !\nTourne très vite : Capi saute !")
    lv.obj.align(titleHint, lv.ALIGN_CENTER, 0, 0)

    showTitle()
    loopTimer = lv.timer.new(tick, TICK_MS, nil)
end

return game
