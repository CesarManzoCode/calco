"""Tests of the SQLite, Lua and TCC cases: the Lua scripts do not read the clock, the SQL
does not use random() or 'now', and the TCC self-compilation line is the one fixed from
TCC's Makefile."""

import re
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "scripts"))
import lua_run  # noqa: E402
import sqlite_run  # noqa: E402
import tcc_selfcompile  # noqa: E402

LUA_BENCH = lua_run.SCRIPTS / "bench"
SQL = [sqlite_run.WORKLOAD, sqlite_run.WORKLOAD.with_name("bench.sql")]


def lua_files():
    return lua_run.scripts() + sorted(LUA_BENCH.glob("*.lua"))


class Lua(unittest.TestCase):
    def test_at_least_8_scripts_and_3_benches(self):
        self.assertGreaterEqual(len(lua_run.scripts()), 8)
        self.assertGreaterEqual(len(list(LUA_BENCH.glob("*.lua"))), 3)

    def test_no_clock(self):
        for path in lua_files():
            text = path.read_text()
            for name in ("os.time", "os.clock", "os.date"):
                self.assertNotIn(name, text, path.name)

    def test_random_is_seeded_with_42(self):
        for path in lua_files():
            text = path.read_text()
            if "math.random(" in text:
                self.assertIn("math.randomseed(42)", text, path.name)


class Sql(unittest.TestCase):
    def test_no_random_or_now(self):
        for path in SQL:
            text = path.read_text().lower()
            self.assertIsNone(re.search(r"random\s*\(", text), path.name)
            self.assertIsNone(re.search(r"randomblob\s*\(", text), path.name)
            self.assertNotIn("'now'", text, path.name)

    def test_required_features(self):
        for path in SQL:
            text = path.read_text().upper()
            for word in ("CREATE TABLE", "WITH RECURSIVE", "CREATE INDEX", "JOIN", "GROUP BY",
                         "OVER (", "JSON", "BEGIN", "COMMIT", "VACUUM"):
                self.assertIn(word, text, f"{path.name}: {word}")

    def test_databases_in_build_sqlite(self):
        self.assertEqual(sqlite_run.OUT, sqlite_run.common.BUILD / "sqlite")


class Tcc(unittest.TestCase):
    def test_compile_line(self):
        out = tcc_selfcompile.OUT / "tcc1"
        self.assertEqual(
            [str(a) for a in tcc_selfcompile.compile_line(out)],
            [str(tcc_selfcompile.TCC0), "-B.", "-I./include", "-I.", "-I.", "tcc.c",
             "-o", str(out), "-ftest-coverage", "-I.", "-lm", "-ldl", "-lpthread"])

    def test_paths(self):
        build = tcc_selfcompile.common.BUILD
        self.assertEqual(tcc_selfcompile.TCC0, build / "targets" / "tcc0")
        self.assertEqual(tcc_selfcompile.TREE, build / "targets" / "src" / "tcc")
        self.assertEqual(tcc_selfcompile.OUT, build / "tcc")


if __name__ == "__main__":
    unittest.main()
