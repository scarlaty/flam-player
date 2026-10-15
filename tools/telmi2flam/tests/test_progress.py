"""
Tests de la sortie machine de telmi2flam : evenements --progress-json, codes de
sortie, annulation sans sortie partielle, --info.

    python -m unittest discover -s tools/telmi2flam/tests -v
"""
import contextlib
import io
import json
import os
import signal
import subprocess
import sys
import unittest
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

from test_telmi2flam import Base, TOOL, base_nodes, std_files  # noqa: E402  (ajoute TOOL au sys.path)
import telmi2flam as T  # noqa: E402

CLI = os.path.join(TOOL, "telmi2flam.py")


def two_stage_nodes():
    n = base_nodes()
    n["stages"]["s1"] = {"image": "s1.png", "audio": "s1.mp3", "ok": None, "home": None,
                         "control": {"ok": True, "home": False, "autoplay": True}}
    n["actions"]["a1"] = [{"stage": "s1"}]
    return n


def parse_lines(text):
    return [json.loads(line) for line in text.splitlines() if line.strip()]


class CliBase(Base):
    def cli(self, *args):
        r = subprocess.run([sys.executable, CLI] + list(args), capture_output=True, text=True,
                           encoding="utf-8", errors="replace")
        return r

    def events(self, files, **kw):
        """Conversion par fonction avec collecte des evenements."""
        evs = []
        zp = self.mkzip(files)
        out = kw.pop("out", os.path.join(self.tmp, "o.plain.pk"))
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            T.convert(zp, out, progress=evs.append, **kw)
        return evs, out

    def others(self):
        """Contenu du dossier de test hors archive d'entree."""
        return sorted(n for n in os.listdir(self.tmp) if n != "in.zip")

    def leftovers(self):
        return [n for n in os.listdir(self.tmp) if n.endswith(".part")]


