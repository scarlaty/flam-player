-- regress_hunt1_ctx_ref.lua — Non-regression : ref context_menu entre deux etats
-- flam-test enchaine plusieurs fichiers (un lua_State chacun) sans fw_reset.
-- g_ctx_entries_ref gardait la ref de l'etat ferme : le premier
-- context_menu.set_entries de l'etat suivant faisait luaL_unref d'un slot
-- vivant du nouveau registre (ici la func_ref d'un timer), reattribue a la
-- table des entrees -> "[TIMER] callback error: attempt to call a table value".
-- ATTENDU : ECHOUE AVANT LE CORRECTIF (fw_register_globals remet la ref a NOREF).
--
-- Le bug demande deux etats dans le meme processus ; test_run.bat lance ce
-- fichier seul. Mode pilote (par defaut) : relance flam-test.exe avec ce meme
-- fichier deux fois, variable FLAM_HUNT1_CTX = chemin d'un fichier marqueur.
-- Mode enfant : absence du marqueur = etat 1 (le cree, set_entries),
-- presence = etat 2 (le supprime, timer puis set_entries).

require("test_helpers")
print("--- regress_hunt1_ctx_ref ---")

local marker = os.getenv("FLAM_HUNT1_CTX")

if marker and marker ~= "" then
    local f = io.open(marker, "r")
    if not f then
        -- Etat 1
        local w = assert(io.open(marker, "w"))
        w:write("1")
        w:close()
        test("etat 1 : context_menu.set_entries", function()
            context_menu.set_entries({ { title = "A", cb = function() end } })
            expect_true(true)
        end)
    else
        -- Etat 2
        f:close()
        os.remove(marker)
        test("etat 2 : la ref du timer survit a set_entries", function()
            local calls = 0
            local t = lv.timer.new(function() calls = calls + 1 end, 5)
            context_menu.set_entries({ { title = "B" } })
            test_tick(10)
            expect_ge(calls, 1, "callback du timer appele")
            lv.timer.del(t)
        end)
    end
    return
end

-- Mode pilote
test("deux etats successifs : set_entries ne libere pas une ref vivante", function()
    local src = debug.getinfo(1, "S").source
    local self_path = src:sub(1, 1) == "@" and src:sub(2) or src
    -- test_run.bat lance flam-test depuis le dossier de build
    local exe = "flam-test.exe"
    local fx = io.open(exe, "rb")
    if not fx then
        print("  [SKIP] flam-test.exe absent du dossier courant")
        expect_true(true)
        return
    end
    fx:close()

    local tmp = os.getenv("TEMP") or os.getenv("TMPDIR") or "."
    local mk = string.format("%s/flam_hunt1_ctx_%d_%d.tmp", tmp, os.time(),
        math.random(1, 1000000))
    os.remove(mk)
    -- guillemets externes : cmd /c retire le premier et le dernier
    local cmd = string.format('"set "FLAM_HUNT1_CTX=%s"&& ".\\%s" --timeout 20 "%s" "%s""',
        mk, exe, self_path, self_path)
    if package.config:sub(1, 1) ~= "\\" then
        cmd = string.format('FLAM_HUNT1_CTX="%s" "./%s" --timeout 20 "%s" "%s"',
            mk, exe, self_path, self_path)
    end
    local ok, how, code = os.execute(cmd)
    os.remove(mk)
    expect_true(ok == true and how == "exit" and code == 0,
        "processus enfant (2 etats) : " .. tostring(how) .. " " .. tostring(code))
end)
