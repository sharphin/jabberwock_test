"""Proof status changes require a trusted verifier, never self-attestation."""
import copy
import tempfile
import sys
from pathlib import Path
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
import refinement_transition as transition


class ProofTransitionTests(unittest.TestCase):
    def setUp(self):
        self.row = {"id": "jwc:semantics:jwc/main.jw", "source_sha256": "abc"}

    def artifact(self):
        return {"obligation_id": self.row["id"], "source_sha256": "abc",
                "required_claims": sorted(transition.REQUIRED),
                "verifier": "checked"}

    def test_absent_untrusted_or_stale_proof_never_proven(self):
        self.assertEqual(transition.status_for(self.row, {})["status"], "REFINEMENT_NOT_PROVEN")
        proof = self.artifact()
        self.assertEqual(transition.status_for(self.row, {self.row["id"]: proof})["status"],
                         "REFINEMENT_NOT_PROVEN")
        proof["source_sha256"] = "old"
        self.assertEqual(transition.status_for(self.row, {self.row["id"]: proof})["status"],
                         "INVALID_EVIDENCE")

    def test_missing_claim_is_invalid(self):
        proof = self.artifact()
        proof["required_claims"].pop()
        self.assertEqual(transition.status_for(self.row, {self.row["id"]: proof},
                                               {"checked": lambda *args: True})["status"],
                         "INVALID_EVIDENCE")

    def test_malformed_duplicate_and_string_claims_rejected(self):
        for claims in ("source_refinement", ["source_refinement"] * 5,
                       [*sorted(transition.REQUIRED), "kernel_proof"], None,
                       [["not-hashable"]]):
            with self.subTest(claims=claims):
                proof = self.artifact()
                proof["required_claims"] = claims
                self.assertEqual(
                    transition.status_for(self.row, {self.row["id"]: proof},
                                          {"checked": lambda *args: True})["status"],
                    "INVALID_EVIDENCE")

    def test_checker_must_actually_return_true(self):
        proof = self.artifact()
        for result, expected in ((False, "REFINEMENT_NOT_PROVEN"),
                                 (True, "REFINEMENT_PROVEN")):
            with self.subTest(result=result):
                self.assertEqual(transition.status_for(
                    self.row, {self.row["id"]: proof},
                    {"checked": lambda *args: result})["status"], expected)

    def test_registered_checker_exception_rejects(self):
        proof = self.artifact()
        def fail(*args):
            raise ValueError("invalid theorem")
        self.assertEqual(transition.status_for(self.row, {self.row["id"]: proof},
                                               {"checked": fail})["status"],
                         "INVALID_EVIDENCE")

    def test_duplicate_and_foreign_proofs_rejected(self):
        base = {"components": [self.row]}
        with mock.patch.object(transition, "validate", return_value=None):
            proof = self.artifact()
            with self.assertRaises(ValueError):
                transition.evaluate(base, {"schema_version": 1, "proofs": [proof, proof]})
            foreign = copy.deepcopy(proof)
            foreign["obligation_id"] = "foreign"
            with self.assertRaises(ValueError):
                transition.evaluate(base, {"schema_version": 1, "proofs": [foreign]})


    def test_cli_unproved_exits_nonzero(self):
        with tempfile.TemporaryDirectory() as directory:
            dest = Path(directory) / "audit.json"
            with mock.patch.object(transition, "inventory", return_value={"components": []}), \
                 mock.patch.object(transition, "evaluate", return_value={
                     "schema_version": 1, "status": "NOT_PROVEN", "required": 1,
                     "proved": 0, "invalid": 0, "findings": []}):
                self.assertEqual(transition.main(["--output", str(dest)]), 1)
                self.assertEqual(__import__("json").loads(dest.read_text())["status"], "NOT_PROVEN")

    def test_cli_invalid_evidence_exits_two(self):
        with tempfile.TemporaryDirectory() as directory:
            dest = Path(directory) / "audit.json"
            with mock.patch.object(transition, "inventory", side_effect=ValueError("bad inventory")):
                self.assertEqual(transition.main(["--output", str(dest)]), 2)


class ElisionBridgeTests(unittest.TestCase):
    """消した検査の検査器（elision-certificate-v1）は部分の確かめ。再生と食い違えば INVALID、通っても証明済みにしない"""
    def setUp(self):
        import elision_bridge
        self.bridge = elision_bridge
        self.row = {"id": elision_bridge.OBLIGATION, "source_sha256": "abc"}
        self.summary = {"subject": "jwc", "status": "VERIFIED", "checked": 2,
                        "results": [{"id": "jwc/main.jw", "compiler": "jwc", "status": "pass"}]}
        self.record = {"obligation_id": self.row["id"], "source_sha256": "abc",
                       "required_claims": sorted(transition.REQUIRED), "verifier": elision_bridge.VERIFIER,
                       "rust_compiler": sys.executable, "summary": copy.deepcopy(self.summary)}

    def test_unrelated_obligation_is_invalid(self):
        row = {"id": "jwc:checks:jwc/main.jw", "source_sha256": "abc"}
        record = dict(self.record, obligation_id=row["id"])
        self.assertEqual(self.bridge.check(row, record, run=lambda rust: self.summary)["status"], "INVALID_EVIDENCE")

    def test_replay_mismatch_is_invalid(self):
        replayed = dict(self.summary, checked=3)
        self.assertEqual(self.bridge.check(self.row, self.record, run=lambda rust: replayed)["status"],
                         "INVALID_EVIDENCE")

    def test_matching_replay_is_a_subset_never_proven(self):
        result = self.bridge.check(self.row, self.record, run=lambda rust: copy.deepcopy(self.summary))
        self.assertEqual(result["status"], "REFINEMENT_NOT_PROVEN")
        self.assertEqual(result["verified_subset"], "jwc.analyze:elided-generation-checks:corpus")
        with mock.patch.object(self.bridge, "replay", return_value=copy.deepcopy(self.summary)):
            self.assertEqual(transition.status_for(self.row, {self.row["id"]: self.record})["status"],
                             "REFINEMENT_NOT_PROVEN")


if __name__ == "__main__":
    unittest.main()
