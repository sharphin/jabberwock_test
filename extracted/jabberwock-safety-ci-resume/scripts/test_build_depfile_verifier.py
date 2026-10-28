"""Unit tests for observed build dependency closure verification."""
import hashlib
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
import build_depfile_verifier as gate


class BuildDepfileVerifierTests(unittest.TestCase):
    def test_missing_hashed_dependency_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / "main.rs").write_text("main", encoding="utf-8")
            (root / "more.rs").write_text("more", encoding="utf-8")
            deps = root / "deps"
            deps.mkdir()
            (deps / "build.d").write_text("out: ../main.rs ../more.rs\n", encoding="utf-8")
            digest = hashlib.sha256(b"main").hexdigest()
            manifest = {"schema_version": 1, "compiler": "rust",
                        "files": {"main.rs": digest}, "depfiles": ["build.d"]}
            with mock.patch.object(gate, "source_identity", return_value={
                    "files": {"main.rs": digest}}):
                with self.assertRaisesRegex(ValueError, "unhashed observed dependency"):
                    gate.verify(manifest, deps, root)

    def test_existing_depfile_is_evidence_not_complete_build_proof(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / "main.rs").write_text("main", encoding="utf-8")
            deps = root / "deps"
            deps.mkdir()
            (deps / "build.d").write_text("out: ../main.rs\n", encoding="utf-8")
            digest = hashlib.sha256(b"main").hexdigest()
            manifest = {"schema_version": 1, "compiler": "rust",
                        "files": {"main.rs": digest}, "depfiles": ["build.d"]}
            with mock.patch.object(gate, "source_identity", return_value={
                    "files": {"main.rs": digest}}):
                result = gate.verify(manifest, deps, root)
            self.assertEqual(result["status"], "DEPFILE_INPUTS_VERIFIED")
            self.assertFalse(result["complete_build_closure_proven"])

    def test_unsafe_depfile_name_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaises(ValueError):
                gate.verify({"schema_version": 1, "compiler": "rust",
                             "files": {"a": "0" * 64}, "depfiles": ["../escape.d"]},
                            Path(d))


if __name__ == "__main__":
    unittest.main()
