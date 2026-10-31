"""Tests of the target programs' scripts: reading targets/busybox/cases.tsv (columns, that
each input exists), counting `PASS:` on a fixed runtest log, the valid subset as the
intersection of two native runs, and the busybox .config edits of build_targets.py."""

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "scripts"))
import build_targets  # noqa: E402
import busybox_run  # noqa: E402
import busybox_suite  # noqa: E402

# Applets that write or remove files; no case may use them.
WRITERS = {"cp", "mv", "rm", "touch", "mkdir", "rmdir", "ln", "dd", "install", "tee",
           "truncate", "chmod", "chown", "split", "patch", "mknod", "mkfifo"}
# Applets whose output depends on the time, the PID, the user or the network.
NONDET = {"ps", "top", "uptime", "id", "whoami", "who", "hostname", "ping", "wget",
          "nc", "shuf", "mktemp", "free", "env", "printenv", "ls", "stat",
          "df", "du", "find", "nslookup", "random"}


class Cases(unittest.TestCase):
    def setUp(self):
        # read_cases fails on a line without applet or with a stdin that is not in inputs/.
        self.cases = busybox_run.read_cases()

    def test_at_least_25_applets(self):
        self.assertGreaterEqual(len({c.applet for c in self.cases}), 25)

    def test_text_binary_and_compressed_inputs(self):
        used = {c.stdin for c in self.cases} | {a for c in self.cases for a in c.args}
        inputs = {p.name for p in busybox_run.INPUTS.iterdir()}
        self.assertIn("text.txt", used & inputs)
        self.assertIn("bin.dat", used & inputs)
        for suffix in (".gz", ".bz2", ".xz"):
            self.assertTrue(any(n.endswith(suffix) for n in used & inputs), suffix)

    def test_no_writer_or_nondeterministic_applet(self):
        for case in self.cases:
            self.assertNotIn(case.applet, WRITERS | NONDET, case.describe())


class ParseCases(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.inputs = Path(self.dir.name)
        (self.inputs / "in.txt").write_text("x\n")

    def tearDown(self):
        self.dir.cleanup()

    def test_columns(self):
        text = "# comment\n\n-\techo\ta b\t\tc\nin.txt\tcat\n"
        cases = busybox_run.parse_cases(text, self.inputs)
        self.assertEqual(len(cases), 2)
        self.assertEqual((cases[0].line, cases[0].stdin, cases[0].applet, cases[0].args),
                         (3, None, "echo", ("a b", "", "c")))
        self.assertEqual((cases[1].line, cases[1].stdin, cases[1].applet, cases[1].args),
                         (4, "in.txt", "cat", ()))

    def test_missing_applet_fails(self):
        for text in ("-\n", "-\t\targ\n", "in.txt\n"):
            with self.assertRaises(ValueError):
                busybox_run.parse_cases(text, self.inputs)

    def test_missing_input_fails(self):
        with self.assertRaises(ValueError):
            busybox_run.parse_cases("nope.txt\tcat\n", self.inputs)
        with self.assertRaises(ValueError):
            busybox_run.parse_cases("../in.txt\tcat\n", self.inputs)


LOG = """\
PASS: cat-prints-a-file
FAIL: cp-preserves-links
UNTESTED: mount
SKIPPED: tc (not built)
PASS: sort
PASS: sort -n
PASS: sort
FAIL: sort -n
PASS: sed n command
FAILED: not a result line
 PASS: indented, not a result line
2 failure(s) detected; running with -v (verbose) will give more info
"""


class Suite(unittest.TestCase):
    def test_results_and_pass_count_on_fixed_log(self):
        got = busybox_suite.results(LOG)
        self.assertEqual(got, {"cat-prints-a-file": (1, 0), "cp-preserves-links": (0, 1),
                               "sort": (2, 0), "sort -n": (1, 1), "sed n command": (1, 0)})
        self.assertEqual(sum(p for p, _ in got.values()), 5)

    def test_subset_is_intersection_of_two_native_runs(self):
        second = ("PASS: cat-prints-a-file\nPASS: cp-preserves-links\nPASS: sort\n"
                  "PASS: sort -n\nFAIL: sed n command\nPASS: only-second\n")
        subset = busybox_suite.valid_subset(LOG, second)
        # sort passed twice in the first run and once in the second; sort -n, sed n
        # command and cp-preserves-links failed in one of the runs.
        self.assertEqual(dict(subset), {"cat-prints-a-file": 1, "sort": 1})
        self.assertEqual(busybox_suite.valid_subset(second, LOG), subset)

    def test_passed_in_counts_only_the_subset(self):
        subset = busybox_suite.valid_subset(LOG, LOG)
        self.assertEqual(dict(subset), {"cat-prints-a-file": 1, "sort": 2,
                                        "sed n command": 1})
        self.assertEqual(busybox_suite.passed_in(LOG, subset), 4)
        calco = "PASS: sort\nFAIL: sort\nPASS: sed n command\nPASS: extra\n"
        self.assertEqual(busybox_suite.passed_in(calco, subset), 2)
        self.assertEqual(busybox_suite.passed_in("", subset), 0)


class BusyboxConfig(unittest.TestCase):
    def test_notes(self):
        text = "# header\n\nCONFIG_TC\twhy it does not build\n"
        self.assertEqual(build_targets.disabled_options(text), ["CONFIG_TC"])
        names = build_targets.disabled_options(build_targets.NOTES.read_text())
        self.assertIn("CONFIG_TC", names)

    def test_set_options(self):
        config = "# Settings\nCONFIG_TC=y\n# CONFIG_STATIC is not set\nCONFIG_CAT=y\n"
        got = build_targets.set_options(config, on=["CONFIG_STATIC"], off=["CONFIG_TC", "CONFIG_X"])
        self.assertEqual(got.splitlines(), [
            "# Settings", "# CONFIG_TC is not set", "CONFIG_STATIC=y", "CONFIG_CAT=y",
            "# CONFIG_X is not set"])


if __name__ == "__main__":
    unittest.main()
