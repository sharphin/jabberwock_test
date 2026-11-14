#!/usr/bin/env python3
"""証明の義務 `jwc:checks:jwc/analyze.jw` に、消した検査の検査器（elision-certificate-v1）の結果をつなぐ。

これは部分の確かめ（translation validation）で、義務そのものは証明しない。
- 今のソースから jwc を作り直し（Rust 版を使う。Rust 版の正しさは前提）、
- jwc が出した解析のあとの IR を、Lean で作った検査器（正しさは ElisionCheck.lean で証明済み）で確かめ直し、
- 記録（evidence）の結果と一字一句同じかを見る。
通っても REFINEMENT_NOT_PROVEN のまま（確かめたのはコーパスの各プログラムで、全プログラムについての analyze の正しさではない）。

使い方（evidence を作る）: python3 scripts/elision_bridge.py --rust target/release/jabberwock --output <evidence.json>
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
OBLIGATION = "jwc:checks:jwc/analyze.jw"
VERIFIER = "elision-certificate-v1"


def replay(rust: Path, timeout: int = 1800) -> dict:
    """今のソースから jwc を作り、全コーパスを検査器にかけた結果（elision_check.py の summary）"""
    with tempfile.TemporaryDirectory(prefix="jw-elision-bridge-") as temporary:
        folder = Path(temporary)
        jwc = folder / ("jwc" + (".exe" if os.name == "nt" else ""))
        built = subprocess.run([str(rust), "build", "--release", "-o", str(jwc), str(ROOT / "jwc" / "main.jw")],
                               cwd=ROOT, capture_output=True, text=True, timeout=timeout, stdin=subprocess.DEVNULL)
        if built.returncode != 0 or not jwc.is_file():
            raise ValueError("jwc を作れません: " + (built.stderr or built.stdout).strip()[-500:])
        summary = folder / "summary.json"
        checked = subprocess.run([sys.executable, str(ROOT / "scripts" / "elision_check.py"), "--jwc", str(jwc), "--build",
                                  "--out", str(folder / "work"), "--output", str(summary)],
                                 cwd=ROOT, capture_output=True, text=True, timeout=timeout, stdin=subprocess.DEVNULL)
        if checked.returncode not in (0, 1) or not summary.is_file():
            raise ValueError("検査器が動きません: " + (checked.stdout + checked.stderr).strip()[-500:])
        return json.loads(summary.read_text(encoding="utf-8"))


def check(obligation: dict, record: dict, run=None) -> dict:
    """refinement_transition.status_for から呼ぶ。部分の確かめなので REFINEMENT_PROVEN は返さない"""
    if obligation.get("id") != OBLIGATION:
        return {"status": "INVALID_EVIDENCE", "reason": "elision certificate attached to unrelated obligation"}
    rust = record.get("rust_compiler")
    summary = record.get("summary")
    if not isinstance(rust, str) or not Path(rust).is_file() or not isinstance(summary, dict):
        return {"status": "INVALID_EVIDENCE", "reason": "elision certificate requires rust_compiler and summary"}
    try:
        current = (run or replay)(Path(rust))
    except (OSError, ValueError, subprocess.TimeoutExpired) as exc:
        return {"status": "INVALID_EVIDENCE", "reason": "elision certificate replay failed: " + str(exc)}
    if current != summary:
        return {"status": "INVALID_EVIDENCE", "reason": "elision certificate differs from independent replay"}
    if current.get("status") != "VERIFIED" or not any(r.get("id") == "jwc/main.jw" for r in current.get("results", [])):
        return {"status": "REFINEMENT_NOT_PROVEN",
                "reason": "independent replay did not verify every elided check (" + str(current.get("status")) + ")"}
    return {"status": "REFINEMENT_NOT_PROVEN",
            "reason": "per-program elision certificates verified over the corpus; analyze refinement for all programs is outstanding",
            "verified_subset": "jwc.analyze:elided-generation-checks:corpus",
            "programs_verified": current.get("checked")}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--rust", type=Path, required=True)
    ap.add_argument("--output", type=Path, required=True)
    a = ap.parse_args()
    sys.path.insert(0, str(ROOT / "scripts"))
    from refinement_transition import REQUIRED
    from whole_compiler_proof import inventory
    row = next(o for o in inventory()["components"] if o["id"] == OBLIGATION)
    summary = replay(a.rust.resolve())
    evidence = {"schema_version": 1, "proofs": [{
        "obligation_id": OBLIGATION, "source_sha256": row["source_sha256"],
        "required_claims": sorted(REQUIRED), "verifier": VERIFIER,
        "rust_compiler": str(a.rust.resolve()), "summary": summary}]}
    a.output.parent.mkdir(parents=True, exist_ok=True)
    a.output.write_text(json.dumps(evidence, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(summary.get("status"), "->", a.output)
    return 0 if summary.get("status") == "VERIFIED" else 1


if __name__ == "__main__":
    sys.exit(main())
