#!/usr/bin/env python3
"""Native measurement helper smoke tests; no Jabberwock compiler required."""
from pathlib import Path
import os
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "bench" / "measure.c"


@unittest.skipUnless(os.name == "posix" and shutil.which("cc"), "POSIX C compiler required")
class MeasureHelperTests(unittest.TestCase):
    def test_repetition_outputs_and_argument_failures(self):
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp)
            binary = folder / "measure"
            build = subprocess.run(["cc", "-O2", "-o", str(binary), str(SOURCE)],
                                   capture_output=True, text=True)
            self.assertEqual(build.returncode, 0, build.stderr)
            stdout = folder / "result.stdout"
            measured = subprocess.run([str(binary), "3", str(stdout),
                                       "/bin/sh", "-c", "printf 'same\n'"],
                                      capture_output=True, text=True)
            self.assertEqual(measured.returncode, 0, measured.stderr)
            self.assertEqual(len(measured.stdout.split()), 4)
            for output in (stdout, Path(str(stdout) + ".run0"),
                           Path(str(stdout) + ".run1")):
                self.assertEqual(output.read_bytes(), b"same\n")
            for count in ("0", "-1", "garbage", "100001"):
                result = subprocess.run([str(binary), count, str(stdout), "/bin/true"],
                                        capture_output=True)
                self.assertNotEqual(result.returncode, 0)


if __name__ == "__main__":
    unittest.main()
