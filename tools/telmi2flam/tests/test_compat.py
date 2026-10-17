"""Tests de compat.py : catalogue factice local, aucun acces reseau."""

import contextlib
import io
import json
import os
import sys
import time
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
            (dict(status="stuck", done_reason="unresponsive"), "fail"),
            (dict(status="stuck", done_reason="no_trace"), "fail"),
            (dict(status="loop", done_reason="loop", scenes_visited=5, scenes_total=20,
                  end_reached=False), "warn"),
            (dict(done_reason="exhausted", scenes_visited=14, scenes_total=14,
                  end_reached=False), "warn"),
            (dict(done_reason="exhausted", scenes_visited=10, scenes_total=10), "ok"),
            (dict(done_reason="conditions", scenes_visited=26, scenes_total=177,
                  end_reached=True), "warn"),
            (dict(status="lua_error", done_reason="error", errors=["Erreur Lua: boum"]), "fail"),
            (dict(status="crash", done_reason="exit"), "fail"),
            (dict(status="bizarre"), "fail"),
            (dict(status="timeout", done_reason="timeout", scenes_visited=50,
                  errors=["Erreur Lua: x"]), "fail"),
        ]
        for kw, want in cases:
            with self.subTest(kw=kw):
                self.assertEqual(self.go(self.res(**kw))["status"], want)

    def test_boucle_sans_erreur_42(self):
        """#42 : une boucle sans erreur Lua ni crash n'est plus un echec d'execution ;
        `stuck`/`stuck` (resultats d'avant #42) est relu comme une boucle."""
        def judge(**kw):
            base = {"status": "stuck", "done_reason": "stuck", "scenes_visited": 40,
                    "scenes_total": 42, "errors": []}
            base.update(kw)
            return compat.interpret_run(base)
        st, msgs = judge()
        self.assertEqual(st, "warn")
        self.assertIn("exploration partielle 40/42 scènes", msgs)
        self.assertTrue(any("boucle sans nouveauté" in m for m in msgs), msgs)
        self.assertEqual(judge(scenes_visited=1)[0], "fail")
        self.assertEqual(judge(errors=["Erreur Lua: boum"])[0], "fail")
        self.assertEqual(judge(status="loop", done_reason="loop")[0], "warn")
        self.assertEqual(judge(status="loop", done_reason="loop", scenes_visited=2,
                               scenes_total=2)[0], "warn")      # boucle complete
        self.assertEqual(judge(status="loop", done_reason="loop",
                               errors=["Erreur Lua: boum"])[0], "fail")
        self.assertEqual(judge(done_reason="unresponsive")[0], "fail")
        self.assertEqual(judge(done_reason="no_start")[0], "fail")
        # retours de secours : l'histoire s'est figee, pas une boucle (revue #75)
        st, msgs = judge(dead_end_backs=2)                    # ancien stuck/stuck
        self.assertEqual(st, "fail")
        self.assertTrue(any("ancien résultat stuck, non reclassé" in m for m in msgs), msgs)
        self.assertFalse(any("boucle sans erreur" in m for m in msgs), msgs)
        st, msgs = judge(status="loop", done_reason="loop", dead_end_backs=1)
        self.assertEqual(st, "fail")
        self.assertEqual(judge(dead_end_backs=0)[0], "warn")
        st, msgs = judge(status="ok", done_reason="exhausted", scenes_visited=42,
                         end_reached=False)
        self.assertEqual(st, "warn")
        self.assertIn("aucune fin jouée (histoire cyclique ?)", msgs)
        # fiche et status.json : avertissement, raison et couverture
        r = self.go(self.res(status="loop", done_reason="loop", scenes_visited=8,
                              scenes_total=10, end_reached=False, home_escapes=3,
                              scenes=["s%d" % i for i in range(8)]))
        self.assertEqual((r["status"], r["done_reason"], r["raw"][0]["home_escapes"]),
                         ("warn", "loop", 3))
        rec = {"uuid": "u", "version": "1", "title": "t", "status": "OK_AVERTISSEMENTS",
               "steps": {"download": {"status": "ok"}, "convert": {"status": "ok"},
                         "validate": {"status": "ok"}, "run": r}}
        self.assertEqual(compat.global_status(rec["steps"]), "OK_AVERTISSEMENTS")
        doc = compat.build_status({"generated": "g", "converter_hash": "h", "stories": [rec]})
        self.assertEqual(doc["schema_version"], 1)
        story = doc["stories"]["u"]
        self.assertEqual((story["run"], story["run_reason"]), ("warn", "loop"))
        self.assertEqual(story["summary"], "Jouable, boucle sans erreur (8/10 scènes)")
        self.assertEqual(story["coverage"]["scenes_visited"], 8)

    def test_conditions_142(self):
        """#142 : `conditions` = avertissement, scenes bloquees dans le message, la fiche,
        status.json, report.md ; resume de la fiche."""
        blocked = ["s%d>g%d" % (i, i) for i in range(7)] + ["m>h@home"]
        r = self.go(self.res(done_reason="conditions", scenes_visited=26, scenes_total=177,
                              end_reached=True, cond_frontier=8, blocked=blocked,
                              scenes=["s%d" % i for i in range(26)]))
        self.assertEqual(r["status"], "warn")
        self.assertIn("exploration partielle 26/177 scènes", r["messages"])
        self.assertTrue(any(m.startswith("bloquées (scène>cible) : s0>g0; s1>g1") and
                            m.endswith("(+3)") for m in r["messages"]), r["messages"])
        self.assertEqual(r["raw"][0]["cond_frontier"], 8)
        self.assertEqual(r["blocked"], sorted(blocked))
        # sans scenes bloquees : message sans ligne « bloquées »
        r0 = self.go(self.res(done_reason="conditions", scenes_visited=26, scenes_total=177))
        self.assertFalse(any("bloquées" in m for m in r0["messages"]))
        self.assertNotIn("blocked", r0)
        r1 = self.go(self.res(done_reason="exhausted", blocked=["a>b"]))
        self.assertNotIn("blocked", r1)                  # hors arret `conditions`
        rec = {"uuid": "u", "version": "1", "title": "Doudou", "status": "OK_AVERTISSEMENTS",
               "steps": {"download": {"status": "ok"}, "convert": {"status": "ok"},
                         "validate": {"status": "ok"}, "run": r}}
        self.assertEqual(compat._summary(rec), "Jouable, exploration partielle (26/177 scènes)")
        story = compat.build_status({"generated": "g", "converter_hash": "h",
                                     "stories": [rec]})["stories"]["u"]
        self.assertEqual((story["run"], story["run_reason"]), ("warn", "conditions"))
        self.assertEqual(story["blocked"], sorted(blocked))
        rec0 = dict(rec, steps=dict(rec["steps"], run=r0))
        story0 = compat.build_status({"generated": "g", "converter_hash": "h",
                                      "stories": [rec0]})["stories"]["u"]
        self.assertNotIn("blocked", story0)
        md = compat.render_markdown({"generated": "g", "converter_hash": "h", "stories": [rec]})
        self.assertIn("| conditions |", md)
        self.assertIn("## Scenes derriere une condition", md)
        self.assertIn("- Doudou : m>h@home; s0>g0", md)

    def test_conditions_blocked_fusion_graines(self):
        """Cible atteinte par une autre graine : plus bloquee dans la fiche fusionnee."""
        runs = [self.res(done_reason="conditions", scenes_visited=2, scenes_total=3,
                         scenes=["a", "b"], blocked=["b>g", "a>h"], seed=0),
                self.res(done_reason="conditions", scenes_visited=2, scenes_total=3,
                         scenes=["a", "g"], blocked=["b>g"], seed=1)]
        it = iter(runs)
        r = compat.do_run("x.pk", lambda *a, **k: dict(next(it)), "p", 60, 0, 100, seeds=2)
        self.assertEqual(r["blocked"], ["a>h"])

    def test_partial_message_and_global_status(self):
        r = self.go(self.res(status="timeout", done_reason="timeout", scenes_visited=50,
                              scenes_total=200))
        self.assertIn("exploration partielle 50/200 scènes", r["messages"])
        rec = {"steps": {"download": {"status": "ok"}, "convert": {"status": "ok"},
                         "validate": {"status": "ok"}, "run": r}}
        self.assertEqual(compat.global_status(rec["steps"]), "OK_AVERTISSEMENTS")
        rec["steps"]["run"] = self.go(self.res(status="stuck", done_reason="unresponsive"))
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
        self.assertTrue(any("interrompue sur erreur bénigne" in m for m in r["messages"]))
        self.assertEqual(r["benign_ignored"], 1)
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
        self.assertEqual(r["params"], {"timeout_s": 60, "max_steps": 100, "seed": 5, "seeds": 3})

    def test_seeds_stop_at_first_failure_and_exception(self):
        calls = []

        def explore(player, story, timeout_s=0, seed=0, max_steps=0):
            calls.append(seed)
            return self.res(status="stuck", done_reason="unresponsive") if seed == 1 else self.res()
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
            doc = json.load(f)
        self.assertEqual((doc["schema_version"], set(doc)), (1, {
            "schema_version", "generated", "converter_hash", "stories"}))
        st = doc["stories"]
        self.assertEqual(list(st), ["good"])
        self.assertEqual(set(st["good"]), {"version", "title", "status", "summary", "coverage",
                                           "run", "run_reason", "stale"})
        self.assertEqual(st["good"]["run_reason"], "timeout")
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
            st = json.load(f)["stories"]
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