class TestEvents(CliBase):
    def setUp(self):
        super().setUp()
        self._interval = T.PROGRESS_INTERVAL
        T.PROGRESS_INTERVAL = 0      # tous les "step" (deterministe)

    def tearDown(self):
        T.PROGRESS_INTERVAL = self._interval
        super().tearDown()

    def test_start_first_done_last(self):
        evs, out = self.events(std_files())
        self.assertEqual(evs[0]["event"], "start")
        self.assertEqual(evs[0]["total"], 4)
        self.assertEqual(evs[0]["phases"], ["images", "audio", "scripts", "zip"])
        self.assertTrue(evs[0]["input"].endswith("in.zip"))
        self.assertEqual(evs[-1]["event"], "done")
        self.assertEqual(evs[-1]["output"], out)
        self.assertEqual(evs[-1]["size"], os.path.getsize(out))
        self.assertEqual(evs[-1]["warnings"], 0)
        self.assertEqual([e["event"] for e in evs].count("done"), 1)

    def test_steps_per_phase(self):
        files = [(n, d) for n, d in std_files(nodes=two_stage_nodes())
                 if not n.startswith(("images/", "audios/"))]
        from test_telmi2flam import png, MP3
        files += [("images/s0.png", png(32, 32)), ("images/s1.png", png(32, 32)),
                  ("audios/s0.mp3", MP3), ("audios/s1.mp3", MP3)]
        evs, out = self.events(files)
        steps = [e for e in evs if e["event"] == "step"]
        self.assertEqual({e["phase"] for e in steps}, set(T.PHASES))
        for phase in T.PHASES:
            ph = [e for e in steps if e["phase"] == phase]
            self.assertEqual(ph[0]["done"], 0, phase)
            self.assertEqual(ph[-1]["done"], ph[-1]["total"], phase)
            self.assertEqual([e["done"] for e in ph], sorted(e["done"] for e in ph))
            self.assertTrue(all(e["total"] == ph[0]["total"] for e in ph))
        by = {p: [e for e in steps if e["phase"] == p][-1]["total"] for p in T.PHASES}
        self.assertEqual((by["images"], by["audio"]), (2, 2))
        with zipfile.ZipFile(out) as z:
            self.assertEqual(by["zip"], len(z.namelist()))
        # les phases scripts puis zip viennent apres images/audio
        idx = {p: max(i for i, e in enumerate(evs) if e.get("phase") == p) for p in T.PHASES}
        self.assertLess(idx["images"], idx["scripts"])
        self.assertLess(idx["scripts"], min(i for i, e in enumerate(evs) if e.get("phase") == "zip"))

    def test_empty_phases_still_announced(self):
        nodes = {"startAction": {"action": "a0", "index": 0},
                 "stages": {"s0": {"control": {"ok": True, "home": False, "autoplay": True}}},
                 "actions": {"a0": [{"stage": "s0"}]}}
        evs, _ = self.events([("metadata.json", '{"title": "T", "uuid": "u"}'),
                              ("nodes.json", json.dumps(nodes))])
        steps = [e for e in evs if e["event"] == "step" and e["phase"] in ("images", "audio")]
        self.assertEqual([(e["phase"], e["done"], e["total"]) for e in steps],
                         [("images", 0, 0), ("audio", 0, 0)])

    def test_step_throttling_keeps_first_and_last(self):
        T.PROGRESS_INTERVAL = 3600
        evs, _ = self.events(std_files())
        z = [e for e in evs if e.get("phase") == "zip"]
        self.assertEqual(z[0]["done"], 0)
        self.assertEqual(z[-1]["done"], z[-1]["total"])
        self.assertLess(len(z), z[-1]["total"])

    def test_warning_event_and_stderr_contract(self):
        files = [(n, d) for n, d in std_files() if n != "audios/s0.mp3"]
        zp = self.mkzip(files)
        evs, err = [], io.StringIO()
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(err):
            T.convert(zp, os.path.join(self.tmp, "o.plain.pk"), allow_missing=True,
                      progress=evs.append)
        warns = [e for e in evs if e["event"] == "warning"]
        codes = [w["code"] for w in warns]
        self.assertIn("audio_missing", codes)
        self.assertIn("assets_missing_allowed", codes)
        self.assertTrue(all(isinstance(w["message"], str) and w["message"] for w in warns))
        self.assertEqual(evs[-1]["warnings"], len(warns))
        # contrat de compat.py : lignes de stderr commencant par "!"
        bangs = [l for l in err.getvalue().splitlines() if l.strip().startswith("!")]
        self.assertEqual(len(bangs), len(warns))
        self.assertIn("audio manquant: s0.mp3", warns[0]["message"] + " " + warns[1]["message"])

    def test_warning_codes_other(self):
        evs, _ = self.events(std_files(meta={"title": "T"}) + [("notes.json", "[]")])
        codes = {e["code"] for e in evs if e["event"] == "warning"}
        self.assertEqual(codes, {"uuid_derived", "notes_ignored"})

    def test_no_callback_no_events_and_default_output_on_stdout(self):
        zp = self.mkzip(std_files())
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            T.convert(zp, os.path.join(self.tmp, "o.plain.pk"))
        self.assertIn("OK ->", out.getvalue())
        self.assertIsNone(T._sink)

    def test_sink_restored_after_error(self):
        with self.assertRaises(T.ConvError):
            self.events(std_files(nodes=False))
        self.assertIsNone(T._sink)


