"""Genere les vecteurs de parite du reechantillonnage par ffmpeg externe (#354) a partir de
tools/telmi2flam/telmi2flam.py, qui sert d'oracle.

La sortie de ffmpeg est opaque : Python et Rust lancent le MEME faux ffmpeg deterministe,
l'executable du test Rust `crates/telmi2flam/tests/ffmpeg.rs` (qui joue ffmpeg quand il recoit
ses arguments, et verifie qu'ils sont exactement ceux attendus). Son chemin est affiche par :
    cargo test -p telmi2flam --test ffmpeg --no-run
(ligne « Executable tests\\ffmpeg.rs (...) »).

Sortie (commitee) : crates/telmi2flam/tests/data/ffmpeg_cases.zip (deflate), lue par
tests/ffmpeg.rs. Elle contient :
- cases/<id>.zip : archives des cas propres au reechantillonnage ; les autres sont lues dans
  package_cases.zip (meme generateur deterministe, SHA-256 de l'archive verifie) ;
- expected.json : une execution par (cas, ffmpeg, mode du faux ffmpeg) : options, erreur ou
  entrees du .plain.pk (nom, SHA-256), SHA-256 du .plain.pk, avertissements (code, message),
  nombre de fichiers reechantillonnes et nombre de lancements de ffmpeg.

Executions : tout le corpus de package_cases avec le faux ffmpeg (mode ok) ; les cas audio
hors 44 100 Hz avec chaque panne du faux ffmpeg, et avec FLAM_FFMPEG vers un fichier absent.

A lancer avec le Python de l'oracle (3.14.8, Pillow 12.3.0), sous Windows, depuis la
racine du depot :
    python crates/telmi2flam/tests/data/gen_ffmpeg_cases.py <chemin du faux ffmpeg>
"""

import contextlib
import hashlib
import io
import json
import os
import shutil
import sys
import tempfile
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.dont_write_bytecode = True

import gen_package_cases as gp  # noqa: E402  (fige l'horloge de zipfile, FLAM_FFMPEG="")

mc, t = gp.mc, gp.t
from test_telmi2flam import base_nodes, mp3_frames, std_files  # noqa: E402

OUT = os.path.join(HERE, "ffmpeg_cases.zip")
MODE = "T2F_FAKE_FFMPEG_MODE"
LOG = "T2F_FAKE_FFMPEG_LOG"
FAILURE_MODES = ("fail", "fail_silent", "flood", "empty", "none", "garbage", "rate48")
# Cas du corpus avec un MP3 hors 44 100 Hz (mpeg2 : 22 050 Hz, mpeg25 : 11 025 Hz).
CORPUS_RESAMPLED = ("mp3_48khz_warning", "mp3_mpeg2", "mp3_mpeg25", "pkg_title_mp3_48k")


def ffmpeg_cases():
    """Cas propres au reechantillonnage."""
    S = std_files
    cases = []
    mp3_48 = mp3_frames(0xFB, 0x94, 80)

    def same_content():
        # meme contenu sous trois noms (deux audios et title.mp3) : un seul ffmpeg
        n = base_nodes()
        n["stages"]["s0"]["audio"] = "a.mp3"
        n["stages"]["s1"] = {"image": "s0.png", "audio": "b.mp3", "control": {}}
        n["actions"]["a0"].append({"stage": "s1"})
        files = [f for f in S(nodes=n) if f[0] != "audios/s0.mp3"]
        return gp.zipc(files + [("audios/a.mp3", mp3_48), ("audios/b.mp3", mp3_48),
                                ("title.mp3", mp3_48)])
    cases.append(mc.Case("ff_same_content", same_content, ()))

    def bitrates():
        # debit estime : 320 -> 192, 32 -> 64, tags ID3v2 (4000 octets) et ID3v1 exclus
        id3 = b"ID3\x03\x00\x00\x00\x00\x1f\x20" + bytes(4000)
        n = base_nodes()
        for i, name in enumerate(("hi.mp3", "lo.mp3", "tagged.mp3", "k22.mp3")):
            n["stages"]["b%d" % i] = {"image": "s0.png", "audio": name, "control": {}}
            n["actions"]["a0"].append({"stage": "b%d" % i})
        return gp.zipc(S(nodes=n) + [
            ("audios/hi.mp3", mp3_frames(0xFB, 0xE4, 60)),
            ("audios/lo.mp3", mp3_frames(0xFB, 0x14, 60)),
            ("audios/tagged.mp3", id3 + mp3_frames(0xFB, 0x94, 60) + b"TAG" + bytes(125)),
            ("audios/k22.mp3", mp3_frames(0xF3, 0x80, 60))])
    cases.append(mc.Case("ff_bitrates", bitrates, ()))

    def mixed():
        # 44 100 et 48 000 Hz dans le meme fichier : seule 48000 est citee
        data = mp3_frames(0xFB, 0x90, 60) + mp3_frames(0xFB, 0x94, 60)
        return gp.zipc(gp.replace(S(), "audios/s0.mp3", data))
    cases.append(mc.Case("ff_mixed_rates", mixed, ()))
    cases.append(mc.Case("ff_mixed_rates_keep", mixed, ("--keep-size",)))
    return cases


