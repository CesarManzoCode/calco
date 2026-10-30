"""Guest code is not executable (contract 6): runs guests/pause.c under calco, reads
calco's /proc/PID/maps while the guest waits on stdin, and checks that no guest region
has execute permission. The only executable mapping in the low half the guest uses is
calco's own code cache."""

import signal
import subprocess
import threading

import common

# Guest memory lies below the top of its main stack (contract 3.1); calco, its
# libraries and its heap are placed by the kernel far above.
GUEST_TOP = 0x0000_4000_0000_0000
CODE_CACHE_SIZE = 256 << 20


def parse_maps(text):
    regions = []
    for line in text.splitlines():
        fields = line.split(maxsplit=5)
        start, end = (int(x, 16) for x in fields[0].split("-"))
        path = fields[5] if len(fields) > 5 else ""
        regions.append((start, end, fields[1], path))
    return regions


def main():
    common.require_non_root()
    calco = common.build_calco()
    guest = common.compile_guest("pause")

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
            common.fail(f"pause printed {line!r} instead of ready ({proc.returncode})")
        maps = f"/proc/{proc.pid}/maps"
        with open(maps) as f:
            regions = parse_maps(f.read())
        proc.stdin.write(b"\n")
        proc.stdin.close()
        proc.stdout.read()
        code = proc.wait()
    finally:
        watchdog.cancel()
    if code != 0:
        common.fail(f"pause under calco ended with {code}: {proc.stderr.read()!r}")

    low = [r for r in regions if r[0] < GUEST_TOP]
    cache = [r for r in low if r[1] - r[0] == CODE_CACHE_SIZE and r[2] == "rwxp" and not r[3]]
    if len(cache) != 1:
        common.fail(f"expected one code cache mapping, found {len(cache)}")
    guest_regions = [r for r in low if r is not cache[0]]
    if not any(r[3] == str(guest) for r in guest_regions):
        common.fail(f"no mapping of {guest} in {maps}")
    executable = [r for r in guest_regions if "x" in r[2]]
    for start, end, perms, path in executable:
        print(f"{start:x}-{end:x} {perms} {path}")
    if executable:
        common.fail(f"{len(executable)} guest regions are executable")
    print("codigo-invitado-no-ejecutable")


if __name__ == "__main__":
    signal.signal(signal.SIGPIPE, signal.SIG_DFL)
    main()
