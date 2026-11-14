"""Negative tests for concrete LLVM IR auditing."""
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
import llvm_ir_audit as audit


class LLVMAuditTests(unittest.TestCase):
    def test_missing_ir_is_not_success(self):
        with tempfile.TemporaryDirectory() as d:
            folder = Path(d)
            with mock.patch.object(audit.shutil, "which", return_value="/fake/opt"):
                self.assertEqual(audit.main(["--ir-root", str(folder),
                                             "--output", str(folder / "report.json")]), 1)
            report = json.loads((folder / "report.json").read_text(encoding="utf-8"))
            self.assertEqual(report["status"], "BLOCKED")
            self.assertFalse(report["semantic_refinement_proven"])

    def test_foreign_or_missing_case_ir_is_blocked(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            ir_root = root / "cases"
            (ir_root / "real").mkdir(parents=True)
            (ir_root / "real" / "jwc.ll").write_text(
                "define i32 @main() { ret i32 0 }", encoding="utf-8")
            benchmark = root / "benchmark.json"
            benchmark.write_text(json.dumps({"suite": ["expected"],
                                             "cases": {"expected": {"status": "PASS"}}}),
                                 encoding="utf-8")
            with mock.patch.object(audit.shutil, "which", return_value=None):
                self.assertEqual(audit.main([
                    "--ir-root", str(ir_root),
                    "--benchmark-report", str(benchmark),
                    "--output", str(root / "report.json")]), 1)
            result = json.loads((root / "report.json").read_text(encoding="utf-8"))
            self.assertTrue(any("missing or unexpected" in e for e in result["errors"]))

    def test_invalid_case_identity_never_passes(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / "case").mkdir()
            (root / "case" / "jwc.ll").write_text("x", encoding="utf-8")
            benchmark = root / "benchmark.json"
            benchmark.write_text(json.dumps({"suite": [["invalid"]],
                                             "cases": {"invalid": {"status": "PASS"}}}),
                                 encoding="utf-8")
            with mock.patch.object(audit.shutil, "which", return_value=None):
                self.assertEqual(audit.main([
                    "--ir-root", str(root),
                    "--benchmark-report", str(benchmark),
                    "--output", str(root / "report.json")]), 1)
            result = json.loads((root / "report.json").read_text(encoding="utf-8"))
            self.assertTrue(any("unsafe benchmark" in e for e in result["errors"]))

    def test_alive2_rejects_failed_refinement(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            for name in ("before.ll", "after.ll"):
                (root / name).write_text("define i32 @main() { ret i32 0 }",
                                         encoding="utf-8")
            pairs = root / "pairs.json"
            pairs.write_text(json.dumps({"pairs": [
                {"before": "before.ll", "after": "after.ll"}]}), encoding="utf-8")
            with mock.patch.object(audit.shutil, "which", side_effect=lambda x: "/fake/" + x):
                with mock.patch.object(audit.subprocess, "run") as process:
                    process.return_value.returncode = 1
                    process.return_value.stdout = "ERROR: Transformation doesn't verify!"
                    process.return_value.stderr = ""
                    self.assertEqual(audit.main([
                        "--ir-root", str(root), "--semantic-pairs", str(pairs),
                        "--output", str(root / "result.json")]), 1)
            result = json.loads((root / "result.json").read_text(encoding="utf-8"))
            self.assertEqual(result["semantic_results"][0]["status"], "BLOCKED")

    def test_failed_llvm_verifier_blocks_ir(self):
        with tempfile.TemporaryDirectory() as d:
            file = Path(d) / "bad.ll"
            file.write_text("not llvm ir", encoding="utf-8")
            with mock.patch.object(audit.subprocess, "run") as process:
                process.return_value.returncode = 1
                process.return_value.stderr = "parse error"
                result = audit.verify_ir(file, "opt")
            self.assertEqual(result["status"], "INVALID_IR")
            self.assertFalse(result["semantic_refinement_proven"])

    def test_ir_well_formed_does_not_claim_semantic_proof(self):
        with tempfile.TemporaryDirectory() as d:
            file = Path(d) / "sample.ll"
            file.write_text("define i32 @main() { ret i32 0 }", encoding="utf-8")
            with mock.patch.object(audit.subprocess, "run") as process:
                process.return_value.returncode = 0
                process.return_value.stderr = ""
                result = audit.verify_ir(file, "opt")
            self.assertEqual(result["status"], "IR_WELL_FORMED")
            self.assertFalse(result["semantic_refinement_proven"])


if __name__ == "__main__":
    unittest.main()
