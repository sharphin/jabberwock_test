#!/usr/bin/env python3
"""Independently replay a supported implementation contract before accepting it.

This executes the proof verifier, not compiler test cases. Acceptance is scoped
to the registered contract and its explicit trusted-frontend premises. It never
discharges a whole-compiler obligation or imports a verifier from a report.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys

from runtime_source_contract import ACCEPTED, contract_id, replay


def accept(report: dict, timeout: int = 600, *, compiler: str = "jwc") -> dict:
    if timeout < 1:
        raise ValueError("timeout must be positive")
    # Fixed local dispatcher: a report cannot choose code, flags, a weaker
    # expected proposition, a different compiler, or a new trust policy.
    if not isinstance(report, dict) or report.get("contract_id") != contract_id(compiler):
        raise ValueError("no registered verifier for this implementation contract")
    verified = replay(report, timeout, compiler=compiler)
    return {"schema_version": 1, "kind": "source-contract-acceptance",
            "status": ACCEPTED, "compiler": compiler, "contract_id": contract_id(compiler),
            "verification": "INDEPENDENT_LOCAL_REPLAY",
            "source_snapshot": verified["source_snapshot"],
            "theorem": verified["theorem"],
            "generated_proof_sha256": verified["generated_proof_sha256"],
            "quantification": verified["quantification"],
            "trust_premises": verified["trust_premises"],
            "excluded": verified["excluded"],
            "whole_implementation": "NOT_PROVEN",
            "discharged_whole_compiler_obligations": []}


def accept_file(path: Path, timeout: int = 600, *, compiler: str = "jwc") -> dict:
    if path.is_symlink() or not path.is_file():
        raise ValueError("contract evidence must be a regular file")
    raw = path.read_bytes()
    result = accept(json.loads(raw), timeout, compiler=compiler)
    if path.is_symlink() or path.read_bytes() != raw:
        raise ValueError("contract evidence changed during verification")
    result["evidence_sha256"] = hashlib.sha256(raw).hexdigest()
    return result


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--compiler", choices=("jwc", "rust"), default="jwc")
    parser.add_argument("--timeout", type=int, default=600,
                        help="timeout per verifier subprocess, in seconds")
    args = parser.parse_args(argv)
    if args.timeout < 1:
        parser.error("timeout must be positive")
    if args.report.resolve() == args.output.resolve():
        parser.error("output must not overwrite input evidence")
    try:
        result = accept_file(args.report, args.timeout, compiler=args.compiler)
    except (OSError, ValueError, KeyError, TypeError, RecursionError,
            subprocess.TimeoutExpired) as error:
        result = {"schema_version": 1, "kind": "source-contract-acceptance",
                  "status": "INVALID_EVIDENCE", "reason": str(error),
                  "whole_implementation": "NOT_PROVEN",
                  "discharged_whole_compiler_obligations": []}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n",
                           encoding="utf-8")
    print(result["status"], "->", args.output, flush=True)
    return 0 if result["status"] == ACCEPTED else 2


if __name__ == "__main__":
    sys.exit(main())
