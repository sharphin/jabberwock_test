#!/usr/bin/env python3
"""Aggregate independent proof boundary reports without inventing proof status."""
from __future__ import annotations
import argparse
import json
from pathlib import Path

from semantic_refinement_boundaries import report as canonical_boundaries
from refinement_transition import validate_transition_report
from whole_compiler_proof import validate as validate_inventory

def object_report(path: Path, kind: str) -> dict:
    result = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(result, dict) or result.get("schema_version") != 1 or result.get("kind") != kind:
        raise ValueError("malformed " + kind + " report")
    return result

def aggregate(boundaries: dict, bindings: dict, refinement: dict,
              inventory: dict | None = None, evidence: dict | None = None) -> dict:
    # The boundary rows must be regenerated from the current production files.
    # A self-supplied obligation list may not omit a stage or alter its hashes.
    if boundaries != canonical_boundaries():
        raise ValueError("semantic boundaries differ from current production sources")
    if inventory is not None:
        validate_inventory(inventory)
        validate_transition_report(inventory, refinement, evidence)
    if boundaries.get("status") != "NOT_PROVEN":
        raise ValueError("unexpected semantic boundary inventory state")
    obligations = boundaries.get("obligations")
    if not isinstance(obligations, list) or not obligations:
        raise ValueError("missing semantic obligations")
    if bindings.get("status") not in ("NOT_PROVEN",):
        raise ValueError("invalid stage binding report")
    if refinement.get("status") not in ("NOT_PROVEN", "PROVEN"):
        raise ValueError("invalid implementation refinement report")
    binding_rows = bindings.get("bindings")
    finding_rows = refinement.get("findings")
    if not isinstance(binding_rows, list) or not isinstance(finding_rows, list):
        raise ValueError("missing verification findings")
    bound = {}
    for row in binding_rows:
        if not isinstance(row, dict) or not isinstance(row.get("stage_id"), str):
            raise ValueError("invalid binding row")
        key = row["stage_id"]
        if key in bound or row.get("status") != "KERNEL_CHECKED_REFINEMENT_NOT_PROVEN":
            raise ValueError("duplicate or unsupported binding verdict")
        bound[key] = row
    if bindings.get("covered") != len(bound) or bindings.get("kernel_checked") != len(bound):
        raise ValueError("binding counters inconsistent")
    seen = set()
    details = []
    for item in obligations:
        if not isinstance(item, dict) or not isinstance(item.get("id"), str):
            raise ValueError("invalid semantic obligation")
        key = item["id"]
        if key in seen:
            raise ValueError("duplicate semantic obligation")
        seen.add(key)
        proof = bound.get(key)
        details.append({
            "id": key, "compiler": item["compiler"], "stage": item["stage"],
            "status": "KERNEL_CHECKED_REFINEMENT_NOT_PROVEN" if proof else "MISSING_PROOF_BINDING",
            "theorem": proof.get("theorem") if proof else None,
            "semantic_refinement": "NOT_PROVEN",
            "source_files": sorted(item["sources"]),
        })
    if set(bound) - seen:
        raise ValueError("binding refers to foreign semantic stage")
    if boundaries.get("required") != len(details):
        raise ValueError("semantic boundary count mismatch")
    # Refinement is tracked at file/goal granularity, not at stage granularity:
    # never project its claims into a stage-level proof without a trusted bridge.
    if any(not isinstance(x, dict) for x in finding_rows):
        raise ValueError("invalid implementation refinement findings")
    if inventory is not None and len(finding_rows) != inventory["total_obligations"]:
        raise ValueError("refinement findings do not cover the full source inventory")
    missing = sum(row["status"] == "MISSING_PROOF_BINDING" for row in details)
    checked = len(details) - missing
    return {
        "schema_version": 1, "kind": "formal-proof-coverage",
        "status": "NOT_PROVEN", "required": len(details),
        "kernel_checked_stage_bindings": checked,
        "missing_stage_bindings": missing, "semantically_proven": 0,
        "implementation_refinement_status": refinement["status"],
        "stages": details,
        "limitations": ["Kernel-checked theorem names and source hashes do not establish compiler semantics.",
                        "Stage proof status requires a trusted semantic implementation-to-model bridge."]
    }

def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--boundaries", type=Path, required=True)
    parser.add_argument("--bindings", type=Path, required=True)
    parser.add_argument("--refinement", type=Path, required=True)
    parser.add_argument("--inventory", type=Path,
                        help="current source inventory for independently replayed refinement audit")
    parser.add_argument("--evidence", type=Path, help="original refinement evidence for replay")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        result = aggregate(
            object_report(args.boundaries, "first-five-semantic-refinement-boundaries"),
            object_report(args.bindings, "stage-refinement-bindings"),
            object_report(args.refinement, "refinement-transition-audit"),
            object_report(args.inventory, "whole-implementation-proof-inventory") if args.inventory else None,
            json.loads(args.evidence.read_text(encoding="utf-8")) if args.evidence else None)
    except (OSError, ValueError, TypeError, KeyError) as exc:
        result = {"schema_version": 1, "kind": "formal-proof-coverage",
                  "status": "INVALID_EVIDENCE", "reason": str(exc)}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(result["status"], "->", args.output, flush=True)
    return 2 if result["status"] == "INVALID_EVIDENCE" else 0

if __name__ == "__main__":
    raise SystemExit(main())
