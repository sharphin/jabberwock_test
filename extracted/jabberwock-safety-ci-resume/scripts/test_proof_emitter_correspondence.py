"""Negative controls for the real-source LLVM generation-check correspondence audit."""
import sys
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
import proof_emitter_correspondence as bind


class EmittedGateTests(unittest.TestCase):
    def test_both_real_emitter_templates_match_the_declared_guard_shape(self):
        for compiler in ("jwc", "rust"):
            with self.subTest(compiler=compiler):
                record = bind.validate(compiler)
                self.assertEqual(record["compiler"], compiler)
                self.assertEqual(record["status"], "EMITTER_GATE_STRUCTURE_MATCHED")
                self.assertEqual(len(record["sha256"]), 64)

    def test_missing_retirement_rejection_fails(self):
        for compiler in ("jwc", "rust"):
            path, code = bind.emitter_segment(compiler)
            mutated = code.replace("icmp eq i64 ", "icmp sle i64 ", 1)
            with self.subTest(compiler=compiler), mock.patch.object(
                    bind, "emitter_segment", return_value=(path, mutated)):
                with self.assertRaises(ValueError):
                    bind.validate(compiler)

    def test_changed_branch_polarity_fails(self):
        for compiler in ("jwc", "rust"):
            path, code = bind.emitter_segment(compiler)
            mutated = code.replace("if_true", "if_false", 1)
            with self.subTest(compiler=compiler), mock.patch.object(
                    bind, "emitter_segment", return_value=(path, mutated)):
                with self.assertRaises(ValueError):
                    bind.validate(compiler)

    def test_removed_atomicity_or_trap_fails(self):
        for compiler in ("jwc", "rust"):
            path, code = bind.emitter_segment(compiler)
            for altered in (code.replace("monotonic, align 8", "non_atomic, align 8", 1),
                            code.replace("jw_fail_dangling", "jw_return_dangling", 1)):
                with self.subTest(compiler=compiler), mock.patch.object(
                        bind, "emitter_segment", return_value=(path, altered)):
                    with self.assertRaises(ValueError):
                        bind.validate(compiler)


    def test_duplicate_gate_fragments_are_rejected(self):
        for compiler in ("jwc", "rust"):
            path, code = bind.emitter_segment(compiler)
            for fragment in bind.STEPS:
                mutated = code + "\\n" + fragment
                with self.subTest(compiler=compiler, fragment=fragment), mock.patch.object(
                        bind, "emitter_segment", return_value=(path, mutated)):
                    with self.assertRaises(ValueError):
                        bind.validate(compiler)

if __name__ == "__main__":
    unittest.main()
