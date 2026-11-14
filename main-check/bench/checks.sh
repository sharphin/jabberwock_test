#!/bin/sh
# 検査の数の見張り（CI 用）。各ベンチマークを `--stats` と `--count-checks` で作り、
# 書き出した検査の数と実行時に通った回数を checks.txt と比べる。増えていたら失敗する（解析が弱くなった印）。
# 時間ではなく数を見るので、共有の CI のマシンでもぶれない。
# 使い方: bench/checks.sh            （比べる）
#         UPDATE=1 bench/checks.sh   （checks.txt を書き直す。減ったときや、ベンチマークを足したとき）
set -e
cd "$(dirname "$0")"
OUT=${BENCH_OUT:-/tmp/jabberwock-bench}/checks
JW=${JW:-../target/release/jabberwock}
mkdir -p "$OUT"
NOW="$OUT/now.txt"
: > "$NOW"
for f in jabberwock/*.jw; do
    n=$(basename "$f" .jw)
    if ! "$JW" build --stats -o "$OUT/$n" "$f" >"$OUT/$n.stats.log" 2>&1; then
        echo "ERROR: compiler failed to collect statistics for $n" >&2
        exit 1
    fi
    st=$(sed -n 's/^\[stats\] 世代の検査 \([0-9]*\) か所・解析で消した \([0-9]*\) か所・添字の検査 \([0-9]*\) か所$/\1 \2 \3/p' "$OUT/$n.stats.log")
    if ! "$JW" build --count-checks -o "$OUT/$n.count" "$f" >"$OUT/$n.build.log" 2>&1; then
        echo "ERROR: compiler failed to instrument $n" >&2
        exit 1
    fi
    if ! "$OUT/$n.count" > /dev/null 2>"$OUT/$n.count.log"; then
        echo "ERROR: instrumented program failed for $n" >&2
        exit 1
    fi
    cnt=$(sed -n 's/^\[checks\] //p' "$OUT/$n.count.log" | awk '{ print $2, $4, $6 }')
    if ! printf "%s\n" "$n $st $cnt" | awk 'NF != 7 { exit 1 } { for (i=2; i<=7; i++) if ($i !~ /^[0-9]+$/) exit 1 }'; then
        echo "ERROR: missing or malformed check counts for $n" >&2
        exit 1
    fi
    echo "$n $st $cnt" >> "$NOW"
done
if [ -n "${UPDATE:-}" ]; then
    { echo "# 名前 場所:世代 場所:消した 場所:添字 実行時:世代 実行時:消した 実行時:添字（bench/checks.sh が書く）"; cat "$NOW"; } > checks.txt
    cat checks.txt
    exit 0
fi
# Validate that no benchmark was skipped or silently produced empty metrics.
awk -f check_counts.awk checks.txt "$NOW"
