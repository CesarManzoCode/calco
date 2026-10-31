"""Tests of bench.py on fixed measurements: medians, the geometric mean of the ratios of
medians, missing fields of results.json and the RSS limit of calco."""

import copy
import math
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "scripts"))
import bench  # noqa: E402


def runs(*seconds, kb=1000):
    return [{"seconds": float(s), "maxrss_kb": kb} for s in seconds]


def results():
    """Two workloads in a complete results.json, plus three more so every name is there."""
    workloads = {
        # Medians: native 1, qemu 4, calco 2.
        "sqlite": {"native": runs(1, 3, 0.5, 1, 9), "qemu": runs(4, 4, 5, 3, 4),
                   "calco": runs(2, 1, 2, 8, 2)},
        # Medians: native 2, qemu 10, calco 5.
        "lua": {"native": runs(2, 2, 2, 2, 2), "qemu": runs(10, 11, 9, 10, 12),
                "calco": runs(5, 5, 6, 4, 5)},
    }
    for name in ("tcc", "gzip", "sha512-sse"):
        workloads[name] = copy.deepcopy(workloads["lua"])
    return {"qemu_version": "qemu-x86_64 version 11.1.2", "cpu_model": "AMD Ryzen 5 5600G",
            "kernel": "7.2.11-arch1-1", "core": 2, "workloads": workloads,
            "geomean_calco_qemu": 0.5, "geomean_calco_native": 2.5}


class Stats(unittest.TestCase):
    def test_median_of_five(self):
        self.assertEqual(bench.median_seconds(runs(5, 1, 4, 2, 3)), 3.0)

    def test_median_ignores_outlier(self):
        self.assertEqual(bench.median_seconds(runs(1, 1, 1, 1, 100)), 1.0)

    def test_geomean_of_ratios_of_medians(self):
        r = results()
        r["workloads"] = {k: r["workloads"][k] for k in ("sqlite", "lua")}
        # calco/qemu: 2/4 and 5/10 -> 0.5; calco/native: 2/1 and 5/2 -> sqrt(5).
        self.assertAlmostEqual(bench.geomean_ratio(r, "calco", "qemu"), 0.5)
        self.assertAlmostEqual(bench.geomean_ratio(r, "calco", "native"), math.sqrt(5))


class Fields(unittest.TestCase):
    def test_complete(self):
        self.assertEqual(bench.missing_fields(results()), [])

    def test_each_metadata_field(self):
        for key in bench.METADATA:
            r = results()
            del r[key]
            self.assertIn(key, bench.missing_fields(r), key)

    def test_empty_metadata(self):
        r = results()
        r["qemu_version"] = None
        r["cpu_model"] = ""
        self.assertEqual(bench.missing_fields(r)[:2], ["qemu_version", "cpu_model"])

    def test_missing_workload(self):
        r = results()
        del r["workloads"]["gzip"]
        self.assertEqual(bench.missing_fields(r), ["workloads.gzip"])

    def test_missing_mode_and_short_list(self):
        r = results()
        del r["workloads"]["lua"]["qemu"]
        r["workloads"]["sqlite"]["calco"].pop()
        self.assertEqual(sorted(bench.missing_fields(r)),
                         ["workloads.lua.qemu", "workloads.sqlite.calco"])

    def test_missing_run_fields(self):
        r = results()
        del r["workloads"]["tcc"]["native"][0]["maxrss_kb"]
        r["workloads"]["tcc"]["native"][1]["seconds"] = None
        self.assertEqual(bench.missing_fields(r), ["workloads.tcc.native[0].maxrss_kb",
                                                   "workloads.tcc.native[1].seconds"])


class Rss(unittest.TestCase):
    def test_under_and_at_limit(self):
        r = results()
        r["workloads"]["sqlite"]["calco"][0]["maxrss_kb"] = bench.RSS_LIMIT_KB
        self.assertEqual(bench.rss_over(r), [])

    def test_over_limit(self):
        r = results()
        r["workloads"]["lua"]["calco"][3]["maxrss_kb"] = bench.RSS_LIMIT_KB + 1
        self.assertEqual(bench.rss_over(r), [("lua", 3, bench.RSS_LIMIT_KB + 1)])

    def test_only_calco_counts(self):
        r = results()
        r["workloads"]["lua"]["qemu"][0]["maxrss_kb"] = 2 * bench.RSS_LIMIT_KB
        self.assertEqual(bench.rss_over(r), [])

    def test_limit_is_one_gib(self):
        self.assertEqual(bench.RSS_LIMIT_KB, 1_048_576)


if __name__ == "__main__":
    unittest.main()
