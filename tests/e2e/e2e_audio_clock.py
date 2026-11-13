"""
Horloge audio sans peripherique (#575) : la lecture doit suivre l'horloge
murale, pas la vitesse du decodage.

    python tests/e2e/e2e_audio_clock.py --build <dossier contenant flam-player.exe>

flam-player est lance sur un script .lua qui joue un MP3 synthetique de ~2,5 s
(pilotes SDL dummy, --stdin-control) et trace chaque callback audio sur stderr :
  A) avec le peripherique "dummy" de SDL (temoin) : le "stop" arrive au plus tot
     apres ~80 % de la duree ;
  B) sans peripherique (FLAM_AUDIO_NO_DEVICE=1 : dev_id == 0, comme une session
     sans son ou une VM) : meme exigence. Avant la correction, SDL ne consommait
     rien, l'horloge avancait au rythme du decodage et le "stop" arrivait en
     une fraction de seconde (lecture ~10 fois trop rapide).
Mesure faite par l'horodatage de reception des lignes ; seule la borne basse est
testee (une machine chargee ne peut pas faire passer ce test a tort).
Code de sortie : 0 OK, 1 echec, 77 prerequis absents (flam-player).
"""
import argparse
import os
import subprocess
import sys
import tempfile
import threading
import time

SKIP = 77
FRAMES = 96                      # MPEG-1 Layer III 44,1 kHz, 1152 echantillons/trame
DURATION = FRAMES * 1152 / 44100.0   # ~2,51 s
FRAME_BYTES = 417                # 144 * 128000 / 44100, sans bourrage

SCRIPT = r'''audio.load(0, "clock.mp3", function(s, t)
  io.stderr:write("AUDIO_EV " .. s .. " " .. string.format("%.2f", t) .. "\n")
end)
audio.play()
'''


def check(cond, what, err=""):
    if not cond:
        print("[ECHEC] " + what)
        if err:
            print("--- sortie du player ---\n" + err)
        raise SystemExit(1)
    print("[OK] " + what)


def make_mp3(path):
    frame = bytes([0xFF, 0xFB, 0x90, 0x00]) + bytes(FRAME_BYTES - 4)
    with open(path, "wb") as f:
        f.write(frame * FRAMES)


def run(exe, tmp, no_device):
    """Lance le player, attend le 'stop'. Retourne (lignes horodatees, sortie, code)."""
    env = dict(os.environ)
    env.update({"SDL_VIDEODRIVER": "dummy", "SDL_AUDIODRIVER": "dummy",
                "SDL_RENDER_DRIVER": "software", "FLAM_SCREENSHOT_AUTO_MS": "0"})
    env.pop("FLAM_AUDIO_NO_DEVICE", None)
    if no_device:
        env["FLAM_AUDIO_NO_DEVICE"] = "1"
    proc = subprocess.Popen(
        [exe, "--stdin-control", "--sounds-dir", tmp, "--save-dir", tmp,
         os.path.join(tmp, "clock.lua")],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        env=env, cwd=os.path.dirname(exe))
    events = []   # (instant, ligne)
    lock = threading.Lock()

    def pump(stream):
        for raw in stream:
            with lock:
                events.append((time.monotonic(), raw.decode("utf-8", "replace").rstrip()))

    threads = [threading.Thread(target=pump, args=(s,), daemon=True)
               for s in (proc.stderr, proc.stdout)]
    for t in threads:
        t.start()
    rc = None
    try:
        deadline = time.monotonic() + DURATION + 15
        while (time.monotonic() < deadline and proc.poll() is None
               and not any("AUDIO_EV stop" in l for _, l in list(events))):
            time.sleep(0.02)
        if proc.poll() is None:
            time.sleep(0.3)
            try:
                proc.stdin.write(b"quit\n")
                proc.stdin.flush()
            except OSError:
                pass
        try:
            rc = proc.wait(8)
        except subprocess.TimeoutExpired:
            rc = None
    finally:
        if proc.poll() is None:
            proc.kill()
            proc.wait()
    for t in threads:
        t.join(2)
    return list(events), "\n".join(l for _, l in events), rc


def span(events):
    t_play = next((t for t, l in events if "AUDIO_EV play" in l), None)
    t_stop = next((t for t, l in events if "AUDIO_EV stop" in l), None)
    return t_play, t_stop


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--build", required=True, help="dossier contenant flam-player")
    a = ap.parse_args()
    exe = os.path.join(os.path.abspath(a.build),
                       "flam-player.exe" if sys.platform == "win32" else "flam-player")
    if not os.path.isfile(exe):
        print("[SKIP] e2e_audio_clock : %s introuvable" % exe)
        return SKIP
    tmp = tempfile.mkdtemp(prefix="flam-aud-")
    try:
        make_mp3(os.path.join(tmp, "clock.mp3"))
        with open(os.path.join(tmp, "clock.lua"), "w", newline="") as f:
            f.write(SCRIPT)
        for no_device, label in ((False, "peripherique dummy"),
                                 (True, "sans peripherique")):
            events, out, rc = run(exe, tmp, no_device)
            t_play, t_stop = span(events)
            check(t_play is not None and t_stop is not None,
                  "%s : 'play' puis 'stop' recus" % label, out)
            elapsed = t_stop - t_play
            check(elapsed >= 0.8 * DURATION,
                  "%s : lecture de %.2f s en %.2f s (au moins %.2f s)"
                  % (label, DURATION, elapsed, 0.8 * DURATION), out)
            check(rc == 0, "%s : le player quitte (code %r)" % (label, rc), out)
    finally:
        import shutil
        shutil.rmtree(tmp, ignore_errors=True)
    print("OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
