"""Self-modifying code (contract 3.13): guests/smc.c, static non-PIE, under calco gives the
same exit status, stdout and stderr as natively. Prints `smc-igual-a-nativo`."""

import common


def main():
    common.require_non_root()
    calco = common.build_calco()
    guest = common.compile_guest("smc")

    native = common.run([guest])
    if native.returncode != 0:
        common.fail(f"native run ended with {native.describe()}")
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
    print("smc-igual-a-nativo")


if __name__ == "__main__":
    main()
