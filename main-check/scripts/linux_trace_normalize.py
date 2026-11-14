#!/usr/bin/env python3
"""Normalize a deliberately restricted subset of Linux strace build events.

Accepted streams require a known cwd for each process and straightforward
absolute or AT_FDCWD paths. Unsupported syscall forms fail closed; the output
is suitable for build_access_audit, but cannot certify tracing completeness.
"""
from __future__ import annotations
import argparse
import json
from pathlib import Path
import re
import sys

LINE = re.compile(r'^(?:\[pid (?P<pid1>\d+)\] |(?P<pid2>\d+)\s+)?(?P<call>[a-z][a-z0-9_]*)\((?P<args>.*)\)\s+=\s+(?P<result>.*)$')
QUOTED = re.compile(r'^"((?:\\.|[^"\\])*)"')
SUPPORTED = {"open", "openat", "execve", "chdir", "clone", "fork", "vfork"}


def normalize(lines: list[str], cwd: Path) -> dict:
    root = cwd.resolve(strict=True)
    cwd_by_pid: dict[str, Path] = {"0": root}
    initial_pid = None
    events = [{"type": "begin"}]
    for count, line in enumerate(lines, 1):
        if not line.strip():
            continue
        match = LINE.fullmatch(line.strip())
        if not match:
            raise ValueError("unparsed syscall record at line " + str(count))
        kind = match["call"]
        if kind not in SUPPORTED:
            raise ValueError("unsupported syscall: " + kind)
        pid = match["pid1"] or match["pid2"] or "0"
        if initial_pid is None:
            initial_pid = pid
            cwd_by_pid[pid] = root
        if pid not in cwd_by_pid:
            raise ValueError("untracked process: " + pid)
        arguments = match["args"]
        returned = match["result"]
        if kind in ("clone", "fork", "vfork"):
            child = re.match(r"(\d+)(?:\s|$)", returned)
            if child:
                cwd_by_pid[child.group(1)] = cwd_by_pid[pid]
            elif not returned.startswith("-1"):
                raise ValueError("unclear process creation")
            continue
        if kind == "openat":
            prefix = "AT_FDCWD, "
            if not arguments.startswith(prefix):
                raise ValueError("openat with untracked directory descriptor")
            arguments = arguments[len(prefix):]
        path_match = QUOTED.match(arguments)
        if not path_match:
            raise ValueError("nonliteral or truncated path")
        raw = path_match.group(1)
        if "\\" in raw:
            raise ValueError("escaped filename requires strict decoder")
        path = Path(raw)
        resolved = (path if path.is_absolute() else cwd_by_pid[pid] / path).resolve()
        if kind == "chdir":
            if returned.startswith("0"):
                if not resolved.is_dir():
                    raise ValueError("successful chdir to unavailable directory")
                cwd_by_pid[pid] = resolved
            continue
        if kind == "execve":
            if not returned.startswith("-1"):
                events.append({"type": "exec", "path": str(resolved)})
            continue
        if kind in ("open", "openat"):
            # Successful reads only. O_WRONLY and O_RDWR cannot be certified
            # as pure reads, so treat mixed accesses conservatively as inputs.
            if not returned.startswith("-1") and "O_WRONLY" not in arguments:
                events.append({"type": "read", "path": str(resolved)})
    if initial_pid is None:
        raise ValueError("empty syscall stream")
    if len(events) == 1:
        raise ValueError("no observable build input accesses")
    events.append({"type": "end"})
    return {"schema_version": 1, "kind": "build-access-events",
            "events": events, "monitor_completeness_proven": False}


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--trace", type=Path, required=True)
    p.add_argument("--cwd", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)
    a = p.parse_args(argv)
    try:
        result = normalize(a.trace.read_text(encoding="utf-8").splitlines(), a.cwd)
    except (OSError, ValueError) as exc:
        result = {"schema_version": 1, "kind": "build-access-events",
                  "status": "BLOCKED", "reason": str(exc)}
    a.output.parent.mkdir(parents=True, exist_ok=True)
    a.output.write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    return 0 if result.get("kind") == "build-access-events" and "events" in result else 1


if __name__ == "__main__":
    sys.exit(main())
