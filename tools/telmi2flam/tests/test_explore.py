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

    def test_max_steps(self):
        pk = self.convert(branching_nodes(), "Steps")
        res = explore.explore(PLAYER, pk[:-3], timeout_s=60, max_steps=3, back_every=0)
        self.assertEqual(res["status"], "ok", self.explain(res))
        self.assertEqual(res["done_reason"], "max_steps")
        self.assertEqual(res["steps"], 3)
        self.assertFalse(res["end_reached"])

    def test_lua_error(self):
        pk = self.convert(branching_nodes(), "Erreur")
        plain = pk[:-3]
        p = os.path.join(plain, "script", "story.lua")
        with open(p, encoding="utf-8") as f:
            src = f.read()
        anchor = "    local st = N.stages[id]\n"
        self.assertIn(anchor, src)
        src = src.replace(anchor, anchor + '    if id == "s3" then local t = nil; t.x = 1 end\n', 1)
        with open(p, "w", encoding="utf-8", newline="\n") as f:
            f.write(src)
        res = explore.explore(PLAYER, plain, timeout_s=90, back_every=0)
        self.assertEqual(res["status"], "lua_error", self.explain(res))
        self.assertTrue(any("attempt to index" in e for e in res["errors"]), self.explain(res))

    def test_infinite_loop(self):
        pk = self.convert(loop_nodes(), "Boucle")
        res = explore.explore(PLAYER, pk[:-3], timeout_s=60, back_every=0)
        self.assertIn(res["status"], ("stuck", "timeout"), self.explain(res))
        self.assertFalse(res["end_reached"])
        self.assertEqual(res["scenes_visited"], 2)


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
