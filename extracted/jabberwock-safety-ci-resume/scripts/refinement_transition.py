#!/usr/bin/env python3
"""Independent, fail-closed proof-status transition gate.

A PASS string, source hash, theorem name, or successful test is not a
refinement proof. This gate accepts only a registered independent verifier.
There is currently NO registered whole-implementation semantics verifier;
therefore actual repository obligations remain NOT_PROVEN. A verifier must
return a kernel-backed result for the exact obligation and source snapshot.
"""
from __future__ import annotations
import argparse
import json
import subprocess
from pathlib import Path
import sys

from whole_compiler_proof import inventory, validate
from lean_evidence_engine import check as check_lean_evidence
from runtime_preprocessing_evidence import capture as capture_preprocessing
from runtime_guard_expression import (inspect_symbolic_alignment,
                                      inspect_generation_macros)


ROOT = Path(__file__).resolve().parents[1]
REQUIRED = frozenset(("source_refinement", "target_semantics", "external_assumptions",
                      "build_closure", "kernel_proof"))
VERIFIERS = {}  # Deliberately empty until an independently audited verifier exists.


def status_for(obligation: dict, proofs: dict, verifier_registry=None) -> dict:
    """Reject every self-attested claim; injected test checkers cannot alter CLI registry."""
    registry = VERIFIERS if verifier_registry is None else verifier_registry
    record = proofs.get(obligation["id"])
    if record is None:
        return {"status": "REFINEMENT_NOT_PROVEN", "reason": "no proof artifact"}
    if not isinstance(record, dict) or record.get("obligation_id") != obligation["id"]:
        return {"status": "INVALID_EVIDENCE", "reason": "wrong proof identity"}
    if record.get("source_sha256") != obligation["source_sha256"]:
        return {"status": "INVALID_EVIDENCE", "reason": "stale source fingerprint"}
    claims = record.get("required_claims")
    if (not isinstance(claims, list) or any(not isinstance(x, str) for x in claims)
            or len(claims) != len(REQUIRED) or set(claims) != REQUIRED):
        return {"status": "INVALID_EVIDENCE", "reason": "missing, malformed or duplicate semantic claims"}
    checker = record.get("verifier")
    if checker == "runtime-generation-bridge-v1" and verifier_registry is None:
        # This is a genuine, independently replayed *subset* audit. The
        # whole-runtime obligation MUST remain unproved: one guard cannot
        # discharge ownership, concurrency, allocation and external effects.
        if obligation.get("source") != "runtime/jw_rt.c" or obligation.get("goal") != "semantics":
            return {"status": "INVALID_EVIDENCE",
                    "reason": "generation bridge attached to unrelated obligation"}
        artifact = record.get("preprocessing")
        manifest = record.get("lean_manifest")
        if not isinstance(artifact, dict) or not isinstance(manifest, dict):
            return {"status": "INVALID_EVIDENCE",
                    "reason": "generation bridge requires preprocessing and Lean manifest"}
        bound = {"path": obligation["source"], "sha256": obligation["source_sha256"]}
        manifest_sources = manifest.get("source_files")
        if not isinstance(manifest_sources, list) or bound not in manifest_sources:
            return {"status": "INVALID_EVIDENCE",
                    "reason": "Lean manifest is not bound to runtime source"}
        if (artifact.get("schema_version") != 1 or
                artifact.get("kind") != "runtime-generation-clang-preprocessing" or
                artifact.get("status") != "PREPROCESSING_CAPTURED_REFINEMENT_NOT_PROVEN" or
                artifact.get("source_sha256") != obligation["source_sha256"] or
                artifact.get("source") != obligation["source"] or
                artifact.get("compiler_refinement") != "NOT_PROVEN"):
            return {"status": "INVALID_EVIDENCE",
                    "reason": "invalid or stale preprocessing artifact"}
        clang, flags = artifact.get("clang"), artifact.get("caller_supplied_flags")
        if (not isinstance(clang, str) or not clang or
                not isinstance(flags, list) or
                not all(isinstance(flag, str) for flag in flags)):
            return {"status": "INVALID_EVIDENCE", "reason": "invalid clang invocation"}
        try:
            current = capture_preprocessing(clang, flags, timeout=120)
            # Re-running preprocessing prevents unauthenticated JSON from
            # becoming proof; compare complete captured evidence.
            if current != artifact:
                raise ValueError("preprocessing output differs from independently replayed invocation")
            if current.get("active_generation_limit_branch") not in ("production", "override"):
                raise ValueError("generation preprocessor branch was not authenticated")
            root_runtime = ROOT / "runtime" / "jw_rt.c"
            lean_model = ROOT / "formal" / "JabberwockFormal" / "RuntimeGenerationBitVec.lean"
            inspect_generation_macros(root_runtime)
            inspect_symbolic_alignment(root_runtime, lean_model)
            kernel = check_lean_evidence(ROOT, manifest)
            required_theorem = "JabberwockFormal.RuntimeGenerationBitVec.guard_iff_spec"
            if (kernel.get("status") != "KERNEL_CHECKED" or
                    required_theorem not in kernel.get("theorems", [])):
                raise ValueError("Lean guard equivalence theorem not kernel checked")
        except (OSError, ValueError, subprocess.TimeoutExpired, TypeError, KeyError) as exc:
            return {"status": "INVALID_EVIDENCE",
                    "reason": "generation bridge rejected: " + str(exc)}
        return {
            "status": "REFINEMENT_NOT_PROVEN",
            "reason": "kernel-backed generation guard subset checked; whole-runtime refinement is outstanding",
            "verified_subset": "runtime.gen_has_room:uint64_guard",
            "lean_kernel": "KERNEL_CHECKED",
            "preprocessing_sha256": current["preprocessed_output_sha256"],
        }
    if checker == "lean-kernel-v1" and verifier_registry is None:
        manifest = record.get("lean_manifest")
        if not isinstance(manifest, dict):
            return {"status": "INVALID_EVIDENCE", "reason": "missing Lean manifest"}
        source_files = manifest.get("source_files")
        binding = {"path": obligation["source"], "sha256": obligation["source_sha256"]}
        if not isinstance(source_files, list) or binding not in source_files:
            return {"status": "INVALID_EVIDENCE", "reason": "Lean evidence not bound to this source"}
        try:
            kernel = check_lean_evidence(ROOT, manifest)
        except (OSError, ValueError, subprocess.TimeoutExpired) as exc:
            return {"status": "INVALID_EVIDENCE", "reason": "Lean check failed: " + str(exc)}
        if kernel.get("status") != "KERNEL_CHECKED":
            return {"status": "INVALID_EVIDENCE", "reason": "Lean kernel verification rejected"}
        return {"status": "REFINEMENT_NOT_PROVEN",
                "reason": "Lean theorem checked but implementation refinement bridge is not proven",
                "lean_kernel": "KERNEL_CHECKED", "theorems": kernel["theorems"]}
    callback = registry.get(checker) if isinstance(checker, str) else None
    if callback is None:
        return {"status": "REFINEMENT_NOT_PROVEN", "reason": "no trusted independent verifier registered"}
    try:
        checked = callback(obligation, record)
    except Exception as exc:
        return {"status": "INVALID_EVIDENCE", "reason": "independent checker failed: " + str(exc)}
    if checked is not True:
        return {"status": "REFINEMENT_NOT_PROVEN", "reason": "independent checker did not discharge proof"}
    return {"status": "REFINEMENT_PROVEN", "reason": "independently verified"}


