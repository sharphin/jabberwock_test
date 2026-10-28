#!/usr/bin/env python3
"""Reproduce existing safety items 8/9; failures stay failures (exit 1), never expected-pass tests."""
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
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", type=Path, default=ROOT / "target/debug" /
                        ("jabberwock.exe" if sys.platform == "win32" else "jabberwock"))
    parser.add_argument("--jwc", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--items", choices=["8", "9", "8,9"], default="8,9")
    parser.add_argument("--subject", choices=["all", "rust", "jwc"], default="all")
    args = parser.parse_args()
    selected = {int(n) for n in args.items.split(",")}
    if sys.platform != "win32" and not sys.platform.startswith("linux"):
        parser.error("This runner requires Windows or Linux")
    if 8 in selected:
        if not sys.platform.startswith("linux"):
            parser.error("Item 8 requires Linux RLIMIT_AS; Windows item 8 is not implemented")
        import resource
    if os.environ.get("JW_CFLAGS", "").strip():
        parser.error("Run without JW_CFLAGS: these probes are uninstrumented")
    compiler = args.compiler.resolve()
    engines = [("rust", compiler)] if args.subject != "jwc" else []
    if args.subject == "jwc" and not args.jwc:
        parser.error("--subject jwc requires --jwc")
    if args.jwc and args.subject != "rust":
        engines.append(("jwc", args.jwc.resolve()))
    for name, path in engines:
        if not path.is_file():
            parser.error(name + " compiler is missing (not a pass): " + str(path))
    results = []
    infrastructure_errors = []

    def run(command, limited=False):
        def bound():
            # CLI reserves a 1 GiB stack. 4 GiB permits startup while the 256 TiB request fails safely.
            resource.setrlimit(resource.RLIMIT_AS, (1 << 32, 1 << 32))
        proc = subprocess.run([str(p) for p in command], cwd=ROOT, text=True, encoding="utf-8", errors="replace",
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=120,
                              preexec_fn=bound if limited else None)
        return {"exit": proc.returncode, "stdout": proc.stdout, "stderr": proc.stderr}

    if 9 in selected:
        oversized = ROOT / "tests/safety/generic-array-limit.jw"
        boundary = ROOT / "tests/safety/generic-array-limit-ok.jw"
        for name, engine in engines:
            for mode in ["--typed", "--ir-raw", "--ir", "--ll"]:
                record = run([engine, mode, oversized])
                # Dump APIs intentionally print diagnostics to stdout and return 0.
                # Require the rejection diagnostic and absence of emitted program text.
                dump_mode = mode in ("--typed", "--ir-raw") or (name == "jwc" and mode == "--ir")
                diagnostic = record["stdout"] if dump_mode else record["stderr"]
                passed = record["exit"] == (0 if dump_mode else 1) and "配列が大きすぎます" in diagnostic
                passed = passed and not any(marker in record["stdout"] for marker in ("define void @jw_main(", "bb0:", "structty Buffer"))
                results.append({"compiler": name, "item": 9, "case": name + mode, "case_status": "pass" if passed else "fail",
                                "expected": "reject expanded array above 1 MiB", "observed": record})
                record = run([engine, mode, boundary])
                passed = record["exit"] == 0 and bool(record["stdout"].strip())
                results.append({"compiler": name, "item": 9, "case": name + mode + "-within-limit", "case_status": "pass" if passed else "fail",
                                "expected": "accept expanded array exactly at 1 MiB", "observed": record})
    if 8 in selected:
        source = ROOT / "tests/safety/resource-exhaustion.jw"
        with tempfile.TemporaryDirectory(prefix="jw-resource-edges-") as temporary:
            directory = Path(temporary)
            executable = directory / "native"
            commands = []
            if args.subject != "jwc":
                built = run([compiler, "build", "-o", executable, source])
                commands.append(("interp", [compiler, source]))
                if built["exit"] == 0:
                    commands.append(("rust-native", [executable]))
                else:
                    infrastructure_errors.append({"case": "rust-native build", "observed": built})
            if args.jwc and args.subject != "rust":
                generated = run([args.jwc.resolve(), "--ll", source])
                if generated["exit"] == 0:
                    ll = directory / "jwc.ll"
                    ll.write_text(generated["stdout"], encoding="utf-8")
                    jwc_executable = directory / "jwc-native"
                    built = run([os.environ.get("JW_CLANG", "clang"), "-O1", ll, ROOT / "runtime/jw_rt.c",
                                 "-lm", "-pthread", "-Wno-override-module", "-o", jwc_executable])
                    if built["exit"] == 0:
                        commands.append(("jwc-native", [jwc_executable]))
                    else:
                        infrastructure_errors.append({"case": "jwc-native build", "observed": built})
                else:
                    infrastructure_errors.append({"case": "jwc generation", "observed": generated})
            for name, command in commands:
                record = run(command, limited=True)
                passed = record["exit"] == 1 and record["stderr"].startswith("error: メモリが足りません")
                results.append({"compiler": "jwc" if name.startswith("jwc") else "rust", "item": 8, "case": name + "-allocation-failure", "case_status": "pass" if passed else "fail",
                                "expected": "specified memory error, not an internal error or panic", "observed": record})
    report = {
        "scope": "Reproduction cases for existing items 8/9; passing cases alone do not complete either whole item",
        "platform": sys.platform, "subject": args.subject, "selected_items": sorted(selected),
        "address_space_limit_bytes": (1 << 32) if 8 in selected else None,
        "runtime_sha256": hashlib.sha256((ROOT / "runtime/jw_rt.c").read_bytes()).hexdigest(),
        "compiler_sha256": {name: hashlib.sha256(path.read_bytes()).hexdigest() for name, path in engines},
        "results": results, "infrastructure_errors": infrastructure_errors,
    }
    text = json.dumps(report, ensure_ascii=False, indent=2) + "\n"
    if args.output:
        args.output.write_text(text, encoding="utf-8")
    print(text, end="")
    return 2 if infrastructure_errors else int(any(r["case_status"] != "pass" for r in results))


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, subprocess.TimeoutExpired) as error:
        print("verification infrastructure failure (not a pass): " + str(error), file=sys.stderr)
        sys.exit(2)
