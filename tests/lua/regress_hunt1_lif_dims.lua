-- regress_hunt1_lif_dims.lua — Non-regression : vignette LIF 2048..4096
-- lv_img_header_t (LVGL 8.3) ne stocke w et h que sur 11 bits (max 2047).
-- lif_decode_* acceptait jusqu'a 4096 : avec w = 2048, header.w valait 0
-- (division par zero dans le navigateur d'histoires), et au-dela les
-- dimensions etaient tronquees.
-- Comportement sur attendu : lv.img_src.load rend nil pour w ou h > 2047,
-- et une image de 2047 garde ses vraies dimensions.
-- ATTENDU : ECHOUE AVANT LE CORRECTIF (get_width rend 0 ou une valeur
-- tronquee au lieu de nil).
-- Lance seul (processus separe) par test_run.bat.

require("test_helpers")
print("--- regress_hunt1_lif_dims ---")

local function be32(n)
    return string.char((n >> 24) & 0xFF, (n >> 16) & 0xFF, (n >> 8) & 0xFF, n & 0xFF)
end

-- LIF minimal : en-tete, un pixel RGB565 puis un run, marqueur de fin.
-- Les pixels non couverts par le payload restent a zero (decodeur).
local function write_lif(path, w, h)
    local f = assert(io.open(path, "wb"))
    f:write("LIFF", be32(w), be32(h), string.char(0xA2),
            string.char(0xFE, 0x1F, 0x00, 0xC0 | 0x3F),
            string.rep("\0", 7), string.char(1))
    f:close()
end

local dir = os.getenv("TEMP") or os.getenv("TMP") or "."
local function tmp(name) return dir .. "/flam_regress_hunt1_" .. name .. ".lif" end

local created = {}
local function make(name, w, h)
    local p = tmp(name)
    write_lif(p, w, h)
    created[#created + 1] = p
    return p
end

test("w = 2048 refuse (header.w serait 0)", function()
    local dsc = lv.img_src.load(make("w2048", 2048, 1))
    expect_nil(dsc, "2048x1 doit etre refuse")
end)

test("h = 2048 refuse (header.h serait 0)", function()
    local dsc = lv.img_src.load(make("h2048", 1, 2048))
    expect_nil(dsc, "1x2048 doit etre refuse")
end)

test("w = 3000 et 4096 refuses (dimensions tronquees)", function()
    expect_nil(lv.img_src.load(make("w3000", 3000, 2)), "3000x2")
    expect_nil(lv.img_src.load(make("w4096", 4096, 1)), "4096x1")
end)

test("2047 accepte avec les vraies dimensions", function()
    local dsc = lv.img_src.load(make("w2047", 2047, 3))
    expect_true(dsc ~= nil, "2047x3 doit etre decode")
    expect_eq(lv.img_src.get_width(dsc), 2047, "largeur")
    expect_eq(lv.img_src.get_height(dsc), 3, "hauteur")
    dsc = nil
    collectgarbage()
end)

test("petite vignette 64x64 toujours decodee", function()
    local dsc = lv.img_src.load(make("w64", 64, 64))
    expect_true(dsc ~= nil, "64x64")
    expect_eq(lv.img_src.get_width(dsc), 64, "largeur")
end)

for _, p in ipairs(created) do os.remove(p) end
