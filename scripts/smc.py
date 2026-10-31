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
    if not common.same_as_native(native, res):
        common.fail("calco differs from native")
    print("smc-igual-a-nativo")


if __name__ == "__main__":
    main()