class TestCliProgress(CliBase):
    def test_stdout_is_only_json_lines(self):
        out = os.path.join(self.tmp, "o.plain.pk")
        r = self.cli(self.mkzip(std_files()), "-o", out, "--progress-json")
        self.assertEqual(r.returncode, 0, r.stderr)
        evs = parse_lines(r.stdout)
        self.assertEqual(r.stdout.count("\n"), len(evs))     # une ligne par evenement
        self.assertEqual(evs[0]["event"], "start")
        self.assertEqual(evs[-1]["event"], "done")
        self.assertEqual(evs[-1]["size"], os.path.getsize(out))
        self.assertIn("OK ->", r.stderr)                      # messages humains sur stderr
        self.assertNotIn("OK ->", r.stdout)

    def test_default_mode_unchanged(self):
        r = self.cli(self.mkzip(std_files()), "-o", os.path.join(self.tmp, "o.plain.pk"))
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("OK ->", r.stdout)
        self.assertNotIn('"event"', r.stdout)

    def test_warning_line_on_stderr_and_event_on_stdout(self):
        files = [(n, d) for n, d in std_files() if n != "images/s0.png"]
        r = self.cli(self.mkzip(files), "-o", os.path.join(self.tmp, "o.plain.pk"),
                     "--allow-missing", "--progress-json")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertTrue(any(l.startswith("  !") for l in r.stderr.splitlines()))
        self.assertIn("image_missing", [e.get("code") for e in parse_lines(r.stdout)])

    def test_plain_dir_events_and_no_leftover(self):
        out = os.path.join(self.tmp, "x.plain")
        r = self.cli(self.mkzip(std_files()), "-o", out, "--progress-json")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertTrue(os.path.isfile(os.path.join(out, "info.plain")))
        done = parse_lines(r.stdout)[-1]
        self.assertEqual(done["event"], "done")
        self.assertGreater(done["size"], 0)
        self.assertEqual(self.leftovers(), [])

    def test_plain_flag_writes_both(self):
        out = os.path.join(self.tmp, "o.plain.pk")
        r = self.cli(self.mkzip(std_files()), "-o", out, "--plain")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertTrue(os.path.isfile(out))
        self.assertTrue(os.path.isfile(os.path.join(self.tmp, "o.plain", "uuid.bin")))
        self.assertEqual(self.leftovers(), [])

    def test_existing_plain_dir_keeps_other_files(self):
        out = os.path.join(self.tmp, "x.plain")
        os.makedirs(out)
        with open(os.path.join(out, "garde.txt"), "w") as f:
            f.write("a conserver")
        r = self.cli(self.mkzip(std_files()), "-o", out)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertTrue(os.path.isfile(os.path.join(out, "garde.txt")))
        self.assertTrue(os.path.isfile(os.path.join(out, "info.plain")))
        self.assertEqual(self.leftovers(), [])


