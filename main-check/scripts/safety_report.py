#!/usr/bin/env python3
"""Run one OS's independent safety items 1..12 and write ONE JSON report.

A passing probe does not establish formal acceptance of the corresponding item.
Python 3.10+; platform-dependent checks are recorded as NOT_RUN, never passed.
"""
from __future__ import annotations

import argparse
import datetime as datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import signal
import subprocess
import sys
import tempfile
import time

import local_safety as safety
import safety_platforms as suites
import safety_compilers
import safety_goals
from formal_report import source_identity
from acceptance_requirements import test_input_identity
from windows_asan_runtime import environment as runtime_environment

ROOT = Path(__file__).resolve().parents[1]
LOG_LIMIT = 24000
HEARTBEAT_SECONDS = 15


class ReportSaveError(Exception):
    pass


class ReportWriter:
    """Keep a complete snapshot even when Windows refuses destination replacement."""
    def __init__(self, path):
        self.path = path

    def save(self, report):
        temporary = None
        try:
            # Unique names also avoid sharing the old fixed .json.tmp file.
            with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", dir=self.path.parent,
                                             prefix=self.path.stem + "-", suffix=".json",
                                             delete=False) as handle:
                temporary = Path(handle.name)
                json.dump(report, handle, ensure_ascii=False, indent=2)
                handle.write("\n")
            # Close the handle before rename, as required on Windows.
            for delay in (0, 0.05, 0.1, 0.2, 0.4):
                if delay:
                    time.sleep(delay)
                try:
                    temporary.replace(self.path)
                    return
                except PermissionError:
                    continue
            previous = self.path
            self.path = temporary
            print("WARNING: report replacement denied: " + str(previous), flush=True)
            print("Report redirected to: " + str(self.path), flush=True)
        except OSError as error:
            # Preserve the existing report; remove only our incomplete new file.
            if temporary is not None:
                try:
                    temporary.unlink(missing_ok=True)
                except OSError:
                    pass
            raise ReportSaveError("cannot save report in " + str(self.path.parent) + ": " + str(error)) from error


def stop_process(process):
    """Bound cleanup time and stop compiler/test descendants where supported."""
    try:
        if os.name == "nt":
            stopped = subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"],
                                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=10)
            if stopped.returncode:
                print("WARNING: taskkill could not confirm process-tree termination for PID " + str(process.pid), flush=True)
        else:
            os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    except (OSError, subprocess.TimeoutExpired) as error:
        print("WARNING: process-tree cleanup failed: " + str(error), flush=True)
    if process.poll() is None:
        try:
            process.kill()
        except OSError as error:
            print("WARNING: child termination failed: " + str(error), flush=True)
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        print("WARNING: child cleanup did not finish; inspect PID " + str(process.pid), flush=True)


def execute_probe(command, *, env, timeout, name, log_paths):
    """Write directly to files: inherited child pipes cannot block timeout cleanup."""
    options = ({"creationflags": subprocess.CREATE_NEW_PROCESS_GROUP} if os.name == "nt"
               else {"start_new_session": True})
    begin = time.monotonic()
    with log_paths["stdout"].open("wb") as out, log_paths["stderr"].open("wb") as err:
        process = subprocess.Popen(command, cwd=ROOT, env=env, stdin=subprocess.DEVNULL,
                                   stdout=out, stderr=err, **options)
        try:
            while True:
                remaining = timeout - (time.monotonic() - begin)
                if remaining <= 0:
                    raise subprocess.TimeoutExpired(command, timeout)
                try:
                    process.wait(timeout=min(HEARTBEAT_SECONDS, remaining))
                    break
                except subprocess.TimeoutExpired:
                    print(f"RUNNING {name}: {int(time.monotonic() - begin)}s / {timeout}s (PID {process.pid})", flush=True)
        except BaseException:
            stop_process(process)
            raise
    return subprocess.CompletedProcess(command, process.returncode,
                                       log_paths["stdout"].read_text(encoding="utf-8", errors="replace"),
                                       log_paths["stderr"].read_text(encoding="utf-8", errors="replace"))


