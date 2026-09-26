"""
mp3map.py — Generateur de table de seek .mp3map (format Lunii/Flam).

Format (cf. src/formats/mp3map_parser.c) :
  header 12 octets (little-endian) :
    [0..3]  total_units   = total_samples * 2   (cadence interne 88200 = 2*44100)
    [4..7]  id3_offset    = offset du 1er frame audio (apres tag ID3v2)
    [8..11] reserved      = 0
  puis N entrees de 8 octets (little-endian) :
    [0..3]  byte_offset   = offset absolu du frame dans le fichier
    [4..7]  unit_pos      = position cumulee en units (samples_avant * 2)

duration_s = total_units / 88200
"""

INTERNAL_RATE = 88200

# Tables MPEG audio
_BITRATES = {
    # version, layer -> liste indexee par bitrate_index (kbps)
    (3, 3): [0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448],   # MPEG1 L1
    (3, 2): [0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384],       # MPEG1 L2
    (3, 1): [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320],        # MPEG1 L3
    (2, 3): [0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256],       # MPEG2 L1
    (2, 2): [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160],            # MPEG2 L2
    (2, 1): [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160],            # MPEG2 L3
}
# MPEG2.5 (version code 0) reutilise les tables MPEG2
_BITRATES[(0, 1)] = _BITRATES[(2, 1)]
_BITRATES[(0, 2)] = _BITRATES[(2, 2)]
_BITRATES[(0, 3)] = _BITRATES[(2, 3)]

_SAMPLERATES = {
    3: [44100, 48000, 32000],   # MPEG1
    2: [22050, 24000, 16000],   # MPEG2
    0: [11025, 12000, 8000],    # MPEG2.5
}

# samples par frame : (version_is_mpeg1, layer) -> samples
def _samples_per_frame(version, layer):
    if layer == 1:   # Layer III
        return 1152 if version == 3 else 576
    if layer == 2:   # Layer II
        return 1152
    return 384       # Layer I


def _skip_id3v2(data):
    """Retourne l'offset du debut des frames audio (saute le tag ID3v2 s'il existe)."""
    if data[:3] == b"ID3" and len(data) >= 10:
        # syncsafe size sur 4 octets (7 bits utiles chacun)
        size = ((data[6] & 0x7F) << 21) | ((data[7] & 0x7F) << 14) | \
               ((data[8] & 0x7F) << 7) | (data[9] & 0x7F)
        footer = 10 if (data[5] & 0x10) else 0
        return 10 + size + footer
    return 0


