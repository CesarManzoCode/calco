"""Comparison of two dump records (contract 4.4).

Compared: GPR, RIP, RFLAGS under the case mask (never RF, VM nor reserved bits), FCW,
FSW, abridged FTW, ST0–ST7, MXCSR, XMM0–XMM15, data buffer and stack window. FIP, FCS,
FDP, FDS, FOP and `mxcsr_mask` are not. The mask decides; `strict` also compares the
undefined flags and is only informative (D09).
"""

from gen import FLAG_BITS, flag_mask
from dump import GPR_NAMES

# CF PF AF ZF SF TF IF DF OF IOPL NT, AC VIF VIP ID: every defined bit but RF and VM.
RFLAGS_COMPARED = 0x3C7FD5


def fields(rec):
    """(name, value) of every compared field except RFLAGS."""
    out = list(zip(GPR_NAMES, rec.gpr))
    out += [("rip", rec.rip), ("fcw", rec.fcw), ("fsw", rec.fsw), ("ftw", rec.ftw)]
    out += [(f"st{i}", rec.st(i)) for i in range(8)]
    out.append(("mxcsr", rec.mxcsr))
    out += [(f"xmm{i}", rec.xmm(i)) for i in range(16)]
    return out + [("buf", rec.buf), ("stack", rec.stack)]


def show(value):
    return value.hex() if isinstance(value, bytes) else f"{value:#x}"


def compare(a, b, undefined=frozenset(), strict=False):
    """Discrepancies between records `a` and `b` as (field, value a, value b)."""
    out = [(name, show(va), show(vb)) for (name, va), (_, vb) in zip(fields(a), fields(b))
           if va != vb]
    keep = RFLAGS_COMPARED if strict else RFLAGS_COMPARED & ~flag_mask(undefined)
    if (a.rflags ^ b.rflags) & keep:
        out.append(("rflags", show(a.rflags & keep), show(b.rflags & keep)))
    if a.index != b.index:
        out.append(("index", show(a.index), show(b.index)))
    return out


def undefined_matches(a, b, undefined):
    """How many of the case's undefined flags are equal in `a` and `b` (strict mode)."""
    return sum(1 for f in undefined if not (a.rflags ^ b.rflags) >> FLAG_BITS[f] & 1)
