"""
Horloge audio sans peripherique (#575) : la lecture doit suivre l'horloge
murale, pas la vitesse du decodage.

    python tests/e2e/e2e_audio_clock.py --build <dossier contenant flam-player.exe>

flam-player est lance sur un script .lua qui joue un MP3 synthetique de ~2,5 s
(pilotes SDL dummy, --stdin-control) et trace chaque callback audio sur stderr :
  A) avec le peripherique "dummy" de SDL (temoin, sans trace de file virtuelle) ;
  B) sans peripherique (FLAM_AUDIO_NO_DEVICE=1 : dev_id == 0, comme une session
     sans son ou une VM), avec la trace dediee prouvant le passage par la file
     virtuelle ;
  C) SDL_QueueAudio en echec (FLAM_AUDIO_QUEUE_FAIL=1, peripherique dummy) :
     bascule tracee sur la file virtuelle ;
  dans A, B et C le "stop" arrive au plus tot apres ~80 % de la duree. Avant la
  correction, SDL ne consommait rien, l'horloge avancait au rythme du decodage
  et le "stop" arrivait en une fraction de seconde (lecture ~10 fois trop rapide) ;
  D) pause puis seek puis reprise (A, B et C) : la lecture en pause n'avance
     pas (pas de "stop" pendant la pause, "play" de reprise a la position du
     seek, duree restante respectee).
Mesure faite par l'horodatage de reception des lignes ; seule la borne basse est
testee (une machine chargee ne peut pas faire passer ce test a tort).
Code de sortie : 0 OK, 1 echec, 77 prerequis absents (flam-player).
"""
import argparse
import os
import struct
import subprocess
import sys
import tempfile
import threading
import time

SKIP = 77
FRAMES = 96                      # MPEG-1 Layer III 44,1 kHz, 1152 echantillons/trame
DURATION = FRAMES * 1152 / 44100.0   # ~2,51 s
FRAMES_LONG = 215                # piste du scenario D : ~5,6 s, la pause tombe avant la fin
DURATION_LONG = FRAMES_LONG * 1152 / 44100.0
FRAME_BYTES = 417                # 144 * 128000 / 44100, sans bourrage

SCRIPT = r'''audio.load(0, "clock.mp3", function(s, t)
  print("AUDIO_EV " .. s .. " " .. string.format("%.2f", t))
end)
audio.play()
'''
# pause a ~0,6 s, seek a 1,0 s pendant la pause, reprise a ~2,6 s
SCRIPT_PAUSE = SCRIPT.replace("clock.mp3", "clock_long.mp3") + r'''local n = 0
lv.timer.new(function()
  n = n + 1
  if n == 6 then audio.pause(); audio.seek(1.0)
  elseif n == 26 then audio.play() end
end, 100)
'''


def check(cond, what, err=""):
    if not cond:
        print("[ECHEC] " + what)
        if err:
            print("--- sortie du player ---\n" + err)
        raise SystemExit(1)
    print("[OK] " + what)


def make_mp3(path, frames=FRAMES):
    frame = bytes([0xFF, 0xFB, 0x90, 0x00]) + bytes(FRAME_BYTES - 4)
    with open(path, "wb") as f:
        f.write(frame * frames)
    # table de seek .mp3map (unites de 1/88200 s) : le seek en depend
    upf = 1152 * 88200 // 44100
    with open(path + "map", "wb") as f:
        f.write(struct.pack("<III", frames * upf, 0, 0))
        for i in range(frames):
            f.write(struct.pack("<II", i * FRAME_BYTES, i * upf))


