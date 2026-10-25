"""
Tests de l'outillage de parite (parity.py, references figees, tests/parity/make_matrix.py),
#123, #140.

    python -m unittest discover -s tools/telmi2flam/tests -v

Le binaire Rust contre les references figees de l'ancien oracle Python : zero ecart hors de
la liste fermee. Les comparaisons sont aussi verifiees sur des resultats fabriques et sur un
faux candidat (script) pour prouver que les ecarts sont detectes, et que la liste fermee des
ecarts admis ne laisse rien passer d'autre. Exige le binaire telmi2flam (converter_bin.py).
"""
import contextlib
import hashlib
import io
import json
import os
import re
import shutil
import struct
import sys
import tempfile
import unittest
import zipfile
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.dirname(HERE)
ROOT = os.path.dirname(os.path.dirname(TOOL))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(HERE, "parity"))

from campaign_fixtures import base_nodes, std_files, zip_bytes  # noqa: E402  (ajoute TOOL au sys.path)
import converter_bin  # noqa: E402
import make_matrix  # noqa: E402
import parity  # noqa: E402


def binary():
    """Binaire telmi2flam ; echec clair s'il est absent (pas de saut silencieux, #465)."""
    try:
        return converter_bin.find_converter()
    except converter_bin.ConverterNotFound as e:
        raise RuntimeError("test_parity.py exige le binaire telmi2flam : %s" % e)


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
                         ["encrypted_entry", "invalid_utf8_name", "jpeg_bounded", "json_limit",
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

    def test_jpeg_rule_is_not_a_status_rule(self):
        # la regle D7 n'admet aucun ecart de statut, meme sur un cas etiquete
        o, c = error_run(6, "internal_error"), error_run(3, "archive_unreadable")
        self.assertEqual(parity.compare_runs(o, c, [parity.TAG_JPEG], ["a"])["verdict"], "diff")
        self.assertEqual(parity.compare_runs(run_dict(), run_dict(warnings=["image_unreadable"]),
                                             [parity.TAG_JPEG], ["a"])["verdict"], "diff")

    def test_harness_error_is_a_diff_and_empty_run_fails(self):
        r = parity.run_converter(["commande-inexistante-xyz"], "a.zip", tempfile.mkdtemp())
        self.assertEqual(r[0]["error_code"], "harness_error")
        self.assertEqual(parity.compare_runs(run_dict(), r[0])["verdict"], "diff")
        self.assertEqual(parity.exit_code(parity.build_report([], "corpus")), 1)


def lif_of(pixels, w=2, h=1):
    """.lif de pixels RGBA donnes sur la grille RGB565 (r, b multiples de 8, g de 4) : un
    opcode 0xFF (couleur et alpha explicites) par pixel."""
    body = b""
    for r, g, b, a in pixels:
        r5, g6, b5 = r // 8, g // 4, b // 8
        body += bytes((0xFF, (b5 & 0x1F) | ((g6 & 7) << 5), ((g6 >> 3) & 7) | (r5 << 3), a))
    return (b"liff" + w.to_bytes(4, "big") + h.to_bytes(4, "big") + b"\xa2" + body
            + b"\x00" * 7 + b"\x01")


def jpeg_runs(a, b, name="img/s0.lif"):
    """Deux resultats identiques sauf le contenu de l'entree `name` (.lif a puis b)."""
    def run(data):
        return run_dict(entries=[
            {"name": "info.json", "method": 0, "utf8": False, "size": 1, "sha256": "x"},
            {"name": name, "method": 0, "utf8": False, "size": len(data),
             "sha256": parity._sha(data), "data": data}])
    return run(a), run(b)


class TestJpegBounded(unittest.TestCase):
    """D7 (#459) : ecart de +-4 avant quantification, sur les seules entrees issues d'un JPEG."""

    BASE = [(80, 80, 80, 255), (200, 40, 16, 255)]

    def verdict(self, other, tags=(parity.TAG_JPEG,), names=("img/s0.lif",), **kw):
        o, c = jpeg_runs(lif_of(self.BASE, **kw), lif_of(other, **kw))
        r = parity.compare_runs(o, c, list(tags), list(names))
        return r["verdict"], r["rule"]

    def test_one_quantization_step_is_admitted(self):
        # un pas de 8 (R, B) ou de 4 (G) : ecart source minimal de 1
        for other in ([(88, 80, 80, 255), self.BASE[1]], [(80, 84, 72, 255), self.BASE[1]],
                      [(72, 76, 88, 255), (192, 44, 24, 255)]):
            self.assertEqual(self.verdict(other), ("allowed", "jpeg_bounded"), other)
        self.assertEqual(self.verdict(self.BASE), ("identical", None))

    def test_deviation_of_five_is_a_diff(self):
        # deux pas de G (8) : ecart source minimal 8 - 3 = 5 > 4 ; deux pas de R : 9
        for other in ([(80, 88, 80, 255), self.BASE[1]], [(96, 80, 80, 255), self.BASE[1]],
                      [self.BASE[0], (200, 40, 16, 250)]):
            self.assertEqual(self.verdict(other), ("diff", None), other)
        # alpha exact (JPEG opaque) : le moindre ecart est refuse
        self.assertEqual(self.verdict([(80, 80, 80, 254), self.BASE[1]]), ("diff", None))
        self.assertEqual(self.verdict([(80, 80, 80, 250), self.BASE[1]]), ("diff", None))
        self.assertEqual(parity.lif_within_jpeg_bound(lif_of(self.BASE), lif_of(self.BASE)), True)

    def test_only_tagged_cases_and_named_entries(self):
        other = [(88, 80, 80, 255), self.BASE[1]]
        self.assertEqual(self.verdict(other, tags=()), ("diff", None))
        self.assertEqual(self.verdict(other, names=("img/title.lif",)), ("diff", None))
        # autre taille, LIF illisible : ecart
        o, _ = jpeg_runs(lif_of(self.BASE), lif_of(self.BASE))
        _, c = jpeg_runs(lif_of(self.BASE), lif_of(self.BASE + self.BASE, w=4))
        self.assertEqual(parity.compare_runs(o, c, [parity.TAG_JPEG], ["img/s0.lif"])["verdict"],
                         "diff")
        _, c = jpeg_runs(lif_of(self.BASE), b"liff garbage")
        self.assertEqual(parity.compare_runs(o, c, [parity.TAG_JPEG], ["img/s0.lif"])["verdict"],
                         "diff")
        # un autre ecart (avertissements) n'est jamais couvert par la regle
        _, c = jpeg_runs(lif_of(self.BASE), lif_of(other))
        c["warnings"] = ["image_unreadable"]
        r = parity.compare_runs(o, c, [parity.TAG_JPEG], ["img/s0.lif"])
        self.assertEqual(r["verdict"], "diff")
        self.assertEqual([d["kind"] for d in r["diffs"]], ["warnings"])

    def test_campaign_jpeg_outputs(self):
        tmp = tempfile.mkdtemp(prefix="t2f_jpeg_")
        try:
            path = os.path.join(tmp, "a.zip")
            jpeg = b"\xff\xd8\xff\xe0" + bytes(12)
            png_ = b"\x89PNG\r\n\x1a\n" + bytes(8)

            def pred(*files, meta=None, stages=(), inventory=()):
                """Histoire minimale, images référencées par des étapes et l'inventaire."""
                nodes = base_nodes()
                for i, ref in enumerate(stages):
                    nodes["stages"]["x%d" % i] = dict(nodes["stages"]["s0"], image=ref)
                if inventory:
                    nodes["inventory"] = [{"name": "o", "image": ref} for ref in inventory]
                story = [(n, d) for n, d in std_files(meta, nodes) if n not in dict(files)]
                with open(path, "wb") as f:
                    f.write(zip_bytes(story + list(files)))
                return parity.jpeg_outputs(path)

            p = pred(("images/Forêt 1.jpg", jpeg), stages=["Forêt 1.jpg"],
                     inventory=["Forêt 1.jpg"])
            for name in ("img/For_t_1.lif", "img/For_t_1_inv.lif", "img/for_t_1.LIF"):
                self.assertTrue(p(name), name)
            # `_N` sans collision possible, image PNG, couverture PNG, hors de img/ : refusés
            for name in ("img/For_t_1_2.lif", "img/s0.lif", "img/title.lif",
                         "img/thumbnail.lif", "img/For_t_1x.lif", "For_t_1.lif"):
                self.assertFalse(p(name), name)
            # JPEG non référencé : pas de sortie
            self.assertIsNone(pred(("images/Forêt 1.jpg", jpeg)))
            # base tirée de la référence (NFC du JSON), entrée trouvée à la NFC et à la casse
            # près (zip macOS en NFD)
            p = pred(("images/FORE" + chr(0x302) + "T 1.jpg", jpeg), stages=["Forêt 1.jpg"])
            self.assertTrue(p("img/For_t_1.lif"))
            # collisions réelles : même nom à la casse près (deux références, une seule
            # entrée comprise), nom réservé, nom qui prolonge une autre sortie de `_N`
            p = pred(("images/Forêt 1.jpg", jpeg), stages=["Forêt 1.jpg", "forêt 1.JPG"])
            self.assertTrue(p("img/for_t_1_2.lif"))
            p = pred(("images/a.jpg", jpeg), ("images/a.png", png_),
                     stages=["a.jpg", "a.png"])
            self.assertTrue(p("img/a_2.lif"))
            p = pred(("images/title.jpg", jpeg), stages=["title.jpg"])
            # couverture PNG : `img/title.lif` reste comparé à l'identique
            self.assertTrue(p("img/title_2.lif") and not p("img/title.lif"))
            p = pred(("images/a_2.jpg", jpeg), ("images/a.png", png_), ("images/A.png", png_),
                     stages=["a.png", "A.png", "a_2.jpg"])
            self.assertTrue(p("img/a_2_2.lif"))
            # couverture : la première présente parmi image de metadata.json, title.png,
            # cover.png, à la racine de l'histoire
            self.assertTrue(pred(("title.png", jpeg))("img/thumbnail.lif"))
            p = pred(("c.jpg", jpeg), meta={"title": "T", "uuid": "u1", "image": "c.jpg"})
            self.assertTrue(p("img/title.lif") and p("img/thumbnail.lif"))
            self.assertIsNone(pred(("c.png", png_), ("cover.png", jpeg),
                                   meta={"title": "T", "uuid": "u1", "image": "c.png"}))
            self.assertIsNone(pred(("extra/x.jpg", jpeg)))
            self.assertIsNone(pred())
        finally:
            shutil.rmtree(tmp, ignore_errors=True)


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
            f.write(zip_bytes(entries))
        return path

    def test_name_method_utf8_content_and_duplicates(self):
        data = zip_bytes([("a.txt", "x"), ("é.txt", "y")])
        path = os.path.join(self.tmp, "x.pk")
        with open(path, "wb") as f:
            f.write(data)
        e = parity.entries_from_pk(path)
        self.assertEqual([x["name"] for x in e], ["a.txt", "é.txt"])
        self.assertEqual([x["utf8"] for x in e], [False, True])
        self.assertEqual({x["method"] for x in e}, {zipfile.ZIP_STORED})
        dup = zip_bytes(std_files())
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


MATRIX_EXPECTED = os.path.join(ROOT, "crates", "telmi2flam", "tests", "data",
                               "matrix_expected.json")


def png_chunks(data):
    """Chunks d'un PNG (type -> donnees), parcourus en suivant les longueurs."""
    out, pos = {}, 8
    while pos + 8 <= len(data):
        n, kind = struct.unpack(">I4s", data[pos:pos + 8])
        out[kind] = out.get(kind, b"") + data[pos + 8:pos + 8 + n]
        pos += 12 + n
    return out


class TestMatrixGenerator(unittest.TestCase):
    """make_matrix.py : matrice complete, deterministe, identique a celle des references."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="t2f_mx_")

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def test_matrix_covers_real_targets_and_sizes(self):
        with open(MATRIX_EXPECTED, encoding="utf-8") as f:
            frozen = json.load(f)
        ids = set(frozen["lifs"])
        self.assertEqual({i.split("__")[1] for i in ids}, set(make_matrix.OPS))
        for size in (1, 3, 97, 641, 2049):
            self.assertTrue(any("_%dx" % size in i for i in ids), size)
        for mode in make_matrix.MODES:
            self.assertTrue(any(i.startswith(mode + "_") for i in ids), mode)
        # la borne 2047 n'est exercee que par les sources plus grandes
        for i in ids:
            if i.endswith("__bound"):
                self.assertIn("2049", i)
        self.assertIn("RGBA_2049x2049_noise", frozen["pngs"])
        self.assertIn("L_2049x2049_noise", frozen["pngs"])

    def test_cell_rules_and_determinism(self):
        a = make_matrix.matrix_png("RGBA", 97, 97, "noise")
        self.assertEqual(a, make_matrix.matrix_png("RGBA", 97, 97, "noise"))
        self.assertNotEqual(a, make_matrix.matrix_png("RGBA", 97, 97, "gradient"))
        self.assertIsNone(make_matrix.matrix_png("RGB", 97, 97, "alpha"))
        # bruit a 2049 : carre et seulement RGBA / L (sinon les PNG seraient enormes)
        self.assertIsNone(make_matrix.matrix_png("RGB", 2049, 2049, "noise"))
        self.assertIsNone(make_matrix.matrix_png("RGBA", 2049, 1537, "noise"))
        self.assertIsNone(make_matrix.matrix_png("RGBA", 1025, 1025, "noise"))

    def test_png_structure_per_mode(self):
        """Profondeur, type de couleur, PLTE / tRNS et taille des donnees de chaque mode."""
        want = {"L": (8, 0), "LA": (8, 4), "P": (8, 3), "PT": (8, 3), "RGB": (8, 2),
                "RGBA": (8, 6), "1": (1, 0), "I16": (16, 0), "RGB16": (16, 2),
                "RGBA16": (16, 6), "LT": (8, 0), "RGBT": (8, 2), "RGBT16": (16, 2)}
        channels = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}
        self.assertEqual(set(want), set(make_matrix.MODES))
        for mode, (depth, ctype) in want.items():
            w, h = 7, 5
            data = make_matrix.matrix_png(mode, w, h, "gradient")
            ihdr = struct.unpack(">IIBBBBB", data[16:29])
            self.assertEqual(ihdr[:4], (w, h, depth, ctype), mode)
            found = png_chunks(data)
            self.assertEqual(b"PLTE" in found, ctype == 3, mode)
            self.assertEqual(b"tRNS" in found, mode in ("PT", "LT", "RGBT", "RGBT16"), mode)
            stride = 1 + (w * channels[ctype] * depth + 7) // 8
            self.assertEqual(len(zlib.decompress(found[b"IDAT"])), stride * h, mode)
            if ctype == 2 and b"tRNS" in found:
                # cle de couleur franchement non grise, canaux distincts (permutation visible)
                key = struct.unpack(">HHH", found[b"tRNS"])
                self.assertEqual(len(set(key)), 3, (mode, key))
                self.assertGreaterEqual(max(key) - min(key), 100 << (depth - 8), (mode, key))
        self.assertIsNone(make_matrix.matrix_png("RGBT", 7, 5, "alpha"))
        self.assertIsNotNone(make_matrix.matrix_png("RGBA16", 7, 5, "alpha"))

    def test_small_cells_match_frozen_pngs(self):
        """Le generateur n'a pas change depuis le gel : PNG des petites tailles identiques a
        l'octet pres (la CI de parite verifie toute la matrice, matrix_render.rs)."""
        with open(MATRIX_EXPECTED, encoding="utf-8") as f:
            frozen = json.load(f)
        entries = make_matrix.write_matrix(self.tmp, sizes=(1, 3, 97))
        pngs = {e["png"] for e in entries}
        self.assertGreater(len(pngs), 100)
        for rel in pngs:
            with open(os.path.join(self.tmp, rel), "rb") as f:
                fp = make_matrix.png_fingerprint(f.read())
            self.assertEqual(fp, frozen["pngs"][os.path.basename(rel)[:-4]], rel)
        for e in entries:
            self.assertIn(e["id"], frozen["lifs"])

    def test_fingerprint_ignores_zlib_but_not_pixels(self):
        data = make_matrix.matrix_png("RGBA", 7, 5, "gradient")
        chunks = png_chunks(data)
        raw = zlib.decompress(chunks[b"IDAT"])

        def rebuild(idat):
            return (data[:8] + make_matrix._chunk(b"IHDR", chunks[b"IHDR"])
                    + make_matrix._chunk(b"IDAT", idat) + make_matrix._chunk(b"IEND", b""))
        fp = make_matrix.png_fingerprint(data)
        self.assertEqual(make_matrix.png_fingerprint(rebuild(zlib.compress(raw, 9))), fp)
        self.assertEqual(make_matrix.png_fingerprint(rebuild(zlib.compress(raw, 0))), fp)
        altered = raw[:-1] + bytes([raw[-1] ^ 1])
        self.assertNotEqual(make_matrix.png_fingerprint(rebuild(zlib.compress(altered))), fp)


# Faux candidat : repond comme un convertisseur qui echoue toujours avec le code donne.
STUB = """
import json, sys
exit_code, code = int(sys.argv[1]), sys.argv[2]
args = sys.argv[3:]
print(json.dumps({"event": "error", "code": code, "message": "stub", "exit": exit_code}))
sys.exit(exit_code)
"""


def subset_references(src, dest, ids, mutate=None):
    """Copie des references figees limitee aux cas `ids` (et a leurs blobs). `mutate(nom,
    contenu) -> contenu` altere une entree a la copie (tests de references corrompues)."""
    with zipfile.ZipFile(src) as z:
        index = [e for e in json.loads(z.read("corpus.json")) if e["id"] in ids]
        doc = json.loads(z.read("expected.json"))
        doc["cases"] = {k: v for k, v in doc["cases"].items() if k in ids}
        files = [("corpus.json", json.dumps(index).encode()),
                 ("expected.json", json.dumps(doc).encode())]
        files += [(e["archive"], z.read(e["archive"])) for e in index]
        files += [(n, z.read(n)) for n in z.namelist() if n.startswith("blobs/")]
    with zipfile.ZipFile(dest, "w") as out:
        for name, data in files:
            out.writestr(name, mutate(name, data) if mutate else data)
    return dest


class TestFrozenReferences(unittest.TestCase):
    """References figees du corpus (#140) : completes, coherentes, provenance notee."""

    @classmethod
    def setUpClass(cls):
        cls.index, cls.expected, cls.blobs, cls.frozen = parity.load_references(
            parity.DEFAULT_REFERENCES)

    def test_cases_tags_and_jpeg_entries(self):
        ids = [e["id"] for e in self.index]
        self.assertEqual(len(ids), len(set(ids)))
        self.assertGreaterEqual(len(ids), 100)
        by_id = {e["id"]: e for e in self.index}
        for cid in ("ok_plain", "missing_audio_plain", "zip_crc_corrupt_plain"):
            self.assertIn("--plain", by_id[cid]["args"], cid)
        known = {parity.TAG_JSON_LIMIT, parity.TAG_ENCRYPTED, parity.TAG_DELEGATED,
                 parity.TAG_BAD_UTF8_NAME, parity.TAG_NEGATIVE_OFFSET, parity.TAG_JPEG}
        self.assertTrue({t for e in self.index for t in e["tags"]} <= known)
        # chaque regle de la liste fermee est exercee par au moins un cas etiquete
        self.assertEqual({t for e in self.index for t in e["tags"]}, known)
        for e in self.index:
            # D7 : un cas JPEG nomme ses entrees issues d'un JPEG, et seulement lui
            self.assertEqual(bool(e.get("jpeg_entries")), parity.TAG_JPEG in e["tags"], e["id"])
            run = self.expected[e["id"]]
            for key in ("status", "exit", "error_code", "warnings", "info_exit", "info",
                        "entries", "plain_entries", "files"):
                self.assertIn(key, run, (e["id"], key))
            # contenu de chaque entree JPEG produite par l'oracle present (regle jpeg_bounded)
            for entry in run["entries"] or ():
                if entry["name"] in e.get("jpeg_entries", ()):
                    self.assertIn(entry["sha256"], self.blobs, (e["id"], entry["name"]))

    # Contenu fige des references (#140) : un changement d'etiquette (qui active une regle de
    # ALLOWED_DEVIATIONS) ou d'attendu ne se voit pas dans le diff d'un ZIP ; il doit passer
    # par la mise a jour visible de ces empreintes, justifiee (PORT-RUST.md §4.5).
    FROZEN_SHA256 = {
        "corpus.json": "577e758e5192c471dbda965ac01b3b78cee870c3f092eca24fec6e9a91653a06",
        "expected.json": "3ce251c276d224eac7e4fe61afae8775863968ee754c3a6983a659c52fd86912",
    }

    def test_frozen_content_unchanged(self):
        with zipfile.ZipFile(parity.DEFAULT_REFERENCES) as z:
            for name, sha in self.FROZEN_SHA256.items():
                self.assertEqual(hashlib.sha256(z.read(name)).hexdigest(), sha, name)
        self.assertEqual(len(self.index), 101)

    def test_provenance(self):
        self.assertEqual(self.frozen["python"], "3.14.8")
        self.assertEqual(self.frozen["pillow"], "12.3.0")
        self.assertRegex(self.frozen["revision"], r"^[0-9a-f]{7,40}$")

    def test_corrupted_references_are_refused(self):
        tmp = tempfile.mkdtemp(prefix="t2f_refs_")
        try:
            ids = {"ok_basic", "img_jpeg"}

            def bad_blob(name, data):
                return data[:-1] + bytes([data[-1] ^ 1]) if name.startswith("blobs/") else data

            def missing_case(name, data):
                if name != "expected.json":
                    return data
                doc = json.loads(data)
                doc["cases"].pop("ok_basic")
                return json.dumps(doc).encode()
            for mutate in (bad_blob, missing_case):
                path = subset_references(parity.DEFAULT_REFERENCES,
                                         os.path.join(tmp, mutate.__name__ + ".zip"), ids,
                                         mutate)
                with self.assertRaises(parity.ReferencesError):
                    parity.load_references(path)
                err = io.StringIO()
                with contextlib.redirect_stderr(err):
                    rc = parity.main(["corpus", "--references", path, "--candidate", "x"])
                self.assertEqual(rc, 2)
        finally:
            shutil.rmtree(tmp, ignore_errors=True)


class TestBinaryAgainstReferences(unittest.TestCase):
    """Le critere : le binaire Rust contre les references figees, zero ecart hors liste."""

    # un cas par famille (nominal, --plain, -o x.plain, image, noms, MP3, erreurs, regles
    # admises, delegation, JPEG) ; le corpus complet est compare par la CI de parite (#134)
    SAMPLE = ("ok_basic", "ok_plain", "missing_audio_plain", "ok_plain_dir_only",
              "zip_crc_corrupt_plain_dir_only", "img_mode_PT", "names_cp437",
              "mp3_id3_prefix", "missing_audio", "zip_crc_corrupt", "json_deep_nesting",
              "zip_encrypted_entry", "img_bmp_delegated", "zip_invalid_utf8_name",
              "zip_negative_header_offset", "img_jpeg")
    STUB_SAMPLE = ("ok_basic", "zip_crc_corrupt", "json_deep_nesting", "zip_encrypted_entry",
                   "img_bmp_delegated", "zip_invalid_utf8_name", "zip_negative_header_offset")

    @classmethod
    def setUpClass(cls):
        cls.exe = binary()
        cls.tmp = tempfile.mkdtemp(prefix="t2f_ref_")
        cls.refs = subset_references(parity.DEFAULT_REFERENCES,
                                     os.path.join(cls.tmp, "sample.zip"), set(cls.SAMPLE))
        cls.small = subset_references(parity.DEFAULT_REFERENCES,
                                      os.path.join(cls.tmp, "small.zip"), set(cls.STUB_SAMPLE))
        cls.stub = os.path.join(cls.tmp, "stub.py")
        with open(cls.stub, "w", encoding="utf-8") as f:
            f.write(STUB)
        cls.archives = os.path.join(cls.tmp, "archives")
        with zipfile.ZipFile(cls.refs) as z:
            cls.entries = json.loads(z.read("corpus.json"))
            z.extractall(cls.archives, [e["archive"] for e in cls.entries])

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.tmp, ignore_errors=True)

    def test_plain_run_has_directory_and_failure_leaves_nothing(self):
        by_id = {e["id"]: e for e in self.entries}
        for cid, ok in (("ok_plain", True), ("missing_audio_plain", False)):
            work = os.path.join(self.tmp, "w_" + cid)
            run, produced = parity.run_converter(
                [self.exe], os.path.join(self.archives, by_id[cid]["archive"]), work,
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
                [self.exe], os.path.join(self.archives, by_id[cid]["archive"]), work,
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
        cmd = '"%s"' % self.exe.replace("\\", "/")
        self.assertEqual(parity.main(["corpus", "--references", self.refs, "--candidate", cmd,
                                      "--report", report]), 0)
        with open(report, encoding="utf-8") as f:
            rep = json.load(f)
        self.assertEqual(rep["schema"], parity.REPORT_SCHEMA)
        self.assertEqual(rep["summary"]["diff"], 0)
        self.assertEqual(rep["summary"]["total"], len(self.SAMPLE))
        self.assertEqual(rep["delegations"], ["img_bmp_delegated"])
        self.assertEqual(rep["allowed_deviations"],
                         {"encrypted_entry": 1, "invalid_utf8_name": 1, "jpeg_bounded": 1,
                          "json_limit": 1, "negative_offset": 1})
        self.assertEqual([c["id"] for c in rep["cases"]], [e["id"] for e in self.entries])

    def test_altered_candidate_output_is_detected(self):
        """Un candidat dont une entree differe d'un octet (meme une entree JPEG hors borne)
        est en ecart : les references ne laissent rien passer."""
        wrapper = os.path.join(self.tmp, "alter.py")
        with open(wrapper, "w", encoding="utf-8") as f:
            f.write("import subprocess, sys, zipfile, os\n"
                    "r = subprocess.run([%r] + sys.argv[1:])\n"
                    "out = sys.argv[sys.argv.index('-o') + 1] if '-o' in sys.argv else None\n"
                    "if r.returncode == 0 and out and out.endswith('.pk'):\n"
                    "    with zipfile.ZipFile(out) as z:\n"
                    "        items = [(i, z.read(i)) for i in z.infolist()]\n"
                    "    with zipfile.ZipFile(out, 'w') as z:\n"
                    "        for i, d in items:\n"
                    "            if i.filename == 'img/s0.lif':\n"
                    "                d = d[:13] + bytes([d[13] ^ 0x80]) + d[14:]\n"
                    "            z.writestr(i, d)\n"
                    "sys.exit(r.returncode)\n" % self.exe)
        res = {r["id"]: r for r in parity.run_corpus(
            self.refs, [sys.executable, wrapper], validate=False)}
        self.assertEqual(res["ok_basic"]["verdict"], "diff")
        self.assertEqual(res["img_jpeg"]["verdict"], "diff")
        self.assertEqual(res["zip_crc_corrupt"]["verdict"], "identical")

    def candidate(self, exit_code, code):
        return [sys.executable, self.stub, str(exit_code), code]

    def verdicts(self, candidate):
        res = parity.run_corpus(self.small, candidate, validate=False)
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
        # chaque regle ne couvre que son cas etiquete : les deux cas du lecteur ZIP, meme
        # couple de codes, sont admis par leur propre regle et par aucune autre
        self.assertEqual(v["zip_invalid_utf8_name"]["rule"], "invalid_utf8_name")
        self.assertEqual(v["zip_negative_header_offset"]["rule"], "negative_offset")
        self.assertEqual(v["json_deep_nesting"]["verdict"], "diff")
        self.assertEqual(v["zip_crc_corrupt"]["verdict"], "diff")

    def test_fake_candidate_delegation(self):
        v = self.verdicts(self.candidate(7, "unsupported_format"))
        self.assertEqual(v["img_bmp_delegated"]["verdict"], "delegated")
        # un candidat qui delegue tout : seul le cas etiquete est admis
        self.assertEqual({k for k, r in v.items() if r["verdict"] == "diff"},
                         set(self.STUB_SAMPLE) - {"img_bmp_delegated"})
        report = parity.build_report(list(v.values()), "corpus")
        self.assertEqual(report["delegations"], ["img_bmp_delegated"])
        self.assertEqual(parity.exit_code(report), 1)

    def test_cli_exit_code_on_deviation(self):
        cmd = '"%s" "%s" 6 internal_error' % (sys.executable.replace("\\", "/"),
                                              self.stub.replace("\\", "/"))
        self.assertEqual(parity.main(["corpus", "--references", self.small, "--candidate", cmd]),
                         1)


class TestLifDecode(unittest.TestCase):
    def test_opcodes(self):
        # 0xFF (couleur + alpha), repetition (0xC0 | n), delta court, index du cache, 0xFE
        px = lambda r, g, b, a: (r * 256 // 32, g * 256 // 64, b * 256 // 32, a)  # noqa: E731
        data = (b"liff" + (6).to_bytes(4, "big") + (1).to_bytes(4, "big") + b"\xa2"
                + bytes((0xFF, 0x0A | (3 << 5), 0x02 | (5 << 3), 200))   # r5 5, g6 19, b5 10
                + bytes((0xC0 | 1,))                                       # 2 de plus
                + bytes((0x40 | (3 << 4) | (2 << 2) | 1,))                 # db +1, dg 0, dr -1
                + bytes(((5 * 7 + 19 * 5 + 10 * 3) & 0x3F,))               # cache : 1re couleur
                + bytes((0xFE, 0x01, 0x08))                                # r5 1, g6 0, b5 1
                + b"\x00" * 7 + b"\x01")
        rgba, w, h = parity.lif_decode(data)
        self.assertEqual((w, h), (6, 1))
        want = [px(5, 19, 10, 200)] * 3 + [px(4, 19, 11, 200), px(5, 19, 10, 200),
                                           px(1, 0, 1, 200)]
        self.assertEqual(rgba, bytes(v for p in want for v in p))
        with self.assertRaises(ValueError):
            parity.lif_decode(b"PNG garbage")


class TestImageReasons(unittest.TestCase):
    """#456 : IMAGE_REASONS (raisons de delegation d'image reconnues par la campagne) suit
    les raisons `PngError::Unsupported` du decodeur Rust, relues dans les sources."""

    def test_reasons_match_rust_sources(self):
        src = os.path.join(ROOT, "crates", "telmi2flam", "src")
        found = set()
        for name in sorted(os.listdir(src)):
            if name.endswith(".rs"):
                with open(os.path.join(src, name), encoding="utf-8") as f:
                    found |= set(re.findall(r'PngError::Unsupported\("([^"]+)"\)', f.read()))
        self.assertTrue(found)
        self.assertEqual(found, set(parity.IMAGE_REASONS))


DELEGATING_STUB = """
import json, sys
print(json.dumps({"event": "error", "code": "unsupported_format", "exit": 7,
                  "message": "images/x.jpg : format non pris en charge : pas un PNG"}))
sys.exit(7)
"""


ZIP_DELEGATING_STUB = """
import json, sys
print(json.dumps({"event": "error", "code": "unsupported_format", "exit": 7,
                  "message": "a.zip : \\"audios/s0.mp3\\" : compression 12 (bzip2/LZMA/Zstandard) "
                             "d\\u00e9l\\u00e9gu\\u00e9e \\u00e0 Python"}))
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
                f.write(zip_bytes(files, compress))
        self.stub = os.path.join(self.tmp, "deleg.py")
        with open(self.stub, "w", encoding="utf-8") as f:
            f.write(DELEGATING_STUB)
        self.out = os.path.join(self.tmp, "out")
        # moteur de reference et candidat identiques : le binaire Rust (l'ancien oracle Python
        # est supprime, #140)
        self.engine = [binary()]

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def test_helpers(self):
        self.assertEqual(parity.image_format(b"\x89PNG\r\n\x1a\nxxxx"), "png")
        self.assertEqual(parity.image_format(b"\xff\xd8\xff\xdb"), "jpeg")
        self.assertEqual(parity.image_format(b"RIFF\0\0\0\0WEBPVP8 "), "webp")
        self.assertIsNone(parity.image_format(b"ID3\x04"))
        self.assertEqual(parity.delegation_cause(
            "a.zip : \"x\" : compression 12 (bzip2/LZMA/Zstandard) déléguée à Python"), "zip:bzip2")
        self.assertEqual(parity.delegation_cause("i.png : format non pris en charge : pas un PNG"),
                         "image:pas un PNG")
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
            f.write(zip_bytes([("audios/a.m4a", mp4), ("x/b.xml", b"<?xml v"),
                                           ("audios/c.mp3", b"BMxxxxxxxx")], zipfile.ZIP_STORED))
        self.assertEqual(parity.archive_inventory(path)["images"], {})

    def test_delegation_cause_is_anchored_with_closed_reasons(self):
        # un nom d'asset ne fausse pas la cause
        self.assertEqual(parity.delegation_cause(
            "compression 3 (x).png : format non pris en charge : pas un PNG"),
            "image:pas un PNG")
        self.assertEqual(parity.delegation_cause(
            "format non pris en charge : APNG.png : compression 93 (bzip2/LZMA/Zstandard) "
            "déléguée à Python"), "zip:zstd")
        self.assertEqual(parity.delegation_cause("compression 12 sans le reste"), "autre")
        # nom piege cote ZIP : la derniere occurrence (le vrai message) fait foi
        self.assertEqual(parity.delegation_cause(
            "a.zip : \"a : compression 3 (bzip2/LZMA/Zstandard) déléguée à Python\" : "
            "compression 12 (bzip2/LZMA/Zstandard) déléguée à Python"), "zip:bzip2")
        self.assertEqual(parity.delegation_cause(
            "a\nb.png : format non pris en charge : pas un PNG"), "image:pas un PNG")
        # raison hors liste fermee, ou message sans ancre : autre
        self.assertEqual(parity.delegation_cause(
            "i.png : format non pris en charge : raison inventee"), "autre")
        # raison retiree par #459 (PNG atypiques decodes par le Rust) : hors liste
        self.assertEqual(parity.delegation_cause(
            "i.png : format non pris en charge : APNG"), "autre")
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
        oracle = self.engine
        doc = parity.run_campaign(self.archives, oracle, oracle, self.out, validate=False,
                                  limit=1)
        self.assertTrue(doc["partial"])
        self.assertEqual(doc["summary"]["total"], 1)
        with open(os.path.join(self.out, "report.json"), encoding="utf-8") as f:
            self.assertTrue(json.load(f)["partial"])
        # #456 : seuil atteint sur une campagne partielle : code de sortie distinct
        self.assertTrue(doc["threshold_met"])
        self.assertEqual(parity.campaign_exit_code(doc), parity.PARTIAL_EXIT)
        doc = parity.run_campaign(self.archives, oracle, oracle, self.out, validate=False)
        self.assertFalse(doc["partial"])
        self.assertEqual(doc["summary"]["total"], 2)
        self.assertEqual(parity.campaign_exit_code(doc), 0)
        self.assertEqual(parity.campaign_exit_code(dict(doc, threshold_met=False)), 1)
        self.assertEqual(parity.campaign_exit_code(dict(doc, pending=1, partial=True)), 1)
        cmd = '"%s"' % self.engine[0].replace("\\", "/")
        with contextlib.redirect_stdout(io.StringIO()):
            rc = parity.main(["campaign", self.archives, "--candidate", cmd, "--oracle", cmd,
                              "--out", self.out, "--limit", "1", "--no-validate"])
        self.assertEqual(rc, parity.PARTIAL_EXIT)

    def test_exception_in_campaign_case_keeps_only_the_type(self):
        """#456 : la branche d'exception de one() ne garde que le nom du type d'exception."""
        real = parity.campaign_case

        def failing(*_a, **_k):
            raise RuntimeError("secret-path-marker")
        parity.campaign_case = failing
        try:
            doc = parity.run_campaign(self.archives, self.engine, self.engine, self.out,
                                      validate=False)
        finally:
            parity.campaign_case = real
        self.assertEqual(doc["summary"]["diff"], 2)
        self.assertEqual({d["candidate"] for c in doc["cases"] for d in c["diffs"]},
                         {"RuntimeError"})
        self.assertNotIn("secret-path-marker", json.dumps(doc))

    def test_unexpected_exception_in_one_is_a_harness_failure(self):
        oracle = self.engine
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

    def test_same_engine_then_resume(self):
        oracle = self.engine
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

    def test_zip_method_delegation_is_a_diff_since_458(self):
        """bzip2/LZMA/Zstandard lus par le Rust (#458) : un candidat qui delegue encore une
        entree ZIP est en ecart (`delegation`), compte hors des delegations admises."""
        stub = os.path.join(self.tmp, "zipdeleg.py")
        with open(stub, "w", encoding="utf-8") as f:
            f.write(ZIP_DELEGATING_STUB)
        doc = parity.run_campaign(self.archives, self.engine, [sys.executable, stub],
                                  self.out, validate=False)
        self.assertEqual(doc["summary"]["delegated"], 0)
        self.assertEqual(doc["summary"]["diff"], 2)
        self.assertEqual(doc["delegations_by_cause"], {})
        self.assertEqual(doc["diff_kinds"], {"delegation": 2})
        self.assertEqual({c["delegation"] for c in doc["cases"]}, {"zip:bzip2"})
        self.assertFalse(doc["threshold_met"])

    def test_resume_requires_validation_and_redoes_broken_state(self):
        oracle = self.engine
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
        oracle = self.engine
        stub = os.path.join(self.tmp, "stub.py")
        with open(stub, "w", encoding="utf-8") as f:
            f.write(STUB)
        cmd = '"%s" "%s" 6 internal_error' % (sys.executable.replace("\\", "/"),
                                              stub.replace("\\", "/"))
        ref = '"%s"' % self.engine[0].replace("\\", "/")
        with contextlib.redirect_stdout(io.StringIO()):
            rc = parity.main(["campaign", self.archives, "--candidate", cmd, "--oracle", ref,
                              "--out", self.out, "--limit", "1", "--no-validate"])
        self.assertEqual(rc, 1)
        with open(os.path.join(self.out, "report.json"), encoding="utf-8") as f:
            doc = json.load(f)
        self.assertFalse(doc["threshold_met"])
        self.assertTrue(doc["partial"])
        self.assertEqual(doc["summary"]["diff"], 1)
        self.assertIn("exit", doc["diff_kinds"])
        self.assertEqual(doc["cases"][0]["candidate"]["error_code"], "internal_error")
        self.assertEqual(os.listdir(os.path.join(self.out, "work")), ["a"])


class TestFrozenOracleVersions(unittest.TestCase):
    """ORACLE-VERSIONS.txt note l'oracle qui a fige les references (PORT-RUST.md §4.5) ; la CI
    n'installe plus ni Pillow ni oracle (#140)."""

    def read(self, *parts):
        with open(os.path.join(ROOT, *parts), encoding="utf-8") as f:
            return f.read()

    def pins(self):
        text = self.read("tools", "telmi2flam", "ORACLE-VERSIONS.txt")
        return dict(
            line.strip().split("=", 1)
            for line in text.splitlines()
            if line.strip() and not line.lstrip().startswith("#")
        )

    def test_references_record_the_oracle_versions(self):
        pins = self.pins()
        self.assertEqual(sorted(pins), ["pillow", "python", "revision"])
        _, _, _, corpus = parity.load_references(parity.DEFAULT_REFERENCES)
        with open(MATRIX_EXPECTED, encoding="utf-8") as f:
            matrix = json.load(f)["frozen_with"]
        for frozen in (corpus, matrix):
            self.assertEqual(frozen, {"python": pins["python"], "pillow": pins["pillow"],
                                      "revision": pins["revision"]})

    def test_ci_installs_neither_pillow_nor_oracle(self):
        versions = set()
        for name in sorted(os.listdir(os.path.join(ROOT, ".github", "workflows"))):
            yml = self.read(".github", "workflows", name)
            self.assertNotRegex(yml, r"(?i)pip install[^\n]*pillow", name)
            # aucun oracle lance ni attendu (les controles d'absence de l'installateur citent
            # encore telmi2flam/telmi2flam.py : chemin livre, pas l'outil)
            self.assertNotRegex(yml, r"tools/telmi2flam/telmi2flam\.py|gen_\w+\.py", name)
            versions.update(re.findall(r'^\s*python-version:\s*"([^"]+)"\s*$', yml, re.M))
            for line in re.findall(r"^\s*run:\s*pip install (.+)$", yml, re.M):
                pins = dict(re.findall(r'"?([A-Za-z]+)==([^"\s]+)"?', line))
                # lupa seul, version exacte (le Lua de validate.py ne derive pas sans qu'on le voie)
                self.assertEqual(sorted(pins), ["lupa"], name)
                self.assertRegex(pins["lupa"], r"^\d+(\.\d+)+$", name)
                self.assertNotRegex(line, r"[<>~!]=?", name)
        # une seule version de Python pour les outils dans toute la CI
        self.assertEqual(len(versions), 1, versions)


if __name__ == "__main__":
    unittest.main()