class TestExitCodes(CliBase):
    def err_event(self, r):
        evs = parse_lines(r.stdout)
        self.assertEqual(len(evs), 1, r.stdout)
        self.assertEqual(evs[0]["event"], "error")
        self.assertEqual(evs[0]["exit"], r.returncode)
        self.assertTrue(evs[0]["message"])
        return evs[0]

    def test_0_ok(self):
        r = self.cli(self.mkzip(std_files()), "-o", os.path.join(self.tmp, "o.plain.pk"))
        self.assertEqual(r.returncode, 0)

    def test_2_missing_argument(self):
        r = self.cli()
        self.assertEqual(r.returncode, 2)
        self.assertEqual(r.stdout, "")

    def test_2_bad_option_with_progress_json(self):
        r = self.cli("x.zip", "--selector", "nope", "--progress-json")
        self.assertEqual(r.returncode, 2)
        self.assertEqual(self.err_event(r)["code"], "bad_arguments")
        self.assertIn("usage", r.stderr.lower())

    def test_3_not_a_zip(self):
        bad = os.path.join(self.tmp, "bad.zip")
        with open(bad, "wb") as f:
            f.write(b"pas un zip")
        r = self.cli(bad, "-o", os.path.join(self.tmp, "o.plain.pk"), "--progress-json")
        self.assertEqual(r.returncode, 3)
        self.assertEqual(self.err_event(r)["code"], "archive_unreadable")
        self.assertIn("ERREUR", r.stderr)
        self.assertNotIn("Traceback", r.stderr)
        self.assertFalse(os.path.exists(os.path.join(self.tmp, "o.plain.pk")))

    def test_3_missing_file(self):
        r = self.cli(os.path.join(self.tmp, "absent.zip"), "--progress-json")
        self.assertEqual(r.returncode, 3)
        self.assertEqual(self.err_event(r)["code"], "archive_unreadable")

    def test_3_not_telmi(self):
        zp = self.mkzip([("readme.txt", "rien")])
        r = self.cli(zp, "-o", os.path.join(self.tmp, "o.plain.pk"), "--progress-json")
        self.assertEqual(r.returncode, 3)
        self.assertEqual(self.err_event(r)["code"], "not_telmi")

    def test_4_invalid_content(self):
        files = [("metadata.json", "{pas du json"), ("nodes.json", "{}")]
        r = self.cli(self.mkzip(files), "-o", os.path.join(self.tmp, "o.plain.pk"),
                     "--progress-json")
        self.assertEqual(r.returncode, 4)
        self.assertEqual(self.err_event(r)["code"], "invalid_content")

    def test_4_wrong_type(self):
        nodes = base_nodes()
        nodes["stages"] = []
        r = self.cli(self.mkzip(std_files(nodes=nodes)), "-o", os.path.join(self.tmp, "o"))
        self.assertEqual(r.returncode, 4)
        self.assertIn("ERREUR", r.stderr)

    def test_5_missing_asset(self):
        files = [(n, d) for n, d in std_files() if n != "audios/s0.mp3"]
        r = self.cli(self.mkzip(files), "-o", os.path.join(self.tmp, "o.plain.pk"),
                     "--progress-json")
        self.assertEqual(r.returncode, 5)
        ev = parse_lines(r.stdout)[-1]
        self.assertEqual((ev["event"], ev["code"], ev["exit"]), ("error", "missing_asset", 5))
        self.assertFalse(os.path.exists(os.path.join(self.tmp, "o.plain.pk")))
        self.assertEqual(self.leftovers(), [])

    def test_6_output_error(self):
        blocker = os.path.join(self.tmp, "fichier")
        with open(blocker, "w") as f:
            f.write("x")
        r = self.cli(self.mkzip(std_files()), "-o", os.path.join(blocker, "sous", "o.plain.pk"),
                     "--progress-json")
        self.assertEqual(r.returncode, 6)
        self.assertEqual(self.err_event_last(r)["code"], "output_error")

    def err_event_last(self, r):
        ev = parse_lines(r.stdout)[-1]
        self.assertEqual(ev["event"], "error")
        self.assertEqual(ev["exit"], r.returncode)
        return ev

    def test_6_internal_error(self):
        orig = T.convert

        def boom(*a, **k):
            raise RuntimeError("bug simule")
        T.convert = boom
        out, err = io.StringIO(), io.StringIO()
        try:
            with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
                rc = T.main([self.mkzip(std_files()), "--progress-json"])
        finally:
            T.convert = orig
        self.assertEqual(rc, 6)
        ev = parse_lines(out.getvalue())[-1]
        self.assertEqual(ev["code"], "internal_error")
        self.assertIn("bug simule", ev["message"])
        self.assertIn("ERREUR inattendue", err.getvalue())

    def test_exit_constants(self):
        self.assertEqual((T.EXIT_OK, T.EXIT_USAGE, T.EXIT_ARCHIVE, T.EXIT_CONTENT, T.EXIT_ASSET,
                          T.EXIT_INTERNAL, T.EXIT_CANCELLED), (0, 2, 3, 4, 5, 6, 130))

    def test_converror_defaults(self):
        e = T.ConvError("x")
        self.assertEqual((e.code, e.exit_code), ("invalid_content", 4))


