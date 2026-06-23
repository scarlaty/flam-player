"""
lif.py — Codec LIF (Lunii Image File Format) pour le convertisseur TELMI -> FLAM.

Le decodeur de reference du player (src/formats/lif_decoder.c) definit le format.
Ce module fournit :
  - encode(rgba, w, h) -> bytes        : encode une image RGBA8888 en .lif
  - decode(data)       -> (rgba, w, h) : decodeur miroir du C (pour self-test)

Format fichier :
  [4]  magic "liff"
  [4]  width  (uint32 big-endian)
  [4]  height (uint32 big-endian)
  [1]  channels = 0xA2 (RGBA)
  [N]  payload (opcodes, voir ci-dessous)
  [8]  end marker = 7x 0x00 + 0x01

Espace couleur interne : RGB565 (r 5 bits 0..31, g 6 bits 0..63, b 5 bits 0..31)
+ alpha 8 bits. Le decodeur etend r*8, g*4, b*8 vers 8 bits.

Opcodes (1er octet b1) :
  0xFE          : couleur RGB565 sans alpha (2 octets lo,hi suivent), alpha inchange
  0xFF          : couleur RGB565 + alpha   (3 octets lo,hi,a suivent)
  tag 0x00 (b1 & 0xC0 == 0x00) : index cache (b1 & 0x3F)
  tag 0x40 : petit delta (db,dg,dr de 2 bits chacun, valeur = bits-2), alpha inchange
  tag 0x80 : delta vert etendu (1 octet b2 suit), alpha inchange
  tag 0xC0 : run-length (run = b1 & 0x3F) -> repete la couleur courante run+1 fois
"""

MAGIC = b"liff"
CHANNEL_RGBA = 0xA2
END_MARKER = b"\x00\x00\x00\x00\x00\x00\x00\x01"


def _hash(r, g, b):
    return (r * 7 + g * 5 + b * 3) & 0x3F


def _mod32(v):
    return ((v % 32) + 32) % 32


def _mod64(v):
    return ((v % 64) + 64) % 64


