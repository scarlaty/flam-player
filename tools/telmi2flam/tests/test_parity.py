"""
Tests de l'outillage de parite (parity.py, tests/parity/make_corpus.py), #123.

    python -m unittest discover -s tools/telmi2flam/tests -v

Python contre Python : zero ecart. Les comparaisons sont aussi verifiees sur des
resultats fabriques et sur un faux candidat (script) pour prouver que les ecarts sont
detectes, et que la liste fermee des ecarts admis ne laisse rien passer d'autre.
"""
import contextlib
import io
import json
import os
import re
import shutil
import sys
import tempfile
import unittest
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.dirname(HERE)
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(HERE, "parity"))

from test_telmi2flam import std_files  # noqa: E402  (ajoute TOOL au sys.path)
import make_corpus  # noqa: E402
import parity  # noqa: E402


def run_dict(**kw):
    base = {"status": "ok", "exit": 0, "error_code": None, "warnings": [], "info_exit": 0,
            "info": '{"title":"T"}', "entries": [
                {"name": "a", "method": 0, "utf8": False, "size": 1, "sha256": "x"},
                {"name": "b", "method": 0, "utf8": False, "size": 1, "sha256": "y"}]}
    base.update(kw)
    return base


def error_run(exit_code, code):
    return run_dict(status="error", exit=exit_code, error_code=code, entries=None,
                    info_exit=exit_code, info="")


class TestCompareRuns(unittest.TestCase):
    def kinds(self, res):
        return sorted({d["kind"] for d in res["diffs"]})

    def test_identical(self):
        r = parity.compare_runs(run_dict(), run_dict())
        self.assertEqual(r["verdict"], "identical")

    def test_exit_status_and_error_code(self):
        r = parity.compare_runs(error_run(4, "invalid_content"), error_run(5, "missing_asset"))
        self.assertEqual(r["verdict"], "diff")
        self.assertEqual(self.kinds(r), ["error_code", "exit", "info_exit"])
        r = parity.compare_runs(error_run(4, "invalid_content"), run_dict())
        self.assertIn("status", self.kinds(r))

    def test_warnings_are_a_multiset(self):
        # l'ordre ne compte pas (liste triee a la collecte), le nombre d'occurrences si
        a = run_dict(warnings=sorted(["image_missing", "audio_missing", "image_missing"]))
        b = run_dict(warnings=sorted(["audio_missing", "image_missing"]))
        self.assertEqual(self.kinds(parity.compare_runs(a, b)), ["warnings"])
        events = [{"event": "warning", "code": "b"}, {"event": "warning", "code": "a"},
                  {"event": "warning", "code": "b"}, {"event": "done"}]
        self.assertEqual(parity.run_to_dict(0, events, 0, "", [])["warnings"], ["a", "b", "b"])

    def test_info_compared_raw(self):
        r = parity.compare_runs(run_dict(info='{"title":"T"}'), run_dict(info='{"title": "T"}'))
        self.assertEqual(self.kinds(r), ["info"])

    def test_entries_fields_and_order(self):
        base = run_dict()["entries"]
        for field, value in (("name", "z"), ("method", 8), ("utf8", True), ("size", 2),
                             ("sha256", "other")):
            mutated = [dict(base[0], **{field: value}), base[1]]
            r = parity.compare_runs(run_dict(), run_dict(entries=mutated))
            self.assertEqual(r["verdict"], "diff", field)
        r = parity.compare_runs(run_dict(), run_dict(entries=list(reversed(base))))
        self.assertEqual(r["verdict"], "diff")
        r = parity.compare_runs(run_dict(), run_dict(entries=base[:1]))
        self.assertEqual(r["verdict"], "diff")

    def test_validation_failure_is_a_diff(self):
        c = run_dict()
        c["validation"] = 1
        self.assertEqual(self.kinds(parity.compare_runs(run_dict(), c)), ["validate"])
        c["validation"] = 0
        self.assertEqual(parity.compare_runs(run_dict(), c)["verdict"], "identical")


