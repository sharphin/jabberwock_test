#!/bin/sh
# run.sh の表に入らない測定（Markdown の表。$BENCH_OUT/extra.md にも書く）
#   1. ビルド時間と実行ファイルの大きさ（run.sh と同じフラグ。1 回ずつ）
#   2. 起動の時間（何もしないプログラム small/hello を RUNS_SMALL 回。既定 50）
#   3. スレッドの数を変えた並列計算（parsum を 1・2・4 スレッドで）
#   4. Go と C++ の速くした書き方（tuned/）と、ふつうの書き方の比べ
# 使い方: bench/extra.sh   （RUNS・JW・BENCH_OUT は run.sh と同じ）
set -e
cd "$(dirname "$0")"
OUT=${BENCH_OUT:-/tmp/jabberwock-bench}/extra
JW=${JW:-../target/release/jabberwock}
RUNS=${RUNS:-5}
RUNS_SMALL=${RUNS_SMALL:-50}
mkdir -p "$OUT"
cc -O2 -o "$OUT/measure" measure.c
M="$OUT/measure"
cell() { echo "$1" | awk '{ printf "%.3f [%.3f–%.3f]", $2, $1, $3 }'; }
med() { echo "$1" | awk '{ printf "%.3f", $2 }'; }
ms() { echo "$1" | awk '{ printf "%.2f", $2 * 1000 }'; }
kb() { stat -c %s "$1" | awk '{ printf "%.0f", $1 / 1024 }'; }
skb() { cp "$1" "$1.s" && strip "$1.s" && kb "$1.s"; }

# 1 回だけ作り、かかった秒を出す
build() { o=$1; shift; $M 1 /dev/null "$@" | awk '{ printf "%.2f", $2 }'; }

B="$OUT/build.md"
printf "| 名前 | 作る秒: jabberwock | Go | C++ | Rust | 大きさ KB: jabberwock | Go | C++ | Rust | strip 後 KB: jabberwock | Go | C++ | Rust |\n|---|---|---|---|---|---|---|---|---|---|---|---|---|\n" > "$B"
for f in jabberwock/*.jw small/hello.jw; do
    n=$(basename "$f" .jw); d=$(dirname "$f"); [ "$d" = jabberwock ] && d=.
    s=$d; [ "$d" = . ] && s=""
    tj=$(build x $JW build -o "$OUT/$n.jw" "$f")
    tg=$(build x go build -o "$OUT/$n.go" "${s:-go}/$n.go")
    tc=$(build x clang++ -O2 -std=c++17 -pthread -o "$OUT/$n.cpp" "${s:-cpp}/$n.cpp")
    tr=$(build x rustc -C opt-level=3 --edition 2021 -o "$OUT/$n.rs" "${s:-rust}/$n.rs")
    printf "| %s | %s | %s | %s | %s | %s | %s | %s | %s | %s | %s | %s | %s |\n" "$n" "$tj" "$tg" "$tc" "$tr" \
        "$(kb "$OUT/$n.jw")" "$(kb "$OUT/$n.go")" "$(kb "$OUT/$n.cpp")" "$(kb "$OUT/$n.rs")" \
        "$(skb "$OUT/$n.jw")" "$(skb "$OUT/$n.go")" "$(skb "$OUT/$n.cpp")" "$(skb "$OUT/$n.rs")" >> "$B"
    echo "build $n: 済み" >&2
done

S="$OUT/start.md"
printf "| | jabberwock | Go | C++ | Rust |\n|---|---|---|---|---|\n" > "$S"
printf "| hello（ミリ秒。$RUNS_SMALL 回の中央値） | %s | %s | %s | %s |\n" \
    "$(ms "$($M "$RUNS_SMALL" "$OUT/o" "$OUT/hello.jw")")" "$(ms "$($M "$RUNS_SMALL" "$OUT/o" "$OUT/hello.go")")" \
    "$(ms "$($M "$RUNS_SMALL" "$OUT/o" "$OUT/hello.cpp")")" "$(ms "$($M "$RUNS_SMALL" "$OUT/o" "$OUT/hello.rs")")" >> "$S"

P="$OUT/scale.md"
printf "| スレッド | jabberwock | Go | C++ | Rust |\n|---|---|---|---|---|\n" > "$P"
for t in 1 2 4; do
    printf "| %s | %s | %s | %s | %s |\n" "$t" "$(cell "$($M "$RUNS" "$OUT/o" "$OUT/parsum.jw" "$t")")" \
        "$(cell "$($M "$RUNS" "$OUT/o" "$OUT/parsum.go" "$t")")" "$(cell "$($M "$RUNS" "$OUT/o" "$OUT/parsum.cpp" "$t")")" \
        "$(cell "$($M "$RUNS" "$OUT/o" "$OUT/parsum.rs" "$t")")" >> "$P"
done
echo "scale: 済み" >&2

U="$OUT/tuned.md"
printf "| 名前 | jabberwock | Go | Go（速くした書き方） | C++ | C++（速くした書き方） |\n|---|---|---|---|---|---|\n" > "$U"
for f in tuned/go/*.go; do
    n=$(basename "$f" .go)
    go build -o "$OUT/$n.tgo" "$f"
    clang++ -O2 -std=c++17 -pthread -o "$OUT/$n.tcpp" "tuned/cpp/$n.cpp"
    $M 1 "$OUT/a" "$OUT/$n.tgo" > /dev/null; $M 1 "$OUT/b" "$OUT/$n.tcpp" > /dev/null; $M 1 "$OUT/c" "$OUT/$n.cpp" > /dev/null
    cmp -s "$OUT/a" "$OUT/c" || echo "!! $n: 速くした Go の出力が違う" >&2
    cmp -s "$OUT/b" "$OUT/c" || echo "!! $n: 速くした C++ の出力が違う" >&2
    printf "| %s | %s | %s | %s | %s | %s |\n" "$n" "$(med "$($M "$RUNS" "$OUT/o" "$OUT/$n.jw")")" \
        "$(med "$($M "$RUNS" "$OUT/o" "$OUT/$n.go")")" "$(med "$($M "$RUNS" "$OUT/o" "$OUT/$n.tgo")")" \
        "$(med "$($M "$RUNS" "$OUT/o" "$OUT/$n.cpp")")" "$(med "$($M "$RUNS" "$OUT/o" "$OUT/$n.tcpp")")" >> "$U"
done
{
    echo "### ビルド時間と実行ファイルの大きさ（秒は 1 回。KB は 1024 バイト）"; echo; cat "$B"; echo
    echo "### 起動の時間"; echo; cat "$S"; echo
    echo "### スレッドの数を変えた並列計算（parsum。秒。$RUNS 回の中央値 [最小–最大]）"; echo; cat "$P"; echo
    echo "### 速くした書き方（秒。$RUNS 回の中央値）"; echo; cat "$U"
} > "$OUT/extra.md"
cat "$OUT/extra.md"
