"""Unit tests for portable verification evidence archives."""
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parent))
from package_verification import package


class EvidenceBundleTests(unittest.TestCase):
    def test_packaging_failing_run_keeps_raw_evidence(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            source = root / "verification-complete"
            source.mkdir()
            summary = {"schema_version": 2, "kind": "local-full-verification",
                       "status": "FAIL", "stages": []}
            (source / "summary.json").write_text(json.dumps(summary), encoding="utf-8")
            (source / "formal.stderr.log").write_bytes(b"test failure")
            output = root / "evidence.zip"
            result = package(source, output)
            self.assertEqual(result["verdict"], "FAIL")
            with zipfile.ZipFile(output) as archive:
                self.assertEqual(archive.read("formal.stderr.log"), b"test failure")
                manifest = json.loads(archive.read("evidence-manifest.json"))
                self.assertEqual(manifest["files"]["formal.stderr.log"]["sha256"],
                                 hashlib.sha256(b"test failure").hexdigest())

    def test_missing_or_invalid_summary_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            source = root / "evidence"
            source.mkdir()
            output = root / "evidence.zip"
            with self.assertRaises(ValueError):
                package(source, output)
            (source / "summary.json").write_text('{"status":"PASS"}', encoding="utf-8")
            with self.assertRaises(ValueError):
                package(source, output)

    def test_archive_cannot_be_placed_among_evidence(self):
        with tempfile.TemporaryDirectory() as d:
            source = Path(d)
            (source / "summary.json").write_text(
                json.dumps({"schema_version": 2,
                            "kind": "local-full-verification", "status": "FAIL"}),
                encoding="utf-8")
            with self.assertRaises(ValueError):
                package(source, source / "nested.zip")


if __name__ == "__main__":
    unittest.main()
