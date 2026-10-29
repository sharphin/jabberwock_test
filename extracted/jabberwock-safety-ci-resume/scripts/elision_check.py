#!/usr/bin/env python3
"""検査を消した `*` を、Lean の検査器（formal の `elision-check`）で 1 本ずつ確かめ直す。

jwc が解析のあとの IR を JSON で書き出し（`jwc --ir-cert`）、検査器が読む。検査器の正しさは
formal/JabberwockFormal/ElisionCheck.lean で証明してある（docs/elision-proof.md）。

対象: examples/*.jw、tests/jwc_cases.txt の各プログラム、tests/safety/*.jw、bench/jabberwock/*.jw、jwc 自身（jwc/main.jw）。
型のエラーなどで IR まで行かないプログラムは数えるだけ（SKIPPED）。

使い方: python3 scripts/elision_check.py --jwc <jwc の実行ファイル> [--out <フォルダ>]
結果: 全部通れば VERIFIED（終了コード 0）、通らない関数があれば NOT_VERIFIED（1）、検査器や jwc が動かなければ TOOL_ERROR（2）。
"""
import argparse
import json
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent


def programs(out: pathlib.Path):
    """(名前, ソースのファイル) を並べる。jwc_cases.txt は 1 本ずつファイルに書き出す"""
    items = []
    for p in sorted((ROOT / "examples").glob("*.jw")):
        items.append(("examples/" + p.name, p))
    cases = (ROOT / "tests" / "jwc_cases.txt").read_text(encoding="utf-8").split("#####\n")
    case_dir = out / "cases"
    case_dir.mkdir(parents=True, exist_ok=True)
    for i, src in enumerate(cases):
        p = case_dir / f"case{i:04}.jw"
        p.write_text(src, encoding="utf-8")
        items.append((f"jwc_cases#{i}", p))
    for d in ["tests/safety", "bench/jabberwock"]:
        for p in sorted((ROOT / d).glob("*.jw")):
            items.append((f"{d}/{p.name}", p))
    items.append(("jwc/main.jw", ROOT / "jwc" / "main.jw"))
    return items


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--jwc", required=True)
    ap.add_argument("--out", default=str(ROOT / "target" / "elision-check"))
    ap.add_argument("--checker", default=str(ROOT / "formal" / ".lake" / "build" / "bin" / "elision-check"))
    a = ap.parse_args()
    out = pathlib.Path(a.out)
    (out / "json").mkdir(parents=True, exist_ok=True)
    checker = pathlib.Path(a.checker)
    if not checker.is_file():
        print(f"TOOL_ERROR: 検査器がありません（cd formal && lake build elision-check）: {checker}")
        return 2
    emitted, skipped = [], []
    for name, src in programs(out):
        dst = out / "json" / (name.replace("/", "_").replace("#", "_") + ".json")
        try:
            r = subprocess.run([a.jwc, "--ir-cert", str(src)], capture_output=True, text=True, timeout=600)
        except (OSError, subprocess.TimeoutExpired) as e:
            print(f"TOOL_ERROR: jwc が動きません: {name}: {e}")
            return 2
        if r.returncode != 0 or not r.stdout.startswith("{"):
            skipped.append(name)
            continue
        dst.write_text(r.stdout, encoding="utf-8")
        emitted.append((name, dst))
    failed = []
    for name, dst in emitted:
        r = subprocess.run([str(checker), str(dst)], capture_output=True, text=True, timeout=3600)
        if r.returncode == 0 and "VERIFIED" in r.stdout:
            continue
        if r.returncode == 1:
            failed.append((name, r.stderr.strip()))
        else:
            print(f"TOOL_ERROR: 検査器が止まりました: {name}: {r.stderr.strip()[:500]}")
            return 2
    status = "VERIFIED" if not failed else "NOT_VERIFIED"
    summary = {"status": status, "checked": len(emitted), "skipped": len(skipped),
               "failed": [{"program": n, "why": w} for n, w in failed], "skipped_programs": skipped}
    (out / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2), encoding="utf-8")
    for n, w in failed:
        print(f"NOT_VERIFIED: {n}\n  {w}")
    print(f"{status}: 確かめた {len(emitted)} 本、IR まで行かず数えただけ {len(skipped)} 本、通らない {len(failed)} 本"
          f"（{out / 'summary.json'}）")
    return 0 if not failed else 1


if __name__ == "__main__":
    sys.exit(main())
