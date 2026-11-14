#!/usr/bin/env python3
"""Tests for the self-hosted jwc / Rust / C++ / Go report. No benchmark executed."""
from pathlib import Path
import os
import tempfile
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import benchmark_report as bench


class BenchmarkReportTests(unittest.TestCase):
    def test_targets_exactly_four(self):
        self.assertEqual(bench.TARGETS, ("jwc", "rust", "cpp", "go"))

    @unittest.skipUnless(os.name == "nt", "Windows native measurement test")
    def test_windows_measurement_records_time_output_and_memory(self):
        with tempfile.TemporaryDirectory() as temp:
            output = Path(temp) / "run.stdout"
            result = bench.run_windows_measure([sys.executable, "-c", "pass"], output, timeout=30)
            # The controlled Python command prints nothing, but process data
            # must remain measurable and the OS peak working set must be positive.
            self.assertEqual(result["exit"], 0)
            metrics = bench.parse_measure(result["stdout"])
            self.assertGreater(metrics["peak_rss_mb"], 0)
            self.assertTrue(output.is_file())

    def test_rounds_cover_all_four_subjects_without_fixed_first(self):
        self.assertEqual(len(bench.TARGETS), 4)
        orders = [bench.rotated_targets(i) for i in range(8)]
        self.assertTrue(all(set(order) == set(bench.TARGETS) for order in orders))
        self.assertEqual({order[0] for order in orders[:4]}, set(bench.TARGETS))

    def test_any_repetition_output_drift_is_rejected(self):
        self.assertTrue(bench.stable_output(["a", "a", "a"]))
        self.assertFalse(bench.stable_output(["a", "b", "a"]))
        self.assertFalse(bench.stable_output([]))

    def test_measure_parser(self):
        result = bench.parse_measure("0.12 0.15 0.20 4.5")
        self.assertEqual(result["median_s"], 0.15)
        self.assertEqual(result["peak_rss_mb"], 4.5)

    def test_invalid_measure(self):
        for invalid in ("0.2 0.1 0.3 1", "nan nan nan 2",
                        "0.1 0.2 inf 2", "0 0 0 2"):
            with self.subTest(value=invalid), self.assertRaises(ValueError):
                bench.parse_measure(invalid)

    def test_regression(self):
        metadata = {"schema_version": 2, "environment": {"os": "linux"},
                    "runs": 5, "subjects": list(bench.TARGETS), "compiler_hashes": {"jwc": "same", "runtime": "fixed"},
                    "source_hashes": {"fib": {"jwc": "a", "rust": "b", "cpp": "c", "go": "d"}},
                    "flags": {"rust": "-O3"}}
        baseline = {**metadata, "cases": {"fib": {"status": "PASS", "metrics": {
            "jwc": {"median_s": 1.0}}}}}
        current = {**metadata, "cases": {"fib": {"status": "PASS", "metrics": {
            "jwc": {"median_s": 1.2}}}}}
        self.assertEqual(bench.compare(current, baseline, 15)["regressions"], ["fib"])

    def test_different_jwc_version_is_comparable(self):
        a = {"schema_version": 2, "environment": {}, "runs": 5,
             "compiler_hashes": {"jwc": "new", "runtime": "fixed"}, "subjects": list(bench.TARGETS),
             "source_hashes": {}, "flags": {}, "cases": {}}
        b = {**a, "compiler_hashes": {"jwc": "old", "runtime": "fixed"}}
        self.assertEqual(bench.compare(a, b, 15)["status"], "NO_MATCHING_CASES")

    def test_changed_workload_is_not_comparable(self):
        a = {"schema_version": 2, "environment": {}, "runs": 5,
             "compiler_hashes": {"jwc": "new", "runtime": "fixed"},
             "source_hashes": {"fib": {"jwc": "new"}}, "flags": {}, "cases": {}}
        b = {**a, "source_hashes": {"fib": {"jwc": "old"}}}
        self.assertEqual(bench.compare(a, b, 15)["status"], "NOT_COMPARABLE")

    def test_missing_or_wrong_compiler_subjects_rejected(self):
        common = {"schema_version": 2, "environment": {}, "runs": 8,
                  "subjects": list(bench.TARGETS), "compiler_hashes": {"runtime": "ok"},
                  "source_hashes": {}, "flags": {}, "cases": {}}
        old = dict(common)
        old["subjects"] = ["jwc", "rust", "cpp"]
        self.assertEqual(bench.compare(common, old, 15)["status"], "NOT_COMPARABLE")
        self.assertEqual(bench.compare(old, common, 15)["status"], "NOT_COMPARABLE")

    def test_incomplete_baseline_never_silently_skips_cases(self):
        metadata = {"schema_version": 2, "environment": {}, "runs": 8,
                    "subjects": list(bench.TARGETS),
                    "compiler_hashes": {"runtime": "same"}, "source_hashes": {},
                    "flags": {}}
        ok = {"status": "PASS", "metrics": {"jwc": {"median_s": 1.0}}}
        current = {**metadata, "cases": {"fib": ok, "sort": ok}}
        baseline = {**metadata, "cases": {"fib": ok}}
        result = bench.compare(current, baseline, 15)
        self.assertEqual(result["status"], "INCOMPLETE_BASELINE")
        self.assertEqual(result["missing_from_baseline"], ["sort"])
        baseline["cases"]["sort"] = {"status": "FAIL"}
        self.assertEqual(bench.compare(current, baseline, 15)["status"], "INCOMPLETE_BASELINE")

    def test_nonfinite_baseline_time_is_invalid(self):
        metadata = {"schema_version": 2, "environment": {}, "runs": 8,
                    "subjects": list(bench.TARGETS), "compiler_hashes": {"runtime": "same"},
                    "source_hashes": {}, "flags": {}}
        ok = {"status": "PASS", "metrics": {"jwc": {"median_s": 1.0}}}
        current = {**metadata, "cases": {"fib": ok}}
        for bad in (0, -1, float("nan"), float("inf")):
            baseline = {**metadata, "cases": {"fib": {"status": "PASS",
                        "metrics": {"jwc": {"median_s": bad}}}}}
            with self.subTest(bad=bad):
                self.assertEqual(bench.compare(current, baseline, 15)["status"], "INVALID_BASELINE")

    def test_old_format_is_not_comparable(self):
        a = {"schema_version": 2}
        b = {"schema_version": 1}
        self.assertEqual(bench.compare(a, b, 15)["status"], "NOT_COMPARABLE")


if __name__ == "__main__":
    unittest.main()
