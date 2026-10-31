"""Tests of scripts/qemu_tcg.py: the testthread comparison of D22 on fixed outputs and the
gcc line of each QEMU test."""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "scripts"))
import common  # noqa: E402
import qemu_tcg  # noqa: E402

NATIVE = (b"thread1: 0 hello1\n"
          b"thread2: 0 hello2\n"
          b"thread1: 1 hello1\n"
          b"thread2: 1 hello2\n"
          b"thread2: 2 hello2\n"
          b"End of pthread test.\n")


class Threads(unittest.TestCase):
    def test_same_output(self):
        self.assertEqual(qemu_tcg.compare_threads(NATIVE, NATIVE), [])

    def test_other_interleaving_passes(self):
        other = (b"thread2: 0 hello2\n"
                 b"thread2: 1 hello2\n"
                 b"thread1: 0 hello1\n"
                 b"thread2: 2 hello2\n"
                 b"thread1: 1 hello1\n"
                 b"End of pthread test.\n")
        self.assertEqual(qemu_tcg.compare_threads(NATIVE, other), [])

    def test_extra_line_fails(self):
        self.assertNotEqual(qemu_tcg.compare_threads(NATIVE, NATIVE + b"thread1: 1 hello1\n"), [])
        self.assertNotEqual(qemu_tcg.compare_threads(NATIVE, NATIVE + b"extra\n"), [])

    def test_missing_line_fails(self):
        self.assertNotEqual(qemu_tcg.compare_threads(NATIVE, NATIVE.replace(b"thread2: 2 hello2\n", b"")), [])

    def test_order_within_a_thread_fails(self):
        swapped = (b"thread1: 1 hello1\n"
                   b"thread2: 0 hello2\n"
                   b"thread1: 0 hello1\n"
                   b"thread2: 1 hello2\n"
                   b"thread2: 2 hello2\n"
                   b"End of pthread test.\n")
        problems = qemu_tcg.compare_threads(NATIVE, swapped)
        self.assertEqual(len(problems), 1)
        self.assertTrue(problems[0].startswith("thread1:"))

    def test_grouping(self):
        groups = qemu_tcg.by_thread(NATIVE)
        self.assertEqual(sorted(groups, key=str), [None, "thread1", "thread2"])
        self.assertEqual(groups["thread2"], [b"thread2: 0 hello2\n", b"thread2: 1 hello2\n",
                                             b"thread2: 2 hello2\n"])
        self.assertEqual(groups[None], [b"End of pthread test.\n"])


class Compare(unittest.TestCase):
    def test_exit_code_and_stderr_count(self):
        ok = common.Result(NATIVE, b"", 0)
        self.assertEqual(qemu_tcg.compare("testthread", ok, ok), [])
        self.assertEqual(len(qemu_tcg.compare("testthread", ok, common.Result(NATIVE, b"", -11))), 1)
        self.assertEqual(len(qemu_tcg.compare("testthread", ok, common.Result(NATIVE, b"x", 0))), 1)

    def test_other_tests_are_byte_exact(self):
        a = common.Result(b"a\nb\n", b"", 0)
        b = common.Result(b"b\na\n", b"", 0)
        self.assertEqual(qemu_tcg.compare("test-x86_64", a, a), [])
        self.assertEqual(qemu_tcg.compare("test-x86_64", a, b), ["stdout differs from line 1"])


class CompileCommand(unittest.TestCase):
    SRC = Path("/src")
    OUT = Path("/out/x")

    def cmd(self, name):
        return qemu_tcg.compile_command(name, self.SRC, self.OUT)

    def test_each_test(self):
        flags = ["gcc", "-Wall", "-O0", "-g", "-fno-strict-aliasing"]
        for name, want in (
                ("test-x86_64", [*flags, "/src/i386/test-i386.c", "-o", "/out/x", "-static",
                                 "-lm", "-lc"]),
                ("sha512-sse", ["gcc", "-msse4.1", "-O3", "/src/multiarch/sha512.c", "-o",
                                "/out/x", "-static"]),
                ("linux-test", [*flags, "/src/multiarch/linux/linux-test.c", "-o", "/out/x",
                                "-static", "-lpthread"]),
                ("testthread", [*flags, "/src/multiarch/testthread.c", "-o", "/out/x",
                                "-static", "-lpthread"])):
            self.assertEqual(self.cmd(name), want, name)

    def test_never_werror(self):
        for name in qemu_tcg.TESTS:
            self.assertNotIn("-Werror", self.cmd(name))


if __name__ == "__main__":
    unittest.main()
