"""Shared helpers of the verification scripts (contract section 6): no root, building
calco, compiling the guests and running programs with their output captured."""

import os
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BUILD = ROOT / "build"
GUESTS = ROOT / "guests"
GUEST_BUILD = BUILD / "guests"
CALCO = ROOT / "target" / "release" / "calco"


def require_non_root():
    """Fails if the script runs as root: everything in calco runs as a normal user."""
    if os.getuid() == 0:
        fail("must not run as root")


def fail(message):
    print(f"{Path(sys.argv[0]).name}: {message}", file=sys.stderr)
    sys.exit(1)


def build_calco():
    """Builds calco with `cargo build --release` (cargo rebuilds only if needed) and
    returns the path of the binary."""
    proc = subprocess.run(["cargo", "build", "--release"], cwd=ROOT)
    if proc.returncode != 0:
        fail("cargo build --release failed")
    return CALCO


def compile_guest(name, static=True, extra=()):
    """Compiles guests/<name>.c with gcc as D21 says (`-O2 -static`, non-PIE; dynamic
    PIE with `static=False`) into build/guests/ and returns the binary path."""
    src = GUESTS / f"{name}.c"
    out = GUEST_BUILD / (name if static else f"{name}-dynamic")
    GUEST_BUILD.mkdir(parents=True, exist_ok=True)
    cmd = ["gcc", "-O2"]
    if static:
        cmd.append("-static")
    cmd += [*extra, "-o", str(out), str(src)]
    proc = subprocess.run(cmd, capture_output=True, text=True)
    if proc.returncode != 0:
        fail(f"cannot compile {src}:\n{proc.stderr}")
    return out


@dataclass
class Result:
    stdout: bytes
    stderr: bytes
    # Like subprocess: negative is death by that signal.
    returncode: int

    def describe(self):
        if self.returncode < 0:
            return f"signal {-self.returncode}"
        return f"exit {self.returncode}"


def run(cmd, stdin=b"", env=None, timeout=120):
    """Runs `cmd` and returns its stdout, stderr and exit code."""
    proc = subprocess.run(
        [str(c) for c in cmd], input=stdin, capture_output=True, env=env, timeout=timeout
    )
    return Result(proc.stdout, proc.stderr, proc.returncode)


def run_calco(calco, program, args=(), **kwargs):
    """Runs `program` under calco."""
    return run([calco, program, *args], **kwargs)
