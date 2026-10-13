"""Genere la musique et les bruitages de "Capi et la riviere" dans <projet>/sounds/.

Usage : python make_sounds.py <dossier_projet>
Synthese numpy (marimba, basse, maracas, fanfare) -> WAV temporaire -> MP3 44,1 kHz
mono 96 kbps via ffmpeg (doit etre dans le PATH).

Musique : title (calme, ecran titre et fin) en 4 segments title_1..4.mp3 enchaines
par le jeu. Pas de musique pendant la partie (voir README, section Son).
Bruitages : glop1/2/3 (mandarine, plus vif a chaque palier), combo, hop1/2/3 (saut),
splash (collision), fanfare (fin de partie).
"""
import shutil
import subprocess
import sys
import tempfile
import wave
from pathlib import Path

import numpy as np

SR = 44100
rng = np.random.default_rng(1234)


def t_axis(dur):
    return np.arange(int(SR * dur)) / SR


def env_exp(dur, decay):
    return np.exp(-t_axis(dur) * decay)


def adsr(n, a=0.01, r=0.05):
    e = np.ones(n)
    na, nr = min(n, int(a * SR)), min(n, int(r * SR))
    if na:
        e[:na] = np.linspace(0, 1, na)
    if nr:
        e[-nr:] *= np.linspace(1, 0, nr)
    return e


def midi_hz(m):
    return 440.0 * 2 ** ((m - 69) / 12)


def marimba(freq, dur):
    t = t_axis(dur)
    s = np.sin(2 * np.pi * freq * t) + 0.35 * np.sin(2 * np.pi * freq * 4 * t) * np.exp(-t * 30)
    return s * np.exp(-t * 7) * adsr(len(t), 0.003, 0.02)


def bass(freq, dur):
    t = t_axis(dur)
    tri = 2 / np.pi * np.arcsin(np.sin(2 * np.pi * freq * t))
    return tri * np.exp(-t * 3) * adsr(len(t), 0.005, 0.04)


def shaker(dur=0.06):
    n = int(SR * dur)
    noise = rng.uniform(-1, 1, n)
    hp = np.diff(noise, prepend=0)  # passe-haut grossier
    return hp * np.exp(-np.arange(n) / SR * 60)


def thump(dur=0.14):
    t = t_axis(dur)
    f = 110 * np.exp(-t * 18) + 45
    return np.sin(2 * np.pi * np.cumsum(f) / SR) * np.exp(-t * 22)


def brass(freq, dur):
    t = t_axis(dur)
    s = sum((1 / k) * np.sin(2 * np.pi * freq * k * t) for k in range(1, 7))
    vib = 1 + 0.004 * np.sin(2 * np.pi * 5.5 * t)
    s = sum((1 / k) * np.sin(2 * np.pi * freq * k * t * vib) for k in range(1, 7))
    return s * adsr(len(t), 0.03, 0.08) * 0.5


def mix_at(buf, sig, start_s, gain=1.0):
    i = int(start_s * SR)
    j = min(len(buf), i + len(sig))
    if i < len(buf):
        buf[i:j] += sig[: j - i] * gain


def normalize(x, peak=0.85):
    m = np.max(np.abs(x)) or 1.0
    return x / m * peak


# ---------------------------------------------------------------------------
# Air de la riviere : 8 mesures en 4/4, croches. 0 = silence. Do majeur.
MELODY = [
    72, 0, 76, 79, 76, 0, 72, 74,   76, 76, 74, 72, 74, 0, 67, 0,
    72, 0, 76, 79, 81, 79, 76, 74,  72, 74, 76, 74, 72, 0, 0, 0,
    77, 0, 81, 84, 81, 0, 77, 79,   81, 79, 77, 76, 74, 0, 67, 0,
    72, 74, 76, 79, 76, 74, 72, 74, 76, 0, 74, 0, 72, 0, 0, 0,
]
BASS = [48, 43, 48, 43, 53, 48, 43, 48]   # une fondamentale par mesure (do-sol-fa)


def tune(bpm, calm=False):
    eighth = 60.0 / bpm / 2
    bars = 8
    dur = bars * 8 * eighth
    buf = np.zeros(int(SR * dur) + SR)
    for i, m in enumerate(MELODY):
        if m:
            mix_at(buf, marimba(midi_hz(m), eighth * 1.8), i * eighth, 0.55 if calm else 0.6)
    for b, m in enumerate(BASS):
        bar0 = b * 8 * eighth
        for beat in range(4):
            note = m if beat % 2 == 0 else m + 7
            mix_at(buf, bass(midi_hz(note), eighth * 1.9), bar0 + beat * 2 * eighth, 0.5)
            if not calm:
                mix_at(buf, thump(), bar0 + beat * 2 * eighth, 0.35 if beat % 2 == 0 else 0.2)
    if not calm:
        for i in range(bars * 8):
            mix_at(buf, shaker(), i * eighth, 0.18 if i % 2 else 0.1)
    buf = buf[: int(SR * dur)]
    # petit fondu aux bornes pour boucler sans clic
    f = int(0.01 * SR)
    buf[:f] *= np.linspace(0, 1, f)
    buf[-f:] *= np.linspace(1, 0, f)
    return normalize(buf, 0.7)


