#!/usr/bin/env python3
"""Negative controls for benchmark check-count comparison (no compiler needed)."""
from pathlib import Path
import os
import shutil
import subprocess
import tempfile
import unittest

AWK = Path(__file__).resolve().parents[1] / "bench" / "check_counts.awk"
ROW_A = "fib 0 1 2 3 4 5\n"
ROW_B = "sort 1 2 3 4 5 6\n"


@unittest.skipUnless(shutil.which("awk"), "awk required")
class CheckCountAuditTests(unittest.TestCase):
    def check(self, baseline: str, current: str) -> int:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            old = root / "baseline.txt"
            new = root / "new.txt"
            old.write_text(baseline, encoding="utf-8")
            new.write_text(current, encoding="utf-8")
            run = subprocess.run(["awk", "-f", str(AWK), str(old), str(new)],
                                 capture_output=True, text=True)
            return run.returncode

    def test_identical_measurements_pass(self):
        self.assertEqual(self.check(ROW_A + ROW_B, ROW_A + ROW_B), 0)

    def test_missing_current_case_fails(self):
        self.assertNotEqual(self.check(ROW_A + ROW_B, ROW_A), 0)

    def test_empty_baseline_fails(self):
        self.assertNotEqual(self.check("", ROW_A), 0)

    def test_empty_current_fails(self):
        self.assertNotEqual(self.check(ROW_A, ""), 0)

    def test_truncated_or_nonnumeric_count_fails(self):
        self.assertNotEqual(self.check(ROW_A, "fib 0 1 2\n"), 0)
        self.assertNotEqual(self.check(ROW_A, "fib 0 a 2 3 4 5\n"), 0)

    def test_duplicate_case_fails(self):
        self.assertNotEqual(self.check(ROW_A, ROW_A + ROW_A), 0)
        self.assertNotEqual(self.check(ROW_A + ROW_A, ROW_A), 0)

    @unittest.skipUnless(os.name == "posix" and shutil.which("sh"), "POSIX shell required")
    def test_compiler_failure_cannot_be_masked_by_statistics(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            bogus = root / "failing-jw"
            bogus.write_text("#!/bin/sh\necho '[stats] 世代の検査 0 か所・解析で消した 0 か所・添字の検査 0 か所'\nexit 17\n", encoding="utf-8")
            bogus.chmod(0o755)
            env = dict(os.environ, JW=str(bogus), BENCH_OUT=str(root))
            script = AWK.parent / "checks.sh"
            run = subprocess.run(["sh", str(script)], env=env, capture_output=True, text=True)
            self.assertNotEqual(run.returncode, 0)
            self.assertIn("compiler failed", run.stderr)

    def test_optimization_regression_fails(self):
        self.assertNotEqual(self.check(ROW_A, "fib 1 1 2 3 4 5\n"), 0)


if __name__ == "__main__":
    unittest.main()
