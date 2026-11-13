--- Carrousel de choix : 3 vignettes (precedente / focus / suivante), compteur
--- N/total, fleches, label texte (notes.json), navigation circulaire, audio par
--- focus. Adapte de image-choice_1_0_0 (meme cycle de vie create/clean/timer).
--- Appele par engine/story.lua:showChoice quand selector == "carousel".
---@class carousel
local carousel = {}
carousel.styles = {}
carousel.events = {}
carousel.animations = {}
carousel.answers = {}
carousel.answerIterator = 1
carousel.parentContainer = nil
carousel.imgL = nil          -- vignette precedente
carousel.imgC = nil          -- vignette focus (centre)
carousel.imgR = nil          -- vignette suivante
carousel.placeholder = nil   -- carte de repli (image manquante)
carousel.placeholderNum = nil
carousel.counter = nil
carousel.label = nil
carousel.arrowL = nil
carousel.arrowR = nil
-- sources LIF des fleches : gardees ici tant que les objets image les affichent
-- (lv.img.set_src ne retient pas la source, le __gc liberait les pixels)
carousel.arrowLData = nil
carousel.arrowRData = nil
-- Sources des vignettes AFFICHEES, et d'elles seules (#620) : chemin -> source.
-- Meme contrainte que les fleches : la reference Lua vit tant qu'un objet image
-- affiche la source ; une source qui n'est plus affichee est relachee aussitot
-- (un cache de toutes les images vues epuisait la memoire de la Flam au 5e choix).
carousel.sources = {}
-- Chemin affiche par chaque vignette ("C", "L", "R"), nil si masquee.
carousel.slotPath = {}
carousel.keyEvent = nil
carousel.inputProcessTimer = nil
carousel.tick = 0
carousel.rowDy = 0         -- decalage vertical des images (ROW_DY si label, 0 sinon)
carousel.hasLabel = false

-- Geometrie (zoom LVGL : 256 = 1x). Images sources = 320x212 (zone sous le bandeau).
local ZOOM_CENTER = 160      -- ~200x132 (image principale)
local ZOOM_SIDE   = 64       -- ~80x53
local SIDE_OPA    = 90        -- opacite des voisins (0..255)
local SIDE_DX     = 112       -- decalage horizontal des voisins
local ROW_DY      = -14       -- centre vertical sur la zone sans texte (au-dessus du label) ;
                             -- 0 si aucun choix n'a de label (barre masquee)
local ARROW_ON    = 255       -- fleche active
local ARROW_DIM   = 60        -- fleche cote "mort" (2 choix : pas de wrap)
local PH_W, PH_H  = 150, 112  -- carte de repli
local ARROW_L = "script/arrow-left-18x18-ui-000.lif"
local ARROW_R = "script/arrow-right-18x18-ui-000.lif"

function carousel.initStyle()
    carousel.styles.parentContainer = lv.style.new()
    lv.style.set_pad_hor(carousel.styles.parentContainer, 0)
    lv.style.set_pad_ver(carousel.styles.parentContainer, 0)
    lv.style.set_bg_color(carousel.styles.parentContainer, lv.color.hex(0x000000))
    lv.style.set_bg_opa(carousel.styles.parentContainer, lv.OPA_COVER)

    carousel.styles.label = lv.style.new()
    lv.style.set_bg_color(carousel.styles.label, lv.color.black())
    lv.style.set_bg_opa(carousel.styles.label, lv.OPA_COVER)
    lv.style.set_text_color(carousel.styles.label, lv.color.hex(0xefedea))
    lv.style.set_text_font(carousel.styles.label, lv.font.nunito_extrabold_14)
    lv.style.set_pad_top(carousel.styles.label, 4)
    lv.style.set_pad_right(carousel.styles.label, 8)
    lv.style.set_pad_bottom(carousel.styles.label, 4)
    lv.style.set_pad_left(carousel.styles.label, 8)
    lv.style.set_radius(carousel.styles.label, 5)

    carousel.styles.placeholder = lv.style.new()
    lv.style.set_bg_color(carousel.styles.placeholder, lv.color.hex(0x1d1b18))
    lv.style.set_bg_opa(carousel.styles.placeholder, lv.OPA_COVER)
    lv.style.set_border_color(carousel.styles.placeholder, lv.color.hex(0xa79f8e))
    lv.style.set_border_width(carousel.styles.placeholder, 2)
    lv.style.set_radius(carousel.styles.placeholder, 6)
    lv.style.set_text_color(carousel.styles.placeholder, lv.color.hex(0xefedea))
    lv.style.set_text_font(carousel.styles.placeholder, lv.font.nunito_extrabold_20)