def _quantize(r8, g8, b8):
    """8 bits -> composantes RGB565, round-trip exact avec expand5/expand6 du C."""
    r = min(31, (r8 + 4) // 8)
    g = min(63, (g8 + 2) // 4)
    b = min(31, (b8 + 4) // 8)
    return r, g, b


def _pack565(r, g, b):
    lo = (b & 0x1F) | ((g & 0x07) << 5)
    hi = ((g >> 3) & 0x07) | ((r & 0x1F) << 3)
    return lo, hi


def encode(rgba, w, h):
    """rgba : bytes/bytearray RGBA8888 row-major (len == w*h*4). Retourne le .lif."""
    n = w * h
    assert len(rgba) == n * 4, "taille buffer RGBA incoherente"

    # Pre-quantifier tous les pixels en (r5, g6, b5, a)
    px = [None] * n
    for i in range(n):
        o = i * 4
        r, g, b = _quantize(rgba[o], rgba[o + 1], rgba[o + 2])
        px[i] = (r, g, b, rgba[o + 3])

    out = bytearray()
    # etat decodeur initial
    cr, cg, cb, ca = 0, 0, 0, 255
    cache = [(0, 0, 0, 0)] * 64

    i = 0

    # 1er pixel TOUJOURS en couleur+alpha explicite (0xFF) : n'assume pas l'etat
    # initial du decodeur (cr/cg/cb/ca). Certains decodeurs initialisent l'alpha
    # a 0 ; sans ce 0xFF, une image opaque encodee via 0xFE/RLE devient
    # transparente (donc noire) a l'affichage.
    if n > 0:
        tr, tg, tb, ta = px[0]
        lo, hi = _pack565(tr, tg, tb)
        out.append(0xFF)
        out.append(lo)
        out.append(hi)
        out.append(ta & 0xFF)
        cr, cg, cb, ca = tr, tg, tb, ta
        cache[_hash(cr, cg, cb)] = (cr, cg, cb, ca)
        i = 1

    while i < n:
        t = px[i]
        tr, tg, tb, ta = t

        # --- RLE : pixels consecutifs identiques a la couleur courante ---
        if t == (cr, cg, cb, ca):
            # run-1 ne doit pas valoir 62 (0xFE) ni 63 (0xFF) : ces octets sont
            # interpretes comme opcodes couleur par le decodeur. Max run = 62.
            run = 1
            while i + run < n and px[i + run] == t and run < 62:
                run += 1
            out.append(0xC0 | (run - 1))
            # le cache est reecrit avec la couleur courante (inchangee)
            cache[_hash(cr, cg, cb)] = (cr, cg, cb, ca)
            i += run
            continue

        # --- Cache hit ---
        idx = _hash(tr, tg, tb)
        if cache[idx] == t:
            out.append(idx)  # tag 0x00 | idx
        else:
            emitted = False
            if ta == ca:
                # --- Petit delta (2 bits par canal, -2..+1) ---
                dr = dg = db = None
                for d in (-2, -1, 0, 1):
                    if _mod32(cr + d) == tr:
                        dr = d
                        break
                for d in (-2, -1, 0, 1):
                    if _mod64(cg + d) == tg:
                        dg = d
                        break
                for d in (-2, -1, 0, 1):
                    if _mod32(cb + d) == tb:
                        db = d
                        break
                if dr is not None and dg is not None and db is not None:
                    out.append(0x40 | ((db + 2) << 4) | ((dg + 2) << 2) | (dr + 2))
                    emitted = True

                if not emitted:
                    # --- Delta vert etendu ---
                    vg = None
                    for v in range(-32, 32):
                        if _mod64(cg + v) == tg:
                            vg = v
                            break
                    if vg is not None:
                        lo = (tr - cr - vg + 8) % 32
                        hi = (tb - cb - vg + 8) % 32
                        if 0 <= lo <= 15 and 0 <= hi <= 15:
                            out.append(0x80 | ((vg + 32) & 0x3F))
                            out.append(((hi & 0x0F) << 4) | (lo & 0x0F))
                            emitted = True

                if not emitted:
                    # --- Couleur pleine sans alpha (0xFE) ---
                    lo, hi = _pack565(tr, tg, tb)
                    out.append(0xFE)
                    out.append(lo)
                    out.append(hi)
                    emitted = True
            else:
                # --- Couleur pleine + alpha (0xFF) ---
                lo, hi = _pack565(tr, tg, tb)
                out.append(0xFF)
                out.append(lo)
                out.append(hi)
                out.append(ta & 0xFF)
                emitted = True

        # mise a jour etat + cache (le decodeur le fait a chaque pixel ecrit)
        cr, cg, cb, ca = tr, tg, tb, ta
        cache[_hash(cr, cg, cb)] = (cr, cg, cb, ca)
        i += 1

    header = bytearray()
    header += MAGIC
    header += w.to_bytes(4, "big")
    header += h.to_bytes(4, "big")
    header.append(CHANNEL_RGBA)
    return bytes(header) + bytes(out) + END_MARKER


# ---------------------------------------------------------------------------
# Decodeur de reference (miroir exact de lif_decoder.c) — pour self-test
# ---------------------------------------------------------------------------

def _expand5(v):
    return (v * 256) // 32


def _expand6(v):
    return (v * 256) // 64


def decode(data):
    assert data[0:4] in (b"liff", b"LIFF"), "magic LIF invalide"
    w = int.from_bytes(data[4:8], "big")
    h = int.from_bytes(data[8:12], "big")
    total = w * h
    payload = data[13:len(data) - 8]
    rgba = bytearray(total * 4)

    cr, cg, cb, ca = 0, 0, 0, 255
    run = 0
    cache = [(0, 0, 0, 0)] * 64
    p = 0
    px = 0
    plen = len(payload)

    while px < total:
        if run > 0:
            run -= 1
        elif p < plen:
            b1 = payload[p]; p += 1
            if b1 == 0xFE:
                lo = payload[p]; hi = payload[p + 1]; p += 2
                cb = lo & 0x1F
                cg = ((lo >> 5) & 0x07) | ((hi << 3) & 0x38)
                cr = (hi >> 3) & 0x1F
            elif b1 == 0xFF:
                lo = payload[p]; hi = payload[p + 1]; ca = payload[p + 2]; p += 3
                cb = lo & 0x1F
                cg = ((lo >> 5) & 0x07) | ((hi << 3) & 0x38)
                cr = (hi >> 3) & 0x1F
            else:
                tag = b1 & 0xC0
                if tag == 0x00:
                    idx = b1 & 0x3F
                    cr, cg, cb, ca = cache[idx]
                elif tag == 0x40:
                    db = ((b1 >> 4) & 0x03) - 2
                    dg = ((b1 >> 2) & 0x03) - 2
                    dr = (b1 & 0x03) - 2
                    cr = _mod32(cr + dr)
                    cg = _mod64(cg + dg)
                    cb = _mod32(cb + db)
                elif tag == 0x80:
                    b2 = payload[p]; p += 1
                    vg = (b1 & 0x3F) - 32
                    cb = _mod32(cb + vg - 8 + ((b2 >> 4) & 0x0F))
                    cg = _mod64(cg + vg)
                    cr = _mod32(cr + vg - 8 + (b2 & 0x0F))
                else:
                    run = b1 & 0x3F
        else:
            break

        cache[_hash(cr, cg, cb)] = (cr, cg, cb, ca)
        o = px * 4
        rgba[o] = _expand5(cr)
        rgba[o + 1] = _expand6(cg)
        rgba[o + 2] = _expand5(cb)
        rgba[o + 3] = ca
        px += 1

    return bytes(rgba), w, h


def quantize_rgba(rgba, w, h):
    """Retourne le buffer RGBA quantifie en RGB565 (ground-truth pour comparaison)."""
    n = w * h
    out = bytearray(n * 4)
    for i in range(n):
        o = i * 4
        r, g, b = _quantize(rgba[o], rgba[o + 1], rgba[o + 2])
        out[o] = _expand5(r)
        out[o + 1] = _expand6(g)
        out[o + 2] = _expand5(b)
        out[o + 3] = rgba[o + 3]
    return bytes(out)
