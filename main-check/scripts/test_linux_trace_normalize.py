"""Controls for restricted Linux build trace normalization."""
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from linux_trace_normalize import normalize


class TraceNormalizeTests(unittest.TestCase):
    def test_root_pid_and_child_process_inputs(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            trace = [
                '123 clone(child_stack=NULL, flags=SIGCHLD) = 124',
                '124 openat(AT_FDCWD, "source.jw", O_RDONLY) = 3',
                '123 execve("/bin/echo", ["echo"], 0x1) = 0']
            result = normalize(trace, root)
            self.assertEqual([event["type"] for event in result["events"]],
                             ["begin", "read", "exec", "end"])
            self.assertFalse(result["monitor_completeness_proven"])

    def test_unknown_process_and_dirfd_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            with self.assertRaisesRegex(ValueError, "untracked process"):
                normalize(['123 open("one", O_RDONLY) = 3',
                           '124 open("two", O_RDONLY) = 3'], root)
            with self.assertRaisesRegex(ValueError, "directory descriptor"):
                normalize(['123 openat(3, "foo", O_RDONLY) = 4'], root)

    def test_unknown_syscall_and_empty_trace_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            with self.assertRaisesRegex(ValueError, "empty syscall"):
                normalize([], root)
            with self.assertRaisesRegex(ValueError, "unsupported syscall"):
                normalize(['123 mmap(NULL, 4096, PROT_READ) = 0'], root)

if __name__ == "__main__":
    unittest.main()
