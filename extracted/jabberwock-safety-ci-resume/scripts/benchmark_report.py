#!/usr/bin/env python3
"""Compare programs compiled by self-hosted jwc with Rust, C++, and Go ONLY.

The Rust-based Jabberwock compiler is used to bootstrap jwc, not benchmarked.
Linux uses bench/measure.c (wait4); Windows uses PeakWorkingSetSize. One JSON report.
"""
from __future__ import annotations

import argparse
import ctypes
import ctypes.wintypes as wintypes
import datetime as dt
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import shutil
import statistics
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
BENCH = ROOT / "bench"
QUICK = ("fib", "matmul", "mandelbrot", "sort", "linkedlist")
TARGETS = ("jwc", "rust", "cpp", "go")
MAX_LOG = 12000


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(command, *, timeout=600, env=None):
    p = subprocess.run([str(v) for v in command], cwd=ROOT, env=env, capture_output=True,
                       text=True, encoding="utf-8", errors="replace", timeout=timeout)
    return {"cmd": [str(v) for v in command], "exit": p.returncode,
            "stdout": p.stdout[-MAX_LOG:], "stderr": p.stderr[-MAX_LOG:]}


def version(command):
    try:
        p = run(command, timeout=20)
        return (p["stdout"] or p["stderr"]).splitlines()[0] if p["exit"] == 0 else None
    except (OSError, subprocess.TimeoutExpired, IndexError):
        return None


def fingerprint():
    return {"platform": platform.platform(), "machine": platform.machine(),
            "processor": platform.processor(),
            "clang": version(["clang", "--version"]),
            "clang++": version(["clang++", "--version"]),
            "rustc": version(["rustc", "--version"]),
            "go": version(["go", "version"])}


def parse_measure(value):
    try:
        low, median, high, rss = (float(v) for v in value.strip().split())
    except ValueError as e:
        raise ValueError("invalid measure.c result: " + repr(value)) from e
    if (not all(math.isfinite(v) for v in (low, median, high, rss))
            or low <= 0 or median < low or high < median or rss < 0):
        raise ValueError("invalid, zero or nonfinite measurement: " + repr(value))
    return {"min_s": low, "median_s": median, "max_s": high, "peak_rss_mb": rss}


def compare(current, baseline, threshold):
    if current.get("schema_version") != 2 or baseline.get("schema_version") != 2:
        return {"status": "NOT_COMPARABLE", "reason": "report schema differs"}
    if (current.get("subjects") != list(TARGETS) or baseline.get("subjects") != list(TARGETS)
            or current["environment"] != baseline.get("environment")
            or current["runs"] != baseline.get("runs")
            or current["flags"] != baseline.get("flags")
            or current["compiler_hashes"].get("runtime") != baseline.get("compiler_hashes", {}).get("runtime")
            or current["source_hashes"] != baseline.get("source_hashes")):
        return {"status": "NOT_COMPARABLE", "reason": "environment, workload sources, runtime, flags or repeats differ"}
    current_cases = current.get("cases", {})
    baseline_cases = baseline.get("cases", {})
    if (not isinstance(current_cases, dict) or not isinstance(baseline_cases, dict)
            or set(current_cases) != set(baseline_cases)):
        return {"status": "INCOMPLETE_BASELINE",
                "missing_from_baseline": sorted(set(current_cases) - set(baseline_cases)),
                "missing_from_current": sorted(set(baseline_cases) - set(current_cases))}
    if any(v.get("status") != "PASS" for v in current_cases.values()) or any(
            v.get("status") != "PASS" for v in baseline_cases.values()):
        return {"status": "INCOMPLETE_BASELINE", "reason": "one or more cases lack passing results"}
    rows = []
    for name, case in current_cases.items():
        prev = baseline_cases[name]
        old = prev["metrics"]["jwc"]["median_s"]
        new = case["metrics"]["jwc"]["median_s"]
        if (not isinstance(old, (float, int)) or not isinstance(new, (float, int))
                or not math.isfinite(old) or not math.isfinite(new) or old <= 0 or new <= 0):
            return {"status": "INVALID_BASELINE", "reason": "nonfinite or nonpositive time for " + name}
        percent = (new / old - 1) * 100
        rows.append({"case": name, "old_s": old, "new_s": new,
                     "change_percent": round(percent, 2), "regression": percent > threshold})
    return {"status": "COMPARED" if rows else "NO_MATCHING_CASES",
            "threshold_percent": threshold, "cases": rows,
            "regressions": [r["case"] for r in rows if r["regression"]]}

