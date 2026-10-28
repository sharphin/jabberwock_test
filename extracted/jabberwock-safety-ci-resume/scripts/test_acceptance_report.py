"""Negative controls for the 34 independent compiler acceptance obligations."""
import copy
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import acceptance_report as audit
import acceptance_requirements as req


def fixture():
    items = []
    goals = []
    direct_goals = []
    probes = []
    plan = []
    for compiler in req.COMPILERS:
        for n in range(1, 13):
            items.append({"id": f"windows-{compiler}-{n:02}", "compiler": compiler,
                          "item": n, "target_os": "windows", "probe_names": [f"{compiler}-probe-{n}"],
                          "status": "PROBES_PASS_ONLY", "acceptance": "BLOCKED",
                          "coverage_gaps": ["whole feature verification missing"]})
            probes.append({"name": f"{compiler}-probe-{n}", "compiler": compiler,
                           "items": [n], "status": "PROBE_PASS", "actual": {"exit_code": 0}})
            plan.append({"name": f"{compiler}-probe-{n}", "compiler": compiler, "items": [n], "goal": None})
        for goal in req.GOALS:
            direct_goals.append({"id": f"windows-{compiler}-{goal}",
                                 "compiler": compiler, "status": "PROBES_PASS_ONLY",
                                 "direct_probe_names": [f"{compiler}-goal-{goal}"]})
            probes.append({"name": f"{compiler}-goal-{goal}", "compiler": compiler,
                           "items": [], "status": "PROBE_PASS", "actual": {"exit_code": 0}})
            plan.append({"name": f"{compiler}-goal-{goal}", "compiler": compiler, "items": [], "goal": goal})
            goals.append({"id": compiler + "-" + goal, "compiler": compiler,
                          "goal": goal, "acceptance": "NOT_PROVEN",
                          "proof_check": {"status": "KERNEL_CHECKED"},
                          "source_sha256": compiler + "-sha",
                          "implementation_correspondence": "MANUAL_MODEL_NOT_PROVEN",
                          "excluded": ["unmodelled LLVM"], "theorems": [goal]})
    practical = {"schema_version": 3, "suite": {"target_os": "windows",
                  "compilers": list(req.COMPILERS)}, "run_status": "COMPLETE",
                  "active_probe": None, "items": items, "goals": direct_goals,
                  "results": probes, "planned_probes": plan}
    formal = {"schema_version": 3, "compilers": {c: {"source": {"sha256": c + "-sha"}}
                                               for c in req.COMPILERS},
              "goals": goals}
    formal["proof_sources_snapshot"] = audit.proof_sources_identity()
    identity = {c: {"sha256": c + "-sha", "files": {"runtime/jw_rt.c": "demo"}} for c in req.COMPILERS}
    practical["source_snapshots"] = copy.deepcopy(identity)
    practical["test_inputs_snapshot"] = {"sha256": "fixture-sha", "files": {"scripts/harness.py": "hash"}}
    return practical, formal, identity