class TestAllowedDeviations(unittest.TestCase):
    def test_list_is_closed(self):
        self.assertEqual(sorted(r.id for r in parity.ALLOWED_DEVIATIONS),
                         ["encrypted_entry", "invalid_utf8_name", "json_limit",
                          "negative_offset"])

    def test_encrypted_entry(self):
        o, c = error_run(6, "internal_error"), error_run(3, "archive_unreadable")
        r = parity.compare_runs(o, c, [parity.TAG_ENCRYPTED])
        self.assertEqual((r["verdict"], r["rule"]), ("allowed", "encrypted_entry"))
        # sans l'etiquette du cas, ou avec un autre couple de codes : ecart
        self.assertEqual(parity.compare_runs(o, c)["verdict"], "diff")
        self.assertEqual(parity.compare_runs(o, c, [parity.TAG_JSON_LIMIT])["verdict"], "diff")
        self.assertEqual(parity.compare_runs(error_run(4, "invalid_content"), c,
                                             [parity.TAG_ENCRYPTED])["verdict"], "diff")

    def test_zip_reader_deviations(self):
        o, c = error_run(6, "internal_error"), error_run(3, "archive_unreadable")
        others = [parity.TAG_ENCRYPTED, parity.TAG_JSON_LIMIT, parity.TAG_BAD_UTF8_NAME,
                  parity.TAG_NEGATIVE_OFFSET, parity.TAG_DELEGATED]
        for tag, rule in ((parity.TAG_BAD_UTF8_NAME, "invalid_utf8_name"),
                          (parity.TAG_NEGATIVE_OFFSET, "negative_offset")):
            r = parity.compare_runs(o, c, [tag])
            self.assertEqual((r["verdict"], r["rule"]), ("allowed", rule))
            # sans l'etiquette du cas, ou avec celle d'une autre regle : ecart
            self.assertEqual(parity.compare_runs(o, c)["verdict"], "diff")
            # l'etiquette d'un autre cas n'active pas cette regle
            for other in others:
                if other != tag:
                    self.assertNotEqual(parity.compare_runs(o, c, [other])["rule"], rule)
            # autre couple de codes : ecart
            for oo, cc in ((error_run(4, "invalid_content"), c),
                           (o, error_run(4, "invalid_content")),
                           (o, error_run(7, "unsupported_format")),
                           (error_run(7, "unsupported_format"), c)):
                self.assertEqual(parity.compare_runs(oo, cc, [tag])["verdict"], "diff")

    def test_json_limit(self):
        c = error_run(4, "invalid_content")
        for o in (run_dict(), error_run(6, "internal_error")):
            r = parity.compare_runs(o, c, [parity.TAG_JSON_LIMIT])
            self.assertEqual((r["verdict"], r["rule"]), ("allowed", "json_limit"))
        self.assertEqual(parity.compare_runs(run_dict(), c)["verdict"], "diff")
        # un autre code d'erreur du candidat n'est pas admis, meme sur un cas etiquete
        self.assertEqual(parity.compare_runs(run_dict(), error_run(3, "archive_unreadable"),
                                             [parity.TAG_JSON_LIMIT])["verdict"], "diff")
        self.assertEqual(parity.compare_runs(error_run(5, "missing_asset"), c,
                                             [parity.TAG_JSON_LIMIT])["verdict"], "diff")

    def test_delegation_counted_apart(self):
        r = parity.compare_runs(run_dict(), error_run(7, "unsupported_format"),
                                [parity.TAG_DELEGATED])
        self.assertEqual(r["verdict"], "delegated")
        # sur un cas non etiquete, deleguer est un ecart (un candidat qui delegue tout)
        self.assertEqual(parity.compare_runs(run_dict(), error_run(7, "unsupported_format"))
                         ["verdict"], "diff")
        report = parity.build_report([dict(r, id="img_x"), dict(parity.compare_runs(
            run_dict(), run_dict()), id="ok")], "corpus")
        self.assertEqual(report["delegations"], ["img_x"])
        self.assertEqual(report["summary"]["delegated"], 1)
        self.assertEqual(parity.exit_code(report), 0)
        # le code 7 sans le bon code d'erreur n'est pas une delegation
        self.assertEqual(parity.compare_runs(run_dict(), error_run(7, "autre"),
                                             [parity.TAG_DELEGATED])["verdict"], "diff")

    def test_error_message_text_is_not_compared(self):
        # §4.1 : le code, pas le texte (--info en erreur : seul le code de sortie compte)
        a = parity.run_to_dict(3, [{"event": "error", "code": "archive_unreadable",
                                    "message": "texte Python"}], 3, None, None)
        b = parity.run_to_dict(3, [{"event": "error", "code": "archive_unreadable",
                                    "message": "texte Rust"}], 3, None, None)
        self.assertEqual(parity.compare_runs(a, b)["verdict"], "identical")

    def test_harness_error_is_a_diff_and_empty_run_fails(self):
        r = parity.run_converter(["commande-inexistante-xyz"], "a.zip", tempfile.mkdtemp())
        self.assertEqual(r[0]["error_code"], "harness_error")
        self.assertEqual(parity.compare_runs(run_dict(), r[0])["verdict"], "diff")
        self.assertEqual(parity.exit_code(parity.build_report([], "corpus")), 1)


class TestLeftovers(unittest.TestCase):
    """--plain et absence de `.part` (#149)."""

    def test_part_file_is_a_deviation_even_on_failure(self):
        ok = run_dict(files=["out.plain.pk"])
        stray = run_dict(files=["out.plain.pk", "out.plain.pk.<pid>.part"])
        r = parity.compare_runs(ok, stray)
        self.assertEqual(r["verdict"], "diff")
        self.assertIn("part", {d["kind"] for d in r["diffs"]})
        # un .part laisse meme sur un cas admis ou delegue : ecart
        r = parity.compare_runs(error_run(6, "internal_error"),
                                dict(error_run(3, "archive_unreadable"),
                                     files=["out.plain.pk.<pid>.part"]),
                                [parity.TAG_ENCRYPTED])
        self.assertEqual(r["verdict"], "diff")
        r = parity.compare_runs(run_dict(), dict(error_run(7, "unsupported_format"),
                                                 files=["out.plain.<pid>.part"]),
                                [parity.TAG_DELEGATED])
        self.assertEqual(r["verdict"], "diff")

    def test_output_left_after_failure(self):
        c = dict(error_run(4, "invalid_content"), files=["out.plain.pk"])
        r = parity.compare_runs(dict(error_run(4, "invalid_content"), files=[]), c)
        self.assertEqual({d["kind"] for d in r["diffs"]}, {"output_on_failure"})
        c = dict(error_run(4, "invalid_content"), files=["out.plain"])
        self.assertEqual(parity.compare_runs(error_run(4, "invalid_content"), c)["verdict"], "diff")
        clean = dict(error_run(4, "invalid_content"), files=[])
        self.assertEqual(parity.compare_runs(clean, clean)["verdict"], "identical")

    def test_plain_dir_is_compared(self):
        def entry(sha):
            return [{"name": "x", "method": None, "utf8": None, "size": 1, "sha256": sha}]
        a, b = run_dict(plain_entries=entry("1")), run_dict(plain_entries=entry("2"))
        r = parity.compare_runs(a, b)
        self.assertEqual(r["verdict"], "diff")
        self.assertEqual({d["kind"] for d in r["diffs"]}, {"plain_entry"})
        self.assertEqual(parity.compare_runs(a, run_dict())["verdict"], "diff")
        self.assertEqual(parity.compare_runs(a, a)["verdict"], "identical")

    def test_part_name_pid_is_neutralised(self):
        d = tempfile.mkdtemp(prefix="t2f_wf_")
        try:
            for n in ("out.plain.pk", "out.plain.pk.4242.part", "out.plain.17.part"):
                open(os.path.join(d, n), "w").close()
            self.assertEqual(parity.work_files(d), ["out.plain.<pid>.part",
                                                    "out.plain.pk", "out.plain.pk.<pid>.part"])
        finally:
            shutil.rmtree(d, ignore_errors=True)


