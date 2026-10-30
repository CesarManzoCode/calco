"""Non-deterministic instructions excluded (contract 6): the generator emits nothing of
`harness/exclusions.tsv` in cases 0…N-1 (the cases of random_compare.py), and
guests/nondet.c (RDTSC, RDTSCP, RDRAND, RCPPS, RSQRTPS) runs under calco with exit 0
and prints `ok`. Prints `no-deterministas-excluidas`.

    python3 scripts/nondet_check.py [--n N] [--calco BIN]
"""

import argparse
import sys
from pathlib import Path

import common

sys.path.insert(0, str(common.ROOT / "harness"))
import gen  # noqa: E402

PREFIXES = {"lock", "rep", "repe", "repne"}


def emitted(case):
    """Mnemonics of every line of the case: instructions and their preparations."""
    for item in case.items:
        for line in item.lines:
            if line.split()[0].endswith(":"):  # local label such as `1:`
                line = line.split(":", 1)[1]
            words = [w for w in line.split() if w not in PREFIXES]
            if words:
                yield item.mnemonic, words[0]


def main():
    common.require_non_root()
    p = argparse.ArgumentParser()
    p.add_argument("--n", type=int, default=100_000)
    p.add_argument("--calco", type=Path)
    args = p.parse_args()

    exclusions = gen.load_exclusions()
    found = sorted(exclusions & set(gen.generatable()))
    for mn in found:
        print(f"plantilla excluida: {mn}")
    for i in range(args.n):
        for item, word in emitted(gen.generate(i)):
            if item in exclusions or word in exclusions:
                found.append(f"{i}:{word}")
                print(f"caso {i}: {word} (item {item})")
    if found:
        common.fail(f"the generator emits {len(found)} excluded instructions")

    calco = args.calco.resolve() if args.calco else common.build_calco()
    guest = common.compile_guest("nondet")
    res = common.run_calco(calco, guest)
    if res.returncode != 0 or res.stdout != b"ok\n":
        common.fail(f"nondet under calco: {res.describe()}, stdout {res.stdout!r}, "
                    f"stderr {res.stderr[-500:]!r}")
    print("no-deterministas-excluidas")


if __name__ == "__main__":
    main()