class AcceptanceRequirementsTests(unittest.TestCase):
    def check(self, practical, formal, identity):
        return audit.review(practical, formal, local_os="windows", identities=identity)

    def test_complete_catalog_is_24_plus_10(self):
        cases = req.obligation_catalog()
        self.assertEqual(len(cases), 34)
        self.assertEqual(len({c["id"] for c in cases}), 34)
        self.assertEqual(sum(c["kind"] == "practical" for c in cases), 24)
        self.assertEqual(sum(c["kind"] == "formal" for c in cases), 10)
        self.assertTrue(all(c["required_statement"] for c in cases))

    def test_mock_kernel_and_all_probes_still_not_whole_implementation_proof(self):
        practical, formal, identities = fixture()
        result = self.check(practical, formal, identities)
        self.assertEqual(result["status"], "NOT_ACCEPTED")
        self.assertEqual(result["summary"], {"total": 34, "accepted": 0,
                                              "practical": 24, "formal": 10, "with_blockers": 34})
        self.assertTrue(all(x["blockers"] for x in result["obligations"]))

    def test_spoofed_acceptance_fields_never_pass(self):
        practical, formal, identities = fixture()
        for item in practical["items"]:
            item["acceptance"] = "PASS"
            item["coverage_gaps"] = []
        for proof in formal["goals"]:
            proof["acceptance"] = "PROVEN"
            proof["implementation_correspondence"] = "KERNEL_VERIFIED_WHOLE_IMPLEMENTATION"
            proof["excluded"] = []
        result = self.check(practical, formal, identities)
        self.assertEqual(result["status"], "NOT_ACCEPTED")
        self.assertEqual(result["summary"]["accepted"], 0)

    def test_missing_or_duplicate_rows_fail_before_counting(self):
        practical, formal, identity = fixture()
        for change in ("missing_item", "duplicate_item", "missing_goal", "foreign_compiler"):
            p, f = copy.deepcopy((practical, formal))
            if change == "missing_item":
                p["items"].pop()
            elif change == "duplicate_item":
                p["items"][-1] = p["items"][0].copy()
            elif change == "missing_goal":
                f["goals"].pop()
            else:
                f["goals"][0]["compiler"] = "rust"
            with self.subTest(change=change), self.assertRaises(ValueError):
                self.check(p, f, identity)

    def test_practical_source_snapshot_stale_or_missing_rejected(self):
        practical, formal, identity = fixture()
        for change in ("missing", "stale", "files"):
            p = copy.deepcopy(practical)
            if change == "missing":
                p.pop("source_snapshots")
            elif change == "stale":
                p["source_snapshots"]["jwc"]["sha256"] = "old"
            else:
                p["source_snapshots"]["rust"]["files"] = {}
            with self.subTest(change=change), self.assertRaises(ValueError):
                self.check(p, formal, identity)

    def test_claimed_probe_success_must_match_actual_probe_records(self):
        practical, formal, identity = fixture()
        practical["results"][0]["status"] = "FAIL"
        result = self.check(practical, formal, identity)
        item = next(x for x in result["obligations"] if x["id"] == "jwc-S01")
        self.assertTrue(any("did not pass" in b for b in item["blockers"]))

    def test_omitted_practical_goal_or_probe_is_rejected(self):
        practical, formal, identity = fixture()
        p = copy.deepcopy(practical)
        p["goals"].pop()
        with self.assertRaises(ValueError):
            self.check(p, formal, identity)
        p = copy.deepcopy(practical)
        p["results"].pop()
        with self.assertRaises(ValueError):
            self.check(p, formal, identity)

    def test_missing_or_stale_test_harness_snapshot_rejected(self):
        practical, formal, identity = fixture()
        p = copy.deepcopy(practical)
        p.pop("test_inputs_snapshot")
        with self.assertRaises(ValueError):
            self.check(p, formal, identity)
        with self.assertRaises(ValueError):
            audit.review(practical, formal, local_os="windows", identities=identity,
                         inputs_identity={"sha256": "different", "files": {}})

    def test_missing_planned_probe_cannot_be_masked(self):
        practical, formal, identities = fixture()
        p = copy.deepcopy(practical)
        p["planned_probes"].pop(0)
        with self.assertRaises(ValueError):
            self.check(p, formal, identities)
        p = copy.deepcopy(practical)
        p["items"][0]["probe_names"] = [p["items"][1]["probe_names"][0]]
        with self.assertRaises(ValueError):
            self.check(p, formal, identities)

    def test_claimed_probe_success_requires_exit_zero(self):
        practical, formal, identities = fixture()
        practical["results"][0]["actual"]["exit_code"] = 17
        result = self.check(practical, formal, identities)
        row = next(x for x in result["obligations"] if x["id"] == "jwc-S01")
        self.assertTrue(any("without a successful process exit" in b for b in row["blockers"]))

    def test_changed_lean_dependency_snapshot_rejected(self):
        practical, formal, identity = fixture()
        formal["proof_sources_snapshot"]["sha256"] = "stale"
        with self.assertRaises(ValueError):
            self.check(practical, formal, identity)

    def test_whole_implementation_inventory_is_obligatory_for_proof_closure(self):
        practical, formal, identities = fixture()
        report = audit.review(practical, formal, local_os="windows",
                              identities=identities)
        for row in report["obligations"]:
            if row["kind"] == "formal":
                self.assertIn("whole-implementation source-to-goal inventory is missing",
                              row["blockers"])

    def test_source_goal_links_report_unproved_refinements(self):
        from unittest import mock
        practical, formal, identities = fixture()
        links = [{"id": "jwc:semantics:jwc/parser.jw",
                  "compiler": "jwc", "goal": "semantics",
                  "status": "REFINEMENT_NOT_PROVEN"}]
        with mock.patch.object(audit, "validate_whole_inventory"):
            report = audit.review(practical, formal, local_os="windows",
                                  identities=identities,
                                  whole_inventory={"components": links})
        selected = next(x for x in report["obligations"]
                        if x["id"] == "jwc-G-semantics")
        self.assertEqual(selected["source_obligation_count"], 1)
        self.assertIn(links[0]["id"], selected["unproved_source_obligations"])
        self.assertTrue(any("implementation refinement obligations" in item
                            for item in selected["blockers"]))

    def test_invalid_whole_inventory_rejected(self):
        practical, formal, identities = fixture()
        with self.assertRaises((ValueError, KeyError)):
            audit.review(practical, formal, local_os="windows",
                         identities=identities,
                         whole_inventory={"components": []})

    def test_wrong_os_rejected(self):
        practical, formal, identities = fixture()
        practical["suite"]["target_os"] = "linux"
        with self.assertRaises(ValueError):
            self.check(practical, formal, identities)

    def test_source_provenance_mismatch_blocks_mathematical_claim(self):
        practical, formal, identities = fixture()
        formal["compilers"]["jwc"]["source"]["sha256"] = "different-snapshot"
        result = self.check(practical, formal, identities)
        jwc = next(x for x in result["obligations"] if x["id"] == "jwc-G-semantics")
        self.assertTrue(any("snapshot differs" in b for b in jwc["blockers"]))

    def test_independent_practical_coverage_gaps_exposed(self):
        practical, formal, identities = fixture()
        practical["items"][0]["coverage_gaps"].append("not validated under concurrency")
        result = self.check(practical, formal, identities)
        first = next(x for x in result["obligations"] if x["id"] == "jwc-S01")
        self.assertTrue(any("concurrency" in b for b in first["blockers"]))


if __name__ == "__main__":
    unittest.main()
