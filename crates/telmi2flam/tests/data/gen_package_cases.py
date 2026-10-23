"""Genere les vecteurs de parite du paquet assemble en memoire par le port Rust (#129) a
partir de tools/telmi2flam/telmi2flam.py, qui sert d'oracle.

Sortie (commitee) : crates/telmi2flam/tests/data/package_cases.zip (deflate), lue par
crates/telmi2flam/tests/package_parity.rs. Elle contient :
- cases/<id>.zip : l'archive TELMI de chaque cas ;
- expected.json : pour chaque cas, les options, les etiquettes du corpus, et soit l'erreur
  de Python (code), soit les entrees du .plain.pk dans l'ordre (nom, SHA-256), le SHA-256 du
  .plain.pk complet (`pk_sha256`) et les codes d'avertissement dans l'ordre d'emission ;
- zip_cases.json : SHA-256 du ZIP ecrit par `zipfile.writestr` pour des listes d'entrees
  synthetiques (noms non ASCII, seuils ZIP64 reduits), relues par output.rs (#130).

L'horloge de `zipfile` est figee au 1980-01-01 00:00:00 : `writestr` horodate chaque entree a
l'heure locale, le port Rust fixe cette date (PORT-RUST.md 4.1). A horodatage egal, le
.plain.pk de Rust doit etre identique a l'octet pres a celui de Python.

Cas : corpus de parite (tests/parity/make_corpus.py) et cas propres aux assets (collisions
de noms, noms reserves, couverture, title.mp3, uuid, metadonnees de types inattendus).
La vignette est produite en mode `bandes` (seul mode porte, #282) et sans ffmpeg
(FLAM_FFMPEG vide ; avec ffmpeg : gen_ffmpeg_cases.py, #354).

A lancer avec le Python de l'installateur (build-converter.ps1 : 3.14.8, Pillow 12.3.0),
depuis la racine du depot :
    python crates/telmi2flam/tests/data/gen_package_cases.py
"""

import hashlib
import json
import os
import shutil
import sys
import tempfile
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", "..", "..", ".."))
TOOL = os.path.join(ROOT, "tools", "telmi2flam")
for _p in (os.path.join(TOOL, "tests", "parity"), os.path.join(TOOL, "tests"), TOOL):
    if _p not in sys.path:
        sys.path.insert(0, _p)
sys.dont_write_bytecode = True
# Vecteurs deterministes : pas de reechantillonnage audio (#46), donc pas de ffmpeg.
os.environ["FLAM_FFMPEG"] = ""

import make_corpus as mc  # noqa: E402
import telmi2flam as t  # noqa: E402
from PIL import Image  # noqa: E402
from test_telmi2flam import MP3, base_nodes, mp3_frames, png, std_files  # noqa: E402

OUT = os.path.join(HERE, "package_cases.zip")
FIXED_DATE = (2026, 1, 1, 0, 0, 0)


class _FrozenClock:
    """Remplace le module `time` vu par `zipfile` : `writestr` date alors chaque entree du
    1980-01-01 00:00:00, la date fixe du port Rust."""

    @staticmethod
    def time():
        return 0.0

    @staticmethod
    def localtime(_secs=None):
        return (1980, 1, 1, 0, 0, 0, 1, 1, -1)


zipfile.time = _FrozenClock


def pattern(i, size):
    """Contenu synthetique de l'entree i (repris par output.rs)."""
    return bytes((i * 7 + j) % 251 for j in range(size))


def zip_cases():
    """ZIP de `zipfile.writestr` pour des entrees synthetiques ; ZIP64 exerce en reduisant
    les seuils du module (globaux lus a chaque ecriture)."""
    lists = {
        "ascii": [("a.txt", 0), ("dir/b.bin", 300)],
        "non_ascii": [("é/ü.mp3", 10), ("日本/語.lif", 5), ("emoji_\U0001F600", 3), ("x", 1)],
        "empty": [],
    }
    cases = []
    # 300 : en-tete local ZIP64 (300 * 1,05 > 300) sans ZIP64 dans le repertoire central.
    for zip64, count in [(None, None), (100, None), (250, None), (300, None), (40, None),
                         (None, 2), (60, 3)]:
        for lid, entries in lists.items():
            saved = zipfile.ZIP64_LIMIT, zipfile.ZIP_FILECOUNT_LIMIT
            path = os.path.join(tempfile.gettempdir(), "zip_case_%d.zip" % os.getpid())
            try:
                if zip64 is not None:
                    zipfile.ZIP64_LIMIT = zip64
                if count is not None:
                    zipfile.ZIP_FILECOUNT_LIMIT = count
                with zipfile.ZipFile(path, "w", zipfile.ZIP_STORED) as z:
                    for i, (name, size) in enumerate(entries):
                        z.writestr(name, pattern(i, size))
                with open(path, "rb") as f:
                    digest = hashlib.sha256(f.read()).hexdigest()
            finally:
                zipfile.ZIP64_LIMIT, zipfile.ZIP_FILECOUNT_LIMIT = saved
                if os.path.exists(path):
                    os.remove(path)
            cases.append({"entries": entries, "zip64": zip64 or zipfile.ZIP64_LIMIT,
                          "file_count": count or zipfile.ZIP_FILECOUNT_LIMIT,
                          "sha256": digest})
    return cases


