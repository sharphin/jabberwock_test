#!/usr/bin/env python3
"""One command: run the complete local Jabberwock verification battery.

Commands are executed, not inferred from summary metadata. Each stage keeps
its stdout/stderr and independent exit status; no failed step is silently
skipped and unknown/unproved acceptance always fails the final gate.
"""
from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import time

from formal_report import source_identity, proof_sources_identity

ROOT = Path(__file__).resolve().parents[1]
WINDOWS = platform.system() == "Windows"
LINUX = platform.system() == "Linux"

def plans(output: Path, *, quick: bool = False) -> list[tuple[str, list[str]]]:
    py = sys.executable
    safety = (["cmd", "/d", "/c", str(ROOT / "scripts/safety_windows.cmd")]
              if WINDOWS else [py, str(ROOT / "scripts/safety_linux.py")])
    benchmark = (["cmd", "/d", "/c", str(ROOT / "scripts/benchmark_windows.cmd")]
                 if WINDOWS else [py, str(ROOT / "scripts/benchmark_report.py")])
    target_os = "windows" if WINDOWS else "linux"
    practical = output / "safety.json"
    formal = output / "formal.json"
    result = [
        ("python-regressions", [py, "-m", "unittest", "discover", "-s", "scripts", "-p", "test_*.py"]),
        ("cargo-tests", ["cargo", "test", "--offline", "--all-targets"]),
        ("cargo-doctests", ["cargo", "test", "--offline", "--doc"]),
        ("cargo-clippy", ["cargo", "clippy", "--offline", "--all-targets", "--", "-D", "warnings"]),
        ("practical-safety", [*safety, "--compiler", "all", "--output", str(practical)]),
        ("whole-implementation-obligations", [py, str(ROOT / "scripts/whole_compiler_proof.py"),
                                              "--output", str(output / "whole-proof-inventory.json")]),
        ("refinement-transition", [py, str(ROOT / "scripts/refinement_transition.py"),
                                   "--output", str(output / "refinement-transition.json")]),
        ("build-dependency-audit", [py, str(ROOT / "scripts/build_dependency_audit.py"),
                                    "--output", str(output / "build-dependency-audit.json")]),
        ("emitter-proof-binding", [py, str(ROOT / "scripts/proof_emitter_correspondence.py"),
                                   "--output", str(output / "emitter-proof-binding.json")]),
        ("formal-proofs", [py, str(ROOT / "scripts/formal_report.py"),
                           "--compiler", "all", "--abstract-model", "--require-full",
                           "--output", str(formal)]),
        ("acceptance-34", [py, str(ROOT / "scripts/acceptance_report.py"),
                            "--os", target_os, "--practical", str(practical), "--formal", str(formal),
                            "--inventory", str(output / "whole-proof-inventory.json"),
                            "--transition", str(output / "refinement-transition.json"),
                            "--output", str(output / "acceptance-34.json")]),
        ("benchmark-four-way", [*benchmark, *(["--full"] if not quick else []),
                                "--output", str(output / "benchmark-four-way.json")]),
        ("llvm-concrete-ir-audit", [py, str(ROOT / "scripts/llvm_ir_audit.py"),
                                   "--ir-root", str(output / "benchmark-four-way-work"),
                                   "--benchmark-report", str(output / "benchmark-four-way.json"),
                                   "--output", str(output / "llvm-ir-audit.json")]),
        ("legacy-optimization-counts", [py, str(ROOT / "scripts/check_bench_checks.py")]),
    ]
    return result


def execute_stage(label: str, command: list[str], output: Path, *, timeout: int) -> dict:
    stdout = output / (label + ".stdout.log")
    stderr = output / (label + ".stderr.log")
    started = dt.datetime.now(dt.timezone.utc).isoformat()
    begin = time.monotonic()
    record = {"name": label, "command": command, "started_utc": started,
              "stdout_log": str(stdout), "stderr_log": str(stderr)}
    print("START", label, flush=True)
    with stdout.open("wb") as out, stderr.open("wb") as err:
        try:
            proc = subprocess.run(command, cwd=ROOT, stdout=out, stderr=err,
                                  timeout=timeout, check=False)
            record["exit_code"] = proc.returncode
            record["status"] = "EXIT_ZERO" if proc.returncode == 0 else "EXIT_NONZERO"
        except subprocess.TimeoutExpired:
            record["status"] = "TIMEOUT"
            record["reason"] = "time limit exceeded: " + str(timeout) + "s"
        except OSError as error:
            record["status"] = "INFRA_ERROR"
            record["reason"] = str(error)
    # Bind every stage result to the exact captured logs, including failures.
    # The evidence bundle can independently verify these digests.
    for stream, path in (("stdout", stdout), ("stderr", stderr)):
        try:
            digest = hashlib.sha256()
            with path.open("rb") as evidence:
                for chunk in iter(lambda: evidence.read(1024 * 1024), b""):
                    digest.update(chunk)
            record[stream + "_sha256"] = digest.hexdigest()
            record[stream + "_bytes"] = path.stat().st_size
        except OSError as error:
            record["status"] = "INFRA_ERROR"
            record["reason"] = "unable to seal " + stream + " log: " + str(error)
    record["duration_s"] = round(time.monotonic() - begin, 3)
    print(record["status"], label, "(logs:", stdout, stderr, ")", flush=True)
    return record


