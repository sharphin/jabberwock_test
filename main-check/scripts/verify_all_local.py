#!/usr/bin/env python3
"""Local-only independent practical and mathematical proof audit.

Always fail closed: a passed subprocess without matching, complete report evidence
is not certification. Input case catalogs may grow without changing the 12+5 goals.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import platform
import subprocess
import sys

from acceptance_report import review as review_acceptance
from acceptance_requirements import test_input_identity
from formal_report import source_identity

ROOT = Path(__file__).resolve().parents[1]
COMPILERS = ("jwc", "rust")
GOALS = ("semantics", "memory", "types", "ownership", "checks")


def judge(practical: dict, formal: dict, compilers=COMPILERS) -> dict:
    if not isinstance(practical, dict) or not isinstance(formal, dict):
        return {"status": "INVALID_EVIDENCE", "reason": "reports must be JSON objects"}
    expected = set(compilers)
    expected_items = {(c, n) for c in expected for n in range(1, 13)}
    expected_goals = {(c, g) for c in expected for g in GOALS}
    items = practical.get("items")
    proofs = formal.get("goals")
    if not isinstance(items, list) or not isinstance(proofs, list):
        return {"status": "INVALID_EVIDENCE", "reason": "item or proof list missing"}
    try:
        item_keys = [(x["compiler"], x["item"]) for x in items]
        goal_keys = [(x["compiler"], x["goal"]) for x in proofs]
    except (KeyError, TypeError, AttributeError):
        return {"status": "INVALID_EVIDENCE", "reason": "item or proof identity missing"}
    if (len(item_keys) != len(expected_items) or set(item_keys) != expected_items or
            len(goal_keys) != len(expected_goals) or set(goal_keys) != expected_goals):
        return {"status": "INVALID_EVIDENCE", "reason": "missing, duplicate or alien goal/item"}
    if formal.get("infrastructure_error") or formal.get("interrupted"):
        return {"status": "INFRA_ERROR", "reason": "formal audit failed or was interrupted"}
    if practical.get("run_status") in ("SOURCE_IDENTITY_ERROR", "TEST_INPUTS_CHANGED", "SOURCE_CHANGED"):
        return {"status": "INVALID_EVIDENCE", "reason": "test/source snapshot changed during audit"}
    if practical.get("run_status") == "INTERRUPTED":
        return {"status": "INTERRUPTED", "reason": "practical audit interrupted"}
    if practical.get("run_status") != "COMPLETE":
        return {"status": "NOT_RUN", "reason": "practical run not complete"}
    if not isinstance(practical.get("suite"), dict) or practical["suite"].get("compilers") not in (list(compilers), tuple(compilers)):
        return {"status": "INVALID_EVIDENCE", "reason": "compiler provenance mismatch"}
    if not isinstance(formal.get("compilers"), dict) or set(formal["compilers"]) != expected:
        return {"status": "INVALID_EVIDENCE", "reason": "formal compiler provenance mismatch"}
    # Preserve why the local evidence is incomplete. A successful kernel proof
    # audit does not establish correspondence to compiled programs.
    practical_states = [x.get("status", "NOT_RUN") for x in items]
    proof_states = [x.get("proof_check", {}).get("status", "NOT_IMPLEMENTED") for x in proofs]
    if any(x == "INFRA_ERROR" for x in practical_states):
        return {"status": "INFRA_ERROR", "reason": "practical probe infrastructure error"}
    if any(x == "INTERRUPTED" for x in practical_states):
        return {"status": "INTERRUPTED", "reason": "practical item was interrupted"}
    if any(x in ("FAIL", "CRASH", "TIMEOUT") for x in practical_states):
        return {"status": "PROBE_FAILURE", "reason": "practical item failed, crashed or timed out"}
    if any(x == "FAIL" for x in proof_states):
        return {"status": "PROOF_CHECK_FAILURE", "reason": "Lean proof check failed"}
    if any(x not in ("PROBES_PASS_ONLY", "NOT_RUN", "PLANNED") for x in practical_states):
        return {"status": "INVALID_EVIDENCE", "reason": "unknown practical item state"}
    if any(x not in ("KERNEL_CHECKED", "FAIL", "STALE", "NOT_IMPLEMENTED", "NOT_RUN")
           for x in proof_states):
        return {"status": "INVALID_EVIDENCE", "reason": "unknown formal proof state"}
    if any(x in ("NOT_RUN", "PLANNED") for x in practical_states):
        return {"status": "NOT_RUN", "reason": "practical probes are missing"}
    if any(x == "STALE" for x in proof_states):
        return {"status": "PROOF_STALE", "reason": "proof binding does not match sources"}
    # Report strings are untrusted input. Do not certify safety from claimed PASS.
    # A future acceptance verifier must validate proof terms, implementation
    # correspondence, exact tool provenance and runtime assumptions first.
    claimed_practical = sum(x.get("acceptance") == "PASS" for x in items)
    claimed_formal = sum(x.get("acceptance") == "PROVEN" and
                         x.get("proof_check", {}).get("status") == "KERNEL_CHECKED" for x in proofs)
    return {"status": "NOT_PROVEN",
            "claimed_practical": claimed_practical,
            "claimed_formal": claimed_formal,
            "verified_practical": 0,
            "verified_formal": 0,
            "required_practical": len(expected_items),
            "required_formal": len(expected_goals),
            "reason": "No independent whole-implementation acceptance verifier exists; report strings are not proof"}

def practical_command_for(os_name: str, path: Path) -> list[str]:
    if os_name == "windows":
        return ["cmd", "/d", "/c", str(ROOT / "scripts/safety_windows.cmd"),
                "--compiler", "all", "--output", str(path)]
    if os_name == "linux":
        return [sys.executable, str(ROOT / "scripts/safety_linux.py"),
                "--compiler", "all", "--output", str(path)]
    raise ValueError("unsupported OS")


def formal_environment_for(os_name: str) -> dict[str, str]:
    env = dict(os.environ)
    if os_name == "windows":
        # The safety batch file initializes a child shell only; its PATH does
        # not propagate back to this process before the independent Lean run.
        home = Path.home()
        prefixes = [home / ".elan" / "bin", home / ".cargo" / "bin"]
        env["PATH"] = os.pathsep.join(str(p) for p in prefixes) + os.pathsep + env.get("PATH", "")
    return env


def main() -> int:
    # No-argument entry is the complete test battery. Explicit legacy evidence
    # arguments remain available for read-only re-auditing.
    if len(sys.argv) == 1:
        from run_everything import main as run_complete
        return run_complete([])
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "target/local-verification.json")
    parser.add_argument("--practical", type=Path, help="existing local safety report (otherwise execute suite)")
    parser.add_argument("--formal", type=Path, help="existing local formal report (otherwise execute Lean audit)")
    args = parser.parse_args()
    if platform.system() not in ("Windows", "Linux"):
        parser.error("only Linux and Windows safety suites exist")
    os_name = platform.system().lower()
    practical_path = args.practical or ROOT / "target" / ("safety-" + os_name + "-report.json")
    formal_path = args.formal or ROOT / "target/formal-report.json"
    executed = []
    practical_command = practical_command_for(os_name, practical_path)
    for supplied, label, command in (
        (args.practical, "practical", practical_command),
        (args.formal, "formal", [sys.executable, str(ROOT / "scripts/formal_report.py"),
                                "--compiler", "all", "--require-full", "--output", str(formal_path)])):
        if supplied is None:
            # Never interpret a report left by an older run as fresh evidence.
            try:
                Path(practical_path if label == "practical" else formal_path).unlink(missing_ok=True)
            except OSError as exc:
                executed.append({"stage": label, "error": "cannot remove stale report: " + str(exc)})
                continue
            try:
                result = subprocess.run(command, cwd=ROOT, timeout=86400,
                                        env=formal_environment_for(os_name))
                executed.append({"stage": label, "exit_code": result.returncode})
            except (OSError, subprocess.TimeoutExpired) as exc:
                executed.append({"stage": label, "error": str(exc)})
    def read(path):
        try:
            return json.loads(path.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            return {}
    practical = read(practical_path)
    formal = read(formal_path)
    verdict = judge(practical, formal)
    acceptance = {"status": "INVALID_EVIDENCE", "reason": "reports not reviewed"}
    try:
        acceptance = review_acceptance(
            practical, formal, local_os=os_name,
            identities={compiler: source_identity(compiler) for compiler in COMPILERS},
            inputs_identity=test_input_identity())
    except (OSError, ValueError, KeyError, TypeError, AttributeError) as exc:
        acceptance = {"status": "INVALID_EVIDENCE", "reason": str(exc)}
    errors = [r for r in executed if "error" in r or r.get("exit_code") not in (0, 1)]
    if errors:
        verdict = {"status": "INFRA_ERROR", "reason": "local suite failed to start or encountered infrastructure failure", "stages": errors}
    report = {"schema_version": 2, "kind": "local-verification", "verdict": verdict,
              "acceptance_34": acceptance,
              "runs": executed, "practical_report": str(practical_path), "formal_report": str(formal_path)}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(verdict, ensure_ascii=False, indent=2))
    print("Report:", args.output)
    if acceptance["status"] == "INVALID_EVIDENCE":
        return 2
    return 2 if verdict["status"] in ("INFRA_ERROR", "INVALID_EVIDENCE") else (0 if verdict["status"] == "PASS" and acceptance["status"] == "ACCEPTED" and all(r.get("exit_code") == 0 for r in executed) else 1)


if __name__ == "__main__":
    sys.exit(main())
