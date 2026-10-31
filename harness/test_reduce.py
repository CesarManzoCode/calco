"""Tests of the reducer: ddmin reaches the minimum of a synthetic predicate, the
preparation of `div`/`idiv` stays with its instruction, immediates go to 0 only in the
instruction line, and the flags of a sublist are recomputed (with the shift refinement)."""

import unittest

import gen
import reduce

ROWS = gen.load_scope()


def div_case():
    """First generated case with a `div` or `idiv` item and something else."""
    for seed in range(5000):
        case = gen.generate(seed)
        if len(case.items) > 2 and any(i.mnemonic in ("div", "idiv") for i in case.items):
            return case
    raise AssertionError("no case with div in 5000 seeds")


class Ddmin(unittest.TestCase):
    def test_reaches_minimum(self):
        calls = []

        def test(items):
            calls.append(list(items))
            return {3, 11, 17} <= set(items)

        self.assertEqual(reduce.ddmin(range(20), test), [3, 11, 17])
        self.assertTrue(calls)

    def test_single_element_order_and_already_minimal(self):
        self.assertEqual(reduce.ddmin(range(37), lambda s: 29 in s), [29])
        need = {0, 5, 6, 30}
        self.assertEqual(reduce.ddmin(list(range(31)), lambda s: need <= set(s)), sorted(need))
        self.assertEqual(reduce.ddmin([7], lambda s: 7 in s), [7])


class DivPreparation(unittest.TestCase):
    def test_preparation_not_separated(self):
        case = div_case()
        div = next(i for i in case.items if i.mnemonic in ("div", "idiv"))
        out = reduce.ddmin(case.items, lambda items: any(i.id == div.id for i in items))
        self.assertEqual(len(out), 1)
        self.assertEqual(out[0].lines, div.lines)
        # Divisor and high half (or sign extension) are set right before the division.
        self.assertGreaterEqual(len(out[0].lines), 3)
        self.assertTrue(out[0].lines[-3].startswith("mov "))

    def test_preparation_never_simplified(self):
        case = div_case()
        div = next(i for i in case.items if i.mnemonic in ("div", "idiv"))
        self.assertEqual(reduce.immediates(div), [])
        self.assertEqual(reduce.simplify([div], lambda items: True), [div])


class Immediates(unittest.TestCase):
    def test_to_zero_in_instruction_line(self):
        item = gen.Item(0, "add", ["lea rbx, [rip + buf+16]", "add dword ptr [rbx+4], -7"])
        out = reduce.simplify([item], lambda items: True)
        self.assertEqual(out[0].lines, ["lea rbx, [rip + buf+16]", "add dword ptr [rbx+4], 0"])

    def test_only_while_failing(self):
        item = gen.Item(0, "shl", ["shl eax, 5"])
        out = reduce.simplify([item], lambda items: items[0].lines[-1] != "shl eax, 0")
        self.assertEqual(out[0].lines, ["shl eax, 5"])

    def test_labels_and_registers_untouched(self):
        self.assertEqual(reduce.immediates(gen.Item(0, "call", ["call 1f", "1: pop rax"])), [])
        self.assertEqual(reduce.immediates(gen.Item(0, "mov", ["mov rax, rbx"])), [])
        self.assertEqual(reduce.immediates(gen.Item(0, "mov", ["mov rax, 0x10"])), [1])


class Flags(unittest.TestCase):
    def test_sublist_reading_undefined_flag_is_invalid(self):
        setb = gen.Item(1, "setb", ["setb al"])
        by_cl = gen.Item(0, "shl", ["shl eax, cl"])
        by_3 = gen.Item(0, "shl", ["shl eax, 3"])
        self.assertFalse(gen.sequence_flags([by_cl, setb], ROWS)[0])
        self.assertEqual(gen.sequence_flags([by_3, setb], ROWS), (True, frozenset({"AF", "OF"})))

    def test_shift_effect(self):
        row = ROWS["shl"]
        one = gen.effect(row, gen.Item(0, "shl", ["shl byte ptr [rip + buf + 3], 1"]))
        self.assertEqual(one, (frozenset({"CF", "PF", "ZF", "SF", "OF"}), frozenset({"AF"})))
        # 8-bit shift by 8, and 64-bit shift by 64 (masked to 0): the table's flags.
        for line in ("shl al, 8", "shl rax, 64", "shl eax, 0"):
            self.assertEqual(gen.effect(row, gen.Item(0, "shl", [line])),
                             (row.written, row.undefined), line)

    def test_missing_jump_target_joins_next_item(self):
        # The jump carries the CF left undefined by `shl ..., cl` past the `add` that
        # defines it; its target (3) is gone, so it lands on item 4, which reads CF.
        items = [gen.Item(0, "shl", ["shl eax, cl"]), gen.Item(1, "je", ["je {T}"], target=3),
                 gen.Item(2, "add", ["add eax, ebx"]), gen.Item(4, "setb", ["setb al"])]
        self.assertFalse(gen.sequence_flags(items, ROWS)[0])
        self.assertTrue(gen.sequence_flags(items[:3], ROWS)[0])


if __name__ == "__main__":
    unittest.main()
