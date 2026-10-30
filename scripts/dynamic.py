"""Dynamic glibc programs (contract 6): guests/hello.c built dynamic with gcc's default PIE
gives under calco the same exit status, stdout and stderr as natively, and with
guests/pause.c built the same way waiting on stdin, no region of the executable, of
ld-linux-x86-64.so.2 or of libc.so.6 in calco's /proc/PID/maps is executable. Prints
`dinamico-igual-a-nativo` and `codigo-invitado-no-ejecutable`."""

import os
import signal
import subprocess
import threading

import common
from noexec import parse_maps

LIBRARIES = ("ld-linux-x86-64.so.2", "libc.so.6")


def compare_hello(calco):
    guest = common.compile_guest("hello", static=False)
    args = ["one", "two words", ""]
    env = dict(os.environ, CALCO_HELLO="dynamic")
    native = common.run([guest, *args], env=env)
    if native.returncode != 3:
        common.fail(f"native hello ended with {native.describe()}")
    res = common.run_calco(calco, guest, args, env=env)
    ok = True
    for what in ("stdout", "stderr"):
        if getattr(res, what) != getattr(native, what):
            print(f"{what}: {getattr(res, what)!r}, native {getattr(native, what)!r}")
            ok = False
    if res.returncode != native.returncode:
        print(f"ended with {res.describe()}, native {native.describe()}")
        ok = False
    if not ok:
        common.fail("dynamic hello under calco differs from native")
    print("dinamico-igual-a-nativo")


def paused_maps(calco, guest):
    """calco's /proc/PID/maps while the dynamic pause guest waits on stdin."""
    proc = subprocess.Popen(
        [str(calco), str(guest)],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    watchdog = threading.Timer(60, proc.kill)
    watchdog.start()
    try:
        line = proc.stdout.readline()
        if line != b"ready\n":
            proc.kill()
            proc.wait()
            common.fail(f"pause printed {line!r} instead of ready: {proc.stderr.read()!r}")
        with open(f"/proc/{proc.pid}/maps") as f:
            regions = parse_maps(f.read())
        proc.stdin.write(b"\n")
        proc.stdin.close()
        proc.stdout.read()
        code = proc.wait()
    finally:
        watchdog.cancel()
    if code != 0:
        common.fail(f"pause under calco ended with {code}: {proc.stderr.read()!r}")
    return regions


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
    executable = [r for rs in found.values() for r in rs if "x" in r[2]]
    for start, end, perms, path in executable:
        print(f"{start:x}-{end:x} {perms} {path}")
    if executable:
        common.fail(f"{len(executable)} guest regions are executable")
    print("codigo-invitado-no-ejecutable")


def main():
    common.require_non_root()
    calco = common.build_calco()
    compare_hello(calco)
    check_noexec(calco)


if __name__ == "__main__":
    signal.signal(signal.SIGPIPE, signal.SIG_DFL)
    main()
