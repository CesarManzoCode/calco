"""Dump record of a case (contract 4.4): 1280 bytes, little-endian.

`0` magic `CALCODMP` · `8` case index u32 · `12` reserved u32 · `16` GPR ×16 in encoding
order · `144` RIP · `152` RFLAGS · `160` FXSAVE64 (512) · `672` data buffer (256) · `928`
256-byte stack window below the initial RSP · `1184` zero padding (96).
"""

import struct
from dataclasses import dataclass

MAGIC = b"CALCODMP"
SIZE = 1280
OFF_INDEX, OFF_GPR, OFF_RIP, OFF_RFLAGS = 8, 16, 144, 152
OFF_FXSAVE, OFF_BUF, OFF_STACK, OFF_PAD = 160, 672, 928, 1184
FXSAVE_SIZE, BUF_SIZE, STACK_SIZE, PAD_SIZE = 512, 256, 256, 96
GPR_NAMES = "rax rcx rdx rbx rsp rbp rsi rdi r8 r9 r10 r11 r12 r13 r14 r15".split()


class DumpError(ValueError):
    pass


@dataclass
class Record:
    index: int
    gpr: list
    rip: int
    rflags: int
    fxsave: bytes
    buf: bytes
    stack: bytes

    # Fields of the FXSAVE64 area.
    @property
    def fcw(self):
        return int.from_bytes(self.fxsave[0:2], "little")

    @property
    def fsw(self):
        return int.from_bytes(self.fxsave[2:4], "little")

    @property
    def ftw(self):
        return self.fxsave[4]

    @property
    def mxcsr(self):
        return int.from_bytes(self.fxsave[24:28], "little")

    def st(self, i):
        """80-bit ST(i)/MMi register (the 6 reserved bytes of its slot are left out)."""
        return self.fxsave[32 + 16 * i : 42 + 16 * i]

    def xmm(self, i):
        return self.fxsave[160 + 16 * i : 176 + 16 * i]


def parse(data):
    """One record from exactly SIZE bytes; DumpError if it is not well formed."""
    if len(data) != SIZE:
        raise DumpError(f"record of {len(data)} bytes, not {SIZE}")
    if data[:8] != MAGIC:
        raise DumpError(f"bad magic {data[:8]!r}")
    index, reserved = struct.unpack_from("<II", data, OFF_INDEX)
    if reserved:
        raise DumpError(f"case {index}: reserved field {reserved:#x}")
    if any(data[OFF_PAD:]):
        raise DumpError(f"case {index}: padding is not zero")
    gpr = list(struct.unpack_from("<16Q", data, OFF_GPR))
    rip, rflags = struct.unpack_from("<QQ", data, OFF_RIP)
    return Record(
        index, gpr, rip, rflags,
        bytes(data[OFF_FXSAVE:OFF_BUF]),
        bytes(data[OFF_BUF:OFF_STACK]),
        bytes(data[OFF_STACK:OFF_PAD]),
    )


def pack(rec):
    out = bytearray(SIZE)
    out[:8] = MAGIC
    struct.pack_into("<II", out, OFF_INDEX, rec.index, 0)
    struct.pack_into("<16Q", out, OFF_GPR, *rec.gpr)
    struct.pack_into("<QQ", out, OFF_RIP, rec.rip, rec.rflags)
    for off, size, blob in ((OFF_FXSAVE, FXSAVE_SIZE, rec.fxsave), (OFF_BUF, BUF_SIZE, rec.buf),
                            (OFF_STACK, STACK_SIZE, rec.stack)):
        if len(blob) != size:
            raise DumpError(f"field at {off} has {len(blob)} bytes, not {size}")
        out[off : off + size] = blob
    return bytes(out)


def split(stream):
    """Records of a stdout stream. A trailing partial record raises DumpError."""
    if len(stream) % SIZE:
        raise DumpError(f"stream of {len(stream)} bytes is not a multiple of {SIZE}")
    return [parse(stream[k : k + SIZE]) for k in range(0, len(stream), SIZE)]
