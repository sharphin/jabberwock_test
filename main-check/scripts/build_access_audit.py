#!/usr/bin/env python3
"""Match a monitored build's normalized read/exec/env events to the declared DAG.

A monitor adapter must capture every relevant process, including children,
with a complete event stream. This consumer rejects missing end markers,
unknown event types and unregistered inputs; it cannot certify the adapter's
own observational completeness.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import sys

from build_closure_graph import audit


def check_events(graph: dict, events: dict, root: Path, compiler: str,
                 environment: dict[str, str]) -> dict:
    dag = audit(graph, root, environment, compiler=compiler)
    if (not isinstance(events, dict) or events.get("schema_version") != 1
            or events.get("kind") != "build-access-events"):
        raise ValueError("invalid build access stream")
    sequence = events.get("events")
    if not isinstance(sequence, list) or len(sequence) < 2:
        raise ValueError("empty or truncated access stream")
    if sequence[0] != {"type": "begin"} or sequence[-1] != {"type": "end"}:
        raise ValueError("missing complete observation boundaries")
    files, tools, variables = set(), set(), set()
    root = root.resolve()
    for identifier, node in graph["nodes"].items():
        if identifier not in dag["leaf_hashes"]:
            continue
        if node["kind"] == "file":
            files.add(str((root / node["path"]).resolve()))
        elif node["kind"] == "tool":
            tools.add(str((root / node["path"]).resolve()))
        elif node["kind"] == "environment":
            variables.add(node["name"])
    observed = set()
    for index, event in enumerate(sequence[1:-1], 1):
        if not isinstance(event, dict):
            raise ValueError("nonobject build access event " + str(index))
        kind = event.get("type")
        if kind == "read":
            if set(event) != {"type", "path"} or not isinstance(event["path"], str):
                raise ValueError("invalid read event")
            path = Path(event["path"]).resolve()
            if str(path) not in files:
                raise ValueError("unregistered file read: " + str(path))
            observed.add(("read", str(path)))
        elif kind == "exec":
            if set(event) != {"type", "path"} or not isinstance(event["path"], str):
                raise ValueError("invalid exec event")
            path = Path(event["path"]).resolve()
            if str(path) not in tools:
                raise ValueError("unregistered tool execution: " + str(path))
            observed.add(("exec", str(path)))
        elif kind == "environment":
            if set(event) != {"type", "name"} or not isinstance(event["name"], str):
                raise ValueError("invalid environment event")
            if event["name"] not in variables:
                raise ValueError("unregistered environment read: " + event["name"])
            observed.add(("environment", event["name"]))
        else:
            raise ValueError("unrecognized build event " + str(kind))
    return {
        "schema_version": 1, "kind": "build-access-audit",
        "status": "RECORDED_ACCESSES_ACCOUNTED_FOR",
        "events_checked": len(sequence) - 2,
        "distinct_inputs_observed": len(observed),
        "source_coverage": dag["source_coverage"],
        "monitor_completeness_verified": False,
        "complete_build_closure_proven": False,
        "limitations": ["An event stream cannot attest its own lack of omissions.",
                        "A trusted OS/build-system monitor is required to assert exhaustive observation."]}


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--graph", type=Path, required=True)
    p.add_argument("--events", type=Path, required=True)
    p.add_argument("--root", type=Path, required=True)
    p.add_argument("--compiler", choices=("jwc", "rust"), required=True)
    p.add_argument("--output", type=Path, required=True)
    args = p.parse_args(argv)
    try:
        result = check_events(json.loads(args.graph.read_text(encoding="utf-8")),
                              json.loads(args.events.read_text(encoding="utf-8")),
                              args.root, args.compiler, dict(os.environ))
    except (OSError, ValueError, TypeError, KeyError) as exc:
        result = {"schema_version": 1, "kind": "build-access-audit",
                  "status": "BLOCKED", "reason": str(exc),
                  "complete_build_closure_proven": False}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(result["status"], "->", args.output)
    return 0 if result["status"] == "RECORDED_ACCESSES_ACCOUNTED_FOR" else 1


if __name__ == "__main__":
    sys.exit(main())