def excerpt(value: str) -> dict:
    """Keep the beginning AND end of diagnostics, including early errors and final summaries."""
    raw = value.encode("utf-8", errors="replace")
    if len(raw) <= LOG_LIMIT:
        return {"text": value, "truncated": False, "bytes": len(raw)}
    front = raw[:LOG_LIMIT // 2].decode("utf-8", errors="replace")
    back = raw[-LOG_LIMIT // 2:].decode("utf-8", errors="replace")
    return {"text": front + "\n...[truncated]...\n" + back,
            "truncated": True, "bytes": len(raw),
            "sha256": hashlib.sha256(raw).hexdigest()}


def version(command: list[str]) -> str | None:
    if not shutil.which(command[0]):
        return None
    try:
        r = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, timeout=8)
        return (r.stdout or r.stderr).strip()[:400] if r.returncode == 0 else None
    except (OSError, subprocess.TimeoutExpired):
        return None


def run_probe(probe: dict, report_dir: Path, dry_run: bool) -> dict:
    name, command = probe["name"], probe["cmd"]
    entry = {"name": name, "items": probe["items"], "command": command,
             "environment_overrides": probe["env"], "status": "NOT_RUN",
             "expected": {"exit_code": 0}, "reason": None, "compiler": probe.get("compiler")}
    if probe.get("provisional_detector"):
        entry["provisional_detector"] = probe["provisional_detector"]
    if probe.get("evidence_scope"):
        entry["evidence_scope"] = probe["evidence_scope"]
    if probe.get("target_os"):
        entry["target_os"] = probe["target_os"]
    if dry_run:
        entry["status"] = "PLANNED"
        entry["reason"] = probe.get("unavailable_reason")
        return entry
    reason = None
    if probe.get("unavailable_reason"):
        reason = probe["unavailable_reason"]
    elif probe.get("target_os") and probe["target_os"] != suites.host_os():
        reason = "probe requires " + probe["target_os"] + "; results from another OS cannot substitute"
    elif probe["linux"] and not safety.IS_LINUX:
        reason = "Linux-only verification unavailable"
    elif probe["clang"] and not shutil.which(os.environ.get("JW_CLANG", "clang")):
        reason = "clang unavailable"
    elif probe["lake"] and not shutil.which("lake"):
        reason = "Lean lake unavailable"
    elif not shutil.which(command[0]) and not Path(command[0]).is_file():
        reason = "command executable unavailable: " + command[0]
    elif probe["jwc"] and not Path(probe.get("jwc_executable", safety.jwc_path())).is_file():
        reason = "jwc-local not available"
    if reason:
        entry["reason"] = reason
        return entry

    nested = probe.get("case_report") or {
        "edges-8-9": report_dir / "edges-8-9.json",
        "effects-10-11-12": report_dir / "practical-effects.json",
    }.get(name)
    nested = Path(nested) if nested else None
    if nested:
        try:
            # Never attach an old run's cases to the current command.
            nested.unlink(missing_ok=True)
        except OSError as error:
            entry["status"], entry["reason"] = "INFRA_ERROR", "cannot clear old case report: " + str(error)
            return entry

    if name in ("cargo-test", "release-test") and not shutil.which(os.environ.get("JW_CLANG", "clang")):
        entry["coverage_notes"] = [
            "clang unavailable: some Cargo tests return early but are displayed as ok; "
            "native/jwc/sanitizer coverage is not established by this result"
        ]

    env = dict(os.environ)
    for k in ("JW_CFLAGS", "JW_NATIVE", "JW_ROBUST_ASAN", "JW_ROBUST_NATIVE",
              "JW_SAFETY_COMPILER", "JW_SAFETY_JWC", "JW_SAFETY_STRICT", "JW_FUZZ_JWC", "JW_FUZZ", "JW_ROBUST", "JWC_REGEN", "JW_REQUIRE_RETIREMENT_TESTS"):
        env.pop(k, None)
    env.update(probe["env"])
    report_dir.mkdir(parents=True, exist_ok=True)
    log_paths = {stream: report_dir / (name + "." + stream + ".log")
                 for stream in ("stdout", "stderr")}
    entry["log_files"] = {k: str(v) for k, v in log_paths.items()}
    print(f"START {name}: timeout {probe['timeout']}s", flush=True)
    print("  command: " + subprocess.list2cmdline(command), flush=True)
    print("  logs: " + str(log_paths["stdout"]) + " | " + str(log_paths["stderr"]), flush=True)
    begin = time.monotonic()
    try:
        run = execute_probe(command, env=env, timeout=probe["timeout"], name=name, log_paths=log_paths)
        entry["executed_required_tests"] = [name for name in probe.get("required_test_names", []) if "test " + name + " ... ok" in run.stdout]
        entry["actual"] = {"exit_code": run.returncode,
                           "stdout": excerpt(run.stdout), "stderr": excerpt(run.stderr)}
        entry["status"] = "PROBE_PASS" if run.returncode == 0 else "FAIL"
        if run.returncode < 0:
            entry["status"] = "CRASH"
            entry["reason"] = "terminated by signal " + str(-run.returncode)
        elif run.returncode == 2 and nested:
            entry["status"] = "INFRA_ERROR"
    except subprocess.TimeoutExpired as error:
        entry["status"], entry["reason"] = "TIMEOUT", "timeout after " + str(probe["timeout"]) + " seconds"
        entry["actual"] = {"stdout": excerpt((error.stdout or b"").decode("utf-8", "replace") if isinstance(error.stdout, bytes) else (error.stdout or "")),
                           "stderr": excerpt((error.stderr or b"").decode("utf-8", "replace") if isinstance(error.stderr, bytes) else (error.stderr or ""))}
    except OSError as error:
        entry["status"], entry["reason"] = "INFRA_ERROR", str(error)
    except KeyboardInterrupt:
        entry["status"], entry["reason"] = "INTERRUPTED", "interrupted by user"
    if entry["status"] in ("TIMEOUT", "INFRA_ERROR", "INTERRUPTED"):
        actual = entry.setdefault("actual", {})
        for stream, path in log_paths.items():
            if path.is_file():
                actual[stream] = excerpt(path.read_text(encoding="utf-8", errors="replace"))
    entry["seconds"] = round(time.monotonic() - begin, 3)

    # Existing targeted scripts write per-case details. Inline them so the JSON is
    # independently usable without uploading adjacent log files.
    if nested and nested.is_file():
        try:
            details = json.loads(nested.read_text(encoding="utf-8"))
            if not isinstance(details, dict) or not isinstance(details.get("results"), list):
                raise ValueError("expected an object containing a results list")
            entry["case_report"] = details
            cases = details["results"]
            if probe.get("expected_subject") and details.get("subject") != probe["expected_subject"]:
                raise ValueError("case report belongs to a different compiler")
            if probe.get("compiler") and any(r.get("compiler") != probe["compiler"] for r in cases if isinstance(r, dict)):
                raise ValueError("mixed compiler cases")
            # Every nested report must identify individual executed cases.
            # Some legacy probes use (case, configuration) rather than id.
            identities = []
            for item in cases:
                if not isinstance(item, dict):
                    raise ValueError("non-object executed case")
                identity = item.get("id")
                if identity is None and isinstance(item.get("case"), str):
                    configuration = item.get("configuration")
                    identity = item["case"] + (":" + configuration if isinstance(configuration, str) else "")
                if not isinstance(identity, str) or not identity:
                    raise ValueError("executed case without identity")
                identities.append(identity)
            if len(identities) != len(set(identities)):
                raise ValueError("duplicate executed case identity")
            # Dynamic case inventories may grow, but the runner must execute
            # exactly the complete inventory emitted by the case generator.
            declared = details.get("expected_case_ids")
            if declared is not None:
                if (not isinstance(declared, list) or not declared or
                        any(not isinstance(x, str) or not x for x in declared) or
                        len(declared) != len(set(declared))):
                    raise ValueError("invalid or duplicate declared case IDs")
                executed = [r.get("id") for r in cases if isinstance(r, dict)]
                if len(executed) != len(set(executed)) or set(executed) != set(declared):
                    raise ValueError("executed case IDs differ from dynamic declared inventory")
            if probe.get("required_case_ids"):
                ids = [r.get("id") for r in cases if isinstance(r, dict)]
                if len(ids) != len(set(ids)) or set(ids) != set(probe["required_case_ids"]):
                    raise ValueError("missing, duplicate or unexpected case IDs")
            if not cases or any(not isinstance(r, dict) or r.get("case_status", r.get("status")) not in ("pass", "fail")
                                for r in cases):
                raise ValueError("no executed cases or invalid case status")
            if probe.get("expected_cases") is not None and len(cases) != probe["expected_cases"]:
                raise ValueError("expected " + str(probe["expected_cases"]) + " cases, got " + str(len(cases)))
            if entry["status"] not in ("TIMEOUT", "CRASH", "INFRA_ERROR", "INTERRUPTED"):
                if details.get("infrastructure_errors"):
                    entry["status"] = "INFRA_ERROR"
                    entry["reason"] = "case report contains infrastructure errors"
                elif any(r.get("case_status", r.get("status")) == "fail" for r in cases):
                    entry["status"] = "FAIL"
        except (OSError, ValueError) as error:
            if entry["status"] not in ("TIMEOUT", "CRASH", "INTERRUPTED"):
                entry["status"], entry["reason"] = "INFRA_ERROR", "invalid case report: " + str(error)
    elif nested and entry["status"] not in ("TIMEOUT", "CRASH", "INFRA_ERROR", "INTERRUPTED"):
        entry["status"], entry["reason"] = "INFRA_ERROR", "case report was not produced"
    if entry["status"] == "PROBE_PASS" and probe.get("required_test_names"):
        if set(entry.get("executed_required_tests", [])) != set(probe["required_test_names"]):
            entry["status"], entry["reason"] = "INFRA_ERROR", "required Cargo test did not report execution"
    if entry["status"] != "PROBE_PASS":
        print(f"{entry['status']} {name}: {entry.get('reason') or 'exit ' + str(entry.get('actual', {}).get('exit_code'))}", flush=True)
        actual = entry.get("actual", {})
        diagnostic = actual.get("stderr", {}).get("text") or actual.get("stdout", {}).get("text", "")
        if diagnostic:
            print(diagnostic[-3000:], flush=True)
    return entry


def main(argv=None, *, target_os=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--platform", choices=suites.TARGETS, help="default: this OS; Windows and Linux results stay separate")
    ap.add_argument("--output", type=Path, help="default: target/safety-<OS>-report.json")
    ap.add_argument("--scale", type=int, default=1, help="fuzz repetitions multiplier")
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--compiler", choices=("all", "jwc", "rust"), default="all")
    args = ap.parse_args(argv)
    if args.scale < 1:
        ap.error("--scale must be >= 1")
    if target_os and args.platform and target_os != args.platform:
        ap.error("this entry point only runs the " + target_os + " suite")
    selected_os = target_os or args.platform or suites.host_os()
    if selected_os not in suites.TARGETS:
        ap.error("only Windows and Linux have safety suites")
    if not args.dry_run and selected_os != suites.host_os():
        ap.error("run the " + selected_os + " suite on " + selected_os + "; WSL2 produces Linux results, not Windows results")
    output = (args.output or ROOT / "target" / ("safety-" + selected_os + "-report.json")).resolve()
    work = output.parent / (output.stem + "-work")
    if not args.dry_run:
        work.mkdir(parents=True, exist_ok=True)

    compilers = safety_compilers.COMPILERS if args.compiler == "all" else (args.compiler,)
    prereq, catalog = safety_compilers.catalog(selected_os, args.scale, work, compilers)
    ordered = prereq + catalog

    def recorded_version(command):
        return None if args.dry_run else version(command)
    git = recorded_version(["git", "rev-parse", "HEAD"])
    report = {
        "schema_version": 3,
        "suite": {"id": selected_os + "-compiler-matrix", "target_os": selected_os, "compilers": compilers, "items_per_compiler": 12, "item_count": 12 * len(compilers), "goals_per_compiler": 5},
        "scope": "Probe results only. Not proof or certification of all 12 acceptance criteria.",
        "started_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "commit": git,
        "platform": {"os": platform.platform(), "system": platform.system(),
                     "machine": platform.machine(), "python": sys.version},
        "tools": {"rustc": recorded_version(["rustc", "--version"]), "cargo": recorded_version(["cargo", "--version"]),
                  "clang": recorded_version([os.environ.get("JW_CLANG", "clang"), "--version"])},
        "scale": args.scale, "source_snapshots": {c: source_identity(c) for c in compilers} if not args.dry_run else {},
        "planned_probes": [{"name": p["name"], "compiler": p.get("compiler"),
                            "items": list(p["items"]), "goal": p.get("goal"),
                            "required_case_ids": p.get("required_case_ids", []),
                            "expected_cases": p.get("expected_cases")} for p in ordered],
        "test_inputs_snapshot": test_input_identity() if not args.dry_run else {},
        "preparation": [], "items": [], "results": [],
    }
    produced = {}
    report.update(run_status="RUNNING", active_probe=None)
    writer = ReportWriter(output)

    def checkpoint():
        report["results"] = [produced[p["name"]] for p in ordered if p["name"] in produced]
        report["preparation"] = [produced[p["name"]] for p in prereq if p["name"] in produced]
        report["items"] = []
        for compiler in compilers:
            for n in range(1, 13):
                references = [p["name"] for p in ordered if n in p["items"] and p.get("compiler") == compiler]
                pending = [name for name in references if name not in produced]
                results = [produced[name] for name in references if name in produced]
                problems = [x for x in results if x["status"] in
                            ("FAIL", "CRASH", "TIMEOUT", "INFRA_ERROR", "INTERRUPTED")]
                unavailable = [x["name"] for x in results if x["status"] in ("NOT_RUN", "PLANNED")]
                status = ("FAIL" if problems else "NOT_RUN" if pending or unavailable or not results
                          or any(x["status"] != "PROBE_PASS" for x in results)
                          else "PROBES_PASS_ONLY")
                report["items"].append({"item": n, "name": safety.NAMES[n],
                                        "status": "PLANNED" if args.dry_run else status,
                                        "compiler": compiler, "id": selected_os + "-" + compiler + "-" + str(n).zfill(2), "target_os": selected_os,
                                        "acceptance": "BLOCKED", "coverage_gaps": safety_goals.GAPS[n], "probe_names": references,
                                        "pending_probe_names": pending,
                                        "failures": [x["name"] for x in problems],
                                        "unavailable": unavailable})
                if not args.dry_run:
                    # 合意した基準（safety_goals.AGREED）。acceptance とは別の欄
                    report["items"][-1]["agreed"] = safety_goals.agreed(report["items"][-1], compiler, selected_os,
                                                                         produced, references)
        report["goals"] = safety_goals.goals(report["items"], compilers, selected_os, catalog, produced)
        if not args.dry_run:
            for goal in report["goals"]:
                goal["agreed"] = safety_goals.agreed_goal(goal, report["items"])
        if not args.dry_run:
            report["updated_utc"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
            writer.save(report)

    try:
        checkpoint()
        for p in ordered:
            report["active_probe"] = p["name"]
            checkpoint()
            dependencies = p.get("dependencies", [])
            blocked = [name for name in dependencies if produced.get(name, {}).get("status") != "PROBE_PASS"]
            if blocked and not args.dry_run:
                r = {"name": p["name"], "compiler": p.get("compiler"), "items": p["items"], "target_os": selected_os,
                     "command": p["cmd"], "environment_overrides": p["env"], "status": "NOT_RUN",
                     "reason": "prerequisite did not pass: " + ", ".join(blocked)}
            else:
                provider = p.get("runtime_environment_from")
                if provider and not args.dry_run:
                    supplied = produced.get(provider, {}).get("case_report", {}).get("runtime_environment", {})
                    directory = supplied.get("path_prepend")
                    dll = supplied.get("dll_path")
                    # Check the exact file verified by preflight has not changed/disappeared.
                    try:
                        valid = directory and dll and Path(dll).is_file() and Path(dll).parent == Path(directory)
                        valid = valid and hashlib.sha256(Path(dll).read_bytes()).hexdigest() == supplied.get("dll_sha256")
                    except OSError:
                        valid = False
                    if not valid:
                        r = {"name": p["name"], "items": p["items"], "compiler": p.get("compiler"),
                             "status": "INFRA_ERROR", "reason": "verified ASan runtime environment missing or changed"}
                    else:
                        effective = {**p, "env": {**p["env"], "PATH": runtime_environment(directory)["PATH"]}}
                        r = run_probe(effective, work, args.dry_run)
                        r["verified_runtime"] = supplied
                else:
                    r = run_probe(p, work, args.dry_run)
            produced[p["name"]] = r
            report["active_probe"] = None
            checkpoint()
            label = "PREP " if p in prereq else "PROBE "
            print(label + p["name"] + ": " + r["status"], flush=True)
            if r["status"] == "INTERRUPTED":
                report["run_status"] = "INTERRUPTED"
                break
        else:
            report["run_status"] = "COMPLETE"
    except KeyboardInterrupt:
        report["run_status"] = "INTERRUPTED"
    except ReportSaveError as error:
        print("INFRA_ERROR: " + str(error), file=sys.stderr, flush=True)
        print("Stopped; last saved report (if present): " + str(writer.path), file=sys.stderr, flush=True)
        return 2
    if not args.dry_run:
        try:
            after = {c: source_identity(c) for c in compilers}
            after_inputs = test_input_identity()
            if after_inputs != report["test_inputs_snapshot"]:
                report["run_status"] = "TEST_INPUTS_CHANGED"
                report["test_inputs_changed"] = True
                report["test_inputs_end_snapshot"] = after_inputs
            if any(after[c] != report["source_snapshots"][c] for c in compilers):
                report["run_status"] = "SOURCE_CHANGED"
                report["source_changed"] = True
                report["source_end_snapshots"] = after
        except (OSError, ValueError, KeyError) as exc:
            report["run_status"] = "SOURCE_IDENTITY_ERROR"
            report["source_identity_error"] = str(exc)
    report["finished_utc"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    try:
        checkpoint()
    except ReportSaveError as error:
        print("INFRA_ERROR: " + str(error), file=sys.stderr, flush=True)
        print("Final results could not be saved; last saved report: " + str(writer.path), file=sys.stderr, flush=True)
        return 2
    for item in report["items"]:
        agreed = f" (agreed: {item['agreed']['status']})" if "agreed" in item else ""
        print(f"{selected_os.upper()} {item['compiler']} ITEM {item['item']:02d}/12 {item['name']}: {item['status']}{agreed}", flush=True)
    for goal in report["goals"]:
        agreed = f", agreed: {goal['agreed']['status']}" if "agreed" in goal else ""
        print(f"{selected_os.upper()} {goal['compiler']} GOAL {goal['name']}: {goal['status']} (acceptance: {goal['acceptance']}{agreed})", flush=True)
    if not args.dry_run:
        print("Report: " + str(writer.path), flush=True)
    else:
        print("Dry run; no report written", flush=True)
    return (130 if report["run_status"] == "INTERRUPTED" else
            int(report["run_status"] != "COMPLETE" or
                len(report["items"]) != 12 * len(compilers) or
                len(report["goals"]) != 5 * len(compilers) or
                any(i["status"] != "PROBES_PASS_ONLY" for i in report["items"] + report["goals"])))


if __name__ == "__main__":
    sys.exit(main())