def evidence_overview(output: Path) -> dict:
    """Extract actionable proof gaps regardless of nonzero stage exit codes."""
    paths = {
        "implementation_inventory": "whole-proof-inventory.json",
        "build_dependencies": "build-dependency-audit.json",
        "refinement_transition": "refinement-transition.json",
        "llvm_ir": "llvm-ir-audit.json",
        "formal": "formal.json",
        "acceptance": "acceptance-34.json",
        "practical": "safety.json",
        "benchmark": "benchmark-four-way.json",
    }
    summary = {}
    for label, filename in paths.items():
        try:
            value = json.loads((output / filename).read_text(encoding="utf-8"))
            if not isinstance(value, dict):
                raise ValueError("report is not a JSON object")
            data = {"status": value.get("status", value.get("run_status", "UNKNOWN"))}
            if label == "implementation_inventory":
                data.update(required=value.get("total_obligations"),
                            proved=value.get("proved_obligations"),
                            uncovered=[x.get("id") for x in value.get("components", [])
                                       if x.get("status") != "REFINEMENT_PROVEN"])
            elif label == "refinement_transition":
                data.update(required=value.get("required"), proved=value.get("proved"),
                            invalid=value.get("invalid"))
            elif label == "build_dependencies":
                data.update(checks=value.get("checks", []))
            elif label == "llvm_ir":
                data.update(results=value.get("results", []), errors=value.get("errors", []))
            elif label == "formal":
                data.update(goals=value.get("summary", {}),
                            infrastructure_error=value.get("infrastructure_error"))
            elif label == "acceptance":
                data.update(summary=value.get("summary", {}),
                            blockers={x["id"]: x.get("blockers", [])
                                      for x in value.get("obligations", [])
                                      if x.get("blockers")})
            elif label == "practical":
                data.update(failed_items=[x.get("id") for x in value.get("items", [])
                                          if x.get("status") != "PROBES_PASS_ONLY"])
            elif label == "benchmark":
                data.update(failed_cases=[name for name, case in value.get("cases", {}).items()
                                          if case.get("status") != "PASS"])
            summary[label] = data
        except (OSError, ValueError, TypeError, KeyError, AttributeError) as exc:
            summary[label] = {"status": "MISSING_OR_INVALID", "reason": str(exc)}
    return summary


def verify_stage_log_integrity(stage: dict, output: Path | None = None) -> bool:
    """Fail closed if any captured log differs from its recorded SHA-256/size.

    Legacy/manually synthesized stage records without sealed evidence are not
    accepted as having verified log integrity.
    """
    for name in ("stdout", "stderr"):
        stored_path = stage.get(name + "_log")
        expected = stage.get(name + "_sha256")
        expected_size = stage.get(name + "_bytes")
        if not isinstance(stored_path, str) or not isinstance(expected, str):
            return False
        if len(expected) != 64 or not isinstance(expected_size, int) or expected_size < 0:
            return False
        try:
            path = Path(stored_path)
            if path.is_symlink() or not path.is_file():
                return False
            if output is not None:
                expected_path = output / (stage.get("name", "") + "." + name + ".log")
                if path.resolve() != expected_path.resolve():
                    return False
            digest = hashlib.sha256()
            length = 0
            with path.open("rb") as stream:
                for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                    digest.update(chunk)
                    length += len(chunk)
            if digest.hexdigest() != expected or length != expected_size:
                return False
        except OSError:
            return False
    return True


