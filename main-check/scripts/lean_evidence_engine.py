#!/usr/bin/env python3
"""Run Lean's kernel against declared proof obligations without treating them as compiler refinement."""
from __future__ import annotations
import argparse
import hashlib
import json
import re
import shutil
import subprocess
import tempfile
from pathlib import Path
from lean_refinement_contracts import check_contracts

AXIOMS = {"propext", "Quot.sound", "Classical.choice"}
NAME = re.compile(r"^[A-Za-z_][A-Za-z0-9_.]*$")
HASH = re.compile(r"^[0-9a-f]{64}$")

class EvidenceError(ValueError):
    pass

def local_file(root: Path, relative: str) -> Path:
    if not isinstance(relative, str) or not relative or relative.startswith("/") or "\\" in relative:
        raise EvidenceError("invalid relative path")
    parts = Path(relative).parts
    if ".." in parts or "." in parts:
        raise EvidenceError("noncanonical relative path")
    path = root.joinpath(*parts)
    if path.is_symlink() or any(p.is_symlink() for p in path.parents if p != root.parent):
        raise EvidenceError("symlink in evidence path")
    if not path.is_file() or not path.resolve().is_relative_to(root.resolve()):
        raise EvidenceError("missing or escaped evidence path")
    return path

def verify_manifest(root: Path, manifest: dict) -> tuple[Path, list[str]]:
    if not isinstance(manifest, dict) or manifest.get("schema_version") != 1:
        raise EvidenceError("unsupported manifest")
    if manifest.get("lean_version") != "4.24.0":
        raise EvidenceError("Lean version must be pinned to 4.24.0")
    project = manifest.get("project")
    if project != "formal":
        raise EvidenceError("only formal project supported")
    names = manifest.get("theorems")
    if not isinstance(names, list) or not names or len(names) != len(set(names)) or not all(isinstance(n, str) and NAME.fullmatch(n) for n in names):
        raise EvidenceError("invalid theorem list")
    for field in ("proof_files", "source_files"):
        entries = manifest.get(field)
        if not isinstance(entries, list) or not entries:
            raise EvidenceError("missing " + field)
        seen = set()
        for entry in entries:
            if not isinstance(entry, dict) or set(entry) != {"path", "sha256"}:
                raise EvidenceError("malformed file entry")
            path = entry["path"]
            digest = entry["sha256"]
            if not isinstance(path, str) or path in seen or not isinstance(digest, str) or not HASH.fullmatch(digest):
                raise EvidenceError("invalid path or digest")
            seen.add(path)
            actual = hashlib.sha256(local_file(root, path).read_bytes()).hexdigest()
            if actual != digest:
                raise EvidenceError("source/proof content differs from manifest: " + path)
            if field == "proof_files" and not path.endswith(".lean"):
                raise EvidenceError("proof must be Lean source")
    project_dir = root / "formal"
    proof_entries = {item["path"]: item["sha256"] for item in manifest["proof_files"]}
    actual_proofs = {}
    for proof in project_dir.rglob("*.lean"):
        if proof.is_symlink():
            raise EvidenceError("symlinked Lean proof source")
        relative = proof.relative_to(root).as_posix()
        actual_proofs[relative] = hashlib.sha256(proof.read_bytes()).hexdigest()
    if not actual_proofs or actual_proofs != proof_entries:
        raise EvidenceError("incomplete Lean source closure")
    source_entries = {item["path"]: item["sha256"] for item in manifest["source_files"]}
    for config in ("formal/lean-toolchain", "formal/lakefile.toml", "formal/lake-manifest.json"):
        if source_entries.get(config) != hashlib.sha256(local_file(root, config).read_bytes()).hexdigest():
            raise EvidenceError("missing or stale Lean config: " + config)
    formal = root / "formal"
    if formal.is_symlink() or not formal.is_dir():
        raise EvidenceError("invalid formal directory")
    return formal, names

def run_checked(argv: list[str], cwd: Path, timeout: int) -> str:
    proc = subprocess.run(argv, cwd=cwd, capture_output=True, text=True,
                          encoding="utf-8", errors="replace", timeout=timeout, check=False)
    if proc.returncode:
        raise EvidenceError("Lean command failed: " + (proc.stderr or proc.stdout)[-1000:])
    return proc.stdout

def check(root: Path, manifest: dict, timeout: int = 120) -> dict:
    project, names = verify_manifest(root.resolve(), manifest)
    lake = shutil.which("lake")
    if not lake:
        raise EvidenceError("lake is not installed")
    version = run_checked([lake, "env", "lean", "--version"], project, timeout)
    if "version 4.24.0," not in version:
        raise EvidenceError("unexpected Lean version: " + version.strip())
    run_checked([lake, "build"], project, timeout)
    checked_types = check_contracts(root.resolve(), lake, names, timeout)
    with tempfile.TemporaryDirectory(prefix="jw-kernel-") as directory:
        path = Path(directory) / "KernelAudit.lean"
        path.write_text("import JabberwockFormal\n" + "\n".join("#print axioms " + name for name in names) + "\n", encoding="utf-8")
        output = run_checked([lake, "env", "lean", str(path)], project, timeout)
    for name in names:
        pattern = re.escape("'" + name + "'") + r" (?:does not depend on any axioms|depends on axioms:\s*\[([^\]]*)\])"
        found = re.search(pattern, output)
        if not found:
            raise EvidenceError("no Lean kernel axiom result for " + name)
        observed = {a.strip() for a in (found.group(1) or "").split(",") if a.strip()}
        if observed - AXIOMS:
            raise EvidenceError("unapproved axioms in " + name)
    verify_manifest(root.resolve(), manifest)
    return {"schema_version": 1, "status": "KERNEL_CHECKED", "theorems": names,
            "contract_types_checked": checked_types,
            "scope": "declared Lean theorems, plus checked-in expected types when present",
            "compiler_refinement": "NOT_PROVEN"}

def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
        result = check(args.root, manifest)
    except (OSError, ValueError, subprocess.TimeoutExpired) as error:
        result = {"schema_version": 1, "status": "INVALID_EVIDENCE", "reason": str(error),
                  "compiler_refinement": "NOT_PROVEN"}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(result["status"], args.output)
    return 0 if result["status"] == "KERNEL_CHECKED" else 2

if __name__ == "__main__":
    raise SystemExit(main())
