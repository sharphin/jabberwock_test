#!/usr/bin/env python3
"""Kernel-check the model corresponding to the reviewed real emitter templates.

The source extractor is a trusted boundary: this certifies the semantics of
its *recognized template*, not arbitrary output of either compiler or LLVM.
"""
from __future__ import annotations
import argparse
import json
import shutil
import subprocess
import tempfile
from pathlib import Path

from proof_emitter_correspondence import validate

ROOT = Path(__file__).resolve().parents[1]
FORMAL = ROOT / "formal"

LEAN = """import JabberwockFormal.EmittedGate

set_option autoImplicit false
set_option warningAsError true

namespace JabberwockFormal.ExtractedEmitterGate

open JabberwockFormal.EmittedGate

/-- LLVM icmp ne, icmp eq -1, or, and reject-if-true, extracted
    from the narrowly validated templates in both compiler sources. -/
def extractedReject (saved current : Word) : Bool :=
  decide (current ≠ saved) || decide (current = retired)

def extractedContinue (saved current : Word) : Bool :=
  !extractedReject saved current

theorem extracted_matches_gate (saved current : Word) :
    extractedReject saved current = llvmBad saved current := by
  rfl

theorem extracted_continue_iff_valid (saved current : Word) :
    extractedContinue saved current = true ↔ valid saved current := by
  change llvmContinues saved current = true ↔ valid saved current
  exact llvm_continues_iff saved current

theorem extracted_reject_iff_invalid (saved current : Word) :
    extractedReject saved current = true ↔ ¬ valid saved current := by
  change llvmBad saved current = true ↔ ¬ valid saved current
  exact llvm_bad_iff saved current

end JabberwockFormal.ExtractedEmitterGate
"""
# Turn the two operations into Lean using the same bitvector operations as
# the compiler template. The extractor must refuse changes to either source.
# The Lean model uses decidable propositions for 64-bit equality.
def verify(timeout: int) -> dict:
    checks = [validate("jwc"), validate("rust")]
    lake = shutil.which("lake")
    if lake is None:
        raise RuntimeError("lake is not installed or unavailable on PATH")
    with tempfile.TemporaryDirectory(prefix="jabberwock-emitter-") as folder:
        path = Path(folder) / "ExtractedEmitterGate.lean"
        path.write_text(LEAN, encoding="utf-8")
        done = subprocess.run([lake, "env", "lean", str(path)],
                              cwd=FORMAL, capture_output=True, text=True,
                              timeout=timeout, check=False)
        if done.returncode:
            raise RuntimeError("Lean kernel rejected emitter semantics: " +
                               (done.stderr + "\n" + done.stdout)[-4000:])
    return {
        "schema_version": 1, "kind": "emitter-template-semantics",
        "status": "KERNEL_CHECKED_TEMPLATE_MODEL_ONLY",
        "compilers": checks,
        "checked_obligations": [
            "Extracted reject predicate agrees for all 64-bit values",
            "Continuation iff valid non-retired generation for all 64-bit values",
            "Reject iff invalid generation for all 64-bit values",
        ],
        "compiler_refinement": "NOT_PROVEN",
        "limitations": [
            "Source-to-model extraction recognizes only one exact checked LLVM template.",
            "No proof of arbitrary compiler-generated LLVM or runtime semantics.",
            "No LLVM pointer provenance, memory safety, or concurrency proof.",
        ],
    }

def main() -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--output", required=True, type=Path)
    p.add_argument("--timeout", type=int, default=600)
    args = p.parse_args()
    try:
        if args.timeout < 1:
            raise ValueError("timeout must be positive")
        result = verify(args.timeout)
    except (RuntimeError, OSError, ValueError, subprocess.TimeoutExpired) as exc:
        result = {"schema_version": 1, "kind": "emitter-template-semantics",
                  "status": "INVALID_EVIDENCE", "reason": str(exc)}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(result["status"], flush=True)
    return 0 if result["status"] == "KERNEL_CHECKED_TEMPLATE_MODEL_ONLY" else 2

if __name__ == "__main__":
    raise SystemExit(main())
