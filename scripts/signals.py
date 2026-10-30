"""Signals (contract 6): guests/signals.c, static non-PIE, under calco gives the same exit
status, stdout and stderr as natively. Prints `senales-igual-a-nativo`."""

import common


def main():
    common.require_non_root()
    calco = common.build_calco()
    guest = common.compile_guest("signals")

    native = common.run([guest])
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
        common.fail("calco differs from native")
    print("senales-igual-a-nativo")


if __name__ == "__main__":
    main()
