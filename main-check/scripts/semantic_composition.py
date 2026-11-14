#!/usr/bin/env python3
"""Derive compositional semantic proof requirements from compiler boundaries.

An individual theorem or hash is not a source-to-target simulation. The
compiler pipeline is only proven if each adjacent transition preserves an
explicit observation relation, its source closure is sound, and the composition
theorem itself is kernel checked. This module *describes* requirements and
rejects unsupported success claims; it does not manufacture Lean proofs.
"""
from __future__ import annotations
import argparse
import json
from pathlib import Path
from semantic_refinement_boundaries import obligation_rows

PIPELINE = ("source_semantics", "parse", "typecheck", "lower", "ir_semantics")
NEEDED = ("transition_relation", "observation_preservation",
          "error_preservation", "termination_or_divergence",
          "assumptions", "source_refinement", "kernel_proof")

def compose(root: Path) -> dict:
    rows = obligation_rows(root)
    lookup = {row["id"]: row for row in rows}
    if len(lookup) != len(rows):
        raise ValueError("duplicate semantic boundary")
    edges = []
    for compiler in ("jwc", "rust"):
        stages = [lookup[compiler + ":" + stage] for stage in PIPELINE]
        for left, right in zip(stages, stages[1:]):
            edges.append({
                "id": compiler + ":" + left["stage"] + "->" + right["stage"],
                "compiler": compiler, "from": left["id"], "to": right["id"],
                "source_sha256": left["sources"],
                "target_sha256": right["sources"],
                "required_proof_components": list(NEEDED),
                "status": "NOT_PROVEN",
                "reason": "no independently checked implementation simulation proof",
            })
    return {
        "schema_version": 1, "kind": "compiler-semantic-composition",
        "status": "NOT_PROVEN", "required_transitions": len(edges),
        "proven_transitions": 0, "whole_pipeline_proven": False,
        "transitions": edges,
        "composition_requires": [
            "A Lean semantics for the actual input and output of each compiler stage",
            "Kernel-checked simulation/refinement for every adjacent stage",
            "A checked composition theorem and closed implementation dependencies",
        ],
        "limitations": ["Pipeline bookkeeping is not a semantic correctness proof."]
    }

def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        result = compose(Path(__file__).resolve().parents[1])
    except (OSError, ValueError, KeyError) as exc:
        result = {"schema_version": 1, "kind": "compiler-semantic-composition",
                  "status": "INVALID_EVIDENCE", "reason": str(exc)}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return 2 if result["status"] == "INVALID_EVIDENCE" else 0

if __name__ == "__main__":
    raise SystemExit(main())
