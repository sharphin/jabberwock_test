"""Linux build capture must fail closed for unsupported environments."""
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock
sys.path.insert(0, str(Path(__file__).resolve().parent))
import linux_build_trace as trace


class BuildTraceTests(unittest.TestCase):
    def test_requires_linux_and_trace_tool(self):
        with tempfile.TemporaryDirectory() as d:
            with mock.patch.object(trace.sys, "platform", "win32"):
                with self.assertRaisesRegex(ValueError, "requires Linux"):
                    trace.collect(["fake"], Path(d), Path(d) / "report.json")
            with mock.patch.object(trace.shutil, "which", return_value=None):
                with mock.patch.object(trace.sys, "platform", "linux"):
                    with self.assertRaisesRegex(ValueError, "strace unavailable"):
                        trace.collect(["fake"], Path(d), Path(d) / "report.json")

    def test_fails_on_unparsed_lines_and_nonzero_build_exit(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            output = root / "audit.json"
            def fake_run(command, **kwargs):
                Path(command[command.index("-o") + 1]).write_text(
                    "bad trace fragment\nopenat(AT_FDCWD, \"missing\", O_RDONLY) = -1 ENOENT\n",
                    encoding="utf-8")
                return type("Result", (), {"returncode": 3})()
            with mock.patch.object(trace.sys, "platform", "linux"):
                with mock.patch.object(trace.shutil, "which", return_value="/bin/strace"):
                    with mock.patch.object(trace.subprocess, "run", side_effect=fake_run):
                        result = trace.collect(["builder"], root, output)
            self.assertEqual(result["status"], "BLOCKED")
            self.assertFalse(result["complete_build_closure_proven"])
            self.assertTrue(any("unparsed trace line" in error for error in result["errors"]))


if __name__ == "__main__":
    unittest.main()
