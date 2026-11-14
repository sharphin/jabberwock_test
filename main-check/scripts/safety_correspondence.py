#!/usr/bin/env python3
"""Verify implementation evidence for the specification safety scope (stage 2).

Uses fixed local proof adapters, never report-supplied commands. Records exact
claims and unresolved premises; a local arithmetic/template/model theorem is
not promoted to the safety of all reachable executions. Stage 3 consumes this
report by replaying it, not by trusting its status strings.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import subprocess
import sys

from safety_scope import ROOT, build_scope, validate_scope, canonical, digest, read_regular
from acceptance_requirements import COMPILERS
from formal_report import proof_sources_identity
from lean_evidence_engine import check as check_kernel
from proof_emitter_correspondence import validate as check_emitter
from runtime_source_contract import check as check_capacity, ACCEPTED, contract_id

EMITTER = "JabberwockFormal.EmittedGate"
ELISION = "JabberwockFormal.ElisionDecision.sound_elision_preserves_read"
SHARED_THEOREMS = (EMITTER + ".branch_continues_iff_valid", EMITTER + ".no_retired_gate", ELISION)


def compiler_theorems(compiler: str) -> tuple[str, str]:
    prefix = "JabberwockFormal." + {"jwc": "Jwc", "rust": "Rust"}[compiler] + ".ReferenceAccess."
    return prefix + "successful_write_is_writable", prefix + "move_preserves_unique"


def checker_identity() -> dict:
    paths = ("scripts/safety_correspondence.py", "scripts/safety_scope.py",
             "scripts/lean_evidence_engine.py", "scripts/lean_refinement_contracts.py",
             "scripts/proof_emitter_correspondence.py", "scripts/runtime_source_contract.py",
             "scripts/runtime_preprocessing_evidence.py", "scripts/sync_runtime.py",
             "scripts/formal_report.py", "scripts/source_contract_acceptance.py")
    return {path: digest(read_regular(path)) for path in paths}


def kernel_manifest(scope: dict, proofs: dict) -> dict:
    names = list(SHARED_THEOREMS)
    sources = {}
    for compiler in scope["compilers"]:
        names.extend(compiler_theorems(compiler))
        for name, sha in scope["source_snapshots"][compiler]["files"].items():
            if name in sources and sources[name] != sha:
                raise ValueError("shared source has conflicting snapshots")
            sources[name] = sha
    for name in ("formal/lean-toolchain", "formal/lakefile.toml", "formal/lake-manifest.json",
                 "formal/refinement-contracts.json"):
        sources[name] = digest(read_regular(name))
    return {"schema_version": 1, "lean_version": "4.24.0", "project": "formal",
            "theorems": sorted(names),
            "proof_files": [{"path": p, "sha256": sha} for p, sha in proofs["files"].items()
                            if p.endswith(".lean")],
            "source_files": [{"path": p, "sha256": sha} for p, sha in sorted(sources.items())]}


def verify(scope: dict, *, compilers: tuple[str, ...] = COMPILERS, timeout: int = 600) -> dict:
    if timeout < 1:
        raise ValueError("timeout must be positive")
    validate_scope(scope, compilers)
    code_before = checker_identity()
    proofs_before = proof_sources_identity()
    # Check all reusable theorems once. Each compiler-specific application is
    # still separately attributed below. Expected propositions are mandatory.
    manifest = kernel_manifest(scope, proofs_before)
    try:
        kernel = check_kernel(ROOT, manifest, timeout=timeout)
    except (OSError, ValueError, subprocess.TimeoutExpired) as error:
        raise ValueError("shared kernel/expected-proposition audit: " + str(error)) from error
    if (kernel.get("status") != "KERNEL_CHECKED"
            or set(kernel.get("theorems", [])) != set(manifest["theorems"])
            or set(kernel.get("contract_types_checked", [])) != set(manifest["theorems"])
            or kernel.get("compiler_refinement") != "NOT_PROVEN"):
        raise ValueError("missing kernel or expected-proposition verification")
    requirements = {row["id"]: row for row in scope["requirements"]}
    contracts = []
    artifacts = {"kernel": kernel}

    def record(compiler, condition, local_id, route, level, statement, artifact,
               assumptions, remaining, theorems=()):
        req = requirements[compiler + ":" + condition]
        if route not in req["permitted_proof_routes"]:
            raise ValueError("adapter uses a route forbidden by the specification policy")
        contracts.append({
            "id": compiler + ":" + local_id, "compiler": compiler,
            "requirement_id": req["id"], "requirement_sha256": req["requirement_sha256"],
            "context_id": scope["context_id"], "route": route,
            "status": level, "statement": statement,
            "artifact": artifact, "artifact_sha256": canonical(artifacts[artifact]),
            "theorems": list(theorems), "assumptions": assumptions,
            "remaining_for_condition": remaining,
            "whole_condition": "NOT_PROVEN",
        })

    for compiler in compilers:
        # Fresh C AST -> exact generated universal proposition -> kernel.
        # The actual compiler identity is an argument, never taken from a report.
        try:
            capacity = check_capacity(timeout, compiler=compiler)
        except (OSError, ValueError, KeyError, TypeError, subprocess.TimeoutExpired) as error:
            raise ValueError(compiler + " runtime-capacity adapter: " + str(error)) from error
        if (capacity.get("status") != ACCEPTED or capacity.get("compiler") != compiler
                or capacity.get("contract_id") != contract_id(compiler)
                or capacity["source_snapshot"].get("compiler_source") != scope["source_snapshots"][compiler]
                or capacity["source_snapshot"].get("proofs") != proofs_before):
            raise ValueError("capacity proof uses another compiler, source or proof context")
        artifact = compiler + ":runtime-capacity"
        artifacts[artifact] = capacity
        record(compiler, "generation-retirement", "runtime-capacity", "invariant",
               "SOURCE_CONTRACT_CHECKED_ASSUMING_FRONTEND",
               capacity["quantification"] + "; actual gen_has_room agrees with the capacity predicate",
               artifact, capacity["trust_premises"],
               ["all actual generation updates use the guard with the required state invariants",
                "retired identities are never reused and metadata survives permitted reference use",
                "allocation, concurrency, emission and build configuration meet the runtime contract"],
               (capacity["theorem"],))

        # The existing emitter recognizer is structural, not a verified parser.
        # Keep that narrower evidence level even though its predicate is proved.
        try:
            binding = check_emitter(compiler)
        except (OSError, ValueError) as error:
            raise ValueError(compiler + " emitter-gate adapter: " + str(error)) from error
        if (binding.get("compiler") != compiler or binding.get("status") != "EMITTER_GATE_STRUCTURE_MATCHED"
                or scope["source_snapshots"][compiler]["files"].get(binding.get("path")) != binding.get("sha256")):
            raise ValueError("emitter binding is missing, stale or belongs to another compiler")
        artifact = compiler + ":emitter-gate"
        artifacts[artifact] = binding
        record(compiler, "reference-access", "emitter-gate", "safe_stop",
               "KERNEL_MODEL_AND_STRUCTURE_CHECKED",
               "The recognized gate predicate permits exactly equal, non-retired generation words",
               artifact, ["recognized emitter template", "accessible metadata", "generation checks enabled"],
               ["every relevant access uses this gate or a verified elision witness",
                "branch helpers, metadata lifetime, atomics and failing-call semantics satisfy the model"],
               SHARED_THEOREMS[:2])
        record(compiler, "check-elision", "elision-model", "elision_witness",
               "KERNEL_MODEL_ONLY",
               "FactSoundAtAccess implies the modeled selected elision preserves a read", "kernel",
               ["FactSoundAtAccess at the actual access"],
               ["actual dataflow/CFG facts establish FactSoundAtAccess at every elision",
                "actual transformations and accesses implement the model"], (ELISION,))
        permission, owner = compiler_theorems(compiler)
        record(compiler, "typing-and-permission", "permission-model", "invariant",
               "KERNEL_MODEL_ONLY", "A successful modeled write requires writable permission", "kernel",
               ["the model is a write-permission abstraction"],
               ["actual typing and permission checks implement the abstraction on admitted paths",
                "remaining typing and representation rules preserve the full condition"], (permission,))
        record(compiler, "owner-transition", "owner-model", "invariant",
               "KERNEL_MODEL_ONLY", "The modeled checked move preserves unique ownership", "kernel",
               ["unique ownership before the move", "successful modeled checkedMove"],
               ["actual moves, drops and failure paths implement the modeled transitions",
                "ownership is initially unique and every other permitted transition preserves it"], (owner,))

    findings = []
    for requirement in scope["requirements"]:
        matching = [row for row in contracts if row["requirement_id"] == requirement["id"]]
        findings.append({
            "id": requirement["id"], "compiler": requirement["compiler"],
            "requirement_sha256": requirement["requirement_sha256"],
            "status": "OPEN_WITH_CHECKED_PARTS" if matching else "OPEN_NO_IMPLEMENTATION_PROOF",
            "checked_contracts": [row["id"] for row in matching],
            "remaining": list(dict.fromkeys(reason for row in matching
                                             for reason in row["remaining_for_condition"]))
                         if matching else [requirement["statement"]],
        })
    validate_scope(scope, compilers)
    if checker_identity() != code_before or proof_sources_identity() != proofs_before:
        raise ValueError("checker or proof inputs changed during verification")
    result = {
        "schema_version": 1, "kind": "safety-implementation-correspondence",
        "status": "CHECKED_WITH_OPEN_CONDITIONS", "compilers": list(compilers),
        "context_id": scope["context_id"], "scope_sha256": canonical(scope),
        "checker_identity": code_before, "proof_sources": proofs_before,
        "contracts": contracts, "artifacts": artifacts, "findings": findings,
        "unreachability_verified": [], "whole_language_safety": "NOT_PROVEN",
        "summary": {"required_conditions": len(findings), "verified_whole_conditions": 0,
                    "checked_contracts": len(contracts),
                    "conditions_with_checked_parts": sum(bool(row["checked_contracts"]) for row in findings)},
        "limitations": [
            "The registered contracts establish only their explicit local conclusions.",
            "Checked local models, structure matches and partial source contracts do not discharge whole conditions.",
            "No unreachability exclusion is inferred from missing probes, source hashes or declarations.",
            "Stage 3 must validate remaining premises, coverage and composition before whole-goal acceptance.",
        ],
    }
    return json.loads(json.dumps(result))  # Canonical JSON types for exact replay.


def replay(report: dict, scope: dict, *, compilers: tuple[str, ...] = COMPILERS,
           timeout: int = 600) -> dict:
    validate_scope(scope, compilers)
    validate_report_binding(report, scope)
    actual = verify(scope, compilers=compilers, timeout=timeout)
    if actual != report:
        raise ValueError("correspondence evidence differs from independent local replay")
    return actual


def validate_report_binding(report: dict, scope: dict) -> None:
    """Integrity/attribution validation only; replay() authenticates proof results."""
    from safety_scope import fields
    if (not isinstance(report, dict) or type(report.get("schema_version")) is not int
            or report["schema_version"] != 1
            or report.get("kind") != "safety-implementation-correspondence"
            or report.get("status") != "CHECKED_WITH_OPEN_CONDITIONS"
            or report.get("context_id") != scope["context_id"]
            or report.get("scope_sha256") != canonical(scope)
            or report.get("compilers") != scope["compilers"]
            or report.get("checker_identity") != checker_identity()
            or report.get("proof_sources") != proof_sources_identity()
            or report.get("whole_language_safety") != "NOT_PROVEN"
            or report.get("unreachability_verified") != []):
        raise ValueError("foreign, stale or overstated correspondence report")
    requirements = {row["id"]: row for row in scope["requirements"]}
    findings = report.get("findings")
    contracts = report.get("contracts")
    artifacts = report.get("artifacts")
    if not isinstance(findings, list) or not isinstance(contracts, list) or not isinstance(artifacts, dict):
        raise ValueError("missing correspondence findings, contracts or artifacts")
    expected_ids = {c + ":" + suffix for c in scope["compilers"] for suffix in
                    ("runtime-capacity", "emitter-gate", "elision-model", "permission-model", "owner-model")}
    mapping = {"runtime-capacity": ("generation-retirement", "invariant", "SOURCE_CONTRACT_CHECKED_ASSUMING_FRONTEND"),
               "emitter-gate": ("reference-access", "safe_stop", "KERNEL_MODEL_AND_STRUCTURE_CHECKED"),
               "elision-model": ("check-elision", "elision_witness", "KERNEL_MODEL_ONLY"),
               "permission-model": ("typing-and-permission", "invariant", "KERNEL_MODEL_ONLY"),
               "owner-model": ("owner-transition", "invariant", "KERNEL_MODEL_ONLY")}
    seen = set()
    for row in contracts:
        if not isinstance(row, dict) or not isinstance(row.get("id"), str):
            raise ValueError("malformed contract record")
        identity = row["id"]
        if identity not in expected_ids or identity in seen:
            raise ValueError("missing, duplicate or foreign local contract")
        seen.add(identity)
        compiler, suffix = identity.split(":")
        condition, route, level = mapping[suffix]
        req = requirements[compiler + ":" + condition]
        artifact = "kernel" if suffix.endswith("model") else identity
        if (row.get("compiler") != compiler or row.get("requirement_id") != req["id"]
                or row.get("requirement_sha256") != req["requirement_sha256"]
                or row.get("context_id") != scope["context_id"]
                or row.get("route") != route or row.get("status") != level
                or row.get("artifact") != artifact or artifact not in artifacts
                or row.get("artifact_sha256") != canonical(artifacts[artifact])
                or row.get("whole_condition") != "NOT_PROVEN"
                or not isinstance(row.get("remaining_for_condition"), list)
                or not row["remaining_for_condition"]):
            raise ValueError("local contract attribution, assumptions or proof scope changed")
    if seen != expected_ids:
        raise ValueError("incomplete local contract inventory")
    expected_artifacts = {"kernel"} | {c + ":" + suffix for c in scope["compilers"]
                                      for suffix in ("runtime-capacity", "emitter-gate")}
    if set(artifacts) != expected_artifacts:
        raise ValueError("missing or foreign proof artifacts")
    seen = set()
    for row in findings:
        fields(row, {"id", "compiler", "requirement_sha256", "status", "checked_contracts", "remaining"}, "condition finding")
        identity = row["id"]
        if not isinstance(identity, str) or identity not in requirements or identity in seen:
            raise ValueError("missing, foreign or duplicate condition finding")
        seen.add(identity)
        req = requirements[identity]
        related = [c for c in contracts if c["requirement_id"] == identity]
        expected_remaining = list(dict.fromkeys(x for c in related for x in c["remaining_for_condition"])) if related else [req["statement"]]
        if (row["compiler"] != req["compiler"] or row["requirement_sha256"] != req["requirement_sha256"]
                or row["checked_contracts"] != [c["id"] for c in related]
                or row["remaining"] != expected_remaining
                or row["status"] != ("OPEN_WITH_CHECKED_PARTS" if related else "OPEN_NO_IMPLEMENTATION_PROOF")):
            raise ValueError("condition finding does not match its checked evidence")
    if seen != set(requirements):
        raise ValueError("correspondence report omitted a required safety condition")
    expected_summary = {"required_conditions": len(requirements), "verified_whole_conditions": 0,
                        "checked_contracts": len(contracts),
                        "conditions_with_checked_parts": sum(bool(x["checked_contracts"]) for x in findings)}
    if report.get("summary") != expected_summary:
        raise ValueError("invalid condition counters")


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", choices=("all", *COMPILERS), default="all")
    parser.add_argument("--scope", type=Path, help="stage 1 report; recomputed and compared before use")
    parser.add_argument("--replay", type=Path, help="re-execute verification and compare a previous report")
    parser.add_argument("--output", type=Path, default=ROOT / "target/safety-correspondence.json")
    parser.add_argument("--timeout", type=int, default=600, help="seconds per verifier subprocess")
    args = parser.parse_args(argv)
    if args.timeout < 1:
        parser.error("timeout must be positive")
    if any(path and path.resolve() == args.output.resolve() for path in (args.scope, args.replay)):
        parser.error("output must not overwrite evidence")
    try:
        selected = COMPILERS if args.compiler == "all" else (args.compiler,)
        scope = json.loads(args.scope.read_text(encoding="utf-8")) if args.scope else build_scope(selected)
        if args.replay:
            report = json.loads(args.replay.read_text(encoding="utf-8"))
            result = replay(report, scope, compilers=selected, timeout=args.timeout)
        else:
            result = verify(scope, compilers=selected, timeout=args.timeout)
    except (OSError, ValueError, KeyError, TypeError, RecursionError, subprocess.TimeoutExpired) as error:
        result = {"schema_version": 1, "kind": "safety-implementation-correspondence",
                  "status": "INVALID_EVIDENCE", "reason": str(error),
                  "whole_language_safety": "NOT_PROVEN"}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    temporary = args.output.with_suffix(args.output.suffix + ".tmp")
    temporary.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    temporary.replace(args.output)
    print(result["status"], "->", args.output, flush=True)
    return 0 if result["status"] == "CHECKED_WITH_OPEN_CONDITIONS" else 2


if __name__ == "__main__":
    sys.exit(main())
