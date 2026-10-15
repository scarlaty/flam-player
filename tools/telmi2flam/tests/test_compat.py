"""Tests de compat.py : catalogue factice local, aucun acces reseau."""

import contextlib
import io
import json
import os
import sys
import unittest
import urllib.error
import zipfile
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
sys.path.insert(0, HERE)
sys.dont_write_bytecode = True

import compat  # noqa: E402
from test_telmi2flam import Base, std_files  # noqa: E402


class FakeResp:
    def __init__(self, data, offset=0, ranged=False):
        self.status = 206 if ranged else 200
        self._b = io.BytesIO(data[offset:])
        self.headers = {"Content-Length": str(len(data) - offset)}
        if ranged:
            self.headers["Content-Range"] = "bytes %d-%d/%d" % (offset, len(data) - 1, len(data))

    def read(self, n=-1):
        return self._b.read(n)

    def close(self):
        pass


class FakeServer:
    """Opener factice : url -> octets, gere Range ; compte les requetes."""

    def __init__(self, files, fail_first=0, range_ok=True):
        self.files, self.calls, self.ranges = files, 0, []
        self.fail_first, self.range_ok = fail_first, range_ok

    def __call__(self, url, headers, timeout):
        self.calls += 1
        if self.calls <= self.fail_first:
            raise urllib.error.URLError("coupure simulee")
        if url not in self.files:
            raise urllib.error.HTTPError(url, 404, "nf", {}, None)
        data = self.files[url]
        rng = headers.get("Range")
        self.ranges.append(rng)
        if rng and self.range_ok:
            off = int(rng.split("=")[1].rstrip("-"))
            return FakeResp(data, off, True)
        return FakeResp(data)


class CompatBase(Base):
    def setUp(self):
        super().setUp()
        self.cache = os.path.join(self.tmp, "cache")
        self.sleeps = []

    def zip_bytes(self, files):
        with open(self.mkzip(files, "tmp%d" % len(os.listdir(self.tmp))), "rb") as f:
            return f.read()

    def entry(self, uuid, url=None, version="1", **kw):
        e = {"source": "cat", "uuid": uuid, "version": version, "title": "Hist " + uuid,
             "license": "CC", "category": "test", "download": url, "size": None, "path": None}
        e.update(kw)
        return e

    def campaign(self, entries, **kw):
        kw.setdefault("log", lambda *a: None)
        kw.setdefault("cache", self.cache)
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            return compat.run_campaign(entries, **kw)

    def process(self, entry, server):
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            return compat.process_story(entry, self.cache, opener=server, sleep=self.sleeps.append)


class TestCatalog(CompatBase):
    def test_tolerant(self):
        p = os.path.join(self.tmp, "cat.json")
        with open(p, "w", encoding="utf-8") as f:
            json.dump({"data": [{"uuid": "a", "version": 2, "title": "A", "download": "u",
                                 "licence": "CC-BY", "categorie": "x"},
                                {"uuid": "b"}, {"title": "sans uuid"}, "bizarre"]}, f)
        es = compat.load_catalog(p)
        self.assertEqual([e["uuid"] for e in es], ["a", "b"])
        self.assertEqual(es[0]["version"], "2")
        self.assertEqual(es[0]["license"], "CC-BY")
        self.assertEqual(es[1]["title"], "b")
        self.assertIsNone(es[1]["download"])
        with open(p, "w") as f:
            f.write('{"pas": "de data"}')
        with self.assertRaises(ValueError):
            compat.load_catalog(p)

    def test_folder_and_select(self):
        d = os.path.join(self.tmp, "zips")
        os.makedirs(d)
        with zipfile.ZipFile(os.path.join(d, "x.zip"), "w") as z:
            z.writestr("metadata.json", json.dumps({"uuid": "U-X", "title": "Xylo"}))
        with open(os.path.join(d, "y.zip"), "wb") as f:
            f.write(b"pas un zip")
        open(os.path.join(d, "notes.txt"), "w").close()
        es = compat.scan_folder(d)
        self.assertEqual([(e["uuid"], e["title"]) for e in es], [("U-X", "Xylo"), ("y", "y")])
        self.assertEqual([e["uuid"] for e in compat.select(es, ["xyl"])], ["U-X"])
        self.assertEqual([e["uuid"] for e in compat.select(es, ["U-*"])], ["U-X"])
        self.assertEqual(len(compat.select(es, limit=1)), 1)
        self.assertEqual(len(compat.collect([], [d, d])), 2)


