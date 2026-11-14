#!/usr/bin/env python3
"""Regenerate the runtime embedded by jwc build, without building or running tests."""
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def render(source: str) -> str:
    body = source.replace("\\", "\\\\").replace("{", "{{").replace("}", "}}")
    return (
        "// 実行時ライブラリ（runtime/jw_rt.c）を文字列にしたもの。`jwc build` が書き出して一緒にコンパイルする。\n"
        "// 手で書き換えない: runtime/jw_rt.c を直したら `python3 scripts/sync_runtime.py` で作り直す。\n"
        'pub fn str source() {\n    """\n' + body + '"""\n}\n'
    )


def main() -> None:
    source = (ROOT / "runtime/jw_rt.c").read_text(encoding="utf-8")
    target = ROOT / "jwc/runtime.jw"
    # Keep the generated file LF-only, including on Windows.
    with target.open("w", encoding="utf-8", newline="\n") as output:
        output.write(render(source))
    print("Updated jwc/runtime.jw (no tests run)")


if __name__ == "__main__":
    main()
