"""Tests of the tables (contract 4.2): the columns of `isa/scope.tsv` are valid, every
mnemonic with undefined flags is in `undefined_flags.tsv` with a reference, and no
exclusion is among the generator's templates or in the cases it emits."""

import re
import sys
import unittest
from pathlib import Path

import gen

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "scripts"))
import mask_check  # noqa: E402
import nondet_check  # noqa: E402

SCOPE_HEADER = ["mnemonic", "ext", "forms", "flags_read", "flags_written", "flags_undefined", "ref"]
EXTS = {"base", "x87", "mmx", "sse", "sse2", "sse3", "ssse3", "sse4.1", "sse4.2"}
FORM = re.compile(r"(-|[bwdq]+):(.*)")


def data_lines(path):
    return [line for line in path.read_text().splitlines() if line and not line.startswith("#")]


class Scope(unittest.TestCase):
    def test_columns(self):
        header, *rows = data_lines(gen.SCOPE)
        self.assertEqual(header.split("\t"), SCOPE_HEADER)
        seen = set()
        for line in rows:
            cols = line.split("\t")
            self.assertEqual(len(cols), len(SCOPE_HEADER), line)
            mn, ext, forms, read, written, undefined, ref = cols
            self.assertRegex(mn, r"^[a-z][a-z0-9]*$", line)
            self.assertNotIn(mn, seen, line)
            seen.add(mn)
            self.assertIn(ext, EXTS, line)
            self.assertTrue(ref.strip(), line)
            for form in forms.split("|"):
                m = FORM.fullmatch(form)
                self.assertTrue(m, (mn, form))
                tokens = m[2].split(",") if m[2] else []
                self.assertLessEqual(set(tokens), gen.TOKENS, (mn, form))
            for flag in read.split() + undefined.split():
                self.assertIn(flag, gen.FLAGS, line)
            for flag in written.split():
                self.assertIn(flag.removeprefix("?"), gen.FLAGS, line)

    def test_every_extension_present(self):
        exts = {row.ext for row in gen.load_scope().values()}
        self.assertEqual(exts, EXTS)

    def test_x87_control_and_environment(self):
        scope = gen.load_scope()
        for mn in ("fldcw fnstcw fnstsw fnstenv fldenv fnsave frstor fninit fnclex ffree "
                   "fdecstp fincstp fxch fxam ftst fcom fucom fcomi fucomi fcmovb fbld fbstp "
                   "fild fistp fisttp fsin fcos fptan fpatan fldpi fldz fld1").split():
            self.assertIn(mn, scope, mn)

    def test_aligned_forms_are_vector_memory(self):
        for row in gen.load_scope().values():
            for _, tokens in row.forms:
                if "xma" in tokens or "m128a" in tokens:
                    self.assertIn(row.ext, EXTS - {"base", "x87"}, row.mnemonic)


class UndefinedFlags(unittest.TestCase):
    def test_columns_revision_and_a_reference_for_every_undefined_mnemonic(self):
        self.assertEqual(data_lines(gen.UNDEFINED_FLAGS)[0].split("\t"),
                         ["mnemonic", "undefined", "ref"])
        # The header names the APM revision; each row has a ref and the flags of the scope.
        self.assertEqual(mask_check.problems(), [])


class Exclusions(unittest.TestCase):
    REQUIRED = set("rdtsc rdtscp rdrand rcpps rcpss rsqrtps rsqrtss cpuid syscall".split())

    def test_columns_and_minimum(self):
        header, *rows = data_lines(gen.EXCLUSIONS)
        self.assertEqual(header.split("\t"), ["mnemonic", "reason"])
        for line in rows:
            mn, reason = line.split("\t")
            self.assertTrue(reason.strip(), line)
        self.assertLessEqual(self.REQUIRED, gen.load_exclusions())

    def test_not_in_templates(self):
        self.assertFalse(gen.load_exclusions() & set(gen.generatable()))

    def test_not_emitted(self):
        exclusions = gen.load_exclusions()
        for seed in range(600):
            for item, word in nondet_check.emitted(gen.generate(seed)):
                self.assertNotIn(item, exclusions, seed)
                self.assertNotIn(word, exclusions, (seed, word))


if __name__ == "__main__":
    unittest.main()
