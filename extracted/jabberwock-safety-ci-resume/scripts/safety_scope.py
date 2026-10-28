#!/usr/bin/env python3
"""Derive compiler-specific safety obligations from reviewed specification rules.

Stage 1 of 3: classify the safety claim, not concrete test executions. This
does not infer unreachability from a declaration, execute tests, or award proof
acceptance. Stages 2/3 must verify the selected protection and its coverage.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import sys

from acceptance_requirements import COMPILERS, GOAL_CRITERIA
from formal_report import source_identity

ROOT = Path(__file__).resolve().parents[1]
POLICY = "formal/safety-scope-policy.json"
NAME = re.compile(r"[a-z][a-z0-9_-]*\Z")
SHA = re.compile(r"[0-9a-f]{64}\Z")
# A route is a *required proposition*, never an assertion that the operation
# cannot execute. In particular, runtime rejection must precede the bad effect.
ROUTES = {
    "static_rejection": "Every input violating the condition is rejected before its execution.",
    "safe_stop": "Every offending reachable operation stops before the forbidden effect, including its check and error path.",
    "invariant": "The condition holds initially and every allowed reachable transition preserves it.",
    "elision_witness": "At every omitted check the required safety condition holds on every reaching execution.",
    "unreachable": "No admitted execution reaches the forbidden operation under the declared entry conditions.",
    "preservation": "The concrete implementation preserves the specified values, effects and permitted failures.",
}


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def canonical(value) -> str:
    return digest(json.dumps(value, ensure_ascii=False, sort_keys=True,
                             separators=(",", ":")).encode("utf-8"))


def read_regular(relative: str, root: Path = ROOT) -> bytes:
    if (not isinstance(relative, str) or not relative or "\\" in relative
            or relative.startswith("/") or ":" in relative
            or any(p in ("", ".", "..") for p in relative.split("/"))):
        raise ValueError("invalid repository path")
    path = root.joinpath(*relative.split("/"))
    if (not path.is_file() or path.is_symlink()
            or any(p.is_symlink() for p in path.parents if p != root.parent)
            or not path.resolve().is_relative_to(root.resolve())):
        raise ValueError("missing, symlinked or escaped source: " + relative)
    return path.read_bytes()


def fields(value, expected: set[str], label: str) -> None:
    if not isinstance(value, dict) or set(value) != expected:
        raise ValueError("invalid fields for " + label)


def names(values, allowed: set[str] | None, label: str) -> None:
    if (not isinstance(values, list) or not values
            or any(not isinstance(x, str) or not NAME.fullmatch(x) for x in values)
            or len(values) != len(set(values))
            or (allowed is not None and not set(values) <= allowed)):
        raise ValueError("invalid or duplicate " + label)


def load_policy() -> tuple[dict, dict]:
    raw = read_regular(POLICY)
    policy = json.loads(raw)
    fields(policy, {"schema_version", "specifications", "conditions", "non_violations"}, "policy")
    if type(policy["schema_version"]) is not int or policy["schema_version"] != 1:
        raise ValueError("unsupported scope policy version")
    specs = policy["specifications"]
    # Repository-owned specification; no report may replace it with a narrower
    # specification or introduce a new trusted external component.
    if not isinstance(specs, dict) or set(specs) != {
            "docs/spec.md", "docs/language-safety-contract.md"}:
        raise ValueError("scope policy must bind both language and safety specification")
    texts = {}
    for name, expected in specs.items():
        content = read_regular(name).decode("utf-8").replace("\r\n", "\n")
        if (not isinstance(expected, str) or not SHA.fullmatch(expected)
                or digest(content.encode("utf-8")) != expected):
            raise ValueError("specification changed; scope policy needs review: " + name)
        texts[name] = content
    conditions, non_violations = policy["conditions"], policy["non_violations"]
    if not isinstance(conditions, list) or not conditions or not isinstance(non_violations, list):
        raise ValueError("scope policy requires conditions and non-violation rules")
    ids = set()
    covered_goals = set()

    def citation(row):
        references = row["spec_refs"]
        if not isinstance(references, list) or not references:
            raise ValueError("missing normative citation: " + row["id"])
        for reference in references:
            fields(reference, {"path", "quote"}, "specification citation")
            path, quote = reference["path"], reference["quote"]
            if (not isinstance(path, str) or path not in texts or not isinstance(quote, str)
                    or not quote.strip() or texts[path].count(quote) != 1):
                raise ValueError("missing or ambiguous normative quote: " + row["id"])

    def identity(row):
        if (not isinstance(row["id"], str) or not NAME.fullmatch(row["id"])
                or row["id"] in ids):
            raise ValueError("invalid or duplicate rule id")
        ids.add(row["id"])
        if not isinstance(row["statement"], str) or not row["statement"].strip():
            raise ValueError("empty rule statement")
        citation(row)

    for row in conditions:
        fields(row, {"id", "statement", "goals", "routes", "spec_refs"}, "condition")
        identity(row)
        names(row["goals"], set(GOAL_CRITERIA), "condition goals")
        names(row["routes"], set(ROUTES), "protection routes")
        covered_goals.update(row["goals"])
    if covered_goals != set(GOAL_CRITERIA):
        raise ValueError("policy omits a required safety goal")
    condition_ids = {row["id"] for row in conditions}
    for row in non_violations:
        fields(row, {"id", "statement", "requires", "spec_refs"}, "non-violation")
        identity(row)
        names(row["requires"], condition_ids, "retained protection conditions")
    snapshot = {"policy_sha256": digest(raw), "specifications": specs,
                "judgement_code": {
                    path: digest(read_regular(path)) for path in
                    ("scripts/safety_scope.py", "scripts/acceptance_requirements.py",
                     "scripts/formal_report.py")}}
    return policy, snapshot


def build_scope(compilers: tuple[str, ...] = COMPILERS) -> dict:
    if (not compilers or len(set(compilers)) != len(compilers)
            or not set(compilers) <= set(COMPILERS)):
        raise ValueError("invalid compiler selection")
    policy, inputs = load_policy()
    source_snapshots = {c: source_identity(c) for c in compilers}
    # source_identity supplies the existing complete input index; validate the
    # actual files as well. No file receives five synthetic theorem obligations.
    for snapshot in source_snapshots.values():
        for path, expected in snapshot["files"].items():
            if digest(read_regular(path)) != expected:
                raise ValueError("source changed during scope discovery: " + path)
    context = {"policy": inputs, "source_snapshots": source_snapshots}
    context_id = canonical(context)
    requirements, roots = [], []
    for compiler in compilers:
        for row in policy["conditions"]:
            requirement = {
                "id": compiler + ":" + row["id"], "compiler": compiler,
                "condition": row["id"], "statement": row["statement"],
                "goals": row["goals"], "spec_refs": row["spec_refs"],
                "permitted_proof_routes": {route: ROUTES[route] for route in row["routes"]},
                "quantification": "all admitted executions reaching the operation, not sampled inputs",
                "status": "REQUIRES_VERIFIED_PROTECTION", "context_id": context_id,
            }
            # A witness in stage 2 must match the exact proposition/attribution
            # record, not just a condition name or a successful test report.
            requirement["requirement_sha256"] = canonical(requirement)
            requirements.append(requirement)
        for goal, statement in GOAL_CRITERIA.items():
            roots.append({
                "id": compiler + ":G:" + goal, "compiler": compiler, "goal": goal,
                "statement": statement, "status": "REQUIRES_COVERAGE_AND_COMPOSITION",
                "required_conditions": [compiler + ":" + row["id"]
                                        for row in policy["conditions"] if goal in row["goals"]],
                "context_id": context_id,
                "coverage_obligation": "Prove that these protections cover all relevant admitted executions; the list alone is not a coverage proof.",
            })
    # Verify that a policy or source edited during construction cannot leave
    # a ready-looking mixed snapshot for the next two mechanisms.
    if (load_policy() != (policy, inputs)
            or {c: source_identity(c) for c in compilers} != source_snapshots):
        raise ValueError("scope inputs changed during classification")
    return {
        "schema_version": 1, "kind": "specification-safety-scope",
        "status": "SCOPE_READY", "compilers": list(compilers),
        "context_id": context_id, **context,
        "requirements": requirements, "goals": roots,
        "specification_non_violations": [{**row,
            "classification": "NOT_A_VIOLATION_BY_ITSELF",
            "implementation_safety": "REQUIRES_RETAINED_PROTECTIONS"}
            for row in policy["non_violations"]],
        "source_index_role": "provenance; not one theorem per file per goal",
        "reachability_exclusions": [],
        "whole_language_safety": "NOT_PROVEN",
        "limitations": [
            "Normative classification is not proof that a real path is unreachable or protected.",
            "A source/specification digest binds identity, not semantic correctness.",
            "Stage 2 must verify protection and stage 3 must verify coverage/composition.",
            "This policy does not add external trust assumptions or remove supported language features.",
        ],
    }


def validate_scope(report: dict, compilers: tuple[str, ...] = COMPILERS) -> None:
    if not isinstance(report, dict) or not isinstance(report.get("compilers"), list):
        raise ValueError("invalid safety scope report")
    if report != build_scope(compilers):
        raise ValueError("scope differs from current specification, sources or mandatory rules")


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", choices=("all", *COMPILERS), default="all")
    parser.add_argument("--output", type=Path, default=ROOT / "target/safety-scope.json")
    args = parser.parse_args(argv)
    try:
        compilers = COMPILERS if args.compiler == "all" else (args.compiler,)
        result = build_scope(compilers)
    except (OSError, ValueError, KeyError, TypeError, RecursionError) as error:
        result = {"schema_version": 1, "kind": "specification-safety-scope",
                  "status": "INVALID_SCOPE", "reason": str(error),
                  "whole_language_safety": "NOT_PROVEN"}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    temporary = args.output.with_suffix(args.output.suffix + ".tmp")
    temporary.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    temporary.replace(args.output)
    print(result["status"], "->", args.output, flush=True)
    return 0 if result["status"] == "SCOPE_READY" else 2


if __name__ == "__main__":
    sys.exit(main())
