#!/usr/bin/env python3
"""Regenerate the standard library embedded by jwc (std/*.jw -> jwc/stdlib.jw), without building or running tests."""
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def render(sources: dict) -> str:
    out = (
        "// 標準ライブラリ（std/*.jw）を文字列にしたもの。`use std.名前;` で modules.jw が読む。\n"
        "// 手で書き換えない: std/ を直したら `python3 scripts/sync_stdlib.py` で作り直す。\n"
        "pub fn str[] names() {\n    let str[] mut out = [];\n"
    )
    for name in sources:
        out += '    out.push("' + name + '");\n'
    out += "    out\n}\n\npub fn Option<str> source(str name) {\n"
    for name, source in sources.items():
        body = source.replace("\\", "\\\\").replace("{", "{{").replace("}", "}}")
        out += '    if name == "' + name + '" {\n        return Some("""\n' + body + '""");\n    }\n'
    return out + "    None\n}\n"


def main() -> None:
    sources = {}
    for path in sorted((ROOT / "std").glob("*.jw")):
        sources[path.stem] = path.read_text(encoding="utf-8").replace("\r\n", "\n")
    target = ROOT / "jwc/stdlib.jw"
    with target.open("w", encoding="utf-8", newline="\n") as output:
        output.write(render(sources))
    print("Updated jwc/stdlib.jw (no tests run)")


if __name__ == "__main__":
    main()
