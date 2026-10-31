"""Dynamic glibc programs (contract 6): guests/hello.c built dynamic with gcc's default PIE
gives under calco the same exit status, stdout and stderr as natively, and with
guests/pause.c built the same way waiting on stdin, no region of the executable, of
ld-linux-x86-64.so.2 or of libc.so.6 in calco's /proc/PID/maps is executable. Prints
`dinamico-igual-a-nativo` and `codigo-invitado-no-ejecutable`."""

import os
import signal

import common
from noexec import fail_if_executable, paused_maps

LIBRARIES = ("ld-linux-x86-64.so.2", "libc.so.6")


def compare_hello(calco):
    guest = common.compile_guest("hello", static=False)
    args = ["one", "two words", ""]
    env = dict(os.environ, CALCO_HELLO="dynamic")
    native = common.run([guest, *args], env=env)
    if native.returncode != 3:
        common.fail(f"native hello ended with {native.describe()}")
    res = common.run_calco(calco, guest, args, env=env)
    if not common.same_as_native(native, res):
        common.fail("dynamic hello under calco differs from native")
    print("dinamico-igual-a-nativo")


def check_noexec(calco):
    guest = common.compile_guest("pause", static=False)
    regions = paused_maps(calco, guest)
    names = {str(guest): "executable", **{lib: lib for lib in LIBRARIES}}
    found = {}
    for region in regions:
        path = region[3]
        key = path if path == str(guest) else os.path.basename(path)
        if key in names:
            found.setdefault(names[key], []).append(region)
    missing = [n for n in names.values() if n not in found]
    if missing:
        common.fail(f"not mapped by calco: {', '.join(missing)}")
    fail_if_executable([r for rs in found.values() for r in rs if "x" in r[2]])


def main():
    common.require_non_root()
    calco = common.build_calco()
    compare_hello(calco)
    check_noexec(calco)


if __name__ == "__main__":
    signal.signal(signal.SIGPIPE, signal.SIG_DFL)
    main()