def run(case, data, tmp, ffmpeg, mode, log):
    src = os.path.join(tmp, "in.zip")
    with open(src, "wb") as f:
        f.write(data)
    out = os.path.join(tmp, "out.plain.pk")
    if os.path.exists(out):
        os.remove(out)
    if os.path.exists(log):
        os.remove(log)
    kw = gp.args_of(case.args)
    # vignette toujours en `bandes` (tests/ffmpeg.rs) : args_of rend aussi `thumb_fit`
    kw.pop("thumb_fit")
    exp = {"id": case.id, "tags": case.tags, **kw,
           "archive_sha256": hashlib.sha256(data).hexdigest(),
           "ffmpeg": "fake" if ffmpeg != "absent" else "absent", "mode": mode}
    os.environ["FLAM_FFMPEG"] = ffmpeg if ffmpeg != "absent" else os.path.join(tmp, "absent", "ffmpeg.exe")
    os.environ[MODE] = mode
    os.environ[LOG] = log
    warnings = []
    err = io.StringIO()
    try:
        with contextlib.redirect_stderr(err):
            t.convert(src, out, thumb_fit="bandes",
                      progress=lambda e: warnings.append([e["code"], e["message"]])
                      if e["event"] == "warning" else None,
                      **kw)
    except t.ConvError as e:
        exp["error"] = e.code
        return exp
    except Exception as e:  # noqa: BLE001
        exp["error"] = "python_crash:%s" % type(e).__name__
        return exp
    with zipfile.ZipFile(out) as z:
        exp["entries"] = [[n, hashlib.sha256(z.read(n)).hexdigest()] for n in z.namelist()]
    with open(out, "rb") as f:
        exp["pk_sha256"] = hashlib.sha256(f.read()).hexdigest()
    exp["warnings"] = warnings
    exp["resampled"] = sum(1 for line in err.getvalue().splitlines()
                           if line.startswith("  i ") and "reechantillonne avec ffmpeg" in line)
    if ffmpeg != "absent":
        exp["ffmpeg_calls"] = (sum(1 for _ in open(log)) if os.path.exists(log) else 0)
    return exp


def main():
    if len(sys.argv) != 2 or not os.path.isfile(sys.argv[1]):
        sys.exit(__doc__)
    fake = os.path.abspath(sys.argv[1])
    if sys.platform != "win32":
        sys.exit("vecteurs a produire sous Windows (create_system = 0 de zipfile)")
    if sys.version_info[:3] != (3, 14, 8) or gp.Image.__version__ != "12.3.0":
        sys.exit("vecteurs a produire avec Python 3.14.8 + Pillow 12.3.0")
    with zipfile.ZipFile(gp.OUT) as z:
        pkg_ids = {n[len("cases/"):-len(".zip")] for n in z.namelist() if n.startswith("cases/")}
    # cas de vignette `flou` ou `rogner` (--thumb-fit) exclus : tests/ffmpeg.rs convertit en
    # `bandes`, le mode de la vignette est verifie par package_parity.rs
    corpus = [c for c in mc.corpus_cases() + gp.package_cases()
              if gp.args_of(c.args)["thumb_fit"] == "bandes"]
    own = ffmpeg_cases()
    runs = []
    tmp = tempfile.mkdtemp(prefix="ffmpeg_cases_")
    log = os.path.join(tmp, "calls.log")
    try:
        for case in corpus:
            if case.id not in pkg_ids:
                sys.exit("%s absent de package_cases.zip : regenerer gen_package_cases.py" % case.id)
            r = run(case, case.build(), tmp, fake, "ok", log)
            r["source"] = "package"
            runs.append(r)
        with zipfile.ZipFile(OUT + ".part", "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
            todo = [(c, "package") for c in corpus if c.id in CORPUS_RESAMPLED]
            for case in own:
                data = case.build()
                z.writestr(zipfile.ZipInfo("cases/%s.zip" % case.id, gp.FIXED_DATE), data,
                           zipfile.ZIP_DEFLATED, 9)
                r = run(case, data, tmp, fake, "ok", log)
                r["source"] = "ffmpeg"
                runs.append(r)
                todo.append((case, "ffmpeg"))
            for case, source in todo:
                data = case.build()
                for ffmpeg, mode in [(fake, m) for m in FAILURE_MODES] + [("absent", "ok")]:
                    r = run(case, data, tmp, ffmpeg, mode, log)
                    r["source"] = source
                    runs.append(r)
            same = [r for r in runs if r["id"] == "ff_same_content" and r["mode"] == "ok"]
            assert same[0]["ffmpeg_calls"] == 1 and same[0]["resampled"] == 3, same
            assert sum(1 for r in runs if r.get("resampled")) >= 5
            body = '{"python": "%d.%d.%d", "pillow": "%s", "runs": [\n%s\n]}\n' % (
                *sys.version_info[:3], gp.Image.__version__,
                ",\n".join(json.dumps(r, ensure_ascii=True, separators=(",", ":")) for r in runs))
            z.writestr(zipfile.ZipInfo("expected.json", gp.FIXED_DATE), body,
                       zipfile.ZIP_DEFLATED, 9)
        os.replace(OUT + ".part", OUT)
    finally:
        for k in (MODE, LOG):
            os.environ.pop(k, None)
        os.environ["FLAM_FFMPEG"] = ""
        shutil.rmtree(tmp, ignore_errors=True)
    print("%d executions ecrites dans %s (%d octets)" % (len(runs), OUT, os.path.getsize(OUT)))


if __name__ == "__main__":
    main()
