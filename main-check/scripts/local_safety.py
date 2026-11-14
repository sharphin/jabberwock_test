#!/usr/bin/env python3
"""Run Jabberwock safety probes locally, without GitHub Actions.

Twelve items are acceptance criteria, NOT twelve automatically proven guarantees.
Every command receives an independent log and the JSON report distinguishes failed,
unavailable, and untested probes. Standard library only (Python >= 3.10).
"""
from __future__ import annotations

import argparse
import datetime as dt
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
ASAN = "-fsanitize=address,undefined -fno-sanitize-recover=undefined -fno-omit-frame-pointer"
MSAN = "-fsanitize=memory -fno-omit-frame-pointer"
TSAN = "-fsanitize=thread"
NAMES = {
    1: "検査省略の解析", 2: "型の通る攻撃", 3: "データ競合",
    4: "生成LLVMの未定義動作", 5: "最適化後の動作",
    6: "未初期化メモリ", 7: "自己ホスト版",
    8: "資源枯渇", 9: "巨大な配列型",
    10: "仕様・型検査・副作用", 11: "所有権・回収・途中失敗",
    12: "実環境・実行構成",
}
IS_LINUX = sys.platform.startswith("linux")


def probe(name, items, cmd, *, env=None, linux=False, clang=False, lake=False, jwc=False, timeout=3600):
    environment = dict(env or {})
    if clang and cmd[:2] == ["cargo", "test"]:
        # A native-only probe must fail if its tests cannot reach clang.
        environment["JW_REQUIRE_RETIREMENT_TESTS"] = "1"
    return dict(name=name, items=items, cmd=cmd, env=environment,
                linux=linux, clang=clang, lake=lake, jwc=jwc, timeout=timeout)


def jwc_path():
    return ROOT / "target" / "release" / ("jwc-local.exe" if os.name == "nt" else "jwc-local")


