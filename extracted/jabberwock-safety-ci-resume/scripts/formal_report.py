#!/usr/bin/env python3
"""Audit five proof obligations independently for each selected compiler."""
import argparse
import datetime
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
PROJECT = ROOT / "formal"
GOALS = ("semantics", "memory", "types", "ownership", "checks")
ALLOWED_AXIOMS = {"propext", "Quot.sound", "Classical.choice"}
IDENTIFIER = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)*$")


def source_identity(compiler):
    roots = ("jwc", "runtime") if compiler == "jwc" else ("src", "runtime")
    paths = sorted(p for root in roots for p in (ROOT / root).rglob("*") if p.is_file())
    paths += [ROOT / "Cargo.toml", ROOT / "Cargo.lock"] if compiler == "rust" else []
    files = {p.relative_to(ROOT).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest() for p in paths}
    digest = hashlib.sha256(json.dumps(files, sort_keys=True).encode()).hexdigest()
    return {"sha256": digest, "files": files}


def proof_sources_identity():
    """Hash every Lean source and proof manifest, including transitive imports."""
    files = sorted(p for p in PROJECT.rglob("*")
                   if p.is_file() and ".lake" not in p.parts
                   and (p.suffix in (".lean", ".json", ".toml")
                        or p.name in ("lean-toolchain", "lakefile.lean")))
    manifest = {p.relative_to(ROOT).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest()
                for p in files}
    return {"sha256": hashlib.sha256(json.dumps(manifest, sort_keys=True).encode()).hexdigest(),
            "files": manifest}


def audit_module(module, theorems):
    if not IDENTIFIER.fullmatch(module) or not theorems or any(not IDENTIFIER.fullmatch(t) for t in theorems):
        raise ValueError("invalid module or empty/invalid theorem list")
    source = PROJECT / (module.replace(".", "/") + ".lean")
    if not source.is_file():
        raise ValueError("proof module missing: " + module)
    version = subprocess.run(["lake", "env", "lean", "--version"], cwd=PROJECT, capture_output=True,
                             text=True, encoding="utf-8", errors="replace", timeout=60)
    if version.returncode or "version 4.24.0," not in version.stdout:
        raise ValueError("Lean 4.24.0 required: " + version.stdout + version.stderr)
    # Kernel audit is the final check: source scanning alone cannot prove absence of sorryAx.
    built = subprocess.run(["lake", "build", module], cwd=PROJECT, capture_output=True,
                           text=True, encoding="utf-8", errors="replace", timeout=1800)
    if built.returncode:
        return {"status": "FAIL", "stdout": built.stdout, "stderr": built.stderr}
    with tempfile.TemporaryDirectory(prefix="jw-proof-audit-") as d:
        path = Path(d) / "Audit.lean"
        path.write_text("import " + module + "\n" + "\n".join("#print axioms " + t for t in theorems) + "\n", encoding="utf-8")
        run = subprocess.run(["lake", "env", "lean", str(path)], cwd=PROJECT, capture_output=True,
                             text=True, encoding="utf-8", errors="replace", timeout=300)
    if run.returncode:
        return {"status": "FAIL", "stdout": run.stdout, "stderr": run.stderr}
    for theorem in theorems:
        match = re.search(re.escape("'" + theorem + "'") + r" (?:does not depend on any axioms|depends on axioms:\s*\[([^\]]*)\])", run.stdout)
        if match is None or set(filter(None, (x.strip() for x in (match[1] or "").split(",")))) - ALLOWED_AXIOMS:
            return {"status": "FAIL", "reason": "missing or unapproved kernel axiom audit: " + theorem, "stdout": run.stdout}
    return {"status": "KERNEL_CHECKED", "module_sha256": hashlib.sha256(source.read_bytes()).hexdigest(), "stdout": run.stdout, "stderr": run.stderr}



