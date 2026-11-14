"""Check that build observation-to-DAG correlation fails closed."""
import hashlib
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
import build_observation_correlation as corr


class CorrelationTests(unittest.TestCase):
    def fixtures(self, root):
        depdir = root / "deps"
        depdir.mkdir()
        (root / "source.jw").write_bytes(b"source")
        (root / "tool.exe").write_bytes(b"tool")
        (depdir / "build.d").write_text("out: source.jw\n", encoding="utf-8")
        h = lambda b: hashlib.sha256(b).hexdigest()
        graph = {"schema_version": 1, "targets": ["build"], "nodes": {
            "source": {"kind": "file", "path": "source.jw", "sha256": h(b"source")},
            "tool": {"kind": "tool", "path": "tool.exe", "sha256": h(b"tool")},
            "build": {"kind": "action", "inputs": ["source", "tool"],
                      "command": ["tool.exe", "source.jw"], "outputs": ["out.ll"]}}}
        manifest = {"schema_version": 1, "compiler": "jwc",
                    "files": {"source.jw": h(b"source")},
                    "depfiles": ["build.d"]}
        return graph, manifest, depdir, h

    def test_observed_source_correlates_with_graph_without_claiming_completion(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            graph, manifest, depdir, h = self.fixtures(root)
            with mock.patch("build_closure_graph.source_identity", return_value={
                    "files": {"source.jw": h(b"source")}}):
                with mock.patch("build_depfile_verifier.source_identity", return_value={
                        "files": {"source.jw": h(b"source")}}):
                    result = corr.correlate(graph, manifest, root=root, depdir=depdir,
                                            build_cwd=root, environment={}, compiler="jwc")
            self.assertEqual(result["status"], "OBSERVED_INPUTS_MATCH_DECLARED_GRAPH")
            self.assertFalse(result["complete_build_closure_proven"])

    def test_dependency_not_in_graph_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            graph, manifest, depdir, h = self.fixtures(root)
            (root / "other.jw").write_bytes(b"other")
            manifest["files"]["other.jw"] = h(b"other")
            (depdir / "build.d").write_text("out: source.jw other.jw\n", encoding="utf-8")
            with mock.patch("build_closure_graph.source_identity", return_value={
                    "files": {"source.jw": h(b"source")}}):
                with mock.patch("build_depfile_verifier.source_identity", return_value={
                        "files": {"source.jw": h(b"source")}}):
                    with self.assertRaisesRegex(ValueError, "absent from DAG"):
                        corr.correlate(graph, manifest, root=root, depdir=depdir,
                                       build_cwd=root, environment={}, compiler="jwc")


if __name__ == "__main__":
    unittest.main()