def catalog(scale, output):
    def fuzz(n, seed, flags=ASAN, o2=False, jwc=False):
        env = {"JW_CFLAGS": flags, "JW_FUZZ": str(n * scale),
               "JW_FUZZ_SEED": str(seed), "JW_FUZZ_DUMP": str(output / "fuzz-inputs")}
        if o2:
            env["JW_NATIVE"] = "o2"
        if jwc:
            env["JW_FUZZ_JWC"] = str(jwc_path())
        return env

    result = [
        probe("cargo-build", [7, 12], ["cargo", "build", "--release"], timeout=1200),
        probe("cargo-test", [1, 2, 4, 7, 10, 11, 12],
              ["cargo", "test", "--no-fail-fast"], timeout=1800),
        probe("native-differential", [1, 2, 4, 5, 7, 10, 11, 12],
              ["cargo", "test", "--no-fail-fast"], env={"JW_NATIVE": "1"}, clang=True),
        probe("retirement", [2, 11],
              ["cargo", "test", "--test", "retirement", "--", "--nocapture"], clang=True),
        probe("bench-checks", [1, 5], ["bash", "bench/checks.sh"], clang=True),
        probe("formal-model", [1, 7], [sys.executable, "scripts/check_formal.py"], lake=True),
        probe("jwc-build", [7, 12],
              [str(ROOT / "target" / "release" / ("jabberwock.exe" if os.name == "nt" else "jabberwock")),
               "build", "-o", str(jwc_path()),
               "jwc/main.jw"], clang=True),
        probe("sanitizer-negative-asan", [2, 4, 5], ["cargo", "test", "--test", "sanitizers", "--", "--nocapture"],
              env={"JW_CFLAGS": ASAN}, linux=True, clang=True),
        probe("robust-asan", [2, 4, 11], ["cargo", "test", "--test", "robust", "--", "--nocapture"],
              env={"JW_ROBUST": str(150 * scale), "JW_ROBUST_SEED": "1000",
                   "JW_ROBUST_ASAN": "1", "JW_ROBUST_DUMP": str(output / "robust-inputs")},
              linux=True, clang=True),
        probe("robust-native-asan", [2, 4, 11], ["cargo", "test", "--test", "robust", "mutated", "--", "--nocapture"],
              env={"JW_ROBUST": str(100 * scale), "JW_ROBUST_SEED": "1500",
                   "JW_ROBUST_ASAN": "1", "JW_ROBUST_NATIVE": "1",
                   "JW_ROBUST_DUMP": str(output / "robust-inputs")}, linux=True, clang=True),
        probe("fuzz-asan-o1", [1, 2, 5], ["cargo", "test", "--test", "fuzz", "--", "--nocapture"],
              env=fuzz(200, 2000), linux=True, clang=True),
        probe("fuzz-asan-o2", [1, 2, 5], ["cargo", "test", "--test", "fuzz", "--", "--nocapture"],
              env=fuzz(100, 2500, o2=True), linux=True, clang=True),
        probe("native-all-asan", [2, 4, 5, 12], ["cargo", "test", "--no-fail-fast"],
              env={"JW_NATIVE": "1", "JW_CFLAGS": ASAN}, linux=True, clang=True),
        probe("sanitizer-negative-tsan", [3], ["cargo", "test", "--test", "sanitizers", "--", "--nocapture"],
              env={"JW_CFLAGS": TSAN, "TSAN_OPTIONS": "halt_on_error=1"}, linux=True, clang=True),
        probe("threads-tsan", [3], ["cargo", "test", "--no-fail-fast", "--test", "threads",
                                     "--test", "native", "--test", "system",
                                     "--test", "sanitizers", "--test", "retirement"],
              env={"JW_NATIVE": "1", "JW_CFLAGS": TSAN, "TSAN_OPTIONS": "halt_on_error=1"},
              linux=True, clang=True),
        probe("sanitizer-negative-msan", [6], ["cargo", "test", "--test", "sanitizers", "--", "--nocapture"],
              env={"JW_CFLAGS": MSAN, "MSAN_OPTIONS": "halt_on_error=1"}, linux=True, clang=True),
        probe("native-all-msan", [6], ["cargo", "test", "--no-fail-fast"],
              env={"JW_NATIVE": "1", "JW_CFLAGS": MSAN, "MSAN_OPTIONS": "halt_on_error=1"},
              linux=True, clang=True),
        probe("jwc-fuzz-asan", [7], ["cargo", "test", "--test", "fuzz", "--", "--nocapture"],
              env=fuzz(200, 3000, jwc=True), linux=True, clang=True, jwc=True),
        probe("jwc-fuzz-o2-asan", [7, 5], ["cargo", "test", "--test", "fuzz", "--", "--nocapture"],
              env=fuzz(100, 3500, o2=True, jwc=True), linux=True, clang=True, jwc=True),
        probe("jwc-fuzz-msan", [6, 7], ["cargo", "test", "--test", "fuzz", "--", "--nocapture"],
              env=fuzz(200, 4000, flags=MSAN, jwc=True), linux=True, clang=True, jwc=True),
        probe("jwc-fuzz-o2-msan", [6, 7], ["cargo", "test", "--test", "fuzz", "--", "--nocapture"],
              env=fuzz(100, 4500, flags=MSAN, o2=True, jwc=True),
              linux=True, clang=True, jwc=True),
        probe("edges-8-9", [8, 9],
              [sys.executable, "scripts/retest_safety_edges.py",
               "--compiler", str(ROOT / "target" / "release" / "jabberwock"),
               "--jwc", str(ROOT / "target" / "release" / "jwc-local"),
               "--output", str(output / "edges-8-9.json")],
              linux=True, clang=True, jwc=True),
        probe("effects-10-11-12", [10, 11, 12],
              [sys.executable, "scripts/retest_practical_effects.py",
               "--jwc", str(ROOT / "target" / "release" / "jwc-local"),
               "--output", str(output / "practical-effects.json")],
              linux=True, clang=True, jwc=True),
        probe("release-test", [10, 11, 12],
              ["cargo", "test", "--release", "--no-fail-fast"]),
    ]
    return result


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("profile", nargs="?", choices=["quick", "full"], default="quick")
    ap.add_argument("--items", help="comma-separated safety items 1..12 (overrides profile selection)")
    ap.add_argument("--scale", type=int, default=1, help="fuzz/robust repetitions, >=1")
    ap.add_argument("--report-dir", type=Path, help="default target/local-safety/TIMESTAMP")
    ap.add_argument("--dry-run", action="store_true", help="show planned commands without running them")
    ap.add_argument("--strict", action="store_true", help="non-zero exit if any selected probe is NOT_RUN")
    args = ap.parse_args()
    if args.scale < 1:
        ap.error("--scale must be >= 1")
    now = dt.datetime.now(dt.timezone.utc)
    output = (args.report_dir or ROOT / "target" / "local-safety" / now.strftime("%Y%m%dT%H%M%SZ")).resolve()
    if args.items:
        try:
            chosen = set(int(x) for x in args.items.split(","))
        except ValueError:
            ap.error("--items must be comma-separated numbers")
        if not chosen or chosen - set(NAMES):
            ap.error("--items supports 1..12")
    elif args.profile == "quick":
        chosen = {1, 2, 4, 5, 7, 10, 11, 12}
    else:
        chosen = set(NAMES)
    quick = {"cargo-build", "cargo-test", "native-differential", "retirement", "bench-checks"}
    selected = [p for p in catalog(args.scale, output)
                if p["name"] in quick and not args.items and args.profile == "quick"
                or (args.items or args.profile == "full") and set(p["items"]) & chosen]
    # Keep prerequisite producers ahead of consumers even when the user selects only one item.
    producers = {"cargo-build", "jwc-build"}
    if any(p["jwc"] for p in selected):
        missing = [p for p in catalog(args.scale, output)
                   if p["name"] in producers and p not in selected]
        selected = missing + selected
    if not args.dry_run:
        output.mkdir(parents=True, exist_ok=True)
    git = subprocess.run(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True,
                         capture_output=True) if shutil.which("git") else None
    report = {
        "scope": "Executed probes only; this report does NOT certify completion of 12 acceptance criteria",
        "started_utc": now.isoformat(), "profile": args.profile, "scale": args.scale,
        "requested_items": sorted(chosen), "platform": platform.platform(),
        "python": sys.version, "commit": git.stdout.strip() if git and git.returncode == 0 else None,
        "results": [], "items": [],
    }
    failed = unavailable = 0
    for index, p in enumerate(selected, 1):
        reason = None
        if p["linux"] and not IS_LINUX:
            reason = "requires Linux sanitizer/resource infrastructure"
        elif p["clang"] and not shutil.which(os.environ.get("JW_CLANG", "clang")):
            reason = "clang unavailable"
        elif p["lake"] and not shutil.which("lake"):
            reason = "Lean lake unavailable"
        elif not shutil.which(p["cmd"][0]) and not Path(p["cmd"][0]).is_file():
            reason = "command executable unavailable: " + p["cmd"][0]
        if p["jwc"] and not args.dry_run and not jwc_path().is_file():
            reason = reason or "jwc-local was not built"
        status = "NOT_RUN" if reason else ("PLANNED" if args.dry_run else "FAIL")
        entry = {"name": p["name"], "items": p["items"], "status": status,
                 "command": p["cmd"], "environment_overrides": p["env"], "reason": reason}
        if not args.dry_run and not reason:
            env = dict(os.environ)
            # Avoid inherited flags changing which backend is actually verified.
            for key in ("JW_CFLAGS", "JW_NATIVE", "JW_ROBUST_ASAN", "JW_ROBUST_NATIVE",
                        "JW_FUZZ_JWC", "JW_FUZZ", "JW_ROBUST", "JWC_REGEN"):
                env.pop(key, None)
            env.update(p["env"])
            log = output / (f"{index:02d}-" + p["name"] + ".log")
            start = time.monotonic()
            try:
                with log.open("w", encoding="utf-8") as f:
                    f.write("COMMAND: " + repr(p["cmd"]) + "\n")
                    f.write("OVERRIDES: " + repr(p["env"]) + "\n\n")
                    f.flush()
                    run = subprocess.run(p["cmd"], cwd=ROOT, env=env, stdout=f,
                                         stderr=subprocess.STDOUT, timeout=p["timeout"])
                entry["exit_code"] = run.returncode
                entry["status"] = "PROBE_PASS" if run.returncode == 0 else "FAIL"
            except subprocess.TimeoutExpired:
                entry["status"], entry["reason"] = "FAIL", "timeout"
            except OSError as exc:
                entry["status"], entry["reason"] = "NOT_RUN", str(exc)
            entry["seconds"] = round(time.monotonic() - start, 2)
            entry["log"] = str(log)
        failed += entry["status"] == "FAIL"
        unavailable += entry["status"] == "NOT_RUN"
        print(f"[{index}/{len(selected)}] {p['name']}: {entry['status']}" +
              (f" ({entry['reason']})" if entry.get("reason") else ""), flush=True)
        report["results"].append(entry)
    for n in sorted(chosen):
        relevant = [r for r in report["results"] if n in r["items"]]
        # A green probe is NOT whole-item acceptance. See verification-plan.md.
        status = ("FAIL" if any(r["status"] == "FAIL" for r in relevant)
                  else "NOT_RUN" if not relevant or any(r["status"] == "NOT_RUN" for r in relevant)
                  else "PROBES_PASS_ONLY")
        report["items"].append({"item": n, "name": NAMES[n], "status": status,
                                "acceptance": "NOT_ASSESSED",
                                "probes": [r["name"] for r in relevant]})
    report["finished_utc"] = dt.datetime.now(dt.timezone.utc).isoformat()
    if not args.dry_run:
        result_file = output / "report.json"
        result_file.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        print(f"Report: {result_file}")
    print(f"Probes: {len(selected)}; failed: {failed}; not run: {unavailable}")
    return int(bool(failed or (args.strict and unavailable)))


if __name__ == "__main__":
    sys.exit(main())
