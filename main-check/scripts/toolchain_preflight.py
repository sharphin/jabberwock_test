#!/usr/bin/env python3
"""Read-only toolchain preflight: never installs software or certifies proof."""
import json
import platform
from pathlib import Path
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
REQUIRED = {"git": ["git", "--version"], "lake": ["lake", "--version"],
            "lean": ["lake", "env", "lean", "--version"],
            "cargo": ["cargo", "--version"], "rustc": ["rustc", "--version"],
            "clang": ["clang", "--version"], "opt": ["opt", "--version"],
            "go": ["go", "version"]}

def check(name, args):
    result = {"tool": name, "path": shutil.which(args[0]), "status": "MISSING"}
    if result["path"] is None:
        return result
    try:
        run = subprocess.run(args, cwd=ROOT / "formal" if name == "lean" else ROOT,
                             capture_output=True, text=True, timeout=30)
        version = (run.stdout + run.stderr).strip()
        result.update(version=version[:1500], exit_code=run.returncode,
                      status="OK" if run.returncode == 0 else "FAILED")
        if name == "lean" and result["status"] == "OK" and "version 4.24.0," not in version:
            result["status"] = "WRONG_VERSION"
    except (OSError, subprocess.TimeoutExpired) as exc:
        result.update(status="FAILED", reason=str(exc))
    return result

def main():
    checks = [check(name, command) for name, command in REQUIRED.items()]
    status = "READY" if all(x["status"] == "OK" for x in checks) else "BLOCKED"
    report = {"schema_version": 1, "kind": "toolchain-preflight",
              "status": status, "host_os": platform.system(), "checks": checks,
              "limitations": ["Tool presence is not proof of correctness.",
                              "Windows sanitizer and external detector compatibility are separate."]}
    output = ROOT / "target" / "toolchain-preflight.json"
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(status, "->", output)
    return 0 if status == "READY" else 2

if __name__ == "__main__":
    sys.exit(main())