def evaluate(base: dict, evidence: dict, verifier_registry=None) -> dict:
    validate(base)
    if not isinstance(evidence, dict) or evidence.get("schema_version") != 1:
        raise ValueError("invalid proof evidence schema")
    records = evidence.get("proofs")
    if not isinstance(records, list):
        raise ValueError("proofs must be an array")
    indexed = {}
    for row in records:
        if not isinstance(row, dict) or not isinstance(row.get("obligation_id"), str):
            raise ValueError("invalid proof record")
        key = row["obligation_id"]
        if key in indexed:
            raise ValueError("duplicate proof: " + key)
        indexed[key] = row
    expected = {o["id"] for o in base["components"]}
    if set(indexed) - expected:
        raise ValueError("proofs refer to foreign obligations")
    findings = []
    for obligation in base["components"]:
        verdict = status_for(obligation, indexed, verifier_registry)
        findings.append({"id": obligation["id"], **verdict})
    total = len(findings)
    proved = sum(x["status"] == "REFINEMENT_PROVEN" for x in findings)
    invalid = sum(x["status"] == "INVALID_EVIDENCE" for x in findings)
    result = {"schema_version": 1, "kind": "refinement-transition-audit",
              "status": "INVALID_EVIDENCE" if invalid else ("PROVEN" if total and proved == total else "NOT_PROVEN"),
              "required": total, "proved": proved, "invalid": invalid,
              "findings": findings,
              "limitations": ["Lean kernel evidence is supported, but whole-implementation refinement is not proven.",
                              "A hash or successful Lean local model alone is not implementation refinement."]}
    return result



def validate_transition_report(base: dict, report: dict, evidence: dict | None = None) -> None:
    """Authenticate the complete report by deterministically replaying its gate.

    A list of REF-INEMENT_PROVEN strings, even with matching source hashes, is
    never accepted as a verifier result. This currently replays the production
    registry, which intentionally contains no whole-implementation verifier.
    """
    expected = evaluate(
        base, {"schema_version": 1, "proofs": []} if evidence is None else evidence)
    if not isinstance(report, dict) or report != expected:
        raise ValueError("refinement report does not match independently replayed proof gate")


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence", type=Path, help="optional proof artifacts, never a proof by itself")
    parser.add_argument("--output", type=Path, default=ROOT / "target/refinement-transition.json")
    args = parser.parse_args(argv)
    try:
        proof = {"schema_version": 1, "proofs": []}
        if args.evidence:
            proof = json.loads(args.evidence.read_text(encoding="utf-8"))
        before = inventory()
        result = evaluate(before, proof)
        # Re-inventory the production inputs after verification. Reject any
        # source/build file changed while proof evidence was being evaluated.
        after = inventory()
        if before != after:
            raise ValueError("production source changed during refinement audit")
    except (OSError, ValueError, TypeError, KeyError) as exc:
        result = {"schema_version": 1, "kind": "refinement-transition-audit",
                  "status": "INVALID_EVIDENCE", "reason": str(exc)}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(result["status"], "->", args.output, flush=True)
    # A successful audit execution is not the same as a discharged proof.
    # Exit zero is reserved for a fully proven implementation.
    return 0 if result["status"] == "PROVEN" else (2 if result["status"] == "INVALID_EVIDENCE" else 1)


if __name__ == "__main__":
    sys.exit(main())
