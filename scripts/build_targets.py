"""Builds the target programs of contract 5 (D19) from their pinned sources into
build/targets/:

    python3 scripts/build_targets.py {busybox|sqlite|lua|tcc|all}

- busybox: `make defconfig`, then CONFIG_STATIC=y and every option listed in
  targets/busybox/config.notes turned off, then `make`. Static. -> build/targets/busybox
- sqlite: `gcc -O2 shell.c sqlite3.c -lm`. Dynamic. -> build/targets/sqlite3
- lua: `make linux MYLIBS=` (no readline). Dynamic. -> build/targets/lua
- tcc: `./configure && make` with gcc. Dynamic. -> build/targets/tcc0, plus what TCC needs
  to compile itself in build/targets/tcc/ (`libtcc1.a` and `include/`), used with
  `tcc0 -B build/targets/tcc`.

Sources are fetched with fetch.py and unpacked (or copied, for the TCC clone) into a fresh
build/targets/src/<name>/ on every build, so a build never reuses a half-done tree.
"""

import os
import shutil
import subprocess
import sys

import common
import fetch

OUT = common.BUILD / "targets"
SRC = OUT / "src"
NOTES = common.ROOT / "targets" / "busybox" / "config.notes"
JOBS = str(os.cpu_count() or 1)


def sh(cmd, cwd, env=None):
    """Runs a build step; on failure prints its output and fails."""
    proc = subprocess.run([str(c) for c in cmd], cwd=cwd, env=env, capture_output=True,
                          text=True, errors="replace")
    if proc.returncode != 0:
        sys.stderr.write(proc.stdout[-8000:] + proc.stderr[-8000:])
        common.fail(f"{' '.join(str(c) for c in cmd)} failed in {cwd}")
    return proc.stdout


def unpack(name):
    """Fresh build/targets/src/<name>/ with the pinned archive unpacked into it; returns
    the single top directory of the archive."""
    archive = fetch.fetch(name)
    dest = SRC / name
    if dest.exists():
        shutil.rmtree(dest)
    dest.mkdir(parents=True)
    sh(["bsdtar", "-xf", archive, "-C", dest], cwd=dest)
    tops = [p for p in dest.iterdir()]
    if len(tops) != 1 or not tops[0].is_dir():
        common.fail(f"{archive}: expected one top directory, found {tops}")
    return tops[0]


def install(src, dest):
    dest.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(src, dest)
    return dest


def disabled_options(text):
    """CONFIG_ names turned off by config.notes: one per line, `CONFIG_NAME<TAB>reason`;
    `#` lines and blank lines are ignored."""
    names = []
    for line in text.splitlines():
        if not line.strip() or line.startswith("#"):
            continue
        name, _, reason = line.partition("\t")
        if not name.startswith("CONFIG_") or not reason.strip():
            common.fail(f"{NOTES}: bad line {line!r} (want CONFIG_NAME<TAB>reason)")
        names.append(name)
    return names


def set_options(config, on=(), off=()):
    """Returns the .config text with each option of `on` set to y and each of `off` unset."""
    wanted = {name: f"{name}=y" for name in on}
    wanted.update({name: f"# {name} is not set" for name in off})
    lines = []
    for line in config.splitlines():
        name = line.split("=")[0] if "=" in line else line.removeprefix("# ").split(" ")[0]
        lines.append(wanted.pop(name) if name in wanted else line)
    lines += wanted.values()
    return "\n".join(lines) + "\n"


def build_busybox():
    top = unpack("busybox")
    sh(["make", "defconfig"], cwd=top)
    off = disabled_options(NOTES.read_text()) if NOTES.exists() else []
    config = top / ".config"
    config.write_text(set_options(config.read_text(), on=["CONFIG_STATIC"], off=off))
    # oldconfig answers the questions the changes open with their defaults.
    subprocess.run(["make", "oldconfig"], cwd=top, input="\n" * 1000, text=True,
                   capture_output=True, check=False)
    final = config.read_text()
    if "CONFIG_STATIC=y" not in final.splitlines():
        common.fail("busybox .config lost CONFIG_STATIC=y")
    sh(["make", f"-j{JOBS}"], cwd=top)
    return install(top / "busybox", OUT / "busybox")


def build_sqlite():
    top = unpack("sqlite")
    sh(["gcc", "-O2", "shell.c", "sqlite3.c", "-lm", "-o", "sqlite3"], cwd=top)
    return install(top / "sqlite3", OUT / "sqlite3")


def build_lua():
    top = unpack("lua")
    sh(["make", f"-j{JOBS}", "linux", "MYLIBS="], cwd=top)
    return install(top / "src" / "lua", OUT / "lua")


def build_tcc():
    clone = fetch.fetch("tcc")
    top = SRC / "tcc"
    if top.exists():
        shutil.rmtree(top)
    shutil.copytree(clone, top, symlinks=True, ignore=shutil.ignore_patterns(".git"))
    sh(["./configure", "--cc=gcc"], cwd=top)
    sh(["make", f"-j{JOBS}"], cwd=top)
    tccdir = OUT / "tcc"
    if tccdir.exists():
        shutil.rmtree(tccdir)
    install(top / "libtcc1.a", tccdir / "libtcc1.a")
    shutil.copytree(top / "include", tccdir / "include")
    return install(top / "tcc", OUT / "tcc0")


BUILDERS = {
    "busybox": build_busybox,
    "sqlite": build_sqlite,
    "lua": build_lua,
    "tcc": build_tcc,
}


def main():
    common.require_non_root()
    if len(sys.argv) != 2 or sys.argv[1] not in (*BUILDERS, "all"):
        common.fail(f"usage: build_targets.py {{{'|'.join(BUILDERS)}|all}}")
    names = list(BUILDERS) if sys.argv[1] == "all" else [sys.argv[1]]
    for name in names:
        print(f"{name}: {BUILDERS[name]()}")


if __name__ == "__main__":
    main()
