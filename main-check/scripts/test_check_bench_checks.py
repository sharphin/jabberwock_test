"""Negative controls for Windows-portable optimization check counter."""
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_bench_checks as checks


class PortableCheckCountTests(unittest.TestCase):
    def test_parse_valid_rows(self):
        self.assertEqual(checks.parse_rows("fib 0 1 2 3 4 5\n"), {"fib": (0, 1, 2, 3, 4, 5)})

    def test_bad_or_duplicate_rows_fail(self):
        for text in ("", "# comment\n", "fib 1 2\n", "fib 1 2 3 4 5 nope\n",
                     "fib 0 1 2 3 4 5\nfib 0 1 2 3 4 5\n"):
            with self.subTest(text=text), self.assertRaises(ValueError):
                checks.parse_rows(text)

    def test_stats_requires_one_valid_proof_of_execution(self):
        sample = "[stats] 世代の検査 1 か所・解析で消した 2 か所・添字の検査 3 か所"
        self.assertEqual(checks.parse_statistics(sample), (1, 2, 3))
        for invalid in ("", sample + "\n" + sample):
            with self.assertRaises(ValueError):
                checks.parse_statistics(invalid)

    def test_runtime_count_does_not_accept_missing_counters(self):
        self.assertEqual(checks.parse_runtime_counts("[checks] gen 1 elided 2 bounds 3"),
                         (1, 2, 3))
        for invalid in ("", "[checks] gen 1 elided 2", "[checks] gen a elided 2 bounds 3",
                        "[checks] gen 1 opt 2 idx 3", "[checks] gen 1 bounds 2 elided 3"):
            with self.assertRaises(ValueError):
                checks.parse_runtime_counts(invalid)

    def test_missing_extra_and_regression_fail(self):
        x = {"fib": (0, 1, 2, 3, 4, 5)}
        self.assertEqual(checks.compare(x, dict(x)), [])
        self.assertTrue(checks.compare(x, {}))
        self.assertTrue(checks.compare(x, {**x, "sort": x["fib"]}))
        self.assertTrue(checks.compare(x, {"fib": (1, 1, 2, 3, 4, 5)}))


if __name__ == "__main__":
    unittest.main()
