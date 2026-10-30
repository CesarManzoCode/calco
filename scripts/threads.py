"""Threads (contract 6): guests/threads.c, static with -pthread, under calco gives the same
exit status, stdout and stderr as natively in R = 50 runs in a row. Prints
`hilos-igual-a-nativo: 50/50`."""

import common

RUNS = 50


def main():
    common.require_non_root()
    calco = common.build_calco()
    guest = common.compile_guest("threads", extra=("-pthread",))

    native = common.run([guest])
    if native.returncode != 0:
        common.fail(f"native run ended with {native.describe()}")
    for i in range(1, RUNS + 1):
        res = common.run_calco(calco, guest)
        ok = True
        for what in ("stdout", "stderr"):
            if getattr(res, what) != getattr(native, what):
                print(f"{what}: {getattr(res, what)!r}, native {getattr(native, what)!r}")
                ok = False
        if res.returncode != native.returncode:
            print(f"ended with {res.describe()}, native {native.describe()}")
            ok = False
        if not ok:
            common.fail(f"run {i} of {RUNS} differs from native")
    print(f"hilos-igual-a-nativo: {RUNS}/{RUNS}")


if __name__ == "__main__":
    main()
