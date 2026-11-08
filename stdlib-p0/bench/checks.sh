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
    st=$($JW build --stats -o "$OUT/$n" "$f" 2>&1 | sed -n 's/^\[stats\] 世代の検査 \([0-9]*\) か所・解析で消した \([0-9]*\) か所・添字の検査 \([0-9]*\) か所$/\1 \2 \3/p')
    $JW build --count-checks -o "$OUT/$n.count" "$f" > /dev/null 2>&1
    cnt=$("$OUT/$n.count" 2>&1 > /dev/null | sed -n 's/^\[checks\] //p' | awk '{ print $2, $4, $6 }')
    echo "$n $st $cnt" >> "$NOW"
done
if [ -n "$UPDATE" ]; then
    { echo "# 名前 場所:世代 場所:消した 場所:添字 実行時:世代 実行時:消した 実行時:添字（bench/checks.sh が書く）"; cat "$NOW"; } > checks.txt
    cat checks.txt
    exit 0
fi
# 世代の検査と添字の検査が増えたら失敗。「消した」は増えてよい（減ったら失敗）
awk 'NR == FNR { if ($1 !~ /^#/) base[$1] = $0; next }
     {
         if (!($1 in base)) { print "!! " $1 ": checks.txt にない（UPDATE=1 で足す）"; bad = 1; next }
         split(base[$1], b, " ")
         if ($2 > b[2] || $4 > b[4] || $5 > b[5] || $7 > b[7] || $3 < b[3] || $6 < b[6]) {
             print "!! " $1 ": 前 [" base[$1] "] 今 [" $0 "]"; bad = 1
         } else print "ok " $0
     }
     END { exit bad }' checks.txt "$NOW"