end

function carousel.clean()
    Global.requestAudioStop(true, true)
    if (carousel.inputProcessTimer ~= nil) then
        lv.timer.del(carousel.inputProcessTimer)
        carousel.inputProcessTimer = nil
    end
    for container, event in pairs(carousel.events) do
        lv.obj.remove_event_cb(container, event.key)
    end
    for anim, animVar in pairs(carousel.animations) do
        lv.anim_var.del(animVar.var)
    end

    lv.group.set_editing(document, false)
    lv.group.remove_all_objs(document)
    lv.obj.clean(window)

    carousel.answerIterator = 1
    carousel.tick = 0
    carousel.parentContainer = nil
    carousel.imgL = nil
    carousel.imgC = nil
    carousel.imgR = nil
    carousel.placeholder = nil
    carousel.placeholderNum = nil
    carousel.counter = nil
    carousel.label = nil
    carousel.arrowL = nil
    carousel.arrowR = nil
    carousel.arrowLData = nil  -- apres lv.obj.clean : plus aucun objet ne les affiche
    carousel.arrowRData = nil
    carousel.sources = {}      -- idem : les vignettes sont detruites par lv.obj.clean
    carousel.slotPath = {}
    carousel.keyEvent = nil
    carousel.styles = {}
    carousel.events = {}
    carousel.animations = {}
    carousel.answers = {}
end

-- Index circulaire 1-based
local function wrap(i, n)
    if i < 1 then return n end
    if i > n then return 1 end
    return i
end

-- Chemin de l'image d'un choix ; nil sans image (empty.lif : carte de repli au
-- centre, vignette masquee sur les cotes).
local function imgPath(ans)
    if ans == nil or ans.img == nil or ans.img == "empty.lif" then
        return nil
    end
    return ans.img
end

-- Navigation : circulaire si >=3 choix, lineaire (clamp) si 2 (pas de wrap
-- trompeur => coherent avec la fleche grisee du cote sans suite).
local function navigate(cur, delta, n)
    local nx = cur + delta
    if n >= 3 then
        if nx < 1 then nx = n elseif nx > n then nx = 1 end
    else
        if nx < 1 then nx = 1 elseif nx > n then nx = n end
    end
    return nx
end

-- Vignettes : centre d'abord (ordre de chargement).
local SLOTS = { "C", "L", "R" }

local function slotObj(slot)
    if slot == "C" then return carousel.imgC end
    if slot == "L" then return carousel.imgL end
    return carousel.imgR
end

-- Voisins affiches (index, nil = masque) : circulaires si >=3 choix ; si 2,
-- l'autre choix du cote naturel seulement ; aucun si 1.
local function neighbours(n, it)
    if n >= 3 then return wrap(it - 1, n), wrap(it + 1, n) end
    if n == 2 then
        if it == 1 then return nil, 2 end
        return 1, nil
    end
    return nil, nil
end

