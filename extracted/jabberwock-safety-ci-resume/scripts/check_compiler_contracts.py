#!/usr/bin/env python3
"""Execute one named compiler's contract cases; never substitute another compiler.

Exit 0 means these recorded cases passed, 1 mismatches, 2 infrastructure failure.
Each process runs in isolation, with a timeout and explicit exit/output assertions.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

from compiler_contracts import CASES, GOAL_CASES

ROOT = Path(__file__).resolve().parents[1]
EXE = ".exe" if os.name == "nt" else ""


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--subject", choices=("rust", "jwc"), required=True)
    parser.add_argument("--compiler", type=Path, required=True)
    parser.add_argument("--profile", required=True)
    parser.add_argument("--mode", choices=("contracts", "address", "thread", "memory", "undefined", "allocation"), default="contracts")
    parser.add_argument("--goal", choices=tuple(GOAL_CASES))
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    compiler = args.compiler.resolve()
    report = {"subject": args.subject, "compiler_profile": args.profile,
              "platform": sys.platform, "mode": args.mode, "goal": args.goal, "results": [], "infrastructure_errors": [],
              "scope": "Named cases and configurations only; whole-item coverage is assessed separately"}
    args.output.parent.mkdir(parents=True, exist_ok=True)

    def save():
        args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

    def run(command, extra=None, timeout=120):
        env = dict(os.environ)
        for name in ("JW_CFLAGS", "JW_NATIVE", "JW_LEAKS", "JW_RETIREMENTS", "JW_TEST_FAIL_ALLOCATION"):
            env.pop(name, None)
        env.update(extra or {})
        p = subprocess.run([str(x) for x in command], cwd=ROOT, env=env, capture_output=True,
                           text=True, encoding="utf-8", errors="replace", timeout=timeout)
        return {"exit": p.returncode, "stdout": p.stdout, "stderr": p.stderr}

    def record(case_id, items, configuration, status, observed, expected):
        report["results"].append({"id": case_id + ":" + configuration, "case": case_id,
                                  "compiler": args.subject, "compiler_profile": args.profile,
                                  "items": items, "configuration": configuration, "status": status,
                                  "observed": observed, "expected": expected})
        save()
        print(args.subject + " " + case_id + " " + configuration + ": " + status, flush=True)

    def generate(source, checked=False):
        options = (["--ll-checked"] if checked else ["--ll"]) if args.subject == "jwc" else \
                  (["--no-elide", "--ll"] if checked else ["--ll"])
        return run([compiler, *options, source])

    flags = [] if args.mode in ("contracts", "allocation") else ["-fsanitize=" + args.mode, "-fno-omit-frame-pointer"]
    if args.mode == "undefined":
        flags.append("-fno-sanitize-recover=undefined")
    clang = os.environ.get("JW_CLANG", "clang")
    link = ["-lshell32", "-Wl,/stack:268435456"] if os.name == "nt" else ["-lm", "-pthread"]

    def build(ll, folder, name, level, runtime=None):
        # Match the production native instrumentation path for generated LLVM.
        text = ll
        if args.mode in ("address", "thread", "memory"):
            text = text.replace(") {\nentry:", ") sanitize_" + args.mode + " {\nentry:")
            if args.mode == "memory":
                lines = []
                for line in text.splitlines():
                    if line.startswith("declare ") and "@llvm." not in line and "(" in line:
                        head, rest = line.split("(", 1)
                        params, tail = rest.split(")", 1)
                        params = ", ".join(p if not p or p == "..." or "noundef" in p else p + " noundef" for p in params.split(", "))
                        line = head + "(" + params + ")" + tail
                    lines.append(line)
                text = "\n".join(lines) + "\n"
        path = folder / (name + ".ll")
        path.write_text(text, encoding="utf-8")
        executable = folder / (name + EXE)
        command = [clang, "-" + level, path, *(runtime or [ROOT / "runtime/jw_rt.c"]),
                   *flags, *link, "-Wno-override-module", "-o", executable]
        if level == "O2":
            command += ["-flto", "-fuse-ld=lld"]
        result = run(command)
        if result["exit"]:
            report["infrastructure_errors"].append({"case": name, "phase": "link", "command": list(map(str, command)), "observed": result})
            save()
            return None
        return executable

    try:
        report["compiler_sha256"] = sha(compiler)
        report["runtime_sha256"] = sha(ROOT / "runtime/jw_rt.c")
        report["manifest_sha256"] = sha(Path(__file__).with_name("compiler_contracts.py"))
        with tempfile.TemporaryDirectory(prefix="jw-" + args.subject + "-contracts-") as temporary:
            folder = Path(temporary)
            if args.mode == "allocation":
                expected_allocations = []
                # Isolated runtime allocation injection, including on Windows.
                obj = folder / "fault-runtime.o"
                command = [clang, "-O1", "-c", ROOT / "runtime/jw_rt.c", "-o", obj,
                           "-Dmalloc=jw_test_malloc", "-Dcalloc=jw_test_calloc", "-Drealloc=jw_test_realloc"]
                compiled = run(command)
                if compiled["exit"]:
                    report["infrastructure_errors"].append({"phase": "allocation runtime build", "observed": compiled})
                else:
                    fixtures = [c for c in CASES if c["id"] in ("own-move-keeps-ref", "list-write-keeps-ref", "map-values", "closure-own", "question-mark-cleanup")]
                    for c in fixtures:
                        expected_allocations.append(c["id"] + ":allocation-baseline")
                        source = folder / (c["id"] + ".jw")
                        source.write_text(c["source"], encoding="utf-8")
                        generated = generate(source)
                        if generated["exit"] or "define void @jw_main(" not in generated["stdout"]:
                            record(c["id"], [8, 11], "generate", "fail", generated, "must generate LLVM")
                            continue
                        exe = build(generated["stdout"], folder, c["id"], "O1", [obj, ROOT / "tests/support/allocation_fault.c"])
                        if exe is None:
                            continue
                        baseline = run([exe], {"JW_LEAKS": "1"})
                        found = re.search(r"\[allocation-count\] (\d+)", baseline["stderr"])
                        count = int(found[1]) if found else 0
                        valid = baseline["exit"] == 0 and baseline["stdout"] == c["stdout"] and "[live] 0" in baseline["stderr"] and 0 < count <= 512
                        record(c["id"], [8, 11], "allocation-baseline", "pass" if valid else "fail", baseline,
                               {"stdout": c["stdout"], "live": 0, "allocation_count": "1..512"})
                        if not valid:
                            continue
                        for n in range(1, count + 1):
                            expected_allocations.append(c["id"] + ":allocation-" + str(n))
                            observed = run([exe], {"JW_TEST_FAIL_ALLOCATION": str(n)})
                            valid = observed["exit"] == 1 and ("[allocation-failure] " + str(n)) in observed["stderr"] and "error: メモリが足りません" in observed["stderr"]
                            record(c["id"], [8, 11], "allocation-" + str(n), "pass" if valid else "fail", observed,
                                   "injected allocation reached; exit 1 with specified memory error")
                report["expected_case_ids"] = expected_allocations
                report["missing_case_ids"] = sorted(set(expected_allocations) - {r["id"] for r in report["results"]})
            else:
                cases = GOAL_CASES[args.goal] if args.goal else [c for c in CASES if args.mode != "thread" or 3 in c["items"]]
                expected = []
                for c in cases:
                    source = folder / (c["id"] + ".jw")
                    source.write_text(c["source"], encoding="utf-8")
                    for checked in (False, True):
                        variant = "checked" if checked else "elided"
                        generated = generate(source, checked)
                        if c["reject"]:
                            configuration = variant + "-reject"
                            expected.append(c["id"] + ":" + configuration)
                            valid = generated["exit"] == 1 and c["reject"] in generated["stderr"] and not generated["stdout"].strip()
                            record(c["id"], c["items"], configuration, "pass" if valid else "fail", generated,
                                   {"exit": 1, "diagnostic_contains": c["reject"], "LLVM": "none"})
                            continue
                        for level in ("O1", "O2"):
                            configuration = variant + "-" + level
                            expected.append(c["id"] + ":" + configuration)
                            if generated["exit"] or "define void @jw_main(" not in generated["stdout"]:
                                record(c["id"], c["items"], configuration, "fail", generated, "must generate LLVM before execution")
                                continue
                            exe = build(generated["stdout"], folder, c["id"] + "-" + configuration, level)
                            if exe is None:
                                continue
                            observed = run([exe], {"JW_LEAKS": "1"})
                            valid = observed["exit"] == (1 if c["error"] else 0) and observed["stdout"] == c["stdout"]
                            valid = valid and (c["error"] in observed["stderr"] if c["error"] else observed["stderr"] == "[live] 0\n")
                            record(c["id"], c["items"], configuration, "pass" if valid else "fail", observed,
                                   {"exit": 1 if c["error"] else 0, "stdout": c["stdout"], "error": c["error"], "live_on_success": 0})
                report["expected_case_ids"] = expected
                actual = {r["id"] for r in report["results"]}
                report["missing_case_ids"] = sorted(set(expected) - actual)
    except (OSError, subprocess.TimeoutExpired) as error:
        report["infrastructure_errors"].append({"error": str(error)})
    save()
    if report["infrastructure_errors"] or not report["results"] or report.get("missing_case_ids"):
        return 2
    return int(any(r["status"] != "pass" for r in report["results"]))


if __name__ == "__main__":
    sys.exit(main())
