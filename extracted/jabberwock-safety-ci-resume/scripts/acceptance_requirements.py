"""Precisely scoped whole-implementation acceptance obligations.

Passing a finite probe or checking a Lean model is necessary evidence, not the
whole-compiler acceptance condition. Keep one independent obligation per
compiler and per safety item/goal; cases may be added without changing IDs.
"""
from __future__ import annotations

import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
COMPILERS = ("jwc", "rust")
GOALS = ("semantics", "memory", "types", "ownership", "checks")

# Criteria are assertions to establish, NOT claims they are already true.
ITEM_CRITERIA = {
  1: ("Elision soundness", "For every supported control flow and IR instruction, every elided generation/bounds check is redundant under the proved compiler invariants."),
  2: ("Well-typed hostile inputs", "All accepted well-typed programs retain memory and ownership safety, including adversarial sequences; rejected programs cannot reach unsafe execution."),
  3: ("Concurrency", "Every permitted shared-memory access is race-free under specified scheduling and memory-order assumptions; joins, channels and reclamation preserve those invariants."),
  4: ("LLVM defined behavior", "Every generated LLVM path respects poison, undef, alignment, pointer provenance, alias, lifetime and attribute rules for every supported configuration."),
  5: ("Optimization preservation", "Every supported optimization/link profile preserves the defined source behavior and all required checks, including LTO and elision."),
  6: ("Initialization", "No executable path can observe uninitialized memory or initializedness-sensitive undefined behavior."),
  7: ("Self-hosting", "Self-hosted stage2 and stage3 reproduce the required semantics and independently meet all twelve safety criteria and five proof goals."),
  8: ("Resource exhaustion", "Compiler and runtime allocations, stacks, threads and OS resource failures terminate safely without use-after-free, leaks or incorrect success."),
  9: ("Layout arithmetic", "All supported nested types and layouts obey proven finite size/alignment arithmetic without overflow, out-of-bounds or unsupported-sized acceptance."),
 10: ("Language semantics", "Every documented feature has specified evaluation order, error behavior, side effects, valid programs and rejection rules matched by compiler behavior."),
 11: ("Ownership and reclamation", "Moves, borrows, drops, errors, early exits, concurrency and FFI preserve ownership and lifetime invariants on all allowed paths."),
 12: ("Supported environments", "Every declared OS/architecture/CPU/toolchain/profile has reproducible correct binaries, and unsupported configurations fail explicitly."),
}

GOAL_CRITERIA = {
 "semantics": "For every source program admitted by the language specification, each implemented compiler pass and generated target execution preserves observations (or the specified safe failure).",
 "memory": "Every admissible generated execution is free of out-of-bounds, use-after-free, uninitialized reads, data races and other specified memory undefined behavior, under explicit runtime/FFI assumptions.",
 "types": "Every typed operational reduction preserves the typing judgment or performs the specified checked failure; accepted programs have progress.",
 "ownership": "All ownership, reference, retirement and concurrency transitions preserve unique ownership, permission and liveness invariants.",
 "checks": "Every inserted check has its specified condition; every omitted check is proved redundant, across all required optimization profiles.",
}


def obligation_catalog() -> list[dict]:
    result = []
    for compiler in COMPILERS:
        for number, (name, criterion) in ITEM_CRITERIA.items():
            result.append({"id": f"{compiler}-S{number:02}", "compiler": compiler,
                           "kind": "practical", "item": number, "name": name,
                           "required_statement": criterion,
                           "closure": ["complete declared support matrix", "executed representative and negative tests",
                                       "closed coverage gaps with auditable evidence and explicit scope"]})
        for goal, statement in GOAL_CRITERIA.items():
            result.append({"id": f"{compiler}-G-{goal}", "compiler": compiler,
                           "kind": "formal", "goal": goal, "required_statement": statement,
                           "closure": ["kernel-checked theorem (no unapproved axioms)",
                                       "validated correspondence of source code, compilation passes and runtime to theorem",
                                       "all excluded cases either discharged or explicitly excluded from the supported language"]})
    assert len(result) == 34
    return result


def load_manifest(compiler):
    path = ROOT / "formal" / "compilers" / compiler / "obligations.json"
    return json.loads(path.read_text(encoding="utf-8"))


def test_input_identity() -> dict:
    """Hash the safety test definitions and fixtures, not just the compilers."""
    roots = (ROOT / "scripts", ROOT / "tests")
    suffixes = {".py", ".cmd", ".sh", ".jw", ".rs", ".c", ".h", ".json", ".toml", ".lean"}
    files = sorted(p for root in roots if root.exists()
                   for p in root.rglob("*") if p.is_file() and p.suffix.lower() in suffixes)
    manifest = {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest() for p in files}
    encoded = json.dumps(manifest, ensure_ascii=False, sort_keys=True).encode("utf-8")
    return {"sha256": hashlib.sha256(encoded).hexdigest(), "files": manifest}