class TestDownload(CompatBase):
    def test_resume_and_skip(self):
        data = self.zip_bytes(std_files())
        srv = FakeServer({"http://x/a.zip": data})
        e = self.entry("a", "http://x/a.zip")
        arch = os.path.join(self.cache, "archives")
        os.makedirs(arch)
        with open(os.path.join(arch, "a-1.zip.part"), "wb") as f:
            f.write(data[:100])
        path, _ = compat.download(e, arch, opener=srv, sleep=self.sleeps.append)
        self.assertEqual(srv.ranges, ["bytes=100-"])
        with open(path, "rb") as f:
            self.assertEqual(f.read(), data)
        self.assertFalse(os.path.exists(path + ".part"))
        # deja complet : aucune requete
        path, msg = compat.download(e, arch, opener=srv)
        self.assertEqual(srv.calls, 1)
        self.assertIn("deja", msg)
        # fichier tronque : retelecharge entierement
        with open(path, "wb") as f:
            f.write(data[:10])
        compat.download(e, arch, opener=srv)
        self.assertEqual(os.path.getsize(path), len(data))

    def test_server_ignores_range(self):
        data = b"PK" + bytes(500)
        srv = FakeServer({"u": data}, range_ok=False)
        arch = os.path.join(self.cache, "archives")
        os.makedirs(arch)
        with open(os.path.join(arch, "a-1.zip.part"), "wb") as f:
            f.write(b"xxxx")
        path, _ = compat.download(self.entry("a", "u"), arch, opener=srv)
        self.assertEqual(os.path.getsize(path), len(data))

    def test_retries_then_fail(self):
        data = b"PK" + bytes(50)
        arch = os.path.join(self.cache, "archives")
        srv = FakeServer({"u": data}, fail_first=2)
        path, msg = compat.download(self.entry("a", "u"), arch, opener=srv,
                                    sleep=self.sleeps.append)
        self.assertIn("tentative 3", msg)
        self.assertEqual(len(self.sleeps), 2)
        srv = FakeServer({"u": data}, fail_first=9)
        with self.assertRaises(compat.DownloadError):
            compat.download(self.entry("b", "u"), arch, opener=srv, sleep=lambda s: None)
        self.assertEqual(srv.calls, 3)
        with self.assertRaises(compat.DownloadError):
            compat.download(self.entry("c", None), arch)

    def test_size_mismatch(self):
        srv = FakeServer({"u": b"PK" + bytes(50)})
        with self.assertRaises(compat.DownloadError):
            compat.download(self.entry("a", "u", size=9999), os.path.join(self.cache, "a"),
                            opener=srv, sleep=lambda s: None)


class TestPipeline(CompatBase):
    def test_success(self):
        srv = FakeServer({"u": self.zip_bytes(std_files())})
        rec = self.process(self.entry("ok1", "u"), srv)
        self.assertEqual(rec["status"], "OK", rec["steps"])
        self.assertEqual([rec["steps"][s]["status"] for s in compat.STEPS],
                         ["ok", "ok", "ok", "skipped"])
        self.assertTrue(os.path.isfile(os.path.join(self.cache, "converted", "ok1-1.plain.pk")))

    def test_missing_audio_fails_conversion(self):
        files = [f for f in std_files() if f[0] != "audios/s0.mp3"]
        srv = FakeServer({"u": self.zip_bytes(files)})
        rec = self.process(self.entry("w", "u"), srv)
        self.assertEqual(rec["status"], "ECHEC_CONVERSION")

    def test_warn_status(self):
        import telmi2flam
        real = telmi2flam.convert

        def fake_convert(zip_path, out_path=None, **kw):
            telmi2flam._warn("caractere remplace")
            return real(zip_path, out_path)
        srv = FakeServer({"u": self.zip_bytes(std_files())})
        with mock.patch.object(telmi2flam, "convert", fake_convert):
            rec = self.process(self.entry("w", "u"), srv)
        self.assertEqual(rec["status"], "OK_AVERTISSEMENTS")
        self.assertEqual(rec["steps"]["convert"]["status"], "warn")
        self.assertIn("caractere remplace", rec["steps"]["convert"]["messages"])

    def test_corrupt_archive(self):
        srv = FakeServer({"u": b"ceci n'est pas un zip"})
        rec = self.process(self.entry("bad", "u"), srv)
        self.assertEqual(rec["status"], "ECHEC_CONVERSION")
        self.assertEqual(rec["steps"]["validate"]["status"], "skipped")
        self.assertIn("BadZipFile", rec["steps"]["convert"]["messages"][0])

    def test_conversion_failure(self):
        srv = FakeServer({"u": self.zip_bytes(std_files(nodes=False))})
        rec = self.process(self.entry("noNodes", "u"), srv)
        self.assertEqual(rec["status"], "ECHEC_CONVERSION")
        self.assertIn("ConvError", rec["steps"]["convert"]["messages"][0])

    def test_validation_failure(self):
        import validate

        def fake_main(pk):
            print("=== x ===\n  [OK]   a\n  [WARN] w1\n  [ERR]  uuid.bin absent")
            return 1
        srv = FakeServer({"u": self.zip_bytes(std_files())})
        with mock.patch.object(validate, "main", fake_main):
            rec = self.process(self.entry("v", "u"), srv)
        self.assertEqual(rec["status"], "ECHEC_VALIDATION")
        self.assertEqual(rec["steps"]["validate"]["messages"], ["uuid.bin absent"])

    def test_download_failure(self):
        rec = self.process(self.entry("dl", "http://absent/x"), FakeServer({}))
        self.assertEqual(rec["status"], "ECHEC_TELECHARGEMENT")
        self.assertEqual(rec["steps"]["convert"]["status"], "skipped")

    def test_local_folder_entry(self):
        p = self.mkzip(std_files(), "loc")
        rec = self.process(self.entry("loc", None, path=p, version="local"), FakeServer({}))
        self.assertEqual(rec["status"], "OK")
        self.assertEqual(rec["steps"]["download"]["status"], "skipped")


