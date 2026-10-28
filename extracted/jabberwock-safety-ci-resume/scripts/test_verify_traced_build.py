"""Traced-build composition must reject incomplete or failed monitor evidence."""
import hashlib
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock
sys.path.insert(0, str(Path(__file__).resolve().parent))
import verify_traced_build as gate


class TracedBuildTests(unittest.TestCase):
    def test_rejects_capture_failure_without_auditing(self):
        with tempfile.TemporaryDirectory() as d:
            with mock.patch.object(gate, "collect", return_value={
                    "status": "BLOCKED", "errors": ["trace truncated"]}):
                with mock.patch.object(gate, "check_events") as check:
                    with self.assertRaisesRegex(ValueError, "capture failed"):
                        gate.verify_build(["build"], Path(d), {}, "jwc",
                                          Path(d) / "trace.json", {})
                    check.assert_not_called()

    def test_rejects_missing_normalized_evidence(self):
        with tempfile.TemporaryDirectory() as d:
            with mock.patch.object(gate, "collect", return_value={
                    "status": "CAPTURED_UNVERIFIED", "normalized_events": None}):
                with self.assertRaisesRegex(ValueError, "normalized events"):
                    gate.verify_build(["build"], Path(d), {}, "jwc",
                                      Path(d) / "trace.json", {})

    def test_successful_evidence_preserves_unproved_completeness(self):
        with tempfile.TemporaryDirectory() as d:
            with mock.patch.object(gate, "collect", return_value={
                    "status": "CAPTURED_UNVERIFIED",
                    "normalized_events": {"events": []},
                    "trace_sha256": hashlib.sha256(b"trace").hexdigest()}):
                with mock.patch.object(gate, "check_events", return_value={
                        "status": "RECORDED_ACCESSES_ACCOUNTED_FOR"}):
                    result = gate.verify_build(["build"], Path(d), {}, "jwc",
                                               Path(d) / "trace.json", {})
            self.assertEqual(result["status"], "RECORDED_ACCESSES_ACCOUNTED_FOR")
            self.assertFalse(result["complete_build_closure_proven"])

if __name__ == "__main__":
    unittest.main()
