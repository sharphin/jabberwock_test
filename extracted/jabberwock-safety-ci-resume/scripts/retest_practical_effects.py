#!/usr/bin/env python3
"""Check exactly-once external effects across compiler profiles and native backends.

Exit 0: all cases pass; 1: behavior mismatch; 2: unavailable verification infrastructure.
This checks part of practical items 10/11/12, not their entire acceptance criteria.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
EXE = ".exe" if sys.platform == "win32" else ""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--debug-compiler", type=Path, default=ROOT / "target/debug" / ("jabberwock" + EXE))
    parser.add_argument("--release-compiler", type=Path, default=ROOT / "target/release" / ("jabberwock" + EXE))
    parser.add_argument("--jwc", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--subject", choices=["all", "rust", "jwc"], default="all")
    args = parser.parse_args()
    if sys.platform != "win32" and not sys.platform.startswith("linux"):
        parser.error("This runner requires Windows or Linux")
    if os.environ.get("JW_CFLAGS", "").strip():
        parser.error("This runner is uninstrumented; run without JW_CFLAGS")
    engines = {"debug": args.debug_compiler.resolve(), "release": args.release_compiler.resolve(),
               "jwc": args.jwc.resolve()}
    engines = {name: path for name, path in engines.items()
               if args.subject == "all" or (name == "jwc") == (args.subject == "jwc")}
    for name, path in engines.items():
        if not path.is_file():
            parser.error(name + " compiler is missing: " + str(path))
    env = os.environ.copy()
    # Measure ordinary CLI execution, without opting into extra native comparisons.
    env.pop("JW_NATIVE", None)
    env.pop("JW_LEAKS", None)
    env.pop("JW_RETIREMENTS", None)
    results = []
    infrastructure = []

    def run(command, native=False):
        current_env = dict(env)
        if native:
            current_env["JW_LEAKS"] = "1"
        proc = subprocess.run([str(x) for x in command], cwd=ROOT, env=current_env,
                              text=True, encoding="utf-8", errors="replace", capture_output=True, timeout=120)
        return {"exit": proc.returncode, "stdout": proc.stdout, "stderr": proc.stderr}

    fixtures = [("effect-once.jw", 0), ("effect-before-error.jw", 1)]
    with tempfile.TemporaryDirectory(prefix="jw-practical-effects-") as temporary:
        folder = Path(temporary)
        for filename, exit_code in fixtures:
            source = ROOT / "tests/safety" / filename
            commands = []
            for name in [name for name in ("debug", "release") if name in engines]:
                for elide in [False, True]:
                    options = [] if elide else ["--no-elide"]
                    commands.append((name + ("-elided" if elide else "-checked"),
                                     [engines[name], *options, source], False))
            for name, compiler in [("rust" if name == "release" else name, path)
                                   for name, path in engines.items() if name != "debug"]:
                generated = run([compiler, "--ll", source])
                if generated["exit"] != 0 or "define void @jw_main(" not in generated["stdout"]:
                    infrastructure.append({"case": name + ":" + filename, "phase": "LLVM generation", "observed": generated})
                    continue
                ll = folder / (name + "-" + filename + ".ll")
                ll.write_text(generated["stdout"], encoding="utf-8")
                for level in ["O1", "O2"]:
                    executable = folder / (name + "-" + filename + "-" + level + EXE)
                    command = [os.environ.get("JW_CLANG", "clang"), "-" + level,
                               ll, ROOT / "runtime/jw_rt.c",
                               "-Wno-override-module", "-o", executable]
                    command.extend(["-lshell32"] if sys.platform == "win32" else ["-lm", "-pthread"])
                    if level == "O2":
                        command.extend(["-flto", "-fuse-ld=lld"])
                    built = run(command)
                    if built["exit"] != 0:
                        infrastructure.append({"case": name + ":" + filename + ":" + level, "phase": "native build", "observed": built})
                    else:
                        commands.append((name + "-native-" + level, [executable], True))
            for name, command, native in commands:
                marker = folder / (filename + "-" + name + ".txt")
                assert not marker.exists()
                observed = run([*command, marker], native=native)
                effect = marker.read_text(encoding="utf-8") if marker.exists() else None
                diagnostic_ok = ("dangling reference" in observed["stderr"] if exit_code else
                                 observed["stderr"] == ("[live] 0\n" if native else ""))
                passed = observed["exit"] == exit_code and observed["stdout"] == "" and effect == "x" and diagnostic_ok
                results.append({"compiler": "jwc" if name.startswith("jwc") else "rust", "case": filename, "configuration": name, "status": "pass" if passed else "fail",
                                "expected_exit": exit_code, "expected_file": "x", "observed_file": effect,
                                "observed": observed})
    report = {
        "scope": "Partial practical items 10/11/12: effects occur exactly once before success/error; not whole-item acceptance",
        "platform": sys.platform, "subject": args.subject, "instrumentation": "none", "ordinary_cli_JW_NATIVE": "unset",
        "compiler_sha256": {name: hashlib.sha256(path.read_bytes()).hexdigest() for name, path in engines.items()},
        "source_sha256": {str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest()
                          for path in [ROOT / "runtime/jw_rt.c", Path(__file__).resolve(),
                                       *(ROOT / "tests/safety" / name for name, _ in fixtures)]},
        "results": results, "infrastructure_errors": infrastructure,
    }
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    failures = sum(r["status"] == "fail" for r in results)
    print(json.dumps({"cases": len(results), "passed": len(results) - failures, "failed": failures,
                      "infrastructure_errors": len(infrastructure), "report": str(args.output)}, ensure_ascii=False))
    return 2 if infrastructure else int(bool(failures))


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, subprocess.TimeoutExpired) as error:
        print("verification infrastructure failure (not a pass): " + str(error), file=sys.stderr)
        sys.exit(2)