class TestCancel(CliBase):
    def run_cancelled(self, phase, out_name="o.plain.pk", extra=()):
        """Leve KeyboardInterrupt au premier evenement `phase` ; renvoie (code, stdout)."""
        real = T._print_event

        def cancel_on(event):
            real(event)
            if event.get("event") == "step" and event.get("phase") == phase and event["done"] >= 1:
                raise KeyboardInterrupt()
        T._print_event = cancel_on
        saved = T.PROGRESS_INTERVAL
        T.PROGRESS_INTERVAL = 0
        out, err = io.StringIO(), io.StringIO()
        try:
            with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
                rc = T.main([self.mkzip(std_files()), "-o", os.path.join(self.tmp, out_name),
                             "--progress-json"] + list(extra))
        finally:
            T._print_event = real
            T.PROGRESS_INTERVAL = saved
        return rc, out.getvalue(), err.getvalue()

    def test_cancel_during_zip_leaves_nothing(self):
        rc, out, err = self.run_cancelled("zip")
        self.assertEqual(rc, 130)
        ev = parse_lines(out)[-1]
        self.assertEqual((ev["event"], ev["code"], ev["exit"]), ("error", "cancelled", 130))
        self.assertEqual(self.others(), [])
        self.assertNotIn("Traceback", err)

    def test_cancel_during_conversion_leaves_nothing(self):
        rc, _out, _err = self.run_cancelled("scripts")
        self.assertEqual(rc, 130)
        self.assertEqual(self.others(), [])

    def test_cancel_plain_dir_leaves_nothing(self):
        rc, _out, _err = self.run_cancelled("zip", out_name="x.plain")
        self.assertEqual(rc, 130)
        self.assertEqual(self.others(), [])

    def test_cancel_with_plain_flag_leaves_nothing(self):
        rc, _out, _err = self.run_cancelled("zip", extra=("--plain",))
        self.assertEqual(rc, 130)
        self.assertEqual(self.others(), [])

    def test_cancel_keeps_previous_output_intact(self):
        dest = os.path.join(self.tmp, "o.plain.pk")
        with open(dest, "wb") as f:
            f.write(b"ancienne sortie")
        rc, _out, _err = self.run_cancelled("zip")
        self.assertEqual(rc, 130)
        with open(dest, "rb") as f:
            self.assertEqual(f.read(), b"ancienne sortie")
        self.assertEqual(self.others(), ["o.plain.pk"])

    def test_cancel_function_api_cleans_temp(self):
        def cb(event):
            if event.get("phase") == "zip" and event.get("done", 0) >= 1:
                raise KeyboardInterrupt()
        saved = T.PROGRESS_INTERVAL
        T.PROGRESS_INTERVAL = 0
        try:
            with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                with self.assertRaises(KeyboardInterrupt):
                    T.convert(self.mkzip(std_files()), os.path.join(self.tmp, "o.plain.pk"),
                              progress=cb)
        finally:
            T.PROGRESS_INTERVAL = saved
        self.assertEqual(self.others(), [])
        self.assertIsNone(T._sink)

    def test_cancel_without_progress_json_is_human_only(self):
        real = T.convert

        def interrupted(*a, **k):
            raise KeyboardInterrupt()
        T.convert = interrupted
        out, err = io.StringIO(), io.StringIO()
        try:
            with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
                rc = T.main([self.mkzip(std_files())])
        finally:
            T.convert = real
        self.assertEqual(rc, 130)
        self.assertEqual(out.getvalue(), "")
        self.assertIn("Annule", err.getvalue())

    def test_sigterm_handler_raises_keyboard_interrupt(self):
        sig = getattr(signal, "SIGTERM")
        saved = signal.getsignal(sig)
        try:
            T._install_cancel_handlers()
            handler = signal.getsignal(sig)
            self.assertTrue(callable(handler))
            with self.assertRaises(KeyboardInterrupt):
                handler(sig, None)
        finally:
            signal.signal(sig, saved)

    @unittest.skipIf(os.name == "nt", "SIGTERM reel : POSIX seulement")
    def test_real_sigterm_subprocess(self):
        # processus enfant bloque dans un rappel de progression, puis termine par SIGTERM
        script = os.path.join(self.tmp, "run.py")
        out = os.path.join(self.tmp, "o.plain.pk")
        with open(script, "w") as f:
            f.write("import sys, time\nsys.path.insert(0, %r)\nimport telmi2flam as T\n"
                    "T.PROGRESS_INTERVAL = 0\n"
                    "real = T._print_event\n"
                    "def slow(e):\n    real(e)\n"
                    "    if e.get('phase') == 'zip' and e['done'] >= 1:\n"
                    "        time.sleep(30)\n"
                    "T._print_event = slow\n"
                    "sys.exit(T.main([%r, '-o', %r, '--progress-json']))\n"
                    % (TOOL, self.mkzip(std_files()), out))
        p = subprocess.Popen([sys.executable, script], stdout=subprocess.PIPE, text=True)
        for line in p.stdout:
            e = json.loads(line)
            if e.get("phase") == "zip" and e["done"] >= 1:
                break
        p.send_signal(signal.SIGTERM)
        rest = p.stdout.read()
        self.assertEqual(p.wait(timeout=20), 130)
        self.assertEqual(parse_lines(rest)[-1]["code"], "cancelled")
        self.assertFalse(os.path.exists(out))
        self.assertEqual(self.leftovers(), [])