def _parse_frame_header(data, off):
    """Retourne (frame_len, samples, samplerate) ou None si pas un header valide."""
    if off + 4 > len(data):
        return None
    b0, b1, b2, b3 = data[off], data[off + 1], data[off + 2], data[off + 3]
    if b0 != 0xFF or (b1 & 0xE0) != 0xE0:
        return None
    version = (b1 >> 3) & 0x03   # 0=2.5, 2=2, 3=1  (1 = reserve)
    layer = (b1 >> 1) & 0x03     # 1=L3, 2=L2, 3=L1 (0 = reserve)
    if version == 1 or layer == 0:
        return None
    bitrate_index = (b2 >> 4) & 0x0F
    sr_index = (b2 >> 2) & 0x03
    padding = (b2 >> 1) & 0x01
    if bitrate_index == 0 or bitrate_index == 15 or sr_index == 3:
        return None
    bitrate = _BITRATES[(version, layer)][bitrate_index] * 1000
    samplerate = _SAMPLERATES[version][sr_index]
    if bitrate == 0 or samplerate == 0:
        return None

    if layer == 3:  # Layer I
        frame_len = (12 * bitrate // samplerate + padding) * 4
    else:           # Layer II / III
        # ISO 11172-3 / 13818-3 : Layer II = 1152 samples/frame dans toutes les
        # versions -> coef 144. Layer III MPEG2/2.5 = 576 samples -> coef 72.
        coef = 144 if (version == 3 or layer == 2) else 72
        frame_len = coef * bitrate // samplerate + padding
    if frame_len <= 0:
        return None
    return frame_len, _samples_per_frame(version, layer), samplerate


# Nombre de frames successeurs valides exiges pour accepter un sync (resync).
_SYNC_CHAIN = 2


def _chain_ok(data, pos, hdr, depth=_SYNC_CHAIN):
    """Vrai si les `depth` frames suivants sont aussi des headers valides (ou si
    on atteint la fin des donnees). Evite de prendre un faux sync (octets 0xFFEx
    dans des donnees quelconques)."""
    for _ in range(depth):
        pos += hdr[0]
        if pos >= len(data):
            return True
        hdr = _parse_frame_header(data, pos)
        if hdr is None:
            return False
    return True


def _resync(data, pos):
    """Cherche le prochain header valide suivi de _SYNC_CHAIN headers valides.
    Retourne (pos, hdr) ou (len(data), None)."""
    n = len(data)
    while pos < n:
        pos = data.find(0xFF, pos)
        if pos < 0:
            return n, None
        hdr = _parse_frame_header(data, pos)
        if hdr is not None and _chain_ok(data, pos, hdr):
            return pos, hdr
        pos += 1
    return n, None


def build(data, info=None):
    """data : bytes du fichier MP3. Retourne (bytes_mp3map, duration_s, num_records).

    Suit le format du mp3map-tool de reference : 1 enregistrement par seconde,
    offsets absolus. unit_pos = position cumulee du frame en unites 88200 Hz,
    soit samples * 88200 / samplerate par frame (= 2304 par frame a 44,1 kHz
    MPEG1 Layer III : sortie identique au mp3map-tool de reference).

    info : dict optionnel rempli avec "samplerates" (ensemble des frequences
    rencontrees) et "frames" (nombre de frames). Leve ValueError si aucun frame
    MPEG valide n'est trouve.
    """
    start = _skip_id3v2(data)
    frames = []           # (byte_offset, unit_pos_cumule)
    lens = []             # (byte_offset, frame_len) pour le controle de couverture
    samplerates = set()
    n = len(data)

    # trouver le 1er frame valide (resync si necessaire)
    pos, hdr = _resync(data, start)
    first_frame = pos
    if hdr is None:
        raise ValueError("aucun frame MPEG audio valide")

    # Cumul exact en fraction : units = samples * 88200 / sr (entier a 44,1 kHz)
    units = 0.0
    while pos < n:
        hdr = _parse_frame_header(data, pos)
        if hdr is None:
            pos, hdr = _resync(data, pos + 1)
            if hdr is None:
                break
        frame_len, samples, sr = hdr
        samplerates.add(sr)
        frames.append((pos, int(round(units))))
        lens.append((pos, frame_len))
        units += samples * INTERNAL_RATE / sr
        pos += frame_len

    nf = len(frames)
    # Donnees majoritairement non MPEG (ex. WAV/OGG renomme en .mp3) : refuser
    covered = sum(min(fl, n - off) for off, fl in lens)
    if covered * 2 < n - start:
        raise ValueError("donnees non MPEG (%d frames couvrant %d%% du fichier)"
                         % (nf, 100 * covered // max(1, n - start)))
    # Duree moyenne d'un frame en units (2304 a 44,1 kHz MPEG1 L3)
    fu = units / nf
    # Correction de delai decodeur (gapless), empirique cf. mp3map-tool de reference :
    # x * 1152 units a 44,1 kHz = x demi-frames, on garde la meme duree en frames.
    x = 34 if nf > 100 else 22
    total_units = max(0, int(round(units - x * fu / 2)))
    num_records = total_units // INTERNAL_RATE

    out = bytearray()
    out += total_units.to_bytes(4, "little")
    out += first_frame.to_bytes(4, "little")
    out += (0).to_bytes(4, "little")

    # 1 enregistrement par seconde : 1er frame dont unit_pos >= k * 88200
    fi = 0
    for k in range(1, num_records + 1):
        target = k * INTERNAL_RATE
        while fi < nf - 1 and frames[fi][1] < target:
            fi += 1
        byte_off, upos = frames[fi]
        out += byte_off.to_bytes(4, "little")
        out += upos.to_bytes(4, "little")

    if info is not None:
        info["samplerates"] = samplerates
        info["frames"] = nf
    return bytes(out), total_units / INTERNAL_RATE, num_records


def build_file(path):
    with open(path, "rb") as f:
        data = f.read()
    return build(data)
