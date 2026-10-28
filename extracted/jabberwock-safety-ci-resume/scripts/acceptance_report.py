#!/usr/bin/env python3
"""Review the 34 acceptance obligations against two independent local reports.

This is a *read-only* evidence review, not a source of mathematical proof.
The user executes the test suites separately. Missing, stale, unverified or
inapplicable evidence remains BLOCKED; no self-attestation can produce ACCEPTED.
"""
from __future__ import annotations

import argparse
import datetime
import hashlib
import json
from pathlib import Path
import sys

from acceptance_requirements import COMPILERS, GOALS, obligation_catalog, test_input_identity
from formal_report import source_identity, proof_sources_identity
from whole_compiler_proof import validate as validate_whole_inventory
from refinement_transition import validate_transition_report

ROOT = Path(__file__).resolve().parents[1]
FAILURES = {"FAIL", "CRASH", "TIMEOUT", "INFRA_ERROR", "INTERRUPTED"}
SUCCESS_PROBE = "PROBES_PASS_ONLY"
SUCCESS_KERNEL = "KERNEL_CHECKED"


def file_hash(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def parse_json(path):
    contents = path.read_text(encoding="utf-8")
    value = json.loads(contents)
    if not isinstance(value, dict):
        raise ValueError("report must be a JSON object: " + str(path))
    return value


def keyed(records, key):
    if not isinstance(records, list):
        raise ValueError("missing list: " + key)
    mapped = {}
    for row in records:
        if not isinstance(row, dict):
            raise ValueError("nonobject entry in " + key)
        identity = row.get(key)
        if identity in mapped or not isinstance(identity, str):
            raise ValueError("duplicate or invalid identity in " + key)
        mapped[identity] = row
    return mapped


def review(practical, formal, *, local_os, identities, inputs_identity=None, whole_inventory=None, refinement=None):
    """Return precise findings without making ungrounded PASS claims."""
    if not isinstance(practical, dict) or not isinstance(formal, dict):
        raise ValueError("input reports must be JSON objects")
    if practical.get("schema_version") != 3 or formal.get("schema_version") != 3:
        raise ValueError("unsupported report schema")
    suite = practical.get("suite")
    if (not isinstance(suite, dict) or suite.get("target_os") != local_os or
            set(suite.get("compilers", [])) != set(COMPILERS) or
            len(suite.get("compilers", [])) != len(COMPILERS)):
        raise ValueError("wrong OS or incomplete compiler suite")
    if not isinstance(formal.get("compilers"), dict) or set(formal["compilers"]) != set(COMPILERS):
        raise ValueError("formal compiler map incomplete")
    item_rows = keyed(practical.get("items"), "id")
    goal_rows = keyed(formal.get("goals"), "id")
    practical_goal_rows = keyed(practical.get("goals"), "id")
    probe_rows = keyed(practical.get("results"), "name")
    plan = keyed(practical.get("planned_probes"), "name")
    if set(plan) != set(probe_rows):
        raise ValueError("planned and executed probe inventories differ")
    expected_items = {f"{local_os}-{c}-{n:02}" for c in COMPILERS for n in range(1, 13)}
    expected_goals = {f"{c}-{g}" for c in COMPILERS for g in GOALS}
    practical_goals = {f"{local_os}-{c}-{g}" for c in COMPILERS for g in GOALS}
    if (set(item_rows) != expected_items or set(goal_rows) != expected_goals or
            set(practical_goal_rows) != practical_goals):
        raise ValueError("missing, extra or duplicated safety items or practical/formal goals")
    practical_snapshots = practical.get("source_snapshots")
    if not isinstance(practical_snapshots, dict) or set(practical_snapshots) != set(COMPILERS):
        raise ValueError("practical compiler source snapshots missing")
    for compiler in COMPILERS:
        snapshot = practical_snapshots.get(compiler)
        if not isinstance(snapshot, dict) or snapshot.get("sha256") != identities[compiler]["sha256"]:
            raise ValueError("practical report uses stale or foreign source snapshot: " + compiler)
        if snapshot.get("files") != identities[compiler]["files"]:
            raise ValueError("practical per-file source snapshot mismatch: " + compiler)
    input_snapshot = practical.get("test_inputs_snapshot")
    if not isinstance(input_snapshot, dict) or "sha256" not in input_snapshot or "files" not in input_snapshot:
        raise ValueError("test harness and case input source snapshot missing")
    if inputs_identity is not None and input_snapshot != inputs_identity:
        raise ValueError("test harness/case inputs differ from practical run")
    if practical.get("run_status") != "COMPLETE" or practical.get("active_probe"):
        suite_complete = False
    else:
        suite_complete = True
    proof_snapshot = formal.get("proof_sources_snapshot")
    if not isinstance(proof_snapshot, dict) or proof_snapshot != proof_sources_identity():
        raise ValueError("Lean sources or imported formal dependencies differ from proof audit")
    # A list of source files is not a formal refinement proof. Check that no
    # entire source/goal obligations were silently omitted, and expose the
    # open links for each mathematical goal.
    source_obligations = {}
    if whole_inventory is not None:
        validate_whole_inventory(whole_inventory)
        for link in whole_inventory["components"]:
            source_obligations.setdefault((link["compiler"], link["goal"]), []).append(link)
    else:
        source_obligations = {}
    # A source inventory cannot certify itself. Check every transition result
    # against the independent transition gate rather than accepting injected
    # REFINEMENT_PROVEN flags in an inventory or report.
    transition_findings = {}
    if refinement is not None:
        if whole_inventory is None:
            raise ValueError("refinement report requires source inventory")
        expected = {row["id"] for row in whole_inventory["components"]}
        if not isinstance(refinement, dict) or refinement.get("schema_version") != 1:
            raise ValueError("invalid refinement report envelope")
        validate_transition_report(whole_inventory, refinement)
        findings = keyed(refinement.get("findings"), "id")
        if set(findings) != expected:
            raise ValueError("refinement report missing or foreign obligations")
        # Without a registered independent proof checker, the output cannot
        # truthfully claim a proven source-to-model correspondence.
        if refinement.get("status") == "PROVEN" or any(
                row.get("status") == "REFINEMENT_PROVEN" for row in findings.values()):
            raise ValueError("unverified refinement success is not acceptable evidence")
        for identity, row in findings.items():
            if row.get("status") not in ("REFINEMENT_NOT_PROVEN", "INVALID_EVIDENCE"):
                raise ValueError("unrecognized refinement transition status: " + identity)
        transition_findings = findings
    else:
        # The absence of a transition report is a distinct coverage gap.
        transition_findings = {}
    infrastructure = bool(formal.get("infrastructure_error") or formal.get("interrupted"))
    records = []
    for obligation in obligation_catalog():
        c = obligation["compiler"]
        if obligation["kind"] == "practical":
            n = obligation["item"]
            row = item_rows[f"{local_os}-{c}-{n:02}"]
            if row.get("compiler") != c or row.get("item") != n or row.get("target_os") != local_os:
                raise ValueError("wrong practical evidence attribution")
            status = row.get("status")
            gaps = row.get("coverage_gaps")
            if not isinstance(gaps, list) or any(not isinstance(g, str) or not g for g in gaps):
                raise ValueError("missing or malformed coverage gap declaration")
            if not isinstance(row.get("probe_names"), list) or not row["probe_names"]:
                raise ValueError("no probe evidence referenced")
            named = row["probe_names"]
            planned_names = {name for name, p in plan.items()
                             if p.get("compiler") == c and n in p.get("items", [])}
            if set(named) != planned_names or len(named) != len(planned_names):
                raise ValueError("item probe references do not match planned inventory")
            if len(named) != len(set(named)):
                raise ValueError("duplicate probe name reference")
            if any(n not in probe_rows for n in named):
                raise ValueError("practical item refers to unrecorded probes: " + str(n))
            observations = [probe_rows[name] for name in named]
            if any(x.get("compiler") != c for x in observations):
                raise ValueError("practical probe has wrong compiler attribution")
            if any(n not in x.get("items", []) for x in observations):
                raise ValueError("practical probe does not actually cover declared item")
            blockers = []
            if any(x.get("status") == "PROBE_PASS" and
                   x.get("actual", {}).get("exit_code") != 0 for x in observations):
                blockers.append("probe claims success without a successful process exit")
            if any(x.get("status") == "PROBE_PASS" and
                   isinstance(x.get("case_report"), dict) and
                   (not x["case_report"].get("results") or
                    any(r.get("status", r.get("case_status")) != "pass"
                        for r in x["case_report"]["results"])) for x in observations):
                blockers.append("nested case results contradict probe success")
            if any(x.get("status") != "PROBE_PASS" for x in observations):
                blockers.append("at least one referenced practical probe did not pass")
            if not suite_complete:
                blockers.append("incomplete practical suite")
            if status != SUCCESS_PROBE:
                blockers.append("practical probe status: " + str(status))
            if row.get("acceptance") != "BLOCKED":
                blockers.append("unexpected or unaudited practical acceptance claim")
            blockers += ["unclosed coverage: " + gap for gap in gaps]
            records.append({**obligation, "probe_status": status,
                            "acceptance": "BLOCKED", "blockers": blockers,
                            "evidence_refs": row.get("probe_names", [])})
        else:
            goal = obligation["goal"]
            row = goal_rows[f"{c}-{goal}"]
            if row.get("compiler") != c or row.get("goal") != goal:
                raise ValueError("wrong mathematical evidence attribution")
            direct = practical_goal_rows[f"{local_os}-{c}-{goal}"]
            if direct.get("compiler") != c:
                raise ValueError("practical goal has wrong compiler attribution")
            direct_names = direct.get("direct_probe_names", [])
            if not isinstance(direct_names, list) or len(direct_names) != len(set(direct_names)) or not direct_names:
                raise ValueError("missing or duplicate direct goal probe names")
            planned_direct = {name for name, p in plan.items()
                              if p.get("compiler") == c and p.get("goal") == goal}
            if set(direct_names) != planned_direct:
                raise ValueError("direct practical goals do not match planned probes")
            if any(n not in probe_rows or probe_rows[n].get("compiler") != c
                   for n in direct_names):
                raise ValueError("direct goal references absent or foreign probes")
            identity = identities[c]["sha256"]
            proof_status = (row.get("proof_check") or {}).get("status", "NOT_RUN")
            proof_identity = formal["compilers"][c].get("source", {}).get("sha256")
            blockers = []
            if direct.get("status") != SUCCESS_PROBE or any(probe_rows[n].get("status") != "PROBE_PASS"
                                                             for n in direct_names):
                blockers.append("independent practical goal tests have not passed")
            if infrastructure:
                blockers.append("formal infrastructure failure or interruption")
            if proof_identity != identity or row.get("source_sha256") != identity:
                blockers.append("proof source snapshot differs from the current source")
            if proof_status != SUCCESS_KERNEL:
                blockers.append("Lean kernel check: " + str(proof_status))
            manifest_path = ROOT / "formal" / "compilers" / c / "obligations.json"
            module = row.get("module")
            if (not isinstance(module, str) or not module.startswith(
                    "JabberwockFormal." + ("Jwc" if c == "jwc" else "Rust") + ".")):
                blockers.append("missing or foreign compiler theorem module")
            else:
                theorem_file = ROOT / "formal" / (module.replace(".", "/") + ".lean")
                if (not theorem_file.is_file() or
                        (row.get("proof_check") or {}).get("module_sha256") != file_hash(theorem_file)):
                    blockers.append("Lean theorem source was not audited at this hash")
            if (not manifest_path.is_file() or
                    formal["compilers"][c].get("manifest_sha256") != file_hash(manifest_path)):
                blockers.append("proof obligation manifest has changed")
            source_links = source_obligations.get((c, goal), [])
            if not source_links:
                blockers.append("whole-implementation source-to-goal inventory is missing")
            if refinement is None:
                blockers.append("independent refinement transition report is missing")
            elif any(link["id"] not in transition_findings or
                     transition_findings[link["id"]]["status"] != "REFINEMENT_PROVEN"
                     for link in source_links):
                blockers.append("independent source-to-implementation transitions not proved")
            # Keep inventory coverage visible even if the separate transition
            # report is absent or incomplete.
            open_links = [link["id"] for link in source_links
                          if link["status"] != "REFINEMENT_PROVEN"]
            if open_links:
                blockers.append(str(len(open_links)) +
                                " implementation refinement obligations remain unproved")
            if row.get("implementation_correspondence") != "KERNEL_VERIFIED_WHOLE_IMPLEMENTATION":
                blockers.append("compiler/LLVM/runtime correspondence not kernel-verified")
            excluded = row.get("excluded", [])
            if excluded:
                blockers.append("remaining model exclusions: " + "; ".join(excluded))
            if row.get("acceptance") != "NOT_PROVEN":
                blockers.append("unexpected or unaudited whole-implementation claim")
            records.append({**obligation, "proof_status": proof_status,
                            "acceptance": "NOT_PROVEN", "blockers": blockers,
                            "theorems": row.get("theorems", []),
                            "source_sha256": identity,
                            "source_obligation_count": len(source_links),
                            "unproved_source_obligations": [x["id"] for x in source_links
                                                           if x["status"] != "REFINEMENT_PROVEN"]})
    return {"schema_version": 1, "kind": "whole-compiler-acceptance-audit",
            "target_os": local_os, "status": "NOT_ACCEPTED",
            "summary": {"total": len(records), "accepted": 0,
                        "practical": 24, "formal": 10,
                        "with_blockers": sum(bool(x["blockers"]) for x in records)},
            "obligations": records,
            "limitations": ["This audit does not execute the user-run tests or Lean.",
                            "Finite representative tests do not establish universal safety.",
                            "Kernel checks of local abstract models do not prove the compiler implementation."]}


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--practical", type=Path, required=True)
    p.add_argument("--formal", type=Path, required=True)
    p.add_argument("--inventory", type=Path, required=True,
                   help="source-derived whole-implementation proof inventory")
    p.add_argument("--os", choices=["windows", "linux"], required=True)
    p.add_argument("--transition", type=Path, required=True,
                   help="independent per-source refinement transition findings")
    p.add_argument("--output", type=Path, default=ROOT / "target/acceptance-34.json")
    args = p.parse_args(argv)
    try:
        practical = parse_json(args.practical)
        formal = parse_json(args.formal)
        whole_inventory = parse_json(args.inventory)
        refinement = parse_json(args.transition)
        identities = {c: source_identity(c) for c in COMPILERS}
        report = review(practical, formal, local_os=args.os, identities=identities,
                        inputs_identity=test_input_identity(),
                        whole_inventory=whole_inventory, refinement=refinement)
        report["inputs"] = {"practical_sha256": file_hash(args.practical),
                            "formal_sha256": file_hash(args.formal),
                            "inventory_sha256": file_hash(args.inventory),
                            "transition_sha256": file_hash(args.transition)}
    except (OSError, ValueError, KeyError, TypeError, AttributeError) as error:
        report = {"schema_version": 1, "kind": "whole-compiler-acceptance-audit",
                  "status": "INVALID_EVIDENCE", "reason": str(error)}
    report["checked_utc"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print("34-obligation audit:", report["status"], "|", args.output)
    if "summary" in report:
        print("Total:", report["summary"]["total"], "Accepted:", report["summary"]["accepted"])
    return 2 if report["status"] == "INVALID_EVIDENCE" else 1


if __name__ == "__main__":
    sys.exit(main())
