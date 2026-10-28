#!/usr/bin/env python3
"""Audit declared build inputs against the compiler source snapshot.

This is a conservative source dependency coverage gate, not a proof of semantic
correctness or of external toolchain closure. Unknown dynamic imports fail closed.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import sys
try:
    import tomllib
except ModuleNotFoundError:  # Python 3.10, still supported by Windows scripts
    tomllib = None

from formal_report import source_identity

ROOT = Path(__file__).resolve().parents[1]
JW_IMPORT = re.compile(r"(?m)^\s*use\s+([A-Za-z_][A-Za-z_0-9]*)\s*;")
RS_MOD = re.compile(r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z_0-9]*)\s*;")
RS_INCLUDE = re.compile(r'\binclude(?:_str|_bytes)?!\s*\(\s*"([^"]+)"\s*\)')


def rel(path: Path) -> str:
    resolved = path.resolve(strict=True)
    if not resolved.is_relative_to(ROOT.resolve()):
        raise ValueError("dependency escapes repository: " + str(path))
    if not resolved.is_file():
        raise ValueError("not a source file: " + str(path))
    return resolved.relative_to(ROOT.resolve()).as_posix()


def check(compiler: str) -> dict:
    snapshot = source_identity(compiler)["files"]
    errors = []
    edges = []
    if compiler == "jwc":
        start = ROOT / "jwc/main.jw"
        module_dir = ROOT / "jwc"
        seen = set()
        def visit(path):
            try:
                key = rel(path)
                if key in seen:
                    return
                seen.add(key)
                if key not in snapshot:
                    errors.append("not in source snapshot: " + key)
                for name in JW_IMPORT.findall(path.read_text(encoding="utf-8")):
                    child = module_dir / (name + ".jw")
                    edges.append([key, child.relative_to(ROOT).as_posix()])
                    if not child.is_file():
                        errors.append("unresolved import: " + key + " -> " + name)
                    else:
                        visit(child)
            except (OSError, ValueError) as exc:
                errors.append(str(exc))
        visit(start)
        # Every known file is conservatively in scope until the linker
        # and dynamic module loading are independently characterized.
        for name in snapshot:
            if name.startswith("jwc/") and name.endswith(".jw") and name not in seen:
                errors.append("unreachable compiler module (review required): " + name)
    elif compiler == "rust":
        for root_file in ("src/lib.rs", "src/main.rs", "src/alice.rs"):
            if root_file not in snapshot:
                errors.append("missing rust target entry: " + root_file)
        for name in snapshot:
            if not name.startswith("src/") or not name.endswith(".rs"):
                continue
            file = ROOT / name
            source = file.read_text(encoding="utf-8")
            for module in RS_MOD.findall(source):
                base = file.parent
                options = [base / (module + ".rs"), base / module / "mod.rs"]
                matches = [path for path in options if path.is_file()]
                if len(matches) != 1:
                    errors.append("ambiguous or missing Rust module: " + name + " -> " + module)
                else:
                    edges.append([name, rel(matches[0])])
            for included in RS_INCLUDE.findall(source):
                included_path = file.parent / included
                try:
                    edges.append([name, rel(included_path)])
                except (OSError, ValueError) as exc:
                    errors.append(str(exc))
            if re.search(r'\b(?:env|option_env|concat)\s*!\s*\(', source):
                errors.append("build-environment-dependent macro requires audit: " + name)
        cargo = ROOT / "Cargo.toml"
        if not cargo.is_file():
            errors.append("missing Cargo.toml")
        else:
            manifest_text = cargo.read_text(encoding="utf-8")
            if tomllib is None:
                # On 3.10 conservatively reject dependency-related Cargo sections.
                sections = re.findall(r"(?m)^\\s*\\[\\s*([^]]+)\\s*\\]", manifest_text)
                if any("dependencies" in s or s.startswith("target.") for s in sections):
                    errors.append("Cargo dependencies require resolved build graph attestation")
                if re.search(r"(?m)^\\s*build\\s*=", manifest_text):
                    errors.append("Cargo build script requires build execution dependency capture")
            else:
                manifest = tomllib.loads(manifest_text)
                if manifest.get("build") or manifest.get("package", {}).get("build"):
                    errors.append("Cargo build script requires build execution dependency capture")
                if manifest.get("dependencies") or manifest.get("build-dependencies") or manifest.get("target"):
                    errors.append("external Cargo dependencies require resolved build graph attestation")
    else:
        raise ValueError("unsupported compiler")
    for path, expected in snapshot.items():
        actual = ROOT / path
        if not actual.is_file() or hashlib.sha256(actual.read_bytes()).hexdigest() != expected:
            errors.append("source changed or missing: " + path)
    return {"compiler": compiler, "status": "DECLARED_CLOSURE_CHECKED" if not errors else "BLOCKED",
            "source_snapshot_sha256": source_identity(compiler)["sha256"],
            "edges": edges, "errors": errors,
            "external_toolchain_semantics_proven": False,
            "build_dependency_closure_proven": False}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "target/build-dependency-audit.json")
    args = parser.parse_args(argv)
    results = []
    for compiler in ("jwc", "rust"):
        try:
            results.append(check(compiler))
        except (OSError, ValueError, TypeError) as exc:
            results.append({"compiler": compiler, "status": "BLOCKED", "errors": [str(exc)]})
    report = {"schema_version": 1, "kind": "build-dependency-audit",
              "status": "DECLARED_CLOSURE_CHECKED" if all(x["status"] == "DECLARED_CLOSURE_CHECKED" for x in results) else "BLOCKED",
              "checks": results,
              "limitations": ["Static module resolution is not full executed build provenance.",
                              "Build tools, dynamic inputs, linker and external dependencies require independent evidence."]}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(report["status"], "->", args.output, flush=True)
    return 0 if report["status"] == "DECLARED_CLOSURE_CHECKED" else 1


if __name__ == "__main__":
    sys.exit(main())
