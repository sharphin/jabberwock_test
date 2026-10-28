"""Tests for transitive build closure, including negative graph controls."""
import errno
import hashlib
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
from build_closure_graph import audit


class BuildGraphTests(unittest.TestCase):
    def graph(self, root):
        (root / "tool.bin").write_bytes(b"tool")
        (root / "input.jw").write_bytes(b"source")
        h = lambda b: hashlib.sha256(b).hexdigest()
        return {
            "schema_version": 1,
            "nodes": {
                "compiler": {"kind": "tool", "path": "tool.bin", "sha256": h(b"tool")},
                "source": {"kind": "file", "path": "input.jw", "sha256": h(b"source")},
                "build": {"kind": "action", "inputs": ["compiler", "source"],
                          "command": ["tool.bin", "input.jw"], "outputs": ["out.ll"]}},
            "targets": ["build"]}

    def test_walks_and_hashes_complete_declared_dag(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            result = audit(self.graph(root), root, {})
            self.assertEqual(result["visited_nodes"], 3)
            self.assertEqual(result["status"], "DECLARED_CLOSURE_VERIFIED")
            self.assertFalse(result["observed_inputs_complete"])

    def test_source_identity_missing_from_graph_blocks(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            graph = self.graph(root)
            required = {"src/lib.rs": hashlib.sha256(b"library").hexdigest()}
            with mock.patch("build_closure_graph.source_identity",
                            return_value={"files": required}):
                with self.assertRaisesRegex(ValueError, "missing compiler source leaves"):
                    audit(graph, root, {}, compiler="rust")

    def test_missing_input_blocks(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            graph = self.graph(root)
            graph["nodes"]["build"]["inputs"].append("missing")
            with self.assertRaisesRegex(ValueError, "missing graph node"):
                audit(graph, root, {})

    def test_unreachable_node_blocks(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            graph = self.graph(root)
            graph["nodes"]["forgotten"] = {"kind": "file", "path": "input.jw",
                                             "sha256": hashlib.sha256(b"source").hexdigest()}
            with self.assertRaisesRegex(ValueError, "unreachable"):
                audit(graph, root, {})

    def test_mutated_file_blocks(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            graph = self.graph(root)
            (root / "input.jw").write_bytes(b"changed")
            with self.assertRaisesRegex(ValueError, "hash mismatch"):
                audit(graph, root, {})

    def test_cycle_blocks(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            graph = self.graph(root)
            graph["nodes"]["build"]["inputs"].append("build")
            with self.assertRaisesRegex(ValueError, "cyclic dependency"):
                audit(graph, root, {})

    def test_duplicate_outputs_block(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            graph = self.graph(root)
            graph["nodes"]["build"]["outputs"] = ["out.ll", "out.ll"]
            with self.assertRaisesRegex(ValueError, "duplicate action outputs"):
                audit(graph, root, {})

    def test_output_path_traversal_blocks(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            graph = self.graph(root)
            graph["nodes"]["build"]["outputs"] = ["../out.ll"]
            with self.assertRaisesRegex(ValueError, "unsafe action output"):
                audit(graph, root, {})

    def test_output_overwrites_source_blocks(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            graph = self.graph(root)
            graph["nodes"]["build"]["outputs"] = ["input.jw"]
            with self.assertRaisesRegex(ValueError, "overwrites tracked input"):
                audit(graph, root, {})

    def test_two_producers_of_same_output_block(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            graph = self.graph(root)
            graph["nodes"]["other"] = {
                "kind": "action", "inputs": ["compiler", "source"],
                "command": ["tool.bin", "input.jw"], "outputs": ["out.ll"]}
            graph["targets"].append("other")
            with self.assertRaisesRegex(ValueError, "multiple producers"):
                audit(graph, root, {})


    def test_windows_absolute_paths_block_on_any_host(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            graph = self.graph(root)
            graph["nodes"]["build"]["outputs"] = [r"C:\\outside.ll"]
            with self.assertRaisesRegex(ValueError, "unsafe action output"):
                audit(graph, root, {})
            graph["nodes"]["build"]["outputs"] = ["out.ll"]
            graph["nodes"]["source"]["path"] = r"C:\\source.jw"
            with self.assertRaisesRegex(ValueError, "absolute/traversal input path"):
                audit(graph, root, {})

    def test_backslash_path_aliases_block(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            graph = self.graph(root)
            graph["nodes"]["build"]["outputs"] = [r"a\\..\\out.ll"]
            with self.assertRaisesRegex(ValueError, "unsafe action output"):
                audit(graph, root, {})


    def test_output_symlink_escape_blocks(self):
        with tempfile.TemporaryDirectory() as d, tempfile.TemporaryDirectory() as outside:
            root = Path(d)
            graph = self.graph(root)
            try:
                (root / "redirect").symlink_to(Path(outside), target_is_directory=True)
            except NotImplementedError as error:
                self.skipTest("directory symlinks unavailable on this host: " + str(error))
            except OSError as error:
                if (getattr(error, "winerror", None) == 1314 or
                        error.errno in (errno.EPERM, errno.EACCES)):
                    self.skipTest("directory symlink privilege unavailable on this host: " + str(error))
                raise
            graph["nodes"]["build"]["outputs"] = ["redirect/out.ll"]
            with self.assertRaisesRegex(ValueError, "action output escapes root"):
                audit(graph, root, {})

    def test_noncanonical_output_components_block(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            graph = self.graph(root)
            for unsafe in ("a/./out.ll", "a//out.ll", "./out.ll", "a/../out.ll"):
                with self.subTest(unsafe=unsafe):
                    graph["nodes"]["build"]["outputs"] = [unsafe]
                    with self.assertRaisesRegex(ValueError, "unsafe action output"):
                        audit(graph, root, {})

    def test_nul_and_backslash_inputs_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            graph = self.graph(root)
            for candidate in ("input" + chr(0) + ".jw", "dir" + chr(92) + "input.jw"):
                with self.subTest(candidate=repr(candidate)):
                    graph["nodes"]["source"]["path"] = candidate
                    with self.assertRaisesRegex(ValueError, "absolute/traversal input path"):
                        audit(graph, root, {})

    def test_duplicate_tracked_leaf_path_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            graph = self.graph(root)
            graph["nodes"]["source-alias"] = {
                "kind": "file", "path": "input.jw",
                "sha256": hashlib.sha256(b"source").hexdigest()}
            graph["nodes"]["build"]["inputs"].append("source-alias")
            with self.assertRaisesRegex(ValueError, "duplicate tracked file/tool path"):
                audit(graph, root, {})

    def test_case_folded_output_collision_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            graph = self.graph(root)
            graph["nodes"]["other"] = {
                "kind": "action", "inputs": ["compiler", "source"],
                "command": ["tool.bin"], "outputs": ["OUT.ll"]}
            graph["targets"].append("other")
            with self.assertRaisesRegex(ValueError, "multiple producers"):
                audit(graph, root, {})

    def test_case_folded_input_collision_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            graph = self.graph(root)
            (root / "INPUT.jw").write_bytes(b"source")
            graph["nodes"]["alias"] = {
                "kind": "file", "path": "INPUT.jw",
                "sha256": hashlib.sha256(b"source").hexdigest()}
            graph["nodes"]["build"]["inputs"].append("alias")
            with self.assertRaisesRegex(ValueError, "duplicate tracked file/tool path"):
                audit(graph, root, {})

    def test_untracked_executable_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            graph = self.graph(root)
            graph["nodes"]["build"]["command"][0] = "untracked-tool"
            with self.assertRaisesRegex(ValueError, "executable not tracked"):
                audit(graph, root, {})

if __name__ == "__main__":
    unittest.main()
