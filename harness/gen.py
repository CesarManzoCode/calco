"""Random case generator (contract 4.3).

Case `i` comes from `random.Random(i)` alone. A case is an initial state (GPR, RFLAGS,
FXSAVE image, 256-byte data buffer) and 1–16 items; an item is the instruction drawn from
`isa/scope.tsv` plus the preparation lines it needs (base registers loaded with `lea` from
the buffer, `div` operands, string pointers). The generator follows which of the six
arithmetic flags are defined and only draws an instruction that reads flags when all of
them are; the case mask is the set left undefined at the end.

Forms in `scope.tsv` are `SIZES:OPERANDS` separated by `|`. `SIZES` are letters `b w d q`
(8–64 bits) or `-` for none. Operand tokens: `r m rm` of the chosen size, `rN mN rmN` of
an explicit size, `i` (immediate of the size, at most 32 bits), `i8` (signed byte), `u8`,
`u16`, `iq` (64-bit immediate), `cl`, `one`, `a` (accumulator) and `rel` (forward jump).
x87 and SIMD tokens: `sti` (`st(i)`), `st0`, `mm`, `x` (XMM), `x0` (implicit XMM0), `mmm`
and `mmm32` (MMX register or memory of 64/32 bits), `xm` (XMM or unaligned m128), `xma`
(XMM or m128 aligned to 16), `xm16 xm32 xm64` (XMM or scalar memory), `m80`, `m128a`,
`menv` (28-byte x87 environment), `msave` (108-byte x87 save area), `i3` and `i4`
(immediates below 8 and 16).
A flag written as `?XX` may be left unchanged (count or repeat of zero). The one
refinement over the table is `effect`: a shift by an immediate count between 1 and the
width minus 1 writes CF, PF, ZF and SF, and also OF if the count is 1.

The initial x87 state has every exception masked, a random precision and rounding
control and 0–8 valid values on the stack; MXCSR has every exception masked and random
rounding, DAZ and FTZ. Instructions that load control state from memory (`fldcw`,
`ldmxcsr`, `fldenv`, `frstor`) get a preparation that writes a state with the exceptions
masked, and `fbld` one that writes valid BCD digits, so no case faults.
"""

import random
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SCOPE = ROOT / "isa" / "scope.tsv"
EXCLUSIONS = ROOT / "harness" / "exclusions.tsv"
UNDEFINED_FLAGS = ROOT / "harness" / "undefined_flags.tsv"

FLAGS = ("CF", "PF", "AF", "ZF", "SF", "OF")
FLAG_BITS = {"CF": 0, "PF": 2, "AF": 4, "ZF": 6, "SF": 7, "OF": 11}
DF_BIT = 10
BUF_SIZE = 256
MAX_ITEMS = 16

SIZES = {"b": 8, "w": 16, "d": 32, "q": 64}
PTR = {8: "byte ptr ", 16: "word ptr ", 32: "dword ptr ", 64: "qword ptr ", 80: "tbyte ptr ",
       128: "xmmword ptr "}
R64 = "rax rcx rdx rbx rsp rbp rsi rdi r8 r9 r10 r11 r12 r13 r14 r15".split()
REGS = {
    64: R64,
    32: "eax ecx edx ebx esp ebp esi edi".split() + [f"r{i}d" for i in range(8, 16)],
    16: "ax cx dx bx sp bp si di".split() + [f"r{i}w" for i in range(8, 16)],
    8: "al cl dl bl spl bpl sil dil".split() + [f"r{i}b" for i in range(8, 16)],
}
RAX, RCX, RDX, RBX, RSP, RBP, RSI, RDI = range(8)
# Register files of the x87 and SIMD tokens: (name format, registers).
REG_FILES = {"sti": ("st({})", 8), "mm": ("mm{}", 8), "x": ("xmm{}", 16)}
# Register-or-memory tokens: (register file, memory bits, alignment).
VECTOR_RM = {"mmm": ("mm", 64, 1), "mmm32": ("mm", 32, 1), "xm": ("x", 128, 1),
             "xma": ("x", 128, 16), "xm16": ("x", 16, 1), "xm32": ("x", 32, 1),
             "xm64": ("x", 64, 1)}
