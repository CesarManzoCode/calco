"""TCC (contract 6): build/targets/tcc0 compiles TCC's own tcc.c natively and under calco
and leaves build/tcc/tcc1.native and build/tcc/tcc1.calco. Exits 0 only if both
compilations end with 0 and both outputs have the same sha256.

    python3 scripts/tcc_selfcompile.py [--calco BIN]

The command line is the one TCC's Makefile uses to build tcc.c with TCC itself (rule
`tcc_c`, `$(TCC) tcc.c -o $@ -ftest-coverage $(DEFINES) $(LIBS)`), as `make -n tcc_c`
expands it in the configured tree build/targets/src/tcc/, with `./tcc` replaced by tcc0:

    ./tcc -B. -I./include -I. -I. tcc.c -o tcc_c -ftest-coverage -I. -lm -ldl -lpthread

It runs with that tree as the current directory. Both runs write to the same path,
build/tcc/tcc1, which is then renamed, so the output name cannot make the two differ.
"""

import argparse
import hashlib
import signal
from pathlib import Path

import common
from build_targets import OUT as TARGETS, SRC, build_tcc

TCC0 = TARGETS / "tcc0"
TREE = SRC / "tcc"
OUT = common.BUILD / "tcc"
ENV = {"PATH": "/usr/bin:/bin", "LC_ALL": "C", "TZ": "UTC"}
TIMEOUT = 1800


def compile_line(output):
    """The self-compilation of TCC's Makefile (rule tcc_c) with `output` after -o."""
    return [TCC0, "-B.", "-I./include", "-I.", "-I.", "tcc.c", "-o", output,
            "-ftest-coverage", "-I.", "-lm", "-ldl", "-lpthread"]


def selfcompile(dest, prefix=()):
    """Runs the self-compilation with `prefix` (calco, or nothing) before and moves the
    result to `dest`. Returns the run's Result."""
    work = OUT / "tcc1"
    for path in (work, dest):
        path.unlink(missing_ok=True)
    res = common.run([*prefix, *compile_line(work)], env=ENV, timeout=TIMEOUT, cwd=TREE)
    if work.exists():
        work.rename(dest)
    return res


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest() if path.exists() else "missing"


def save(path, res):
    path.write_bytes(res.stdout + b"\n--- stderr ---\n" + res.stderr
                     + f"\n--- {res.describe()} ---\n".encode())


def main():
    common.require_non_root()
    parser = argparse.ArgumentParser()
    parser.add_argument("--calco", type=Path)
    args = parser.parse_args()

    calco = args.calco or common.build_calco()
    if not TCC0.exists() or not (TREE / "tcc.c").exists():
        build_tcc()
    OUT.mkdir(parents=True, exist_ok=True)

    native_bin, calco_bin = OUT / "tcc1.native", OUT / "tcc1.calco"
    native = selfcompile(native_bin)
    save(OUT / "native.out", native)
    if native.returncode != 0 or not native_bin.exists():
        common.fail(f"native tcc0 ended with {native.describe()}; see {OUT / 'native.out'}")
    under = selfcompile(calco_bin, prefix=(calco,))
    save(OUT / "calco.out", under)

    native_hash, calco_hash = sha256(native_bin), sha256(calco_bin)
    problems = []
    if under.returncode != 0:
        problems.append(f"tcc0 under calco ended with {under.describe()}")
    if native_hash != calco_hash:
        problems.append(f"sha256 differs: native {native_hash}, calco {calco_hash}")
    if problems:
        common.fail(f"{'; '.join(problems)}; outputs in {OUT}")
    print(f"tcc: tcc1.native y tcc1.calco iguales (sha256 {native_hash}, "
          f"{native_bin.stat().st_size} bytes)")


if __name__ == "__main__":
    signal.signal(signal.SIGPIPE, signal.SIG_DFL)
    main()
