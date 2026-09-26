-- regress_hunt2_ctx_menu.lua — Non-regression : construction du menu contextuel
-- show_context_menu est appele depuis fw_pump (boucle principale, hors pcall).
--  (a) une entree dont le metatable a un __index qui leve une erreur :
--      lua_getfield propageait l'erreur hors mode protege -> "PANIC: unprotected
--      error in call to Lua API" puis abort.
--  (b) des milliers d'entrees : lv_btn_create/lv_label_create epuisaient le tas
--      LVGL (512 Ko) sans garde -> 0xC0000005.
-- ATTENDU : ECHOUE AVANT LE CORRECTIF (construction sous lua_pcall, acces bruts,
-- garde du tas avant chaque bouton, menu tronque).
-- Note : en Debug, l'abort d'avant le correctif ouvre une boite modale (le
-- processus enfant reste bloque).
--
-- flam-test n'appelle pas fw_pump : le test lance flam-player.exe (pilotes SDL
-- dummy, rendu logiciel) sur un petit script ; FLAM_TEST_CTX_MENU_MS simule l'appui sur M
-- (sdl_driver.c), un timer du script termine le processus par os.exit(0).

require("test_helpers")
print("--- regress_hunt2_ctx_menu ---")

local exe = "flam-player.exe"   -- test_run.bat lance depuis le dossier de build
local is_win = package.config:sub(1, 1) == "\\"
local tmp = os.getenv("TEMP") or os.getenv("TMPDIR") or "."
local tag = string.format("%d_%d", os.time(), math.random(1, 1000000))

local function have_exe()
    local f = io.open(exe, "rb")
    if f then f:close() return true end
    return false
end

local function read_all(path)
    local f = io.open(path, "rb")
    if not f then return "" end
    local s = f:read("a") or ""
    f:close()
    return s
end

-- Lance flam-player sur 'body' (script Lua) ; retourne ok, code, stderr
local function run_child(name, body)
    local script = string.format("%s/flam_hunt2_%s_%s.lua", tmp, name, tag)
    local errf   = string.format("%s/flam_hunt2_%s_%s.err", tmp, name, tag)
    local save   = string.format("%s/flam_hunt2_%s_%s_save", tmp, name, tag)
    local w = assert(io.open(script, "w"))
    -- Timer de sortie en premier : il termine le processus meme si la
    -- suite du script echoue (flam-player resterait sinon ouvert)
    w:write([[
lv.timer.new(function()
    io.stderr:write("HUNT2_ALIVE\n")
    io.stderr:flush()
    os.exit(0)
end, 1500)
]])
    w:write(body)
    w:close()
    local cmd
    if is_win then
        -- guillemets externes : cmd /c retire le premier et le dernier
        cmd = string.format('"set "SDL_VIDEODRIVER=dummy"&& set "SDL_AUDIODRIVER=dummy"'
            .. '&& set "SDL_RENDER_DRIVER=software"&& set "FLAM_TEST_CTX_MENU_MS=200"&& ".\\%s" --watchdog 0'
            .. ' --save-dir "%s" "%s" 1>nul 2>"%s""', exe, save, script, errf)
    else
        cmd = string.format('SDL_VIDEODRIVER=dummy SDL_AUDIODRIVER=dummy SDL_RENDER_DRIVER=software '
            .. 'FLAM_TEST_CTX_MENU_MS=200 "./%s" --watchdog 0 --save-dir "%s" "%s"'
            .. ' >/dev/null 2>"%s"', exe, save, script, errf)
    end
    local ok, how, code = os.execute(cmd)
    local err = read_all(errf)
    os.remove(script)
    os.remove(errf)
    return ok == true and how == "exit" and code == 0, tostring(how) .. " " .. tostring(code), err
end

local function check(name, body, extra)
    if not have_exe() then
        print("  [SKIP] flam-player.exe absent du dossier courant")
        expect_true(true)
        return
    end
    local ok, st, err = run_child(name, body)
    expect_true(ok, "flam-player termine normalement : " .. st)
    expect_true(not err:find("PANIC", 1, true), "pas de PANIC Lua")
    expect_true(err:find("HUNT2_ALIVE", 1, true) ~= nil,
        "le script a survecu a l'ouverture du menu")
    if extra then extra(err) end
end

test("entree avec __index qui leve une erreur : pas de PANIC", function()
    check("index", [[
context_menu.set_entries({
    { title = "Normale", cb = function() end },
    setmetatable({}, { __index = function(t, k) error("boom " .. k) end }),
})
]])
end)

test("3000 entrees : menu tronque, pas de crash du tas LVGL", function()
    check("many", [[
local e = {}
for i = 1, 3000 do e[i] = { title = "Entree numero " .. i, cb = function() end } end
context_menu.set_entries(e)
]], function(err)
        expect_true(err:find("menu tronque", 1, true) ~= nil,
            "message de troncature du menu")
    end)
end)

test("menu normal : ni troncature ni erreur", function()
    check("normal", [[
context_menu.set_entries({
    { title = "A", cb = function() end },
    { title = 42 },
    { title = "C", cb = function() end },
})
]], function(err)
        expect_true(not err:find("context_menu:", 1, true),
            "aucun message context_menu")
    end)
end)