def zipc(files):
    return mc.zip_bytes(files)


def replace(files, name, data):
    return [(n, data if n == name else d) for n, d in files]


def without(files, name):
    return [(n, d) for n, d in files if n != name]


def package_cases():
    S = std_files
    cases = []

    def add(cid, build, args=()):
        cases.append(mc.Case(cid, build, args))

    # --- allocation des noms : collisions insensibles a la casse, reserves, nettoyage
    def collisions():
        n = base_nodes()
        st = n["stages"]
        st["s1"] = {"image": "S0.png", "audio": "S0.mp3", "control": {}}
        st["s2"] = {"image": "s0!.png", "audio": "title.mp3", "control": {}}
        st["s3"] = {"image": "title.png", "audio": "silent.mp3", "control": {}}
        st["s4"] = {"image": "dir/s0_2.png", "audio": "Scène #1.mp3", "control": {}}
        st["s5"] = {"image": "empty.png", "audio": "sub/.mp3", "control": {}}
        st["s6"] = {"image": "s0.png", "audio": "s0.mp3", "control": {}}  # caches
        n["actions"]["a0"] = [{"stage": s} for s in st]
        files = S(nodes=n) + [("images/S0.png", png(30, 20, (0, 0, 200, 255))),
                              ("images/s0!.png", png(20, 30, (0, 200, 0, 255))),
                              ("images/title.png", png(10, 10)),
                              ("images/dir/s0_2.png", png(12, 12)),
                              ("images/empty.png", png(8, 8)),
                              ("audios/S0.mp3", mp3_frames(n=30)),
                              ("audios/title.mp3", mp3_frames(n=31)),
                              ("audios/silent.mp3", mp3_frames(n=32)),
                              ("audios/Scène #1.mp3", mp3_frames(n=33)),
                              ("audios/sub/.mp3", mp3_frames(n=34))]
        return zipc(files)
    add("pkg_name_collisions", collisions)

    def inventory_shared():
        n = base_nodes()
        n["inventory"] = [{"name": "a", "image": "s0.png"}, {"name": "b", "image": "s0.png"},
                          {"name": "c", "image": "S0.PNG"}]
        return zipc(S(nodes=n))
    add("pkg_inventory_shared_image", inventory_shared)
    add("pkg_inventory_keep_size", inventory_shared, ["--keep-size"])

    # --- couverture et titre
    add("pkg_cover_from_metadata", lambda: zipc(
        S(meta={"title": "T", "uuid": "u1", "image": "art/c.png"})
        + [("art/c.png", png(300, 200, (1, 2, 3, 255)))]))
    add("pkg_cover_metadata_missing", lambda: zipc(
        S(meta={"title": "T", "uuid": "u1", "image": "absent.png"})))
    add("pkg_cover_fallback_cover_png", lambda: zipc(
        without(S(), "title.png") + [("cover.png", png(50, 90))]))
    add("pkg_cover_empty_title_png", lambda: zipc(
        replace(S(), "title.png", b"") + [("cover.png", png(90, 50))]))
    add("pkg_no_cover", lambda: zipc(without(S(), "title.png")))
    add("pkg_cover_portrait_tall", lambda: zipc(replace(S(), "title.png", png(30, 400))))
    add("pkg_title_mp3", lambda: zipc(S() + [("title.mp3", mp3_frames(n=40))]))
    add("pkg_title_mp3_48k", lambda: zipc(S() + [("title.mp3", mp3_frames(0xFB, 0x94, 40))]))
    add("pkg_title_mp3_garbage", lambda: zipc(S() + [("title.mp3", b"garbage")]))
    add("pkg_title_folded", lambda: zipc(
        without(S(), "title.png") + [("TITLE.PNG", png(40, 40)), ("Title.MP3", MP3)]))

    def cover_jpeg():
        import io
        b = io.BytesIO()
        Image.new("RGB", (64, 48), (10, 20, 30)).save(b, "JPEG")
        return zipc(replace(S(), "title.png", b.getvalue()))
    cases.append(mc.Case("pkg_cover_jpeg_delegated", cover_jpeg, (), [mc.TAG_DELEGATED]))

    # --- metadonnees : titre, categorie, uuid
    for i, meta in enumerate([
        {"title": "", "uuid": "u1"},
        {"title": 12, "category": 3.5, "uuid": "u1"},
        {"title": True, "category": True, "uuid": "u1"},
        {"title": ["a", 'b"\'', None, 1.5e16], "category": ["x"], "uuid": "u1"},
        {"title": {"k": "é\xa0\n"}, "category": {"k": 1}, "uuid": "u1"},
        {"title": "  Titre\tsur\nlignes  ", "category": "Conte – “nuit”", "uuid": "u1"},
        {"title": "T", "uuid": "   "},
        {"title": "T", "uuid": 42},
        {"title": "T", "uuid": "Ünïcødé-ü"},
        {"title": "日本語", "category": "", "uuid": "u1"},
        {"title": None, "category": None},
        {"title": "T", "uuid": "u1", "image": ""},
        {"title": "T", "uuid": "u1", "image": 5},
    ]):
        add("pkg_meta_%d" % i, lambda meta=meta: zipc(S(meta=meta)))

    # --- images : illisibles, reutilisees ; audio absent avec --allow-missing
    def unreadable_twice():
        n = base_nodes()
        n["stages"]["s1"] = {"image": "s0.png", "audio": "s0.mp3", "control": {}}
        n["actions"]["a0"].append({"stage": "s1"})
        return zipc(replace(S(nodes=n), "images/s0.png", b"\x89PNG\r\n\x1a\nbroken"))
    add("pkg_unreadable_cached", unreadable_twice)

    def missing_many():
        n = base_nodes()
        for i in range(7):
            n["stages"]["m%d" % i] = {"image": "no%d.png" % i, "audio": "no%d.mp3" % i,
                                      "control": {}}
            n["actions"]["a0"].append({"stage": "m%d" % i})
        return zipc(S(nodes=n))
    add("pkg_missing_many", missing_many)
    add("pkg_missing_many_allowed", missing_many, ["--allow-missing"])
    add("pkg_keep_size_selector", lambda: zipc(S(nodes=mc.two_node_choices())),
        ["--keep-size", "--selector", "image"])
    return cases


