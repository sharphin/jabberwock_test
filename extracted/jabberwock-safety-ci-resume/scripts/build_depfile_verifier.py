#!/usr/bin/env python3
"""Verify actual depfile-derived build inputs against a hashed build-input manifest.

Compiler-produced .d files identify observed file dependencies. Their coverage
depends on the invoked compiler and flags; paths or file hashes alone do not
prove complete toolchain/build dependency closure.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import sys

from formal_report import source_identity
ROOT = Path(__file__).resolve().parents[1]


def parse_depfile(path: Path) -> set[str]:
    text = path.read_text(encoding="utf-8")
    # GNU Make continuation; do not silently interpret arbitrary Make syntax.
    text = text.replace("\\\n", " ")
    if ":" not in text:
        raise ValueError("missing Make depfile target")
    # Ignore target up to first colon followed by whitespace (Windows drive
    # letters may contain a colon that does not delimit a Make rule).
    match = re.search(r":\s", text)
    if not match:
        raise ValueError("missing Make dependency separator")
    content = text[match.end():]
    if re.search(r"(?<!\\)[#:$]", content):
        raise ValueError("unsupported depfile syntax")
    tokens = re.findall(r"(?:\\.|[^\s])+", content)
    paths = set()
    for token in tokens:
        value = re.sub(r"\\([ #\\])", r"\1", token)
        if not value:
            raise ValueError("empty depfile input")
        paths.add(value)
    if not paths:
        raise ValueError("empty depfile dependencies")
    return paths


def verify(manifest: dict, depdir: Path, repository: Path = ROOT, build_cwd: Path | None = None) -> dict:
    if not isinstance(manifest, dict) or manifest.get("schema_version") != 1:
        raise ValueError("invalid build manifest")
    compiler = manifest.get("compiler")
    files = manifest.get("files")
    depfiles = manifest.get("depfiles")
    if compiler not in ("jwc", "rust") or not isinstance(files, dict) or not files:
        raise ValueError("invalid compiler or empty build inputs")
    if not isinstance(depfiles, list) or not depfiles or any(
            not isinstance(x, str) or not x or Path(x).is_absolute() or
            ".." in Path(x).parts for x in depfiles):
        raise ValueError("missing/unsafe depfile list")
    if len(depfiles) != len(set(depfiles)):
        raise ValueError("duplicate depfile")
    required = source_identity(compiler)["files"]
    cwd = (build_cwd or depdir).resolve()
    if not cwd.is_dir():
        raise ValueError("build working directory missing")
    checked = {}
    for name, digest in files.items():
        if not isinstance(name, str) or not isinstance(digest, str) or not re.fullmatch(r"[0-9a-f]{64}", digest):
            raise ValueError("invalid input path or sha256")
        path = (repository / name).resolve()
        if not path.is_relative_to(repository.resolve()) or not path.is_file():
            raise ValueError("missing/outside build input: " + name)
        actual = hashlib.sha256(path.read_bytes()).hexdigest()
        if actual != digest:
            raise ValueError("stale build input: " + name)
        checked[str(path)] = name
    if any(name not in files or files[name] != digest for name, digest in required.items()):
        raise ValueError("build inputs omit production compiler sources")
    observed = set()
    for name in depfiles:
        dep = (depdir / name).resolve()
        if not dep.is_relative_to(depdir.resolve()) or not dep.is_file():
            raise ValueError("missing/outside depfile: " + name)
        for raw in parse_depfile(dep):
            dependency = (cwd / raw).resolve()
            if str(dependency) not in checked:
                raise ValueError("unhashed observed dependency: " + str(dependency))
            observed.add(checked[str(dependency)])
    return {"status": "DEPFILE_INPUTS_VERIFIED", "compiler": compiler,
            "checked_inputs": len(checked), "observed_inputs": sorted(observed),
            "complete_build_closure_proven": False,
            "limitations": ["Compiler depfiles do not capture all tools, environment, linker or dynamic inputs."]}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--depdir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--build-cwd", type=Path,
                        help="working directory used by compiler when depfiles were produced")
    args = parser.parse_args(argv)
    try:
        result = verify(json.loads(args.manifest.read_text(encoding="utf-8")),
                        args.depdir, build_cwd=args.build_cwd)
    except (OSError, ValueError, TypeError, KeyError) as exc:
        result = {"status": "BLOCKED", "reason": str(exc),
                  "complete_build_closure_proven": False}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(result["status"], "->", args.output)
    return 0 if result["status"] == "DEPFILE_INPUTS_VERIFIED" else 1


if __name__ == "__main__":
    sys.exit(main())
