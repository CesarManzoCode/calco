"""Downloads a pinned third-party input (contract 5) to build/third_party/<name>/ and checks
its sha256 (or its commit). Idempotent: what is already there and verified is kept.

    python3 scripts/fetch.py NAME...

Pins live in scripts/pins.json. An `archive` with `extract` also gets that member
extracted with `bsdtar` (for qemu-user, build/tools/qemu-x86_64-static).
"""

import hashlib
import json
import shutil
import subprocess
import sys
import tempfile
import urllib.request
from pathlib import Path

import common

PINS = common.ROOT / "scripts" / "pins.json"
THIRD_PARTY = common.BUILD / "third_party"


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def download(urls, dest, digest):
    """Puts the file with sha256 `digest` at `dest`, trying each URL in order."""
    if dest.exists() and sha256(dest) == digest:
        return dest
    dest.parent.mkdir(parents=True, exist_ok=True)
    errors = []
    for url in urls:
        tmp = dest.with_name(dest.name + ".part")
        try:
            with urllib.request.urlopen(url, timeout=120) as resp, open(tmp, "wb") as f:
                shutil.copyfileobj(resp, f)
        except OSError as e:
            errors.append(f"{url}: {e}")
            continue
        got = sha256(tmp)
        if got == digest:
            tmp.replace(dest)
            return dest
        tmp.unlink()
        errors.append(f"{url}: sha256 {got}, expected {digest}")
    common.fail(f"cannot fetch {dest.name}:\n  " + "\n  ".join(errors))


def extract(archive, member, to):
    """Extracts `member` of `archive` with bsdtar into an empty directory and moves it to `to`."""
    to = common.ROOT / to
    if to.exists():
        return to
    to.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=archive.parent) as tmp:
        proc = subprocess.run(["bsdtar", "-xf", str(archive), "-C", tmp, member],
                              capture_output=True, text=True)
        if proc.returncode != 0:
            common.fail(f"bsdtar on {archive} failed: {proc.stderr}")
        got = Path(tmp) / member
        got.chmod(0o755)
        shutil.move(str(got), str(to))
    return to


def git(url, commit, dest):
    def head():
        proc = subprocess.run(["git", "-C", str(dest), "rev-parse", "HEAD"],
                              capture_output=True, text=True)
        return proc.stdout.strip() if proc.returncode == 0 else None

    if dest.exists() and head() == commit:
        return dest
    if dest.exists():
        shutil.rmtree(dest)
    for cmd in (["git", "clone", "--quiet", url, str(dest)],
                ["git", "-C", str(dest), "checkout", "--quiet", commit]):
        if subprocess.run(cmd).returncode != 0:
            common.fail(f"{' '.join(cmd)} failed")
    if head() != commit:
        common.fail(f"{dest}: HEAD is {head()}, expected {commit}")
    return dest


def fetch(name):
    """Fetches `name` and returns its main path: the extracted member, the directory of
    the files or of the clone, or the archive."""
    pins = json.loads(PINS.read_text())
    if name not in pins:
        common.fail(f"no pin named {name} (known: {', '.join(sorted(pins))})")
    pin = pins[name]
    base = THIRD_PARTY / name
    if pin["kind"] == "archive":
        archive = download(pin["urls"], base / pin["file"], pin["sha256"])
        if "extract" in pin:
            return extract(archive, pin["extract"]["member"], pin["extract"]["to"])
        return archive
    if pin["kind"] == "files":
        for path, digest in pin["files"].items():
            download([pin["url"].format(commit=pin["commit"], path=path)], base / path, digest)
        return base
    if pin["kind"] == "git":
        return git(pin["url"], pin["commit"], base / "src")
    common.fail(f"pin {name}: unknown kind {pin['kind']}")


def main():
    common.require_non_root()
    if len(sys.argv) < 2:
        common.fail("usage: fetch.py NAME...")
    for name in sys.argv[1:]:
        print(f"{name}: {fetch(name)}")


if __name__ == "__main__":
    main()