class TestCampaign(CompatBase):
    def setUp(self):
        super().setUp()
        self.zips = {}
        for name, files in (("good", std_files()), ("noNodes", std_files(nodes=False))):
            self.zips[name] = self.mkzip(files, name)
        self.zips["corrupt"] = os.path.join(self.tmp, "corrupt.zip")
        with open(self.zips["corrupt"], "wb") as f:
            f.write(b"junk")
        self.entries = [self.entry(n, None, path=p, version="local")
                        for n, p in self.zips.items()]

    def test_report(self):
        doc = self.campaign(self.entries)
        st = {s["uuid"]: s["status"] for s in doc["stories"]}
        self.assertEqual(st, {"good": "OK", "noNodes": "ECHEC_CONVERSION",
                              "corrupt": "ECHEC_CONVERSION"})
        d = os.path.join(self.cache, "compat")
        with open(os.path.join(d, "report.json"), encoding="utf-8") as f:
            js = json.load(f)
        self.assertEqual(len(js["stories"]), 3)
        for k in ("source", "uuid", "version", "title", "license", "category", "steps", "status"):
            self.assertIn(k, js["stories"][0])
        with open(os.path.join(d, "report.md"), encoding="utf-8") as f:
            md = f.read()
        self.assertIn("ECHEC_CONVERSION : 2", md)
        self.assertIn("Echecs par cause", md)
        self.assertIn("| Hist good |", md)

    def test_resume_and_force(self):
        calls = []
        real = compat.do_convert

        def counting(z, o):
            calls.append(z)
            return real(z, o)
        with mock.patch.object(compat, "do_convert", counting):
            self.campaign(self.entries)
            self.assertEqual(len(calls), 3)
            self.campaign(self.entries)
            self.assertEqual(len(calls), 3)              # rien refait
            self.campaign(self.entries, force=True)
            self.assertEqual(len(calls), 6)
            with mock.patch.object(compat, "converter_hash", lambda root=None: "autre"):
                self.campaign(self.entries)              # convertisseur modifie
            self.assertEqual(len(calls), 9)

    def test_failed_download_is_retried(self):
        e = self.entry("dl", "http://absent/x")
        calls = []
        real = compat.process_story

        def counting(entry, cache, **kw):
            calls.append(1)
            return real(entry, cache, sleep=lambda s: None, opener=FakeServer({}))
        with mock.patch.object(compat, "process_story", counting):
            self.assertEqual(self.campaign([e])["stories"][0]["status"], "ECHEC_TELECHARGEMENT")
            self.campaign([e])
        self.assertEqual(len(calls), 2)

    def test_run_step(self):
        good = self.entries[:1]
        seen = {}

        def explore(player, story, timeout_s=120, seed=0, max_steps=200):
            seen.update(player=player, story=story, timeout_s=timeout_s, seed=seed,
                        max_steps=max_steps)
            return {"status": "ok"}
        doc = self.campaign(good, explore=explore, player_exe="p.exe", seed=7)
        self.assertEqual(doc["stories"][0]["steps"]["run"]["status"], "ok")
        self.assertEqual(seen["player"], "p.exe")
        self.assertEqual(seen["seed"], 7)
        self.assertTrue(seen["story"].endswith("good-local.plain.pk"))
        for status, exc in (("lua_error", None), ("timeout", None), (None, RuntimeError("boum"))):
            def bad(player, story, timeout_s=120, seed=0, max_steps=200):
                if exc:
                    raise exc
                return {"status": status, "message": "detail"}
            doc = self.campaign(good, explore=bad, force=True)
            s = doc["stories"][0]
            self.assertEqual(s["status"], "ECHEC_EXECUTION")
            self.assertEqual(s["steps"]["run"]["status"], "fail")

    def test_run_skipped_then_done_later(self):
        good = self.entries[:1]
        doc = self.campaign(good)
        self.assertEqual(doc["stories"][0]["steps"]["run"]["status"], "skipped")
        doc = self.campaign(good, explore=lambda *a, **k: {"status": "ok"})
        self.assertEqual(doc["stories"][0]["steps"]["run"]["status"], "ok")

    def test_jobs(self):
        doc = self.campaign(self.entries, jobs=2)
        self.assertEqual([s["uuid"] for s in doc["stories"]], ["good", "noNodes", "corrupt"])
        self.assertEqual(doc["stories"][0]["status"], "OK")

    def test_cli_with_catalog_file(self):
        cat = os.path.join(self.tmp, "cat.json")
        with open(cat, "w") as f:
            json.dump({"data": [{"uuid": "z", "version": "1", "title": "Z"}]}, f)
        with contextlib.redirect_stdout(io.StringIO()):
            rc = compat.main(["--catalog", cat, "--cache", self.cache, "--only", "z"])
        self.assertEqual(rc, 1)    # pas d'URL de telechargement : echec attendu
        with open(os.path.join(self.cache, "compat", "report.json")) as f:
            self.assertEqual(json.load(f)["stories"][0]["status"], "ECHEC_TELECHARGEMENT")

    def test_converter_hash_stable(self):
        self.assertEqual(compat.converter_hash(), compat.converter_hash())


