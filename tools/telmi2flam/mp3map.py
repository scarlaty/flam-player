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
        coef = 144 if version == 3 else 72
        if layer == 3:
            coef = 12
        frame_len = coef * bitrate // samplerate + padding
    if frame_len <= 0:
        return None
    return frame_len, _samples_per_frame(version, layer), samplerate


def build(data):
    """data : bytes du fichier MP3. Retourne (bytes_mp3map, duration_s, num_records).

    Suit le format du mp3map-tool de reference : 1 enregistrement par seconde,
    unit_pos = index_frame * 2304 (= 1152 samples * 2), offsets absolus.
    """
    start = _skip_id3v2(data)
    frames = []           # (byte_offset, unit_pos_cumule)
    total_samples = 0
    pos = start
    n = len(data)

    # trouver le 1er frame valide (resync si necessaire)
    while pos < n:
        if _parse_frame_header(data, pos) is not None:
            break
        pos += 1
    first_frame = pos

    while pos < n:
        hdr = _parse_frame_header(data, pos)
        if hdr is None:
            pos += 1
            continue
        frame_len, samples, _sr = hdr
        frames.append((pos, total_samples * 2))
        total_samples += samples
        pos += frame_len

    nf = len(frames)
    # Correction de delai decodeur (gapless), empirique cf. mp3map-tool de reference.
    x = 34 if nf > 100 else 22
    total_units = max(0, nf * 2 * 1152 - x * 1152)
    num_records = total_units // INTERNAL_RATE

    out = bytearray()
    out += total_units.to_bytes(4, "little")
    out += first_frame.to_bytes(4, "little")
    out += (0).to_bytes(4, "little")

    # 1 enregistrement par seconde : frame_index = ceil(k * 88200 / 2304)
    for k in range(1, num_records + 1):
        fi = -(-(k * INTERNAL_RATE) // 2304)   # ceil division
        if fi >= nf:
            fi = nf - 1
        byte_off = frames[fi][0]
        out += byte_off.to_bytes(4, "little")
        out += (fi * 2304).to_bytes(4, "little")

    return bytes(out), total_units / INTERNAL_RATE, num_records


def build_file(path):
    with open(path, "rb") as f:
        data = f.read()
    return build(data)
