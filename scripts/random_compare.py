"""N random sequences, 0 discrepancies (contract 6): cases 0…N-1 in batches of 500, each
batch one test ELF run natively, under calco and under qemu-x86_64. Native is the
oracle; calco must match it under the case mask. Prints `discrepancias: <n>` and the
informative `qemu-x86_64 discrepancias: <n>`; with `--strict`, also how many undefined
flags matched. Dumps go to <out>/{native,calco,qemu}/ and the discrepancies, for the
reducer, to <out>/discrepancies.json.

    python3 scripts/random_compare.py [--n N] [--calco BIN] [--strict] [--out DIR]
"""

import argparse
import json
import os
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import common

sys.path.insert(0, str(common.ROOT / "harness"))
import build  # noqa: E402
import compare  # noqa: E402
import dump  # noqa: E402
import run  # noqa: E402


def records(result):
    """{case index: record} of a run; the complete records written before any failure."""
    data = result.stdout[: len(result.stdout) - len(result.stdout) % dump.SIZE]
    try:
        return {rec.index: rec for rec in dump.split(data)}
    except dump.DumpError:
        return {}


def batch(number, first, count, args, calco, qemu):
    out = Path(args.out)
    cases = build.batch(first, count)
    elf = build.build(cases, out / "elf", f"batch_{number:04d}")
    name = f"batch_{number:04d}.bin"
    native = run.run(elf, "native", out=out / "native" / name)
    if not native.ok() or len(records(native)) != count:
        common.fail(f"{elf} natively: {native.describe()}, {len(records(native))}/{count} "
                    f"records: {native.stderr[-500:]!r}")
    expected = records(native)
    runs = {"calco": run.run(elf, "calco", calco=calco, out=out / "calco" / name)}
    if qemu:
        runs["qemu"] = run.run(elf, "qemu", qemu=qemu, out=out / "qemu" / name)
    found = {mode: [] for mode in runs}
    strict = [0, 0]
    for mode, result in runs.items():
        got = records(result)
        for case in cases:
            want = expected[case.index]
            if case.index not in got:
                found[mode].append((case.index, "missing", "record", result.describe()))
                continue
            for field, a, b in compare.compare(want, got[case.index], case.undefined):
                found[mode].append((case.index, field, a, b))
            if mode == "calco" and args.strict:
                strict[0] += compare.undefined_matches(want, got[case.index], case.undefined)
                strict[1] += len(case.undefined)
    return found, strict


def find_qemu():
    try:
        import fetch

        return fetch.fetch("qemu-user")
    except SystemExit:
        return None


def main():
    common.require_non_root()
    p = argparse.ArgumentParser()
    p.add_argument("--n", type=int, default=100_000)
    p.add_argument("--calco", type=Path)
    p.add_argument("--strict", action="store_true")
    p.add_argument("--out", type=Path, default=common.BUILD / "random")
    args = p.parse_args()
    calco = args.calco.resolve() if args.calco else common.build_calco()
    qemu = find_qemu()

    jobs = []
    with ThreadPoolExecutor(max_workers=os.cpu_count() or 1) as pool:
        for number, first in enumerate(range(0, args.n, build.BATCH)):
            count = min(build.BATCH, args.n - first)
            jobs.append(pool.submit(batch, number, first, count, args, calco, qemu))
        results = [job.result() for job in jobs]

    calco_found = [d for found, _ in results for d in found["calco"]]
    qemu_found = [d for found, _ in results for d in found.get("qemu", [])]
    entries = [{"case": c, "field": f, "native": a, "calco": b} for c, f, a, b in calco_found]
    args.out.mkdir(parents=True, exist_ok=True)
    (args.out / "discrepancies.json").write_text(json.dumps(entries, indent=1) + "\n")
    for c, f, a, b in calco_found[:20]:
        print(f"caso {c}: {f} nativo={a} calco={b}")
    failing = len({d[0] for d in calco_found})
    print(f"discrepancias: {failing}")
    if qemu:
        print(f"qemu-x86_64 discrepancias: {len({d[0] for d in qemu_found})}")
    else:
        print("qemu-x86_64 discrepancias: sin qemu")
    if args.strict:
        same = sum(s[0] for _, s in results)
        total = sum(s[1] for _, s in results)
        print(f"estricto: flags indefinidas iguales {same}/{total}")
    sys.exit(0 if failing == 0 else 1)


if __name__ == "__main__":
    main()