class TestEntries(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="t2f_par_")

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def pk(self, name, entries):
        path = os.path.join(self.tmp, name)
        with open(path, "wb") as f:
            f.write(make_corpus.zip_bytes(entries))
        return path

    def test_name_method_utf8_content_and_duplicates(self):
        data = make_corpus.zip_bytes([("a.txt", "x"), ("é.txt", "y")])
        path = os.path.join(self.tmp, "x.pk")
        with open(path, "wb") as f:
            f.write(data)
        e = parity.entries_from_pk(path)
        self.assertEqual([x["name"] for x in e], ["a.txt", "é.txt"])
        self.assertEqual([x["utf8"] for x in e], [False, True])
        self.assertEqual({x["method"] for x in e}, {zipfile.ZIP_STORED})
        dup = make_corpus.zip_bytes(std_files())
        self.assertEqual(len(parity.entries_from_pk(self._write("d.pk", dup))), len(std_files()))

    def _write(self, name, data):
        p = os.path.join(self.tmp, name)
        with open(p, "wb") as f:
            f.write(data)
        return p

    def test_compare_two_outputs_cli(self):
        a = self.pk("a.pk", [("n", "1"), ("m", "2")])
        b = self.pk("b.pk", [("n", "1"), ("m", "2")])
        c = self.pk("c.pk", [("n", "1"), ("m", "3")])
        report = os.path.join(self.tmp, "r.json")
        self.assertEqual(parity.main(["compare", a, b, "--report", report]), 0)
        self.assertEqual(parity.main(["compare", a, c, "--report", report]), 1)
        with open(report, encoding="utf-8") as f:
            rep = json.load(f)
        self.assertEqual(rep["summary"]["diff"], 1)
        self.assertEqual(rep["cases"][0]["diffs"][0]["field"], "sha256")

    def test_compare_two_plain_dirs(self):
        for d, content in (("p1", "A"), ("p2", "A"), ("p3", "B")):
            os.makedirs(os.path.join(self.tmp, d, "sub"))
            with open(os.path.join(self.tmp, d, "sub", "f.txt"), "w") as f:
                f.write(content)
        j = lambda d: os.path.join(self.tmp, d)  # noqa: E731
        self.assertEqual(parity.main(["compare", j("p1"), j("p2")]), 0)
        self.assertEqual(parity.main(["compare", j("p1"), j("p3")]), 1)
        # un dossier vide en plus est un ecart (entree `nom/`)
        os.makedirs(os.path.join(self.tmp, "p2", "vide", "aussi"))
        self.assertEqual([e["name"] for e in parity.entries_from_dir(j("p2"))],
                         ["sub/f.txt", "vide/aussi/"])
        self.assertEqual(parity.main(["compare", j("p1"), j("p2")]), 1)


class TestLifDirs(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="t2f_lif_")
        self.a = os.path.join(self.tmp, "a")
        self.b = os.path.join(self.tmp, "b")
        os.makedirs(self.a)
        os.makedirs(self.b)

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def put(self, d, name, data):
        with open(os.path.join(d, name), "wb") as f:
            f.write(data)

    def test_byte_exact(self):
        self.put(self.a, "x.lif", b"abcdef")
        self.put(self.b, "x.lif", b"abcdef")
        self.put(self.a, "y.lif", b"abcdef")
        self.put(self.b, "y.lif", b"abcdeg")
        self.put(self.a, "only_a.lif", b"1")
        self.put(self.b, "only_b.lif", b"1")
        res = {r["id"]: r for r in parity.compare_lif_dirs(self.a, self.b)}
        self.assertEqual(res["x"]["verdict"], "identical")
        self.assertEqual(res["y"]["verdict"], "diff")
        self.assertEqual(res["y"]["diffs"][0]["first_difference"], 5)
        self.assertEqual(res["only_a"]["verdict"], "diff")
        self.assertEqual(res["only_b"]["verdict"], "diff")


