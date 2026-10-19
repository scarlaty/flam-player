"""Genere les images PNG de "Capi et la riviere" dans <projet>/img/.

Usage : python make_images.py <dossier_projet>
Dessin vectoriel Pillow en sur-echantillonnage x4 puis reduction (bords doux).
Palette douce et coloree ; sprites paysage ou carres (<= 72 px).
"""
import math
import random
import sys
from pathlib import Path

from PIL import Image, ImageDraw

K = 4  # sur-echantillonnage

# Palette
WATER = (86, 196, 214)
WATER_DEEP = (60, 168, 196)
WATER_LIGHT = (170, 232, 238)
BANK = (118, 186, 92)
BANK_DARK = (78, 148, 70)
SAND = (236, 214, 150)
FUR = (176, 118, 72)
FUR_DARK = (128, 82, 50)
FUR_LIGHT = (212, 160, 110)
RING = (255, 120, 150)
RING_STRIPE = (255, 246, 236)
ORANGE = (255, 150, 30)
ORANGE_DARK = (226, 108, 18)
LEAF = (70, 170, 80)
INK = (60, 40, 30)

RIVER_X0, RIVER_X1 = 44, 276  # doit correspondre a capi_1_0_0.lua


def canvas(w, h):
    return Image.new("RGBA", (w * K, h * K), (0, 0, 0, 0))


def down(im, w, h):
    return im.resize((w, h), Image.LANCZOS)


def S(*v):
    """Coordonnees logiques -> sur-echantillonnees."""
    return tuple(int(round(x * K)) for x in v)


# ---------------------------------------------------------------------------
def mandarine(size=22):
    im = canvas(size, size)
    d = ImageDraw.Draw(im)
    s = size
    d.ellipse(S(1, 3, s - 1, s - 1), fill=ORANGE, outline=ORANGE_DARK, width=K)
    d.ellipse(S(s * 0.28, s * 0.32, s * 0.48, s * 0.50), fill=(255, 200, 120))
    # feuille + queue
    d.line(S(s / 2, 5, s / 2 + 1, 1), fill=(90, 70, 30), width=K)
    d.ellipse(S(s / 2, 0, s / 2 + s * 0.36, s * 0.22), fill=LEAF)
    return down(im, s, s)


