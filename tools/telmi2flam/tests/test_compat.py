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


if __name__ == "__main__":
    unittest.main()
