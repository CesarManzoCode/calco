"""Tests of the generator: determinism, memory inside the buffer, flags read only when
defined, `div`/`idiv` preparation."""

import re
import unittest

import gen

SEEDS = range(600)
ROWS = gen.load_scope()
PTR_BYTES = {"byte": 1, "word": 2, "dword": 4, "qword": 8}
MEM = re.compile(r"(?:(byte|word|dword|qword) ptr )?\[([^\]]*)\]")
SET_REG = re.compile(r"mov (\w+), (-?(?:0x)?[0-9a-f]+)$")
LEA_BUF = re.compile(r"lea (\w+), \[rip \+ buf([+-]\d+)\]$")


def reg64(name):
    for regs in gen.REGS.values():
        if name in regs:
            return gen.R64[regs.index(name)]
    return None


def offset(inner, regs):
    """Buffer offset an address expression of the generator points to."""
    inner = inner.replace(" ", "")
    for head in ("rip+buf", "buf"):
        if inner.startswith(head):
            return int(inner[len(head):] or 0)
    m = re.fullmatch(r"(\w+)(?:\+(\w+)\*(\d))?([+-]\d+)", inner)
    if not m:
        raise AssertionError(f"unexpected address {inner}")
    base, index, scale, disp = m.groups()
    value = regs[base] + int(disp)
    if index:
        value += regs[index] * int(scale)
    return value


class Generator(unittest.TestCase):
    def test_same_seed_same_case(self):
        for seed in range(50):
            self.assertEqual(gen.generate(seed), gen.generate(seed))
        self.assertNotEqual(gen.generate(1).items, gen.generate(2).items)

    def test_lengths(self):
        for seed in SEEDS:
            n = len(gen.generate(seed).items)
            self.assertTrue(1 <= n <= gen.MAX_ITEMS, (seed, n))

    def test_memory_inside_buffer(self):
        for seed in SEEDS:
            case = gen.generate(seed)
            for off, size in case.accesses:
                self.assertTrue(0 <= off and off + size <= gen.BUF_SIZE, (seed, off, size))
            for item in case.items:
                regs = {}
                for line in item.lines:
                    if m := LEA_BUF.match(line):
                        regs[m[1]] = int(m[2])
                        continue
                    if line.startswith("lea "):
                        continue  # labels and frame pointers, not buffer accesses
                    if (m := SET_REG.match(line)) and reg64(m[1]):
                        regs[reg64(m[1])] = int(m[2], 0)
                    for ptr, inner in MEM.findall(line):
                        size = PTR_BYTES.get(ptr, 16 if "cmpxchg16b" in line else 1)
                        off = offset(inner, regs)
                        self.assertTrue(0 <= off and off + size <= gen.BUF_SIZE,
                                        (seed, line, off))
                        if "cmpxchg16b" in line:
                            self.assertEqual(off % 16, 0, (seed, line))

    def test_flags_read_only_when_defined(self):
        exclusions = gen.load_exclusions()
        for seed in SEEDS:
            case = gen.generate(seed)
            undefined, pending = frozenset(), {}
            for item in case.items:
                self.assertNotIn(item.mnemonic, exclusions)
                row = ROWS[item.mnemonic]
                undefined |= pending.pop(item.id, frozenset())
                self.assertFalse(row.read & undefined, (seed, item.mnemonic, undefined))
                undefined = (undefined - row.written) | row.undefined
                if item.target is not None:
                    pending[item.target] = pending.get(item.target, frozenset()) | undefined
            for later in pending.values():
                undefined |= later
            self.assertEqual(undefined, case.undefined, seed)

    def test_some_flags_end_undefined(self):
        self.assertTrue(any(gen.generate(seed).undefined for seed in SEEDS))

    def test_div_preparation(self):
        seen = set()
        for seed in SEEDS:
            for item in gen.generate(seed).items:
                if item.mnemonic not in ("div", "idiv"):
                    continue
                seen.add(item.mnemonic)
                *prep, insn = item.lines
                divisor = insn.split(" ", 1)[1]
                ptr = MEM.match(divisor)
                bits = 8 * PTR_BYTES[ptr[1]] if ptr else next(
                    b for b, regs in gen.REGS.items() if divisor in regs)
                mask = (1 << bits) - 1
                sets = [line for line in prep if line.startswith(f"mov {divisor}, ")]
                self.assertEqual(len(sets), 1, (seed, item.lines))
                v = int(sets[0].rsplit(", ", 1)[1], 0) & mask
                self.assertNotEqual(v, 0, (seed, item.lines))
                if item.mnemonic == "div":
                    high = [line for line in prep if line.startswith(f"mov {gen.HIGH[bits]}, ")]
                    self.assertEqual(len(high), 1, (seed, item.lines))
                    self.assertLess(int(high[0].rsplit(", ", 1)[1], 0), v, (seed, item.lines))
                else:
                    self.assertNotEqual(v, mask, (seed, item.lines))
                    self.assertEqual(prep[-1], gen.SIGN_EXTEND[bits], (seed, item.lines))
                if not ptr:
                    self.assertNotIn(reg64(divisor), ("rax", "rdx"), (seed, item.lines))
        self.assertEqual(seen, {"div", "idiv"})

    def test_no_rsp_destination(self):
        for seed in SEEDS:
            for item in gen.generate(seed).items:
                for line in item.lines:
                    words = line.replace("lock ", "").split(None, 1)
                    if len(words) == 2 and words[0] not in gen.READ_FIRST | {"lea"}:
                        first = words[1].split(",")[0].strip()
                        self.assertNotIn(first, ("rsp", "esp", "sp", "spl"), (seed, line))


if __name__ == "__main__":
    unittest.main()