def capi(size=44, mouth_open=False):
    """Capi vu de dessus sur sa bouee, museau vers le haut, mandarine sur le crane.

    Dessine dans un repere logique 48x48 puis reduit a `size` (net a toute taille).
    """
    L = 48
    f = size * K / L                                   # unite logique -> pixels 4x
    im = Image.new("RGBA", (size * K, size * K), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)

    def box(x0, y0, x1, y1):
        return tuple(int(round(v * f)) for v in (x0, y0, x1, y1))

    def ell(cx, cy, rx, ry, **kw):
        if "width" in kw:
            kw["width"] = max(1, int(round(kw["width"] * f)))
        d.ellipse(box(cx - rx, cy - ry, cx + rx, cy + ry), **kw)

    cx, cy = 24, 27
    ell(cx, cy, 20, 18, fill=RING, outline=(214, 80, 112), width=1)        # bouee
    for a0 in (20, 110, 200, 290):
        d.pieslice(box(cx - 20, cy - 18, cx + 20, cy + 18), a0, a0 + 34, fill=RING_STRIPE)
    ell(cx, cy, 20, 18, outline=(214, 80, 112), width=1)
    ell(cx - 7, cy - 9, 5, 2.5, fill=(255, 228, 232, 200))                 # reflet
    ell(cx, cy + 1, 11, 10, fill=WATER_DEEP)                               # trou
    ell(cx, cy + 2, 10, 12, fill=FUR, outline=FUR_DARK, width=1)           # corps
    ell(cx, cy + 4, 6, 7, fill=FUR_LIGHT)
    ell(cx - 10, cy - 5, 3.2, 2.6, fill=FUR, outline=FUR_DARK, width=1)    # pattes
    ell(cx + 10, cy - 5, 3.2, 2.6, fill=FUR, outline=FUR_DARK, width=1)
    ell(cx - 8, 15, 2.6, 2.4, fill=FUR_DARK)                               # oreilles
    ell(cx + 8, 15, 2.6, 2.4, fill=FUR_DARK)
    ell(cx, 12, 9, 8.5, fill=FUR, outline=FUR_DARK, width=1)               # tete
    d.rounded_rectangle(box(cx - 5.5, 1.5, cx + 5.5, 7.5), radius=int(3 * f), fill=FUR_DARK)
    if mouth_open:
        ell(cx, 2.5, 2.6, 2, fill=(210, 84, 84))                           # « glop ! »
    else:
        ell(cx - 2.2, 3.5, 1, 0.8, fill=INK)                               # narines
        ell(cx + 2.2, 3.5, 1, 0.8, fill=INK)
    for ex in (cx - 4.8, cx + 4.8):                                        # yeux
        ell(ex, 9, 1.6, 1.6, fill=INK)
        ell(ex - 0.5, 8.5, 0.6, 0.6, fill=(255, 255, 255))
    ell(cx - 7, 11.5, 1.5, 1, fill=(255, 150, 140, 170))                   # joues
    ell(cx + 7, 11.5, 1.5, 1, fill=(255, 150, 140, 170))
    ell(cx, 15.5, 4, 3.8, fill=ORANGE, outline=ORANGE_DARK, width=1)       # mandarine
    ell(cx - 1.4, 14.4, 1.2, 0.9, fill=(255, 214, 130))
    d.polygon([(cx * f, 12 * f), ((cx + 4) * f, 10.8 * f), ((cx + 2) * f, 13.2 * f)], fill=LEAF)
    return down(im, size, size)


def shadow(w=40, h=14):
    im = canvas(w, h)
    d = ImageDraw.Draw(im)
    d.ellipse(S(0, 0, w, h), fill=(20, 80, 100, 110))
    return down(im, w, h)


def rock(w=36, h=30):
    im = canvas(w, h)
    d = ImageDraw.Draw(im)
    d.ellipse(S(0, h * 0.45, w, h), fill=(230, 250, 252, 200))  # ecume
    d.polygon([S(3, h - 6), S(6, 6), S(16, 1), S(29, 3), S(w - 2, 13), S(w - 4, h - 5),
               S(18, h - 2)], fill=(150, 150, 168), outline=(104, 104, 124))
    d.polygon([S(9, 8), S(16, 4), S(25, 6), S(16, 12)], fill=(196, 196, 212))
    d.ellipse(S(22, 15, 28, 20), fill=(124, 124, 142))
    return down(im, w, h)


def log(w=72, h=24):
    im = canvas(w, h)
    d = ImageDraw.Draw(im)
    d.rounded_rectangle(S(4, 3, w - 4, h - 3), radius=9 * K, fill=(164, 108, 60),
                        outline=(112, 70, 36), width=K)
    for y in (8, 13, 17):
        d.line(S(14, y, w - 18, y), fill=(130, 84, 44), width=K)
    d.ellipse(S(w - 14, 3, w - 2, h - 3), fill=(222, 176, 120), outline=(112, 70, 36), width=K)
    d.ellipse(S(w - 10, 8, w - 6, h - 8), outline=(170, 120, 70), width=K)
    # petite branche feuillue
    d.line(S(24, 4, 20, 0), fill=(112, 70, 36), width=K)
    d.ellipse(S(16, 0, 23, 4), fill=LEAF)
    return down(im, w, h)


