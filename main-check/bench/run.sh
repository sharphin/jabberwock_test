#!/bin/sh
# jabberwock build・Go・C++（clang++ -O2）・Rust（rustc -C opt-level=3）で同じ処理を走らせて比べる
# 使い方: bench/run.sh [名前...]   （名前を省くと全部）
#   RUNS=回数（既定 5）・JW=jabberwock の場所・BENCH_OUT=作業フォルダ
# 出すもの（Markdown の表。$BENCH_OUT/results.md にも書く）:
#   1. 時間: 中央値の秒と [最小–最大]。jabberwock は 3 通り（ふつう・検査を全部残す `--no-elide`・検査を全部消す `--unchecked`）、
#      C++ は 2 通り（ふつう・境界チェックつき `-D_GLIBCXX_ASSERTIONS`）
#   2. 最大メモリ（最大 RSS、MB）
#   3. 検査の数: 書き出した場所の数（`--stats`）と、実行時に通った回数（`--count-checks` で作った別の実行ファイル）
set -e
cd "$(dirname "$0")"
OUT=${BENCH_OUT:-/tmp/jabberwock-bench}
JW=${JW:-../target/release/jabberwock}
RUNS=${RUNS:-5}
mkdir -p "$OUT"
cc -O2 -o "$OUT/measure" measure.c
M="$OUT/measure"
names=${*:-$(ls jabberwock | sed 's/\.jw$//')}

# measure の 1 行（最小 中央値 最大 RSS）を「中央値 [最小–最大]」にする
cell() { echo "$1" | awk '{ printf "%.3f [%.3f–%.3f]", $2, $1, $3 }'; }
rss() { echo "$1" | awk '{ printf "%.1f", $4 }'; }

T="$OUT/time.md"; R="$OUT/rss.md"; C="$OUT/checks.md"
printf "| 名前 | jabberwock | 検査を全部残す | 検査を全部消す | Go | C++ | C++（境界チェック） | Rust |\n|---|---|---|---|---|---|---|---|\n" > "$T"
printf "| 名前 | jabberwock | Go | C++ | Rust |\n|---|---|---|---|---|\n" > "$R"
printf "| 名前 | 場所: 世代 | 場所: 解析で消した | 場所: 添字 | 実行時: 世代 | 実行時: 解析で消した | 実行時: 添字 |\n|---|---|---|---|---|---|---|\n" > "$C"
for n in $names; do
    st=$($JW build --stats -o "$OUT/$n.jw" "jabberwock/$n.jw" 2>&1 | sed -n 's/^\[stats\] //p')
    $JW build --no-elide -o "$OUT/$n.all" "jabberwock/$n.jw" > /dev/null 2>&1
    $JW build --unchecked -o "$OUT/$n.none" "jabberwock/$n.jw" > /dev/null 2>&1
    $JW build --count-checks -o "$OUT/$n.count" "jabberwock/$n.jw" > /dev/null 2>&1
    go build -o "$OUT/$n.go" "go/$n.go"
    clang++ -O2 -std=c++17 -pthread -o "$OUT/$n.cpp" "cpp/$n.cpp"
    clang++ -O2 -std=c++17 -pthread -D_GLIBCXX_ASSERTIONS -o "$OUT/$n.chk" "cpp/$n.cpp"
    rustc -C opt-level=3 --edition 2021 -o "$OUT/$n.rs" "rust/$n.rs"
    w=$($M "$RUNS" "$OUT/out.jw" "$OUT/$n.jw")
    a=$($M "$RUNS" "$OUT/out.all" "$OUT/$n.all")
    z=$($M "$RUNS" "$OUT/out.none" "$OUT/$n.none")
    g=$($M "$RUNS" "$OUT/out.go" "$OUT/$n.go")
    c=$($M "$RUNS" "$OUT/out.cpp" "$OUT/$n.cpp")
    k=$($M "$RUNS" "$OUT/out.chk" "$OUT/$n.chk")
    r=$($M "$RUNS" "$OUT/out.rs" "$OUT/$n.rs")
    for v in jw all none go chk rs; do
        cmp -s "$OUT/out.$v" "$OUT/out.cpp" || echo "!! $n: 出力が違う $v=[$(cat "$OUT/out.$v")] c++=[$(cat "$OUT/out.cpp")]" >&2
    done
    # 数える版は 1 回だけ（数えるぶん遅いので時間は見ない）
    cnt=$("$OUT/$n.count" 2>&1 > /dev/null | sed -n 's/^\[checks\] //p')
    printf "| %s | %s | %s | %s | %s | %s | %s | %s |\n" "$n" "$(cell "$w")" "$(cell "$a")" "$(cell "$z")" "$(cell "$g")" "$(cell "$c")" "$(cell "$k")" "$(cell "$r")" >> "$T"
    printf "| %s | %s | %s | %s | %s |\n" "$n" "$(rss "$w")" "$(rss "$g")" "$(rss "$c")" "$(rss "$r")" >> "$R"
    sg=$(echo "$st" | sed -n 's/^世代の検査 \([0-9]*\) か所・解析で消した \([0-9]*\) か所・添字の検査 \([0-9]*\) か所$/\1 | \2 | \3/p')
    cg=$(echo "$cnt" | awk '{ printf "%s | %s | %s", $2, $4, $6 }')
    printf "| %s | %s | %s |\n" "$n" "$sg" "$cg" >> "$C"
    echo "$n: 済み" >&2
done
{
    echo "### 時間（秒。$RUNS 回の中央値 [最小–最大]）"; echo; cat "$T"; echo
    echo "### 最大メモリ（最大 RSS、MB。$RUNS 回の最大）"; echo; cat "$R"; echo
    echo "### 世代と添字の検査の数"; echo; cat "$C"
} > "$OUT/results.md"
cat "$OUT/results.md"
