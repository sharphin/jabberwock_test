"""Negative tests for declared build dependency coverage."""
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
import build_dependency_audit as audit


class BuildDependencyTests(unittest.TestCase):
    def test_source_snapshot_change_blocks_closure(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / "jwc").mkdir()
            (root / "jwc/main.jw").write_text("use missing;", encoding="utf-8")
            with mock.patch.object(audit, "ROOT", root):
                with mock.patch.object(audit, "source_identity", return_value={
                    "sha256": "test", "files": {"jwc/main.jw": "incorrect"}}):
                    result = audit.check("jwc")
            self.assertEqual(result["status"], "BLOCKED")
            self.assertTrue(any("unresolved import" in e for e in result["errors"]))
            self.assertFalse(result["build_dependency_closure_proven"])

    def test_existing_source_does_not_imply_build_proof(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / "jwc").mkdir()
            (root / "jwc/main.jw").write_text("", encoding="utf-8")
            import hashlib
            digest = hashlib.sha256(b"").hexdigest()
            with mock.patch.object(audit, "ROOT", root):
                with mock.patch.object(audit, "source_identity", return_value={
                    "sha256": "test", "files": {"jwc/main.jw": digest}}):
                    result = audit.check("jwc")
            self.assertEqual(result["status"], "DECLARED_CLOSURE_CHECKED")
            self.assertFalse(result["build_dependency_closure_proven"])


if __name__ == "__main__":
    unittest.main()
