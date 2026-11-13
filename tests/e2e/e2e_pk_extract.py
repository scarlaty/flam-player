"""
Extraction d'un .plain.pk et dossier .plain d'auteur (#567).

    python tests/e2e/e2e_pk_extract.py --build <dossier contenant flam-player.exe>

flam-player est lance sur un .plain.pk (pilotes SDL dummy, --stdin-control) :
  1. dossier .plain d'auteur (main.lua, aucun temoin) a cote du .pk : il est
     joue tel quel, jamais ecrase, et aucun temoin n'est cree ;
  2. dossier absent : extraction complete, temoin .extracted, pas de .extracting ;
  3. .extracting present (extraction interrompue) : re-extraction ;
  4. .extracted different de l'empreinte du .pk : re-extraction ;
  5. .extracted a jour : le dossier n'est pas touche, trace "a jour, non extrait"
     et le player demarre (trace "Loading story") ;
  6. .pk plus recent que le dossier d'auteur : le dossier reste joue tel quel ;
  7. .extracted present sans main.lua : re-extraction ;
  8. archive tronquee, signature inconnue au milieu, entree a data descriptor :
     extraction incomplete, pas de temoin .extracted, .extracting laisse ;
  9. re-extraction : un fichier disparu du .pk disparait du dossier extrait, un
     fichier ajoute a la main reste, rien n'est supprime a travers un lien ou
     une jonction ;
 10. .pk supprime ou remplace par un fichier non ZIP apres extraction : le
     dossier extrait reste intact et l'histoire est jouee (trace ".pk absent ou
     illisible" pour un .pk supprime) ;
 11. temoins forges (noms . / casse / point ou espace final dans le .pk, liste
     hostile dans .extracted) : rien n'est ecrit ni supprime hors du dossier ;
     purge reprise depuis le .extracting d'une extraction interrompue ;
     noms de peripheriques Windows (CON, NUL.txt, COM1, sub/aux), composants a
     point ou espace final, alias 8.3 (EXTRAC~1), composant .extracted/x : refuses ;
 12. passe a blanc avant purge : .pk tronque apres main.lua, a data descriptor,
     ou dont main.lua est compresse, apres une extraction valide : l'ancienne
     extraction reste intacte (fichiers, .extracted) et l'histoire est jouee ;
     alias 8.3 du temoin suivi d'une signature inconnue : .extracting non ecrase ;
 13. alias 8.3 HACHE du temoin (EX577D~1, quand EXTRAC~1 a ~4 sont pris) : ecriture
     refusee par le verrou, .extracting non ecrase.
Code 77 aussi si la jonction / le lien de test ne peut pas etre cree, ou si les noms
courts sont desactives sur le volume (cas 13 saute : FLAM_NO_SKIP=1 le voit).
Code de sortie : 0 OK, 1 echec, 77 prerequis absents (flam-player).
"""
import argparse
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import zipfile

SKIP = 77
LUA = 'local l = lv.label.new(window)\nlv.label.set_text(l, "pk")\n-- %s\n'


def make_pk(path, tag, extra=None):
    with zipfile.ZipFile(path, "w", zipfile.ZIP_STORED) as z:
        z.writestr("main.lua", LUA % tag)
        for name, data in (extra or {}).items():
            z.writestr(name, data)


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="") as f:
        f.write(text)


def read(path):
    with open(path, encoding="utf-8", newline="") as f:
        return f.read()


def check(cond, what, err=""):
    if not cond:
        print("[ECHEC] " + what)
        if err:
            print("--- stderr du player ---\n" + err)
        raise SystemExit(1)
    print("[OK] " + what)


