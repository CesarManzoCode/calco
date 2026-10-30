"""Runs a test ELF natively, under calco or under qemu-x86_64, with a time limit, and
returns its stdout (the dump records) and how it ended."""

import subprocess
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
QEMU = ROOT / "build" / "tools" / "qemu-x86_64-static"
TIMEOUT = 120
MODES = ("native", "calco", "qemu")


@dataclass
class Run:
    mode: str
    stdout: bytes
    stderr: bytes
    # Like subprocess: negative is death by that signal; None is a timeout.
    returncode: int

    def ok(self):
        return self.returncode == 0

    def describe(self):
        if self.returncode is None:
            return "timeout"
        if self.returncode < 0:
            return f"signal {-self.returncode}"
        return f"exit {self.returncode}"


def command(elf, mode, calco=None, qemu=QEMU):
    if mode == "native":
        return [str(elf)]
    if mode == "calco":
        return [str(calco), str(elf)]
    if mode == "qemu":
        return [str(qemu), str(elf)]
    raise ValueError(f"unknown mode {mode}")


def run(elf, mode, calco=None, qemu=QEMU, timeout=TIMEOUT, out=None):
    """Runs `elf` in `mode`. If `out` is given, stdout is also written there."""
    cmd = command(elf, mode, calco, qemu)
    try:
        proc = subprocess.run(cmd, capture_output=True, timeout=timeout, stdin=subprocess.DEVNULL)
        result = Run(mode, proc.stdout, proc.stderr, proc.returncode)
    except subprocess.TimeoutExpired as e:
        result = Run(mode, e.stdout or b"", e.stderr or b"", None)
    if out is not None:
        Path(out).parent.mkdir(parents=True, exist_ok=True)
        Path(out).write_bytes(result.stdout)
    return result
