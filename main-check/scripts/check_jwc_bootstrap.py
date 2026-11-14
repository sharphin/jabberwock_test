#!/usr/bin/env python3
"""Build stage2 and stage3 with jwc itself; check exits and stable LLVM output."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--jwc", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)
    a = p.parse_args()
    report = {"subject": "jwc", "results": [], "infrastructure_errors": [],
              "scope": "stage1 -> stage2 -> stage3 self-compilation and selected stable outputs; not trusted-bootstrap proof"}
    def save():
        a.output.parent.mkdir(parents=True, exist_ok=True)
        a.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    def run(command, timeout):
        p = subprocess.run(list(map(str, command)), cwd=ROOT, capture_output=True, text=True,
                           encoding="utf-8", errors="replace", timeout=timeout)
        return {"exit": p.returncode, "stdout": p.stdout, "stderr": p.stderr}
    def record(name, passed, observed):
        report["results"].append({"id": name, "compiler": "jwc", "items": [7, 12],
                                  "status": "pass" if passed else "fail", "observed": observed})
        save()
        print(name + ": " + ("pass" if passed else "fail"), flush=True)
    try:
        stage1 = a.jwc.resolve()
        report["stage1_sha256"] = hashlib.sha256(stage1.read_bytes()).hexdigest()
        with tempfile.TemporaryDirectory(prefix="jw-bootstrap-") as temporary:
            folder = Path(temporary)
            stages = [stage1]
            for number in (2, 3):
                dest = folder / ("jwc-stage" + str(number) + (".exe" if os.name == "nt" else ""))
                result = run([stages[-1], "build", "--clang", os.environ.get("JW_CLANG", "clang"),
                              "-o", dest, ROOT / "jwc/main.jw"], 1800)
                passed = result["exit"] == 0 and dest.is_file()
                record("build-stage" + str(number), passed, result)
                if not passed:
                    return 1
                report["stage" + str(number) + "_sha256"] = hashlib.sha256(dest.read_bytes()).hexdigest()
                stages.append(dest)
            for filename in ("examples/fib.jw", "examples/bst.jw", "jwc/main.jw"):
                outputs = [run([s, "--ll", ROOT / filename], 900) for s in stages]
                valid = all(o["exit"] == 0 and "define void @jw_main(" in o["stdout"] for o in outputs)
                valid = valid and outputs[0]["stdout"] == outputs[1]["stdout"] == outputs[2]["stdout"]
                record("stable-llvm-" + filename, valid,
                       [{"exit": o["exit"], "stderr": o["stderr"],
                         "LLVM_sha256": hashlib.sha256(o["stdout"].encode()).hexdigest()} for o in outputs])
            # Actually execute code emitted by both self-built compilers.
            source = folder / "contract.jw"
            source.write_text("fn main() { let a = own 7; print(*a); }", encoding="utf-8")
            for number, compiler in enumerate(stages[1:], 2):
                dest = folder / ("program" + str(number) + (".exe" if os.name == "nt" else ""))
                built = run([compiler, "build", "--clang", os.environ.get("JW_CLANG", "clang"), "-o", dest, source], 180)
                observed = run([dest], 30) if built["exit"] == 0 and dest.is_file() else built
                record("stage" + str(number) + "-generated-execution",
                       built["exit"] == 0 and observed["exit"] == 0 and observed["stdout"] == "7\n", observed)
    except (OSError, subprocess.TimeoutExpired) as error:
        report["infrastructure_errors"].append({"error": str(error)})
        return 2
    finally:
        save()
    return int(any(r["status"] != "pass" for r in report["results"]))


if __name__ == "__main__":
    sys.exit(main())