def run_player(exe, pk, tmp, wait_for):
    """Lance le player sur pk, attend une trace, l'arrete par 'quit'."""
    env = dict(os.environ)
    env.update({"SDL_VIDEODRIVER": "dummy", "SDL_AUDIODRIVER": "dummy",
                "SDL_RENDER_DRIVER": "software", "FLAM_SCREENSHOT_AUTO_MS": "0"})
    proc = subprocess.Popen(
        [exe, "--stdin-control", "--save-dir", tmp, pk], stdin=subprocess.PIPE,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env,
        cwd=os.path.dirname(exe))
    lines = []

    def pump(stream):
        for raw in stream:
            lines.append(raw.decode("utf-8", "replace").rstrip())

    # stdout (trace "Loading story") est tamponne : lu en entier a la sortie
    t = threading.Thread(target=pump, args=(proc.stderr,), daemon=True)
    t.start()
    t2 = threading.Thread(target=pump, args=(proc.stdout,), daemon=True)
    t2.start()
    try:
        deadline = time.monotonic() + 20
        while (time.monotonic() < deadline and proc.poll() is None
               and not any(wait_for in x for x in lines)):
            time.sleep(0.05)
        time.sleep(0.3)
        if proc.poll() is None:
            try:
                proc.stdin.write(b"quit\n")
                proc.stdin.flush()
            except OSError:
                pass
            try:
                proc.wait(10)
            except subprocess.TimeoutExpired:
                pass
    finally:
        if proc.poll() is None:
            proc.kill()
            proc.wait()
    t.join(2)
    t2.join(2)
    return "\n".join(lines)


def fresh(tmp, name, extra=None):
    root = os.path.join(tmp, name)
    shutil.rmtree(root, ignore_errors=True)
    os.makedirs(root)
    pk = os.path.join(root, "foo.plain.pk")
    make_pk(pk, "PK", extra)
    return root, pk, os.path.join(root, "foo.plain")


def second_entry_offset(pk):
    """Decalage du deuxieme en-tete local de l'archive (signature PK 03 04)."""
    with open(pk, "rb") as f:
        data = f.read()
    first = data.find(b"PK\x03\x04")
    second = data.find(b"PK\x03\x04", first + 4)
    assert first == 0 and second > 0
    return data, second


def patch_pk(pk, offset, new):
    with open(pk, "r+b") as f:
        f.seek(offset)
        f.write(new)


def make_link(link, target):
    """Jonction (Windows) ou lien symbolique (POSIX) ; False si impossible."""
    try:
        if sys.platform == "win32":
            r = subprocess.run(["cmd", "/c", "mklink", "/J", link.replace("/", "\\"),
                                target.replace("/", "\\")],
                               capture_output=True)
            return r.returncode == 0
        os.symlink(target, link)
        return True
    except OSError:
        return False


def drop_link(link):
    try:
        if sys.platform == "win32":
            os.rmdir(link)
        else:
            os.remove(link)
    except OSError:
        pass


FOREIGN = 4   # fichiers etrangers qui prennent EXTRAC~1 a ~4 (le suivant recoit un alias hache)


