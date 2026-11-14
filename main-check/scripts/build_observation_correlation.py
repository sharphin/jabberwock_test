#!/usr/bin/env python3
"""Correlate declared build DAG leaves with depfile-observed input identities.

This is a verification of the observation-to-declaration correspondence, NOT
an assertion that depfiles capture every possible read or build-system event.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import sys

from build_closure_graph import audit
from build_depfile_verifier import verify


def correlate(graph: dict, dep_manifest: dict, *, root: Path, depdir: Path,
              build_cwd: Path, environment: dict[str, str], compiler: str) -> dict:
    dag = audit(graph, root, environment, compiler=compiler)
    dep = verify(dep_manifest, depdir, repository=root, build_cwd=build_cwd)
    if dep["compiler"] != compiler:
        raise ValueError("compiler mismatch between depfile and graph")
    file_nodes = {
        node["path"] for identifier, node in graph["nodes"].items()
        if identifier in dag["leaf_hashes"] and node["kind"] == "file"
    }
    observed = set(dep["observed_inputs"])
    missing = sorted(observed - file_nodes)
    if missing:
        raise ValueError("depfile-observed dependencies absent from DAG: " + repr(missing))
    return {"schema_version": 1, "kind": "build-observation-correlation",
            "status": "OBSERVED_INPUTS_MATCH_DECLARED_GRAPH",
            "compiler": compiler, "observed_count": len(observed),
            "declared_files": len(file_nodes),
            "unobserved_declared_files": sorted(file_nodes - observed),
            "complete_build_closure_proven": False,
            "limitations": [
                "Depfiles omit many environment, linker, runtime, and tool reads.",
                "Completeness requires independent tracing of the actual build process."]}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--graph", type=Path, required=True)
    parser.add_argument("--dep-manifest", type=Path, required=True)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--depdir", type=Path, required=True)
    parser.add_argument("--build-cwd", type=Path, required=True)
    parser.add_argument("--compiler", choices=("jwc", "rust"), required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        result = correlate(
            json.loads(args.graph.read_text(encoding="utf-8")),
            json.loads(args.dep_manifest.read_text(encoding="utf-8")),
            root=args.root, depdir=args.depdir, build_cwd=args.build_cwd,
            environment=dict(os.environ), compiler=args.compiler)
    except (OSError, ValueError, TypeError, KeyError) as exc:
        result = {"schema_version": 1, "kind": "build-observation-correlation",
                  "status": "BLOCKED", "reason": str(exc),
                  "complete_build_closure_proven": False}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(result["status"], "->", args.output)
    return 0 if result["status"] == "OBSERVED_INPUTS_MATCH_DECLARED_GRAPH" else 1


if __name__ == "__main__":
    sys.exit(main())
