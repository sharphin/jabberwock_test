#!/usr/bin/env python3
"""Report formal verification TOOL implementation separately from target proof results.

This report is deliberately independent of running Lean/cargo/Windows tests.
Human-reviewed capability states are inputs; no source-file count or passing
compiler theorem is used as the denominator.
"""
from __future__ import annotations
import argparse
import json
from pathlib import Path

STATES = {"IMPLEMENTED", "PARTIAL", "MISSING", "UNKNOWN"}
CAPABILITIES = (
    "proof_engine_and_axiom_audit",
    "proof_obligation_inventory",
    "implementation_correspondence_checker",
    "practical_safety_orchestration",
    "negative_tests_and_tamper_rejection",
    "isolated_runner_and_sealed_logs",
    "explainable_machine_readable_reports",
    "evidence_packaging_and_provenance",
    "windows_setup_and_local_operation",
    "tool_regressions_and_single_entrypoint",
)

def assess(entries, *, tool_tests="NOT_RUN", target_proof_acceptance="NOT_PROVEN"):
    if not isinstance(entries, dict) or set(entries) != set(CAPABILITIES):
        raise ValueError("inventory must contain exactly the declared tool capabilities")
    for key, item in entries.items():
        if not isinstance(item, dict) or item.get("state") not in STATES:
            raise ValueError("invalid capability state: " + key)
        if not isinstance(item.get("evidence"), str):
            raise ValueError("missing evidence description: " + key)
        if item["state"] == "IMPLEMENTED" and not item["evidence"].strip():
            raise ValueError("implemented capability requires code evidence: " + key)
    if tool_tests not in {"NOT_RUN", "PASS", "FAIL"}:
        raise ValueError("tool test status must be NOT_RUN, PASS or FAIL")
    if target_proof_acceptance not in {"NOT_RUN", "NOT_PROVEN", "PROVEN"}:
        raise ValueError("target proof acceptance must be NOT_RUN, NOT_PROVEN or PROVEN")
    completed = sum(x["state"] == "IMPLEMENTED" for x in entries.values())
    total = len(CAPABILITIES)
    return {
        "schema_version": 1,
        "kind": "tool-implementation-progress",
        "implemented": completed,
        "total": total,
        "percent": round(100 * completed / total, 1),
        "implementation_complete": completed == total,
        "tool_tests": tool_tests,
        "target_proof_acceptance": target_proof_acceptance,
        "capabilities": entries,
        "notice": "Implementation progress only. Never infer compiler proof success or execution-test success.",
    }

def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--inventory", type=Path, default=Path(__file__).resolve().parents[1] / "docs/tool-capabilities.json")
    p.add_argument("--output", type=Path)
    p.add_argument("--tool-tests", choices=("NOT_RUN", "PASS", "FAIL"), default="NOT_RUN",
                   help="independent tool test status; never inferred from implementation progress")
    p.add_argument("--target-proof-acceptance", choices=("NOT_RUN", "NOT_PROVEN", "PROVEN"),
                   default="NOT_PROVEN",
                   help="target compiler proof result; independent from tool implementation progress")
    args = p.parse_args()
    result = assess(json.loads(args.inventory.read_text(encoding="utf-8")),
                    tool_tests=args.tool_tests,
                    target_proof_acceptance=args.target_proof_acceptance)
    data = json.dumps(result, ensure_ascii=False, indent=2) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(data, encoding="utf-8")
    else:
        print(data, end="")
if __name__ == "__main__":
    main()
