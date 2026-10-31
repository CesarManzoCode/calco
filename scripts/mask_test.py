"""Mask test (contract 6): takes a generated case that leaves some flags undefined and
others defined, runs it natively and checks on its record that altering an undefined
flag is not a discrepancy, altering a defined one is, and strict mode sees both.
Prints `mascara-ok`.
"""

import dataclasses
import sys

import common

sys.path.insert(0, str(common.ROOT / "harness"))
import build  # noqa: E402
import compare  # noqa: E402
import dump  # noqa: E402
import gen  # noqa: E402
import run  # noqa: E402


def main():
    common.require_non_root()
    case = next(c for c in map(gen.generate, range(10_000))
                if c.undefined and len(c.undefined) < len(gen.FLAGS))
    elf = build.build([case], common.BUILD / "mask-test", f"case_{case.index}")
    result = run.run(elf, "native")
    if not result.ok():
        common.fail(f"{elf} natively: {result.describe()}")
    (rec,) = dump.split(result.stdout)
    undefined = sorted(case.undefined)[0]
    defined = next(f for f in gen.FLAGS if f not in case.undefined)

    def flipped(flag):
        return dataclasses.replace(rec, rflags=rec.rflags ^ gen.flag_mask([flag]))

    checks = {
        "same record": compare.compare(rec, rec, case.undefined) == [],
        f"undefined {undefined} altered": compare.compare(rec, flipped(undefined), case.undefined) == [],
        f"defined {defined} altered": [f for f, _, _ in compare.compare(rec, flipped(defined), case.undefined)] == ["rflags"],
        f"strict, undefined {undefined} altered": compare.compare(rec, flipped(undefined), case.undefined, strict=True) != [],
    }
    print(f"caso {case.index}: indefinidas {' '.join(sorted(case.undefined))}")
    for name, ok in checks.items():
        print(f"{name}: {'ok' if ok else 'FALLA'}")
    if not all(checks.values()):
        common.fail("mask test failed")
    print("mascara-ok")


if __name__ == "__main__":
    main()
