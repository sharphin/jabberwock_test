#!/usr/bin/env python3
"""Check compiler-stage-to-Lean theorem bindings without confusing them with a refinement proof.

The checked-in registry specifies reviewed stage/theorem relationships.  For a
bound theorem this checker can invoke Lean's actual kernel audit, rather than
accepting theorem-name strings as proof.  It never certifies semantic
correspondence between a compiler and an abstract model merely from hashes.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
from pathlib import Path

from semantic_refinement_boundaries import obligation_rows
from lean_refinement_contracts import load_contracts
from lean_evidence_engine import check as check_kernel
from prepare_lean_evidence import prepare as prepare_manifest

ROOT = Path(__file__).resolve().parents[1]


def fingerprint(path: Path) -> str:
    """Authenticate a regular review file, rejecting symlink substitutions."""
    if path.is_symlink() or not path.is_file():
        raise ValueError("missing or symlinked review file: " + str(path))
    return hashlib.sha256(path.read_bytes()).hexdigest()


def inspect(root: Path, *, timeout: int = 600) -> dict:
    boundaries = obligation_rows(root)
    registry = root / "formal" / "stage-refinement-bindings.json"
    contracts = root / "formal" / "refinement-contracts.json"
    # Missing registries are an honest unproved state, but a partial
    # registry must never silently lose its reviewed theorem contracts.
    if registry.is_symlink():
        raise ValueError("symlinked stage binding registry")
    if not registry.exists():
        return {"schema_version": 1, "kind": "stage-refinement-bindings",
                "status": "NOT_PROVEN", "required": len(boundaries), "covered": 0,
                "kernel_checked": 0, "bindings": [],
                "reason": "no reviewed stage-to-theorem bindings provided"}
    registry_before = fingerprint(registry)
    if not contracts.exists():
        raise ValueError("stage bindings require a reviewed theorem contract registry")
    contracts_before = fingerprint(contracts)
    reviewed = load_contracts(root, None)
    contract_names = {x["theorem"] for x in reviewed}
    document = json.loads(registry.read_text(encoding="utf-8"))
    if fingerprint(registry) != registry_before or fingerprint(contracts) != contracts_before:
        raise ValueError("review registry changed while being parsed")
    if (not isinstance(document, dict) or document.get("schema_version") != 1
            or not isinstance(document.get("bindings"), list)):
        raise ValueError("invalid stage binding registry")
    expected = {row["id"]: row for row in boundaries}
    visited = set()
    entries = []
    # Validate all bindings before invoking the Lean toolchain. A malformed
    # registry must not be partially accepted based on earlier entries.
    for entry in document["bindings"]:
        if not isinstance(entry, dict) or set(entry) != {"stage_id", "theorem"}:
            raise ValueError("stage binding requires stage_id and theorem")
        stage_id, theorem = entry["stage_id"], entry["theorem"]
        if not isinstance(stage_id, str) or stage_id in visited or stage_id not in expected:
            raise ValueError("unknown or duplicate stage")
        if not isinstance(theorem, str) or theorem not in contract_names:
            raise ValueError("theorem has no reviewed expected-type contract")
        visited.add(stage_id)
        entries.append((stage_id, theorem))

    findings = []
    if entries:
        # The previous implementation rebuilt and audited the entire Lean
        # project once per bound stage. Instead create one source-closed
        # manifest containing every reviewed theorem and every bound source,
        # then independently check the whole set in a single kernel invocation.
        anchor_stage, anchor_theorem = entries[0]
        manifest = prepare_manifest(root, anchor_stage, anchor_theorem)
        source_files = {
            entry["path"]: entry["sha256"]
            for entry in manifest["source_files"]
        }
        for stage_id, _theorem in entries:
            for source, digest in expected[stage_id]["sources"].items():
                previous = source_files.setdefault(source, digest)
                if previous != digest:
                    raise ValueError("conflicting stage source digest: " + source)
        manifest["source_files"] = [
            {"path": source, "sha256": digest}
            for source, digest in sorted(source_files.items())
        ]
        manifest["theorems"] = sorted(set(theorem for _, theorem in entries))
        verified = check_kernel(root, manifest, timeout=timeout)
        if verified.get("status") != "KERNEL_CHECKED":
            raise ValueError("kernel did not verify bound theorems")
        checked_types = set(verified.get("contract_types_checked", []))
        for stage_id, theorem in entries:
            if theorem not in checked_types:
                raise ValueError("expected proposition not checked by Lean: " + theorem)
            findings.append({
                "stage_id": stage_id, "theorem": theorem,
                "source_hashes_checked": True,
                "kernel_type_checked": True,
                "status": "KERNEL_CHECKED_REFINEMENT_NOT_PROVEN"
            })
    # Check review files and the source closure again after the kernel audit.
    # Otherwise source or theorem registry changes during the Lean build
    # could leave a stale but successful-looking binding report.
    if fingerprint(registry) != registry_before or fingerprint(contracts) != contracts_before:
        raise ValueError("review registry changed during kernel audit")
    if obligation_rows(root) != boundaries:
        raise ValueError("compiler sources changed during stage binding audit")
    return {"schema_version": 1, "kind": "stage-refinement-bindings",
            "status": "NOT_PROVEN", "required": len(boundaries),
            "covered": len(findings), "kernel_checked": len(findings),
            "bindings": findings,
            "limitations": ["A source fingerprint is not a compiler refinement proof.",
                            "The theorem's expected type must itself describe the real implementation.",
                            "No whole-compiler correctness is certified by this stage."]}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--stage-timeout", type=int, default=600)
    args = parser.parse_args()
    if args.stage_timeout < 1:
        parser.error("timeout must be positive")
    try:
        result = inspect(ROOT, timeout=args.stage_timeout)
    except (ValueError, OSError, KeyError, TypeError, subprocess.TimeoutExpired) as exc:
        result = {"schema_version": 1, "kind": "stage-refinement-bindings",
                  "status": "INVALID_EVIDENCE", "reason": str(exc)}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(result["status"], "stage bindings ->", args.output, flush=True)
    return 2 if result["status"] == "INVALID_EVIDENCE" else 0


if __name__ == "__main__":
    raise SystemExit(main())
