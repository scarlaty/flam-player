"""
Tests de l'explorateur sans fenetre (explore.py) sur des paquets TELMI
synthetiques convertis a la volee.

    python -m unittest discover -s tools/telmi2flam/tests -v

Necessitent flam-player.exe (variable FLAM_BUILD_DIR, sinon <depot>/build) :
sautes sinon.
"""
import contextlib
import io
import json
import os
import shutil
import sys
import tempfile
import time
import unittest
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.dirname(HERE)
sys.path.insert(0, TOOL)
sys.path.insert(0, HERE)
sys.dont_write_bytecode = True

import explore  # noqa: E402
import telmi2flam as T  # noqa: E402
from test_telmi2flam import mp3_frames, png  # noqa: E402

PLAYER = explore.find_player()
CTL = {"ok": True, "home": False, "autoplay": True}
# options d'un choix : non autoplay (sinon TELMI joue la scene sans afficher le choix)
OPT = {"ok": True, "home": False, "autoplay": False}


def stage(audio, ok, ctrl=CTL, image="s.png"):
    return {"image": image, "audio": audio, "ok": ok and {"action": ok, "index": 0},
            "home": None, "control": ctrl}


def branching_nodes():
    """start -> p0 (passage) -> s0 -> s1 -> choix {o1, o2} ; o1 -> s2 -> fin,
    o2 -> s3 -> s4 -> fin. 8 stages, 2 fins."""
    return {
        "startAction": {"action": "a0", "index": 0},
        "stages": {
            "p0": {"image": None, "audio": None, "ok": {"action": "a1", "index": 0},
                   "home": None, "control": CTL},
            "s0": stage("x.mp3", "a2"),
            "s1": stage("x.mp3", "ac"),
            "o1": stage("x.mp3", "a3", OPT),
            "o2": stage("x.mp3", "a4", OPT),
            "s2": stage("x.mp3", None),
            "s3": stage("x.mp3", "a5"),
            "s4": stage("x.mp3", None),
        },
        "actions": {
            "a0": [{"stage": "p0"}], "a1": [{"stage": "s0"}], "a2": [{"stage": "s1"}],
            "ac": [{"stage": "o1"}, {"stage": "o2"}],
            "a3": [{"stage": "s2"}], "a4": [{"stage": "s3"}], "a5": [{"stage": "s4"}],
        },
    }


def loop_nodes():
    """start -> s0 -> s1 -> s0 ... : aucune fin."""
    return {
        "startAction": {"action": "a0", "index": 0},
        "stages": {"s0": stage("x.mp3", "a1"), "s1": stage("x.mp3", "a0")},
        "actions": {"a0": [{"stage": "s0"}], "a1": [{"stage": "s1"}]},
    }


@unittest.skipIf(PLAYER is None, "flam-player introuvable (FLAM_BUILD_DIR ou build/)")
class TestExplore(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="t2f_explore_")

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def convert(self, nodes, name):
        zpath = os.path.join(self.tmp, name + ".zip")
        with zipfile.ZipFile(zpath, "w") as z:
            z.writestr("metadata.json", json.dumps({"title": name, "uuid": "explore-" + name}))
            z.writestr("nodes.json", json.dumps(nodes))
            z.writestr("images/s.png", png(320, 240))
            z.writestr("audios/x.mp3", mp3_frames(n=20))   # ~0,5 s
        pk = os.path.join(self.tmp, name + ".plain.pk")
        with contextlib.redirect_stdout(io.StringIO()):
            T.convert(zpath, pk, emit_plain=True)
        return pk

    def explain(self, res):
        return json.dumps(res, ensure_ascii=False, indent=1)

    def test_complete_ok(self):
        pk = self.convert(branching_nodes(), "Branches")
        res = explore.explore(PLAYER, pk, timeout_s=90, back_every=2)   # .plain.pk extrait
        self.assertEqual(res["status"], "ok", self.explain(res))
        self.assertEqual(res["done_reason"], "complete", self.explain(res))
        self.assertTrue(res["end_reached"])
        self.assertEqual(res["ends"], 2, self.explain(res))
        self.assertEqual(res["scenes_total"], 8)
        self.assertEqual(res["scenes_visited"], 8, self.explain(res))
        self.assertEqual(res["choices_explored"], 2)
        self.assertEqual(res["errors"], [])
        self.assertGreaterEqual(res["back_tests"], 1, self.explain(res))
        self.assertGreaterEqual(res["menu_tests"], 1, self.explain(res))
        self.assertIsNone(res["exit_code"])
        self.assertEqual(res["coverage"], 1)
        self.assertEqual(res["dead_end_backs"], 0)

    def test_max_steps(self):
        pk = self.convert(branching_nodes(), "Steps")
        res = explore.explore(PLAYER, pk[:-3], timeout_s=60, max_steps=3, back_every=0)
        self.assertEqual(res["status"], "ok", self.explain(res))
        self.assertEqual(res["done_reason"], "max_steps")
        self.assertEqual(res["steps"], 3)
        self.assertEqual(res["seed"], 0)
        self.assertLess(res["coverage"], 1)
        self.assertFalse(res["end_reached"])

    def inject(self, plain, code):
        """Ajoute `code` au debut de enterStage (script/story.lua)."""
        p = os.path.join(plain, "script", "story.lua")
        with open(p, encoding="utf-8") as f:
            src = f.read()
        anchor = "    local st = N.stages[id]\n"
        self.assertIn(anchor, src)
        with open(p, "w", encoding="utf-8", newline="\n") as f:
            f.write(src.replace(anchor, anchor + code + "\n", 1))

    def test_lua_error(self):
        pk = self.convert(branching_nodes(), "Erreur")
        plain = pk[:-3]
        self.inject(plain, '    if id == "s3" then local t = nil; t.x = 1 end')
        res = explore.explore(PLAYER, plain, timeout_s=90, back_every=0)
        self.assertEqual(res["status"], "lua_error", self.explain(res))
        self.assertEqual(res["done_reason"], "error")
        self.assertTrue(any("attempt to index" in e for e in res["errors"]), self.explain(res))
        # s3 n'est atteinte que par l'option o2 : l'erreur survient avant le
        # noeud suivant, o2 ne doit donc pas etre comptee comme visitee
        # (p0 s0 s1 = 3, ou + o1 s2 = 5 si o1 a ete essayee avant)
        self.assertIn(res["scenes_visited"], (3, 5), self.explain(res))

    def test_lua_error_then_exit(self):
        # erreur Lua fatale puis sortie du player (os.exit) : lua_error, pas crash
        pk = self.convert(branching_nodes(), "Fatale")
        plain = pk[:-3]
        self.inject(plain, '    if id == "s1" then local ok, e = pcall(function() local t = nil; t.x = 1 end); '
                           'io.stderr:write("Erreur Lua: " .. tostring(e) .. "\\n"); io.stderr:flush(); '
                           'os.exit(3) end')
        res = explore.explore(PLAYER, plain, timeout_s=60, back_every=0)
        self.assertEqual(res["status"], "lua_error", self.explain(res))
        self.assertEqual(res["exit_code"], 3, self.explain(res))
        self.assertEqual(res["done_reason"], "exit")

    def test_infinite_loop(self):
        pk = self.convert(loop_nodes(), "Boucle")
        res = explore.explore(PLAYER, pk[:-3], timeout_s=60, back_every=0)
        self.assertEqual(res["status"], "stuck", self.explain(res))
        self.assertEqual(res["done_reason"], "stuck")
        self.assertFalse(res["end_reached"])
        self.assertEqual(res["scenes_visited"], 2)


