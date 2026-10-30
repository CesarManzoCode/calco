"""Tests of the dump record format (contract 4.4)."""

import random
import unittest

import dump


def sample(index=7, seed=0):
    r = random.Random(seed)
    return dump.Record(index, [r.getrandbits(64) for _ in range(16)], r.getrandbits(64),
                       r.getrandbits(64), r.randbytes(512), r.randbytes(256), r.randbytes(256))


class Format(unittest.TestCase):
    def test_size(self):
        self.assertEqual(dump.SIZE, 1280)
        self.assertEqual(len(dump.pack(sample())), 1280)
        self.assertEqual(dump.OFF_PAD + dump.PAD_SIZE, dump.SIZE)

    def test_round_trip(self):
        for seed in range(20):
            rec = sample(seed * 3, seed)
            data = dump.pack(rec)
            self.assertEqual(dump.parse(data), rec)
            self.assertEqual(dump.pack(dump.parse(data)), data)

    def test_offsets(self):
        rec = sample()
        data = dump.pack(rec)
        self.assertEqual(data[:8], b"CALCODMP")
        self.assertEqual(int.from_bytes(data[8:12], "little"), rec.index)
        self.assertEqual(int.from_bytes(data[16 + 8 * 4 : 24 + 8 * 4], "little"), rec.gpr[4])
        self.assertEqual(int.from_bytes(data[144:152], "little"), rec.rip)
        self.assertEqual(int.from_bytes(data[152:160], "little"), rec.rflags)
        self.assertEqual(data[160:672], rec.fxsave)
        self.assertEqual(data[672:928], rec.buf)
        self.assertEqual(data[928:1184], rec.stack)
        self.assertEqual(data[1184:], bytes(96))

    def test_fxsave_fields(self):
        fx = bytearray(512)
        fx[0:2], fx[2:4], fx[4] = b"\x7f\x03", b"\x00\x38", 0x81
        fx[24:28] = (0x1F80).to_bytes(4, "little")
        fx[32 + 16 * 2 : 42 + 16 * 2] = bytes(range(1, 11))
        fx[160 + 16 * 5 : 176 + 16 * 5] = bytes(range(16))
        rec = dump.Record(0, [0] * 16, 0, 0, bytes(fx), bytes(256), bytes(256))
        self.assertEqual((rec.fcw, rec.fsw, rec.ftw, rec.mxcsr), (0x037F, 0x3800, 0x81, 0x1F80))
        self.assertEqual(rec.st(2), bytes(range(1, 11)))
        self.assertEqual(rec.xmm(5), bytes(range(16)))

    def test_split(self):
        recs = [sample(i, i) for i in range(3)]
        self.assertEqual(dump.split(b"".join(map(dump.pack, recs))), recs)
        self.assertEqual(dump.split(b""), [])

    def test_malformed(self):
        data = dump.pack(sample())
        for bad in (data[:-1], b"X" + data[1:], data[:12] + b"\x01" + data[13:],
                    data[:-1] + b"\x01"):
            with self.assertRaises(dump.DumpError):
                dump.parse(bad)
        with self.assertRaises(dump.DumpError):
            dump.split(data + data[:100])


if __name__ == "__main__":
    unittest.main()