# Memory-only tokens with their own size: (bits, alignment).
MEMORY = {"m80": (80, 1), "m128a": (128, 16), "menv": (28 * 8, 1), "msave": (108 * 8, 1)}
TOKENS = {"r", "m", "rm", "i", "i8", "u8", "u16", "iq", "cl", "one", "a", "rel"} | {
    f"{k}{n}" for k in ("r", "m", "rm") for n in (8, 16, 32, 64, 128)
} | set(REG_FILES) | set(VECTOR_RM) | set(MEMORY) | {"st0", "x0", "i3", "i4"}

# Instructions whose first operand is only read: RSP may appear there.
READ_FIRST = {"cmp", "test", "bt", "push", "nop"}
# Instructions that write every register operand.
WRITE_ALL = {"xchg", "xadd", "cmpxchg"}
LOCKABLE = {"add", "adc", "and", "btc", "btr", "bts", "cmpxchg", "cmpxchg8b", "cmpxchg16b",
            "dec", "inc", "neg", "not", "or", "sbb", "sub", "xadd", "xchg", "xor"}
# Registers the preparation of an instruction sets: never memory bases, never its divisor.
IMPLICIT = {"div": {RAX, RDX}, "idiv": {RAX, RDX}}
STRINGS = {"movs": "rep", "stos": "rep", "lods": "rep", "cmps": "repe repne", "scas": "repe repne"}
JUMPS = {"loop", "loope", "loopne", "jrcxz"}  # rel8 only: target at most two items ahead
SIGN_EXTEND = {8: "cbw", 16: "cwd", 32: "cdq", 64: "cqo"}
HIGH = {8: "ah", 16: "dx", 32: "edx", 64: "rdx"}
STACK_LOW, STACK_HIGH = -64, 128  # net bytes pushed by the sequence, kept in this range
MASKMOV = {"maskmovq": 8, "maskmovdqu": 16}  # store to [rdi] of this many bytes
# Bytes 12–27 of the x87 environment: FIP, FCS, FOP, FDP, FDS. FIP points to the code
# cache under calco, so they are cleared before the buffer is compared (contract 3.8).
ENV_POINTERS = range(12, 28, 4)


# State instructions with a memory operand that need lines around them (`control_memory`).
X87_MEMORY = {"fldcw", "ldmxcsr", "fbld", "fldenv", "frstor", "fnstenv", "fnsave"}


def flag_mask(flags):
    """RFLAGS bits of a set of flag names."""
    return sum(1 << FLAG_BITS[f] for f in flags)


@dataclass(frozen=True)
class Row:
    mnemonic: str
    ext: str
    forms: tuple  # ((size letters or "", (tokens...)), ...)
    read: frozenset
    written: frozenset  # written for sure
    maybe: frozenset  # possibly left unchanged
    undefined: frozenset
    ref: str


def _flags(text):
    return frozenset(text.split())


def load_scope(path=SCOPE):
    rows = {}
    lines = Path(path).read_text().splitlines()
    for line in lines[1:]:
        if not line.strip():
            continue
        mn, ext, forms, read, written, undef, ref = line.split("\t")
        parsed = []
        for form in forms.split("|"):
            sizes, _, ops = form.partition(":")
            parsed.append(("" if sizes == "-" else sizes, tuple(ops.split(",")) if ops else ()))
        flags = written.split()
        rows[mn] = Row(mn, ext, tuple(parsed), _flags(read),
                       frozenset(f for f in flags if not f.startswith("?")),
                       frozenset(f[1:] for f in flags if f.startswith("?")), _flags(undef), ref)
    return rows


def load_exclusions(path=EXCLUSIONS):
    return {line.split("\t")[0] for line in Path(path).read_text().splitlines()[1:] if line}


def load_undefined(path=UNDEFINED_FLAGS):
    """({mnemonic: (undefined flags, ref)}, header comment lines) of `undefined_flags.tsv`."""
    lines = Path(path).read_text().splitlines()
    comments = [line for line in lines if line.startswith("#")]
    rows = {}
    for line in [line for line in lines if line and not line.startswith("#")][1:]:
        mn, undefined, ref = line.split("\t")
        rows[mn] = (_flags(undefined), ref.strip())
    return rows, comments


SHIFTS = {"shl", "shr", "sar"}
SHIFT_DEFINED = frozenset({"CF", "PF", "ZF", "SF"})


