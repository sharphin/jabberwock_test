#!/usr/bin/env python3
"""Run a traced build and validate every normalized access against its build DAG.

This command performs a build, so it is run locally by the user and never by
the assistant. Completion is not asserted unless a separately established,
complete observation contract is available.
"""
from __future__ import annotations
import argparse
import json
import os
from pathlib import Path
import sys

from linux_build_trace import collect
from build_access_audit import check_events


def verify_build(command: list[str], cwd: Path, graph: dict, compiler: str,
                 evidence_path: Path, environment: dict[str, str]) -> dict:
    capture = collect(command, cwd, evidence_path)
    if capture["status"] != "CAPTURED_UNVERIFIED":
        raise ValueError("build capture failed: " + repr(capture["errors"]))
    event_stream = capture.get("normalized_events")
    if event_stream is None:
        raise ValueError("capture did not produce normalized events")
    report = check_events(graph, event_stream, cwd, compiler, environment)
    if report["status"] != "RECORDED_ACCESSES_ACCOUNTED_FOR":
        raise ValueError("access auditing failed")
    return {"schema_version": 1, "kind": "traced-build-closure",
            "status": "RECORDED_ACCESSES_ACCOUNTED_FOR",
            "compiler": compiler,
            "capture_report": str(evidence_path),
            "capture_sha256": capture["trace_sha256"],
            "access_audit": report,
            "complete_build_closure_proven": False}


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--graph", type=Path, required=True)
    p.add_argument("--compiler", choices=("jwc", "rust"), required=True)
    p.add_argument("--cwd", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("command", nargs=argparse.REMAINDER)
    args = p.parse_args(argv)
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    try:
        graph = json.loads(args.graph.read_text(encoding="utf-8"))
        capture_path = args.output.with_name(args.output.stem + "-trace.json")
        result = verify_build(command, args.cwd, graph, args.compiler,
                              capture_path, dict(os.environ))
    except (OSError, ValueError, TypeError, KeyError) as exc:
        result = {"schema_version": 1, "kind": "traced-build-closure",
                  "status": "BLOCKED", "reason": str(exc),
                  "complete_build_closure_proven": False}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n",
                           encoding="utf-8")
    return 0 if result["status"] == "RECORDED_ACCESSES_ACCOUNTED_FOR" else 1


if __name__ == "__main__":
    sys.exit(main())