def lily(w=36, h=34):
    im = canvas(w, h)
    d = ImageDraw.Draw(im)
    d.pieslice(S(1, 1, w - 1, h - 1), 300, 250 + 360, fill=(96, 190, 96),
               outline=(56, 140, 64), width=K)
    for a in (20, 90, 160, 230):
        r = math.radians(a)
        d.line(S(w / 2, h / 2, w / 2 + math.cos(r) * 13, h / 2 + math.sin(r) * 12),
               fill=(70, 156, 74), width=K)
    # fleur rose
    fx, fy = w / 2 + 3, h / 2 + 2
    for a in range(0, 360, 60):
        r = math.radians(a)
        d.ellipse(S(fx + math.cos(r) * 4 - 4, fy + math.sin(r) * 4 - 4,
                    fx + math.cos(r) * 4 + 4, fy + math.sin(r) * 4 + 4), fill=(255, 170, 200))
    d.ellipse(S(fx - 3, fy - 3, fx + 3, fy + 3), fill=(255, 226, 90))
    return down(im, w, h)


def splash(w=56, h=40):
    im = canvas(w, h)
    d = ImageDraw.Draw(im)
    rnd = random.Random(7)
    d.ellipse(S(6, 10, w - 6, h - 4), outline=(255, 255, 255, 230), width=3 * K)
    for _ in range(14):
        a = rnd.uniform(0, 2 * math.pi)
        rr = rnd.uniform(12, 24)
        x = w / 2 + math.cos(a) * rr
        y = h / 2 + math.sin(a) * rr * 0.7
        r = rnd.uniform(1.5, 3.5)
        d.ellipse(S(x - r, y - r, x + r, y + r), fill=(235, 252, 255, 240))
    d.ellipse(S(w / 2 - 7, h / 2 - 4, w / 2 + 7, h / 2 + 6), fill=(200, 240, 250, 200))
    return down(im, w, h)


def ripple(w=24, h=8):
    im = canvas(w, h)
    d = ImageDraw.Draw(im)
    pts = [S(x, 4 + 2.2 * math.sin(x / w * 2 * math.pi)) for x in range(0, w + 1, 2)]
    d.line(pts, fill=WATER_LIGHT + (210,), width=2 * K, joint="curve")
    return down(im, w, h)


def fern(w=40, h=32, flower=False):
    im = canvas(w, h)
    d = ImageDraw.Draw(im)
    for i, a in enumerate((-60, -30, 0, 30, 60, 90, 120, 150, 180, 210, 240)):
        r = math.radians(a)
        x, y = w / 2 + math.cos(r) * 15, h / 2 + math.sin(r) * 12
        d.line(S(w / 2, h / 2, x, y), fill=BANK_DARK, width=3 * K)
        d.ellipse(S(x - 5, y - 4, x + 5, y + 4), fill=(96, 176, 84) if i % 2 else LEAF)
    if flower:
        for (fx, fy, c) in ((w / 2 - 4, h / 2 - 3, (255, 140, 170)), (w / 2 + 6, h / 2 + 4, (255, 210, 80))):
            for a in range(0, 360, 72):
                r = math.radians(a)
                d.ellipse(S(fx + math.cos(r) * 3 - 3, fy + math.sin(r) * 3 - 3,
                            fx + math.cos(r) * 3 + 3, fy + math.sin(r) * 3 + 3), fill=c)
            d.ellipse(S(fx - 2, fy - 2, fx + 2, fy + 2), fill=(255, 255, 230))
    return down(im, w, h)


def life(size=16):
    im = canvas(size, size)
    d = ImageDraw.Draw(im)
    d.ellipse(S(1, 1, size - 1, size - 1), fill=RING, outline=(214, 80, 112), width=K)
    for i in range(4):
        d.pieslice(S(1, 1, size - 1, size - 1), i * 90 + 20, i * 90 + 45, fill=RING_STRIPE)
    d.ellipse(S(5, 5, size - 5, size - 5), fill=(0, 0, 0, 0))
    # trou transparent : on efface
    hole = Image.new("L", im.size, 0)
    ImageDraw.Draw(hole).ellipse(S(5.5, 5.5, size - 5.5, size - 5.5), fill=255)
    im.putalpha(Image.composite(Image.new("L", im.size, 0), im.getchannel("A"), hole))
    return down(im, size, size)


