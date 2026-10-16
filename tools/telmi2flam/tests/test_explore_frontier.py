"""
Tests de la frontiere de l'exploration (explore.FRONTIER_LUA) sur des graphes
synthetiques, avec lupa : ni player ni histoire reelle.

    python -m unittest discover -s tools/telmi2flam/tests -v

Sautes si lupa est absent.
"""
import os
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.dirname(HERE)
sys.path.insert(0, TOOL)
sys.dont_write_bytecode = True

import explore  # noqa: E402

try:
    from lupa import LuaRuntime
except ImportError:   # lupa optionnel
    LuaRuntime = None

HOME = {"ok": False, "home": True, "autoplay": True}
NOHOME = {"ok": True, "home": False, "autoplay": True}


def st(ok=None, home=None, ctrl=NOHOME, ok_index=None, home_index=None):
    """Stage au format nodes.lua. ok / home : id d'action."""
    d = {"ctrl": ctrl}
    if ok:
        d["ok"] = {"action": ok}
        if ok_index is not None:
            d["ok"]["indexItem"] = ok_index
    if home:
        d["home"] = {"action": home}
        if home_index is not None:
            d["home"]["indexItem"] = home_index
    return d


GUARD = [{"item": 5, "cmp": 2, "num": 1}]     # objet 5 == 1 : jamais pose par l'histoire