-- Met les sources en conformite avec le plan `want` (vignette -> chemin ou nil)
-- sans jamais retenir plus que les sources du plan, soit 3 au plus (#620) :
--  1. une vignette dont le chemin change est masquee : plus aucun rendu ne lit
--     son ancienne source (le rendu LVGL n'a lieu qu'apres ce rafraichissement) ;
--  2. les sources hors plan sont relachees puis un GC complet libere leurs
--     pixels AVANT tout chargement ;
--  3. seules les sources du plan absentes sont chargees : celles qui restent
--     affichees (2 sur 3 a chaque cran) ne sont pas rechargees, un fichier commun
--     a plusieurs choix n'est charge qu'une fois.
-- Seuls des chemins passent par des variables locales : aucune source relachee
-- ne reste sur la pile Lua (donc atteignable) pendant le GC.
local function syncSources(want)
    for _, slot in ipairs(SLOTS) do
        if carousel.slotPath[slot] ~= nil and carousel.slotPath[slot] ~= want[slot] then
            lv.obj.add_flag(slotObj(slot), lv.OBJ_FLAG_HIDDEN)
            carousel.slotPath[slot] = nil
        end
    end
    local dropped = false
    for path in pairs(carousel.sources) do
        if path ~= want.C and path ~= want.L and path ~= want.R then
            carousel.sources[path] = nil   -- effacer pendant pairs : permis en Lua
            dropped = true
        end
    end
    if dropped then
        collectgarbage("collect")
    end
    for _, slot in ipairs(SLOTS) do
        local path = want[slot]
        if path ~= nil and carousel.sources[path] == nil then
            carousel.sources[path] = Global.load_image(path)   -- nil si illisible
        end
    end
end

-- Affiche une vignette avec la source retenue de `path`, ou la masque s'il n'y
-- en a pas. Re-aligne APRES set_src : l'auto-size change la taille de l'objet,
-- donc la position calculee a la creation (taille 0) serait fausse.
local function showSlot(slot, path, zoom, opa, dx)
    local obj = slotObj(slot)
    local data = path and carousel.sources[path]
    if not data then
        lv.obj.add_flag(obj, lv.OBJ_FLAG_HIDDEN)
        carousel.slotPath[slot] = nil
        return false
    end
    lv.img.set_src(obj, data)
    lv.img.set_zoom(obj, zoom)
    lv.obj.set_style_img_opa(obj, opa, lv.STATE_DEFAULT)
    lv.obj.align(obj, lv.ALIGN_CENTER, dx, carousel.rowDy)
    lv.obj.clear_flag(obj, lv.OBJ_FLAG_HIDDEN)
    carousel.slotPath[slot] = path
    return true
end

-- Fleche active (opaque) ou "morte" (grisee).
local function setArrow(arrowObj, active)
    lv.obj.set_style_img_opa(arrowObj, active and ARROW_ON or ARROW_DIM, lv.STATE_DEFAULT)
end

-- Met a jour les 3 vignettes, les fleches et le label (SANS audio).
function carousel.refreshVisual()
    local n = #carousel.answers
    local it = carousel.answerIterator
    local center = carousel.answers[it]
    local li, ri = neighbours(n, it)
    local want = {
        C = imgPath(center),
        L = li and imgPath(carousel.answers[li]) or nil,
        R = ri and imgPath(carousel.answers[ri]) or nil,
    }
    syncSources(want)

    -- Centre : image ou carte de repli.
    if showSlot("C", want.C, ZOOM_CENTER, lv.OPA_COVER, 0) then
        lv.obj.add_flag(carousel.placeholder, lv.OBJ_FLAG_HIDDEN)
    else
        lv.obj.clear_flag(carousel.placeholder, lv.OBJ_FLAG_HIDDEN)
        lv.label.set_text(carousel.placeholderNum, tostring(it))
        lv.obj.align(carousel.placeholderNum, lv.ALIGN_CENTER, 0, 0)
    end

    -- Voisins (attenues, en arriere-plan).
    showSlot("L", want.L, ZOOM_SIDE, SIDE_OPA, -SIDE_DX)
    showSlot("R", want.R, ZOOM_SIDE, SIDE_OPA, SIDE_DX)

    -- Fleches : actives si >=3 choix ; si 2, celle du cote sans suite est
    -- grisee (pas de wrap) ; aucune si 1 choix.
    if n >= 3 then
        setArrow(carousel.arrowL, true)
        setArrow(carousel.arrowR, true)
    elseif n == 2 then
        setArrow(carousel.arrowL, it ~= 1)
        setArrow(carousel.arrowR, it == 1)
    else
        lv.obj.add_flag(carousel.arrowL, lv.OBJ_FLAG_HIDDEN)
        lv.obj.add_flag(carousel.arrowR, lv.OBJ_FLAG_HIDDEN)
    end

    -- Label texte (notes.json) ou espace pour garder la barre.
    lv.label.set_text(carousel.label, (center and center.label) or " ")
end

-- stopNow : changement de focus => l'audio de l'option quittee est coupe tout
-- de suite, seul le chargement du nouvel audio reste differe (B6).
function carousel.playFocus(stopNow)
    local ans = carousel.answers[carousel.answerIterator]
    if ans == nil then return end
    local priority = true
    if ans.priority ~= nil then priority = ans.priority end
    Global.requestAudioPlay({ path = ans.audio, priority = priority, stopNow = stopNow })
end

function carousel.refresh()
    carousel.refreshVisual()
    carousel.playFocus(true)
end

function carousel.audioFeedback(state, second)
    if (state == "stop") then
        carousel.playFocus()
    end
end

function carousel.processKeyEvent()
    -- Fenetre anti-rebond a l'entree (~200 ms) : ignore le clic qui a OUVERT le
    -- choix pour eviter une validation immediate de la 1re option. On avance sur
    -- le TEMPS (a chaque tick), PAS sur le nb d'appuis : sinon la 1re option
    -- reste non validable tant qu'on n'a pas navigue (le compteur ne montait
    -- qu'a chaque touche => les 2 premiers ENTER etaient avales).
    if (carousel.tick <= 1) then
        carousel.tick = carousel.tick + 1
        carousel.keyEvent = nil
        return
    end
    if (carousel.keyEvent ~= nil) then
        local b = string.byte(carousel.keyEvent)
        local n = #carousel.answers
        if (b == 20 or b == 19) then
            local delta = (b == 20) and -1 or 1
            carousel.answerIterator = navigate(carousel.answerIterator, delta, n)
        end
        if (b == 10) then
            -- ENTER consomme AVANT le callback : sinon cb() est rappele a chaque
            -- tick (100 ms) tant que le module suivant n'a pas supprime ce timer.
            local cb = carousel.answers[carousel.answerIterator].cb
            carousel.keyEvent = nil
            if (cb ~= nil) then
                cb()
            end
        else
            carousel.refresh()
            carousel.keyEvent = nil
        end
    end
end

-- B7 : appele par Global.flushPendingKey avant un retour (Home/ESC) : un ENTER
-- en attente (hors fenetre anti-rebond) est traite tout de suite au lieu d'etre
-- perdu au nettoyage du module. Les autres touches en attente sont ignorees
-- (le retour porte sur l'option affichee). Renvoie true si un ENTER est traite.
function carousel.flushPendingKey()
    if (carousel.keyEvent == nil or carousel.tick <= 1) then
        return false
    end
    if (string.byte(carousel.keyEvent) ~= 10) then
        return false
    end
    carousel.processKeyEvent()
    return true
end

function carousel.keyPressed(e)
    lv.timer.reset(carousel.inputProcessTimer)
    carousel.keyEvent = lv.event.get_key_value(e)
end

-- Cree un objet image vignette (auto-size sur la source, pivot = centre).
local function newVignette(parent, dx, dy)
    local img = lv.img.new(parent)
    lv.obj.remove_style_all(img)
    -- FLOATING : les boites image (320x212) debordent du conteneur ; sans ce
    -- flag, le conteneur scrollable (garde pour ENTER) auto-scrolle vers un
    -- voisin qui deborde => tout l'ecran se decale (cf. bug pos2). FLOATING =
    -- objet ignore par scroll/layout, positionne uniquement par align.
    lv.obj.add_flag(img, lv.OBJ_FLAG_FLOATING)
    lv.obj.add_flag(img, lv.OBJ_FLAG_HIDDEN)
    lv.obj.align(img, lv.ALIGN_CENTER, dx, dy)
    return img
end

function carousel.create(args)
    lv.obj.clean(window)
    lv.group.set_editing(document, true)
    carousel.initStyle()

    carousel.parentContainer = lv.obj.new(window)
    lv.obj.remove_style_all(carousel.parentContainer)
    lv.obj.set_size(carousel.parentContainer, lv.obj.get_width(window), lv.obj.get_height(window))
    lv.obj.add_style(carousel.parentContainer, carousel.styles.parentContainer, lv.STATE_DEFAULT)
    -- NE PAS retirer OBJ_FLAG_SCROLLABLE : l'encodeur ne delivre LV_KEY_ENTER
    -- (EVENT_KEY byte 10 => validation) que si l'objet focus est editable OU
    -- scrollable (cf. lv_indev.c indev_encoder_proc, release ENTER). image-choice
    -- garde ce flag pour la meme raison.

    carousel.events[carousel.parentContainer] = {}
    carousel.events[carousel.parentContainer].key = lv.obj.add_event_cb(carousel.parentContainer,
        carousel.keyPressed, lv.EVENT_KEY)

    -- Collecte des choix (respecte show=false comme image-choice).
    local lastCb = nil
    for _, v in ipairs(args.choices) do
        if v.show ~= false then
            table.insert(carousel.answers, v)
            lastCb = v.cb
        end
    end

    -- Barre de label seulement si au moins un choix a un label (notes.json) :
    -- sinon (cas frequent, texte deja dans l'image) images centrees, pas de barre vide.
    carousel.hasLabel = false
    for _, v in ipairs(carousel.answers) do
        if type(v.label) == "string" and v.label ~= "" then carousel.hasLabel = true; break end
    end
    carousel.rowDy = carousel.hasLabel and ROW_DY or 0

    if (#carousel.answers == 0) then
        return args.exitCb()
    elseif #carousel.answers == 1 and lastCb ~= nil and args.skipIfLastChoice == true then
        return lastCb()
    end

    -- Vignettes (voisins derriere, centre devant) + carte de repli.
    carousel.imgL = newVignette(carousel.parentContainer, -SIDE_DX, carousel.rowDy)
    carousel.imgR = newVignette(carousel.parentContainer, SIDE_DX, carousel.rowDy)
    carousel.imgC = newVignette(carousel.parentContainer, 0, carousel.rowDy)

    carousel.placeholder = lv.obj.new(carousel.parentContainer)
    lv.obj.remove_style_all(carousel.placeholder)
    lv.obj.set_size(carousel.placeholder, PH_W, PH_H)
    lv.obj.add_style(carousel.placeholder, carousel.styles.placeholder, lv.STATE_DEFAULT)
    lv.obj.clear_flag(carousel.placeholder, lv.OBJ_FLAG_SCROLLABLE)
    lv.obj.align(carousel.placeholder, lv.ALIGN_CENTER, 0, carousel.rowDy)
    lv.obj.add_flag(carousel.placeholder, lv.OBJ_FLAG_HIDDEN)
    carousel.placeholderNum = lv.label.new(carousel.placeholder)
    lv.obj.remove_style_all(carousel.placeholderNum)
    lv.obj.add_style(carousel.placeholderNum, carousel.styles.placeholder, lv.STATE_DEFAULT)
    lv.obj.set_style_bg_opa(carousel.placeholderNum, lv.OPA_TRANSP, lv.STATE_DEFAULT)
    lv.obj.align(carousel.placeholderNum, lv.ALIGN_CENTER, 0, 0)

    -- Fleches (devant les vignettes). Masquees s'il n'y a qu'un choix.
    local aw, ah
    carousel.arrowLData, aw, ah = Global.load_image(ARROW_L)
    carousel.arrowL = lv.img.new(carousel.parentContainer)
    lv.obj.remove_style_all(carousel.arrowL)
    lv.img.set_src(carousel.arrowL, carousel.arrowLData)
    lv.obj.set_size(carousel.arrowL, aw, ah)
    lv.obj.align(carousel.arrowL, lv.ALIGN_LEFT_MID, 6, carousel.rowDy)

    local arw, arh
    carousel.arrowRData, arw, arh = Global.load_image(ARROW_R)
    carousel.arrowR = lv.img.new(carousel.parentContainer)
    lv.obj.remove_style_all(carousel.arrowR)
    lv.img.set_src(carousel.arrowR, carousel.arrowRData)
    lv.obj.set_size(carousel.arrowR, arw, arh)
    lv.obj.align(carousel.arrowR, lv.ALIGN_RIGHT_MID, -6, carousel.rowDy)

    if (#carousel.answers < 2) then
        lv.obj.add_flag(carousel.arrowL, lv.OBJ_FLAG_HIDDEN)
        lv.obj.add_flag(carousel.arrowR, lv.OBJ_FLAG_HIDDEN)
    end

    -- Label (bas centre, defilement circulaire si long).
    carousel.label = lv.label.new(carousel.parentContainer)
    lv.obj.remove_style_all(carousel.label)
    lv.obj.set_width(carousel.label, 296)
    lv.label.set_long_mode(carousel.label, lv.LABEL_LONG_SCROLL_CIRCULAR)
    lv.obj.clear_flag(carousel.label, lv.OBJ_FLAG_SCROLLABLE)
    lv.obj.clear_flag(carousel.label, lv.OBJ_FLAG_CLICK_FOCUSABLE)
    lv.obj.add_style(carousel.label, carousel.styles.label, lv.STATE_DEFAULT)
    lv.obj.set_style_text_align(carousel.label, lv.TEXT_ALIGN_CENTER, 0)
    lv.obj.align(carousel.label, lv.ALIGN_BOTTOM_MID, 0, -4)
    if not carousel.hasLabel then
        lv.obj.add_flag(carousel.label, lv.OBJ_FLAG_HIDDEN)
    end

    lv.group.add_obj(document, carousel.parentContainer)

    carousel.refreshVisual()
    if (args.title_audio ~= nil) then
        Global.requestAudioPlay({ path = args.title_audio, AFCb = carousel.audioFeedback, priority = true })
    else
        carousel.playFocus()
    end

    carousel.inputProcessTimer = lv.timer.new(carousel.processKeyEvent, 100, nil)
end

return carousel