class TestReviewFixes(CompatBase):
    def res(self, **kw):
        base = {"status": "ok", "done_reason": "complete", "errors": [], "scenes_visited": 4,
                "scenes_total": 4, "end_reached": True, "duration_s": 1.0, "steps": 9,
                "ends": 1, "dead_end_backs": 0, "scenes": ["a", "b", "c", "d"]}
        base.update(kw)
        return base

    def entry_for(self, name):
        p = self.mkzip(std_files(), name)
        return self.entry(name, None, path=p, version="local",
                          fingerprint=compat._file_fingerprint(p))

    def report(self):
        with open(os.path.join(self.cache, "compat", "report.json"), encoding="utf-8") as f:
            return json.load(f)

    def test_raw_kept_bounded_and_reinterpreted_without_player(self):
        e = self.entry_for("a")
        errs = ["Erreur Lua: %d " % i + "x" * 500 for i in range(300)]
        calls = []

        def explore(*a, **k):
            calls.append(1)
            return self.res(status="lua_error", done_reason="error", errors=list(errs))
        doc = self.campaign([e], explore=explore, player_exe="p")
        run = doc["stories"][0]["steps"]["run"]
        raw = run["raw"][0]
        self.assertEqual(len(raw["errors"]), 200)
        self.assertTrue(all(len(x) <= 200 for x in raw["errors"]))
        self.assertTrue(raw["errors"][-1].startswith("Erreur Lua: 299"))   # la fin est conservee
        self.assertEqual((raw["status"], raw["done_reason"]), ("lua_error", "error"))
        self.assertEqual(doc["stories"][0]["status"], "ECHEC_EXECUTION")
        # tout devient benin : reinterpretation automatique, player non relance
        doc = self.campaign([e], explore=explore, player_exe="p", benign=(".",))
        self.assertEqual(len(calls), 1)
        s = doc["stories"][0]
        self.assertEqual(s["steps"]["run"]["status"], "warn")
        self.assertEqual(s["status"], "OK_AVERTISSEMENTS")
        self.assertEqual(s["steps"]["run"]["benign_ignored"], 200)
        # sans changement de liste : rien n'est recalcule ; reinterpret=True force
        with mock.patch.object(compat, "reinterpret_run", wraps=compat.reinterpret_run) as m:
            self.campaign([e], explore=explore, player_exe="p", benign=(".",))
            self.assertEqual(m.call_count, 0)
            self.campaign([e], explore=explore, player_exe="p", benign=(".",), reinterpret=True)
            self.assertEqual(m.call_count, 1)
        # sans player du tout (explore None) la reinterpretation reste possible
        doc = self.campaign([e], benign=())
        self.assertEqual(doc["stories"][0]["steps"]["run"]["status"], "fail")
        self.assertEqual(len(calls), 1)

    def test_invalidation_seed_and_legacy(self):
        e = self.entry_for("a")
        calls = []

        def explore(*a, **k):
            calls.append(k["seed"])
            return self.res()
        self.campaign([e], explore=explore, player_exe="p", seed=0)
        self.campaign([e], explore=explore, player_exe="p", seed=0)
        self.assertEqual(calls, [0])
        self.campaign([e], explore=explore, player_exe="p", seed=3)       # graine de depart
        self.assertEqual(calls, [0, 3])
        # fiche anterieure : run fait mais sans params -> refaite
        rp = os.path.join(self.cache, "compat", "report.json")
        doc = self.report()
        del doc["stories"][0]["steps"]["run"]["params"]
        with open(rp, "w", encoding="utf-8") as f:
            json.dump(doc, f)
        self.campaign([e], explore=explore, player_exe="p", seed=3)
        self.assertEqual(calls, [0, 3, 3])
        # sans explore, la fiche existante est simplement reutilisee
        self.campaign([e], explore=None)
        self.assertEqual(calls, [0, 3, 3])

    def test_parallel_run_jobs(self):
        import threading
        ents = [self.entry_for(n) for n in ("a", "b", "c", "d")]
        lock, state = threading.Lock(), {"now": 0, "max": 0}

        def explore(*a, **k):
            with lock:
                state["now"] += 1
                state["max"] = max(state["max"], state["now"])
            time.sleep(1.5)
            with lock:
                state["now"] -= 1
            return self.res()
        doc = self.campaign(ents, explore=explore, player_exe="p", run_jobs=2)
        self.assertEqual(state["max"], 2)
        self.assertEqual([s["status"] for s in doc["stories"]], ["OK"] * 4)
        self.assertEqual(sorted(s["uuid"] for s in self.report()["stories"]), list("abcd"))
        state["max"] = 0
        self.campaign(ents, explore=explore, player_exe="p", force=True)   # defaut : 1 a la fois
        self.assertEqual(state["max"], 1)

    def test_benign_mixed_and_anchored(self):
        r = compat.interpret_run(self.res(status="lua_error", done_reason="error",
                                          errors=["SDL: warning benin", "attempt to index nil"]),
                                 (r"^SDL: warning benin$",))
        self.assertEqual(r[0], "fail")
        self.assertIn("attempt to index nil", r[1])
        r = compat.interpret_run(self.res(status="lua_error", done_reason="error",
                                          errors=["SDL: warning benin"]),
                                 (r"^SDL: warning benin$",))
        self.assertEqual(r[0], "warn")
        # un motif trop large masque une vraie erreur : c'est pourquoi les motifs sont ancres
        r = compat.interpret_run(self.res(status="lua_error", done_reason="error",
                                          errors=["Erreur Lua: attempt to index nil"]),
                                 ("erreur",))
        self.assertEqual(r[0], "warn")

    def test_status_duplicates_by_version(self):
        def story(version, evaluated):
            return {"uuid": "u", "version": version, "title": "T", "status": "OK",
                    "evaluated": evaluated, "key": "k" + version, "steps": {}}
        doc = {"generated": "g", "converter_hash": "h", "stories": [
            story("2", "2026-01-02"), story("10", "2026-01-01"), story("9", "2026-01-03")]}
        self.assertEqual(compat.build_status(doc)["stories"]["u"]["version"], "10")
        doc["stories"] = [story("local", "2026-01-01"), story("beta", "2026-02-01"),
                          story("1.2", "2026-03-01"), story("1.10", "2026-01-01")]
        self.assertEqual(compat.build_status(doc)["stories"]["u"]["version"], "1.10")
        doc["stories"] = [story("local", "2026-02-01"), story("beta", "2026-01-01")]
        self.assertEqual(compat.build_status(doc)["stories"]["u"]["version"], "local")

    def test_player_env_isolated(self):
        import explore as ex
        with open(ex.__file__, encoding="utf-8") as f:
            src = f.read()
        self.assertIn('env["FLAM_SCREENSHOT_AUTO_MS"] = "0"', src)
        self.assertIn('os.path.join(cwd, "screenshot.bmp")', src)


