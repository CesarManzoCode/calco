"""CPUID without AVX, AVX gives SIGILL (contract 6): guests/avx.c under calco prints the
CPUID bits exactly as guests/avx.expected and dies by SIGILL on vaddps."""

import signal

import common


def main():
    common.require_non_root()
    calco = common.build_calco()
    guest = common.compile_guest("avx")
    expected = (common.GUESTS / "avx.expected").read_bytes()

    res = common.run_calco(calco, guest)
    ok = True
    if res.stdout != expected:
        print(f"stdout: {res.stdout!r}, expected {expected!r}")
        ok = False
    if res.returncode != -signal.SIGILL:
        print(f"ended with {res.describe()}, expected signal {int(signal.SIGILL)}")
        ok = False
    if not ok:
        common.fail(f"stderr: {res.stderr!r}")
    print("cpuid-sin-avx-sigill")


if __name__ == "__main__":
    main()
