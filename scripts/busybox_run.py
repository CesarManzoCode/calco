"""busybox cases (contract 6): runs each case of targets/busybox/cases.tsv natively and
under calco with build/targets/busybox and compares them. Exits 0 only if every case is
equal to native: same exit code (or death signal), same stdout byte by byte, same stderr.

    python3 scripts/busybox_run.py [--calco BIN]

cases.tsv has one case per line, tab separated, `stdin<TAB>applet<TAB>arg1<TAB>arg2…`,
where `stdin` is a file of targets/busybox/inputs/ or `-` (empty stdin); `#` lines are
comments. Each case runs `busybox applet args…` with inputs/ as its current directory and
a fixed environment. The raw output of each differing case is kept in
build/busybox-run/<line>.{native,calco}.out.
"""

import argparse
import signal
from dataclasses import dataclass
from pathlib import Path

import common
from build_targets import build_busybox

TARGET = common.ROOT / "targets" / "busybox"
CASES = TARGET / "cases.tsv"
INPUTS = TARGET / "inputs"
BUSYBOX = common.BUILD / "targets" / "busybox"
OUT = common.BUILD / "busybox-run"
# Same environment natively and under calco; nothing from the user's one.
ENV = {"PATH": "/usr/bin:/bin", "LC_ALL": "C", "TZ": "UTC"}
TIMEOUT = 300


@dataclass(frozen=True)
class Case:
    # Line number in cases.tsv: the name of the case.
    line: int
    # File name in inputs/, or None for empty stdin.
    stdin: str | None
    applet: str
    args: tuple

    def describe(self):
        return f"line {self.line}: {' '.join((self.applet, *self.args))}"


def parse_cases(text, inputs=INPUTS):
    """The cases of a cases.tsv text. Fails on a line with fewer than two columns, an empty
    applet or a stdin that is not a file of `inputs`."""
    cases = []
    for number, line in enumerate(text.splitlines(), 1):
        if not line or line.startswith("#"):
            continue
        cols = line.split("\t")
        if len(cols) < 2 or not cols[1]:
            raise ValueError(f"line {number}: want stdin<TAB>applet[<TAB>arg…], got {line!r}")
        stdin = None if cols[0] == "-" else cols[0]
        if stdin is not None and ("/" in stdin or not (Path(inputs) / stdin).is_file()):
            raise ValueError(f"line {number}: stdin {stdin!r} is not a file of {inputs}")
        cases.append(Case(number, stdin, cols[1], tuple(cols[2:])))
    return cases


def read_cases(path=CASES, inputs=INPUTS):
    try:
        return parse_cases(Path(path).read_text(), inputs)
    except ValueError as e:
        common.fail(f"{path}: {e}")


def run_case(case, prefix=()):
    """Runs `case` with `prefix` before `busybox` (calco, or nothing for native)."""
    stdin = (INPUTS / case.stdin).read_bytes() if case.stdin else b""
    return common.run([*prefix, BUSYBOX, case.applet, *case.args], stdin=stdin, env=ENV,
                      timeout=TIMEOUT, cwd=INPUTS)


def differences(native, calco):
    problems = []
    if native.returncode != calco.returncode:
        problems.append(f"native ended with {native.describe()}, calco with {calco.describe()}")
    if native.stdout != calco.stdout:
        problems.append("stdout differs")
    if native.stderr != calco.stderr:
        problems.append("stderr differs")
    return problems


def save(path, res):
    path.write_bytes(res.stdout + b"\n--- stderr ---\n" + res.stderr
                     + f"\n--- {res.describe()} ---\n".encode())


def main():
    common.require_non_root()
    parser = argparse.ArgumentParser()
    parser.add_argument("--calco", type=Path)
    args = parser.parse_args()

    cases = read_cases()
    calco = args.calco or common.build_calco()
    if not BUSYBOX.exists():
        build_busybox()
    OUT.mkdir(parents=True, exist_ok=True)
    for old in OUT.glob("*.out"):
        old.unlink()
    failed = 0
    for case in cases:
        native = run_case(case)
        under = run_case(case, prefix=(calco,))
        problems = differences(native, under)
        if problems:
            failed += 1
            save(OUT / f"{case.line}.native.out", native)
            save(OUT / f"{case.line}.calco.out", under)
            print(f"FAIL {case.describe()}: {'; '.join(problems)}")
    applets = len({c.applet for c in cases})
    print(f"busybox: {len(cases) - failed}/{len(cases)} casos iguales a nativo ({applets} applets)")
    if failed:
        common.fail(f"{failed} cases differ from native; outputs in {OUT}")


if __name__ == "__main__":
    signal.signal(signal.SIGPIPE, signal.SIG_DFL)
    main()
