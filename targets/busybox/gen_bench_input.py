"""Input of the busybox gzip workload of bench.py (contract 8): 16 MiB of pseudorandom
text with a fixed seed, written to OUT.

    python3 targets/busybox/gen_bench_input.py OUT

The text is a stream of words drawn with Zipf-like weights from a vocabulary that is
itself drawn from the same generator, with numbers and line breaks mixed in, so it is
compressible (repeated words, skewed byte frequencies) but has no period gzip can
exploit. Same seed, same bytes: `random.Random(seed)` is reproducible across runs.
"""

import random
import sys
from pathlib import Path

SIZE = 16 * 1024 * 1024
SEED = 20261030
VOCABULARY = 4096
LETTERS = "abcdefghijklmnopqrstuvwxyz"
# Words drawn per round; the stream is cut at SIZE.
ROUND = 1 << 16


def generate(size=SIZE, seed=SEED):
    """`size` bytes of compressible pseudorandom text for `seed`."""
    rng = random.Random(seed)
    words = ["".join(rng.choices(LETTERS, k=rng.randint(2, 10))) for _ in range(VOCABULARY)]
    words += [str(rng.randrange(100000)) for _ in range(VOCABULARY // 8)]
    weights = [1 / (rank + 1) for rank in range(len(words))]
    out = bytearray()
    while len(out) < size:
        for word in rng.choices(words, weights, k=ROUND):
            out += word.encode()
            out += b"\n" if rng.random() < 0.08 else b" "
    return bytes(out[:size])


def main():
    if len(sys.argv) != 2:
        sys.exit("usage: gen_bench_input.py OUT")
    out = Path(sys.argv[1])
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_bytes(generate())


if __name__ == "__main__":
    main()
