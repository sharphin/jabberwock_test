#!/usr/bin/env python3
"""Check concrete LLVM IR using LLVM's verifier, not emitter template strings.

Passing LLVM verify means syntactically/well-formed IR only. This does not
establish observational equivalence, no UB, or refinement to Jabberwock.
"""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys

def verify_ir(path: Path, opt: str, timeout: int = 120) -> dict:
    if not path.is_file() or path.suffix.lower() != ".ll":
        raise ValueError("missing or non-LLVM IR file: " + str(path))
    # LLVM verifier checks structural correctness, NOT behavioral equivalence.
    command = [opt, "-passes=verify", "-disable-output", str(path)]
    result = subprocess.run(command, capture_output=True, text=True,
                            encoding="utf-8", errors="replace", timeout=timeout)
    return {"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
            "exit_code": result.returncode, "status": "IR_WELL_FORMED" if result.returncode == 0 else "INVALID_IR",
            "stderr": result.stderr[-6000:],
            "semantic_refinement_proven": False}

def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ir-root", type=Path, required=True,
                        help="directory containing generated .ll outputs")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--benchmark-report", type=Path,
                        help="require exact IR files for all reported benchmark cases")
    parser.add_argument("--opt", default="opt")
    parser.add_argument("--semantic-pairs", type=Path,
                        help="JSON source/optimized LLVM pairs checked with Alive2 alive-tv")
    parser.add_argument("--alive-tv", default="alive-tv")
    args = parser.parse_args(argv)
    results = []
    errors = []
    tool = shutil.which(args.opt)
    if tool is None:
        errors.append("LLVM opt unavailable: " + args.opt)
    files = sorted(args.ir_root.rglob("*.ll")) if args.ir_root.is_dir() else []
    if not files:
        errors.append("no concrete LLVM IR found; emitter template matching is not sufficient")
    if args.benchmark_report:
        try:
            benchmark = json.loads(args.benchmark_report.read_text(encoding="utf-8"))
            cases = benchmark["cases"]
            suite = benchmark.get("suite")
            if (not isinstance(cases, dict) or not cases or not isinstance(suite, list)
                    or any(not isinstance(n, str) or not n or n in (".", "..")
                           or "/" in n or chr(92) in n for n in suite)
                    or len(suite) != len(set(suite)) or set(suite) != set(cases)):
                raise ValueError("missing, foreign or unsafe benchmark case identities")
            expected = {(args.ir_root / name / "jwc.ll").resolve() for name in suite}
            actual = {p.resolve() for p in files}
            if actual != expected:
                errors.append("missing or unexpected generated LLVM IR: " + str({
                    "missing": sorted(str(p) for p in expected - actual),
                    "extra": sorted(str(p) for p in actual - expected)}))
            if any(not isinstance(case, dict) or case.get("status") != "PASS"
                   for case in cases.values()):
                errors.append("benchmark compilation/execution did not pass for every case")
        except (OSError, ValueError, TypeError, KeyError) as exc:
            errors.append("benchmark report not verifiable: " + str(exc))
    if tool:
        for path in files:
            try:
                results.append(verify_ir(path, tool))
            except (OSError, ValueError, subprocess.TimeoutExpired) as exc:
                errors.append(str(exc))
    semantic_results = []
    if args.semantic_pairs:
        try:
            pair_doc = json.loads(args.semantic_pairs.read_text(encoding="utf-8"))
            pairs = pair_doc["pairs"]
            if not isinstance(pairs, list) or not pairs:
                raise ValueError("no semantic refinement pairs")
            alive = shutil.which(args.alive_tv)
            if alive is None:
                raise ValueError("Alive2 alive-tv unavailable")
            seen = set()
            for pair in pairs:
                if not isinstance(pair, dict) or set(pair) != {"before", "after"}:
                    raise ValueError("semantic pair must contain only before/after paths")
                if (not isinstance(pair["before"], str) or
                        not isinstance(pair["after"], str) or
                        Path(pair["before"]).is_absolute() or
                        Path(pair["after"]).is_absolute()):
                    raise ValueError("invalid LLVM refinement pair paths")
                before = (args.ir_root / pair["before"]).resolve()
                after = (args.ir_root / pair["after"]).resolve()
                if (not before.is_relative_to(args.ir_root.resolve()) or
                        not after.is_relative_to(args.ir_root.resolve()) or
                        not before.is_file() or not after.is_file() or
                        before.suffix != ".ll" or after.suffix != ".ll"):
                    raise ValueError("missing/escaping LLVM refinement input")
                identity = (str(before), str(after))
                if identity in seen:
                    raise ValueError("duplicate LLVM refinement pair")
                seen.add(identity)
                execution = subprocess.run([alive, str(before), str(after)],
                                           capture_output=True, text=True, encoding="utf-8",
                                           errors="replace", timeout=120)
                output_text = execution.stdout + execution.stderr
                # Alive2 can fail or report unsupported/timeout without returning a
                # semantic proof. Require explicit successful transformation proof.
                success = (execution.returncode == 0 and
                           "Transformation seems to be correct!" in output_text and
                           "ERROR:" not in output_text and
                           "Timeout" not in output_text)
                semantic_results.append({"before": str(before), "after": str(after),
                                         "before_sha256": hashlib.sha256(before.read_bytes()).hexdigest(),
                                         "after_sha256": hashlib.sha256(after.read_bytes()).hexdigest(),
                                         "status": "REFINEMENT_CHECKED" if success else "BLOCKED",
                                         "checker_exit": execution.returncode,
                                         "checker_output": output_text[-6000:]})
                if not success:
                    errors.append("Alive2 failed to establish IR refinement: " + str(identity))
        except (OSError, ValueError, KeyError, TypeError, subprocess.TimeoutExpired) as exc:
            errors.append("semantic refinement failed: " + str(exc))
    status = "IR_VERIFIED_ONLY" if not errors and all(x["status"] == "IR_WELL_FORMED" for x in results) else "BLOCKED"
    report = {"schema_version": 1, "kind": "llvm-ir-audit", "status": status,
              "results": results, "semantic_results": semantic_results, "errors": errors,
              "semantic_refinement_proven": False,
              "limitations": ["LLVM verifier does not prove absence of undefined behavior.",
                              "Source-to-LLVM semantics and optimization refinement remain unproved."]}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(status, "->", args.output, flush=True)
    return 0 if status == "IR_VERIFIED_ONLY" else 1

if __name__ == "__main__":
    sys.exit(main())
