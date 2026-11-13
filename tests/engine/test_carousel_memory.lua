-- test_carousel_memory.lua — carrousel du runtime telmi2flam (#620) : sources
-- d'images retenues par le module, sur le VRAI LVGL de flam-test.
-- Lancement : flam-test tests\engine\test_carousel_memory.lua (suite test_run.bat).
--
-- Sur la Flam, lv.img.set_src ne retient pas la source : seule la reference Lua
-- du module la garde en vie, et le firmware n'a de place que pour peu d'images
-- decodees. Le carrousel mettait en cache l'image de chaque choix vu : avec un
-- menu de 18 choix a images, la Flam redemarrait des le 5e choix.
--
-- Les sources sont ici des tables factices, comme sur la Flam retenues par le
-- seul Lua (le simulateur ancre les vraies sources dans l'objet image, ce qui
-- masquerait une source affichee mais relachee). On verifie, a chaque cran :
--   - au plus 3 sources retenues, a tout instant (pic mesure a chaque chargement,
--     apres un GC complet, depuis l'interieur du rafraichissement) ;
--   - les sources retenues sont exactement celles des vignettes affichees ;
--   - chaque vignette affichee montre la bonne image et sa source est vivante ;
--   - une image qui reste affichee n'est pas rechargee (1 chargement par cran).

local src = debug.getinfo(1, "S").source
local HERE = src:sub(1, 1) == "@" and src:sub(2):match("^(.*)[/\\]") or "."
-- FLAM_RUNTIME_DIR : autre copie du runtime (ex. verifier que les tests echouent sur l'original)
local RUNTIME = os.getenv("FLAM_RUNTIME_DIR") or (HERE .. "/../../tools/telmi2flam/runtime/script/")
package.path = HERE .. "/../lua/?.lua;" .. RUNTIME .. "?.lua;" .. package.path

require("test_helpers")
print("--- test_carousel_memory (#620 : sources retenues par le carrousel) ---")

local KEY_NEXT, KEY_PREV = 19, 20

state = state or { visited_funs = {}, current_fun = "f1" }
screen = screen or { set_state = function() end }

-- Audio mock : sans effet (le carrousel demande l'audio du focus a chaque cran)
local A = { status = "stop" }
audio = {
    load = function() return 0 end,
    play = function() A.status = "play" end, pause = function() A.status = "pause" end,
    stop = function() A.status = "stop" end, get_status = function() return A.status end,
    seek = function() end, duration = function() return 0 end,
}

Global = require("global")
Global.init()

-- ---------------------------------------------------------------------------
-- Instrumentation : sources factices, vignettes affichees
-- ---------------------------------------------------------------------------
local live = setmetatable({}, { __mode = "k" })   -- source vivante -> chemin
local shown = setmetatable({}, { __mode = "v" })  -- objet image -> source affichee
local hidden = {}                                 -- objet image -> masque ?
local M = { loads = 0, peak = 0, failed = 0, unreadable = {} }   -- unreadable : chemins illisibles

local function isArrow(p) return p:find("arrow", 1, true) ~= nil end

-- Sources d'images (hors fleches) encore atteignables apres un GC complet.
local function retained()
    collectgarbage("collect"); collectgarbage("collect")
    local paths, n = {}, 0
    for s, p in pairs(live) do
        if not isArrow(p) then
            n = n + 1
            paths[#paths + 1] = p
        end
    end
    table.sort(paths)
    return n, paths
end

local saved = {
    load_image = Global.load_image,
    set_src = lv.img.set_src,
    add_flag = lv.obj.add_flag,
    clear_flag = lv.obj.clear_flag,
}

Global.load_image = function(p)
    if not isArrow(p) then
        -- pic : sources deja retenues au moment du chargement, plus la nouvelle
        local n = retained()
        if n + 1 > M.peak then M.peak = n + 1 end
        M.loads = M.loads + 1
        -- apres le GC, au moment ou une source relachee serait fatale sur la Flam :
        -- toute vignette non masquee doit afficher une source vivante
        local car = M.car
        if car and car.imgC and M.bad == nil then
            for slot, obj in pairs({ C = car.imgC, L = car.imgL, R = car.imgR }) do
                if hidden[obj] == false then
                    local s = shown[obj]
                    if s == nil or live[s] == nil then
                        M.bad = "vignette " .. slot .. " visible sur une source liberee (chargement de " .. p .. ")"
                    end
                end
            end
        end
        if M.unreadable[p] then
            M.failed = M.failed + 1
            return nil, 0, 0               -- image illisible (lv.img_src.load -> nil)
        end
    end
    local s = { path = p }
    live[s] = p
    return s, 18, 18
end
lv.img.set_src = function(obj, s)
    shown[obj] = s
    return saved.set_src(obj, s)      -- table : ignoree par le binding
end
lv.obj.add_flag = function(obj, f)
    if f == lv.OBJ_FLAG_HIDDEN then hidden[obj] = true end
    return saved.add_flag(obj, f)
end
lv.obj.clear_flag = function(obj, f)
    if f == lv.OBJ_FLAG_HIDDEN then hidden[obj] = false end
    return saved.clear_flag(obj, f)
end

-- ---------------------------------------------------------------------------
-- Outils
-- ---------------------------------------------------------------------------
local function wrap(i, n)
    if i < 1 then return n end
    if i > n then return 1 end
    return i
end

-- Image attendue de chaque vignette (nil = masquee), d'apres le choix focalise.
local function expectedSlots(imgs, n, it)
    local function path(i)
        local p = i and imgs[i]
        if p == nil or p == "empty.lif" or M.unreadable[p] then return nil end
        return p
    end
    local li, ri
    if n >= 3 then li, ri = wrap(it - 1, n), wrap(it + 1, n)
    elseif n == 2 then
        if it == 1 then ri = 2 else li = 1 end
    end
    return { C = path(it), L = path(li), R = path(ri) }
end

local function check(car, imgs, where)
    local n = #imgs
    local want = expectedSlots(imgs, n, car.answerIterator)
    expect_nil(M.bad, where .. " : pendant le chargement")
    local count, paths = retained()
    expect_true(count <= 3, where .. " : " .. count .. " sources retenues (> 3) : " .. table.concat(paths, ","))
    expect_true(M.peak <= 3, where .. " : pic de " .. M.peak .. " sources (> 3)")
    -- sources retenues = images affichees (distinctes)
    local set, distinct = {}, 0
    for _, p in pairs(want) do
        if not set[p] then set[p] = true; distinct = distinct + 1 end
    end
    expect_eq(count, distinct, where .. " : sources retenues = images affichees (" .. table.concat(paths, ",") .. ")")
    for _, p in ipairs(paths) do
        expect_true(set[p], where .. " : source retenue non affichee " .. p)
    end
    -- chaque vignette affichee montre la bonne image, source vivante
    for slot, obj in pairs({ C = car.imgC, L = car.imgL, R = car.imgR }) do
        if want[slot] == nil then
            expect_true(hidden[obj] == true, where .. " : vignette " .. slot .. " masquee")
        else
            expect_true(hidden[obj] == false, where .. " : vignette " .. slot .. " visible")
            local s = shown[obj]
            expect_true(s ~= nil and live[s] ~= nil, where .. " : source de la vignette " .. slot .. " liberee")
            expect_eq(live[s], want[slot], where .. " : image de la vignette " .. slot)
        end
    end
    -- carte de repli seulement sans image au centre
    expect_eq(hidden[car.placeholder] == true, want.C ~= nil, where .. " : carte de repli")
end

-- Ouvre le carrousel sur `imgs` (focus initial `sel`) ; renvoie le module.
local function open(imgs, sel)
    M.loads, M.peak, M.failed, M.bad = 0, 0, 0, nil
    local car = Global.load_module("carousel", "1_0_0")
    M.car = car
    if sel then car.answerIterator = sel end
    local choices = {}
    for i = 1, #imgs do
        choices[i] = { img = imgs[i], audio = "o" .. i .. ".mp3", label = "Choix " .. i }
    end
    car.create({ choices = choices })
    choices = nil
    car.tick = 2   -- fenetre anti-rebond passee
    return car
end

-- Un cran (meme chemin que le timer 100 ms du module).
local function step(car, k)
    car.keyEvent = string.char(k)
    car.processKeyEvent()
end

local function close()
    Global.cleanCurrentModule()
    M.car, M.unreadable = nil, {}
    local count, paths = retained()
    expect_eq(count, 0, "clean : plus aucune source retenue (" .. table.concat(paths, ",") .. ")")
    test_tick(2)
end

local function numbered(n)
    local t = {}
    for i = 1, n do t[i] = string.format("img%02d.lif", i) end
    return t
end

-- ---------------------------------------------------------------------------
-- Tests
-- ---------------------------------------------------------------------------
test("#620 carrousel 18 choix : tour complet dans les deux sens, <= 3 sources", function()
    local imgs = numbered(18)
    local car = open(imgs)
    check(car, imgs, "ouverture")
    expect_eq(M.loads, 3, "ouverture : 3 images chargees")
    for i = 1, 20 do                       -- 1 -> 18 -> 1 -> 3 (retour circulaire)
        local before = M.loads
        step(car, KEY_NEXT)
        expect_eq(car.answerIterator, (i % 18) + 1, "cran avant " .. i)
        check(car, imgs, "avant " .. i)
        expect_eq(M.loads - before, 1, "avant " .. i .. " : une seule image chargee")
    end
    for i = 1, 22 do                       -- 3 -> 1 -> 18 -> ... -> 17
        local before = M.loads
        step(car, KEY_PREV)
        check(car, imgs, "arriere " .. i)
        expect_eq(M.loads - before, 1, "arriere " .. i .. " : une seule image chargee")
    end
    expect_eq(car.answerIterator, 17, "position finale")
    -- demi-tour : l'image qui sort n'est pas rechargee, une seule nouvelle
    local before = M.loads
    step(car, KEY_NEXT)
    check(car, imgs, "demi-tour")
    expect_eq(M.loads - before, 1, "demi-tour : une seule image chargee")
    close()
end)

test("#620 carrousel 30 choix ouvert sur le 30e : retour circulaire 30 -> 1", function()
    local imgs = numbered(30)
    local car = open(imgs, 30)
    check(car, imgs, "ouverture sur 30")
    step(car, KEY_NEXT)
    expect_eq(car.answerIterator, 1, "30 -> 1")
    check(car, imgs, "30 -> 1")
    step(car, KEY_PREV); step(car, KEY_PREV)
    expect_eq(car.answerIterator, 29, "1 -> 29")
    check(car, imgs, "1 -> 29")
    close()
end)

test("#620 carrousel 3 choix : tout reste en memoire, aucun rechargement", function()
    local imgs = numbered(3)
    local car = open(imgs)
    check(car, imgs, "ouverture")
    local before = M.loads
    for i = 1, 4 do step(car, KEY_NEXT); check(car, imgs, "avant " .. i) end
    for i = 1, 4 do step(car, KEY_PREV); check(car, imgs, "arriere " .. i) end
    expect_eq(M.loads, before, "aucun rechargement")
    close()
end)

test("#620 carrousel 2 choix (non circulaire) et 1 choix", function()
    local imgs = numbered(2)
    local car = open(imgs)
    check(car, imgs, "2 choix, ouverture")
    step(car, KEY_PREV)                    -- borne : pas de wrap
    expect_eq(car.answerIterator, 1, "pas de wrap a gauche")
    check(car, imgs, "2 choix, borne gauche")
    step(car, KEY_NEXT)
    check(car, imgs, "2 choix, cran 2")
    step(car, KEY_NEXT)
    expect_eq(car.answerIterator, 2, "pas de wrap a droite")
    check(car, imgs, "2 choix, borne droite")
    expect_eq(M.loads, 2, "2 images chargees une fois chacune")
    close()

    imgs = numbered(1)
    car = open(imgs)
    check(car, imgs, "1 choix")
    close()
end)

test("#620 carrousel : choix sans image et fichier partage par plusieurs choix", function()
    -- 1 et 3 partagent a.lif, 2 et 5 sans image, 4 et 6 partagent b.lif
    local imgs = { "a.lif", "empty.lif", "a.lif", "b.lif", "empty.lif", "b.lif", "c.lif" }
    local car = open(imgs)
    check(car, imgs, "ouverture")
    for i = 1, 9 do step(car, KEY_NEXT); check(car, imgs, "avant " .. i) end
    for i = 1, 9 do step(car, KEY_PREV); check(car, imgs, "arriere " .. i) end
    close()

    -- toutes les options sur la meme image : chargee une seule fois
    imgs = { "a.lif", "a.lif", "a.lif", "a.lif", "a.lif" }
    car = open(imgs)
    for i = 1, 6 do step(car, KEY_NEXT); check(car, imgs, "meme image " .. i) end
    expect_eq(M.loads, 1, "meme image : un seul chargement")
    close()

    -- aucune image : cartes de repli, aucune source
    imgs = { "empty.lif", "empty.lif", "empty.lif", "empty.lif" }
    car = open(imgs)
    for i = 1, 5 do step(car, KEY_NEXT); check(car, imgs, "sans image " .. i) end
    expect_eq(M.loads, 0, "sans image : aucun chargement")
    close()
end)

test("#620 carrousel : image illisible -> masquee ou carte de repli, reessayee au cran suivant", function()
    M.unreadable = { ["img02.lif"] = true }
    local imgs = numbered(6)
    local car = open(imgs)                 -- 6, 1, 2 : 2 illisible a droite
    check(car, imgs, "ouverture")
    expect_eq(M.failed, 1, "ouverture : 1 essai sur l'image illisible")
    step(car, KEY_NEXT)                    -- focus 2 : carte de repli
    check(car, imgs, "illisible au centre")
    expect_eq(M.failed, 2, "nouvel essai au cran suivant")
    step(car, KEY_NEXT)                    -- 2 a gauche
    check(car, imgs, "illisible a gauche")
    expect_eq(M.failed, 3, "nouvel essai au cran suivant")
    step(car, KEY_NEXT)                    -- 2 n'est plus affichee
    check(car, imgs, "illisible hors affichage")
    expect_eq(M.failed, 3, "plus d'essai hors affichage")
    close()
end)

-- Restauration (autres fichiers du meme processus flam-test)
Global.load_image = saved.load_image
lv.img.set_src = saved.set_src
lv.obj.add_flag = saved.add_flag
lv.obj.clear_flag = saved.clear_flag
if Global.audioDelayTimer then lv.timer.del(Global.audioDelayTimer); Global.audioDelayTimer = nil end
print(string.format("Results: %d passed, %d failed", TEST_PASS, TEST_FAIL))
