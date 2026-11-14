#!/usr/bin/env python3
"""Fail-closed structural audit of the two production CFG worklists.

This is source-template evidence, never a proof of transfer soundness or
termination. Does not modify proof coverage or a PROVEN decision.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPECS = {
    "rust": (
        Path("src/ir/analyze.rs"),
        '    while let Some(b) = work.pop() {',
        (
            '            let next = match &entry[s] {',
            '                Some(old) => meet(old, &facts),',
            '                None => facts.clone(),',
            '            };',
            '            if entry[s].as_ref() != Some(&next) {',
            '                entry[s] = Some(next);',
            '                work.push(s);',
            '            }',
        ),
    ),
    "jwc": (
        Path("jwc/analyze.jw"),
        '    while work.len() > 0 {',
        (
            '            let next = if has[s] { meet(entry[s], facts) } else { copy_facts(facts) };',
            '            if !(has[s] && same_facts(entry[s], next)) {',
            '                has[s] = true;',
            '                entry[s] = next;',
            '                work.push(s);',
            '            }',
        ),
    ),
}

def inspect(name: str) -> dict:
    path, loop, suffix = SPECS[name]
    data = (ROOT / path).read_bytes()
    source = data.decode("utf-8")
    contiguous = "\n".join(suffix)
    if source.count(loop) != 1 or source.count(contiguous) != 1:
        raise ValueError(f"{name}: worklist loop or successor-update pattern changed")
    if source.index(loop) > source.index(contiguous):
        raise ValueError(f"{name}: successor-update sequence escaped the worklist")
    # The join's actual implementation is part of the proof boundary.
    # Never accept a worklist that calls a drifted or widening 'meet'.
    if name == "rust":
        meet_source = (
            'fn meet(a: &Facts, b: &Facts) -> Facts {\n'
            '    a.iter().filter(|(k, v)| b.get(k) == Some(v)).map(|(k, v)| (*k, *v)).collect()\n'
            '}'
        )
    else:
        meet_source = (
            'fn Facts meet(ref Facts a, ref Facts b) {\n'
            '    let mut out = no_facts();\n'
            '    let mut j = 0;\n'
            '    for (i, k) in a.ks {\n'
            '        while j < b.ks.len() && b.ks[j] < k { j += 1; }\n'
            '        if j < b.ks.len() && b.ks[j] == k && b.vs[j] == a.vs[i] {\n'
            '            out.ks.push(k);\n'
            '            out.vs.push(a.vs[i]);\n'
            '        }\n'
            '    }\n'
            '    out\n'
            '}'
        )
    if source.count(meet_source) != 1:
        raise ValueError(f"{name}: production meet implementation differs from intersection model")
    return {
        "compiler": name,
        "source": path.as_posix(),
        "sha256": hashlib.sha256(data).hexdigest(),
        "initial_edge": "copies incoming facts",
        "known_edge": "intersects facts",
        "meet_implementation": "exact source body matched",
        "reschedule": "only when successor entry differs",
    }

def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path,
                        default=ROOT / "target" / "formal-worklist-structure.json")
    args = parser.parse_args()
    checks = []
    failures = []
    for name in SPECS:
        try:
            checks.append(inspect(name))
        except (OSError, UnicodeError, ValueError) as exc:
            failures.append(str(exc))
    result = {
        "schema_version": 1,
        "kind": "worklist-structure-audit",
        "status": "STRUCTURE_MATCHED" if not failures else "FAIL",
        "checks": checks,
        "failures": failures,
        "compiler_refinement": "NOT_PROVEN",
        "limitations": [
            "No proof that modeled facts match actual transfer functions.",
            "No dynamic worklist termination proof.",
            "Source-template correspondence, not verified extraction.",
        ],
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(result["status"], args.output)
    return 0 if not failures else 1

if __name__ == "__main__":
    raise SystemExit(main())
