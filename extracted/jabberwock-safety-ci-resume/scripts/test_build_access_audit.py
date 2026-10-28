"""Negative and positive controls for recorded build access correlation."""
import hashlib
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock
sys.path.insert(0, str(Path(__file__).resolve().parent))
from build_access_audit import check_events


class AccessEventsTests(unittest.TestCase):
    def fixtures(self, root):
        source = root / "main.jw"
        tool = root / "builder"
        source.write_bytes(b"source")
        tool.write_bytes(b"tool")
        sha = lambda value: hashlib.sha256(value).hexdigest()
        graph = {"schema_version": 1, "targets": ["build"], "nodes": {
            "input": {"kind": "file", "path": "main.jw", "sha256": sha(b"source")},
            "compiler": {"kind": "tool", "path": "builder", "sha256": sha(b"tool")},
            "build": {"kind": "action", "inputs": ["input", "compiler"],
                      "command": ["builder", "main.jw"], "outputs": ["out.ll"]}}}
        sequence = {"schema_version": 1, "kind": "build-access-events", "events": [
            {"type": "begin"}, {"type": "exec", "path": str(tool)},
            {"type": "read", "path": str(source)}, {"type": "end"}]}
        return graph, sequence, sha

    def test_recorded_inputs_accepted_without_exhaustiveness_claim(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            graph, sequence, sha = self.fixtures(root)
            with mock.patch("build_closure_graph.source_identity", return_value={
                    "files": {"main.jw": sha(b"source")}}):
                result = check_events(graph, sequence, root, "jwc", {})
            self.assertEqual(result["status"], "RECORDED_ACCESSES_ACCOUNTED_FOR")
            self.assertFalse(result["complete_build_closure_proven"])

    def test_unregistered_read_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            graph, sequence, sha = self.fixtures(root)
            sequence["events"].insert(-1, {"type": "read", "path": str(root / "secret")})
            with mock.patch("build_closure_graph.source_identity", return_value={
                    "files": {"main.jw": sha(b"source")}}):
                with self.assertRaisesRegex(ValueError, "unregistered file read"):
                    check_events(graph, sequence, root, "jwc", {})

    def test_missing_end_marker_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            graph, sequence, sha = self.fixtures(root)
            sequence["events"].pop()
            with mock.patch("build_closure_graph.source_identity", return_value={
                    "files": {"main.jw": sha(b"source")}}):
                with self.assertRaisesRegex(ValueError, "observation boundaries"):
                    check_events(graph, sequence, root, "jwc", {})


if __name__ == "__main__":
    unittest.main()