# ---------------------------------------------------------------------------
def background(w=320, h=212):
    """Decor fixe : berges de jungle a gauche/droite, riviere au centre."""
    im = Image.new("RGBA", (w * K, h * K), BANK + (255,))
    d = ImageDraw.Draw(im)
    rnd = random.Random(42)
    # riviere en degrade horizontal (plus profonde au centre)
    for x in range(RIVER_X0 * K, RIVER_X1 * K):
        t = abs((x / K - (RIVER_X0 + RIVER_X1) / 2) / ((RIVER_X1 - RIVER_X0) / 2))
        c = tuple(int(WATER_DEEP[i] * (1 - t) + WATER[i] * t) for i in range(3))
        d.line([(x, 0), (x, h * K)], fill=c)
    # bords sableux ondules
    for side in (0, 1):
        pts = []
        for y in range(0, h + 8, 6):
            off = 3 * math.sin(y / 17.0) + 2 * math.sin(y / 7.0)
            x = (RIVER_X0 + off) if side == 0 else (RIVER_X1 + off)
            pts.append((x, y))
        for (x, y) in pts:
            d.ellipse(S(x - 5, y - 5, x + 5, y + 5), fill=SAND)
    # touffes d'herbe sur les berges
    for _ in range(70):
        side = rnd.choice((0, 1))
        x = rnd.uniform(2, RIVER_X0 - 8) if side == 0 else rnd.uniform(RIVER_X1 + 8, w - 2)
        y = rnd.uniform(0, h)
        r = rnd.uniform(2, 5)
        d.ellipse(S(x - r, y - r, x + r, y + r), fill=rnd.choice((BANK_DARK, (138, 200, 104), LEAF)))
    return im.resize((w, h), Image.LANCZOS)


def title_image(w=320, h=212):
    im = background(w, h)
    rnd = random.Random(3)
    for (x, y) in ((54, 128), (236, 62), (244, 132)):
        im.alpha_composite(lily(), (x, y))
    for (x, y) in ((66, 80), (250, 104), (76, 150), (226, 162)):
        im.alpha_composite(mandarine(), (x, y))
    for _ in range(6):
        im.alpha_composite(ripple(), (rnd.randint(60, 240), rnd.randint(10, 200)))
    big = capi(110)
    im.alpha_composite(big, (105, 50))
    return im


def thumbnail():
    """Vignette 104x136 (portrait, format des vignettes officielles)."""
    t = Image.new("RGBA", (104, 136), WATER + (255,))
    d = ImageDraw.Draw(t)
    d.rectangle((0, 0, 10, 136), fill=BANK)
    d.rectangle((93, 0, 104, 136), fill=BANK)
    t.alpha_composite(ripple(), (18, 12))
    t.alpha_composite(ripple(), (50, 108))
    t.alpha_composite(mandarine(), (62, 16))
    t.alpha_composite(lily().resize((28, 26), Image.LANCZOS), (16, 100))
    c = capi(70)
    t.alpha_composite(c, ((104 - c.size[0]) // 2, 44))
    return t


def main():
    out = Path(sys.argv[1] if len(sys.argv) > 1 else ".") / "img"
    out.mkdir(parents=True, exist_ok=True)
    items = {
        "bg.png": background(),
        "title.png": title_image(),
        "capi.png": capi(),
        "capi_gob.png": capi(mouth_open=True),
        "capi_jump.png": capi(56),
        "shadow.png": shadow(),
        "rock.png": rock(),
        "log.png": log(),
        "lily.png": lily(),
        "mandarine.png": mandarine(),
        "splash.png": splash(),
        "ripple.png": ripple(),
        "fern.png": fern(),
        "flower.png": fern(flower=True),
        "life.png": life(),
        "thumbnail.png": thumbnail(),
    }
    for name, im in items.items():
        im.save(out / name)
    print("ok", len(items), "images ->", out)


if __name__ == "__main__":
    main()
