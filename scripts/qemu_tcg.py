"""QEMU tests/tcg (contract 6 and D22): fetches the pinned sources with fetch.py, compiles
one test with the QEMU flags of vault/RESEARCH.md (without -Werror) into
build/qemu-tcg/<name>, runs it natively and under calco, keeps the raw output of each run
in build/qemu-tcg/<name>.{native,calco}.out and exits 0 only if both runs are equal.

    python3 scripts/qemu_tcg.py {test-x86_64|sha512-sse|linux-test|testthread} [--calco BIN]

"Equal" is the same exit code (or death signal), the same stderr and the same stdout byte
by byte; for testthread, stdout is compared as D22 says: the sequence of lines of each
`threadN:` prefix, the sequence of the remaining lines and the multiset of all lines.

Each .out file holds the raw stdout, then a `--- stderr ---` line and the raw stderr, then
a `--- exit N ---` (or `--- signal N ---`) line.
"""

import argparse
import re
import sys
from collections import Counter
from dataclasses import dataclass
from pathlib import Path

import common
import fetch

OUT = common.BUILD / "qemu-tcg"
# tests/tcg/Makefile.target: CFLAGS and LDFLAGS of every test, -Werror left out.
CFLAGS = ("-Wall", "-O0", "-g", "-fno-strict-aliasing")
LDFLAGS = ("-static",)
TIMEOUT = 600


@dataclass(frozen=True)
class Test:
    # Path under build/third_party/qemu-tests/ (that is, under tests/tcg/).
    source: str
    cflags: tuple
    # Libraries the per-target Makefile adds after LDFLAGS.
    libs: tuple = ()
    threads: bool = False


TESTS = {
    # x86_64/Makefile.target: test-x86_64 from i386/test-i386.c, LDFLAGS += -lm -lc.
    "test-x86_64": Test("i386/test-i386.c", CFLAGS, ("-lm", "-lc")),
    # i386/Makefile.target: `sha512-sse: CFLAGS=-msse4.1 -O3` replaces the CFLAGS.
    "sha512-sse": Test("multiarch/sha512.c", ("-msse4.1", "-O3")),
    # multiarch/Makefile.target links the thread tests with -lpthread.
    "linux-test": Test("multiarch/linux/linux-test.c", CFLAGS, ("-lpthread",)),
    "testthread": Test("multiarch/testthread.c", CFLAGS, ("-lpthread",), threads=True),
}

THREAD = re.compile(rb"^(thread\d+):")


def compile_command(name, sources, out):
    """gcc line of test `name` as tests/tcg builds it: `$(CC) $(CFLAGS) $< -o $@ $(LDFLAGS)`."""
    test = TESTS[name]
    return ["gcc", *test.cflags, str(Path(sources) / test.source), "-o", str(out),
            *LDFLAGS, *test.libs]


def by_thread(stdout):
    """Lines of `stdout` grouped by their `threadN:` prefix, in order; the lines without
    a prefix go under None."""
    groups = {}
    for line in stdout.splitlines(keepends=True):
        m = THREAD.match(line)
        groups.setdefault(m[1].decode() if m else None, []).append(line)
    return groups


def compare_threads(native, calco):
    """Differences between two testthread stdouts under D22; empty if they are equal."""
    problems = []
    a, b = by_thread(native), by_thread(calco)
    for key in sorted(set(a) | set(b), key=lambda k: (k is None, k or "")):
        name = key or "lines without a thread prefix"
        if a.get(key, []) != b.get(key, []):
            problems.append(f"{name}: native {len(a.get(key, []))} lines, "
                            f"calco {len(b.get(key, []))} lines, sequences differ")
    if Counter(native.splitlines(keepends=True)) != Counter(calco.splitlines(keepends=True)):
        problems.append("multiset of lines differs")
    return problems


def compare(name, native, calco):
    """Differences between the native and the calco run of test `name`."""
    problems = []
    if native.returncode != calco.returncode:
        problems.append(f"native ended with {native.describe()}, calco with {calco.describe()}")
    if native.stderr != calco.stderr:
        problems.append("stderr differs")
    if TESTS[name].threads:
        problems += compare_threads(native.stdout, calco.stdout)
    elif native.stdout != calco.stdout:
        problems.append(f"stdout differs from line {first_difference(native.stdout, calco.stdout)}")
    return problems


def first_difference(a, b):
    la, lb = a.splitlines(), b.splitlines()
    for i, (x, y) in enumerate(zip(la, lb)):
        if x != y:
            return i + 1
    return min(len(la), len(lb)) + 1


def save(path, res):
    status = f"signal {-res.returncode}" if res.returncode < 0 else f"exit {res.returncode}"
    path.write_bytes(res.stdout + b"\n--- stderr ---\n" + res.stderr
                     + f"\n--- {status} ---\n".encode())


def build(name):
    sources = fetch.fetch("qemu-tests")
    OUT.mkdir(parents=True, exist_ok=True)
    out = OUT / name
    cmd = compile_command(name, sources, out)
    print(" ".join(cmd))
    res = common.run(cmd, timeout=TIMEOUT)
    if res.returncode != 0:
        common.fail(f"cannot compile {name}:\n{res.stderr.decode(errors='replace')}")
    return out


def main():
    common.require_non_root()
    parser = argparse.ArgumentParser()
    parser.add_argument("name", choices=sorted(TESTS))
    parser.add_argument("--calco", type=Path)
    args = parser.parse_args()

    calco = args.calco or common.build_calco()
    binary = build(args.name)
    native = common.run([binary], timeout=TIMEOUT)
    under = common.run_calco(calco, binary, timeout=TIMEOUT)
    save(OUT / f"{args.name}.native.out", native)
    save(OUT / f"{args.name}.calco.out", under)

    problems = compare(args.name, native, under)
    if problems:
        print("\n".join(problems))
        common.fail(f"{args.name}: calco differs from native (outputs in {OUT})")
    print(f"{args.name}: igual a nativo")


if __name__ == "__main__":
    sys.exit(main())