class TestCorpusGenerator(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="t2f_corp_")

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def test_ids_unique_and_tags_known(self):
        cases = make_corpus.corpus_cases() + make_corpus.output_cases()
        ids = [c.id for c in cases]
        self.assertEqual(len(ids), len(set(ids)))
        # cas --plain (nominal et en echec) presents
        by_id = {c.id: c for c in cases}
        for cid in ("ok_plain", "missing_audio_plain", "zip_crc_corrupt_plain"):
            self.assertIn("--plain", by_id[cid].args, cid)
        known = {make_corpus.TAG_JSON_LIMIT, make_corpus.TAG_ENCRYPTED, make_corpus.TAG_DELEGATED,
                 make_corpus.TAG_BAD_UTF8_NAME, make_corpus.TAG_NEGATIVE_OFFSET}
        self.assertTrue({t for c in cases for t in c.tags} <= known)
        # les etiquettes lues par parity.py sont celles du generateur
        self.assertEqual(parity.TAG_JSON_LIMIT, make_corpus.TAG_JSON_LIMIT)
        self.assertEqual(parity.TAG_ENCRYPTED, make_corpus.TAG_ENCRYPTED)
        self.assertEqual(parity.TAG_BAD_UTF8_NAME, make_corpus.TAG_BAD_UTF8_NAME)
        self.assertEqual(parity.TAG_NEGATIVE_OFFSET, make_corpus.TAG_NEGATIVE_OFFSET)

    def test_corpus_is_deterministic(self):
        d1, d2 = os.path.join(self.tmp, "1"), os.path.join(self.tmp, "2")
        make_corpus.write_corpus(d1)
        make_corpus.write_corpus(d2)
        for name in sorted(os.listdir(os.path.join(d1, "corpus"))):
            with open(os.path.join(d1, "corpus", name), "rb") as f1, \
                    open(os.path.join(d2, "corpus", name), "rb") as f2:
                self.assertEqual(f1.read(), f2.read(), name)

    def test_matrix_covers_real_targets_and_sizes(self):
        entries = make_corpus.write_matrix(self.tmp)
        ops = {e["op"] for e in entries}
        self.assertEqual(ops, set(make_corpus.OPS))
        ids = {e["png"] for e in entries}
        for size in (1, 3, 97, 641, 2049):
            self.assertTrue(any("_%dx" % size in i for i in ids), size)
        for mode in make_corpus.MODES:
            self.assertTrue(any(os.path.basename(i).startswith(mode + "_") for i in ids), mode)
        # la borne 2047 n'est exercee que par les sources plus grandes
        for e in entries:
            if e["op"] == "bound":
                self.assertIn("2049", e["png"])
        # bruit a graine fixe : memes octets a chaque generation
        a = make_corpus.matrix_png("RGBA", 97, 97, "noise")
        self.assertEqual(a, make_corpus.matrix_png("RGBA", 97, 97, "noise"))
        self.assertNotEqual(a, make_corpus.matrix_png("RGBA", 97, 97, "gradient"))
        self.assertIsNone(make_corpus.matrix_png("RGB", 97, 97, "alpha"))
        # bruit a 2049 : carre et seulement RGBA / L (sinon les PNG seraient enormes)
        self.assertIsNone(make_corpus.matrix_png("RGB", 2049, 2049, "noise"))
        self.assertIsNone(make_corpus.matrix_png("RGBA", 2049, 1537, "noise"))
        self.assertIsNone(make_corpus.matrix_png("RGBA", 1025, 1025, "noise"))
        names = {os.path.basename(e) for e in ids}
        self.assertIn("RGBA_2049x2049_noise.png", names)
        self.assertIn("L_2049x2049_noise.png", names)

    def test_matrix_new_png_kinds(self):
        """RGB16 / RGBA16 / tRNS de couleur cle : le PNG a bien la structure annoncee."""
        import io
        import struct
        from PIL import Image
        want = {"RGB16": (16, 2, False), "RGBA16": (16, 6, False), "LT": (8, 0, True),
                "RGBT": (8, 2, True), "RGBT16": (16, 2, True)}
        def chunks(data):
            """Chunks du PNG (type -> donnees), parcourus en suivant les longueurs."""
            out, pos = {}, 8
            while pos + 8 <= len(data):
                n, kind = struct.unpack(">I4s", data[pos:pos + 8])
                out[kind] = data[pos + 8:pos + 8 + n]
                pos += 12 + n
            return out

        for mode, (depth, ctype, trns) in want.items():
            data = make_corpus.matrix_png(mode, 7, 5, "gradient")
            ihdr = struct.unpack(">IIBBBBB", data[16:29])
            self.assertEqual((ihdr[2], ihdr[3]), (depth, ctype), mode)
            found = chunks(data)
            self.assertEqual(b"tRNS" in found, trns, mode)
            if ctype == 2 and trns:
                # cle de couleur franchement non grise, canaux distincts (permutation visible)
                key = struct.unpack(">HHH", found[b"tRNS"])
                self.assertEqual(len(set(key)), 3, (mode, key))
                self.assertGreaterEqual(max(key) - min(key), 100 << (depth - 8), (mode, key))
            Image.open(io.BytesIO(data)).load()
        self.assertIsNone(make_corpus.matrix_png("RGBT", 7, 5, "alpha"))
        self.assertIsNotNone(make_corpus.matrix_png("RGBA16", 7, 5, "alpha"))


# Faux candidat : repond comme un convertisseur qui echoue toujours avec le code donne.
STUB = """
import json, sys
exit_code, code = int(sys.argv[1]), sys.argv[2]
args = sys.argv[3:]
print(json.dumps({"event": "error", "code": code, "message": "stub", "exit": exit_code}))
sys.exit(exit_code)
"""


