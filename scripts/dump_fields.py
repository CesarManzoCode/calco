"""Dump fields (contract 6): checks every record that random_compare.py left in
build/random/{native,calco,qemu}/: magic, size, reserved and padding fields, every field
of 4.4 readable, and case indices in order (batch b holds cases 500·b onward). Prints
`registros: <n>` and `campos-ok`.

    python3 scripts/dump_fields.py [DIR]
"""

import sys
from pathlib import Path

import common

sys.path.insert(0, str(common.ROOT / "harness"))
import build  # noqa: E402
import compare  # noqa: E402
import dump  # noqa: E402


def check_file(path):
    """Number of records in `path`; fails on the first malformed one."""
    first = int(path.stem.split("_")[1]) * build.BATCH
    data = path.read_bytes()
    try:
        recs = dump.split(data)
    except dump.DumpError as e:
        common.fail(f"{path}: {e}")
    for k, rec in enumerate(recs):
        if rec.index != first + k:
            common.fail(f"{path}: record {k} has case {rec.index}, expected {first + k}")
        names = [name for name, _ in compare.fields(rec)]
        if len(names) != 16 + 1 + 3 + 8 + 1 + 16 + 2:
            common.fail(f"{path}: case {rec.index} has {len(names)} fields")
        if len(rec.fxsave) != dump.FXSAVE_SIZE or len(rec.buf) != dump.BUF_SIZE \
                or len(rec.stack) != dump.STACK_SIZE:
            common.fail(f"{path}: case {rec.index} has a truncated field")
    return len(recs)


def main():
    common.require_non_root()
    base = Path(sys.argv[1]) if len(sys.argv) > 1 else common.BUILD / "random"
    total = 0
    for mode in ("native", "calco", "qemu"):
        files = sorted((base / mode).glob("batch_*.bin"))
        if mode != "qemu" and not files:
            common.fail(f"no dumps in {base / mode}: run random_compare.py first")
        total += sum(check_file(f) for f in files)
    print(f"registros: {total}")
    print("campos-ok")


if __name__ == "__main__":
    main()