def hashed_busy_alias(tmp):
    """Nom court 8.3 que recevra .extracting une fois EXTRAC~1 a ~4 pris par des fichiers
    etrangers (ex. 'EX577D~1'), ou None (hors Windows, noms courts desactives sur le
    volume, alias non hache). Calcule dans un dossier jetable avec GetShortPathNameW."""
    if sys.platform != "win32":
        return None
    import ctypes
    probe = os.path.join(tmp, "sonde-alias")
    os.makedirs(probe)
    try:
        for i in range(FOREIGN):
            write(probe + "/extracting_%d" % i, "etranger")
        busy = probe + "/.extracting"
        write(busy, "")
        buf = ctypes.create_unicode_buffer(1024)
        if not ctypes.windll.kernel32.GetShortPathNameW(busy.replace("/", "\\"), buf, 1024):
            return None
        alias = os.path.basename(buf.value.replace("\\", "/"))
        if "~" not in alias or alias.lower().startswith("extrac~") or alias.lower() == ".extracting":
            return None
        return alias
    finally:
        shutil.rmtree(probe, ignore_errors=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--build", required=True, help="dossier contenant flam-player")
    a = ap.parse_args()
    exe = os.path.join(os.path.abspath(a.build),
                       "flam-player.exe" if sys.platform == "win32" else "flam-player")
    if not os.path.isfile(exe):
        print("[SKIP] e2e_pk_extract : %s introuvable" % exe)
        return SKIP
    tmp = tempfile.mkdtemp(prefix="flam-pkx-")
    link_skipped = False
    alias_skipped = False
    try:
        # 1. dossier d'auteur
        root, pk, d = fresh(tmp, "auteur")
        write(d + "/main.lua", LUA % "AUTEUR")
        write(d + "/notes.txt", "a moi")
        err = run_player(exe, pk, tmp, "dossier d'auteur")
        check("dossier d'auteur" in err, "trace du dossier d'auteur", err)
        check("supprimer le dossier pour forcer la re-extraction" in err,
              "trace du dossier d'auteur : remede indique", err)
        check(read(d + "/main.lua") == LUA % "AUTEUR", "main.lua d'auteur intact", err)
        check(read(d + "/notes.txt") == "a moi", "fichier d'auteur intact", err)
        check(not os.path.exists(d + "/.extracted")
              and not os.path.exists(d + "/.extracting"),
              "aucun temoin cree dans le dossier d'auteur", err)

        # 2. dossier absent
        root, pk, d = fresh(tmp, "absent")
        err = run_player(exe, pk, tmp, "Extracted")
        check(read(d + "/main.lua") == LUA % "PK", "extraction d'un dossier absent", err)
        check(os.path.isfile(d + "/.extracted"), "temoin .extracted pose", err)
        check(not os.path.exists(d + "/.extracting"), "pas de .extracting", err)

        # 5. a jour : modification locale conservee, trace attendue (pas de delai)
        write(d + "/main.lua", LUA % "LOCAL")
        t0 = time.monotonic()
        err = run_player(exe, pk, tmp, "a jour, non extrait")
        check("a jour, non extrait" in err, "trace de l'etat a jour", err)
        check("Loading story" in err, "le player a demarre sur le dossier a jour", err)
        check(time.monotonic() - t0 < 15, "pas d'attente de 20 s", err)
        check(read(d + "/main.lua") == LUA % "LOCAL",
              "dossier a jour non touche", err)

        # 4. empreinte differente
        write(d + "/.extracted", "0 0\n")
        err = run_player(exe, pk, tmp, "Extracted")
        check(read(d + "/main.lua") == LUA % "PK", "empreinte differente : re-extraction", err)
        check(not os.path.exists(d + "/.extracting"), "pas de .extracting apres re-extraction", err)

        # 3. extraction interrompue
        root, pk, d = fresh(tmp, "interrompu")
        write(d + "/main.lua", LUA % "PARTIEL")
        write(d + "/.extracting", "")
        err = run_player(exe, pk, tmp, "Extracted")
        check(read(d + "/main.lua") == LUA % "PK", ".extracting : re-extraction", err)
        check(os.path.isfile(d + "/.extracted")
              and not os.path.exists(d + "/.extracting"),
              "temoins apres reprise", err)

        # 6. .pk plus recent que le dossier d'auteur : toujours joue tel quel
        root, pk, d = fresh(tmp, "auteur-recent")
        write(d + "/main.lua", LUA % "AUTEUR")
        future = time.time() + 3600
        os.utime(pk, (future, future))
        err = run_player(exe, pk, tmp, "dossier d'auteur")
        check("dossier d'auteur" in err, ".pk recent : trace du dossier d'auteur", err)
        check(read(d + "/main.lua") == LUA % "AUTEUR", ".pk recent : dossier d'auteur intact", err)
        check(not os.path.exists(d + "/.extracted")
              and not os.path.exists(d + "/.extracting"),
              ".pk recent : aucun temoin cree", err)

        # 7. .extracted sans main.lua : re-extraction
        root, pk, d = fresh(tmp, "sans-main")
        write(d + "/.extracted", "0 0\n")
        err = run_player(exe, pk, tmp, "Extracted")
        check(read(d + "/main.lua") == LUA % "PK", ".extracted sans main.lua : re-extraction", err)
        check(os.path.isfile(d + "/.extracted")
              and not os.path.exists(d + "/.extracting"),
              ".extracted sans main.lua : temoins apres reprise", err)

        # 8a. archive tronquee au milieu de la 2e entree
        root, pk, d = fresh(tmp, "tronque", {"data.bin": "x" * 5000})
        data, second = second_entry_offset(pk)
        with open(pk, "wb") as f:
            f.write(data[:second + 30 + len("data.bin") + 100])
        err = run_player(exe, pk, tmp, "extraction incomplete")
        check("extraction incomplete" in err, "archive tronquee : extraction incomplete", err)
        check(not os.path.exists(d + "/.extracted"),
              "archive tronquee : pas de temoin .extracted", err)
        check(os.path.exists(d + "/.extracting"),
              "archive tronquee : .extracting laisse", err)

        # 8b. signature inconnue au milieu de l'archive
        root, pk, d = fresh(tmp, "signature", {"data.bin": "x" * 100})
        data, second = second_entry_offset(pk)
        patch_pk(pk, second, b"JUNK")
        err = run_player(exe, pk, tmp, "signature inconnue")
        check("signature inconnue" in err, "signature inconnue : trace", err)
        check(not os.path.exists(d + "/.extracted"),
              "signature inconnue : pas de temoin .extracted", err)
        check(os.path.exists(d + "/.extracting"),
              "signature inconnue : .extracting laisse", err)

        # 8c. entree a data descriptor (bit 3 des indicateurs)
        root, pk, d = fresh(tmp, "descriptor", {"data.bin": "x" * 100})
        data, second = second_entry_offset(pk)
        patch_pk(pk, second + 6, b"\x08\x00")
        err = run_player(exe, pk, tmp, "data descriptor")
        check("data descriptor" in err, "data descriptor : trace", err)
        check(not os.path.exists(d + "/.extracted"),
              "data descriptor : pas de temoin .extracted", err)
        check(os.path.exists(d + "/.extracting"),
              "data descriptor : .extracting laisse", err)

        # 9. re-extraction : purge de l'ancienne version
        root, pk, d = fresh(tmp, "purge", {"old.txt": "v1", "sub/old2.txt": "v1",
                                           "keep/shared.txt": "v1"})
        err = run_player(exe, pk, tmp, "Extracted")
        check(read(d + "/old.txt") == "v1" and read(d + "/sub/old2.txt") == "v1",
              "purge : v1 extraite", err)
        write(d + "/mine.txt", "a moi")
        make_pk(pk, "V2", {"keep/shared.txt": "v2"})   # old.txt et sub/ disparaissent
        err = run_player(exe, pk, tmp, "Extracted")
        check(read(d + "/main.lua") == LUA % "V2", "purge : v2 extraite", err)
        check(not os.path.exists(d + "/old.txt"), "purge : fichier disparu supprime", err)
        check(not os.path.exists(d + "/sub"), "purge : dossier vide supprime", err)
        check(read(d + "/keep/shared.txt") == "v2", "purge : fichier conserve mis a jour", err)
        check(read(d + "/mine.txt") == "a moi", "purge : fichier ajoute a la main conserve", err)

        # 9b. aucun fichier supprime a travers une jonction / un lien
        root, pk, d = fresh(tmp, "purge-lien", {"sub/old2.txt": "v1"})
        err = run_player(exe, pk, tmp, "Extracted")
        outside = os.path.join(tmp, "dehors")
        os.makedirs(outside)
        write(outside + "/old2.txt", "precieux")
        os.remove(d + "/sub/old2.txt")
        os.rmdir(d + "/sub")
        if make_link(d + "/sub", outside):
            try:
                make_pk(pk, "V2")                       # sub/old2.txt disparait du .pk
                err = run_player(exe, pk, tmp, "Extracted")
                check(read(d + "/main.lua") == LUA % "V2", "lien : v2 extraite", err)
                check(read(outside + "/old2.txt") == "precieux",
                      "lien : fichier hors du dossier d'extraction intact", err)
            finally:
                drop_link(d + "/sub")
        else:
            print("[SKIP] purge a travers un lien : creation du lien impossible")
            link_skipped = True

        # 10a. .pk supprime apres extraction : dossier intact, histoire jouee
        root, pk, d = fresh(tmp, "pk-supprime", {"data/a.txt": "A"})
        err = run_player(exe, pk, tmp, "Extracted")
        check(os.path.isfile(d + "/.extracted"), ".pk supprime : extraction initiale", err)
        os.remove(pk)
        err = run_player(exe, pk, tmp, ".pk absent ou illisible")
        check(".pk absent ou illisible" in err and "a jour, non extrait" not in err,
              ".pk supprime : trace '.pk absent ou illisible' (et non 'a jour')", err)
        check("Loading story" in err, ".pk supprime : l'histoire est jouee", err)
        check(read(d + "/main.lua") == LUA % "PK" and read(d + "/data/a.txt") == "A",
              ".pk supprime : dossier extrait intact", err)
        check(os.path.isfile(d + "/.extracted"), ".pk supprime : temoin conserve", err)

        # 10b. .pk remplace par un fichier non ZIP : dossier intact, histoire jouee
        root, pk, d = fresh(tmp, "pk-non-zip", {"data/a.txt": "A"})
        err = run_player(exe, pk, tmp, "Extracted")
        with open(pk, "wb") as f:
            f.write(b"ceci n'est pas une archive ZIP" * 7)
        err = run_player(exe, pk, tmp, "extraction impossible")
        check("illisible, incomplet ou sans main.lua" in err, ".pk non ZIP : trace", err)
        check("Loading story" in err, ".pk non ZIP : l'histoire est jouee", err)
        check(read(d + "/main.lua") == LUA % "PK" and read(d + "/data/a.txt") == "A",
              ".pk non ZIP : dossier extrait intact", err)
        check(os.path.isfile(d + "/.extracted"), ".pk non ZIP : temoin conserve", err)

        # 11a. noms de temoins forges et chemins hostiles dans le .pk
        root, pk, d = fresh(tmp, "temoins-forges")
        sentinel = os.path.join(root, "sentinelle.txt")
        write(sentinel, "S")
        hostile = ["./.extracting", ".extracted.", ".EXTRACTED ", "sub/.extracting",
                   "sub/.Extracted...", "../sentinelle.txt", "../x", "C:/x", "/abs",
                   "sub\\..\\..\\x", "./ok.txt", "a/./b.txt",
                   "CON", "nul.txt", "COM1", "lpt9.log", "sub/aux", "Prn .txt",
                   "dot./c.txt", "d.txt ", "e.txt.", ".extracted/x", "EXTRAC~1",
                   "extrac~12.x", "EXTRAC~2", "a?b.txt", "x<y.txt", 'q"r.txt', "p|q.txt",
                   "s*t.txt", "u>v.txt", "COM¹", "lpt².txt", "Com³"]
        with zipfile.ZipFile(pk, "w", zipfile.ZIP_STORED) as z:
            z.writestr("main.lua", LUA % "PK")
            z.writestr("keep.txt", "k")
            for name in hostile:
                z.writestr(zipfile.ZipInfo(name), "FORGE")
        err = run_player(exe, pk, tmp, "Extracted")
        check(read(sentinel) == "S", "temoins forges : sentinelle hors dossier intacte", err)
        check(not os.path.exists(os.path.join(root, "x")), "temoins forges : pas de ../x", err)
        check(not os.path.exists(d + "/ok.txt") and not os.path.exists(d + "/a"),
              "temoins forges : composants '.' refuses", err)
        check(os.path.isfile(d + "/.extracted") and not os.path.exists(d + "/.extracting"),
              "temoins forges : temoins du player seuls", err)
        stamp_lines = read(d + "/.extracted").split("\n")
        check(stamp_lines[1:] == ["main.lua", "keep.txt", ""],
              "temoins forges : liste = fichiers reellement extraits", err)
        check(not os.path.exists(d + "/sub"), "temoins forges : aucun dossier sub", err)
        for bad in ("CON", "nul.txt", "COM1", "lpt9.log", "dot.", "d.txt", "e.txt",
                    "extrac~12.x", "Prn .txt", "a?b.txt", "x<y.txt", "s*t.txt"):   # EXTRAC~N : alias reel des temoins
            check(not os.path.exists(d + "/" + bad),
                  "temoins forges : '%s' non extrait" % bad, err)

        # 11b. temoin hostile deja present : la purge ne sort pas du dossier
        root, pk, d = fresh(tmp, "temoin-hostile")
        sentinel = os.path.join(root, "sentinelle.txt")
        sentinel2 = os.path.join(tmp, "sentinelle2.txt")
        write(sentinel, "S")
        write(sentinel2, "S2")
        write(d + "/main.lua", LUA % "ANCIEN")
        write(d + "/mine.txt", "a moi")
        write(d + "/.extracted", "0 0\n../sentinelle.txt\nC:/x\n/abs\n"
              + sentinel2 + "\nsub\\..\\..\\sentinelle.txt\n./mine.txt\nmain.lua\n")
        err = run_player(exe, pk, tmp, "Extracted")
        check(read(sentinel) == "S" and read(sentinel2) == "S2",
              "temoin hostile : sentinelles hors dossier intactes", err)
        check(read(d + "/mine.txt") == "a moi", "temoin hostile : './mine.txt' non suivi", err)
        check(read(d + "/main.lua") == LUA % "PK", "temoin hostile : re-extraction", err)

        # 11c. purge depuis le .extracting d'une extraction interrompue
        root, pk, d = fresh(tmp, "interrompu-purge")
        write(d + "/main.lua", LUA % "PARTIEL")
        write(d + "/old.txt", "v1")
        write(d + "/mine.txt", "a moi")
        write(d + "/.extracting", "main.lua\nold.txt\n")
        err = run_player(exe, pk, tmp, "Extracted")
        check(read(d + "/main.lua") == LUA % "PK", "interrompu : re-extraction", err)
        check(not os.path.exists(d + "/old.txt"), "interrompu : fichier de la liste purge", err)
        check(read(d + "/mine.txt") == "a moi", "interrompu : fichier hors liste conserve", err)
        check(os.path.isfile(d + "/.extracted") and not os.path.exists(d + "/.extracting"),
              "interrompu : temoins apres reprise", err)

        # 12a. .pk tronque apres main.lua, remplace apres une extraction valide
        root, pk, d = fresh(tmp, "blanc-tronque", {"old.txt": "v1", "data.bin": "x" * 5000})
        err = run_player(exe, pk, tmp, "Extracted")
        check(read(d + "/old.txt") == "v1" and os.path.isfile(d + "/.extracted"),
              "passe a blanc : extraction initiale", err)
        stamp_before = read(d + "/.extracted")
        make_pk(pk, "V2", {"data.bin": "y" * 5000})
        data, second = second_entry_offset(pk)
        with open(pk, "wb") as f:
            f.write(data[:second + 30 + len("data.bin") + 100])   # coupe dans data.bin
        err = run_player(exe, pk, tmp, "extraction impossible")
        check("illisible, incomplet ou sans main.lua" in err,
              "passe a blanc : .pk tronque refuse", err)
        check("Loading story" in err, "passe a blanc : l'histoire est jouee", err)
        check(read(d + "/main.lua") == LUA % "PK" and read(d + "/old.txt") == "v1"
              and read(d + "/data.bin") == "x" * 5000,
              "passe a blanc : ancienne extraction intacte (tronque)", err)
        check(read(d + "/.extracted") == stamp_before
              and not os.path.exists(d + "/.extracting"),
              "passe a blanc : temoins intacts (tronque)", err)

        # 12b. entree a data descriptor apres main.lua
        make_pk(pk, "V2", {"data.bin": "y" * 100})
        data, second = second_entry_offset(pk)
        patch_pk(pk, second + 6, b"\x08\x00")
        err = run_player(exe, pk, tmp, "extraction impossible")
        check("illisible, incomplet ou sans main.lua" in err,
              "passe a blanc : data descriptor refuse", err)
        check(read(d + "/main.lua") == LUA % "PK" and read(d + "/old.txt") == "v1"
              and read(d + "/.extracted") == stamp_before,
              "passe a blanc : ancienne extraction intacte (descriptor)", err)

        # 12c. main.lua stocke de taille 0 (passait pk_has_entry) puis compresse
        with zipfile.ZipFile(pk, "w", zipfile.ZIP_STORED) as z:
            z.writestr("main.lua", "")
            z.writestr("data.bin", "y" * 100)
        err = run_player(exe, pk, tmp, "extraction impossible")
        check("illisible, incomplet ou sans main.lua" in err,
              "passe a blanc : main.lua de taille 0 refuse", err)
        check(read(d + "/main.lua") == LUA % "PK" and read(d + "/old.txt") == "v1"
              and read(d + "/.extracted") == stamp_before,
              "passe a blanc : ancienne extraction intacte (taille 0)", err)
        with zipfile.ZipFile(pk, "w", zipfile.ZIP_DEFLATED) as z:
            z.writestr("main.lua", (LUA % "V2") * 50)
        err = run_player(exe, pk, tmp, "extraction impossible")
        check("illisible, incomplet ou sans main.lua" in err,
              "passe a blanc : main.lua compresse refuse", err)
        check(read(d + "/main.lua") == LUA % "PK" and read(d + "/.extracted") == stamp_before,
              "passe a blanc : ancienne extraction intacte (compresse)", err)

        # 12e. v2 avec un nom que Windows ne peut pas ecrire : refuse avant la purge de v1
        make_pk(pk, "V2", {"a?b.txt": "x", "k.txt": "k"})
        err = run_player(exe, pk, tmp, "Extracted")
        check(read(d + "/main.lua") == (LUA % "V2") and not os.path.exists(d + "/old.txt"),
              "nom interdit : v2 extraite (nom refuse, extraction complete)", err)
        check(os.path.isfile(d + "/.extracted") and not os.path.exists(d + "/.extracting"),
              "nom interdit : temoins complets, pas de re-extraction a chaque lancement", err)
        check(read(d + "/k.txt") == "k" and not os.path.exists(d + "/a?b.txt"),
              "nom interdit : entree saine extraite, entree interdite non extraite", err)

        # 12d. alias 8.3 du temoin puis signature inconnue : .extracting non ecrase
        root, pk, d = fresh(tmp, "alias-court", {"EXTRAC~1": "FORGE", "EXTRAC~2": "FORGE",
                                                 "k.txt": "k"})
        with open(pk, "rb") as f:
            data = f.read()
        last = data.rfind(b"PK\x03\x04")
        patch_pk(pk, last, b"JUNK")
        err = run_player(exe, pk, tmp, "signature inconnue")
        check(not os.path.exists(d + "/.extracted"), "alias 8.3 : pas de .extracted", err)
        check(os.path.isfile(d + "/.extracting")
              and read(d + "/.extracting") == "main.lua\n",
              "alias 8.3 : .extracting = liste reelle, non ecrase", err)

        # 12f. alias 8.3 HACHE du temoin (EXTRAC~1 a ~4 pris par des fichiers etrangers :
        # .extracting recoit un alias du type EX577D~1, que le filtre de noms ne connait
        # pas). Le verrou d'ecriture empeche l'ecrasement : l'extraction reste incomplete.
        alias = hashed_busy_alias(tmp)
        if alias is None:
            print("[SKIP] alias 8.3 hache : noms courts desactives ou alias non hache")
            alias_skipped = True
        else:
            root, pk, d = fresh(tmp, "alias-hache")
            for i in range(FOREIGN):
                write(d + "/extracting_%d" % i, "etranger")
            with zipfile.ZipFile(pk, "w", zipfile.ZIP_STORED) as z:
                z.writestr("main.lua", LUA % "PK")
                z.writestr(alias, "FORGE")
                z.writestr("k.txt", "k")
            err = run_player(exe, pk, tmp, "extraction incomplete")
            check("ouverture impossible" in err, "alias hache '%s' : ecriture refusee" % alias, err)
            check(read(d + "/.extracting") == "main.lua\nk.txt\n",
                  "alias hache : .extracting = liste reelle, non ecrase", err)
            check(not os.path.exists(d + "/.extracted"), "alias hache : pas de .extracted", err)
            check(read(d + "/k.txt") == "k", "alias hache : entree saine extraite", err)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    if link_skipped or alias_skipped:
        print("e2e_pk_extract : OK, mais un cas a ete saute (SKIP) : %s"
              % ", ".join(n for n, f in (("lien", link_skipped), ("alias 8.3 hache", alias_skipped)) if f))
        return SKIP
    print("e2e_pk_extract : OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
