"""Performance (contract 8 and 9): runs the five fixed workloads natively, under
qemu-x86_64 and under calco, writes build/bench/results.json and prints

    calco/qemu: <x>
    calco/nativo: <y>

with 4 decimals. Exits 0 only if results.json has every field of contract 8, the
geometric mean calco/qemu is at most 0.5 and no calco run went over 1 048 576 KiB of
`ru_maxrss`.

    python3 scripts/bench.py [--calco BIN]

Workloads (contract 8):
- sqlite: build/targets/sqlite3 on targets/sqlite/bench.sql (stdin), new database each run.
- lua: build/targets/lua on each targets/lua/bench/*.lua, one after the other.
- tcc: the self-compilation of tcc_selfcompile.py, without comparing against native.
- gzip: `busybox gzip -9 -c` of the 16 MiB file of targets/busybox/gen_bench_input.py,
  then `busybox sha256sum` of the result.
- sha512-sse: the QEMU test, compiled as qemu_tcg.py compiles it.

Methodology: every process runs as `taskset -c 2 [qemu|calco] PROGRAM ARGS`. Per workload,
one warm-up run in each mode, discarded, is the one checked: each mode must end the same
way as native, with the same stdout and stderr and the same files (the database, the TCC
output); otherwise the script fails before measuring. Then 5 measured rounds, each of
native, qemu and calco in that order, also checked. A run's `seconds` is the wall time of
its processes by `time.perf_counter` (summed over the processes of a workload with more
than one, lua and gzip) and its `maxrss_kb` is `ru_maxrss` from `os.wait4` (the largest
of its processes). The median of the 5 runs is taken per workload and mode; the result is
the geometric mean over the workloads of the ratios of medians.

Outputs of each run stay in build/bench/<workload>/<mode>/.
"""

import argparse
import hashlib
import json
import math
import os
import signal
import statistics
import subprocess
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path

import common
import fetch
import qemu_tcg
from build_targets import build_busybox, build_lua, build_sqlite, build_tcc
from busybox_run import BUSYBOX
from sqlite_run import SIDE_FILES, SQLITE
from lua_run import LUA
from tcc_selfcompile import TCC0, TREE, compile_line

OUT = common.BUILD / "bench"
RESULTS = OUT / "results.json"
CORE = 2
MODES = ("native", "qemu", "calco")
RUNS = 5
RSS_LIMIT_KB = 1_048_576
GEOMEAN_LIMIT = 0.5
ENV = {"PATH": "/usr/bin:/bin", "LC_ALL": "C", "TZ": "UTC", "HOME": str(OUT)}
SQL = common.ROOT / "targets" / "sqlite" / "bench.sql"
LUA_SCRIPTS = common.ROOT / "targets" / "lua" / "bench"
GEN_INPUT = common.ROOT / "targets" / "busybox" / "gen_bench_input.py"
WORKLOADS = ("sqlite", "lua", "tcc", "gzip", "sha512-sse")
METADATA = ("qemu_version", "cpu_model", "kernel", "core", "workloads",
            "geomean_calco_qemu", "geomean_calco_native")


@dataclass
class Step:
    """One process of a workload. `stdout` is a file to keep (else a log of the run)."""
    argv: list
    cwd: Path
    stdin: Path = None
    stdout: Path = None


@dataclass
class Workload:
    name: str
    steps: list
    # Called before each run (fresh database, no previous output).
    prepare: callable = lambda: None
    # Files whose sha256 is part of what must equal native.
    files: list = field(default_factory=list)


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest() if Path(path).exists() else "missing"


def run_step(step, prefix, logs, index):
    """Runs one process pinned to CORE; returns (outcome, seconds, maxrss_kb)."""
    stdout = step.stdout or logs / f"{index}.stdout"
    stderr = logs / f"{index}.stderr"
    cmd = ["taskset", "-c", str(CORE), *map(str, prefix), *map(str, step.argv)]
    with (open(step.stdin or os.devnull, "rb") as fin, open(stdout, "wb") as fout,
          open(stderr, "wb") as ferr):
        start = time.perf_counter()
        proc = subprocess.Popen(cmd, stdin=fin, stdout=fout, stderr=ferr, cwd=step.cwd,
                                env=ENV)
        _, status, usage = os.wait4(proc.pid, 0)
        seconds = time.perf_counter() - start
    proc.returncode = os.waitstatus_to_exitcode(status)
    return (proc.returncode, sha256(stdout), sha256(stderr)), seconds, usage.ru_maxrss


def run_workload(workload, mode, prefix):
    """One run of `workload` in `mode`: (outcome, {"seconds", "maxrss_kb"})."""
    logs = OUT / workload.name / mode
    logs.mkdir(parents=True, exist_ok=True)
    workload.prepare()
    outcome, seconds, rss = [], 0.0, 0
    for index, step in enumerate(workload.steps):
        got, secs, kb = run_step(step, prefix, logs, index)
        outcome.append(got)
        seconds += secs
        rss = max(rss, kb)
    outcome += [sha256(f) for f in workload.files]
    return outcome, {"seconds": seconds, "maxrss_kb": rss}


