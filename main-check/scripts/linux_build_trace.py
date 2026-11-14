#!/usr/bin/env python3
"""Capture Linux build filesystem/process events with strace.

Coverage is deliberately limited to Linux strace's traced process tree.
Failure to attach, unfinished syscalls, truncated logs, unknown syscalls,
network access and unsafe path resolution are hard errors. This captures a
subset of build I/O, not a formal demonstration of monitor completeness.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

SYSCALLS = "open,openat,execve,chdir,clone,fork,vfork"
UNKNOWN = ("unfinished ...", "resumed>", "detached", "ptrace", "+++ killed")
CALL = re.compile(r'^(?:\[pid \d+\] )?(?:\d+\s+)?(\w+)\(')
PATH = re.compile(r'\"((?:\\.|[^\"\\])*)\"')


def collect(command: list[str], cwd: Path, output: Path, timeout: int = 3600) -> dict:
    if sys.platform != "linux":
        raise ValueError("strace adapter requires Linux")
    if not command:
        raise ValueError("missing build command")
    binary = shutil.which("strace")
    if binary is None:
        raise ValueError("strace unavailable")
    cwd = cwd.resolve(strict=True)
    output.parent.mkdir(parents=True, exist_ok=True)
    trace = output.with_suffix(".strace.log")
    invocation = [binary, "-f", "-qq", "-s", "4096", "-e", "trace=" + SYSCALLS,
                  "-o", str(trace), "--", *command]
    process = subprocess.run(invocation, cwd=cwd, capture_output=True, timeout=timeout)
    errors = []
    if process.returncode:
        errors.append("monitored build exited " + str(process.returncode))
    lines = trace.read_text(encoding="utf-8", errors="replace").splitlines()
    if not lines:
        errors.append("empty strace trace")
    captured = []
    from linux_trace_normalize import normalize
    for line in lines:
        if any(mark in line for mark in UNKNOWN):
            errors.append("unresolved trace fragment: " + line[:160])
            continue
        match = CALL.match(line)
        if not match:
            errors.append("unparsed trace line: " + line[:160])
            continue
        kind = match.group(1)
        if kind not in SYSCALLS.split(","):
            errors.append("unknown operation: " + kind)
        paths = PATH.findall(line)
        if kind in ("open", "openat", "openat2", "execve", "execveat") and not paths:
            errors.append("missing path in " + kind)
        captured.append({"syscall": kind, "raw": line[:4096]})
    normalized = None
    if not errors:
        try:
            normalized = normalize(lines, cwd)
        except (OSError, ValueError) as exc:
            errors.append("trace cannot be normalized: " + str(exc))
    result = {"schema_version": 1, "kind": "build-syscall-trace",
              "status": "CAPTURED_UNVERIFIED" if not errors else "BLOCKED",
              "tool": binary, "command": command, "build_exit": process.returncode,
              "trace_sha256": hashlib.sha256(trace.read_bytes()).hexdigest(),
              "events": captured, "normalized_events": normalized, "errors": errors,
              "complete_build_closure_proven": False,
              "limitations": ["strace does not prove absence of unobserved filesystem/environment or toolchain effects.",
                              "Raw syscall trace has not yet been resolved to a full per-process file dependency graph."]}
    output.write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    return result


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--cwd", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("--timeout", type=int, default=3600)
    p.add_argument("command", nargs=argparse.REMAINDER)
    a = p.parse_args(argv)
    command = a.command[1:] if a.command[:1] == ["--"] else a.command
    try:
        result = collect(command, a.cwd, a.output, timeout=a.timeout)
    except (OSError, ValueError, subprocess.TimeoutExpired) as exc:
        result = {"schema_version": 1, "kind": "build-syscall-trace",
                  "status": "BLOCKED", "reason": str(exc),
                  "complete_build_closure_proven": False}
        a.output.parent.mkdir(parents=True, exist_ok=True)
        a.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(result["status"], "->", a.output)
    return 0 if result["status"] == "CAPTURED_UNVERIFIED" else 1

if __name__ == "__main__":
    sys.exit(main())
