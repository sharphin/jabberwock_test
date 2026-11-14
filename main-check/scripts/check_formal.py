#!/usr/bin/env python3
"""Build the bounded core and audit every declared theorem's kernel axioms."""

from pathlib import Path
import re
import shutil
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[1]
PROJECT = ROOT / "formal"
ALLOWED_AXIOMS = {"propext", "Quot.sound", "Classical.choice"}


def discover_theorems(project=PROJECT):
    """Return kernel-audit names from every root and compiler-specific module."""
    theorems = []
    for path in sorted((project / "JabberwockFormal").rglob("*.lean")):
        source = path.read_text(encoding="utf-8")
        if re.search(r"\b(sorry|admit|axiom|unsafe|native_decide)\b", source):
            raise SystemExit("unapproved proof escape in " + str(path))
        if "set_option warningAsError true" not in source:
            raise SystemExit("proof warnings must be errors: " + str(path))
        namespace = re.search(r"^namespace ([A-Za-z0-9_.]+)$", source, re.MULTILINE)
        if namespace is None:
            raise SystemExit("missing explicit namespace: " + str(path))
        theorems.extend(namespace[1] + "." + name for name in
                        re.findall(r"^theorem ([A-Za-z0-9_]+)\b", source, re.MULTILINE))
    if len(theorems) != len(set(theorems)):
        raise SystemExit("duplicate theorem declarations in proof audit")
    if not theorems:
        raise SystemExit("no theorems found")
    return theorems


def main():
    lake = shutil.which("lake")
    if lake is None:
        raise SystemExit("lake is required (see formal/lean-toolchain); no skip is allowed")
    version = subprocess.check_output([lake, "env", "lean", "--version"], cwd=PROJECT, text=True)
    if "version 4.24.0," not in version:
        raise SystemExit("expected Lean 4.24.0, got: " + version.strip())
    theorems = discover_theorems()
    subprocess.run([lake, "build"], cwd=PROJECT, check=True)
    with tempfile.TemporaryDirectory(prefix="jabberwock-formal-") as temporary:
        audit = Path(temporary) / "Audit.lean"
        audit.write_text("import JabberwockFormal\n" + "\n".join(
            "#print axioms " + name for name in theorems) + "\n", encoding="utf-8")
        report = subprocess.check_output([lake, "env", "lean", str(audit)],
                                         cwd=PROJECT, text=True, stderr=subprocess.STDOUT)
    for name in theorems:
        entry = re.search(re.escape("'" + name + "'") +
                          r" (?:does not depend on any axioms|depends on axioms:\s*\[([^\]]*)\])",
                          report)
        if entry is None:
            raise SystemExit("missing axiom audit: " + name + "\n" + report)
        dependencies = set(filter(None, (item.strip() for item in (entry[1] or "").split(","))))
        if dependencies - ALLOWED_AXIOMS:
            raise SystemExit("unapproved axioms for " + name + ": " + repr(dependencies))
    print(version.strip())
    print(str(len(theorems)) + " theorems checked; only approved standard Lean axioms; no skipped proofs")
    print("Scope: abstract generation/retirement and owner moves, not the full language or implementation")


if __name__ == "__main__":
    main()
