#!/usr/bin/env python3
"""Fail-closed structural binding check for the C generation runtime and Lean model.

STRUCTURE_MATCHED means the reviewed source fragments and Lean declarations are
present in this snapshot. It is deliberately NOT a semantic refinement proof.
"""
from __future__ import annotations
import argparse
import json
import re
from pathlib import Path
from runtime_guard_expression import (inspect as inspect_guard, inspect_lean,
                                      inspect_symbolic_alignment, inspect_generation_macros,
                                      inspect_native_build_path)

ROOT = Path(__file__).resolve().parents[1]
C_SOURCE = ROOT / "runtime" / "jw_rt.c"
LEAN_SOURCE = ROOT / "formal" / "JabberwockFormal" / "RuntimeGenerationEncoding.lean"
BITVEC_SOURCE = ROOT / "formal" / "JabberwockFormal" / "RuntimeGenerationBitVec.lean"
NATIVE_SOURCE = ROOT / "src" / "native.rs"

C_PATTERNS = {
    "retired_sentinel": r"#define\s+GEN_RETIRED\s+UINT64_MAX",
    "counter_mask": r"#define\s+GEN_COUNTER_MASK\s+\(\(UINT64_C\(1\)\s*<<\s*62\)\s*-\s*1\)",
    "production_limit": r"#define\s+GEN_LIMIT\s+GEN_COUNTER_MASK",
    "test_limit_override": r"#ifdef\s+JW_TEST_GENERATION_LIMIT[\s\S]*?#define\s+GEN_LIMIT\s+\(\(uint64_t\)JW_TEST_GENERATION_LIMIT\)[\s\S]*?#else\s*#define\s+GEN_LIMIT\s+GEN_COUNTER_MASK\s*#endif",
    "test_limit_static_assert": r"_Static_assert\(JW_TEST_GENERATION_LIMIT\s*>=\s*3\s*&&\s*JW_TEST_GENERATION_LIMIT\s*<=\s*GEN_COUNTER_MASK",
    "capacity_guard": r"return\s+gen\s*!=\s*GEN_RETIRED\s*&&\s*step\s*<=\s*GEN_LIMIT\s*&&\s*\(gen\s*&\s*GEN_COUNTER_MASK\)\s*<=\s*GEN_LIMIT\s*-\s*step\s*;",
    "advance_retires_on_failure": r"if\s*\(!gen_has_room\(gen,\s*step\)\)\s*\{\s*gen_retire\(p\);\s*return\s+0;\s*\}\s*gen_set\(p,\s*gen\s*\+\s*step\);\s*return\s+1;",
    "free_reserves_two_generation_steps": r"if\s*\(!gen_has_room\(gen_get\(p\),\s*2\)\)\s*\{\s*remember_sent\(p,\s*why\);\s*gen_retire\(p\);\s*return;\s*\}\s*\(void\)gen_advance\(p,\s*1\);",
    "reuse_checks_remaining_generation_step": r"static int remember_reuse\(void \*p\)\s*\{\s*if\s*\(!gen_has_room\(gen_get\(p\),\s*1\)\)\s*\{\s*gen_retire\(p\);\s*return 0;\s*\}",
    "retire_sentinel_store": r"gen_set\(p,\s*GEN_RETIRED\);",
    "retire_list_release_cas": r"__atomic_compare_exchange_n\(&retired_head,\s*&head,\s*p,\s*0,\s*__ATOMIC_RELEASE,\s*__ATOMIC_RELAXED\)",
    "view_generation_gate": r"uint64_t\s+gen\s*=\s*gen_get\(c->target\);\s*if\s*\(gen\s*==\s*GEN_RETIRED\s*\|\|\s*gen\s*!=\s*c->gen\)\s*jw_fail_dangling\(use,\s*c->site,\s*c->target,\s*c->gen\);\s*return\s+c->target;",
    "slab_init_before_pointer_subtraction": r"static void \*slab_take\(size_t n\)\s*\{\s*const size_t slab_size\s*=\s*256\s*\*\s*1024;[\s\S]*?if\s*\(!slab_cur\s*\|\|\s*!slab_end\s*\|\|\s*\(size_t\)\(slab_end\s*-\s*slab_cur\)\s*<\s*n\)",
}
LEAN_TOKENS = (
    "def wordCounter",
    "def concreteGuard",
    "theorem packed_counter_mod",
    "theorem concreteGuard_matches_abstract",
    "def concreteAdvance",
    "theorem concreteAdvance_matches_abstract",
    "theorem packed_word_step_no_uint64_wrap",
    "def capacityCheckAt",
    "theorem capacityCheckAt_iff_sum",
    "theorem packed_word_step_no_uint64_wrap_at_limit",
    "def freeThenReuse",
    "theorem room_two_implies_room_after_first",
    "theorem freeThenReuse_matches_two_step",
    "theorem freeThenReuse_rejects_old_reference",
)
BITVEC_TOKENS = (
    "abbrev Word := BitVec 64",
    "def guardSpec",
    "theorem guard_iff_spec",
    "theorem guard_rejects_retired",
    "theorem masked_counter_matches_mod",
    "theorem guardSpec_matches_nat",
    "theorem guard_matches_nat",
    "theorem unsigned_subtraction_matches_nat",
    "theorem accepted_increment_no_wrap",
    "theorem accepted_increment_changes_word",
    "def viewGate",
    "theorem viewGate_matches_spec",
    "theorem viewGate_rejects_retired",
    "theorem viewGate_rejects_after_advance",
)


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "target/runtime-generation-binding.json")
    args = parser.parse_args(argv)
    findings = []
    try:
        c_text = C_SOURCE.read_text(encoding="utf-8")
        lean_text = LEAN_SOURCE.read_text(encoding="utf-8")
        bitvec_text = BITVEC_SOURCE.read_text(encoding="utf-8")
        guard = inspect_guard(C_SOURCE)
        lean_guard = inspect_lean(BITVEC_SOURCE)
        symbolic = inspect_symbolic_alignment(C_SOURCE, BITVEC_SOURCE)
        macros = inspect_generation_macros(C_SOURCE)
        native_path = inspect_native_build_path(NATIVE_SOURCE)
        findings.append({"id": "native:clang_runtime_build_path",
                         "status": "MATCHED" if native_path["status"] ==
                         "BUILD_SOURCE_PATH_MATCHED_FLAGS_UNVERIFIED" else "MISSING"})
        findings.append({"id": "c:generation_macro_branches",
                         "status": "MATCHED" if macros["status"] ==
                         "MACRO_BRANCHES_REVIEWED_BUILD_CONFIGURATION_UNKNOWN" else "MISSING"})
        findings.append({"id": "c-bitvec:unsigned_guard_symbolic_semantics",
                         "status": "MATCHED" if symbolic["status"] == "SYMBOLIC_SUBSET_ALIGNED_NOT_PROVEN" else "MISSING"})
        findings.append({"id": "bitvec:guard_expression_body",
                         "status": "MATCHED" if lean_guard["status"] == "LEAN_EXPRESSION_ALIGNED_MODEL_ONLY" else "MISSING"})
        findings.append({"id": "c:guard_expression_semantics",
                         "status": "MATCHED" if guard["status"] == "EXPRESSION_ALIGNED_MODEL_ONLY" else "MISSING"})
        for name, pattern in C_PATTERNS.items():
            matched = re.search(pattern, c_text, flags=re.MULTILINE) is not None
            findings.append({"id": "c:" + name, "status": "MATCHED" if matched else "MISSING"})
        for token in LEAN_TOKENS:
            matched = token in lean_text
            findings.append({"id": "lean:" + token, "status": "MATCHED" if matched else "MISSING"})
        for token in BITVEC_TOKENS:
            matched = token in bitvec_text
            findings.append({"id": "bitvec:" + token, "status": "MATCHED" if matched else "MISSING"})
        missing = [item["id"] for item in findings if item["status"] != "MATCHED"]
        result = {
            "schema_version": 1,
            "kind": "runtime-generation-structural-binding",
            "status": "STRUCTURE_MATCHED" if not missing else "STRUCTURE_MISMATCH",
            "checked": len(findings),
            "matched": len(findings) - len(missing),
            "findings": findings,
            "limitations": [
                "The restricted generation guard AST and BitVec operator trees align symbolically under explicit assumptions; this is not a verified C semantics proof.",
                "Structural matching is not semantic equivalence or a kernel proof.",
                "The BitVec model kernel-checks 64-bit arithmetic, but source matching is structural, not a verified C parser or compiler proof.",
                "The Nat model remains an abstraction; its mask operation is not itself a proof of C bitwise-and semantics.",
                "This check does not prove concurrent lifetime safety, allocator correctness, or memory ordering.",
                "Both generation limit macro branches are inspected, but actual compiler flags and active preprocessing branch remain unverified."
            ],
        }
        if missing:
            result["missing"] = missing
    except (OSError, ValueError) as exc:
        result = {"schema_version": 1, "kind": "runtime-generation-structural-binding",
                  "status": "STRUCTURE_MISMATCH", "reason": str(exc), "findings": []}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(result["status"], result.get("matched", 0), "/", result.get("checked", 0),
          "C/Lean structural bindings ->", args.output, flush=True)
    return 0 if result["status"] == "STRUCTURE_MATCHED" else 1


if __name__ == "__main__":
    raise SystemExit(main())
