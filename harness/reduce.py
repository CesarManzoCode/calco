"""Reducer of counterexamples (contract 4.5).

A case is reduced over its items with ddmin, keeping its initial state fixed. An item is
one instruction together with its preparation lines (base registers, `div`/`idiv`
operands, string pointers), so the preparation of a `div` never leaves its instruction.
Then every immediate operand of an instruction is tried at 0. A candidate is kept only if
it is still a valid case (it reads only defined flags; its mask is recomputed) and there
is still a discrepancy between the native run and the given binary.

    python3 harness/reduce.py --calco BIN [--work DIR] CASE
"""

import argparse
import dataclasses
import re
import sys
from pathlib import Path

import build
import compare
import dump
import gen
import run

TIMEOUT = 20
NUMBER = re.compile(r"-?(?:0x[0-9a-f]+|\d+)$")


def ddmin(items, test):
    """1-minimal sublist of `items` (order kept) for which `test` holds, by ddmin
    (Zeller and Hildebrandt, 2002); `test(items)` is assumed to hold."""
    items, n = list(items), 2
    while len(items) >= 2:
        size = -(-len(items) // n)
        chunks = [items[k : k + size] for k in range(0, len(items), size)]
        for chunk in chunks:
            if test(chunk):
                items, n = chunk, 2
                break
        else:
            for k in range(len(chunks)):
                rest = [x for j, chunk in enumerate(chunks) if j != k for x in chunk]
                if len(chunks) > 2 and test(rest):
                    items, n = rest, max(n - 1, 2)
                    break
            else:
                if n >= len(items):
                    break
                n = min(2 * n, len(items))
    return items


def immediates(item):
    """Indexes of the immediate operands of the instruction line of `item` (its last
    line) that are not 0. Preparation lines are never touched."""
    if item.mnemonic in ("div", "idiv"):
        return []
    mnemonic, _, ops = item.lines[-1].partition(" ")
    if not ops or ":" in mnemonic:
        return []
    return [k for k, op in enumerate(ops.split(", ")) if NUMBER.match(op) and int(op, 0) != 0]


def with_zero(item, k):
    """`item` with immediate operand `k` of its instruction line set to 0."""
    mnemonic, _, ops = item.lines[-1].partition(" ")
    ops = ops.split(", ")
    ops[k] = "0"
    return dataclasses.replace(item, lines=item.lines[:-1] + [f"{mnemonic} {', '.join(ops)}"])


def simplify(items, test):
    """Sets immediates to 0, one at a time, while `test` holds."""
    items = list(items)
    for i in range(len(items)):
        for k in immediates(items[i]):
            candidate = items[:i] + [with_zero(items[i], k)] + items[i + 1 :]
            if test(candidate):
                items = candidate
    return items


class Oracle:
    """Says whether a list of items of `case` still shows a discrepancy between native
    and `calco`. Each candidate is built alone as a one-case test ELF in `work`."""

    def __init__(self, case, calco, work, rows=None):
        self.case = case
        self.calco = calco
        self.work = Path(work)
        self.rows = gen.load_scope() if rows is None else rows
        self.seen = {}
        self.runs = 0

    def candidate(self, items):
        """The case with `items` and its recomputed mask, or None if it would read an
        undefined flag."""
        ok, undefined = gen.sequence_flags(items, self.rows)
        if not ok:
            return None
        return dataclasses.replace(self.case, items=list(items), undefined=undefined)

    def __call__(self, items):
        key = tuple((item.id, tuple(item.lines)) for item in items)
        if key not in self.seen:
            self.seen[key] = self.fails(items)
        return self.seen[key]

    def fails(self, items):
        case = self.candidate(items)
        if case is None:
            return False
        self.runs += 1
        elf = build.build([case], self.work, "reduce")
        native = run.run(elf, "native", timeout=TIMEOUT)
        if not native.ok() or len(native.stdout) != dump.SIZE:
            return False
        want = dump.parse(native.stdout)
        got = run.run(elf, "calco", calco=self.calco, timeout=TIMEOUT)
        if len(got.stdout) < dump.SIZE:
            return True  # calco died or wrote no record
        try:
            rec = dump.parse(got.stdout[: dump.SIZE])
        except dump.DumpError:
            return True
        return bool(compare.compare(want, rec, case.undefined))


def reduce(case, calco, work, rows=None):
    """(reduced case, oracle) of a failing `case`, or (None, oracle) if `case` alone does
    not fail."""
    oracle = Oracle(case, calco, work, rows)
    if not oracle(case.items):
        return None, oracle
    items = simplify(ddmin(case.items, oracle), oracle)
    return oracle.candidate(items), oracle


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--calco", type=Path, required=True)
    p.add_argument("--work", type=Path, default=Path(__file__).resolve().parent.parent
                   / "build" / "reduce")
    p.add_argument("case", type=int)
    args = p.parse_args()
    reduced, oracle = reduce(gen.generate(args.case), args.calco.resolve(), args.work)
    if reduced is None:
        print(f"caso {args.case}: no falla solo", file=sys.stderr)
        sys.exit(1)
    print("\n".join(build.case_text(reduced)))
    print(f"# {len(reduced.items)} instrucciones, {oracle.runs} corridas")


if __name__ == "__main__":
    main()