def width(operand):
    """Bits of a register or `ptr` memory operand of the generator; None if unknown."""
    for bits, prefix in PTR.items():
        if operand.startswith(prefix):
            return bits
    return next((bits for bits, regs in REGS.items() if operand in regs), None)


def effect(row, item):
    """(written, undefined) flags of `item`. They are the row's, except for `shl`, `shr`
    and `sar` by an immediate count that, masked as the CPU masks it, is between 1 and the
    width minus 1: then CF, PF, ZF and SF are written, OF too if the count is 1, and only
    AF (and OF for other counts) is undefined (APM3 SAL/SHL, SHR, SAR)."""
    if row.mnemonic in SHIFTS:
        dest, _, count = item.lines[-1].partition(" ")[2].rpartition(", ")
        bits = width(dest)
        if bits and count.isdigit():
            n = int(count) & (63 if bits == 64 else 31)
            if 0 < n < bits:
                if n == 1:
                    return SHIFT_DEFINED | {"OF"}, frozenset({"AF"})
                return SHIFT_DEFINED, frozenset({"AF", "OF"})
    return row.written, row.undefined


def apply(row, item, undefined):
    """Undefined flags after `item`, an instance of `row`."""
    written, undef = effect(row, item)
    return (undefined - written) | undef


def sequence_flags(items, rows):
    """(whether every item reads only defined flags, mask at the end) of `items` run in
    order from a fully defined state, with forward jumps joining at their targets. Items
    may be any subset of a generated case (the reducer's candidates); a jump whose target
    is gone lands on the next item kept, as `build.case_text` does."""
    undefined, pending, ok = frozenset(), {}, True
    ids = [item.id for item in items]
    for item in items:
        for t in [t for t in pending if t <= item.id]:
            undefined |= pending.pop(t)
        row = rows[item.mnemonic]
        if row.read & undefined:
            ok = False
        undefined = apply(row, item, undefined)
        if item.target is not None:
            target = next((i for i in ids if i >= item.target), None)
            key = target if target is not None else float("inf")
            pending[key] = pending.get(key, frozenset()) | undefined
    for later in pending.values():
        undefined |= later
    return ok, undefined


@dataclass
class Item:
    id: int  # position in the generated sequence; the reducer keeps it
    mnemonic: str
    lines: list
    # Id of the item a forward jump goes to (`{T}` in `lines`); None if no jump.
    target: int = None


@dataclass
class Case:
    index: int
    gpr: list  # 16 values in encoding order; RSP is set by the batch
    rflags: int
    fxsave: bytes
    buf: bytes
    items: list
    undefined: frozenset
    # (offset, size) in the buffer of every explicit memory operand.
    accesses: list = field(default_factory=list)


