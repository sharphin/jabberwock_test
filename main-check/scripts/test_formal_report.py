"""Compiler proof attribution regressions; no real Lean processes in these cases."""
import contextlib
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock
import formal_report as formal

class FormalTests(unittest.TestCase):
    def test_all_five_scoped_goal_theorems_are_registered_without_trust_escape(self):
        path = formal.PROJECT / "JabberwockFormal" / "ScopedGoals.lean"
        source = path.read_text(encoding="utf-8")
        entry = (formal.PROJECT / "JabberwockFormal.lean").read_text(encoding="utf-8")
        self.assertIn("import JabberwockFormal.ScopedGoals", entry)
        for goal in ("g1_scoped_semantics", "g2_scoped_read_safe",
                     "g3_scoped_type_preservation", "g4_scoped_ownership",
                     "g5_scoped_check_correct"):
            with self.subTest(goal=goal):
                self.assertIn("theorem " + goal, source)
        import re
        self.assertIsNone(re.search(r"\\b(?:sorry|admit|axiom|unsafe|native_decide)\\b", source))

    def test_each_compiler_has_five_independent_unproved_obligations(self):
        with tempfile.TemporaryDirectory() as d:
            dest = Path(d) / "formal.json"
            with mock.patch.object(sys, "argv", ["formal_report", "--require-full", "--output", str(dest)]), \
                 mock.patch.object(formal.subprocess, "run") as run, \
                 mock.patch.object(formal, "audit_module", return_value={"status": "KERNEL_CHECKED"}), contextlib.redirect_stdout(io.StringIO()):
                code = formal.main()
            data = json.loads(dest.read_text(encoding="utf-8"))
            self.assertEqual(code, 1)
            self.assertEqual(len(data["goals"]), 10)
            for compiler in ("jwc", "rust"):
                self.assertEqual({x["goal"] for x in data["goals"] if x["compiler"] == compiler}, set(formal.GOALS))
            self.assertTrue(all(x["status"] == "NOT_PROVEN" for x in data["goals"]))
            self.assertNotEqual(data["compilers"]["jwc"]["source"]["sha256"], data["compilers"]["rust"]["source"]["sha256"])
            run.assert_not_called()

    def test_jwc_selection_does_not_include_rust_proofs(self):
        with tempfile.TemporaryDirectory() as d:
            dest = Path(d) / "formal.json"
            with mock.patch.object(sys, "argv", ["formal_report", "--compiler", "jwc", "--output", str(dest)]), \
                 mock.patch.object(formal, "audit_module", return_value={"status": "KERNEL_CHECKED"}), contextlib.redirect_stdout(io.StringIO()):
                formal.main()
            data = json.loads(dest.read_text(encoding="utf-8"))
            self.assertEqual(len(data["goals"]), 5)
            self.assertEqual(set(data["compilers"]), {"jwc"})

    def test_manifest_rejects_missing_and_foreign_theorems(self):
        for compiler in ("jwc", "rust"):
            manifest = json.loads((formal.PROJECT / "compilers" / compiler / "obligations.json").read_text(encoding="utf-8"))
            formal.validate_manifest(manifest, compiler)
            broken = json.loads(json.dumps(manifest))
            broken["goals"]["memory"]["theorems"] = []
            with self.assertRaises(ValueError):
                formal.validate_manifest(broken, compiler)
            broken = json.loads(json.dumps(manifest))
            broken["goals"]["memory"]["theorems"] = ["JabberwockFormal.Other.ReferenceAccess.read_requires_valid"]
            with self.assertRaises(ValueError):
                formal.validate_manifest(broken, compiler)
            broken = json.loads(json.dumps(manifest))
            broken["goals"]["types"]["theorems"] *= 2
            with self.assertRaises(ValueError):
                formal.validate_manifest(broken, compiler)

    def test_full_proof_is_not_claimed_by_kernel_success(self):
        with tempfile.TemporaryDirectory() as d:
            dest = Path(d) / "formal.json"
            with mock.patch.object(sys, "argv", ["formal_report", "--require-full", "--output", str(dest)]), \
                 mock.patch.object(formal, "audit_module", return_value={"status": "KERNEL_CHECKED"}), contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(formal.main(), 1)
            data = json.loads(dest.read_text(encoding="utf-8"))
            self.assertEqual(data["summary"]["expected_goal_count"], 10)
            self.assertEqual(data["summary"]["whole_compiler_proven_goal_count"], 0)
            self.assertFalse(data["summary"]["all_compilers_proven"])

    def test_unapproved_axioms_and_missing_audit_fail_closed(self):
        theorem = "JabberwockFormal.Jwc.ReferenceAccess.gate_semantics"
        module = "JabberwockFormal.Jwc.ReferenceAccess"
        version = mock.Mock(returncode=0, stdout="Lean (version 4.24.0, release)", stderr="")
        built = mock.Mock(returncode=0, stdout="", stderr="")
        for output in ("", "'" + theorem + "' depends on axioms: [sorryAx]"):
            audited = mock.Mock(returncode=0, stdout=output, stderr="")
            with mock.patch.object(formal.subprocess, "run", side_effect=[version, built, audited]):
                record = formal.audit_module(module, [theorem])
            self.assertEqual(record["status"], "FAIL")

if __name__ == "__main__":
    unittest.main()