class TestPythonAgainstPython(unittest.TestCase):
    """Le critere de l'issue : Python contre Python, zero ecart."""

    # un cas par famille (nominal, image, noms, MP3, erreurs, delegation) ; le corpus
    # complet est compare par la CI de parite (#134), trop long pour ce test unitaire
    SAMPLE = ("ok_basic", "ok_plain", "missing_audio_plain", "ok_plain_dir_only",
              "zip_crc_corrupt_plain_dir_only", "img_mode_PT", "names_cp437",
              "mp3_id3_prefix", "missing_audio", "zip_crc_corrupt", "json_deep_nesting",
              "zip_encrypted_entry", "img_jpeg_delegated", "zip_invalid_utf8_name", "zip_negative_header_offset")
    STUB_SAMPLE = ("ok_basic", "zip_crc_corrupt", "json_deep_nesting", "zip_encrypted_entry",
                   "img_jpeg_delegated", "zip_invalid_utf8_name", "zip_negative_header_offset")

    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.mkdtemp(prefix="t2f_pp_")
        entries = make_corpus.write_corpus(cls.tmp)
        cls.entries = [e for e in entries if e["id"] in cls.SAMPLE]
        with open(os.path.join(cls.tmp, "corpus.json"), "w", encoding="utf-8") as f:
            json.dump(cls.entries, f)
        cls.small = os.path.join(cls.tmp, "small")
        os.makedirs(cls.small)
        with open(os.path.join(cls.small, "corpus.json"), "w", encoding="utf-8") as f:
            # chemins d'archive absolus : os.path.join(dossier, chemin absolu) = chemin absolu
            json.dump([dict(e, archive=os.path.join(cls.tmp, e["archive"]))
                       for e in cls.entries if e["id"] in cls.STUB_SAMPLE], f)
        cls.stub = os.path.join(cls.tmp, "stub.py")
        with open(cls.stub, "w", encoding="utf-8") as f:
            f.write(STUB)

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.tmp, ignore_errors=True)

    def test_plain_run_has_directory_and_failure_leaves_nothing(self):
        by_id = {e["id"]: e for e in self.entries}
        for cid, ok in (("ok_plain", True), ("missing_audio_plain", False)):
            work = os.path.join(self.tmp, "w_" + cid)
            run, produced = parity.run_converter(
                parity.default_oracle(), os.path.join(self.tmp, by_id[cid]["archive"]), work,
                by_id[cid]["args"])
            if ok:
                self.assertIn("info.plain", [e["name"] for e in run["plain_entries"]])
                self.assertEqual(run["files"], ["out.plain", "out.plain.pk"])
            else:
                self.assertEqual(run["status"], "error")
                self.assertIsNone(produced)
                self.assertEqual(run["files"], [])

    def test_plain_dir_only_output(self):
        """-o x.plain : dossier seul, compare par plain_entries, rien laisse en echec."""
        by_id = {e["id"]: e for e in self.entries}
        for cid, ok in (("ok_plain_dir_only", True), ("zip_crc_corrupt_plain_dir_only", False)):
            work = os.path.join(self.tmp, "w_" + cid)
            run, produced = parity.run_converter(
                parity.default_oracle(), os.path.join(self.tmp, by_id[cid]["archive"]), work,
                by_id[cid]["args"])
            self.assertIsNone(produced, cid)
            self.assertIsNone(run["entries"], cid)
            if ok:
                self.assertEqual(run["status"], "ok")
                self.assertIn("info.plain", [e["name"] for e in run["plain_entries"]])
                self.assertEqual(run["files"], ["out.plain"])
            else:
                self.assertEqual(run["status"], "error")
                self.assertIsNone(run["plain_entries"])
                self.assertEqual(run["files"], [])

    def test_zero_deviation(self):
        self.assertEqual(len(self.entries), len(self.SAMPLE))
        report = os.path.join(self.tmp, "report.json")
        self.assertEqual(parity.main(["corpus", self.tmp, "--report", report]), 0)
        with open(report, encoding="utf-8") as f:
            rep = json.load(f)
        self.assertEqual(rep["schema"], parity.REPORT_SCHEMA)
        self.assertEqual(rep["summary"], {"total": len(self.SAMPLE), "identical": len(self.SAMPLE),
                                          "allowed": 0, "delegated": 0, "diff": 0})
        self.assertEqual([c["id"] for c in rep["cases"]], [e["id"] for e in self.entries])

    def candidate(self, exit_code, code):
        return [sys.executable, self.stub, str(exit_code), code]

    def verdicts(self, candidate):
        res = parity.run_corpus(self.small, parity.default_oracle(), candidate, validate=False)
        return {r["id"]: r for r in res}

    def test_fake_candidate_deviations_follow_the_closed_list(self):
        v = self.verdicts(self.candidate(4, "invalid_content"))
        # etiquete json_limit : admis ; tout le reste echoue par ecart
        self.assertEqual(v["json_deep_nesting"]["verdict"], "allowed")
        self.assertEqual(v["json_deep_nesting"]["rule"], "json_limit")
        self.assertEqual(v["ok_basic"]["verdict"], "diff")
        self.assertEqual(v["zip_encrypted_entry"]["verdict"], "diff")
        v = self.verdicts(self.candidate(3, "archive_unreadable"))
        self.assertEqual(v["zip_encrypted_entry"]["verdict"], "allowed")
        self.assertEqual(v["zip_encrypted_entry"]["rule"], "encrypted_entry")
        # chaque regle ne couvre que son cas etiquete : les deux nouveaux cas, meme couple de
        # codes, sont admis par leur propre regle et par aucune autre
        self.assertEqual(v["zip_invalid_utf8_name"]["rule"], "invalid_utf8_name")
        self.assertEqual(v["zip_negative_header_offset"]["rule"], "negative_offset")
        self.assertEqual(v["json_deep_nesting"]["verdict"], "diff")
        self.assertEqual(v["zip_crc_corrupt"]["verdict"], "diff")

    def test_fake_candidate_delegation(self):
        v = self.verdicts(self.candidate(7, "unsupported_format"))
        self.assertEqual(v["img_jpeg_delegated"]["verdict"], "delegated")
        # un candidat qui delegue tout : seul le cas etiquete est admis
        self.assertEqual({k for k, r in v.items() if r["verdict"] == "diff"},
                         set(self.STUB_SAMPLE) - {"img_jpeg_delegated"})
        report = parity.build_report(list(v.values()), "corpus")
        self.assertEqual(report["delegations"], ["img_jpeg_delegated"])
        self.assertEqual(parity.exit_code(report), 1)

    def test_cli_exit_code_on_deviation(self):
        cmd = '"%s" "%s" 6 internal_error' % (sys.executable.replace("\\", "/"),
                                              self.stub.replace("\\", "/"))
        self.assertEqual(parity.main(["corpus", self.small, "--candidate", cmd]), 1)


