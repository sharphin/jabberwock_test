"""Fail-closed local acceptance matrix tests. No compiler or Lean required."""
from pathlib import Path
import sys
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parent))
import verify_all_local as verify


def fixture():
    compilers = list(verify.COMPILERS)
    items = [{"compiler": c, "item": n, "status": "PROBES_PASS_ONLY", "acceptance": "BLOCKED"}
             for c in compilers for n in range(1, 13)]
    proofs = [{"compiler": c, "goal": g, "acceptance": "NOT_PROVEN",
               "proof_check": {"status": "KERNEL_CHECKED"}}
              for c in compilers for g in verify.GOALS]
    practical = {"run_status": "COMPLETE", "suite": {"compilers": compilers}, "items": items}
    formal = {"compilers": {c: {} for c in compilers}, "goals": proofs}
    return practical, formal


class LocalVerifierTests(unittest.TestCase):
    def test_windows_entry_invokes_toolchain_initializing_batch(self):
        command = verify.practical_command_for("windows", Path("result.json"))
        self.assertEqual(command[:3], ["cmd", "/d", "/c"])
        self.assertTrue(command[3].endswith("safety_windows.cmd"))
        self.assertEqual(command[4:], ["--compiler", "all", "--output", "result.json"])
        self.assertTrue(verify.practical_command_for("linux", Path("result.json"))[1].endswith("safety_linux.py"))
        with self.assertRaises(ValueError):
            verify.practical_command_for("macos", Path("result.json"))

    def test_lean_tool_paths_are_available_on_windows(self):
        import os
        search = verify.formal_environment_for("windows")["PATH"]
        self.assertIn(".elan", search)
        self.assertIn(".cargo", search)
        self.assertEqual(verify.formal_environment_for("linux")["PATH"], os.environ.get("PATH", ""))

    def test_unproved_with_complete_counts(self):
        practical, formal = fixture()
        result = verify.judge(practical, formal)
        self.assertEqual(result["status"], "NOT_PROVEN")
        self.assertEqual((result["required_practical"], result["required_formal"]), (24, 10))

    def test_forged_cross_compiler_or_duplicate_evidence_is_rejected(self):
        for corrupt in ("item", "goal", "compiler"):
            p, f = fixture()
            if corrupt == "item":
                p["items"][-1] = dict(p["items"][0])
            elif corrupt == "goal":
                f["goals"][-1] = dict(f["goals"][0])
            else:
                f["compilers"] = {"jwc": {}}
            self.assertEqual(verify.judge(p, f)["status"], "INVALID_EVIDENCE")

    def test_absent_and_incomplete_reports_are_not_accepted(self):
        p, f = fixture()
        self.assertEqual(verify.judge({}, f)["status"], "INVALID_EVIDENCE")
        p["run_status"] = "RUNNING"
        self.assertEqual(verify.judge(p, f)["status"], "NOT_RUN")

    def test_stage_failures_are_not_collapsed_into_generic_unproved(self):
        p, f = fixture()
        p["items"][0]["status"] = "FAIL"
        self.assertEqual(verify.judge(p, f)["status"], "PROBE_FAILURE")
        p["items"][0]["status"] = "NOT_RUN"
        self.assertEqual(verify.judge(p, f)["status"], "NOT_RUN")
        p["items"][0]["status"] = "PROBES_PASS_ONLY"
        f["goals"][0]["proof_check"]["status"] = "STALE"
        self.assertEqual(verify.judge(p, f)["status"], "PROOF_STALE")
        f["goals"][0]["proof_check"]["status"] = "FAIL"
        self.assertEqual(verify.judge(p, f)["status"], "PROOF_CHECK_FAILURE")

    def test_all_failure_classes_are_visible(self):
        p, f = fixture()
        for state, expected in (("FAIL", "PROBE_FAILURE"),
                                ("CRASH", "PROBE_FAILURE"),
                                ("TIMEOUT", "PROBE_FAILURE"),
                                ("INFRA_ERROR", "INFRA_ERROR"),
                                ("INTERRUPTED", "INTERRUPTED"),
                                ("UNKNOWN", "INVALID_EVIDENCE")):
            p["items"][0]["status"] = state
            with self.subTest(state=state):
                self.assertEqual(verify.judge(p, f)["status"], expected)
        p["items"][0]["status"] = "PROBES_PASS_ONLY"
        f["infrastructure_error"] = "Lean unavailable"
        self.assertEqual(verify.judge(p, f)["status"], "INFRA_ERROR")

    def test_kernel_only_is_never_formal_acceptance(self):
        p, f = fixture()
        for x in p["items"]:
            x["acceptance"] = "PASS"
        self.assertEqual(verify.judge(p, f)["status"], "NOT_PROVEN")
        self.assertEqual(verify.judge(p, f)["verified_formal"], 0)
        for x in f["goals"]:
            x["acceptance"] = "PROVEN"
        result = verify.judge(p, f)
        self.assertEqual(result["status"], "NOT_PROVEN")
        self.assertEqual(result["claimed_formal"], 10)
        self.assertEqual(result["verified_formal"], 0)


if __name__ == "__main__":
    unittest.main()
