"""
Tests de l'outillage de parite (parity.py, tests/parity/make_corpus.py), #123.

    python -m unittest discover -s tools/telmi2flam/tests -v

Python contre Python : zero ecart. Les comparaisons sont aussi verifiees sur des
resultats fabriques et sur un faux candidat (script) pour prouver que les ecarts sont
detectes, et que la liste fermee des ecarts admis ne laisse rien passer d'autre.
"""
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
                         ["encrypted_entry", "json_limit"])

    def test_encrypted_entry(self):
        o, c = error_run(6, "internal_error"), error_run(3, "archive_unreadable")
        r = parity.compare_runs(o, c, [parity.TAG_ENCRYPTED])
        self.assertEqual((r["verdict"], r["rule"]), ("allowed", "encrypted_entry"))
        # sans l'etiquette du cas, ou avec un autre couple de codes : ecart
        self.assertEqual(parity.compare_runs(o, c)["verdict"], "diff")
        self.assertEqual(parity.compare_runs(o, c, [parity.TAG_JSON_LIMIT])["verdict"], "diff")
        self.assertEqual(parity.compare_runs(error_run(4, "invalid_content"), c,
                                             [parity.TAG_ENCRYPTED])["verdict"], "diff")

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
        r = parity.compare_runs(run_dict(), error_run(7, "unsupported_format"))
        self.assertEqual(r["verdict"], "delegated")
        report = parity.build_report([dict(r, id="img_x"), dict(parity.compare_runs(
            run_dict(), run_dict()), id="ok")], "corpus")
        self.assertEqual(report["delegations"], ["img_x"])
        self.assertEqual(report["summary"]["delegated"], 1)
        self.assertEqual(parity.exit_code(report), 0)
        # le code 7 sans le bon code d'erreur n'est pas une delegation
        self.assertEqual(parity.compare_runs(run_dict(), error_run(7, "autre"))["verdict"], "diff")


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
        cases = make_corpus.corpus_cases()
        ids = [c.id for c in cases]
        self.assertEqual(len(ids), len(set(ids)))
        known = {make_corpus.TAG_JSON_LIMIT, make_corpus.TAG_ENCRYPTED, make_corpus.TAG_DELEGATED}
        self.assertTrue({t for c in cases for t in c.tags} <= known)
        # les etiquettes lues par parity.py sont celles du generateur
        self.assertEqual(parity.TAG_JSON_LIMIT, make_corpus.TAG_JSON_LIMIT)
        self.assertEqual(parity.TAG_ENCRYPTED, make_corpus.TAG_ENCRYPTED)

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
        self.assertIsNone(make_corpus.matrix_png("RGBA", 2049, 2049, "noise"))


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
    SAMPLE = ("ok_basic", "img_mode_PT", "names_cp437", "mp3_id3_prefix", "missing_audio",
              "zip_crc_corrupt", "json_deep_nesting", "zip_encrypted_entry")
    STUB_SAMPLE = ("ok_basic", "zip_crc_corrupt", "json_deep_nesting", "zip_encrypted_entry")

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
        self.assertEqual(v["json_deep_nesting"]["verdict"], "diff")
        self.assertEqual(v["zip_crc_corrupt"]["verdict"], "diff")

    def test_fake_candidate_delegation(self):
        v = self.verdicts(self.candidate(7, "unsupported_format"))
        self.assertTrue(all(r["verdict"] == "delegated" for r in v.values()))
        report = parity.build_report(list(v.values()), "corpus")
        self.assertEqual(report["summary"]["delegated"], len(self.STUB_SAMPLE))
        self.assertEqual(parity.exit_code(report), 0)

    def test_cli_exit_code_on_deviation(self):
        cmd = "%s %s 6 internal_error" % (sys.executable.replace("\\", "/"),
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
        self.assertIn('python-version: "%s"' % py, yml)
        self.assertIn('pip install "Pillow==%s"' % pillow, yml)
        self.assertNotIn("Pillow>=", yml)


if __name__ == "__main__":
    unittest.main()