class TestMatrixPythonAgainstPython(unittest.TestCase):
    def test_small_matrix_zero_deviation_and_detects_change(self):
        tmp = tempfile.mkdtemp(prefix="t2f_mx_")
        try:
            entries = make_corpus.write_matrix(tmp, sizes=(1, 3))
            self.assertGreater(len(entries), 100)
            res = parity.run_matrix(tmp)
            self.assertEqual({r["verdict"] for r in res}, {"identical"})
            self.assertEqual(len(res), len(entries))
            # un .lif candidat modifie d'un octet est detecte
            cand = os.path.join(tmp, "cand")
            make_corpus.render_matrix_python(tmp, cand)
            victim = os.path.join(cand, entries[0]["id"] + ".lif")
            with open(victim, "rb") as f:
                data = bytearray(f.read())
            data[-1] ^= 1
            with open(victim, "wb") as f:
                f.write(data)
            res = parity.run_matrix(tmp, cand)
            self.assertEqual([r["id"] for r in res if r["verdict"] == "diff"], [entries[0]["id"]])
        finally:
            shutil.rmtree(tmp, ignore_errors=True)


DELEGATING_STUB = """
import json, sys
print(json.dumps({"event": "error", "code": "unsupported_format", "exit": 7,
                  "message": "images/x.jpg : format non pris en charge : pas un PNG"}))
sys.exit(7)
"""


