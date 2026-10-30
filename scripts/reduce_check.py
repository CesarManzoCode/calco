"""Reduction (contract 4.5 and 6): for every mutant of harness/mutants.tsv, takes the
first case with a discrepancy (from the `scripts/mutants.py` run in
build/mutants/<name>/random/, or a new one if it is missing), reduces it with
harness/reduce.py, and checks that the reduced case still fails under the mutant and has
at most 3 instructions. Prints one line per mutant; exits with 0 only if all comply.

    python3 scripts/reduce_check.py
"""

import json
import sys

import common
import mutants

sys.path.insert(0, str(common.ROOT / "harness"))
import gen  # noqa: E402
import reduce  # noqa: E402

MAX_INSNS = 3


def first_failure(feature, binary):
    """Index of the first case with a discrepancy under the mutant, or None."""
    path = mutants.compare_dir(feature) / "discrepancies.json"
    if not path.exists():
        mutants.random_compare(feature, binary)
    if not path.exists():
        return None
    cases = [entry["case"] for entry in json.loads(path.read_text())]
    return min(cases) if cases else None


def check(feature):
    """(whether the mutant complies, line to print)."""
    binary = mutants.build_mutant(feature)
    index = first_failure(feature, binary)
    if index is None:
        return False, f"{feature}: sin discrepancia que reducir"
    work = mutants.OUT / mutants.name(feature) / "reduce"
    case = gen.generate(index)
    reduced, oracle = reduce.reduce(case, binary, work)
    if reduced is None:
        return False, f"{feature}: el caso {index} no falla solo"
    # A fresh oracle, so the final verdict does not come from the cache.
    still = reduce.Oracle(reduced, binary, work)(reduced.items)
    n = len(reduced.items)
    ok = still and n <= MAX_INSNS
    mnemonics = " ".join(item.mnemonic for item in reduced.items) or "(vacío)"
    verdict = "sigue fallando" if still else "ya no falla"
    return ok, (f"{feature}: caso {index}, {len(case.items)} -> {n} instrucciones "
                f"[{mnemonics}], {verdict}, {oracle.runs} corridas")


def main():
    common.require_non_root()
    results = [check(feature) for feature, _ in mutants.load()]
    for _, line in results:
        print(line)
    good = sum(ok for ok, _ in results)
    print(f"reducciones correctas: {good}/{len(results)}")
    sys.exit(0 if results and good == len(results) else 1)


if __name__ == "__main__":
    main()
