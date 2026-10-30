"""Tests of the generator: determinism, memory inside the buffer and aligned where the
form requires it, flags read only when defined, `div`/`idiv` preparation, initial x87
and MXCSR state and the preparation of the x87/MXCSR state instructions."""

import re
import unittest

import gen

SEEDS = range(600)
ROWS = gen.load_scope()
PTR_BYTES = {"byte": 1, "word": 2, "dword": 4, "qword": 8, "tbyte": 10, "xmmword": 16}
# Size of memory operands written without `ptr`.
BARE_BYTES = {"cmpxchg16b": 16, "fnstenv": 28, "fldenv": 28, "fnsave": 108, "frstor": 108}
MEM = re.compile(r"(?:\b(byte|word|dword|qword|tbyte|xmmword) ptr )?\[([^\]]*)\]")
ALIGNED = {mn for mn, row in ROWS.items()
           if any("xma" in t or "m128a" in t for _, t in row.forms)}
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


def addresses(item):
    """(line, size, buffer offset) of every memory operand of an item's lines."""
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
            mnemonic = line.removeprefix("lock ").split()[0]
            size = PTR_BYTES[ptr] if ptr else BARE_BYTES.get(mnemonic, 1)
            yield line, size, offset(inner, regs)


def fx_word(fx, at, size=2):
    return int.from_bytes(fx[at:at + size], "little")


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
                for line, size, off in addresses(item):
                    self.assertTrue(0 <= off and off + size <= gen.BUF_SIZE, (seed, line, off))
                    if "cmpxchg16b" in line:
                        self.assertEqual(off % 16, 0, (seed, line))

    def test_aligned_vector_memory(self):
        """Forms that require alignment (`xma`, `m128a`) get a 16-aligned address; the
        buffer itself is 64-aligned in the test ELF."""
        seen = set()
        for seed in SEEDS:
            for item in gen.generate(seed).items:
                if item.mnemonic not in ALIGNED:
                    continue
                insn = item.lines[-1]
                for line, size, off in addresses(item):
                    if line == insn and size == 16:
                        self.assertEqual(off % 16, 0, (seed, line, off))
                        seen.add(item.mnemonic)
        self.assertGreater(len(seen), 20)

    def test_vector_registers(self):
        lines = [line for seed in SEEDS for item in gen.generate(seed).items for line in item.lines]
        text = "\n".join(lines)
        for reg in ("mm0", "mm7", "xmm0", "xmm15", "st(0)", "st(7)"):
            self.assertIn(reg, text)

    def test_x87_and_mxcsr_initial_state(self):
        loaded_counts, rounding, daz, ftz, precision = set(), set(), set(), set(), set()
        for seed in SEEDS:
            fx = gen.generate(seed).fxsave
            fcw, fsw, mxcsr = fx_word(fx, 0), fx_word(fx, 2), fx_word(fx, 24, 4)
            self.assertEqual(fcw & 0x3F, 0x3F, seed)  # x87 exceptions masked
            self.assertNotEqual(fcw >> 8 & 3, 1, seed)  # reserved precision control
            self.assertEqual(fsw & 0xC7FF, 0, seed)  # no exception or condition flags
            self.assertEqual(mxcsr & 0x1F80, 0x1F80, seed)  # SSE exceptions masked
            self.assertEqual(mxcsr & ~0xFFC0, 0, seed)  # no exception flags or reserved bits
            precision.add(fcw >> 8 & 3)
            rounding.add(mxcsr >> 13 & 3)
            daz.add(mxcsr >> 6 & 1)
            ftz.add(mxcsr >> 15)
            top, tags = fsw >> 11 & 7, fx[4]
            loaded = bin(tags).count("1")
            loaded_counts.add(loaded)
            self.assertEqual((top + loaded) % 8, 0, seed)
            self.assertEqual(tags, sum(1 << (top + k) % 8 for k in range(loaded)), seed)
            for k in range(loaded):  # ST(k): a valid value
                value = int.from_bytes(fx[32 + 16 * k : 42 + 16 * k], "little")
                exp, mant = value >> 64 & 0x7FFF, value & (1 << 64) - 1
                self.assertTrue(exp == mant == 0 or mant >> 63, (seed, k, hex(value)))
        self.assertEqual(loaded_counts, set(range(9)))
        self.assertEqual(rounding, {0, 1, 2, 3})
        self.assertEqual(precision, {0, 2, 3})
        self.assertEqual(daz, {0, 1})
        self.assertEqual(ftz, {0, 1})

    def test_x87_state_memory(self):
        """fldcw and ldmxcsr load masked exceptions; fldenv and frstor reload what
        fnstenv and fnsave stored; the pointer fields stored in the buffer are cleared."""
        seen = set()
        for seed in range(3000):
            for item in gen.generate(seed).items:
                mn = item.mnemonic
                if mn not in gen.X87_MEMORY:
                    continue
                at = next((k for k, line in enumerate(item.lines) if line.startswith(mn + " ")), None)
                if at is None or "[" not in item.lines[at]:
                    continue
                seen.add(mn)
                (off,) = {off for line, _, off in addresses(item) if line == item.lines[at]}
                before, after = item.lines[:at], item.lines[at + 1:]
                clears = [f"mov dword ptr [rip + buf + {off + k}], 0" for k in gen.ENV_POINTERS]
                if mn == "fldcw":
                    value = int(before[-1].rsplit(", ", 1)[1], 0)
                    self.assertEqual(before[-1].split(",")[0], f"mov word ptr [rip + buf + {off}]")
                    self.assertEqual(value & 0x3F, 0x3F, (seed, item.lines))
                elif mn == "ldmxcsr":
                    value = int(before[-1].rsplit(", ", 1)[1], 0)
                    self.assertEqual(before[-1].split(",")[0], f"mov dword ptr [rip + buf + {off}]")
                    self.assertEqual(value & 0x1F80, 0x1F80, (seed, item.lines))
                    self.assertLess(value, 1 << 16)
                elif mn == "fbld":
                    digits = [int(line.rsplit(", ", 1)[1], 0) for line in before[-3:]]
                    self.assertTrue(all(c in "0123456789" for c in f"{digits[0]:x}{digits[1]:x}"))
                    self.assertTrue(all(c in "0123456789" for c in f"{digits[2] & 0xFF:x}"))
                    self.assertEqual(digits[2] & 0x7F00, 0, (seed, item.lines))
                elif mn in ("fldenv", "frstor"):
                    store = "fnstenv" if mn == "fldenv" else "fnsave"
                    self.assertEqual(before[-5:], [f"{store} [rip + buf + {off}]"] + clears)
                else:
                    self.assertEqual(after, clears, (seed, item.lines))
        self.assertEqual(seen, gen.X87_MEMORY)

    def test_maskmov_destination(self):
        seen = set()
        for seed in range(3000):
            for item in gen.generate(seed).items:
                if item.mnemonic in gen.MASKMOV:
                    seen.add(item.mnemonic)
                    m = LEA_BUF.match(item.lines[-2])
                    self.assertTrue(m and m[1] == "rdi", (seed, item.lines))
                    off = int(m[2])
                    self.assertTrue(0 <= off <= gen.BUF_SIZE - gen.MASKMOV[item.mnemonic])
        self.assertEqual(seen, set(gen.MASKMOV))

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
                undefined = gen.apply(row, item, undefined)
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