FULL = {"status": "ok", "errors": [], "scenes_visited": 3, "scenes_total": 4,
        "end_reached": True, "duration_s": 1.5}


class TestReview(CompatBase):
    def setUp(self):
        super().setUp()
        self.entries = []
        for n in ("a", "b", "c"):
            p = self.mkzip(std_files(), n)
            self.entries.append(self.entry(n, None, path=p, version="local",
                                           fingerprint=compat._file_fingerprint(p)))

    def report(self):
        with open(os.path.join(self.cache, "compat", "report.json"), encoding="utf-8") as f:
            return json.load(f)

    def test_selection_keeps_previous(self):
        self.campaign(self.entries)
        doc = self.campaign(self.entries[:1], force=True)
        self.assertEqual([s["uuid"] for s in doc["stories"]], ["b", "c", "a"])
        self.assertEqual([s["uuid"] for s in self.report()["stories"]], ["b", "c", "a"])
        self.assertFalse(any(s["stale"] for s in doc["stories"]))
        # autre convertisseur : les fiches non selectionnees sont marquees perimees
        with mock.patch.object(compat, "converter_hash", lambda root=None: "nouveau"):
            doc = self.campaign(self.entries[:1])
        st = {s["uuid"]: s["stale"] for s in doc["stories"]}
        self.assertEqual(st, {"a": False, "b": True, "c": True})
        with open(os.path.join(self.cache, "compat", "report.md"), encoding="utf-8") as f:
            self.assertIn("(perime)", f.read())

    def test_run_details(self):
        good = self.entries[:1]
        doc = self.campaign(good, explore=lambda *a, **k: dict(FULL), player_exe="p")
        r = doc["stories"][0]["steps"]["run"]
        self.assertEqual((r["status"], r["scenes_visited"], r["scenes_total"],
                          r["end_reached"], r["duration_s"]), ("ok", 3, 4, True, 1.5))
        with open(os.path.join(self.cache, "compat", "report.md"), encoding="utf-8") as f:
            md = f.read()
        self.assertIn("3/4", md)
        self.assertIn("oui", md)
        bad = dict(FULL, status="lua_error", errors=["Erreur Lua: boum"], end_reached=False)
        doc = self.campaign(good, explore=lambda *a, **k: dict(bad), force=True)
        s = doc["stories"][0]
        self.assertEqual(s["status"], "ECHEC_EXECUTION")
        self.assertIn("Erreur Lua: boum", s["steps"]["run"]["messages"])
        self.assertIs(s["steps"]["run"]["end_reached"], False)

    def test_unexpected_exceptions(self):
        import http.client
        for url in (123, ["x"], "http://exemple/\x7f"):
            rec = self.process(self.entry("e", url), FakeServer({}))
            self.assertEqual(rec["status"], "ECHEC_TELECHARGEMENT", url)

        def boom(url, headers, timeout):
            raise http.client.IncompleteRead(b"ab")
        rec = self.process(self.entry("e2", "u"), boom)
        self.assertEqual(rec["status"], "ECHEC_TELECHARGEMENT")
        # filet de securite : plantage imprevu pendant la conversion
        with mock.patch.object(compat, "do_convert", side_effect=RuntimeError("zut")):
            rec = self.process(self.entry("e3", None, path=self.entries[0]["path"]),
                               FakeServer({}))
        self.assertEqual(rec["status"], "ECHEC_CONVERSION")
        self.assertIn("zut", rec["steps"]["convert"]["messages"][0])
        # et la campagne (y compris en parallele) va au bout et ecrit le rapport
        ents = [self.entry("n%d" % i, 42) for i in range(2)] + self.entries[:1]
        doc = self.campaign(ents, jobs=2)
        self.assertEqual(len(doc["stories"]), 3)
        self.assertTrue(os.path.isfile(os.path.join(self.cache, "compat", "report.md")))

    def test_incremental_report(self):
        seen = []
        real = compat.do_validate

        def spy(pk):
            try:
                seen.append(len(self.report()["stories"]))
            except OSError:
                seen.append(0)
            return real(pk)
        with mock.patch.object(compat, "do_validate", spy):
            self.campaign(self.entries)
        self.assertEqual(seen, [0, 1, 2])           # report.json ecrit apres chaque histoire
        self.assertFalse(os.path.exists(os.path.join(self.cache, "compat", "report.json.tmp")))

    def test_local_zip_replaced(self):
        calls = []
        real = compat.do_convert

        def counting(z, o):
            calls.append(z)
            return real(z, o)
        ent = compat.scan_folder(os.path.dirname(self.entries[0]["path"]))
        ent = [e for e in ent if e["uuid"] == "u1"][:1] or ent[:1]
        with mock.patch.object(compat, "do_convert", counting):
            self.campaign(ent)
            self.campaign(ent)
            self.assertEqual(len(calls), 1)
            p = ent[0]["path"]
            with open(p, "ab") as f:
                f.write(b"\0")                       # zip modifie (taille differente)
            ent2 = compat.scan_folder(os.path.dirname(p))
            self.campaign([e for e in ent2 if e["path"] == p])
            self.assertEqual(len(calls), 2)

    def test_updated_at(self):
        cat = os.path.join(self.tmp, "c.json")
        with open(cat, "w") as f:
            json.dump({"data": [{"uuid": "q", "updated_at": "2026-01-02"}, {"uuid": "r"}]}, f)
        es = compat.load_catalog(cat)
        self.assertEqual([e["fingerprint"] for e in es], ["2026-01-02", None])

    def test_416_without_size_checks_zip(self):
        arch = os.path.join(self.cache, "archives")
        os.makedirs(arch)
        part = os.path.join(arch, "a-1.zip.part")

        def gone(url, headers, timeout):
            raise urllib.error.HTTPError(url, 416, "range", {}, None)
        with open(part, "wb") as f:
            f.write(b"pas un zip du tout")
        with self.assertRaises(compat.DownloadError):
            compat.download(self.entry("a", "u"), arch, opener=gone, sleep=lambda s: None)
        self.assertFalse(os.path.exists(part))
        with open(part, "wb") as f:
            f.write(self.zip_bytes(std_files()))
        path, _ = compat.download(self.entry("a", "u"), arch, opener=gone)
        self.assertTrue(zipfile.is_zipfile(path))

    def test_cli_negative_limit_and_md_escape(self):
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            compat.main(["--folder", self.tmp, "--limit", "-1"])
        doc = {"generated": "t", "converter_hash": "h", "stories": [
            {"title": "T", "uuid": "a|b", "version": "1", "status": "OK", "key": "k",
             "steps": {}}]}
        self.assertIn("a\\|b", compat.render_markdown(doc))


