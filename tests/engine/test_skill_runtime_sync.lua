-- test_skill_runtime_sync.lua — garde de derive du runtime embarque par le skill
-- flam-homebrew (#620). Lancement : flam-test tests\engine\test_skill_runtime_sync.lua
-- (suite test_run.bat).
--
-- Le skill embarque une copie du runtime de telmi2flam (skills/flam-homebrew/
-- assets/runtime) : chaque correctif du runtime doit atteindre les deux copies.
-- Pour chaque fichier du manifeste de telmi2flam (tous les .lua de script/ et les
-- .lif de img/script/), la copie du skill doit exister et avoir les MEMES OCTETS
-- (fins de ligne comprises : LF des deux cotes). Aucune exception aujourd'hui ;
-- toute divergence voulue doit etre declaree dans EXCEPTIONS, avec sa raison.
-- Limite : un fichier present seulement dans le skill n'est pas detecte (pas de
-- listage de dossier en Lua) ; flam_build.py l'embarquerait quand meme.

local src = debug.getinfo(1, "S").source
local HERE = src:sub(1, 1) == "@" and src:sub(2):match("^(.*)[/\\]") or "."
package.path = HERE .. "/../lua/?.lua;" .. package.path
require("test_helpers")
print("--- test_skill_runtime_sync (copie du runtime dans le skill flam-homebrew) ---")

local ROOT = HERE .. "/../../"
-- FLAM_SKILL_RUNTIME_DIR : autre copie a comparer (ex. verifier que la garde echoue)
local REF = ROOT .. "tools/telmi2flam/runtime/"
local SKILL = os.getenv("FLAM_SKILL_RUNTIME_DIR") or (ROOT .. "skills/flam-homebrew/assets/runtime/")

-- chemin relatif -> raison d'une divergence acceptee (vide : copies identiques)
local EXCEPTIONS = {}

local function slurp(path)
    local f = io.open(path, "rb")
    if f == nil then return nil end
    local data = f:read("a")
    f:close()
    return data
end

local function manifest()
    local text = slurp(REF .. "manifest.txt")
    expect_true(text ~= nil, "manifeste de telmi2flam illisible : " .. REF .. "manifest.txt")
    local files = {}
    for line in text:gmatch("[^\r\n]+") do
        if line:sub(1, 1) ~= "#" then
            local rel = line:gsub("^!", "")
            if rel ~= "" then files[#files + 1] = rel end
        end
    end
    return files
end

test("manifeste de telmi2flam lu (scripts et icones)", function()
    local files = manifest()
    expect_ge(#files, 40, "nombre d'entrees du manifeste")
end)

test("runtime du skill identique a celui de telmi2flam (octets)", function()
    local missing, differ = {}, {}
    for _, rel in ipairs(manifest()) do
        if EXCEPTIONS[rel] == nil then
            local a = slurp(REF .. rel)
            local b = slurp(SKILL .. rel)
            expect_true(a ~= nil, "fichier du manifeste absent de telmi2flam : " .. rel)
            if b == nil then
                missing[#missing + 1] = rel
            elseif a ~= b then
                differ[#differ + 1] = rel
            end
        end
    end
    if #missing > 0 or #differ > 0 then
        error(string.format("absents du skill : [%s] ; differents : [%s] -> recopier "
            .. "tools/telmi2flam/runtime dans skills/flam-homebrew/assets/runtime (memes octets)",
            table.concat(missing, ", "), table.concat(differ, ", ")))
    end
end)

print(string.format("Results: %d passed, %d failed", TEST_PASS, TEST_FAIL))