def fake_story(tmp, main_py):
    """Faux paquet pour un faux player (python) : `python <dossier>` execute
    <dossier>/__main__.py. Teste la classification sans flam-player."""
    plain = os.path.join(tmp, "Faux.plain")
    os.makedirs(plain)
    with open(os.path.join(plain, "__main__.py"), "w", encoding="utf-8") as f:
        f.write(main_py)
    return plain


class TestFakePlayer(unittest.TestCase):
    """Statuts crash / lua_error / stuck / timeout avec un faux player."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="t2f_explore_")

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def run_fake(self, main_py, **kw):
        return explore.explore(sys.executable, fake_story(self.tmp, main_py), **kw)

    def test_crash(self):
        res = self.run_fake("import sys\nsys.exit(5)\n", timeout_s=20)
        self.assertEqual(res["status"], "crash", res)
        self.assertEqual(res["exit_code"], 5)
        self.assertEqual(res["done_reason"], "exit")

    def test_lua_error_before_exit(self):
        res = self.run_fake("import sys\n"
                            "sys.stderr.write('Erreur Lua: main.lua:3: attempt to call a nil value\\n')\n"
                            "sys.stderr.flush()\n"
                            "sys.exit(3)\n", timeout_s=20)
        self.assertEqual(res["status"], "lua_error", res)
        self.assertEqual(res["exit_code"], 3)

    def test_stuck_no_trace(self):
        res = self.run_fake("import time\n"
                            "print('[EXPLORE] start seed=0', flush=True)\n"
                            "time.sleep(60)\n", timeout_s=20, stuck_s=1)
        self.assertEqual(res["status"], "stuck", res)
        self.assertEqual(res["done_reason"], "no_trace")
        self.assertIsNone(res["exit_code"])
        self.assertLess(res["duration_s"], 15)

    def test_timeout_while_progressing(self):
        res = self.run_fake("import time\n"
                            "print('[EXPLORE] start seed=0', flush=True)\n"
                            "for i in range(600):\n"
                            "    print('[EXPLORE] step=%d x' % (i + 1), flush=True)\n"
                            "    time.sleep(0.1)\n", timeout_s=2, stuck_s=30)
        self.assertEqual(res["status"], "timeout", res)
        self.assertEqual(res["done_reason"], "timeout")
        self.assertGreater(res["steps"], 0)


class TestRunPlayer(unittest.TestCase):
    def test_no_orphan_on_exception(self):
        tmp = tempfile.mkdtemp(prefix="t2f_explore_")
        try:
            beat = os.path.join(tmp, "beat.txt")
            script = os.path.join(tmp, "child.py")
            with open(script, "w", encoding="utf-8") as f:
                f.write("import time\n"
                        "for i in range(600):\n"
                        "    open(%r, 'a').write('.')\n"
                        "    time.sleep(0.05)\n" % beat)

            def until():
                if os.path.exists(beat):
                    raise RuntimeError("erreur dans until")
                return False

            with self.assertRaises(RuntimeError):
                explore.run_player(sys.executable, [script], tmp, 30, until=until)
            time.sleep(0.5)
            size = os.path.getsize(beat)
            time.sleep(0.5)
            self.assertEqual(os.path.getsize(beat), size, "processus enfant encore vivant")
        finally:
            shutil.rmtree(tmp, ignore_errors=True)


class TestPrepare(unittest.TestCase):
    def test_pk_zip_slip(self):
        tmp = tempfile.mkdtemp(prefix="t2f_explore_")
        try:
            pk = os.path.join(tmp, "Evil.plain.pk")
            with zipfile.ZipFile(pk, "w") as z:
                z.writestr("../evil.txt", "x")
            with self.assertRaises(ValueError):
                explore.prepare(pk, os.path.join(tmp, "work"))
            self.assertFalse(os.path.exists(os.path.join(tmp, "evil.txt")))
        finally:
            shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    unittest.main()
