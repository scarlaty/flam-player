"""
Tests de validate.py sur des paquets produits par le binaire telmi2flam (repris de l'ancien
test_telmi2flam.py, #140) : validate.py sert d'oracle independant en CI (parity.py, test de
l'archive des podcasts du launcher), il doit donc rester teste directement.

    python -m unittest discover -s tools/telmi2flam/tests -v
"""
import contextlib
import io
import os
import subprocess
import sys
import unittest
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.dont_write_bytecode = True

from campaign_fixtures import Base, base_nodes, std_files  # noqa: E402  (ajoute TOOL au sys.path)
import converter_bin  # noqa: E402
import validate  # noqa: E402


def setUpModule():
    """Echec clair si le binaire est absent (pas de saut silencieux, #465)."""
    global EXE
    try:
        EXE = converter_bin.find_converter()
    except converter_bin.ConverterNotFound as e:
        raise RuntimeError("test_validate.py exige le binaire telmi2flam : %s" % e)


class TestValidate(Base):
    def convert(self, files, name="in", out=None):
        """Archive TELMI convertie par le binaire ; `out` : sortie (.plain.pk ou dossier
        .plain), defaut <tmp>/<name>.plain.pk."""
        src = self.mkzip(files, name)
        out = out or os.path.join(self.tmp, name + ".plain.pk")
        r = subprocess.run([EXE, src, "-o", out], capture_output=True, text=True,
                           encoding="utf-8", errors="replace",
                           env=dict(os.environ, FLAM_FFMPEG=""))
        self.assertEqual(r.returncode, 0, r.stderr)
        return out

    def rewrite(self, pk, changes):
        out = pk + ".mod.pk"
        with zipfile.ZipFile(pk) as zi, zipfile.ZipFile(out, "w", zipfile.ZIP_STORED) as zo:
            for i in zi.infolist():
                if i.filename in changes:
                    if changes[i.filename] is not None:
                        zo.writestr(i.filename, changes[i.filename])
                else:
                    zo.writestr(i.filename, zi.read(i.filename))
            for k, v in changes.items():
                if k not in zi.namelist() and v is not None:
                    zo.writestr(k, v)
        return out

    def run_validate(self, pk):
        with contextlib.redirect_stdout(io.StringIO()) as out:
            rc = validate.main(pk)
        return rc, out.getvalue()

    def test_valid_package(self):
        rc, out = self.run_validate(self.convert(std_files()))
        self.assertEqual(rc, 0, out)
        self.assertIn("nodes.lua charge", out)

    def test_detects_problems(self):
        n = base_nodes()
        n["inventory"] = [{"name": "x", "initialNumber": 0, "maxNumber": 3, "display": 0}]
        n["actions"]["a0"][0]["conditions"] = [{"comparator": 0, "item": 5, "number": 1}]
        rc, out = self.run_validate(self.convert(std_files(nodes=n)))
        self.assertEqual(rc, 1)
        self.assertIn("inventaire", out)
        cases = {
            "uuid.bin": None,                                  # absent : pas de plantage
            "img/s0.lif": b"xxxx" + bytes(30),                 # en-tete LIF invalide
            "sounds/s0.mp3map": bytes(12) + (99).to_bytes(4, "little") + bytes(4),
            "../evil.txt": b"x",                               # nom dangereux
        }
        pk = self.convert(std_files(), name="ok")
        for k, v in cases.items():
            rc, out = self.run_validate(self.rewrite(pk, {k: v}))
            self.assertEqual(rc, 1, "%s\n%s" % (k, out))
            self.assertNotIn("Traceback", out)
        if validate.LuaRuntime is not None or validate.find_lua():
            rc, out = self.run_validate(self.rewrite(pk, {"main.lua": b"local = 1\n"}))
            self.assertEqual(rc, 1, out)
            self.assertIn("main.lua : ERREUR syntaxe", out)

    # #44 : home -> backAction -> backStage -> backChildAction vide = convention Telmi Sync
    # "quitter l'histoire" : pas d'avertissement ; une action vide atteinte par ok reste
    # signalee.
    def back_nodes(self, ok_to_empty=False):
        n = base_nodes()
        n["stages"]["s0"]["control"] = {"ok": True, "home": True, "autoplay": True}
        n["stages"]["s0"]["home"] = {"action": "backAction", "index": 0}
        n["stages"]["backStage"] = {"image": None, "audio": None,
                                    "ok": {"action": "backChildAction", "index": 0},
                                    "home": None,
                                    "control": {"ok": True, "home": False, "autoplay": True}}
        n["actions"]["backAction"] = [{"stage": "backStage"}]
        n["actions"]["backChildAction"] = []
        if ok_to_empty:
            n["stages"]["s0"]["ok"] = {"action": "finVide", "index": 0}
            n["actions"]["finVide"] = []
        return n

    def test_back_child_action_empty_no_warning(self):
        rc, out = self.run_validate(self.convert(std_files(nodes=self.back_nodes())))
        self.assertEqual(rc, 0, out)
        self.assertNotIn("vide", out)

    def test_empty_action_via_ok_still_warns(self):
        rc, out = self.run_validate(self.convert(std_files(nodes=self.back_nodes(True))))
        self.assertIn("stage s0.ok -> action 'finVide' vide", out)
        self.assertNotIn("backStage.ok -> action 'backChildAction' vide", out)

    def test_shared_empty_action_still_warns(self):
        n = self.back_nodes()
        n["stages"]["s0"]["ok"] = {"action": "backChildAction", "index": 0}
        rc, out = self.run_validate(self.convert(std_files(nodes=n)))
        self.assertIn("stage s0.ok -> action 'backChildAction' vide", out)

    # index -1 (tirage aleatoire, spec TELMI) n'est pas "hors de l'action"
    def test_random_index_accepted(self):
        n = base_nodes()
        n["stages"]["s1"] = {"image": "s0.png", "control": {}}
        n["actions"]["a1"] = [{"stage": "s0"}, {"stage": "s1"}]
        n["stages"]["s0"]["ok"] = {"action": "a1", "index": -1}
        n["stages"]["s1"]["home"] = {"action": "a1", "index": 5}
        rc, out = self.run_validate(self.convert(std_files(nodes=n)))
        self.assertEqual(rc, 0, out)
        self.assertNotIn("index -1", out)
        self.assertIn("index 5 hors", out)

    # paquet sans runtime (setup() echouerait)
    def test_runtime_entries_required(self):
        pk = self.convert(std_files())
        for k in ("script/global.lua", "script/list-choice_1_0_0.lua",
                  "img/script/arrow-right-ui-000.lif"):
            rc, out = self.run_validate(self.rewrite(pk, {k: None}))
            self.assertEqual(rc, 1, "%s\n%s" % (k, out))
            self.assertIn(k + " absente", out)

    # dossier .plain -> memes controles
    def test_plain_dir(self):
        plain = self.convert(std_files(), out=os.path.join(self.tmp, "d.plain"))
        self.assertTrue(os.path.isdir(plain))
        rc, out = self.run_validate(plain)
        self.assertEqual(rc, 0, out)
        self.assertNotIn("illisible", out)
        self.assertIn("STORED non applicable", out)
        self.assertIn("nodes.lua charge", out)
        self.assertIn("tous les stages sont atteignables", out)
        pk = self.convert(std_files(), name="same")
        rc_pk, out_pk = self.run_validate(pk)
        self.assertEqual(rc_pk, 0, out_pk)
        self.assertEqual(out.count("[OK]"), out_pk.count("[OK]"))
        os.remove(os.path.join(plain, "script", "global.lua"))
        with open(os.path.join(plain, "img", "s0.lif"), "wb") as f:
            f.write(b"xxxx" + bytes(30))
        rc, out = self.run_validate(plain)
        self.assertEqual(rc, 1, out)
        self.assertIn("script/global.lua absente", out)
        self.assertIn("LIF img/s0.lif", out)
        self.assertNotIn("Traceback", out)


