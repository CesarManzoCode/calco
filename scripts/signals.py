"""Signals (contract 6): guests/signals.c, static non-PIE, under calco gives the same exit
status, stdout and stderr as natively. Prints `senales-igual-a-nativo`."""

import common


def main():
    common.require_non_root()
    calco = common.build_calco()
    guest = common.compile_guest("signals")

    native = common.run([guest])
    res = common.run_calco(calco, guest)
    if not common.same_as_native(native, res):
        common.fail("calco differs from native")
    print("senales-igual-a-nativo")


if __name__ == "__main__":
    main()