class TestCampaign(unittest.TestCase):
    """Campagne sur archives reelles (#135) : reprise, delegations comptees, seuil."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="t2f_camp_")
        self.archives = os.path.join(self.tmp, "archives")
        os.makedirs(self.archives)
        jpeg = b"\xff\xd8\xff\xe0" + b"\0" * 32
        for name, files, compress in (
                ("a.zip", std_files(), zipfile.ZIP_STORED),
                ("b.zip", std_files() + [("images/x.jpg", jpeg)],
                 {"audios/s0.mp3": zipfile.ZIP_BZIP2})):
            with open(os.path.join(self.archives, name), "wb") as f:
                f.write(make_corpus.zip_bytes(files, compress))
        self.stub = os.path.join(self.tmp, "deleg.py")
        with open(self.stub, "w", encoding="utf-8") as f:
            f.write(DELEGATING_STUB)
        self.out = os.path.join(self.tmp, "out")

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def test_helpers(self):
        self.assertEqual(parity.image_format(b"\x89PNG\r\n\x1a\nxxxx"), "png")
        self.assertEqual(parity.image_format(b"\xff\xd8\xff\xdb"), "jpeg")
        self.assertEqual(parity.image_format(b"RIFF\0\0\0\0WEBPVP8 "), "webp")
        self.assertIsNone(parity.image_format(b"ID3\x04"))
        self.assertEqual(parity.delegation_cause(
            "a.zip : \"x\" : compression 12 (bzip2/LZMA/Zstandard) déléguée à Python"), "zip:bzip2")
        self.assertEqual(parity.delegation_cause("i.png : format non pris en charge : APNG"),
                         "image:APNG")
        self.assertEqual(parity.delegation_cause(
            "metadata.json title : liste ou objet non pris en charge"), "metadata")
        self.assertEqual(parity.delegation_cause(None), "autre")
        inv = parity.archive_inventory(os.path.join(self.archives, "b.zip"))
        self.assertEqual(inv["images"], {"jpeg": 1, "png": 2})
        self.assertEqual(inv["methods"], {"bzip2": 1, "stored": 5})
        bad = os.path.join(self.tmp, "bad.zip")
        with open(bad, "wb") as f:
            f.write(b"pas une archive")
        self.assertEqual(parity.archive_inventory(bad)["error"], "BadZipFile")

    def test_image_format_signatures_are_narrow(self):
        mp4 = b"\0\0\0\x20ftypisom\0\0\2\0"
        m4a = b"\0\0\0\x20ftypM4A \0\0\0\0"
        self.assertIsNone(parity.image_format(mp4, "audios/s0.m4a"))
        self.assertIsNone(parity.image_format(m4a, "images/x.avif"))
        for brand in (b"avif", b"avis", b"heic", b"heix", b"mif1"):
            self.assertEqual(parity.image_format(b"\0\0\0\x20ftyp" + brand + b"\0\0"),
                             "avif_heif")
        # BM et <?xml : seulement avec une extension d'image
        self.assertIsNone(parity.image_format(b"BMxxxxxxxxxxxxxx", "audios/s0.mp3"))
        self.assertIsNone(parity.image_format(b"BMxxxxxxxxxxxxxx"))
        self.assertEqual(parity.image_format(b"BMxxxxxxxxxxxxxx", "images/x.BMP"), "bmp")
        self.assertIsNone(parity.image_format(b"<?xml version=1.0", "metadata.xml"))
        self.assertEqual(parity.image_format(b"<?xml version=1.0", "images/x.svg"), "svg")
        self.assertEqual(parity.image_format(b"<svg xmlns=...>"), "svg")
        # inventaire : une entree MP4 ou XML non image n'est comptee nulle part
        path = os.path.join(self.tmp, "n.zip")
        with open(path, "wb") as f:
            f.write(make_corpus.zip_bytes([("audios/a.m4a", mp4), ("x/b.xml", b"<?xml v"),
                                           ("audios/c.mp3", b"BMxxxxxxxx")], zipfile.ZIP_STORED))
        self.assertEqual(parity.archive_inventory(path)["images"], {})

    def test_delegation_cause_is_anchored_with_closed_reasons(self):
        # un nom d'asset ne fausse pas la cause
        self.assertEqual(parity.delegation_cause(
            "compression 3 (x).png : format non pris en charge : APNG"), "image:APNG")
        self.assertEqual(parity.delegation_cause(
            "format non pris en charge : APNG.png : compression 93 (bzip2/LZMA/Zstandard) "
            "déléguée à Python"), "zip:zstd")
        self.assertEqual(parity.delegation_cause("compression 12 sans le reste"), "autre")
        # nom piege cote ZIP : la derniere occurrence (le vrai message) fait foi
        self.assertEqual(parity.delegation_cause(
            "a.zip : \"a : compression 3 (bzip2/LZMA/Zstandard) déléguée à Python\" : "
            "compression 12 (bzip2/LZMA/Zstandard) déléguée à Python"), "zip:bzip2")
        self.assertEqual(parity.delegation_cause(
            "a\nb.png : format non pris en charge : APNG"), "image:APNG")
        # raison hors liste fermee, ou message sans ancre : autre
        self.assertEqual(parity.delegation_cause(
            "i.png : format non pris en charge : raison inventee"), "autre")
        self.assertEqual(parity.delegation_cause(
            "format non pris en charge : APNG"), "autre")
        for why in parity.IMAGE_REASONS:
            self.assertEqual(parity.delegation_cause("i.png : format non pris en charge : " + why),
                             "image:" + why)

    def test_delegation_warning_counts_only(self):
        self.assertIsNone(parity.delegation_warning({}))
        self.assertIsNone(parity.delegation_warning({"zip:bzip2": 3, "image:APNG": 1}))
        w = parity.delegation_warning({"autre": 2, "metadata": 1, "zip:zstd": 4})
        self.assertIn("2 `autre`", w)
        self.assertIn("1 `metadata`", w)
        self.assertNotIn("zstd", w)
        self.assertIn("1 `metadata`", parity.delegation_warning({"metadata": 1}))

    def test_limit_marks_report_partial(self):
        oracle = parity.default_oracle()
        doc = parity.run_campaign(self.archives, oracle, oracle, self.out, validate=False,
                                  limit=1)
        self.assertTrue(doc["partial"])
        self.assertEqual(doc["summary"]["total"], 1)
        with open(os.path.join(self.out, "report.json"), encoding="utf-8") as f:
            self.assertTrue(json.load(f)["partial"])
        doc = parity.run_campaign(self.archives, oracle, oracle, self.out, validate=False)
        self.assertFalse(doc["partial"])
        self.assertEqual(doc["summary"]["total"], 2)

    def test_unexpected_exception_in_one_is_a_harness_failure(self):
        oracle = parity.default_oracle()
        real = parity._write_json_atomic

        def failing(path, doc):
            if os.path.basename(os.path.dirname(path)) == "cases":
                raise RuntimeError("secret-path-marker")
            return real(path, doc)
        parity._write_json_atomic = failing
        stderr = io.StringIO()
        try:
            with contextlib.redirect_stderr(stderr):
                doc = parity.run_campaign(self.archives, oracle, oracle, self.out,
                                          validate=False)
        finally:
            parity._write_json_atomic = real
        self.assertEqual(doc["summary"]["diff"], 2)
        self.assertFalse(doc["threshold_met"])
        self.assertEqual({d["kind"] for c in doc["cases"] for d in c["diffs"]}, {"harness"})
        self.assertEqual(doc["cases"][0]["diffs"][0]["candidate"], "RuntimeError")
        self.assertEqual(stderr.getvalue(), "")
        self.assertNotIn("secret-path-marker", json.dumps(doc))

    def test_python_against_python_then_resume(self):
        oracle = parity.default_oracle()
        doc = parity.run_campaign(self.archives, oracle, oracle, self.out, jobs=2,
                                  validate=False)
        self.assertEqual(doc["summary"]["identical"], 2)
        self.assertTrue(doc["threshold_met"])
        self.assertEqual(doc["pending"], 0)
        self.assertEqual(doc["corpus_archives"]["images"], {"jpeg": 1, "png": 2})
        self.assertEqual(doc["corpus_entries"]["zip_methods"], {"bzip2": 1, "stored": 10})
        # dossiers de travail supprimes quand il n'y a pas d'ecart
        self.assertEqual(os.listdir(os.path.join(self.out, "work")), [])
        # reprise : un etat obtenu avec les memes moteurs est repris tel quel
        state = os.path.join(self.out, "cases", "a.json")
        with open(state, encoding="utf-8") as f:
            res = json.load(f)
        res["marker"] = True
        with open(state, "w", encoding="utf-8") as f:
            json.dump(res, f)
        doc = parity.run_campaign(self.archives, oracle, oracle, self.out, validate=False)
        self.assertTrue(next(c for c in doc["cases"] if c["id"] == "a")["marker"])
        # autre candidat : tout est refait, les delegations sont comptees a part
        cand = [sys.executable, self.stub]
        doc = parity.run_campaign(self.archives, oracle, cand, self.out, validate=False)
        self.assertNotIn("marker", doc["cases"][0])
        self.assertEqual(doc["summary"]["delegated"], 2)
        self.assertEqual(doc["delegations_by_cause"], {"image:pas un PNG": 2})
        self.assertFalse(doc["partial"])
        self.assertEqual(doc["delegated_archives"]["images"], {"jpeg": 1, "png": 2})
        self.assertEqual(doc["delegated_archives"]["zip_methods"], {"bzip2": 1, "stored": 2})
        self.assertTrue(doc["threshold_met"])

    def test_resume_requires_validation_and_redoes_broken_state(self):
        oracle = parity.default_oracle()
        parity.run_campaign(self.archives, oracle, oracle, self.out, validate=False)
        state = os.path.join(self.out, "cases", "a.json")
        with open(state, encoding="utf-8") as f:
            res = json.load(f)
        res["marker"] = True
        with open(state, "w", encoding="utf-8") as f:
            json.dump(res, f)
        with open(os.path.join(self.out, "cases", "b.json"), "w", encoding="utf-8") as f:
            f.write('{"verdict": "ident')                    # etat tronque
        # une campagne qui valide ne reprend pas un etat obtenu sans validate.py
        doc = parity.run_campaign(self.archives, oracle, oracle, self.out, validate=True)
        self.assertEqual(doc["summary"]["identical"], 2)
        self.assertNotIn("marker", doc["cases"][0])
        self.assertEqual(doc["cases"][0]["candidate"]["validation"], 0)
        # l'inverse est admis : un etat valide sert a une campagne sans validation
        with open(state, encoding="utf-8") as f:
            res = json.load(f)
        res["marker"] = True
        with open(state, "w", encoding="utf-8") as f:
            json.dump(res, f)
        doc = parity.run_campaign(self.archives, oracle, oracle, self.out, validate=False)
        self.assertTrue(doc["cases"][0]["marker"])

    def test_fingerprint_covers_path_lookup_and_sibling_modules(self):
        script_dir = os.path.join(self.tmp, "eng")
        os.makedirs(script_dir)
        main_py = os.path.join(script_dir, "main.py")
        helper = os.path.join(script_dir, "helper.py")
        for p in (main_py, helper):
            with open(p, "w", encoding="utf-8") as f:
                f.write("x = 1\n")
        exe = os.path.basename(sys.executable)
        old_path = os.environ.get("PATH", "")
        os.environ["PATH"] = os.path.dirname(sys.executable) + os.pathsep + old_path
        try:
            a = parity.command_fingerprint([exe, main_py])
            self.assertEqual(a, parity.command_fingerprint([exe, main_py]))
            with open(helper, "w", encoding="utf-8") as f:
                f.write("x = 2\n")                      # module voisin modifie
            self.assertNotEqual(a, parity.command_fingerprint([exe, main_py]))
        finally:
            os.environ["PATH"] = old_path
        with self.assertRaises(ValueError):
            parity.command_fingerprint(["commande-inexistante-xyz"])

    def test_deviation_keeps_work_and_fails_threshold(self):
        oracle = parity.default_oracle()
        stub = os.path.join(self.tmp, "stub.py")
        with open(stub, "w", encoding="utf-8") as f:
            f.write(STUB)
        cmd = '"%s" "%s" 6 internal_error' % (sys.executable.replace("\\", "/"),
                                              stub.replace("\\", "/"))
        rc = parity.main(["campaign", self.archives, "--candidate", cmd, "--out", self.out,
                          "--limit", "1", "--no-validate"])
        self.assertEqual(rc, 1)
        with open(os.path.join(self.out, "report.json"), encoding="utf-8") as f:
            doc = json.load(f)
        self.assertFalse(doc["threshold_met"])
        self.assertTrue(doc["partial"])
        self.assertEqual(doc["summary"]["diff"], 1)
        self.assertIn("exit", doc["diff_kinds"])
        self.assertEqual(doc["cases"][0]["candidate"]["error_code"], "internal_error")
        self.assertEqual(os.listdir(os.path.join(self.out, "work")), ["a"])


class TestPinnedOracle(unittest.TestCase):
    """Pillow et Python de la CI = ceux de l'installateur (PORT-RUST.md §4.3 point 4)."""

    ROOT = os.path.dirname(os.path.dirname(TOOL))

    def read(self, *parts):
        with open(os.path.join(self.ROOT, *parts), encoding="utf-8") as f:
            return f.read()

    def test_ci_pins_match_build_converter(self):
        ps1 = self.read("apps", "flam-launcher", "scripts", "build-converter.ps1")
        yml = self.read(".github", "workflows", "telmi2flam.yml")
        py = re.search(r"\$PythonVersion = '([^']+)'", ps1).group(1)
        pillow = re.search(r"\$PillowVersion = '([^']+)'", ps1).group(1)
        # versions lues dans le yml (un assertIn sur le texte brut se contenterait d'un
        # commentaire ou d'une autre ligne)
        ci_py = re.findall(r'^\s*python-version:\s*"([^"]+)"\s*$', yml, re.M)
        self.assertEqual(ci_py, [py])
        installs = re.findall(r"^\s*run:\s*pip install (.+)$", yml, re.M)
        self.assertEqual(len(installs), 1)
        pins = dict(re.findall(r'"?([A-Za-z]+)==([^"\s]+)"?', installs[0]))
        self.assertEqual(pins.get("Pillow"), pillow)
        # lupa : version exacte (le Lua de l'oracle ne doit pas deriver sans qu'on le voie)
        self.assertRegex(pins.get("lupa", ""), r"^\d+(\.\d+)+$")
        self.assertEqual(sorted(pins), ["Pillow", "lupa"])
        self.assertNotRegex(installs[0], r"[<>~!]=?")


if __name__ == "__main__":
    unittest.main()
