"""busybox's own test suite (contract 6, D18): runs `testsuite/runtest` of busybox 1.36.1
twice natively and once with the binary under calco, and compares the number of `PASS:`
inside the valid subset. Exits 0 only if every test of the subset passes under calco.

    python3 scripts/busybox_suite.py [--calco BIN] [--timeout SECONDS]

The valid subset is measured on every run, not guessed: the tests that pass in both
native runs as this non-root user with SKIP_INTERNET_TESTS=1. It is written to
build/busybox-suite/suite.list (one test name per line, repeated as many times as it
passes), with the raw logs next to it: native.log and native.2.log (the two native runs)
and calco.log.

Each run gets a fresh copy of the source testsuite/ under build/busybox-suite/<run>/ and a
bin/ directory with busybox's .config and a `busybox`: the static binary for the native
runs, and for calco a wrapper `exec calco --argv0 "$0" busybox "$@"`. runtest fills
bin/runtest-tempdir-links/ with one link per applet to that bin/busybox, so under calco
every applet runs translated with its name as argv[0].
"""

import argparse
import os
import shlex
import shutil
import signal
import subprocess
from collections import Counter
from pathlib import Path

import common
from build_targets import build_busybox, SRC

BUSYBOX = common.BUILD / "targets" / "busybox"
OUT = common.BUILD / "busybox-suite"
TIMEOUT = 7200


def source_tree():
    """The busybox source tree build_targets.py built: its testsuite/ and .config."""
    tops = sorted((SRC / "busybox").glob("busybox-*")) if (SRC / "busybox").exists() else []
    if not BUSYBOX.exists() or not tops or not (tops[0] / ".config").exists():
        build_busybox()
        tops = sorted((SRC / "busybox").glob("busybox-*"))
    return tops[0]


def results(log):
    """Test name -> (times it printed `PASS:`, times it printed `FAIL:`) in a runtest log."""
    passes, fails = Counter(), Counter()
    for line in log.splitlines():
        if line.startswith("PASS: "):
            passes[line[6:].rstrip()] += 1
        elif line.startswith("FAIL: "):
            fails[line[6:].rstrip()] += 1
    return {name: (passes[name], fails[name]) for name in passes | fails}


def valid_subset(first, second):
    """The tests that pass in both native logs and never fail in either, each counted the
    smaller number of times it passed."""
    a, b = results(first), results(second)
    subset = Counter()
    for name in a.keys() & b.keys():
        (pa, fa), (pb, fb) = a[name], b[name]
        if fa == 0 and fb == 0 and min(pa, pb) > 0:
            subset[name] = min(pa, pb)
    return subset


def passed_in(log, subset):
    """How many `PASS:` of `log` fall inside `subset` (each test up to its count there)."""
    got = results(log)
    return sum(min(got.get(name, (0, 0))[0], n) for name, n in subset.items())


def prepare(run, tree, calco=None):
    """Fresh build/busybox-suite/<run>/ with testsuite/ and bin/; returns the testsuite dir."""
    base = OUT / run
    if base.exists():
        shutil.rmtree(base)
    tests = base / "testsuite"
    shutil.copytree(tree / "testsuite", tests, symlinks=True)
    bindir = base / "bin"
    bindir.mkdir()
    shutil.copy2(tree / ".config", bindir / ".config")
    busybox = bindir / "busybox"
    if calco is None:
        shutil.copy2(BUSYBOX, busybox)
    else:
        busybox.write_text("#!/bin/sh\n"
                           f'exec {shlex.quote(str(calco))} --argv0 "$0" '
                           f'{shlex.quote(str(BUSYBOX))} "$@"\n')
        busybox.chmod(0o755)
    return tests


def run_suite(run, tree, log, timeout, calco=None):
    """Runs runtest in a fresh tree with stdout and stderr into `log`; returns the log text.
    On timeout the whole process group is killed and the log keeps what it had."""
    tests = prepare(run, tree, calco)
    env = dict(os.environ, bindir=str(tests.parent / "bin"), srcdir=str(tests),
               LC_ALL="C", TZ="UTC", SKIP_INTERNET_TESTS="1")
    env.pop("VERBOSE", None)
    print(f"{run}: runtest in {tests}", flush=True)
    with open(log, "wb") as out:
        proc = subprocess.Popen([str(tests / "runtest")], cwd=tests, env=env,
                                stdin=subprocess.DEVNULL, stdout=out,
                                stderr=subprocess.STDOUT, start_new_session=True)
        try:
            proc.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            os.killpg(proc.pid, signal.SIGKILL)
            proc.wait()
            out.write(f"\n--- timeout after {timeout} s ---\n".encode())
    return log.read_text(errors="replace")


def main():
    common.require_non_root()
    parser = argparse.ArgumentParser()
    parser.add_argument("--calco", type=Path)
    parser.add_argument("--timeout", type=int, default=TIMEOUT)
    args = parser.parse_args()

    calco = (args.calco or common.build_calco()).resolve()
    tree = source_tree()
    OUT.mkdir(parents=True, exist_ok=True)
    first = run_suite("native", tree, OUT / "native.log", args.timeout)
    second = run_suite("native.2", tree, OUT / "native.2.log", args.timeout)
    subset = valid_subset(first, second)
    (OUT / "suite.list").write_text(
        "".join(f"{name}\n" * n for name, n in sorted(subset.items())))
    under = run_suite("calco", tree, OUT / "calco.log", args.timeout, calco=calco)

    total = sum(subset.values())
    passed = passed_in(under, subset)
    print(f"subconjunto: {total} pruebas (de {sum(p for p, _ in results(first).values())} "
          f"PASS en la primera corrida nativa)")
    print(f"busybox-suite: {passed}/{total} PASS")
    got = results(under)
    for name in sorted(subset):
        if got.get(name, (0, 0))[0] < subset[name]:
            print(f"FAIL {name}")
    if total == 0:
        common.fail("empty subset: no test passed in both native runs")
    if passed != total:
        common.fail(f"{total - passed} tests of the subset do not pass under calco")


if __name__ == "__main__":
    main()