class _Gen:
    def __init__(self, index, rows):
        self.rng = random.Random(index)
        self.index = index
        self.rows = rows
        self.accesses = []

    # Values with a bias to edges.
    def value(self, bits, signed=False):
        r = self.rng
        lo, hi = (-(1 << bits - 1), (1 << bits - 1) - 1) if signed else (0, (1 << bits) - 1)
        k = r.randrange(8)
        if k == 0:
            v = r.choice([0, 1, -1, hi, lo, hi >> 1, 1 << bits - 1])
        elif k == 1:
            v = 1 << r.randrange(bits)
        elif k == 2:
            v = (1 << r.randrange(bits)) - 1
        else:
            v = r.randint(lo, hi)
        if signed:
            v &= (1 << bits) - 1
            return v - (1 << bits) if v >> bits - 1 else v
        return v & hi

    def reg(self, bits, used, dest):
        free = [i for i in range(16) if i not in used and not (dest and i == RSP)]
        i = self.rng.choice(free)
        used.add(i)
        return REGS[bits][i]

    def mem(self, bits, used, align=1, lo=0, hi=BUF_SIZE):
        r = self.rng
        n = max(bits // 8, 1) if bits else 1
        off = r.randrange(lo, hi - n + 1)
        off -= off % align
        self.accesses.append((off, n))
        ptr = PTR.get(bits, "")
        mode = r.randrange(3)
        if mode == 0:
            return [], f"{ptr}[rip + buf + {off}]"
        if mode == 1:
            return [], f"{ptr}[buf + {off}]"
        free = [i for i in range(16) if i not in used and i != RSP]
        base, index = r.sample(free, 2)
        used.update((base, index))
        if r.randrange(3) == 0:
            disp = r.randint(-128, 128)
            return [f"lea {R64[base]}, [rip + buf{off - disp:+d}]"], f"{ptr}[{R64[base]}{disp:+d}]"
        scale, ival, disp = r.choice((1, 2, 4, 8)), r.randint(0, 8), r.randint(-64, 64)
        b = off - ival * scale - disp
        prep = [f"lea {R64[base]}, [rip + buf{b:+d}]", f"mov {REGS[32][index]}, {ival}"]
        return prep, f"{ptr}[{R64[base]} + {R64[index]}*{scale}{disp:+d}]"

    def operand(self, tok, bits, used, dest, prep, mn):
        r = self.rng
        if tok in VECTOR_RM:
            file, mbits, align = VECTOR_RM[tok]
            if r.randrange(2):
                tok = file
            else:
                p, text = self.mem(mbits, used, align)
                prep += p
                return text
        if tok in REG_FILES:
            name, count = REG_FILES[tok]
            return name.format(r.randrange(count))
        if tok == "st0":
            return "st(0)"
        if tok == "x0":
            return "xmm0"
        if tok in ("i3", "i4"):
            return str(r.randrange(1 << int(tok[1])))
        if tok in MEMORY:
            mbits, align = MEMORY[tok]
            p, text = self.mem(mbits, used, align)
            prep += p
            return text
        if tok[-1].isdigit() and tok not in ("i8", "u8", "u16"):
            kind = tok.rstrip("0123456789")
            bits = int(tok[len(kind):])
            tok = kind
        if tok == "rm":
            tok = r.choice(("r", "m"))
        if tok == "r":
            return self.reg(bits, used, dest)
        if tok == "m":
            align = {"cmpxchg16b": 16, "cmpxchg8b": 8}.get(mn, 1)
            lo, hi = (16, BUF_SIZE - 24) if mn.startswith("bt") else (0, BUF_SIZE)
            p, text = self.mem(None if mn == "lea" else bits, used, align, lo, hi)
            prep += p
            return text.removeprefix(PTR[128]) if mn == "cmpxchg16b" else text
        if tok == "i":
            return str(self.value(min(bits, 32), signed=True))
        if tok == "i8":
            return str(self.value(8, signed=True))
        if tok == "u8":
            return str(r.choice([0, 1, bits - 1, bits, bits + 1, 255, r.randrange(256)]))
        if tok == "iq":
            return hex(self.value(64))
        if tok == "cl":
            used.add(RCX)
            return "cl"
        if tok == "one":
            return "1"
        if tok == "a":
            used.add(RAX)
            return REGS[bits][RAX]
        raise ValueError(f"token {tok} of {mn}")

    def instruction(self, row, i, n, depth):
        """Returns (item lines, jump target or None, stack delta), or None if `row` does
        not fit here."""
        r, mn = self.rng, row.mnemonic
        sizes, toks = r.choice(row.forms)
        bits = SIZES[r.choice(sizes)] if sizes else 64
        used = set(IMPLICIT.get(mn, ()))
        prep, lines = [], []

        if mn in ("jmp", "call", "ret"):
            return self.control(mn, toks, i, n, used)
        if "rel" in toks:
            target = r.randint(i + 1, min(n, i + 2) if mn in JUMPS else n)
            return [f"{mn} {{T}}"], target, 0
        if mn in STRINGS:
            letter = {8: "b", 16: "w", 32: "d", 64: "q"}[bits]
            lines = ["lea rsi, [rip + buf + 96]", "lea rdi, [rip + buf + 128]"]
            text = f"{mn}{letter}"
            if r.randrange(2):
                lines.append(f"mov ecx, {r.randint(0, 8)}")
                text = f"{r.choice(STRINGS[mn].split())} {text}"
            self.accesses += [(32, 136), (64, 136)]
            return lines + [text], None, 0

        delta = {"push": 8, "pop": -8, "pushf": 8}.get(mn, 0)
        after = []  # lines after the instruction
        if mn == "enter":
            frame, level = r.choice((0, 8, 16, 32)), r.randrange(4)
            delta = 8 + 8 * level + frame
            if level:
                lines.append("mov rbp, rsp")
            text = f"enter {frame}, {level}"
        elif mn == "leave":
            k = r.randrange(3)
            delta = -8 - 8 * k
            lines.append(f"lea rbp, [rsp + {8 * k}]")
            text = "leave"
        elif mn == "popf":
            value = sum(1 << b for b in [*FLAG_BITS.values(), DF_BIT] if r.randrange(2))
            lines.append(f"push {value | 2}")
            text = "popfq"
        elif mn == "pushf":
            text = "pushfq"
        elif mn == "xlat":
            lines.append("lea rbx, [rip + buf]")
            self.accesses.append((0, BUF_SIZE))
            text = "xlatb"
        else:
            ops = []
            for k, tok in enumerate(toks):
                dest = mn in WRITE_ALL or (k == 0 and mn not in READ_FIRST)
                if mn in ("div", "idiv") or mn.startswith("bt"):
                    dest = True
                ops.append(self.operand(tok, bits, used, dest, prep, mn))
            if mn in ("div", "idiv"):
                lines += self.divide(mn, bits, ops[0], prep)
            elif mn in MASKMOV:
                off = r.randrange(BUF_SIZE - MASKMOV[mn] + 1)
                self.accesses.append((off, MASKMOV[mn]))
                lines.append(f"lea rdi, [rip + buf{off:+d}]")
            elif ops and ops[0].endswith("]") and mn in X87_MEMORY:
                before, after = self.control_memory(mn, self.accesses[-1][0])
                lines += prep + before
            elif mn.startswith("bt") and toks[1] == "r" and ops[0].endswith("]"):
                lines += prep + [f"mov {ops[1]}, {r.randint(-64, 63)}"]
            else:
                lines += prep
            text = f"{mn} {', '.join(ops)}" if ops else mn
            if mn in LOCKABLE and ops and ops[0].endswith("]") and r.randrange(2):
                text = "lock " + text
        if not STACK_LOW <= depth + delta <= STACK_HIGH:
            return None
        return lines + [text] + after, None, delta

    def control(self, mn, toks, i, n, used):
        r = self.rng
        tmp = self.reg(64, used, True)
        if mn == "ret":
            if toks:
                return [f"push {tmp}", "call 1f", "jmp 2f", "1: ret 8", "2:"], None, 0
            return ["call 1f", "jmp 2f", "1: ret", "2:"], None, 0
        target = None
        if mn == "jmp":
            target = r.randint(i + 1, n)
            dest = "{T}"
        else:
            dest = "1f"
        lines = []
        if toks[0] == "rel":
            lines.append(f"{mn} {dest}")
        else:
            lines.append(f"lea {tmp}, [rip + {dest}]")
            if toks[0] == "m64":
                prep, op = self.mem(64, used)
                lines += prep + [f"mov {op}, {tmp}", f"{mn} {op}"]
            else:
                lines.append(f"{mn} {tmp}")
        if mn == "call":
            lines.append(f"1: pop {self.reg(64, used, True)}")
        return lines, target, 0

    def control_memory(self, mn, off):
        """Lines before and after an x87 or MXCSR state instruction whose memory operand
        is at buffer offset `off`."""
        r = self.rng
        at = lambda k: f"[rip + buf + {off + k}]"  # noqa: E731
        clear = [f"mov dword ptr {at(k)}, 0" for k in ENV_POINTERS]
        if mn == "fldcw":
            return [f"mov word ptr {at(0)}, {self.fcw():#x}"], []
        if mn == "ldmxcsr":
            return [f"mov dword ptr {at(0)}, {self.mxcsr():#x}"], []
        if mn == "fbld":
            digits = [int("".join(r.choice("0123456789") for _ in range(n)), 16) for n in (8, 8, 2)]
            sign = r.randrange(2) << 15
            return [f"mov dword ptr {at(0)}, {digits[0]:#x}", f"mov dword ptr {at(4)}, {digits[1]:#x}",
                    f"mov word ptr {at(8)}, {sign | digits[2]:#x}"], []
        if mn in ("fldenv", "frstor"):
            return [f"{'fnstenv' if mn == 'fldenv' else 'fnsave'} {at(0)}"] + clear, []
        return [], clear  # fnstenv, fnsave

    def fcw(self):
        """x87 control word: exceptions masked, random precision (24, 53 or 64 bits) and rounding."""
        r = self.rng
        return 0x7F | r.choice((0, 2, 3)) << 8 | r.randrange(4) << 10

    def mxcsr(self):
        """MXCSR: exceptions masked, random rounding, DAZ and FTZ."""
        r = self.rng
        return 0x1F80 | r.randrange(4) << 13 | r.randrange(2) << 6 | r.randrange(2) << 15

    def x87_value(self):
        """A valid 80-bit value: ±0, ±infinity or a normal number near 1."""
        r = self.rng
        sign, k = r.randrange(2) << 79, r.randrange(8)
        if k == 0:
            return sign
        if k == 1:
            return sign | 0x7FFF << 64 | 1 << 63
        return sign | (0x3FFF + r.randint(-70, 70)) << 64 | 1 << 63 | r.getrandbits(63)

    def divide(self, mn, bits, divisor, prep):
        """Preparation of `div`/`idiv`: a non-zero divisor and a quotient that fits."""
        r = self.rng
        mask = (1 << bits) - 1
        if divisor.endswith("]") and bits == 64:
            v = 0
            while v in (0, -1):
                v = self.value(32, signed=True)
            v &= mask
        else:
            v = 0
            while v == 0 or (mn == "idiv" and v == mask):
                v = self.value(bits)
        shown = v - (1 << 64) if divisor.endswith("]") and v >> 63 else v
        lines = prep + [f"mov {divisor}, {shown if divisor.endswith(']') else hex(v)}"]
        if mn == "div":
            h = r.choice([0, v - 1, r.randrange(v)])
            return lines + [f"mov {HIGH[bits]}, {hex(h)}"]
        return lines + [SIGN_EXTEND[bits]]

    def initial(self):
        r = self.rng
        gpr = [0 if k == RSP else self.value(64) for k in range(16)]
        rflags = 2 | sum(1 << b for b in FLAG_BITS.values() if r.randrange(2))
        fx = bytearray(512)
        fx[0:2] = self.fcw().to_bytes(2, "little")
        # `loaded` values on the stack: ST(0)…ST(loaded-1) are the physical registers
        # TOP…7, tagged valid in the abridged tag word. The empty ones keep random bytes,
        # which MMX reads as MM registers.
        loaded = r.randrange(9)
        top = -loaded % 8
        fx[2:4] = (top << 11).to_bytes(2, "little")
        fx[4] = sum(1 << (top + k) % 8 for k in range(loaded))
        fx[24:28] = self.mxcsr().to_bytes(4, "little")
        for k in range(8):
            value = self.x87_value().to_bytes(10, "little") if k < loaded else r.randbytes(10)
            fx[32 + 16 * k : 42 + 16 * k] = value
        fx[160:416] = r.randbytes(256)
        return gpr, rflags, bytes(fx), r.randbytes(BUF_SIZE)

    def case(self):
        r = self.rng
        gpr, rflags, fx, buf = self.initial()
        pool = sorted(self.rows)
        n = r.randint(1, MAX_ITEMS)
        undefined, pending, depth, items = frozenset(), {}, 0, []
        for i in range(n):
            undefined |= pending.pop(i, frozenset())
            while True:
                row = self.rows[r.choice(pool)]
                if row.read & undefined:
                    continue
                accesses = len(self.accesses)
                got = self.instruction(row, i, n, depth)
                if got is None:
                    del self.accesses[accesses:]
                    continue
                break
            lines, target, delta = got
            depth += delta
            item = Item(i, row.mnemonic, lines, target)
            undefined = apply(row, item, undefined)
            if target is not None:
                pending[target] = pending.get(target, frozenset()) | undefined
            items.append(item)
        undefined |= pending.pop(n, frozenset())
        return Case(self.index, gpr, rflags, fx, buf, items, undefined, self.accesses)


_ROWS = None


def generatable(rows=None, exclusions=None):
    """Rows of the scope the generator can draw: not excluded and with known tokens."""
    rows = load_scope() if rows is None else rows
    exclusions = load_exclusions() if exclusions is None else exclusions
    return {
        mn: row for mn, row in rows.items()
        if mn not in exclusions and all(set(t) <= TOKENS for _, t in row.forms)
    }


def generate(index, rows=None):
    """The case of seed `index`."""
    global _ROWS
    if rows is None:
        if _ROWS is None:
            _ROWS = generatable()
        rows = _ROWS
    return _Gen(index, rows).case()