class TestRuntimeManifest(Base):
    def write(self, content):
        path = os.path.join(self.tmp, "manifest.txt")
        with open(path, "w", encoding="utf-8") as f:
            f.write(content)
        return path

    def test_without_required_lines_is_an_error(self):
        for content in ("", "# commentaire seul\n\n# autre\n", "!script/global.lua\n",
                        "!img/script/a.lif\n"):
            with self.assertRaises(ValueError, msg=repr(content)):
                validate._runtime_required(self.write(content))

    def test_bom_comments_and_optional_lines(self):
        self.assertEqual(validate._runtime_required(
            self.write("﻿!script/global.lua\n!img/script/a.lif\n")),
            (("global.lua",), ("a.lif",)))
        self.assertEqual(validate._runtime_required(self.write(
            "# !script/ignore.lua\n  !script/global.lua  \nscript/autre.lua\n"
            "#!img/script/b.lif\n!img/script/a.lif\n")), (("global.lua",), ("a.lif",)))

    def test_repository_manifest_matches_dirs(self):
        with open(validate.RUNTIME_MANIFEST, encoding="utf-8") as f:
            listed = [ln.strip().lstrip("!") for ln in f if ln.strip() and not ln.startswith("#")]
        root = os.path.dirname(validate.RUNTIME_MANIFEST)
        on_disk = (["script/" + n for n in sorted(os.listdir(os.path.join(root, "script")))
                    if n.endswith(".lua")]
                   + ["img/script/" + n
                      for n in sorted(os.listdir(os.path.join(root, "img", "script")))
                      if n.endswith(".lif")])
        self.assertEqual(listed, on_disk)
        self.assertEqual(validate.LIF_MAX_DIM, 2047)


class TestParseLuaData(unittest.TestCase):
    def test_subset(self):
        src = ('return {\n  a = "x\\ny", b = 1, c = -2.5, d = true, e = nil,\n'
               '  t = { 1, 2, { k = "v" } },\n  ["s p"] = false\n}\n')
        self.assertEqual(validate.parse_lua_data(src),
                         {"a": "x\ny", "b": 1, "c": -2.5, "d": True, "e": None,
                          "t": [1, 2, {"k": "v"}], "s p": False})


if __name__ == "__main__":
    unittest.main()