@unittest.skipIf(LuaRuntime is None, "lupa absent")
class TestFrontier(unittest.TestCase):
    def build(self, nodes, seen, shown=()):
        """(F, lua) : frontiere sur le graphe `nodes` ; seen / shown : ids."""
        lua = LuaRuntime(unpack_returned_tuples=True)
        def tolua(o):
            if isinstance(o, dict):
                t = lua.table()
                for k, v in o.items():
                    t[k] = tolua(v)
                return t
            if isinstance(o, (list, tuple)):
                t = lua.table()
                for i, v in enumerate(o, 1):
                    t[i] = tolua(v)
                return t
            return o
        N = tolua(nodes)
        S = tolua({i: True for i in seen})
        H = tolua({i: True for i in shown})
        new = lua.execute(explore.FRONTIER_LUA + "\nreturn newFrontier")
        return new(N, S, H), S

    # --- #76 : conditions et indexItem comptes ---------------------------------

    def doudou(self):
        """Miniature de Doudou : un hub dont l'unique issue vers la suite est gardee
        par une condition jamais satisfaite ; le reste de l'histoire est vu."""
        return {
            "stages": {
                "s0": st(ok="a0"), "hub": st(ok="ahub"), "fin": st(),
                "secret": st(ok="asec"), "suite": st(),
            },
            "actions": {
                "a0": [{"stage": "hub"}],
                "ahub": [{"stage": "fin"}, {"stage": "secret", "cond": GUARD}],
                "asec": [{"stage": "suite"}],
            },
        }

    def test_scene_derriere_condition_comptee(self):
        F, _ = self.build(self.doudou(), ["s0", "hub", "fin"])
        self.assertEqual(F.frontier(), 0)          # rien de sans condition
        self.assertEqual(F.condFrontier(), 1)      # mais `secret` reste derriere GUARD

    def test_condition_satisfaite_plus_comptee(self):
        F, _ = self.build(self.doudou(), ["s0", "hub", "fin", "secret"])
        self.assertEqual(F.condFrontier(), 0)
        # `suite` : suite sans condition de `secret`, comptee par la frontiere ok
        self.assertEqual(F.frontier(), 1)

    def test_index_item_compte(self):
        nodes = {
            "stages": {"s0": st(ok="a0", ok_index=3), "x": st(), "y": st()},
            "actions": {"a0": [{"stage": "x"}, {"stage": "y"}]},
        }
        F, _ = self.build(nodes, ["s0", "x"])
        self.assertEqual(F.frontier(), 0)          # indexItem : ignore par frontier()
        self.assertEqual(F.condFrontier(), 1)      # y atteignable selon l'objet
        F, _ = self.build(nodes, ["s0", "x", "y"])
        self.assertEqual(F.condFrontier(), 0)

    def test_complete_sans_condition(self):
        nodes = {"stages": {"a": st(ok="a0"), "b": st()}, "actions": {"a0": [{"stage": "b"}]}}
        F, _ = self.build(nodes, ["a", "b"])
        self.assertEqual(F.restartVerdict(0, 2), "complete")

    def test_conditions_jamais_satisfaites_pas_complete(self):
        """Doudou en miniature : jamais `complete` tant que `secret` n'est pas vue ;
        3 redemarrages sans nouveaute sont permis, puis `conditions`."""
        F, _ = self.build(self.doudou(), ["s0", "hub", "fin"])
        self.assertEqual([F.restartVerdict(0, 7) for _ in range(3)], [None, None, None])
        self.assertEqual(F.restartVerdict(0, 7), "conditions")

    def test_nouveaute_relance_le_compte(self):
        F, _ = self.build(self.doudou(), ["s0", "hub", "fin"])
        for _ in range(3):
            self.assertIsNone(F.restartVerdict(0, 7))
        self.assertIsNone(F.restartVerdict(0, 8))   # une scene / option nouvelle
        self.assertIsNone(F.restartVerdict(0, 8))
        self.assertIsNone(F.restartVerdict(0, 8))
        self.assertEqual(F.restartVerdict(0, 8), "conditions")

    def test_option_a_essayer_reste_prioritaire(self):
        F, _ = self.build(self.doudou(), ["s0", "hub", "fin"])
        self.assertIsNone(F.restartVerdict(2, 7))   # pending > 0 : reprise, compte remis a 0
        self.assertEqual([F.restartVerdict(0, 7) for _ in range(3)], [None] * 3)
        self.assertEqual(F.restartVerdict(0, 7), "conditions")

    def test_condition_atteinte_pendant_la_reprise(self):
        F, S = self.build(self.doudou(), ["s0", "hub", "fin"])
        self.assertIsNone(F.restartVerdict(0, 7))
        S["secret"] = True                          # le pilote a satisfait la condition
        S["suite"] = True
        self.assertEqual(F.restartVerdict(0, 9), "complete")

    # --- #87 : entrees conditionnelles de `home` ------------------------------

    def home_nodes(self, home_index=None):
        return {
            "stages": {
                "m": st(ok="am", home="ah", ctrl=HOME, home_index=home_index),
                "z": st(), "g": st(),
            },
            "actions": {
                "am": [{"stage": "z"}],
                "ah": [{"stage": "z"}, {"stage": "g", "cond": GUARD}],
            },
        }

    def test_home_conditionnelle_parcourue(self):
        F, _ = self.build(self.home_nodes(), ["m", "z"])
        self.assertEqual(F.homeFrontier(), 0)       # homeNew ignore `cond`
        self.assertEqual(F.condFrontier(), 1)       # #87 : condFrontier la voit
        self.assertIsNone(F.restartVerdict(0, 1))

    def test_home_index_item_parcourue(self):
        nodes = self.home_nodes(home_index=2)
        nodes["actions"]["ah"] = [{"stage": "z"}, {"stage": "g"}]
        F, _ = self.build(nodes, ["m", "z"])
        self.assertEqual(F.homeFrontier(), 1)       # g sans condition : home utile
        self.assertEqual(F.condFrontier(), 1)

    def test_home_inutilisable_ignoree(self):
        nodes = self.home_nodes()
        nodes["stages"]["m"]["ctrl"] = NOHOME       # pas de home sur ce noeud
        F, _ = self.build(nodes, ["m", "z"])
        self.assertEqual(F.condFrontier(), 0)

    def test_home_cible_vue_ne_compte_pas(self):
        F, _ = self.build(self.home_nodes(), ["m", "z", "g"])
        self.assertEqual(F.condFrontier(), 0)

    # --- non-regression : frontiere #45 et choix affiches --------------------

    def test_chemin_lineaire_coupe(self):
        nodes = {"stages": {"a": st(ok="a0"), "b": st(ok="a1"), "c": st()},
                 "actions": {"a0": [{"stage": "b"}], "a1": [{"stage": "c"}]}}
        F, _ = self.build(nodes, ["a", "b"])
        self.assertEqual(F.frontier(), 1)
        self.assertIsNone(F.restartVerdict(0, 3))

    def test_choix_affiche_suivi_par_pending(self):
        nodes = {"stages": {"h": st(ok="ac"), "o1": st(), "o2": st()},
                 "actions": {"ac": [{"stage": "o1"}, {"stage": "o2"}]}}
        F, _ = self.build(nodes, ["h", "o1"], shown=["ac"])
        self.assertEqual(F.frontier(), 0)
        F, _ = self.build(nodes, ["h", "o1"])
        self.assertEqual(F.frontier(), 1)

    def test_pilote_instancie_la_frontiere(self):
        """Le gabarit du pilote contient bien la frontiere et la decision (pas de '%')."""
        self.assertIn("F.restartVerdict", explore.DRIVER)
        self.assertIn("%(frontier)s", explore.DRIVER)
        self.assertIn("newFrontier", explore.FRONTIER_LUA)
        # pilote complet rendu : syntaxe Lua valide
        src = explore.DRIVER % {"frontier": explore.FRONTIER_LUA.replace("%", "%%"),
                                "seed": 0, "max_steps": 10, "back_every": 8}
        ok = LuaRuntime().eval("function(s) return load(s) ~= nil end")(src)
        self.assertTrue(ok)


if __name__ == "__main__":
    unittest.main()
