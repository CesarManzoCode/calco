"""Lua (contract 6): runs build/targets/lua on every targets/lua/*.lua natively and under
calco. Exits 0 only if each script ends the same way, with the same stdout byte by byte
and the same stderr.

    python3 scripts/lua_run.py [--calco BIN] [SCRIPT...]

Each script runs as `lua <name>.lua` with targets/lua/ as the current directory. The
outputs of a script that differs stay in build/lua/<name>.{native,calco}.out.
"""

import argparse
import signal
from pathlib import Path

import common
from build_targets import build_lua

SCRIPTS = common.ROOT / "targets" / "lua"
LUA = common.BUILD / "targets" / "lua"
OUT = common.BUILD / "lua"
ENV = {"PATH": "/usr/bin:/bin", "LC_ALL": "C", "TZ": "UTC"}
TIMEOUT = 600


def scripts():
    return sorted(SCRIPTS.glob("*.lua"))


def run_script(script, prefix=()):
    """Runs lua on `script` with `prefix` (calco, or nothing) before."""
    return common.run([*prefix, LUA, script.name], env=ENV, timeout=TIMEOUT,
                      cwd=script.parent)


def differences(native, calco):
    problems = []
    if native.returncode != calco.returncode:
        problems.append(f"native ended with {native.describe()}, calco with {calco.describe()}")
    if native.stdout != calco.stdout:
        problems.append("stdout differs")
    if native.stderr != calco.stderr:
        problems.append("stderr differs")
    return problems


def save(path, res):
    path.write_bytes(res.stdout + b"\n--- stderr ---\n" + res.stderr
                     + f"\n--- {res.describe()} ---\n".encode())


def main():
    common.require_non_root()
    parser = argparse.ArgumentParser()
    parser.add_argument("--calco", type=Path)
    parser.add_argument("script", nargs="*", type=Path)
    args = parser.parse_args()

    calco = args.calco or common.build_calco()
    if not LUA.exists():
        build_lua()
    OUT.mkdir(parents=True, exist_ok=True)

    failed = []
    selected = [p.resolve() for p in args.script] or scripts()
    for script in selected:
        native = run_script(script)
        under = run_script(script, prefix=(calco,))
        problems = differences(native, under)
        if native.returncode != 0:
            problems.insert(0, f"native lua ended with {native.describe()}")
        if problems:
            save(OUT / f"{script.stem}.native.out", native)
            save(OUT / f"{script.stem}.calco.out", under)
            failed.append(script.name)
            print(f"{script.name}: {'; '.join(problems)}")
        else:
            lines = native.stdout.count(b"\n")
            print(f"{script.name}: igual a nativo ({lines} líneas)")
    if failed:
        common.fail(f"lua: {len(failed)}/{len(selected)} distintos: {' '.join(failed)}; "
                    f"salidas en {OUT}")
    print(f"lua: {len(selected)}/{len(selected)} iguales a nativo")


if __name__ == "__main__":
    signal.signal(signal.SIGPIPE, signal.SIG_DFL)
    main()
