"""Unit controls for the single-command Windows/Linux verification driver.

These test stage contracts without executing the actual expensive suite.
"""
from pathlib import Path
import sys
import tempfile
import json
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
import run_everything as full


class FullRunnerTests(unittest.TestCase):
    def test_inventory_is_not_an_execution_failure(self):
        with tempfile.TemporaryDirectory() as d:
            output = Path(d)
            inventory = {"status": "NOT_PROVEN", "total_obligations": 1,
                         "proved_obligations": 0,
                         "components": [{"id": "jwc:semantics:jwc/main.jw",
                                         "status": "REFINEMENT_NOT_PROVEN"}]}
            (output / "whole-proof-inventory.json").write_text(json.dumps(inventory))
            stages = [{"name": name, "status": "EXIT_ZERO", "exit_code": 0}
                      for name, _ in full.plans(output)]
            summary = full.summarize(stages, output, quick=False)
            self.assertEqual(summary["status"], "FAIL")
            self.assertEqual(summary["evidence"]["implementation_inventory"]["proved"], 0)
            self.assertNotEqual(summary["reason"], "one or more verification stages failed")

    def test_default_plan_runs_every_required_stage(self):
        output = Path("dummy")
        tasks = full.plans(output)
        self.assertEqual([x[0] for x in tasks], [
            "python-regressions", "cargo-tests", "cargo-doctests", "cargo-clippy",
            "practical-safety", "whole-implementation-obligations", "refinement-transition", "build-dependency-audit", "emitter-proof-binding", "formal-proofs",
            "acceptance-34", "benchmark-four-way", "llvm-concrete-ir-audit", "legacy-optimization-counts"])
        self.assertIn("test_*.py", tasks[0][1])
        self.assertIn("--require-full", dict(tasks)["formal-proofs"])
        self.assertIn("--benchmark-report", dict(tasks)["llvm-concrete-ir-audit"])
        self.assertIn("refinement-transition", dict(tasks))
        self.assertIn("--full", dict(tasks)["benchmark-four-way"])
        self.assertNotIn("--full", dict(full.plans(output, quick=True))["benchmark-four-way"])

    def test_windows_launches_batches_and_uses_python_for_formal(self):
        with mock.patch.object(full, "WINDOWS", True):
            steps = dict(full.plans(Path("out")))
            self.assertEqual(steps["practical-safety"][:3], ["cmd", "/d", "/c"])
            self.assertTrue(steps["practical-safety"][3].endswith("safety_windows.cmd"))
            self.assertTrue(steps["benchmark-four-way"][3].endswith("benchmark_windows.cmd"))
            self.assertEqual(steps["formal-proofs"][0], sys.executable)

    def test_missing_or_failed_stage_never_passes(self):
        with tempfile.TemporaryDirectory() as d:
            folder = Path(d)
            names = [n for n, _ in full.plans(folder)]
            stages = [{"name": n, "status": "EXIT_ZERO", "exit_code": 0} for n in names]
            self.assertEqual(full.summarize(stages, folder, quick=False)["status"], "FAIL")
            self.assertEqual(full.summarize(stages[:-1], folder, quick=False)["status"], "FAIL")
            stages[1]["status"] = "EXIT_NONZERO"
            self.assertEqual(full.summarize(stages, folder, quick=False)["status"], "FAIL")

    def test_unproved_implementation_links_appear_even_on_nonzero_stage(self):
        with tempfile.TemporaryDirectory() as d:
            output = Path(d)
            report = {"status": "NOT_PROVEN", "total_obligations": 2,
                      "proved_obligations": 0,
                      "components": [
                          {"id": "jwc:types:jwc/typeck.jw", "status": "REFINEMENT_NOT_PROVEN"},
                          {"id": "rust:memory:runtime/jw_rt.c", "status": "REFINEMENT_NOT_PROVEN"}
                      ]}
            (output / "whole-proof-inventory.json").write_text(json.dumps(report))
            names = [name for name, _ in full.plans(output)]
            failed = [{"name": name, "status": "EXIT_NONZERO", "exit_code": 1}
                      for name in names]
            result = full.summarize(failed, output, quick=False)
            self.assertEqual(result["status"], "FAIL")
            evidence = result["evidence"]["implementation_inventory"]
            self.assertEqual(evidence["required"], 2)
            self.assertEqual(len(evidence["uncovered"]), 2)
            self.assertEqual(set(result["failed_stages"]), set(names))

    def test_no_summary_pass_with_duplicate_stage(self):
        with tempfile.TemporaryDirectory() as d:
            output = Path(d)
            names = [name for name, _ in full.plans(output)]
            stages = [{"name": name, "status": "EXIT_ZERO", "exit_code": 0}
                      for name in names]
            stages[0] = stages[1].copy()
            self.assertEqual(full.summarize(stages, output, quick=False)["status"], "FAIL")

    def test_actual_failure_is_not_masked_by_unproved_refinement(self):
        with tempfile.TemporaryDirectory() as d:
            folder = Path(d)
            (folder / "whole-proof-inventory.json").write_text(json.dumps({
                "status": "NOT_PROVEN", "total_obligations": 1,
                "proved_obligations": 0, "components": [{
                    "id": "jwc:semantics:jwc/main.jw",
                    "status": "REFINEMENT_NOT_PROVEN"}]}), encoding="utf-8")
            stages = [{"name": name, "status": "EXIT_ZERO", "exit_code": 0}
                      for name, _ in full.plans(folder)]
            stages[0]["status"] = "EXIT_NONZERO"
            stages[0]["exit_code"] = 7
            result = full.summarize(stages, folder, quick=False)
            self.assertEqual(result["status"], "FAIL")
            self.assertEqual(result["failed_stages"], ["python-regressions"])
            self.assertIn("python-regressions", result["reason"])

    def test_runtime_stage_failure_is_retained_with_logs(self):
        with tempfile.TemporaryDirectory() as d:
            folder = Path(d)
            cmd = [sys.executable, "-c", "import sys; print('evidence'); sys.exit(17)"]
            stage = full.execute_stage("controlled-failure", cmd, folder, timeout=10)
            self.assertEqual(stage["status"], "EXIT_NONZERO")
            self.assertEqual(stage["exit_code"], 17)
            self.assertIn("evidence", Path(stage["stdout_log"]).read_text())
            self.assertTrue(Path(stage["stderr_log"]).is_file())


if __name__ == "__main__":
    unittest.main()