class TestRunInterpretation(CompatBase):
    """Etape run : interpretation des resultats d'explore() (faux explore)."""

    def res(self, **kw):
        base = {"status": "ok", "done_reason": "complete", "errors": [], "scenes_visited": 10,
                "scenes_total": 10, "coverage": 1.0, "end_reached": True, "duration_s": 2.0,
                "seed": 0, "steps": 40, "ends": 2, "dead_end_backs": 0,
                "dead_end_scenes": [], "scenes": ["s%d" % i for i in range(10)]}
        base.update(kw)
        return base

    def go(self, res, **kw):
        return compat.do_run("x.pk", lambda *a, **k: dict(res), "p", 60, 0, 100, **kw)

    def test_cases(self):
        cases = [
            (dict(), "ok"),
            (dict(status="timeout", done_reason="timeout", scenes_visited=50,
                  scenes_total=200), "warn"),
            (dict(status="timeout", done_reason="timeout", scenes_visited=3,
                  scenes_total=200), "warn"),
            (dict(status="timeout", done_reason="timeout", scenes_visited=2), "fail"),
            (dict(status="timeout", done_reason="timeout", scenes_visited=1), "fail"),
            (dict(status="timeout", done_reason="timeout", scenes_visited=0), "fail"),
            (dict(done_reason="max_steps", scenes_visited=5, scenes_total=10), "warn"),
            (dict(done_reason="max_steps", scenes_visited=10, scenes_total=10), "ok"),
            (dict(status="stuck", done_reason="stuck"), "fail"),
            (dict(status="lua_error", done_reason="error", errors=["Erreur Lua: boum"]), "fail"),
            (dict(status="crash", done_reason="exit"), "fail"),
            (dict(status="bizarre"), "fail"),
            (dict(status="timeout", done_reason="timeout", scenes_visited=50,
                  errors=["Erreur Lua: x"]), "fail"),
        ]
        for kw, want in cases:
            with self.subTest(kw=kw):
                self.assertEqual(self.go(self.res(**kw))["status"], want)

    def test_partial_message_and_global_status(self):
        r = self.go(self.res(status="timeout", done_reason="timeout", scenes_visited=50,
                              scenes_total=200))
        self.assertIn("exploration partielle 50/200 scènes", r["messages"])
        rec = {"steps": {"download": {"status": "ok"}, "convert": {"status": "ok"},
                         "validate": {"status": "ok"}, "run": r}}
        self.assertEqual(compat.global_status(rec["steps"]), "OK_AVERTISSEMENTS")
        rec["steps"]["run"] = self.go(self.res(status="stuck", done_reason="stuck"))
        self.assertEqual(compat.global_status(rec["steps"]), "ECHEC_EXECUTION")

    def test_copied_fields(self):
        r = self.go(self.res(dead_end_backs=2, dead_end_scenes=["n1 why=x"]))
        for k, v in (("done_reason", "complete"), ("steps", 40), ("ends", 2), ("seed", 0),
                     ("seeds", [0]), ("dead_end_backs", 2), ("scenes_visited", 10),
                     ("scenes_total", 10), ("coverage", 1.0), ("end_reached", True),
                     ("duration_s", 2.0), ("dead_end_scenes", ["n1 why=x"])):
            self.assertEqual(r[k], v, k)

    def test_benign_errors(self):
        errs = ["Erreur Lua: boum", "warning: SDL audio dummy error"]
        r = self.go(self.res(status="lua_error", done_reason="error", errors=errs),
                     benign=(r"sdl audio",))
        self.assertEqual(r["status"], "fail")           # une erreur reste non benigne
        self.assertEqual([g["signature"] for g in r["error_signatures"]],
                         ["Erreur Lua: boum"])
        r = self.go(self.res(status="lua_error", done_reason="error", errors=errs[1:]),
                     benign=(r"sdl audio",))
        self.assertEqual(r["status"], "warn")            # tout est benin : interrompu mais pas fail
        self.assertNotIn("error_signatures", r)
        self.assertEqual(self.go(self.res(errors=errs[1:]), benign=(r"SDL AUDIO",))["status"],
                         "ok")                           # ok + benin : rien a signaler
        self.assertEqual(compat.BENIGN_ERRORS, ())       # liste fournie vide au depart

    def test_error_signature(self):
        a = r"C:\Users\x\Temp\flam_explore_ab12\s\script\story.lua:123: attempt to index nil"
        b = "/tmp/flam_explore_zz/s/script/story.lua:7: attempt to index nil"
        self.assertEqual(compat.error_signature(a), compat.error_signature(b))
        self.assertNotIn("123", compat.error_signature(a))
        self.assertNotEqual(compat.error_signature("boum 0x1f"), compat.error_signature("pouf"))
        g = compat.group_errors([a, b, "autre"])
        self.assertEqual([x["count"] for x in g], [2, 1])

    def test_seeds_merge(self):
        calls = []

        def explore(player, story, timeout_s=0, seed=0, max_steps=0):
            calls.append(seed)
            return self.res(seed=seed, scenes=["a", "b"] if seed == 5 else ["b", "c", "d"],
                            scenes_visited=2 if seed == 5 else 3, scenes_total=6,
                            coverage=0, steps=10, ends=1, dead_end_backs=1,
                            dead_end_scenes=["n%d" % seed], end_reached=seed == 6,
                            done_reason="max_steps", duration_s=1.0)
        r = compat.do_run("x.pk", explore, "p", 60, 5, 100, seeds=3)
        self.assertEqual(calls, [5, 6, 7])
        self.assertEqual((r["seeds"], r["scenes_visited"], r["coverage"]), ([5, 6, 7], 4, 0.667))
        self.assertEqual((r["steps"], r["ends"], r["dead_end_backs"], r["duration_s"]),
                         (30, 3, 3, 3.0))
        self.assertEqual(r["dead_end_scenes"], ["n5", "n6", "n7"])
        self.assertTrue(r["end_reached"])
        self.assertNotIn("seed", r)
        self.assertEqual(len(r["runs"]), 3)
        self.assertEqual(r["status"], "warn")            # max_steps partiel
        self.assertEqual(r["params"], {"timeout_s": 60, "max_steps": 100, "seeds": 3})

    def test_seeds_stop_at_first_failure_and_exception(self):
        calls = []

        def explore(player, story, timeout_s=0, seed=0, max_steps=0):
            calls.append(seed)
            return self.res(status="stuck", done_reason="stuck") if seed == 1 else self.res()
        r = compat.do_run("x.pk", explore, "p", 60, 0, 100, seeds=4)
        self.assertEqual((calls, r["status"]), ([0, 1], "fail"))
        self.assertTrue(r["messages"][0].startswith("[graine 1]"))

        def boom(*a, **k):
            raise RuntimeError("boum")
        r = compat.do_run("x.pk", boom, "p", 60, 0, 100, seeds=2)
        self.assertEqual(r["status"], "fail")
        self.assertIn("boum", r["messages"][0])

    def test_status_json_and_report(self):
        good = self.mkzip(std_files(), "good")
        e = self.entry("good", None, path=good, version="local")
        self.campaign([e], explore=lambda *a, **k: self.res(
            status="timeout", done_reason="timeout", scenes_visited=5, scenes_total=9,
            scenes=["s%d" % i for i in range(5)], errors=[], dead_end_backs=2), player_exe="p", seeds=2)
        with open(os.path.join(self.cache, "compat", "status.json"), encoding="utf-8") as f:
            st = json.load(f)
        self.assertEqual(list(st), ["good"])
        self.assertEqual(set(st["good"]), {"version", "title", "status", "summary", "coverage",
                                           "run", "stale"})
        self.assertEqual(st["good"]["status"], "OK_AVERTISSEMENTS")
        self.assertEqual(st["good"]["run"], "warn")
        self.assertIn("exploration partielle", st["good"]["summary"])
        self.assertEqual(st["good"]["coverage"]["scenes_visited"], 5)   # union des scenes
        with open(os.path.join(self.cache, "compat", "report.md"), encoding="utf-8") as f:
            md = f.read()
        for needle in ("timeout", "5/9", "Impasses", "Graine(s)", "Couverture"):
            self.assertIn(needle, md)

    def test_status_json_failures_and_skipped(self):
        good = self.mkzip(std_files(), "good")
        bad = os.path.join(self.tmp, "bad.zip")
        with open(bad, "wb") as f:
            f.write(b"junk")
        ents = [self.entry("good", None, path=good, version="local"),
                self.entry("bad", None, path=bad, version="local")]
        self.campaign(ents)
        with open(os.path.join(self.cache, "compat", "status.json"), encoding="utf-8") as f:
            st = json.load(f)
        self.assertEqual((st["good"]["status"], st["good"]["run"], st["good"]["coverage"]),
                         ("OK", "skipped", None))
        self.assertEqual(st["bad"]["status"], "ECHEC_CONVERSION")
        self.assertIn("Conversion en échec", st["bad"]["summary"])

    def test_rerun_when_settings_change(self):
        good = self.mkzip(std_files(), "good")
        e = self.entry("good", None, path=good, version="local",
                       fingerprint=compat._file_fingerprint(good))
        calls = []

        def explore(*a, **k):
            calls.append(k["max_steps"])
            return self.res()
        self.campaign([e], explore=explore, player_exe="p", max_steps=100)
        self.campaign([e], explore=explore, player_exe="p", max_steps=100)
        self.assertEqual(calls, [100])
        self.campaign([e], explore=explore, player_exe="p", max_steps=200)
        self.assertEqual(calls, [100, 200])

    def test_zip_size_limit(self):
        big = self.mkzip(std_files(), "big")
        with self.assertRaises(ValueError):
            compat.check_zip_size(big, max_bytes=10)
        with self.assertRaises(ValueError):
            compat.check_zip_size(big, max_entries=1)
        compat.check_zip_size(big)
        e = self.entry("big", None, path=big, version="local")
        with mock.patch.object(compat, "MAX_UNZIPPED", 10):
            rec = compat.process_story(e, self.cache, max_unzipped=10)
        self.assertEqual(rec["status"], "ECHEC_CONVERSION")
        self.assertIn("trop volumineuse", rec["steps"]["convert"]["messages"][0])

    def test_cli_options(self):
        cat = os.path.join(self.tmp, "cat.json")
        with open(cat, "w") as f:
            json.dump({"data": []}, f)
        seen = {}

        def fake(entries, cache, jobs, force, explore, player, timeout, seed, steps, **kw):
            seen.update(explore=explore, player=player, timeout=timeout, steps=steps, **kw)
            return {"stories": []}
        with mock.patch.object(compat, "run_campaign", fake), \
                mock.patch.object(compat, "_find_player", lambda p=None: "pl.exe"), \
                contextlib.redirect_stdout(io.StringIO()):
            compat.main(["--catalog", cat, "--seeds", "3", "--benign-error", "foo"])
        self.assertIsNotNone(seen["explore"])            # run actif par defaut
        self.assertEqual((seen["player"], seen["seeds"], seen["benign"]), ("pl.exe", 3, ("foo",)))
        self.assertGreaterEqual(seen["timeout"], 300)
        self.assertGreaterEqual(seen["steps"], 1000)
        with mock.patch.object(compat, "run_campaign", fake), \
                mock.patch.object(compat, "_find_player", lambda p=None: None), \
                contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            compat.main(["--catalog", cat])
            self.assertIsNone(seen["explore"])           # player absent : run desactive
            self.assertEqual(compat.main(["--catalog", cat, "--run"]), 2)
        with mock.patch.object(compat, "run_campaign", fake), \
                mock.patch.object(compat, "_find_player", lambda p=None: "pl.exe"), \
                contextlib.redirect_stdout(io.StringIO()):
            compat.main(["--catalog", cat, "--no-run"])
        self.assertIsNone(seen["explore"])


try:
    import explore as _explore
    PLAYER = _explore.find_player()
except Exception:
    PLAYER = None


@unittest.skipIf(PLAYER is None, "flam-player introuvable (FLAM_BUILD_DIR ou build/)")
class TestRunReal(CompatBase):
    def test_real_exploration(self):
        good = self.mkzip(std_files(), "good")
        e = self.entry("good", None, path=good, version="local")
        doc = self.campaign([e], explore=_explore.explore, player_exe=PLAYER,
                            run_timeout=90, max_steps=100, seeds=2)
        run = doc["stories"][0]["steps"]["run"]
        self.assertIn(run["status"], ("ok", "warn"), run["messages"])
        self.assertEqual(run["seeds"], [0, 1])
        for k in ("done_reason", "steps", "ends", "dead_end_backs", "scenes_visited",
                  "scenes_total", "coverage"):
            self.assertIn(k, run)
        self.assertGreaterEqual(run["scenes_visited"], 1)


if __name__ == "__main__":
    unittest.main()
