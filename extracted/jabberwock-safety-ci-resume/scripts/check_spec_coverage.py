#!/usr/bin/env python3
"""10 の合意した基準: docs/spec.md の節ごとに、正しい例と拒否される例が少なくとも 1 つずつあり、全部が正解どおりに動くか。

表は tests/spec_coverage.json。正解は Rust 版のインタプリタ（`jabberwock <file>`）の出力・エラーの文・終了コード。
確かめる側（--subject）のコンパイラで実行ファイルを作って動かし、標準出力・標準エラー・成功/失敗が正解と一字一句同じかを見る
（作るときの警告・エラーも標準エラーに含める。「作りました: 」の行だけ除く）。
- ok は正解が成功（終了コード 0）、reject は正解が失敗（エラーの文がある）でなければ、表の誤りとして fail。
- accept:<path> は実行せず、両方が IR まで受け付けることだけを見る（複数ファイルのプログラムのため）。
- spec.md の節が表にも除外の一覧にもない、ok か reject が空、ケースが見つからない、は表の誤りとして fail。

使い方: python3 scripts/check_spec_coverage.py --subject jwc --compiler <jwc> --rust <jabberwock> --output <json>
終了コード: 0 全部 pass、1 fail がある、2 道具が動かない。
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def spec_sections():
    """spec.md の `## 1.`・`### 3.2.1` の見出しの番号"""
    found = []
    for line in (ROOT / "docs" / "spec.md").read_text(encoding="utf-8").splitlines():
        m = re.match(r"#{2,3} (\d+(?:\.\d+)*)\.? ", line)
        if m:
            found.append(m.group(1))
    return found


def jwc_cases():
    cases = (ROOT / "tests" / "jwc_cases.txt").read_text(encoding="utf-8").split("#####\n")
    table = {}
    for c in cases:
        table.setdefault(hashlib.sha256(c.encode("utf-8")).hexdigest()[:12], []).append(c)
    return table


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--subject", choices=("jwc", "rust"), required=True)
    ap.add_argument("--compiler", type=Path, required=True)
    ap.add_argument("--rust", type=Path, required=True)
    ap.add_argument("--output", type=Path, required=True)
    ap.add_argument("--table", type=Path, default=ROOT / "tests" / "spec_coverage.json")
    a = ap.parse_args()
    clang = os.environ.get("JW_CLANG", "clang")
    report = {"subject": a.subject, "results": [], "infrastructure_errors": [], "table_errors": []}

    def save():
        a.output.parent.mkdir(parents=True, exist_ok=True)
        a.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

    def run(command, timeout=300):
        p = subprocess.run(list(map(str, command)), cwd=ROOT, capture_output=True, text=True,
                           encoding="utf-8", errors="replace", timeout=timeout, stdin=subprocess.DEVNULL)
        return p.returncode, p.stdout, p.stderr

    def record(case_id, passed, observed):
        report["results"].append({"id": case_id, "compiler": a.subject, "status": "pass" if passed else "fail",
                                  "observed": observed})

    try:
        table = json.loads(a.table.read_text(encoding="utf-8"))
        covered = {s["section"] for s in table["sections"]} | {s["section"] for s in table["excluded"]}
        for missing in [s for s in spec_sections() if s not in covered]:
            report["table_errors"].append("spec.md の節 " + missing + " が表にありません")
        cases = jwc_cases()
        with tempfile.TemporaryDirectory(prefix="jw-spec-coverage-") as temporary:
            folder = Path(temporary)
            for entry in table["sections"]:
                section = entry["section"]
                for kind in ("ok", "reject"):
                    refs = entry.get(kind, [])
                    if not refs:
                        report["table_errors"].append("節 " + section + " に " + kind + " の例がありません")
                    for ref in refs:
                        case_id = section + ":" + kind + ":" + ref
                        if ref.startswith("accept:"):
                            path = ROOT / ref[len("accept:"):]
                            want = run([a.rust, "--ir", path])
                            got = run([a.compiler, "--ir", path]) if a.subject == "jwc" else want
                            record(case_id, kind == "ok" and path.is_file() and want[0] == 0 and got[0] == 0,
                                   {"reference_exit": want[0], "subject_exit": got[0]})
                            continue
                        source = cases.get(ref[len("jwc_cases:"):]) if ref.startswith("jwc_cases:") else None
                        if not source or len(source) != 1:
                            report["table_errors"].append("ケース " + ref + " が見つからないか、ひとつに決まりません")
                            continue
                        path = folder / (ref.replace(":", "_") + ".jw")
                        path.write_text(source[0], encoding="utf-8")
                        want = run([a.rust, path])
                        exe = folder / (ref.replace(":", "_") + (".exe" if os.name == "nt" else ""))
                        build = ([a.compiler, "build", "--clang", clang, "-o", exe, path] if a.subject == "jwc"
                                 else [a.compiler, "build", "-o", exe, path])
                        built = run(build, 600)
                        log = "".join(l for l in built[2].splitlines(True) if not l.startswith("作りました: "))
                        if built[0] == 0:
                            ran = run([exe])
                            got = (ran[0], ran[1], log + ran[2])
                        else:
                            got = (built[0], built[1], log)
                        # 正解そのものが ok / reject の約束どおりか（表の誤りを見つける）
                        shaped = (want[0] == 0) if kind == "ok" else (want[0] != 0 and "error:" in want[2])
                        same = (want[0] == 0) == (got[0] == 0) and want[1] == got[1] and want[2] == got[2]
                        record(case_id, shaped and same,
                               {"reference": {"exit": want[0], "stdout": want[1][-2000:], "stderr": want[2][-2000:]},
                                "subject": {"exit": got[0], "stdout": got[1][-2000:], "stderr": got[2][-2000:]},
                                "reference_matches_kind": shaped})
        for message in report["table_errors"]:
            record("table:" + message, False, message)
    except (OSError, ValueError, KeyError, subprocess.TimeoutExpired) as error:
        report["infrastructure_errors"].append({"error": str(error)})
        save()
        return 2
    save()
    failed = [r["id"] for r in report["results"] if r["status"] != "pass"]
    for f in failed:
        print("fail: " + f, flush=True)
    print(f"spec coverage ({a.subject}): {len(report['results']) - len(failed)} pass, {len(failed)} fail", flush=True)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