class TestInfo(CliBase):
    META = {"title": "Mon histoire", "uuid": "abc-123", "version": 3, "age": 6,
            "description": "Une description", "category": "Aventure", "image": "cover.png"}

    def test_info_function(self):
        files = std_files(meta=self.META) + [("cover.png", b"x")]
        info = T.read_info(self.mkzip(files))
        self.assertEqual(info, {"title": "Mon histoire", "uuid": "abc-123", "version": 3,
                                "age": 6, "category": "Aventure",
                                "description": "Une description", "cover": True})

    def test_info_without_optional_fields(self):
        info = T.read_info(self.mkzip(std_files(meta={"title": "T"})))
        self.assertIsNone(info["uuid"])
        self.assertIsNone(info["description"])
        self.assertIsNone(info["age"])
        self.assertTrue(info["cover"])           # title.png present dans std_files

    def test_info_no_cover(self):
        files = [(n, d) for n, d in std_files() if n != "title.png"]
        self.assertFalse(T.read_info(self.mkzip(files))["cover"])

    def test_info_cover_in_subfolder_root_and_case(self):
        files = [("histoire/" + n, d) for n, d in std_files(meta=self.META)]
        files.append(("histoire/COVER.PNG", b"x"))
        self.assertTrue(T.read_info(self.mkzip(files))["cover"])

    def test_info_ignores_non_scalar_values(self):
        info = T.read_info(self.mkzip(std_files(meta={"title": ["x"], "age": True, "uuid": {}})))
        self.assertIsNone(info["title"])
        self.assertIsNone(info["age"])
        self.assertIsNone(info["uuid"])

    def test_info_cli_json_and_no_conversion(self):
        out = os.path.join(self.tmp, "o.plain.pk")
        r = self.cli("--info", self.mkzip(std_files(meta=self.META)), "-o", out)
        self.assertEqual(r.returncode, 0, r.stderr)
        lines = r.stdout.splitlines()
        self.assertEqual(len(lines), 1)
        self.assertEqual(json.loads(lines[0])["title"], "Mon histoire")
        self.assertFalse(os.path.exists(out))
        self.assertEqual(r.stderr, "")

    def test_info_non_ascii_is_escaped(self):
        meta = {"title": "Le chat élégant ☃", "uuid": "u"}
        r = self.cli("--info", self.mkzip(std_files(meta=meta)))
        self.assertTrue(r.stdout.isascii())
        self.assertEqual(json.loads(r.stdout)["title"], meta["title"])

    def test_info_errors_use_exit_codes(self):
        bad = os.path.join(self.tmp, "bad.zip")
        with open(bad, "wb") as f:
            f.write(b"non")
        self.assertEqual(self.cli("--info", bad).returncode, 3)
        self.assertEqual(self.cli("--info", self.mkzip([("a.txt", "x")], "n")).returncode, 3)
        r = self.cli("--info", self.mkzip([("metadata.json", "{x"), ("nodes.json", "{}")], "j"),
                     "--progress-json")
        self.assertEqual(r.returncode, 4)
        self.assertEqual(parse_lines(r.stdout)[-1]["code"], "invalid_content")


if __name__ == "__main__":
    unittest.main()
