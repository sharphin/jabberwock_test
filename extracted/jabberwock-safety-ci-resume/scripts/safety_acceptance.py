#!/usr/bin/env python3
"""Replay implementation evidence and judge coverage/composition (stage 3).

The default is independent local replay, including Lean/Clang. No uploaded
status, list of hashes, or asserted unreachable path can award acceptance.
Reviewed goal-composition contracts are optional repository inputs: absence
is an open obligation, never an implicit proof or an execution failure.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import subprocess
import sys

from acceptance_requirements import COMPILERS
from formal_report import proof_sources_identity
from lean_evidence_engine import check as check_kernel
from lean_refinement_contracts import load_contracts
from safety_scope import ROOT, build_scope, validate_scope, read_regular, canonical, digest, fields
from safety_correspondence import replay, verify, kernel_manifest

REGISTRY = "formal/safety-composition-contracts.json"


def registry_for(scope: dict) -> tuple[dict, str | None]:
    path = ROOT / REGISTRY
    if not path.exists() and not path.is_symlink():
        return {}, None
    raw = read_regular(REGISTRY)
    data = json.loads(raw)
    fields(data, {"schema_version", "bindings"}, "composition registry")
    if type(data["schema_version"]) is not int or data["schema_version"] != 1 or not isinstance(data["bindings"], list):
        raise ValueError("invalid composition registry")
    roots = {row["id"]: row for row in scope["goals"]}
    # A reviewed exact proposition must be present in the existing repository
    # contract registry. The caller cannot supply a weaker proposition.
    propositions = {row["theorem"] for row in load_contracts(ROOT, None)}
    found = {}
    for binding in data["bindings"]:
        fields(binding, {"goal_id", "context_id", "required_conditions", "theorem", "domain", "assumptions"}, "composition binding")
        key = binding["goal_id"]
        if not isinstance(key, str) or key not in roots or key in found:
            raise ValueError("unknown or duplicate composition goal")
        root = roots[key]
        if (binding["context_id"] != scope["context_id"]
                or binding["required_conditions"] != root["required_conditions"]
                or binding["domain"] != root["statement"]
                or not isinstance(binding["theorem"], str)
                or binding["theorem"] not in propositions):
            raise ValueError("composition does not cover this exact goal, context and condition set")
        # Any external premises require explicit treatment, not a name on an
        # uploaded whitelist. This initial gate only accepts assumption-free
        # composition of the already verified whole-condition contracts.
        assumptions = binding["assumptions"]
        if (not isinstance(assumptions, list) or
                any(not isinstance(a, str) or not a.strip() for a in assumptions)
                or len(set(assumptions)) != len(assumptions)):
            raise ValueError("invalid composition premises")
        found[key] = binding
    return found, digest(raw)


def judge(scope: dict, correspondence: dict | None = None, *,
          compilers: tuple[str, ...] = COMPILERS, timeout: int = 600) -> dict:
    if timeout < 1:
        raise ValueError("timeout must be positive")
    validate_scope(scope, compilers)
    code_before = digest(read_regular("scripts/safety_acceptance.py"))
    proofs_before = proof_sources_identity()
    bindings, registry_sha = registry_for(scope)
    # Only the result of our real verifier reaches the judgement calculation.
    checked = (verify(scope, compilers=compilers, timeout=timeout) if correspondence is None
               else replay(correspondence, scope, compilers=compilers, timeout=timeout))
    if checked["proof_sources"] != proofs_before:
        raise ValueError("proof sources changed before correspondence replay")
    findings = {row["id"]: row for row in checked["findings"]}
    claims = {row["id"]: row for row in checked["contracts"]}
    goals = []
    for root in scope["goals"]:
        blockers = []
        dependency_rows = []
        for identity in root["required_conditions"]:
            finding = findings[identity]
            if finding["compiler"] != root["compiler"]:
                raise ValueError("cross-compiler condition in composition")
            related = [claims[name] for name in finding["checked_contracts"]]
            # Current registered adapters report local contracts with open
            # premises. Never reinterpret those as verified whole conditions.
            condition_proven = (finding["status"] == "PROTECTION_VERIFIED"
                                and not finding["remaining"] and bool(related)
                                and any(c["whole_condition"] == "VERIFIED" for c in related))
            dependency_rows.append({
                "id": identity, "status": "VERIFIED" if condition_proven else "NOT_PROVEN",
                "checked_contracts": finding["checked_contracts"], "remaining": finding["remaining"],
            })
            if not condition_proven:
                blockers.append({"kind": "OPEN_CONDITION", "id": identity,
                                 "remaining": finding["remaining"]})
        binding = bindings.get(root["id"])
        coverage = {"status": "NOT_PROVEN", "theorem": None}
        if binding is None:
            blockers.append({"kind": "MISSING_COVERAGE_COMPOSITION_PROOF", "id": root["id"]})
        elif binding["assumptions"]:
            coverage = {"status": "OPEN_PREMISES", "theorem": binding["theorem"]}
            blockers.append({"kind": "OPEN_COMPOSITION_PREMISES", "id": root["id"],
                             "remaining": binding["assumptions"]})
        else:
            manifest = kernel_manifest(scope, proofs_before)
            manifest["theorems"] = [binding["theorem"]]
            result = check_kernel(ROOT, manifest, timeout=timeout)
            if (result.get("status") != "KERNEL_CHECKED"
                    or result.get("contract_types_checked") != [binding["theorem"]]):
                raise ValueError("composition theorem or exact proposition not checked")
            coverage = {"status": "KERNEL_CHECKED_REVIEWED_COMPOSITION",
                        "theorem": binding["theorem"], "kernel_result": result,
                        "domain": binding["domain"]}
        accepted = not blockers
        goals.append({
            "id": root["id"], "compiler": root["compiler"], "goal": root["goal"],
            "statement": root["statement"], "context_id": scope["context_id"],
            "status": "ACCEPTED" if accepted else "NOT_PROVEN",
            "dependencies": dependency_rows, "coverage_composition": coverage, "blockers": blockers,
        })
    normative = []
    for compiler in compilers:
        for rule in scope["specification_non_violations"]:
            dependencies = [compiler + ":" + name for name in rule["requires"]]
            normative.append({"id": compiler + ":" + rule["id"], "compiler": compiler,
                              "classification": rule["classification"],
                              "required_protections": dependencies,
                              "does_not_discharge": dependencies})
    validate_scope(scope, compilers)
    if (registry_for(scope) != (bindings, registry_sha)
            or digest(read_regular("scripts/safety_acceptance.py")) != code_before
            or proof_sources_identity() != proofs_before):
        raise ValueError("acceptance inputs changed during independent verification")
    accepted_count = sum(row["status"] == "ACCEPTED" for row in goals)
    return {
        "schema_version": 1, "kind": "specification-safety-acceptance",
        "status": "ACCEPTED" if goals and accepted_count == len(goals) else "NOT_PROVEN",
        "verification": "INDEPENDENT_LOCAL_REPLAY" if correspondence is not None else "LOCAL_VERIFIER_EXECUTION",
        "compilers": list(compilers), "context_id": scope["context_id"],
        "scope_sha256": canonical(scope), "correspondence_sha256": canonical(checked),
        "checker_sha256": code_before, "composition_registry_sha256": registry_sha,
        "goals": goals, "specification_classification": normative,
        "unreachability_exclusions": checked["unreachability_verified"],
        "summary": {"required_goals": len(goals), "accepted_goals": accepted_count,
                    "open_goals": len(goals) - accepted_count,
                    "checked_local_contracts": len(checked["contracts"]),
                    "by_compiler": {c: {"required": sum(r["compiler"] == c for r in goals),
                                         "accepted": sum(r["compiler"] == c and r["status"] == "ACCEPTED" for r in goals)}
                                    for c in compilers}},
        "limitations": [
            "No practical safety tests are run by this command.",
            "Local proof success does not discharge unresolved reachable-execution conditions.",
            "Normative non-violation classifications do not remove the required protections.",
            "Reviewed formal composition contracts must express the stated coverage; hashes and theorem names alone are insufficient.",
        ],
    }


def validate_result_binding(report: dict, scope: dict, correspondence: dict) -> None:
    """Check attribution of a freshly executed stage, not authenticate uploads.

    Uploaded evidence must go through judge(), which replays the verifier.
    """
    from safety_correspondence import validate_report_binding
    validate_report_binding(correspondence, scope)
    if (report.get("kind") != "specification-safety-acceptance"
            or report.get("verification") != "INDEPENDENT_LOCAL_REPLAY"
            or report.get("compilers") != scope["compilers"]
            or report.get("context_id") != scope["context_id"]
            or report.get("scope_sha256") != canonical(scope)
            or report.get("correspondence_sha256") != canonical(correspondence)
            or report.get("checker_sha256") != digest(read_regular("scripts/safety_acceptance.py"))
            or report.get("composition_registry_sha256") != registry_for(scope)[1]
            or report.get("unreachability_exclusions") != correspondence["unreachability_verified"]):
        raise ValueError("acceptance context or independently replayed evidence differs")
    goals = report.get("goals")
    if not isinstance(goals, list) or len(goals) != len(scope["goals"]):
        raise ValueError("missing acceptance goals")
    findings = {r["id"]: r for r in correspondence["findings"]}
    for row, root in zip(goals, scope["goals"]):
        if any(row.get(k) != root[k] for k in ("id", "compiler", "goal", "statement", "context_id")):
            raise ValueError("foreign or reordered acceptance goal")
        deps = row.get("dependencies", [])
        if [d["id"] for d in deps] != root["required_conditions"]:
            raise ValueError("acceptance condition coverage differs")
        for dep in deps:
            finding = findings[dep["id"]]
            if (dep["checked_contracts"] != finding["checked_contracts"]
                    or dep["remaining"] != finding["remaining"]
                    or dep["status"] not in ("VERIFIED", "NOT_PROVEN")
                    or (dep["status"] == "VERIFIED" and
                        (finding["status"] != "PROTECTION_VERIFIED" or finding["remaining"]))):
                raise ValueError("acceptance condition contradicts verified correspondence")
        blockers = row.get("blockers")
        if not isinstance(blockers, list):
            raise ValueError("missing acceptance blockers")
        expected_open = {d["id"] for d in deps if d["status"] != "VERIFIED"}
        if {b["id"] for b in blockers if b["kind"] == "OPEN_CONDITION"} != expected_open:
            raise ValueError("omitted open condition")
        accepted = (not blockers and not expected_open and
                    row["coverage_composition"]["status"] == "KERNEL_CHECKED_REVIEWED_COMPOSITION")
        if row.get("status") != ("ACCEPTED" if accepted else "NOT_PROVEN"):
            raise ValueError("acceptance claim contradicts dependencies")
    count = sum(r["status"] == "ACCEPTED" for r in goals)
    expected = {"required_goals": len(goals), "accepted_goals": count,
                "open_goals": len(goals) - count,
                "checked_local_contracts": len(correspondence["contracts"]),
                "by_compiler": {c: {"required": sum(r["compiler"] == c for r in goals),
                                     "accepted": sum(r["compiler"] == c and r["status"] == "ACCEPTED" for r in goals)}
                                for c in scope["compilers"]}}
    if (report.get("summary") != expected or
            report.get("status") != ("ACCEPTED" if goals and count == len(goals) else "NOT_PROVEN")):
        raise ValueError("acceptance totals contradict verified goals")


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", choices=("all", *COMPILERS), default="all")
    parser.add_argument("--scope", type=Path)
    parser.add_argument("--correspondence", type=Path)
    parser.add_argument("--output", type=Path, default=ROOT / "target/safety-acceptance.json")
    parser.add_argument("--timeout", type=int, default=600)
    args = parser.parse_args(argv)
    if args.timeout < 1:
        parser.error("timeout must be positive")
    if any(p and p.resolve() == args.output.resolve() for p in (args.scope, args.correspondence)):
        parser.error("output must not overwrite evidence")
    try:
        selected = COMPILERS if args.compiler == "all" else (args.compiler,)
        scope = json.loads(args.scope.read_text(encoding="utf-8")) if args.scope else build_scope(selected)
        correspondence = json.loads(args.correspondence.read_text(encoding="utf-8")) if args.correspondence else None
        result = judge(scope, correspondence, compilers=selected, timeout=args.timeout)
    except (OSError, ValueError, KeyError, TypeError, RecursionError, subprocess.TimeoutExpired) as error:
        result = {"schema_version": 1, "kind": "specification-safety-acceptance",
                  "status": "INVALID_EVIDENCE", "reason": str(error)}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    temp = args.output.with_suffix(args.output.suffix + ".tmp")
    temp.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    temp.replace(args.output)
    print(result["status"], "->", args.output, flush=True)
    return {"ACCEPTED": 0, "NOT_PROVEN": 1, "INVALID_EVIDENCE": 2}[result["status"]]


if __name__ == "__main__":
    sys.exit(main())
