#!/usr/bin/env python3
"""Source-to-proof gate correspondence for actual Rust/jwc LLVM emitters.

The narrow theorem is about the *generated generation predicate*, NOT whole
compiler safety. Check exact ordered LLVM templates from each implementation.
Any changed or missing instruction fails; no trust in a self-reported PASS.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[1]

# Exact literal LLVM instruction fragments that both implementations emit.
# The emitted program tests (current != saved) OR (current == -1), and takes
# its rejecting branch when true.
STEPS = (
    "load atomic i64, ptr ",
    "icmp ne i64 ",
    "icmp eq i64 ",
    "or i1 ",
    "call void @jw_fail_dangling(",
    "unreachable",
)


def emitter_segment(compiler: str) -> tuple[str, str]:
    if compiler == "jwc":
        path = ROOT / "jwc" / "llvm.jw"
        start = 'pub fn str check_ref('
        end = '\nfn str lower_h('
    elif compiler == "rust":
        path = ROOT / "src" / "ir" / "llvm.rs"
        start = 'fn check_ref(b: &mut B,'
        end = '\n/// 中の own を取り出そうとしたときの案内'
    else:
        raise ValueError("unknown compiler")
    text = path.read_text(encoding="utf-8")
    if text.count(start) != 1 or text.count(end) < 1:
        raise ValueError("missing or ambiguous source emitter: " + compiler)
    portion = text.split(start, 1)[1].split(end, 1)[0]
    if not portion:
        raise ValueError("empty emitter")
    return str(path.relative_to(ROOT)), portion


def validate(compiler: str) -> dict:
    path, body = emitter_segment(compiler)
    positions = [body.find(fragment) for fragment in STEPS]
    if any(i < 0 for i in positions) or positions != sorted(positions):
        raise ValueError("emitter instruction order does not match proof model: " + compiler)
    for fragment in STEPS:
        if body.count(fragment) != 1:
            raise ValueError("missing or duplicate LLVM gate instruction: " + fragment)
    if body.count("b.if_true(") != 1:
        raise ValueError("ambiguous rejecting branch")
    if 'monotonic, align 8' not in body:
        raise ValueError("generation load lost atomic alignment or ordering")
    if compiler == "jwc":
        required = (
            'icmp ne i64 " + cur + ", " + g',
            'icmp eq i64 " + cur + ", -1',
            'or i1 " + different + ", " + retired',
            'b.if_true(bad)',
        )
    else:
        required = (
            'icmp ne i64 {}, {}", g, gen',
            'icmp eq i64 {}, -1", g',
            'or i1 {}, {}", bad, retired',
            'b.if_true(&bad)',
        )
    if any(fragment not in body for fragment in required):
        raise ValueError("generation comparison/rejection wiring changed: " + compiler)
    # The Rust emitter has an explicit unchecked early return when o.gen is
    # disabled. Reject drift in this bypass: the Lean continuation theorem
    # applies only to the checked path, never unconditionally.
    if compiler == "rust":
        bypass = (
            '    let addr = b.val(format!("extractvalue {} {}, 0", rt, r));',
            '    if !o.gen {',
            '        return addr;',
            '    }',
            '    count(b, o, "gen");',
        )
        if body.count("\n".join(bypass)) != 1:
            raise ValueError("Rust generation-check bypass differs from the formal option model")
    else:
        if "if !o.gen" in body:
            raise ValueError("jwc unexpectedly bypasses the modeled check")
    # Check the complete contiguous decision path, not merely the presence of
    # isolated instruction fragments. This prevents an injected assignment or
    # branch between the comparison and the rejecting trap from passing the
    # source/model structural binding check.
    if compiler == "jwc":
        decision = (
            '    let different = b.val("icmp ne i64 " + cur + ", " + g);',
            '    let retired = b.val("icmp eq i64 " + cur + ", -1");',
            '    let bad = b.val("or i1 " + different + ", " + retired);',
            '    let ok = b.if_true(bad);',
            '    b.op("call void @jw_fail_dangling(ptr " + use_site + ", ptr " + st + ", ptr " + blk + ", i64 " + g + ")");',
            '    b.op("unreachable");',
            '    b.label(ok);',
            '    addr',
        )
    else:
        decision = (
            '    let bad = b.val(format!("icmp ne i64 {}, {}", g, gen));',
            '    let retired = b.val(format!("icmp eq i64 {}, -1", g));',
            '    let bad = b.val(format!("or i1 {}, {}", bad, retired));',
            '    let (_, ok) = b.if_true(&bad);',
            '    b.op(format!("call void @jw_fail_dangling(ptr {}, ptr {}, ptr {}, i64 {})", use_site, site, blk, gen));',
            '    b.op("unreachable");',
            '    b.label(&ok);',
            '    addr',
        )
    if body.count("\n".join(decision)) != 1:
        raise ValueError("emitted guard is not a contiguous, modeled reject path: " + compiler)
    # The guard emitted on the rejecting branch must trap rather than return.
    if body.find("call void @jw_fail_dangling(") > body.find("unreachable"):
        raise ValueError("dangling trap order invalid")
    return {"compiler": compiler, "path": path,
            "sha256": hashlib.sha256((ROOT / path).read_bytes()).hexdigest(),
            "status": "EMITTER_GATE_STRUCTURE_MATCHED",
            "scope": "One reference-generation predicate and its failing branch, not the whole compiler"}


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--output", type=Path, default=ROOT / "target/proof-emitter-correspondence.json")
    args = p.parse_args(argv)
    results = []
    errors = []
    for compiler in ("jwc", "rust"):
        try:
            results.append(validate(compiler))
        except (OSError, ValueError) as exc:
            errors.append({"compiler": compiler, "reason": str(exc)})
    report = {"schema_version": 1, "kind": "emitter-correspondence",
              "status": "STRUCTURE_MATCHED" if not errors else "FAIL",
              "checks": results, "failures": errors,
              "limitations": ["Source template inspection is not proof of generated LLVM correctness.",
                              "No proof of pointer lifetime, optimization or runtime behavior."]}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(report["status"], ":", args.output)
    return 0 if not errors else 1


if __name__ == "__main__":
    sys.exit(main())
