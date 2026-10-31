"""Tests of scripts/coverage.py name normalization and of the mutant list of
scripts/mutants.py (contract 4.6 and 6)."""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "scripts"))
import coverage  # noqa: E402
import mutants  # noqa: E402

SCOPE = {"cmpps": None, "cmpss": None, "cmppd": None, "cmpsd": None, "cmps": None}


class Normalize(unittest.TestCase):
    def test_compare_predicate_aliases(self):
        for text, mn in [("cmpeqps xmm1,xmm2", "cmpps"), ("cmpunordss xmm0,xmm3", "cmpss"),
                         ("cmpnlepd xmm4,xmmword ptr [rax]", "cmppd"),
                         ("cmpltsd xmm1,qword ptr [rbx]", "cmpsd"), ("cmpps xmm1,xmm2,0x7", "cmpps")]:
            self.assertEqual(coverage.normalize(text, SCOPE), mn, text)

    def test_string_compare_untouched(self):
        self.assertEqual(coverage.normalize("cmps byte ptr ds:[rsi],byte ptr es:[rdi]", SCOPE),
                         "cmps")


class Mutants(unittest.TestCase):
    def test_header_is_not_a_mutant(self):
        features = [feature for feature, _ in mutants.load()]
        self.assertEqual(features, ["mutant-adc-carry", "mutant-shift-flags", "mutant-movsxd",
                                    "mutant-riprel-disp", "mutant-pshufd-imm", "mutant-fadd-fsub"])


if __name__ == "__main__":
    unittest.main()