def run(exe, tmp, mode, script):
    """Lance le player, attend le 'stop'. Retourne (lignes horodatees, sortie, code)."""
    env = dict(os.environ)
    env.update({"SDL_VIDEODRIVER": "dummy", "SDL_AUDIODRIVER": "dummy",
                "SDL_RENDER_DRIVER": "software", "FLAM_SCREENSHOT_AUTO_MS": "0"})
    env.pop("FLAM_AUDIO_NO_DEVICE", None)
    env.pop("FLAM_AUDIO_QUEUE_FAIL", None)
    if mode == "nodevice":
        env["FLAM_AUDIO_NO_DEVICE"] = "1"
    elif mode == "queuefail":
        env["FLAM_AUDIO_QUEUE_FAIL"] = "1"
    elif mode == "zero":   # valeur "0" : ne doit rien desactiver
        env["FLAM_AUDIO_NO_DEVICE"] = "0"
        env["FLAM_AUDIO_QUEUE_FAIL"] = "0"
    proc = subprocess.Popen(
        [exe, "--stdin-control", "--sounds-dir", tmp, "--save-dir", tmp,
         os.path.join(tmp, script)],
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
        deadline = time.monotonic() + DURATION_LONG + 15
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
        make_mp3(os.path.join(tmp, "clock_long.mp3"), FRAMES_LONG)
        with open(os.path.join(tmp, "clock.lua"), "w", newline="") as f:
            f.write(SCRIPT)
        with open(os.path.join(tmp, "clock_pause.lua"), "w", newline="") as f:
            f.write(SCRIPT_PAUSE)
        NODEV = "desactive par FLAM_AUDIO_NO_DEVICE"
        QFAIL = "SDL_QueueAudio a echoue"
        # (mode, libelle, trace attendue, traces interdites)
        cases = (("device", "A peripherique dummy", None, (NODEV, QFAIL)),
                 ("zero", "A' variables a 0 (sans effet)", None, (NODEV, QFAIL)),
                 ("nodevice", "B sans peripherique", NODEV, (QFAIL,)),
                 ("queuefail", "C SDL_QueueAudio en echec", QFAIL, (NODEV,)))
        for mode, label, trace, banned in cases:
            events, out, rc = run(exe, tmp, mode, "clock.lua")
            t_play, t_stop = span(events)
            check(t_play is not None and t_stop is not None,
                  "%s : 'play' puis 'stop' recus" % label, out)
            elapsed = t_stop - t_play
            check(elapsed >= 0.8 * DURATION,
                  "%s : lecture de %.2f s en %.2f s (au moins %.2f s)"
                  % (label, DURATION, elapsed, 0.8 * DURATION), out)
            if trace:
                check(trace in out, "%s : trace '%s'" % (label, trace), out)
            for b in banned:
                check(b not in out, "%s : pas de trace '%s'" % (label, b), out)
            check(rc == 0, "%s : le player quitte (code %r)" % (label, rc), out)

        # D : pause + seek + reprise
        for mode, label in (("device", "D peripherique dummy"),
                            ("nodevice", "D sans peripherique"),
                            ("queuefail", "D SDL_QueueAudio en echec")):
            events, out, rc = run(exe, tmp, mode, "clock_pause.lua")
            idx_pause = next((i for i, (_, l) in enumerate(events) if "AUDIO_EV pause" in l), None)
            check(idx_pause is not None, "%s : evenement 'pause' recu" % label, out)
            idx_res = next((i for i in range(idx_pause, len(events))
                            if "AUDIO_EV play" in events[i][1]), None)
            check(idx_res is not None, "%s : 'play' de reprise recu" % label, out)
            between = [l for _, l in events[idx_pause:idx_res] if "AUDIO_EV stop" in l]
            check(not between, "%s : aucun 'stop' pendant la pause" % label, out)
            t_pos = float(events[idx_res][1].split()[-1])
            check(0.9 <= t_pos <= 1.3,
                  "%s : reprise a la position du seek (%.2f s)" % (label, t_pos), out)
            t_resume = events[idx_res][0]
            t_stop = next((t for t, l in events[idx_res:] if "AUDIO_EV stop" in l), None)
            check(t_stop is not None, "%s : 'stop' apres la reprise" % label, out)
            check(t_stop - t_resume >= 0.8 * (DURATION_LONG - 1.1),
                  "%s : duree restante respectee (%.2f s)" % (label, t_stop - t_resume), out)
            check(events[idx_res][0] - events[idx_pause][0] >= 1.5,
                  "%s : pause d'au moins 1,5 s (%.2f s)"
                  % (label, events[idx_res][0] - events[idx_pause][0]), out)
            check(rc == 0, "%s : le player quitte (code %r)" % (label, rc), out)
    finally:
        import shutil
        shutil.rmtree(tmp, ignore_errors=True)
    print("OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