def workloads(sha512):
    sqlite_dir, tcc_dir, gzip_dir = OUT / "sqlite", OUT / "tcc", OUT / "gzip"
    for d in (sqlite_dir, tcc_dir, gzip_dir):
        d.mkdir(parents=True, exist_ok=True)
    db = sqlite_dir / "bench.db"
    tcc_out = tcc_dir / "tcc1"
    data = gzip_dir / "input.bin"
    packed = gzip_dir / "input.bin.gz"
    subprocess.run([sys.executable, GEN_INPUT, data], check=True)

    def fresh_db():
        for suffix in SIDE_FILES:
            Path(f"{db}{suffix}").unlink(missing_ok=True)

    return [
        Workload("sqlite", [Step([SQLITE, "-batch", db], sqlite_dir, stdin=SQL)],
                 prepare=fresh_db, files=[db]),
        Workload("lua", [Step([LUA, s.name], LUA_SCRIPTS)
                         for s in sorted(LUA_SCRIPTS.glob("*.lua"))]),
        Workload("tcc", [Step(compile_line(tcc_out), TREE)],
                 prepare=lambda: tcc_out.unlink(missing_ok=True), files=[tcc_out]),
        Workload("gzip", [Step([BUSYBOX, "gzip", "-9", "-c", data.name], gzip_dir,
                               stdout=packed),
                          Step([BUSYBOX, "sha256sum", packed.name], gzip_dir)]),
        Workload("sha512-sse", [Step([sha512], qemu_tcg.OUT)]),
    ]


def median_seconds(runs):
    return statistics.median(r["seconds"] for r in runs)


def geomean_ratio(results, num, den):
    """Geometric mean over the workloads of median(num) / median(den)."""
    ratios = [median_seconds(w[num]) / median_seconds(w[den])
              for w in results["workloads"].values()]
    return math.exp(sum(math.log(r) for r in ratios) / len(ratios))


def missing_fields(results):
    """What contract 8 asks of results.json and is not there."""
    missing = [key for key in METADATA if results.get(key) in (None, "")]
    workloads = results.get("workloads") or {}
    missing += [f"workloads.{name}" for name in WORKLOADS if name not in workloads]
    for name, modes in workloads.items():
        for mode in MODES:
            runs = modes.get(mode)
            if not isinstance(runs, list) or len(runs) != RUNS:
                missing.append(f"workloads.{name}.{mode}")
                continue
            for i, run in enumerate(runs):
                if not isinstance(run.get("seconds"), float):
                    missing.append(f"workloads.{name}.{mode}[{i}].seconds")
                if not isinstance(run.get("maxrss_kb"), int):
                    missing.append(f"workloads.{name}.{mode}[{i}].maxrss_kb")
    return missing


def rss_over(results, limit=RSS_LIMIT_KB):
    """Calco runs whose maxrss_kb goes over `limit`, as (workload, run, kb)."""
    return [(name, i, run["maxrss_kb"])
            for name, modes in results["workloads"].items()
            for i, run in enumerate(modes.get("calco", []))
            if run.get("maxrss_kb", 0) > limit]


def cpu_model():
    for line in Path("/proc/cpuinfo").read_text().splitlines():
        key, _, value = line.partition(":")
        if key.strip() == "model name":
            return value.strip()
    return None


def first_line(cmd):
    res = common.run(cmd)
    lines = res.stdout.decode(errors="replace").splitlines()
    return lines[0].strip() if res.returncode == 0 and lines else None


def main():
    common.require_non_root()
    parser = argparse.ArgumentParser()
    parser.add_argument("--calco", type=Path)
    args = parser.parse_args()

    calco = args.calco or common.build_calco()
    qemu = fetch.fetch("qemu-user")
    for binary, build in ((SQLITE, build_sqlite), (LUA, build_lua), (BUSYBOX, build_busybox)):
        if not binary.exists():
            build()
    if not TCC0.exists() or not (TREE / "tcc.c").exists():
        build_tcc()
    sha512 = qemu_tcg.build("sha512-sse")
    prefixes = {"native": (), "qemu": (qemu,), "calco": (calco,)}

    results = {
        "qemu_version": first_line([qemu, "--version"]),
        "cpu_model": cpu_model(),
        "kernel": os.uname().release,
        "core": CORE,
        "workloads": {},
    }
    for workload in workloads(sha512):
        want = None
        for mode in MODES:
            got, _ = run_workload(workload, mode, prefixes[mode])
            if want is None:
                want = got
            if got != want:
                common.fail(f"{workload.name}: {mode} differs from native "
                            f"(outputs in {OUT / workload.name})")
        runs = {mode: [] for mode in MODES}
        for _ in range(RUNS):
            for mode in MODES:
                got, measure = run_workload(workload, mode, prefixes[mode])
                if got != want:
                    common.fail(f"{workload.name}: a measured {mode} run differs from "
                                f"native (outputs in {OUT / workload.name})")
                runs[mode].append(measure)
        results["workloads"][workload.name] = runs
        print(f"{workload.name}: " + ", ".join(
            f"{mode} {median_seconds(runs[mode]):.3f} s" for mode in MODES))

    results["geomean_calco_qemu"] = geomean_ratio(results, "calco", "qemu")
    results["geomean_calco_native"] = geomean_ratio(results, "calco", "native")
    RESULTS.write_text(json.dumps(results, indent=2) + "\n")
    print(f"calco/qemu: {results['geomean_calco_qemu']:.4f}")
    print(f"calco/nativo: {results['geomean_calco_native']:.4f}")

    problems = []
    missing = missing_fields(results)
    if missing:
        problems.append(f"missing in results.json: {', '.join(missing)}")
    if results["geomean_calco_qemu"] > GEOMEAN_LIMIT:
        problems.append(f"calco/qemu {results['geomean_calco_qemu']:.4f} > {GEOMEAN_LIMIT}")
    for name, i, kb in rss_over(results):
        problems.append(f"{name}: calco run {i} maxrss {kb} KiB > {RSS_LIMIT_KB}")
    if problems:
        common.fail("; ".join(problems))


if __name__ == "__main__":
    signal.signal(signal.SIGPIPE, signal.SIG_DFL)
    main()
