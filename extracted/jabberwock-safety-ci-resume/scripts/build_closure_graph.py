#!/usr/bin/env python3
"""Verify a declared build DAG and the hashes of all reachable input artifacts.

Contract: a manifest must identify every input, executable and environment
variable read by the build. This verifies the declared transitive closure,
NOT the truth or completeness of the producer's observations. A separate,
trusted build-system monitor is required for that completeness claim.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path, PureWindowsPath
import re
import sys

from formal_report import source_identity

HEX = re.compile(r"[0-9a-f]{64}\Z")


def safe_relative_path(name: str) -> bool:
    """Reject cross-platform absolute paths, aliases, and traversal."""
    if not isinstance(name, str) or not name or chr(0) in name or chr(92) in name:
        return False
    windows = PureWindowsPath(name)
    return (not Path(name).is_absolute() and not windows.is_absolute()
            and not windows.drive and
            all(part not in ("", ".", "..") for part in name.split("/")))


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def audit(manifest: dict, root: Path, environment: dict[str, str],
          compiler: str | None = None) -> dict:
    if not isinstance(manifest, dict) or manifest.get("schema_version") != 1:
        raise ValueError("unsupported closure manifest")
    nodes = manifest.get("nodes")
    targets = manifest.get("targets")
    if not isinstance(nodes, dict) or not nodes or not isinstance(targets, list) or not targets:
        raise ValueError("empty build graph or target list")
    if any(not isinstance(k, str) or not k for k in nodes):
        raise ValueError("invalid node identifier")
    if any(not isinstance(k, str) for k in targets) or len(set(targets)) != len(targets):
        raise ValueError("invalid or duplicate targets")
    visited = set()
    active = set()
    dependencies = {}
    leaf_hashes = {}
    produced_outputs = {}
    declared_leaf_paths = {}
    portable_names = set()
    root = root.resolve()

    def require_file(label: str, description: dict) -> None:
        name = description.get("path")
        sha = description.get("sha256")
        if (not isinstance(name, str) or not name or
                not isinstance(sha, str) or not HEX.fullmatch(sha)):
            raise ValueError("invalid artifact description: " + label)
        if not safe_relative_path(name):
            raise ValueError("absolute/traversal input path: " + label)
        path = (root / name).resolve(strict=True)
        if not path.is_relative_to(root) or not path.is_file():
            raise ValueError("artifact escapes root or is not a file: " + label)
        actual = digest(path)
        if actual != sha:
            raise ValueError("artifact hash mismatch: " + label)
        if name.casefold() in portable_names:
            raise ValueError("duplicate tracked file/tool path: " + name)
        portable_names.add(name.casefold())
        declared_leaf_paths[name] = label
        leaf_hashes[label] = actual

    def visit(identifier: str) -> None:
        if identifier in active:
            raise ValueError("cyclic dependency: " + identifier)
        if identifier in visited:
            return
        node = nodes.get(identifier)
        if not isinstance(node, dict):
            raise ValueError("missing graph node: " + identifier)
        kind = node.get("kind")
        active.add(identifier)
        if kind in ("file", "tool"):
            if set(node) != {"kind", "path", "sha256"}:
                raise ValueError("unrecognized file/tool fields: " + identifier)
            require_file(identifier, node)
            children = []
        elif kind == "environment":
            if set(node) != {"kind", "name", "sha256"}:
                raise ValueError("unrecognized environment fields: " + identifier)
            name = node["name"]
            sha = node["sha256"]
            if not isinstance(name, str) or name not in environment or not isinstance(sha, str) or not HEX.fullmatch(sha):
                raise ValueError("unavailable environment input: " + identifier)
            actual = hashlib.sha256(environment[name].encode("utf-8")).hexdigest()
            if actual != sha:
                raise ValueError("environment fingerprint mismatch: " + identifier)
            leaf_hashes[identifier] = actual
            children = []
        elif kind == "action":
            if set(node) != {"kind", "inputs", "command", "outputs"}:
                raise ValueError("unrecognized action fields: " + identifier)
            children = node["inputs"]
            if (not isinstance(children, list) or not children or
                    any(not isinstance(x, str) for x in children) or
                    len(children) != len(set(children)) or
                    not isinstance(node["command"], list) or not node["command"] or
                    any(not isinstance(x, str) for x in node["command"]) or
                    not isinstance(node["outputs"], list) or not node["outputs"] or
                    any(not isinstance(x, str) or not x for x in node["outputs"])):
                raise ValueError("malformed build action: " + identifier)
            if not any(nodes.get(child, {}).get("kind") == "tool" for child in children if isinstance(nodes.get(child), dict)):
                raise ValueError("action missing executable dependency: " + identifier)
            # The executable invoked by an action must be one of its
            # explicitly tracked tool inputs, not an untracked binary.
            tool_paths = {nodes[child]["path"] for child in children
                          if isinstance(nodes.get(child), dict)
                          and nodes[child].get("kind") == "tool"}
            if node["command"][0] not in tool_paths:
                raise ValueError("action command executable not tracked: " + identifier)
            # Reject ambiguous build outputs and path traversal.
            if len(node["outputs"]) != len(set(node["outputs"])):
                raise ValueError("duplicate action outputs: " + identifier)
            for output in node["outputs"]:
                output_path = Path(output)
                if not safe_relative_path(output):
                    raise ValueError("unsafe action output: " + identifier)
                if not (root / output).resolve().is_relative_to(root):
                    raise ValueError("action output escapes root: " + output)
                if output.casefold() in (p.casefold() for p in produced_outputs):
                    raise ValueError("multiple producers for output: " + output)
                produced_outputs[output] = identifier
            for child in children:
                visit(child)
        else:
            raise ValueError("unsupported graph node: " + identifier)
        active.remove(identifier)
        visited.add(identifier)
        dependencies[identifier] = children

    for target in targets:
        visit(target)
    tracked_paths = {nodes[k]["path"] for k in leaf_hashes
                     if nodes[k]["kind"] in ("file", "tool")}
    collisions = {p for p in produced_outputs if p.casefold() in {t.casefold() for t in tracked_paths}}
    if collisions:
        raise ValueError("build action overwrites tracked input: " + repr(sorted(collisions)))
    if visited != set(nodes):
        raise ValueError("unreachable/unaccounted manifest nodes: " +
                         repr(sorted(set(nodes) - visited)))
    source_coverage = None
    if compiler is not None:
        if compiler not in ("jwc", "rust"):
            raise ValueError("unsupported compiler")
        required = source_identity(compiler)["files"]
        by_path = {nodes[k]["path"]: leaf_hashes[k] for k in leaf_hashes
                   if nodes[k]["kind"] == "file"}
        missing = sorted(name for name, sha in required.items()
                         if by_path.get(name) != sha)
        if missing:
            raise ValueError("missing compiler source leaves: " + repr(missing[:20]))
        source_coverage = {"compiler": compiler, "required": len(required),
                           "accounted_for": len(required)}
    return {
        "schema_version": 1,
        "kind": "declared-build-closure",
        "status": "DECLARED_CLOSURE_VERIFIED",
        "targets": targets,
        "visited_nodes": len(visited),
        "dependencies": dependencies,
        "leaf_hashes": leaf_hashes,
        "output_producers": produced_outputs,
        "source_coverage": source_coverage,
        "observed_inputs_complete": False,
        "external_toolchain_semantics_proven": False,
    }


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--manifest", type=Path, required=True)
    p.add_argument("--root", type=Path, required=True)
    p.add_argument("--compiler", choices=("jwc", "rust"), required=True)
    p.add_argument("--output", type=Path, required=True)
    args = p.parse_args(argv)
    import os
    try:
        report = audit(json.loads(args.manifest.read_text(encoding="utf-8")),
                       args.root, dict(os.environ), compiler=args.compiler)
    except (OSError, ValueError, TypeError, KeyError) as exc:
        report = {"schema_version": 1, "kind": "declared-build-closure",
                  "status": "BLOCKED", "reason": str(exc)}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(report["status"], "->", args.output)
    return 0 if report["status"] == "DECLARED_CLOSURE_VERIFIED" else 1


if __name__ == "__main__":
    sys.exit(main())