def summarize(stages: list[dict], output: Path, *, quick: bool) -> dict:
    by_name = {x["name"]: x for x in stages}
    expected = {"python-regressions", "cargo-tests", "cargo-doctests", "cargo-clippy", "practical-safety", "whole-implementation-obligations", "refinement-transition", "build-dependency-audit", "emitter-proof-binding", "formal-proofs",
                "acceptance-34", "benchmark-four-way", "llvm-concrete-ir-audit", "legacy-optimization-counts"}
    verdict = "FAIL"
    reason = "one or more verification stages failed"
    if set(by_name) != expected:
        reason = "required stages missing or duplicated"
    elif len(stages) == len(expected) and all(x.get("status") == "EXIT_ZERO" and x.get("exit_code") == 0 for x in stages) and all(verify_stage_log_integrity(x, output) for x in stages):
        try:
            acceptance = json.loads((output / "acceptance-34.json").read_text(encoding="utf-8"))
            benchmark = json.loads((output / "benchmark-four-way.json").read_text(encoding="utf-8"))
            practical = json.loads((output / "safety.json").read_text(encoding="utf-8"))
            formal = json.loads((output / "formal.json").read_text(encoding="utf-8"))
            count = acceptance.get("summary", {})
            if (acceptance.get("status") == "ACCEPTED" and count.get("accepted") == 34 and
                    practical.get("run_status") == "COMPLETE" and
                    formal.get("summary", {}).get("all_compilers_proven") is True and
                    all(c.get("status") == "PASS" for c in benchmark.get("cases", {}).values()) and
                    len(benchmark.get("cases", {})) == (5 if quick else 21)):
                verdict, reason = "PASS", "all required evidence and execution stages passed"
            else:
                reason = "34-obligation proof, safety, or benchmark acceptance not established"
        except (OSError, ValueError, TypeError, AttributeError) as error:
            reason = "missing or malformed final reports: " + str(error)
    evidence = evidence_overview(output)
    failed_stage_names = [x["name"] for x in stages if x.get("status") != "EXIT_ZERO" or x.get("exit_code") != 0]
    invalid_logs = [x["name"] for x in stages if not verify_stage_log_integrity(x, output)]
    if failed_stage_names:
        reason = "verification stages failed: " + ", ".join(failed_stage_names)
    elif invalid_logs:
        reason = "stage logs are missing or differ from recorded hashes: " + ", ".join(invalid_logs)
    elif verdict != "PASS" and evidence["implementation_inventory"]["status"] == "NOT_PROVEN":
        reason = "compiler-wide refinement obligations remain unproved; inspect evidence.implementation_inventory.uncovered"
    return {"schema_version": 2, "kind": "local-full-verification",
            "status": verdict, "reason": reason, "mode": "quick" if quick else "full",
            "host_os": platform.system(), "stages": stages,
            "failed_stages": failed_stage_names,
            "invalid_stage_logs": invalid_logs,
            "evidence": evidence,
            "passed_stages": sum(x["status"] == "EXIT_ZERO" for x in stages),
            "required_stages": len(expected),
            "created_utc": dt.datetime.now(dt.timezone.utc).isoformat()}


def source_snapshots() -> dict:
    """Pin both compilers and all formal dependencies to exact file contents."""
    return {
        "compilers": {name: source_identity(name) for name in ("jwc", "rust")},
        "formal": proof_sources_identity(),
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "target" / "verification-complete")
    parser.add_argument("--quick", action="store_true",
                        help="5 benchmark cases rather than 21; never qualifies as full acceptance")
    parser.add_argument("--stage-timeout", type=int, default=86400)
    args = parser.parse_args(argv)
    if not (WINDOWS or LINUX):
        parser.error("supported only on Windows and Linux")
    if args.stage_timeout < 1:
        parser.error("stage timeout must be positive")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    try:
        initial_snapshots = source_snapshots()
        snapshot_error = None
    except (OSError, ValueError, KeyError) as exc:
        initial_snapshots = None
        snapshot_error = "unable to snapshot starting sources: " + str(exc)
    stages = []
    for label, command in plans(output, quick=args.quick):
        # Delete prior output: never reuse a report from a previous attempt.
        report_name = {"practical-safety": "safety.json", "formal-proofs": "formal.json",
                       "emitter-proof-binding": "emitter-proof-binding.json",
                       "build-dependency-audit": "build-dependency-audit.json",
                       "refinement-transition": "refinement-transition.json",
                       "whole-implementation-obligations": "whole-proof-inventory.json",
                       "acceptance-34": "acceptance-34.json",
                       "benchmark-four-way": "benchmark-four-way.json",
                       "llvm-concrete-ir-audit": "llvm-ir-audit.json"}.get(label)
        if report_name:
            try:
                (output / report_name).unlink(missing_ok=True)
            except OSError as exc:
                stages.append({"name": label, "status": "INFRA_ERROR", "reason": str(exc)})
                continue
        stages.append(execute_stage(label, command, output, timeout=args.stage_timeout))
    result = summarize(stages, output, quick=args.quick)
    try:
        final_snapshots = source_snapshots()
        if initial_snapshots != final_snapshots:
            snapshot_error = "compiler or formal sources changed during verification"
    except (OSError, ValueError, KeyError) as exc:
        snapshot_error = "unable to snapshot final sources: " + str(exc)
    result["source_snapshot_consistent"] = snapshot_error is None
    if snapshot_error:
        result["status"] = "FAIL"
        result["reason"] = snapshot_error
    if args.quick:
        result["status"] = "FAIL"
        result["reason"] = "quick sampling cannot satisfy full acceptance"
    destination = output / "summary.json"
    destination.write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print("FINAL:", result["status"], "|", result["reason"], flush=True)
    print("Report:", destination, flush=True)
    return 0 if result["status"] == "PASS" else 1


if __name__ == "__main__":
    sys.exit(main())