def glop(level):
    k = 1 + 0.12 * (level - 1)          # plus aigu et plus court a chaque palier
    dur = 0.22 / k
    t = t_axis(dur)
    f = (320 + 900 * (t / dur) ** 0.6) * k
    s = np.sin(2 * np.pi * np.cumsum(f) / SR) * adsr(len(t), 0.005, 0.05)
    out = np.zeros(int(SR * (dur + 0.2)))
    mix_at(out, s, 0, 0.8)
    mix_at(out, marimba(midi_hz(88 + 2 * (level - 1)), 0.2), dur * 0.7, 0.6)
    return normalize(out)


def combo():
    out = np.zeros(int(SR * 0.6))
    for i, m in enumerate((84, 88, 91, 96)):
        mix_at(out, marimba(midi_hz(m), 0.25), i * 0.08, 0.7)
    return normalize(out)


def hop(level):
    k = 1 + 0.15 * (level - 1)
    dur = 0.32 / k
    t = t_axis(dur)
    f = (180 + 520 * (t / dur)) * k * (1 + 0.06 * np.sin(2 * np.pi * 28 * t))
    s = np.sin(2 * np.pi * np.cumsum(f) / SR) * adsr(len(t), 0.005, 0.08)
    return normalize(s, 0.75)


def splash():
    dur = 0.55
    n = int(SR * dur)
    noise = rng.uniform(-1, 1, n)
    # bruit "eau" : passe-bas glissant par moyenne mobile
    lp = np.convolve(noise, np.ones(6) / 6, mode="same")
    s = lp * np.exp(-np.arange(n) / SR * 9)
    t = t_axis(0.35)
    f = 700 * np.exp(-t * 6) + 120          # "plouf" qui descend
    bloop = np.sin(2 * np.pi * np.cumsum(f) / SR) * adsr(len(t), 0.005, 0.1)
    out = np.zeros(n + SR // 10)
    mix_at(out, s, 0, 0.8)
    mix_at(out, bloop, 0.04, 0.6)
    # deux gouttes rigolotes
    for i, m in enumerate((91, 86)):
        mix_at(out, marimba(midi_hz(m), 0.12), 0.32 + i * 0.09, 0.35)
    return normalize(out)


def fanfare():
    out = np.zeros(int(SR * 3.2))
    for i, m in enumerate((60, 64, 67, 72)):
        mix_at(out, brass(midi_hz(m), 0.22), i * 0.18, 0.6)
    for m in (60, 64, 67, 72, 76):
        mix_at(out, brass(midi_hz(m), 1.9), 0.78, 0.35)
    for i in range(10):
        mix_at(out, marimba(midi_hz(84 + (i % 5) * 3), 0.3), 0.8 + i * 0.11, 0.25)
    for i in range(4):
        mix_at(out, thump(), i * 0.18, 0.4)
    f = int(0.3 * SR)
    out[-f:] *= np.linspace(1, 0, f)
    return normalize(out)


NCHUNK = 4


def split(sig, n):
    """Coupe une musique en n segments egaux (frontieres de mesure), petits fondus."""
    size = len(sig) // n
    f = int(0.004 * SR)
    parts = []
    for k in range(n):
        p = sig[k * size:(k + 1) * size].copy()
        p[:f] *= np.linspace(0, 1, f)
        p[-f:] *= np.linspace(1, 0, f)
        parts.append(p)
    return parts


# ---------------------------------------------------------------------------
def write_mp3(path, sig):
    pcm = (np.clip(sig, -1, 1) * 32767).astype("<i2")
    with tempfile.TemporaryDirectory() as td:
        wav = Path(td) / "x.wav"
        with wave.open(str(wav), "wb") as w:
            w.setnchannels(1)
            w.setsampwidth(2)
            w.setframerate(SR)
            w.writeframes(pcm.tobytes())
        subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-i", str(wav), "-ar", "44100",
                        "-codec:a", "libmp3lame", "-b:a", "96k", "-map_metadata", "-1",
                        "-id3v2_version", "0", str(path)], check=True)


def main():
    if shutil.which("ffmpeg") is None:
        sys.exit("ffmpeg introuvable dans le PATH")
    out = Path(sys.argv[1] if len(sys.argv) > 1 else ".") / "sounds"
    out.mkdir(parents=True, exist_ok=True)
    items = {
        "glop1.mp3": glop(1), "glop2.mp3": glop(2), "glop3.mp3": glop(3),
        "combo.mp3": combo(),
        "hop1.mp3": hop(1), "hop2.mp3": hop(2), "hop3.mp3": hop(3),
        "splash.mp3": splash(),
        "fanfare.mp3": fanfare(),
    }
    # Musiques en 4 segments de 2 mesures : le jeu enchaine les segments et,
    # apres un bruitage, reprend au segment suivant (pas de seek sur device).
    # Pas de musique pendant la partie (arreter une piste fige l'image sur la
    # Flam) : seulement la musique du titre / de la fin.
    for base, sig in (("title", tune(96, calm=True)),):
        for k, part in enumerate(split(sig, NCHUNK), 1):
            items["%s_%d.mp3" % (base, k)] = part
    for old in out.glob("*.mp3"):
        if old.name not in items:
            old.unlink()                     # anciens fichiers (ex. music1.mp3)
    for name, sig in items.items():
        write_mp3(out / name, sig)
        print(f"{name:12s} {len(sig) / SR:5.2f} s")
    print("ok", len(items), "sons ->", out)


if __name__ == "__main__":
    main()
