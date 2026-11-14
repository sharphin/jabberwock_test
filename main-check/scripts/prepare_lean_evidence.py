#!/usr/bin/env python3
"""Generate a source-bound Lean evidence manifest for one compiler stage.

This prepares verifier input, not a proof. The result can be passed to the
one-command runner using --lean-manifest. No theorem is fabricated.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
from semantic_refinement_boundaries import obligation_rows

ROOT = Path(__file__).resolve().parents[1]

def digest(path: Path) -> str:
    if not path.is_file() or path.is_symlink():
        raise ValueError("missing or symlinked input: " + str(path))
    return hashlib.sha256(path.read_bytes()).hexdigest()

def prepare(root: Path, stage_id: str, theorem: str) -> dict:
    stages = {row["id"]: row for row in obligation_rows(root)}
    if stage_id not in stages:
        raise ValueError("unknown compiler stage: " + stage_id)
    if not theorem or not all(c.isalnum() or c in "._" for c in theorem):
        raise ValueError("invalid Lean theorem name")
    project = root / "formal"
    proof_files = []
    for path in sorted(project.rglob("*.lean")):
        relative = path.relative_to(root).as_posix()
        proof_files.append({"path": relative, "sha256": digest(path)})
    if not proof_files:
        raise ValueError("no Lean sources")
    configs = ("formal/lean-toolchain", "formal/lakefile.toml", "formal/lake-manifest.json")
    sources = set(stages[stage_id]["sources"]) | set(configs)
    return {"schema_version": 1, "lean_version": "4.24.0",
            "project": "formal", "theorems": [theorem],
            "proof_files": proof_files,
            "source_files": [{"path": path, "sha256": digest(root / path)}
                             for path in sorted(sources)]}

def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--stage", required=True)
    parser.add_argument("--theorem", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        result = prepare(ROOT, args.stage, args.theorem)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        temporary = args.output.with_suffix(args.output.suffix + ".tmp")
        temporary.write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        os.replace(temporary, args.output)
    except (OSError, ValueError) as exc:
        parser.exit(2, "cannot create evidence manifest: " + str(exc) + "\n")
    print(args.output)
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
