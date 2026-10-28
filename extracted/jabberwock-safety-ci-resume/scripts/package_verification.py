#!/usr/bin/env python3
"""Package local verification evidence for the execute -> review -> fix cycle.

This tool never changes test results or awards acceptance. Review the archive
before sharing it: build/test logs may contain local paths or sensitive data.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import sys
import zipfile

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_INPUT = ROOT / "target" / "verification-complete"
DEFAULT_OUTPUT = ROOT / "target" / "verification-evidence.zip"


def package(source: Path, destination: Path) -> dict:
    source = source.resolve()
    destination = destination.resolve()
    if not source.is_dir():
        raise ValueError("verification directory not found: " + str(source))
    summary_path = source / "summary.json"
    if not summary_path.is_file():
        raise ValueError("missing summary.json; execute py scripts/verify_all_local.py first")
    summary = json.loads(summary_path.read_text(encoding="utf-8"))
    if not isinstance(summary, dict) or summary.get("kind") != "local-full-verification":
        raise ValueError("invalid verification summary")
    if summary.get("schema_version") != 2:
        raise ValueError("unknown summary schema")
    files = sorted(p for p in source.rglob("*") if p.is_file())
    if not files:
        raise ValueError("empty evidence directory")
    if destination == source or source in destination.parents:
        raise ValueError("archive must be outside the evidence directory")
    inventory = {}
    for file in files:
        relative = file.relative_to(source).as_posix()
        if file.is_symlink() or not file.resolve().is_relative_to(source):
            raise ValueError("unsafe evidence path: " + relative)
        inventory[relative] = {
            "sha256": hashlib.sha256(file.read_bytes()).hexdigest(),
            "bytes": file.stat().st_size,
        }
    manifest = {
        "schema_version": 1,
        "kind": "local-verification-evidence-bundle",
        "verdict": summary.get("status", "UNKNOWN"),
        "files": inventory,
    }
    destination.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(destination, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for file in files:
            archive.write(file, arcname=file.relative_to(source).as_posix())
        archive.writestr("evidence-manifest.json", json.dumps(manifest, indent=2, ensure_ascii=False) + "\n")
    return {"archive": str(destination), "files": len(files), "verdict": manifest["verdict"]}


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, default=DEFAULT_INPUT)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    args = parser.parse_args(argv)
    try:
        result = package(args.input, args.output)
    except (OSError, ValueError, TypeError, json.JSONDecodeError) as error:
        print("EVIDENCE_PACKAGE_ERROR:", error, file=sys.stderr)
        return 2
    print(json.dumps(result, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