def validate_manifest(manifest, compiler):
    """Reject missing/foreign proof obligations before invoking Lean.

    This verifies bookkeeping, not mathematical adequacy or source equivalence.
    """
    if not isinstance(manifest, dict) or manifest.get("compiler") != compiler:
        raise ValueError("compiler mismatch in proof manifest")
    goals = manifest.get("goals")
    if not isinstance(goals, dict) or set(goals) != set(GOALS):
        raise ValueError("missing or unexpected proof goals")
    prefix = "JabberwockFormal." + ("Jwc" if compiler == "jwc" else "Rust") + "."
    for goal in GOALS:
        entry = goals[goal]
        if not isinstance(entry, dict):
            raise ValueError("invalid proof obligation: " + goal)
        for field in ("statement", "proof_scope", "implementation_correspondence",
                      "implementation_sha256"):
            if not isinstance(entry.get(field), str) or not entry[field].strip():
                raise ValueError("missing proof metadata " + goal + "." + field)
        excluded = entry.get("excluded")
        if not isinstance(excluded, list) or any(not isinstance(x, str) or not x.strip() for x in excluded):
            raise ValueError("invalid exclusions: " + goal)
        module = entry.get("module")
        theorems = entry.get("theorems")
        if not isinstance(module, str) or not IDENTIFIER.fullmatch(module) or not module.startswith(prefix):
            raise ValueError("invalid or foreign proof module: " + goal)
        if not isinstance(theorems, list) or not theorems or any(not isinstance(t, str) for t in theorems):
            raise ValueError("missing or malformed theorem: " + goal)
        if len(theorems) != len(set(theorems)):
            raise ValueError("duplicate theorem: " + goal)
        for theorem in theorems:
            if not IDENTIFIER.fullmatch(theorem) or not theorem.startswith(module + "."):
                raise ValueError("foreign or malformed theorem: " + goal)
    return goals


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", choices=("all", "jwc", "rust"), default="all")
    parser.add_argument("--output", type=Path, default=ROOT / "target/formal-report.json")
    parser.add_argument("--require-full", action="store_true", help="nonzero unless whole-compiler goals are proved")
    parser.add_argument("--abstract-model", action="store_true", help="also audit the shared abstract model; never substitutes compiler proofs")
    args = parser.parse_args()
    selected = ("jwc", "rust") if args.compiler == "all" else (args.compiler,)
    report = {"schema_version": 3, "kind": "formal-proof", "started_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "compilers": {}, "goals": [], "abstract_model": {"status": "NOT_RUN"},
              "proof_sources_snapshot": proof_sources_identity()}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    def save():
        args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    audit_cache = {}
    try:
        for compiler in selected:
            identity = source_identity(compiler)
            manifest_path = PROJECT / "compilers" / compiler / "obligations.json"
            manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
            validate_manifest(manifest, compiler)
            report["compilers"][compiler] = {"source": identity, "manifest_sha256": hashlib.sha256(manifest_path.read_bytes()).hexdigest()}
            for goal in GOALS:
                obligation = manifest["goals"][goal]
                record = {"id": compiler + "-" + goal, "compiler": compiler, "goal": goal,
                          "statement": obligation["statement"], "proof_scope": obligation.get("proof_scope"), "excluded": obligation.get("excluded", []), "implementation_correspondence": obligation.get("implementation_correspondence"), "module": obligation.get("module"), "theorems": obligation.get("theorems", []), "source_sha256": identity["sha256"],
                          "status": "NOT_PROVEN", "acceptance": "NOT_PROVEN", "proof_check": {"status": "NOT_IMPLEMENTED"}}
                report["goals"].append(record)
                save()
                if obligation.get("module"):
                    # A Rust module or shared abstract theorem cannot silently fill a jwc obligation.
                    prefix = "JabberwockFormal." + ("Jwc" if compiler == "jwc" else "Rust") + "."
                    if not obligation["module"].startswith(prefix) or any(not t.startswith(prefix) for t in obligation.get("theorems", [])):
                        raise ValueError("proof module belongs to another compiler")
                    module = obligation["module"]
                    all_theorems = sorted({t for o in manifest["goals"].values() if o.get("module") == module for t in o.get("theorems", [])})
                    if any(not t.startswith(prefix) for t in all_theorems):
                        raise ValueError("theorem belongs to another compiler")
                    key = (compiler, module, tuple(all_theorems))
                    if key not in audit_cache:
                        print("VERIFY " + compiler + " " + module, flush=True)
                        audit_cache[key] = audit_module(module, all_theorems)
                    record["proof_check"] = dict(audit_cache[key])
                    # A stale source binding must not suppress the independent
                    # Lean kernel audit, nor become a passing compiler proof.
                    if obligation.get("implementation_sha256") != identity["sha256"]:
                        record["proof_check"]["source_binding"] = "STALE"
                        record["proof_check"]["reason"] = "proof binding differs from current compiler sources"
                    else:
                        record["proof_check"]["source_binding"] = "MATCH"
                    record["reason"] = "implementation correspondence and scope require review; kernel success alone is not whole-compiler proof"
                print(record["id"] + ": " + record["proof_check"]["status"] + " / NOT_PROVEN", flush=True)
                save()
        if args.abstract_model:
            run = subprocess.run([sys.executable, str(ROOT / "scripts/check_formal.py")], cwd=ROOT,
                                 capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=1800)
            report["abstract_model"] = {"status": "MODEL_PASS" if run.returncode == 0 else "FAIL", "stdout": run.stdout, "stderr": run.stderr}
    except (OSError, ValueError, KeyError, subprocess.TimeoutExpired) as error:
        report["infrastructure_error"] = str(error)
    except KeyboardInterrupt:
        report["interrupted"] = True
    if not report.get("infrastructure_error") and not report.get("interrupted"):
        try:
            if proof_sources_identity() != report["proof_sources_snapshot"]:
                report["infrastructure_error"] = "Lean sources/manifests changed during proof audit"
            for compiler in selected:
                if source_identity(compiler) != report["compilers"][compiler]["source"]:
                    report["infrastructure_error"] = "source changed during Lean audit: " + compiler
                    break
        except (OSError, KeyError, ValueError) as error:
            report["infrastructure_error"] = "cannot recheck proof source snapshot: " + str(error)
    report["summary"] = {
        "expected_goal_count": len(selected) * len(GOALS),
        "recorded_goal_count": len(report["goals"]),
        "kernel_checked_goal_count": sum(g["proof_check"]["status"] == "KERNEL_CHECKED" for g in report["goals"]),
        "whole_compiler_proven_goal_count": sum(g["acceptance"] == "PROVEN" for g in report["goals"]),
        "all_compilers_proven": False,
        "interpretation": "Local model theorem checks only; implementation correspondence not proved"
    }
    save()
    print("Formal report: " + str(args.output))
    if report.get("interrupted"):
        return 130
    if report.get("infrastructure_error"):
        return 2
    if args.require_full:
        return 1  # Whole-compiler correspondence remains unproved.
    return int(len(report["goals"]) != len(selected) * len(GOALS) or any(g["proof_check"]["status"] != "KERNEL_CHECKED" or g["proof_check"].get("source_binding") != "MATCH" for g in report["goals"])
               or (args.abstract_model and report["abstract_model"]["status"] != "MODEL_PASS"))

if __name__ == "__main__":
    sys.exit(main())
