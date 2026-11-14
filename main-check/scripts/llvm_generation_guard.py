#!/usr/bin/env python3
"""Validate one *generated* LLVM reference-generation guard.

This is a bounded IR-level correspondence check, not a formal proof of the
compiler, pointer validity, LLVM optimizer, or concurrent memory semantics.
Strictly rejects unsupported variants instead of guessing equivalence.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import sys

SSA = r"%[-A-Za-z$._0-9]+"
TOKEN = re.compile(r"^\s*(%[-A-Za-z$._0-9]+)\s*=\s*(.+?)\s*$")
LABEL = re.compile(r"^\s*([-A-Za-z$._0-9]+):\s*(?:;.*)?$")
BRANCH = re.compile(
    r"^br i1 (%[-A-Za-z$._0-9]+), label %([-A-Za-z$._0-9]+), "
    r"label %([-A-Za-z$._0-9]+)(?:,.*)?$")
TRAP = re.compile(r"^call void @jw_fail_dangling\(.*\)(?:\s+.*)?$")
LOAD = re.compile(r"^load atomic i64, ptr (%[-A-Za-z$._0-9]+) unordered, align 8$")
NE = re.compile(r"^icmp ne i64 (%[-A-Za-z$._0-9]+), (%[-A-Za-z$._0-9]+)$")
RETIRED = re.compile(r"^icmp eq i64 (%[-A-Za-z$._0-9]+), -1$")
OR = re.compile(r"^or i1 (%[-A-Za-z$._0-9]+), (%[-A-Za-z$._0-9]+)$")


def verify_llvm_guard(text: str) -> dict:
    """Require exact dataflow and rejecting control-flow for a single gate.

    The caller must separately establish that this gate dominates every
    relevant payload access and that the loaded generation pointer is valid.
    """
    if not isinstance(text, str) or not text.strip():
        raise ValueError("empty LLVM module")
    lines = [line.split(";", 1)[0].strip() for line in text.splitlines()]
    definitions: dict[str, tuple[int, str]] = {}
    labels: dict[str, int] = {}
    for i, line in enumerate(lines):
        label = LABEL.fullmatch(line)
        if label:
            if label[1] in labels:
                raise ValueError("duplicate block label")
            labels[label[1]] = i
        definition = TOKEN.fullmatch(line)
        if definition:
            if definition[1] in definitions:
                raise ValueError("duplicate SSA definition")
            definitions[definition[1]] = (i, definition[2])

    successes = []
    for load_var, (load_idx, load_op) in definitions.items():
        match_load = LOAD.fullmatch(load_op)
        if not match_load:
            continue
        for ne_var, (ne_idx, ne_op) in definitions.items():
            different = NE.fullmatch(ne_op)
            if not different or not (load_idx < ne_idx):
                continue
            if different[1] != load_var or different[2] == load_var:
                continue
            for retired_var, (ret_idx, ret_op) in definitions.items():
                is_retired = RETIRED.fullmatch(ret_op)
                if not is_retired or is_retired[1] != load_var or not (ne_idx < ret_idx):
                    continue
                for bad_var, (or_idx, or_op) in definitions.items():
                    operands = OR.fullmatch(or_op)
                    if not operands or (operands[1], operands[2]) != (ne_var, retired_var):
                        continue
                    if not (ret_idx < or_idx):
                        continue
                    for branch_idx in range(or_idx + 1, len(lines)):
                        branch = BRANCH.fullmatch(lines[branch_idx])
                        if not branch or branch[1] != bad_var:
                            continue
                        reject, good = branch[2], branch[3]
                        if reject == good or reject not in labels or good not in labels:
                            continue
                        trap_start = labels[reject]
                        good_start = labels[good]
                        if not (branch_idx < trap_start < good_start):
                            continue
                        # The rejection block must consist of the failing call
                        # and unreachable, with no possible normal return.
                        fail_ops = [line for line in lines[trap_start+1:good_start] if line]
                        if len(fail_ops) != 2 or not TRAP.fullmatch(fail_ops[0]) or fail_ops[1] != "unreachable":
                            continue
                        # The gate's comparisons must not be separated by an
                        # unrecognized instruction that can change control flow.
                        candidate = (load_idx, ne_idx, ret_idx, or_idx, branch_idx)
                        if any(not lines[k].startswith("%") for k in range(load_idx + 1, branch_idx)):
                            continue
                        successes.append({
                            "generation_load": load_var,
                            "saved_generation": different[2],
                            "different": ne_var,
                            "retired": retired_var,
                            "reject_condition": bad_var,
                            "reject_block": reject,
                            "continue_block": good,
                            "line": load_idx + 1,
                        })
    if len(successes) != 1:
        raise ValueError(f"expected exactly one unambiguous generation guard, found {len(successes)}")
    return {"status": "LLVM_GUARD_DATAFLOW_MATCHED", **successes[0],
            "limitations": "No proof of dominance, pointer lifetime, optimizer correctness or whole compiler"}


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("llvm", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        record = verify_llvm_guard(args.llvm.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        record = {"status": "FAIL", "reason": str(error)}
    rendered = json.dumps(record, ensure_ascii=False, indent=2) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0 if record["status"] == "LLVM_GUARD_DATAFLOW_MATCHED" else 1


if __name__ == "__main__":
    sys.exit(main())
