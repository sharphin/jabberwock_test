#!/usr/bin/env python3
"""Capture clang preprocessing evidence for runtime generation macros.

This is an opt-in, real compiler invocation, not a compiler-correctness proof.
The caller must supply the exact flags of the build being audited.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
RUNTIME = ROOT / "runtime" / "jw_rt.c"


def digest(path: Path) -> str:
    if path.is_symlink() or not path.is_file():
        raise ValueError("runtime source missing or symlinked")
    return hashlib.sha256(path.read_bytes()).hexdigest()


def capture(clang: str, flags: list[str], timeout: int) -> dict:
    if not clang or timeout < 1:
        raise ValueError("clang and positive timeout required")
    # All flags after -- are passed as literal argv tokens, never through shell.
    # Require caller to provide the actual build flags; no default JW_CFLAGS
    # inference is claimed to reflect an already produced binary.
    before = digest(RUNTIME)
    outputs = {}
    for kind, switches in (
        ("macros", ["-dM", "-E", "-x", "c"]),
        ("preprocessed", ["-E", "-P", "-x", "c"]),
    ):
        argv = [clang, *flags, *switches, str(RUNTIME)]
        result = subprocess.run(
            argv, cwd=ROOT, capture_output=True, timeout=timeout,
            check=False,
        )
        if result.returncode:
            raise ValueError(
                kind + " preprocessing failed (exit " + str(result.returncode)
                + "): " + result.stderr.decode("utf-8", "replace")[-600:]
            )
        outputs[kind] = {
            "argv": argv,
            "sha256": hashlib.sha256(result.stdout).hexdigest(),
            "bytes": len(result.stdout),
            "content": result.stdout.decode("utf-8", "replace"),
        }
    if digest(RUNTIME) != before:
        raise ValueError("runtime source changed during preprocessing")
    macros = {}
    for line in outputs["macros"]["content"].splitlines():
        for name in ("GEN_RETIRED", "GEN_COUNTER_MASK", "GEN_LIMIT", "JW_TEST_GENERATION_LIMIT"):
            prefix = "#define " + name + " "
            if line.startswith(prefix):
                if name in macros:
                    raise ValueError("duplicate expanded macro " + name)
                macros[name] = line[len(prefix):]
    for required in ("GEN_RETIRED", "GEN_COUNTER_MASK", "GEN_LIMIT"):
        if required not in macros:
            raise ValueError("missing expanded generation macro " + required)
    # Derive which branch clang actually selected, rather than trusting the
    # caller's claimed build mode. The -dM output is the active macro table.
    override = macros.get("JW_TEST_GENERATION_LIMIT")
    limit_definition = re.sub(r"\s+", "", macros["GEN_LIMIT"])
    if override is None:
        if limit_definition != "GEN_COUNTER_MASK":
            raise ValueError("production GEN_LIMIT does not expand to counter mask")
        branch = "production"
    else:
        if limit_definition != "((uint64_t)JW_TEST_GENERATION_LIMIT)":
            raise ValueError("override GEN_LIMIT differs from reviewed macro")
        branch = "override"
        # A valid C integer literal is required, not a self-referential or
        # arbitrary expression injected through build flags.
        literal = override.strip()
        if not re.fullmatch(r"(?:0[xX][0-9a-fA-F]+|[0-9]+)(?:[uU][lL]{0,2}|[lL]{1,2}[uU]?)?", literal):
            raise ValueError("unsupported generation limit override literal")
        value = int(re.sub(r"(?i)(?:ull|llu|ul|lu|ll|u|l)$", "", literal), 0 if literal.lower().startswith("0x") else 10)
        if not (3 <= value <= (1 << 62) - 1 and (value & 3) == 3):
            raise ValueError("generation override violates required bounds")
    # A mere occurrence of the identifier is not proof of a surviving
    # definition. Require the reviewed function signature in clang output.
    processed = outputs["preprocessed"]["content"]
    if len(re.findall(r"\bgen_has_room\s*\(\s*uint64_t\s+gen\s*,\s*uint64_t\s+step\s*\)\s*\{", processed)) != 1:
        raise ValueError("generation guard missing or ambiguous after preprocessing")
    # Do not claim that preprocessing preserved the expression merely
    # because the function name survived. Record the expanded body and leave
    # semantic authentication outstanding until a C frontend parses it.
    expanded_guard_bodies = re.findall(
        r"static\s+int\s+gen_has_room\s*\(\s*uint64_t\s+gen\s*,\s*"
        r"uint64_t\s+step\s*\)\s*\{([^{}]*)\}", processed
    )
    if len(expanded_guard_bodies) != 1:
        raise ValueError("preprocessed guard body missing or ambiguous")
    expanded_guard_body = expanded_guard_bodies[0].strip()
    if not re.fullmatch(r"return\s+[^;{}]+;", expanded_guard_body, re.DOTALL):
        raise ValueError("unsupported expanded guard body")
    return {
        "schema_version": 1,
        "kind": "runtime-generation-clang-preprocessing",
        "status": "PREPROCESSING_CAPTURED_REFINEMENT_NOT_PROVEN",
        "source": "runtime/jw_rt.c",
        "source_sha256": before,
        "clang": clang,
        "caller_supplied_flags": flags,
        "macros": macros,
        "active_generation_limit_branch": branch,
        "macro_output_sha256": outputs["macros"]["sha256"],
        "preprocessed_output_sha256": outputs["preprocessed"]["sha256"],
        "macro_output_bytes": outputs["macros"]["bytes"],
        "preprocessed_output_bytes": outputs["preprocessed"]["bytes"],
        "preprocessed_text": outputs["preprocessed"]["content"],
        "expanded_guard_body": expanded_guard_body,
        "compiler_refinement": "NOT_PROVEN",
        "limitations": [
            "Flags are supplied by the caller, not authenticated against an existing binary.",
            "Compiler version, driver internals, include dependencies and C semantics are not proven.",
            "Preprocessing evidence alone cannot establish semantic correspondence.",
        ],
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--clang", default="clang")
    parser.add_argument("--timeout", type=int, default=120)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("flags", nargs="*", help="compiler flags after --")
    args = parser.parse_args()
    flags = args.flags[1:] if args.flags[:1] == ["--"] else args.flags
    try:
        result = capture(args.clang, flags, args.timeout)
    except (ValueError, OSError, subprocess.TimeoutExpired) as exc:
        result = {"schema_version": 1, "kind": "runtime-generation-clang-preprocessing",
                  "status": "INVALID_EVIDENCE", "reason": str(exc),
                  "compiler_refinement": "NOT_PROVEN"}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(result["status"], "->", args.output, flush=True)
    return 0 if result["status"] == "PREPROCESSING_CAPTURED_REFINEMENT_NOT_PROVEN" else 2


if __name__ == "__main__":
    raise SystemExit(main())
