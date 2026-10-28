#!/usr/bin/env python3
"""Portable check-count regression test for the legacy Rust bootstrap compiler.

Unlike bench/checks.sh, this runner works under Windows with 'py'. It does NOT
pretend that the self-hosted jwc supports the Rust CLI's --stats flags.
"""
from __future__ import annotations

import argparse
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
BENCH = ROOT / "bench"
STATS = re.compile(r"^\[stats\] 世代の検査 (\d+) か所・解析で消した (\d+) か所・添字の検査 (\d+) か所$", re.M)
COUNT = re.compile(r"^\[checks\] (.+)$", re.M)
HEADER = "# 名前 場所:世代 場所:消した 場所:添字 実行時:世代 実行時:消した 実行時:添字（bench/checks.sh が書く）"


def parse_rows(text: str) -> dict[str, tuple[int, ...]]:
    rows = {}
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        fields = line.split()
        if len(fields) != 7 or not all(re.fullmatch(r"\d+", x) for x in fields[1:]):
            raise ValueError("malformed check-count row: " + line)
        name = fields[0]
        if name in rows:
            raise ValueError("duplicate check-count case: " + name)
        rows[name] = tuple(int(x) for x in fields[1:])
    if not rows:
        raise ValueError("no check-count cases")
    return rows


def parse_statistics(output: str) -> tuple[int, int, int]:
    matches = STATS.findall(output)
    if len(matches) != 1:
        raise ValueError("missing or duplicate compiler statistics")
    return tuple(map(int, matches[0]))


def parse_runtime_counts(output: str) -> tuple[int, int, int]:
    matches = COUNT.findall(output)
    if len(matches) != 1:
        raise ValueError("missing or duplicate runtime check counters")
    fields = matches[0].split()
    if (len(fields) != 6 or fields[0::2] != ["gen", "elided", "bounds"]
            or not all(re.fullmatch(r"\d+", fields[i]) for i in (1, 3, 5))):
        raise ValueError("malformed or unexpected runtime check labels")
    return tuple(int(fields[i]) for i in (1, 3, 5))


def compare(baseline: dict, current: dict) -> list[str]:
    errors = []
    for name in sorted(set(baseline) ^ set(current)):
        errors.append("missing/extra benchmark case: " + name)
    for name in sorted(set(baseline) & set(current)):
        old, new = baseline[name], current[name]
        if (new[0] > old[0] or new[2] > old[2] or new[3] > old[3] or
                new[5] > old[5] or new[1] < old[1] or new[4] < old[4]):
            errors.append("check-count regression: " + name)
    return errors


def invoke(command: list[str], *, timeout: int = 1200) -> subprocess.CompletedProcess:
    result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True,
                            encoding="utf-8", errors="replace", timeout=timeout)
    if result.returncode != 0:
        raise RuntimeError("command failed (" + str(result.returncode) + "): " +
                           " ".join(command) + "\n" + result.stderr[-3000:])
    return result


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", type=Path, default=ROOT / "target/release" /
                        ("jabberwock.exe" if os.name == "nt" else "jabberwock"))
    parser.add_argument("--baseline", type=Path, default=BENCH / "checks.txt")
    parser.add_argument("--update", action="store_true")
    args = parser.parse_args(argv)
    compiler = args.compiler.resolve()
    if not compiler.is_file():
        parser.error("legacy compiler not found: " + str(compiler))
    sources = sorted((BENCH / "jabberwock").glob("*.jw"))
    if not sources:
        parser.error("no benchmark sources")
    try:
        with tempfile.TemporaryDirectory(prefix="jw-check-count-") as work:
            temp = Path(work)
            actual = {}
            for source in sources:
                name = source.stem
                normal = temp / (name + (".exe" if os.name == "nt" else ""))
                instrumented = temp / (name + "-count" + (".exe" if os.name == "nt" else ""))
                stats = invoke([str(compiler), "build", "--stats", "-o", str(normal), str(source)])
                generated = parse_statistics(stats.stdout + "\n" + stats.stderr)
                invoke([str(compiler), "build", "--count-checks", "-o", str(instrumented), str(source)])
                run = invoke([str(instrumented)])
                counted = parse_runtime_counts(run.stderr)
                if name in actual:
                    raise ValueError("duplicate benchmark stem: " + name)
                actual[name] = generated + counted
                print("CHECK", name, *actual[name], flush=True)
            if args.update:
                args.baseline.parent.mkdir(parents=True, exist_ok=True)
                body = HEADER + "\n" + "".join(name + " " + " ".join(map(str, actual[name])) + "\n"
                                                    for name in sorted(actual))
                args.baseline.write_text(body, encoding="utf-8")
            else:
                baseline = parse_rows(args.baseline.read_text(encoding="utf-8"))
                errors = compare(baseline, actual)
                if errors:
                    for error in errors:
                        print("FAIL:", error, file=sys.stderr)
                    return 1
        return 0
    except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired) as exc:
        print("INFRA_ERROR:", exc, file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