class Killed(BaseException):
    """Simule une mort brutale du processus : n'est pas capturee par `except Exception`."""


class TestAlwaysAFiche(CompatBase):
    """#166 : une histoire produit TOUJOURS une fiche, meme si l'exploration meurt."""

    def good(self):
        return [self.entry("good", None, path=self.mkzip(std_files(), "good"), version="local")]

    def status_json(self):
        with open(os.path.join(self.cache, "compat", "status.json"), encoding="utf-8") as f:
            return json.load(f)["stories"]

    def test_describe_exit_code(self):
        d = compat.describe_exit_code
        self.assertIsNone(d(0))
        self.assertIsNone(d(None))
        self.assertIn("tué de l'extérieur", d(-1))
        self.assertIn("0xFFFFFFFF", d(-1))
        self.assertIn("violation d'accès", d(-1073741819))
        self.assertIn("0xC0000005", d(-1073741819))
        self.assertIn("0xC0000005", d(0xC0000005))
        self.assertIn("arrêt anormal", d(-1073740000))
        self.assertEqual(d(3), "code de sortie 3")

    def test_player_exit_code_translated(self):
        def explore(player, story, timeout_s=120, seed=0, max_steps=200):
            return {"status": "crash", "done_reason": "exit", "exit_code": -1073741819,
                    "errors": ["flam-player arrete seul (code -1073741819)"]}
        doc = self.campaign(self.good(), explore=explore, player_exe="p.exe")
        s = doc["stories"][0]
        run = s["steps"]["run"]
        self.assertEqual(s["status"], "ECHEC_EXECUTION")
        self.assertEqual(run["done_reason"], "exit")
        self.assertTrue(any("violation d'accès" in m for m in run["messages"]), run["messages"])
        self.assertEqual(run["raw"][0]["exit_code"], -1073741819)

    def test_player_killed_minus_one(self):
        def explore(player, story, timeout_s=120, seed=0, max_steps=200):
            return {"status": "crash", "done_reason": "exit", "exit_code": -1}
        run = self.campaign(self.good(), explore=explore, player_exe="p.exe"
                            )["stories"][0]["steps"]["run"]
        self.assertTrue(any("code -1" in m for m in run["messages"]), run["messages"])

    def test_explore_exception_is_crash(self):
        for exc in (RuntimeError("boum"), SystemExit(3), MemoryError()):
            def explore(player, story, timeout_s=120, seed=0, max_steps=200, exc=exc):
                raise exc
            doc = self.campaign(self.good(), explore=explore, force=True)
            s = doc["stories"][0]
            self.assertEqual(s["status"], "ECHEC_EXECUTION")
            self.assertEqual(s["steps"]["run"]["status"], "fail")
            self.assertEqual(s["steps"]["run"]["done_reason"], "crash")
            self.assertEqual(self.status_json()["good"]["run_reason"], "crash")

    def test_exception_around_run_is_caught(self):
        def boom(*a, **k):
            raise ValueError("fiche cassée")
        with mock.patch.object(compat, "do_run", boom):
            doc = self.campaign(self.good(), explore=lambda *a, **k: {}, player_exe="p")
        s = doc["stories"][0]
        self.assertEqual(s["status"], "ECHEC_EXECUTION")
        self.assertEqual(s["steps"]["run"]["done_reason"], "crash")
        self.assertIn("ValueError", s["steps"]["run"]["messages"][0])
        self.assertEqual(self.status_json()["good"]["status"], "ECHEC_EXECUTION")

    def test_process_story_exception_gives_a_record(self):
        def boom(*a, **k):
            raise OSError("disque plein")
        with mock.patch.object(compat, "process_story", boom):
            doc = self.campaign(self.good())
        s = doc["stories"][0]
        self.assertEqual(s["status"], "ECHEC_CONVERSION")
        self.assertIn("disque plein", s["steps"]["convert"]["messages"][0])

    def test_dead_process_leaves_a_provisional_record(self):
        """Processus tue pendant l'exploration : report.json garde une fiche d'echec."""
        def explore(player, story, timeout_s=120, seed=0, max_steps=200):
            raise Killed()
        with self.assertRaises(Killed):
            self.campaign(self.good(), explore=explore, player_exe="p.exe")
        with open(os.path.join(self.cache, "compat", "report.json"), encoding="utf-8") as f:
            stories = json.load(f)["stories"]
        self.assertEqual(len(stories), 1)
        self.assertEqual(stories[0]["status"], "ECHEC_EXECUTION")
        self.assertEqual(stories[0]["steps"]["run"]["done_reason"], "crash")
        self.assertIn("interrompu", stories[0]["steps"]["run"]["messages"][-1])
        st = self.status_json()["good"]
        self.assertEqual((st["status"], st["run_reason"]), ("ECHEC_EXECUTION", "crash"))
        # sans --force, la fiche provisoire n'est pas reprise comme resultat : on refait
        doc = self.campaign(self.good(), explore=lambda *a, **k: {"status": "ok"},
                            player_exe="p.exe")
        self.assertEqual(doc["stories"][0]["steps"]["run"]["status"], "ok")

    def test_provisional_not_reused_without_run(self):
        def explore(player, story, timeout_s=120, seed=0, max_steps=200):
            raise Killed()
        with self.assertRaises(Killed):
            self.campaign(self.good(), explore=explore, player_exe="p.exe")
        doc = self.campaign(self.good())                       # --no-run
        self.assertEqual(doc["stories"][0]["steps"]["run"]["status"], "skipped")
        self.assertEqual(doc["stories"][0]["status"], "OK")

    def test_transient_failure_is_retried(self):
        calls = []
        real = compat.process_story

        def flaky(entry, cache, **kw):
            calls.append(1)
            if len(calls) == 1:
                raise OSError("disque plein")
            return real(entry, cache, **kw)
        with mock.patch.object(compat, "process_story", flaky):
            self.assertEqual(self.campaign(self.good())["stories"][0]["status"],
                             "ECHEC_CONVERSION")
            self.assertEqual(self.campaign(self.good())["stories"][0]["status"], "OK")
        self.assertEqual(len(calls), 2)

    def test_parallel_run_jobs_exception(self):
        ents = []
        for n in ("a", "b"):
            ents.append(self.entry(n, None, path=self.mkzip(std_files(), n), version="local"))

        def explore(player, story, timeout_s=120, seed=0, max_steps=200):
            if "a-local" in story:
                raise RuntimeError("boum")
            return {"status": "ok"}
        doc = self.campaign(ents, explore=explore, player_exe="p", run_jobs=2)
        st = {s["uuid"]: s["status"] for s in doc["stories"]}
        self.assertEqual(st["a"], "ECHEC_EXECUTION")
        self.assertIn(st["b"], ("OK", "OK_AVERTISSEMENTS"))


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

    def test_two_players_simultaneously(self):
        ents = []
        for n in ("p1", "p2"):
            p = self.mkzip(std_files(), n)
            ents.append(self.entry(n, None, path=p, version="local"))
        doc = self.campaign(ents, explore=_explore.explore, player_exe=PLAYER,
                            run_timeout=90, max_steps=60, run_jobs=2)
        for s in doc["stories"]:
            self.assertIn(s["steps"]["run"]["status"], ("ok", "warn"), s["steps"]["run"])


if __name__ == "__main__":
    unittest.main()
