"""SQLite (contract 6): runs build/targets/sqlite3 on targets/sqlite/workload.sql natively
and under calco, each time on a new database, build/sqlite/native.db and
build/sqlite/calco.db. Exits 0 only if both runs end the same way, with the same stdout
byte by byte, the same stderr and the same sha256 of the database file.

    python3 scripts/sqlite_run.py [--calco BIN] [--sql FILE]

The SQL goes in through stdin (`sqlite3 DB < workload.sql`), so the shell dot-commands
of the workload apply. The raw outputs stay in build/sqlite/{native,calco}.out.
"""

import argparse
import hashlib
import signal
from pathlib import Path

import common
from build_targets import build_sqlite

WORKLOAD = common.ROOT / "targets" / "sqlite" / "workload.sql"
SQLITE = common.BUILD / "targets" / "sqlite3"
OUT = common.BUILD / "sqlite"
ENV = {"PATH": "/usr/bin:/bin", "LC_ALL": "C", "TZ": "UTC", "HOME": str(OUT)}
TIMEOUT = 1800
# Files SQLite may leave next to a database.
SIDE_FILES = ("", "-journal", "-wal", "-shm")


def fresh_db(name):
    """build/sqlite/<name>.db with no previous database or journal left."""
    db = OUT / f"{name}.db"
    for suffix in SIDE_FILES:
        Path(f"{db}{suffix}").unlink(missing_ok=True)
    return db


def run_workload(db, sql, prefix=()):
    """Runs sqlite3 on `db` with `sql` as stdin and `prefix` (calco, or nothing) before."""
    # -batch: no prompts and no ~/.sqliterc, so nothing outside the workload matters.
    return common.run([*prefix, SQLITE, "-batch", db], stdin=Path(sql).read_bytes(),
                      env=ENV, timeout=TIMEOUT, cwd=OUT)


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def differences(native, calco, native_hash, calco_hash):
    problems = []
    if native.returncode != calco.returncode:
        problems.append(f"native ended with {native.describe()}, calco with {calco.describe()}")
    if native.stdout != calco.stdout:
        problems.append("stdout differs")
    if native.stderr != calco.stderr:
        problems.append("stderr differs")
    if native_hash != calco_hash:
        problems.append(f"database sha256 differs: native {native_hash}, calco {calco_hash}")
    return problems


def save(path, res):
    path.write_bytes(res.stdout + b"\n--- stderr ---\n" + res.stderr
                     + f"\n--- {res.describe()} ---\n".encode())


def main():
    common.require_non_root()
    parser = argparse.ArgumentParser()
    parser.add_argument("--calco", type=Path)
    parser.add_argument("--sql", type=Path, default=WORKLOAD)
    args = parser.parse_args()

    calco = args.calco or common.build_calco()
    if not SQLITE.exists():
        build_sqlite()
    OUT.mkdir(parents=True, exist_ok=True)

    native_db = fresh_db("native")
    native = run_workload(native_db, args.sql)
    save(OUT / "native.out", native)
    if native.returncode != 0:
        common.fail(f"native sqlite3 ended with {native.describe()}; see {OUT / 'native.out'}")
    calco_db = fresh_db("calco")
    under = run_workload(calco_db, args.sql, prefix=(calco,))
    save(OUT / "calco.out", under)

    native_hash = sha256(native_db)
    calco_hash = sha256(calco_db) if calco_db.exists() else "missing"
    problems = differences(native, under, native_hash, calco_hash)
    lines = native.stdout.count(b"\n")
    if problems:
        common.fail(f"{args.sql.name}: {'; '.join(problems)}; outputs in {OUT}")
    print(f"sqlite: {args.sql.name} igual a nativo ({lines} líneas, base sha256 {native_hash})")


if __name__ == "__main__":
    signal.signal(signal.SIGPIPE, signal.SIG_DFL)
    main()
