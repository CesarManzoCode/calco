"""Tests of the comparison: mask, strict mode and fields left out."""

import dataclasses
import unittest

import compare
import gen
from test_dump import sample


def with_flags(rec, *flags):
    return dataclasses.replace(rec, rflags=rec.rflags ^ gen.flag_mask(flags))


def with_fx(rec, start, end):
    fx = bytearray(rec.fxsave)
    fx[start:end] = bytes(b ^ 0xFF for b in fx[start:end])
    return dataclasses.replace(rec, fxsave=bytes(fx))


class Compare(unittest.TestCase):
    def setUp(self):
        self.rec = sample()

    def test_equal(self):
        self.assertEqual(compare.compare(self.rec, self.rec), [])

    def test_undefined_flag_altered_is_not_a_discrepancy(self):
        for flag in gen.FLAGS:
            self.assertEqual(compare.compare(self.rec, with_flags(self.rec, flag), {flag}), [])

    def test_defined_flag_altered_is_a_discrepancy(self):
        for flag in gen.FLAGS:
            other = next(f for f in gen.FLAGS if f != flag)
            got = compare.compare(self.rec, with_flags(self.rec, flag), {other})
            self.assertEqual([f for f, _, _ in got], ["rflags"], flag)

    def test_strict_compares_undefined_flags(self):
        altered = with_flags(self.rec, "AF")
        self.assertEqual([f for f, _, _ in compare.compare(self.rec, altered, {"AF"}, True)],
                         ["rflags"])
        self.assertEqual(compare.undefined_matches(self.rec, altered, {"AF", "OF"}), 1)

    def test_rf_and_reserved_bits_ignored(self):
        for bit in (1, 3, 5, 15, 16, 17, 22, 40):
            altered = dataclasses.replace(self.rec, rflags=self.rec.rflags ^ 1 << bit)
            self.assertEqual(compare.compare(self.rec, altered, strict=True), [], bit)
        for bit in (8, 9, 10, 18, 21):
            altered = dataclasses.replace(self.rec, rflags=self.rec.rflags ^ 1 << bit)
            self.assertNotEqual(compare.compare(self.rec, altered), [], bit)

    def test_fip_fdp_fop_and_mxcsr_mask_ignored(self):
        # FOP 6–7, FIP+FCS 8–15, FDP+FDS 16–23, MXCSR_MASK 28–31, reserved bytes of ST0.
        for start, end in ((6, 8), (8, 16), (16, 24), (28, 32), (42, 48)):
            self.assertEqual(compare.compare(self.rec, with_fx(self.rec, start, end)), [],
                             (start, end))

    def test_compared_fx_fields(self):
        for (start, end), field in (((0, 2), "fcw"), ((2, 4), "fsw"), ((4, 5), "ftw"),
                                    ((24, 28), "mxcsr"), ((32 + 16 * 7, 33 + 16 * 7), "st7"),
                                    ((160 + 16 * 3, 161 + 16 * 3), "xmm3")):
            got = compare.compare(self.rec, with_fx(self.rec, start, end))
            self.assertEqual([f for f, _, _ in got], [field])

    def test_registers_memory_and_stack(self):
        gpr = list(self.rec.gpr)
        gpr[9] ^= 1
        got = compare.compare(self.rec, dataclasses.replace(self.rec, gpr=gpr))
        self.assertEqual([f for f, _, _ in got], ["r9"])
        for name in ("buf", "stack"):
            blob = bytearray(getattr(self.rec, name))
            blob[100] ^= 0x10
            got = compare.compare(self.rec, dataclasses.replace(self.rec, **{name: bytes(blob)}))
            self.assertEqual([f for f, _, _ in got], [name])


if __name__ == "__main__":
    unittest.main()