def args_of(argv):
    kw = {"keep_size": False, "selector": "carousel", "allow_missing": False}
    it = iter(argv)
    for a in it:
        if a == "--keep-size":
            kw["keep_size"] = True
        elif a == "--allow-missing":
            kw["allow_missing"] = True
        elif a == "--selector":
            kw["selector"] = next(it)
        elif a != "--plain":
            raise ValueError(a)
    return kw


def run(case, data, tmp):
    src = os.path.join(tmp, "in.zip")
    with open(src, "wb") as f:
        f.write(data)
    out = os.path.join(tmp, "out.plain.pk")
    if os.path.exists(out):
        os.remove(out)
    kw = args_of(case.args)
    warnings = []
    exp = {"id": case.id, "tags": case.tags, **kw}
    try:
        t.convert(src, out, thumb_fit="bandes",
                  progress=lambda e: warnings.append(e["code"]) if e["event"] == "warning" else None,
                  **kw)
    except t.ConvError as e:
        exp["error"] = e.code
        return exp
    except Exception as e:  # noqa: BLE001  (exception non rattrapee de Python : internal_error)
        exp["error"] = "python_crash:%s" % type(e).__name__
        return exp
    with zipfile.ZipFile(out) as z:
        exp["entries"] = [[n, hashlib.sha256(z.read(n)).hexdigest()] for n in z.namelist()]
    with open(out, "rb") as f:
        exp["pk_sha256"] = hashlib.sha256(f.read()).hexdigest()
    exp["warnings"] = warnings
    return exp


def main():
    # create_system (octet du repertoire central) vaut 0 sous Windows seulement : le port Rust
    # reproduit le Python de l'installateur, sous Windows.
    if sys.platform != "win32":
        sys.exit("vecteurs a produire sous Windows (create_system = 0 de zipfile)")
    if sys.version_info[:3] != (3, 14, 8) or Image.__version__ != "12.3.0":
        sys.exit("vecteurs a produire avec Python 3.14.8 + Pillow 12.3.0 (oracle de build-converter.ps1)")
    builds = mc.corpus_cases() + package_cases()
    expected = []
    tmp = tempfile.mkdtemp(prefix="package_cases_")
    try:
        with zipfile.ZipFile(OUT + ".part", "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
            for case in builds:
                data = case.build()
                expected.append(run(case, data, tmp))
                z.writestr(zipfile.ZipInfo("cases/%s.zip" % case.id, FIXED_DATE), data,
                           zipfile.ZIP_DEFLATED, 9)
            body = '{"python": "%d.%d.%d", "pillow": "%s", "cases": [\n%s\n]}\n' % (
                *sys.version_info[:3], Image.__version__,
                ",\n".join(json.dumps(c, ensure_ascii=True, separators=(",", ":"))
                           for c in expected))
            z.writestr(zipfile.ZipInfo("expected.json", FIXED_DATE), body, zipfile.ZIP_DEFLATED, 9)
            z.writestr(zipfile.ZipInfo("zip_cases.json", FIXED_DATE),
                       json.dumps(zip_cases(), ensure_ascii=True, indent=0) + "\n",
                       zipfile.ZIP_DEFLATED, 9)
        os.replace(OUT + ".part", OUT)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    print("%d cas ecrits dans %s (%d octets)" % (len(expected), OUT, os.path.getsize(OUT)))


if __name__ == "__main__":
    main()