def rotated_targets(round_id: int) -> tuple[str, ...]:
    """Rotate and periodically reverse orders to limit fixed-order bias."""
    if round_id < 0:
        raise ValueError("negative round")
    offset = round_id % len(TARGETS)
    order = TARGETS[offset:] + TARGETS[:offset]
    return tuple(reversed(order)) if (round_id // len(TARGETS)) % 2 else order


def stable_output(digests: list[str]) -> bool:
    """Every repetition must produce exactly the same output."""
    return bool(digests) and len(set(digests)) == 1



def run_windows_measure(binary: Path | str | list[str], output: Path, *, timeout: int = 3600) -> dict:
    """Measure one Windows process with perf_counter and peak working set.

    Accept an executable path or an explicit command for controlled probes.
    Values are working-set MB on Windows, not Linux wait4 maxrss.
    """
    if sys.platform != "win32":
        raise RuntimeError("Windows measurement requires Windows")
    command = ([str(binary)] if isinstance(binary, (str, Path))
               else [str(part) for part in binary])
    if not command:
        raise ValueError("missing Windows measurement command")
    class MemoryCounters(ctypes.Structure):
        _fields_ = [("cb", wintypes.DWORD), ("PageFaultCount", wintypes.DWORD),
                    ("PeakWorkingSetSize", ctypes.c_size_t), ("WorkingSetSize", ctypes.c_size_t),
                    ("QuotaPeakPagedPoolUsage", ctypes.c_size_t), ("QuotaPagedPoolUsage", ctypes.c_size_t),
                    ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t), ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
                    ("PagefileUsage", ctypes.c_size_t), ("PeakPagefileUsage", ctypes.c_size_t)]
    psapi = ctypes.WinDLL("psapi", use_last_error=True)
    get_memory = psapi.GetProcessMemoryInfo
    get_memory.argtypes = [wintypes.HANDLE, ctypes.POINTER(MemoryCounters), wintypes.DWORD]
    get_memory.restype = wintypes.BOOL
    start = time.perf_counter()
    peak = 0
    measured = False
    error_output = Path(str(output) + ".stderr")
    with output.open("wb") as dest, error_output.open("wb") as errors:
        proc = subprocess.Popen(command, cwd=ROOT, stdout=dest,
                                stderr=errors, stdin=subprocess.DEVNULL)
        try:
            while True:
                counters = MemoryCounters()
                counters.cb = ctypes.sizeof(counters)
                if get_memory(int(proc._handle), ctypes.byref(counters), counters.cb):
                    peak = max(peak, counters.PeakWorkingSetSize)
                    measured = True
                if proc.poll() is not None:
                    break
                if time.perf_counter() - start > timeout:
                    proc.kill()
                    proc.wait(timeout=10)
                    raise subprocess.TimeoutExpired(command, timeout)
                time.sleep(0.002)
            proc.wait()
        finally:
            if proc.poll() is None:
                proc.kill()
                proc.wait()
    elapsed = time.perf_counter() - start
    if proc.returncode != 0:
        return {"cmd": command, "exit": proc.returncode,
                "stdout": "", "stderr": error_output.read_text(encoding="utf-8", errors="replace")[-MAX_LOG:]}
    if not measured:
        raise ValueError("Windows peak working set could not be measured")
    return {"cmd": command, "exit": 0,
            "stdout": "{0:.6f} {0:.6f} {0:.6f} {1:.3f}".format(elapsed, peak / 1048576.0),
            "stderr": "", "memory_semantics": "Windows PeakWorkingSetSize; Linux maxrss is different"}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--full", action="store_true")
    p.add_argument("--cases", nargs="+")
    p.add_argument("--runs", type=int, default=8)
    p.add_argument("--jwc", type=Path, help="existing self-hosted compiler (skips bootstrap)")
    p.add_argument("--output", type=Path, default=ROOT / "target/benchmark-report.json")
    p.add_argument("--baseline", type=Path)
    p.add_argument("--threshold", type=float, default=15)
    p.add_argument("--dry-run", action="store_true")
    a = p.parse_args()
    if a.runs < 1 or not math.isfinite(a.threshold) or a.threshold < 0:
        p.error("runs must be >=1, threshold must be finite and >=0")
    all_names = sorted(x.stem for x in (BENCH / "jabberwock").glob("*.jw"))
    selected = a.cases if a.cases else all_names if a.full else list(QUICK)
    if not selected or len(selected) != len(set(selected)) or set(selected) - set(all_names):
        p.error("unknown or duplicate case; available: " + ", ".join(all_names))
    if a.dry_run:
        print("Compilers: jwc (self-hosted) / Rust (rustc) / C++ (clang++) / Go (go)")
        print("Cases:", ", ".join(selected))
        return 0

    out = a.output.resolve()
    out.parent.mkdir(parents=True, exist_ok=True)
    work = out.parent / (out.stem + "-work")
    work.mkdir(parents=True, exist_ok=True)
    exe_suffix = ".exe" if os.name == "nt" else ""
    known_jwc = [ROOT / "target/release" / ("jwc-local" + exe_suffix),
                 ROOT / "target/release" / ("jwc" + exe_suffix),
                 ROOT / ("jwc-bin" + exe_suffix),
                 ROOT / "target/release" / ("jwc-benchmark" + exe_suffix)]
    existing_jwc = next((candidate for candidate in known_jwc
                         if candidate.is_file() and os.access(candidate, os.X_OK)), None)
    jwc = a.jwc.resolve() if a.jwc else (existing_jwc or ROOT / "target/release" / ("jwc-benchmark" + exe_suffix))
    bootstrap_needed = not a.jwc and existing_jwc is None
    rust_bootstrap = ROOT / "target/release" / ("jabberwock" + exe_suffix)
    measure = work / "measure"
    flags = {"jwc_codegen": ["--ll"], "jwc_native": ["clang", "-O2", "-flto", "-fuse-ld=lld"],
             "rust": ["rustc", "-C", "opt-level=3", "-C", "lto=fat", "--edition", "2021"],
             "cpp": ["clang++", "-O2", "-flto", "-fuse-ld=lld", "-std=c++17"] + ([] if os.name == "nt" else ["-pthread"]),
             "go": ["go", "build"]}
    report = {"schema_version": 2, "started_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
              "git_sha": version(["git", "rev-parse", "HEAD"]), "environment": fingerprint(),
              "runs": a.runs, "suite": selected, "subjects": list(TARGETS),
              "measurement_order_balance": ("BALANCED" if a.runs % len(TARGETS) == 0 else "PARTIAL_CYCLE"),
              "flags": flags, "compiler_hashes": {}, "source_hashes": {}, "preparation": [], "cases": {},
              "baseline_comparison": None,
              "limitations": ("Rust-based Jabberwock compiler is bootstrap only. jwc emits LLVM; clang links shared C runtime. "
                              "Equivalent outputs required. Differences in libraries and algorithms remain; "
                              "LTO flags are set for each compiled target but are not identical optimizers.")}

    def save():
        report["updated_utc"] = dt.datetime.now(dt.timezone.utc).isoformat()
        out.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

    def prepare(command, label, timeout=1200):
        try:
            res = run(command, timeout=timeout)
            report["preparation"].append({"name": label, **res})
            save()
            return res["exit"] == 0
        except (OSError, subprocess.TimeoutExpired) as e:
            report["preparation"].append({"name": label, "error": str(e)})
            save()
            return False

    missing = [v for v in (("cc", "clang", "clang++", "rustc", "ld.lld", "go")
                           if sys.platform.startswith("linux") else ("clang", "clang++", "rustc", "go"))
               if not shutil.which(v)]
    if bootstrap_needed and not shutil.which("cargo"):
        missing.append("cargo")
    reason = None
    if sys.platform not in ("linux", "win32"):
        reason = "benchmarks supported only on Windows and Linux"
    elif missing:
        reason = "missing programs: " + ", ".join(missing)
    elif bootstrap_needed:
        if not prepare(["cargo", "build", "--release"], "bootstrap-rust-compiler"):
            reason = "failed to build bootstrap Rust Jabberwock compiler"
        elif not prepare([rust_bootstrap, "build", "-o", jwc, ROOT / "jwc/main.jw"], "bootstrap-jwc", timeout=1800):
            reason = "failed to bootstrap jwc"
    if not reason and not jwc.is_file():
        reason = "jwc executable missing: " + str(jwc)
    if not reason and sys.platform.startswith("linux") and not prepare(["cc", "-O2", "-o", measure, BENCH / "measure.c"], "measure-helper"):
        reason = "failed to compile Linux timing helper"
    if reason:
        for name in selected:
            report["cases"][name] = {"status": "NOT_RUN", "reason": reason}
        save()
        print("NOT_RUN:", reason)
        return 2

    report["compiler_hashes"] = {"jwc": sha(jwc), "runtime": sha(ROOT / "runtime/jw_rt.c"),
                                 "rustc": report["environment"]["rustc"],
                                 "clang++": report["environment"]["clang++"],
                                 "go": report["environment"]["go"]}
    save()
    for index, name in enumerate(selected, 1):
        dest = work / name
        dest.mkdir(exist_ok=True)
        source = BENCH / "jabberwock" / (name + ".jw")
        cpp = BENCH / "cpp" / (name + ".cpp")
        rust = BENCH / "rust" / (name + ".rs")
        go = BENCH / "go" / (name + ".go")
        report["source_hashes"][name] = {lang: sha(path) for lang, path in (("jwc", source), ("cpp", cpp), ("rust", rust), ("go", go))}
        ll = dest / "jwc.ll"
        binaries = {k: dest / (k + exe_suffix) for k in TARGETS}
        item = {"status": "INFRA_ERROR", "commands": [], "metrics": {}, "outputs": {}}
        report["cases"][name] = item
        try:
            # LLVM can be larger than log excerpts: capture full stdout as bytes.
            ll_process = subprocess.run([str(jwc), "--ll", str(source)], cwd=ROOT,
                                        capture_output=True, timeout=600)
            if ll_process.returncode or b"define void @jw_main(" not in ll_process.stdout:
                raise ValueError("jwc LLVM capture failed")
            ll.write_bytes(ll_process.stdout)
            item["commands"].append({"stage": "jwc-LLVM", "exit": ll_process.returncode,
                                     "cmd": [str(jwc), "--ll", str(source)],
                                     "llvm_bytes": len(ll_process.stdout),
                                     "stderr": ll_process.stderr.decode("utf-8", "replace")[-MAX_LOG:]})
            link = (["-lshell32", "-Wl,/stack:268435456"] if os.name == "nt"
                    else ["-lm", "-pthread"])
            compile_steps = [
                ("jwc-native", [*flags["jwc_native"], ll, ROOT / "runtime/jw_rt.c",
                                *link, "-Wno-override-module", "-o", binaries["jwc"]]),
                ("rust", [*flags["rust"], "-o", binaries["rust"], rust]),
                ("cpp", [*flags["cpp"], "-o", binaries["cpp"], cpp]),
                ("go", [*flags["go"], "-o", binaries["go"], go])]
            for label, command in compile_steps:
                result = run(command, timeout=600)
                item["commands"].append({"stage": label, **result})
                if result["exit"]:
                    raise ValueError(label + " compilation failed")
            timings = {target: [] for target in TARGETS}
            memory = {target: [] for target in TARGETS}
            checksums = {target: [] for target in TARGETS}
            for round_id in range(a.runs):
                # Rotate the order so one language is not always first or last.
                order = rotated_targets(round_id)
                for target in order:
                    output = dest / (target + ".run" + str(round_id) + ".stdout")
                    result = (run_windows_measure(binaries[target], output, timeout=3600)
                              if os.name == "nt" else
                              run([measure, 1, output, binaries[target]], timeout=3600))
                    item["commands"].append({"stage": "measure-" + target, "round": round_id, **result})
                    if result["exit"]:
                        raise ValueError(target + " execution failed in round " + str(round_id))
                    one = parse_measure(result["stdout"])
                    timings[target].append(one["median_s"])
                    memory[target].append(one["peak_rss_mb"])
                    data = output.read_bytes()
                    if not data:
                        raise ValueError(target + " produced empty output in round " + str(round_id))
                    checksums[target].append(hashlib.sha256(data).hexdigest())
            for target in TARGETS:
                if not stable_output(checksums[target]):
                    raise ValueError(target + " output changes across repetitions")
                values = sorted(timings[target])
                item["metrics"][target] = {"min_s": values[0],
                                           "median_s": statistics.median(values),
                                           "max_s": values[-1],
                                           "peak_rss_mb": max(memory[target]),
                                           "samples_s": timings[target]}
                item["outputs"][target] = {"sha256": checksums[target][0],
                                           "all_repetitions_verified": len(checksums[target])}
            if len({v["sha256"] for v in item["outputs"].values()}) != 1:
                item["status"] = "OUTPUT_MISMATCH"
            else:
                item["status"] = "PASS"
                item["ratios"] = {
                    "jwc_vs_rust": round(item["metrics"]["jwc"]["median_s"] /
                                         item["metrics"]["rust"]["median_s"], 4),
                    "jwc_vs_cpp": round(item["metrics"]["jwc"]["median_s"] /
                                        item["metrics"]["cpp"]["median_s"], 4),
                    "jwc_vs_go": round(item["metrics"]["jwc"]["median_s"] /
                                       item["metrics"]["go"]["median_s"], 4)}
        except (OSError, ValueError, subprocess.TimeoutExpired) as e:
            item["status"] = "FAIL"
            item["reason"] = str(e)
        save()
        print(f"[{index}/{len(selected)}] {name}: {item['status']}", flush=True)
    if a.baseline:
        try:
            baseline = json.loads(a.baseline.read_text(encoding="utf-8"))
            report["baseline_comparison"] = compare(report, baseline, a.threshold)
        except (OSError, ValueError, KeyError, TypeError) as e:
            report["baseline_comparison"] = {"status": "INVALID_BASELINE", "reason": str(e)}
    save()
    print("Report:", out)
    comparison = report["baseline_comparison"]
    return int(any(x["status"] != "PASS" for x in report["cases"].values())
               or (comparison is not None and
                   (comparison["status"] != "COMPARED" or bool(comparison.get("regressions")))))


if __name__ == "__main__":
    sys.exit(main())
