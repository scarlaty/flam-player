-- audio_helpers.lua — outils des tests audio reels (cible flam-test-audio)
--
-- Genere des MP3 synthetiques (MPEG-1 Layer III 128 kbit/s, trames de
-- silence) et leur table .mp3map, sans dependance externe. Les fichiers
-- sont ecrits dans le dossier courant (dossier de build, cf. test_run.bat)
-- et supprimes par cleanup().

local H = {}

-- Rend test_helpers.lua (tests/lua) accessible depuis tests/audio
local src = debug.getinfo(1, "S").source
local here = src:sub(1, 1) == "@" and src:sub(2):match("^(.*)[/\\]") or "."
package.path = here .. "/../lua/?.lua;" .. package.path
H.dir = here
H.root = here .. "/../.."

-- octet 2 de l'en-tete : bitrate 128k (1001) + frequence
local SR_BITS = { [44100] = 0x90, [48000] = 0x94, [32000] = 0x98 }

H.SAMPLES_PER_FRAME = 1152
H.UNITS_PER_S = 88200          -- unite interne du .mp3map (2 x 44100)

local created = {}

local function frame_bytes(sr)
    return math.floor(144 * 128000 / sr)
end

-- Duree (s) de n trames a la frequence sr
function H.duration(n, sr)
    return n * H.SAMPLES_PER_FRAME / sr
end

-- Nombre de trames pour une duree donnee
function H.frames_for(seconds, sr)
    return math.ceil(seconds * sr / H.SAMPLES_PER_FRAME)
end

-- Ecrit <name> (MP3) et, si with_map, <name>map. Retourne le chemin.
function H.make_mp3(name, sr, nframes, with_map)
    local b2 = assert(SR_BITS[sr], "frequence non geree " .. tostring(sr))
    local fb = frame_bytes(sr)
    local frame = string.char(0xFF, 0xFB, b2, 0x00) .. string.rep("\0", fb - 4)
    local f = assert(io.open(name, "wb"))
    f:write(string.rep(frame, nframes))
    f:close()
    created[#created + 1] = name
    if with_map then
        local upf = H.SAMPLES_PER_FRAME * H.UNITS_PER_S // sr   -- unites / trame
        local parts = { string.pack("<I4I4I4", nframes * H.SAMPLES_PER_FRAME * H.UNITS_PER_S // sr, 0, 0) }
        for i = 0, nframes - 1 do
            parts[#parts + 1] = string.pack("<I4I4", i * fb, i * upf)
        end
        local m = assert(io.open(name .. "map", "wb"))
        m:write(table.concat(parts))
        m:close()
        created[#created + 1] = name .. "map"
    end
    return name
end

function H.make_empty(name)
    local f = assert(io.open(name, "wb"))
    f:close()
    created[#created + 1] = name
    return name
end

function H.cleanup()
    for _, p in ipairs(created) do os.remove(p) end
    created = {}
end

-- Enregistreur de callbacks audio : rec.cb(status, t)
function H.recorder()
    local rec = { events = {} }
    rec.cb = function(status, t)
        rec.events[#rec.events + 1] = { s = status, t = t }
    end
    function rec.count(s)
        local n = 0
        for _, e in ipairs(rec.events) do if e.s == s then n = n + 1 end end
        return n
    end
    function rec.last(s)
        for i = #rec.events, 1, -1 do
            if rec.events[i].s == s then return rec.events[i] end
        end
    end
    function rec.reset() rec.events = {} end
    return rec
end

-- Pompe (test_tick) jusqu'a pred() ou max_ticks. Retourne true si pred().
function H.tick_until(pred, max_ticks)
    for _ = 1, max_ticks do
        if pred() then return true end
        test_tick(1)
    end
    return pred()
end

function H.near(a, b, tol)
    return type(a) == "number" and math.abs(a - b) <= tol
end

return H
