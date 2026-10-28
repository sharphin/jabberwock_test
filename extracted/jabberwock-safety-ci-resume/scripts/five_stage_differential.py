#!/usr/bin/env python3
"""Executable, fail-closed five-stage differential verification driver.

Input JSON schema:
{
  "schema_version": 1,
  "stages": {
    "source_semantics": {"jwc": ["tool", "arg", "{source}"], "rust": [...]},
    "parse": {...}, "typecheck": {...}, "lower": {...}, "ir_semantics": {...}
  },
  "cases": [{"id": "unique", "source": "tests/example.jw",
             "expect": {"source_semantics": {"exit": 0, "stdout": "..."},
                        "parse": {"exit": 0, "stdout": "..."}, ...}}]
}
Commands are argument arrays (never shell strings). The checker does NOT infer
semantic completeness from a finite test suite or allow absent stages to PASS.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys

STAGES = ("source_semantics", "parse", "typecheck", "lower", "ir_semantics")
COMPILERS = ("jwc", "rust")


def _object(value, context):
    if not isinstance(value, dict):
        raise ValueError(context + " must be an object")
    return value


def validate(config, root):
    cfg = _object(config, "config")
    if cfg.get("schema_version") != 1:
        raise ValueError("unsupported schema")
    drivers = _object(cfg.get("stages"), "stages")
    if set(drivers) != set(STAGES):
        raise ValueError("all five stages must have drivers")
    for stage in STAGES:
        row = _object(drivers[stage], stage)
        if set(row) != set(COMPILERS):
            raise ValueError(stage + " must have both compilers")
        for compiler in COMPILERS:
            cmd = row[compiler]
            if not isinstance(cmd, list) or not cmd or any(
                not isinstance(part, str) or not part for part in cmd
            ):
                raise ValueError("invalid argv: " + compiler + "/" + stage)
            if not any("{source}" in part for part in cmd):
                raise ValueError("driver does not consume input: " + compiler + "/" + stage)
    cases = cfg.get("cases")
    if not isinstance(cases, list) or not cases:
        raise ValueError("no cases")
    identities = set()
    for case in cases:
        case = _object(case, "case")
        ident = case.get("id")
        if not isinstance(ident, str) or not ident or ident in identities:
            raise ValueError("missing/duplicate case ID")
        identities.add(ident)
        src = case.get("source")
        if not isinstance(src, str) or not src or Path(src).is_absolute():
            raise ValueError("unsafe source path")
        file = (root / src).resolve()
        if not file.is_relative_to(root.resolve()) or not file.is_file():
            raise ValueError("missing or escaped case source: " + src)
        expected = _object(case.get("expect"), "expect")
        if set(expected) != set(STAGES):
            raise ValueError("case missing stage expectations: " + ident)
        for stage in STAGES:
            observation = _object(expected[stage], stage)
            if set(observation) != {"exit", "stdout"}:
                raise ValueError("expectations require exact exit and stdout")
            if not isinstance(observation["exit"], int) or isinstance(observation["exit"], bool):
                raise ValueError("expected exit must be an integer")
            if not isinstance(observation["stdout"], str):
                raise ValueError("expected stdout must be a string")


def evaluate(config, root, timeout=30):
    validate(config, root)
    results = []
    for case in config["cases"]:
        source = (root / case["source"]).resolve()
        digest = hashlib.sha256(source.read_bytes()).hexdigest()
        for stage in STAGES:
            expected = case["expect"][stage]
            for compiler in COMPILERS:
                argv = [part.replace("{source}", str(source))
                        for part in config["stages"][stage][compiler]]
                try:
                    completed = subprocess.run(
                        argv, cwd=root, capture_output=True, text=True,
                        timeout=timeout, check=False, encoding="utf-8", errors="replace")
                    matched = (completed.returncode == expected["exit"]
                               and completed.stdout == expected["stdout"])
                    result = {"status": "PASS" if matched else "FAIL",
                              "exit": completed.returncode,
                              "stdout": completed.stdout,
                              "stderr": completed.stderr}
                except (OSError, subprocess.TimeoutExpired) as exc:
                    result = {"status": "ERROR", "reason": str(exc)}
                results.append({"case": case["id"], "source_sha256": digest,
                                "stage": stage, "compiler": compiler, **result})
    passed = sum(row["status"] == "PASS" for row in results)
    return {"schema_version": 1, "status": "PASS" if passed == len(results) else "FAIL",
            "passed": passed, "total": len(results), "results": results,
            "scope": "finite differential tests only; not a universal semantic proof"}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("configuration", type=Path)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--timeout", type=float, default=30)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        if args.timeout <= 0:
            raise ValueError("timeout must be positive")
        result = evaluate(json.loads(args.configuration.read_text(encoding="utf-8")),
                          args.root.resolve(), args.timeout)
    except (OSError, ValueError, TypeError, KeyError) as exc:
        result = {"schema_version": 1, "status": "INVALID", "reason": str(exc)}
    rendered = json.dumps(result, indent=2, ensure_ascii=False) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0 if result["status"] == "PASS" else 1


if __name__ == "__main__":
    sys.exit(main())
