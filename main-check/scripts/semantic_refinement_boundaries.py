#!/usr/bin/env python3
"""First five semantic-refinement boundaries for both production compilers.

This module establishes exact *obligations*, never fabricates a proof. A stage
can be marked proven only by a separately kernel-verified theorem concerning
the actual implementation and its defined observations.
"""
from __future__ import annotations

import hashlib
import json
import argparse
from pathlib import Path

BOUNDARIES = (
    ("source_semantics", "Whole-language source evaluation, errors and observable I/O"),
    ("parse", "Concrete lexer/parser output refines source AST syntax"),
    ("typecheck", "Concrete type-checking acceptance and annotation refine typing judgments"),
    ("lower", "Concrete typed AST to IR lowering preserves observations"),
    ("ir_semantics", "Every implemented IR instruction has an executable operational rule"),
)

# This is a dependency map, NOT a claim that a filename proves any property.
PHASE_SOURCES = {
    "jwc": {
        "source_semantics": ("jwc/main.jw",),
        "parse": ("jwc/lexer.jw", "jwc/parser.jw"),
        "typecheck": ("jwc/typeck.jw",),
        "lower": ("jwc/lower.jw",),
        "ir_semantics": ("jwc/ir.jw", "jwc/main.jw"),
    },
    "rust": {
        "source_semantics": ("src/interp.rs",),
        "parse": ("src/lexer.rs", "src/parser.rs"),
        "typecheck": ("src/types.rs",),
        "lower": ("src/ir/lower.rs",),
        "ir_semantics": ("src/ir/mod.rs", "src/interp.rs"),
    },
}


def obligation_rows(root: Path | None = None) -> list[dict]:
    """Bind phase obligations to current implementation bytes, rejecting drift."""
    # production_files uses its repository root for its complete source closure.
    # A caller-provided root is used for isolated checks of the selected files.
    base = Path(__file__).resolve().parents[1] if root is None else root
    rows = []
    for compiler, stages in PHASE_SOURCES.items():
        for stage, statement in BOUNDARIES:
            files = {}
            for name in stages[stage]:
                path = base / name
                if path.is_symlink() or not path.is_file():
                    raise ValueError("missing or symlinked implementation file: " + name)
                files[name] = hashlib.sha256(path.read_bytes()).hexdigest()
            rows.append({
                "id": compiler + ":" + stage,
                "compiler": compiler,
                "stage": stage,
                "statement": statement,
                "sources": files,
                "proof_status": "NOT_PROVEN",
                "required": [
                    "kernel-checked implementation-refinement theorem",
                    "formal source semantics and observation relation",
                    "verified source-to-theorem binding",
                    "explicit external assumptions and build closure",
                ],
            })
    return rows


def report(root: Path | None = None) -> dict:
    rows = obligation_rows(root)
    return {
        "schema_version": 1,
        "kind": "first-five-semantic-refinement-boundaries",
        "status": "NOT_PROVEN",
        "required": len(rows),
        "proven": 0,
        "obligations": rows,
        "note": "Ten compiler-specific obligations; no implementation theorem supplied.",
    }


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        result = report()
    except (OSError, ValueError) as exc:
        result = {"schema_version": 1, "kind": "first-five-semantic-refinement-boundaries",
                  "status": "INVALID_INVENTORY", "reason": str(exc)}
    data = json.dumps(result, indent=2, ensure_ascii=False) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(data, encoding="utf-8")
    else:
        print(data, end="")
    return 2 if result["status"] == "INVALID_INVENTORY" else 0


if __name__ == "__main__":
    raise SystemExit(main())
