"""Mutants (contract 4.6 and 6): for every row of harness/mutants.tsv, builds calco with
that feature into build/mutants/<name>/ and runs 20 000 random cases against native with
`random_compare.py --calco`. A mutant is detected if there is at least one discrepancy.
Prints one line per mutant and `mutantes detectados: X/M`; exits with 0 only if X = M.

    python3 scripts/mutants.py [--n N]
"""

import argparse
import subprocess
import sys

import common

MUTANTS = common.ROOT / "harness" / "mutants.tsv"
OUT = common.BUILD / "mutants"
CASES = 20_000


def load():
    """[(feature, description)] of harness/mutants.tsv."""
    lines = MUTANTS.read_text().splitlines()[1:]
    return [tuple(line.split("\t", 1)) for line in lines if line.strip()]


def name(feature):
    return feature.removeprefix("mutant-")


def build_mutant(feature):
    """Builds the mutant as 4.6 says and returns the path of its binary."""
    target = OUT / name(feature)
    cmd = ["cargo", "build", "--release", "--features", feature, "--target-dir", str(target)]
    if subprocess.run(cmd, cwd=common.ROOT).returncode != 0:
        common.fail(f"cargo build of {feature} failed")
    return target / "release" / "calco"


def compare_dir(feature):
    return OUT / name(feature) / "random"


def random_compare(feature, binary, n=CASES):
    """Discrepancies of `binary` in cases 0…n-1, or None if random_compare.py did not
    report a count."""
    cmd = [sys.executable, str(common.ROOT / "scripts" / "random_compare.py"), "--n", str(n),
           "--calco", str(binary), "--out", str(compare_dir(feature)), "--no-qemu"]
    proc = subprocess.run(cmd, cwd=common.ROOT, capture_output=True, text=True)
    for line in proc.stdout.splitlines():
        if line.startswith("discrepancias: "):
            return int(line.split()[1])
    sys.stderr.write(proc.stderr[-2000:])
    return None


def main():
    common.require_non_root()
    p = argparse.ArgumentParser()
    p.add_argument("--n", type=int, default=CASES)
    args = p.parse_args()
    mutants = load()
    detected = 0
    for feature, _ in mutants:
        found = random_compare(feature, build_mutant(feature), args.n)
        if found is None:
            print(f"{feature}: error de random_compare.py")
        elif found:
            detected += 1
            print(f"{feature}: detectado ({found} casos con discrepancia)")
        else:
            print(f"{feature}: no detectado")
    print(f"mutantes detectados: {detected}/{len(mutants)}")
    sys.exit(0 if mutants and detected == len(mutants) else 1)


if __name__ == "__main__":
    main()
