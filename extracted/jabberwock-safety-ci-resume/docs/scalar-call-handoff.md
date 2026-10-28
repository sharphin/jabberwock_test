# 数値などの直接呼び出し後に残る参照チェックの改善

Claude によるレビュー・修正のための引き継ぎ。調査・実装日: 2026-10-25。
基準コミット: `2229e23`（main）。この文書と同じブランチの差分が今回の変更。

## 変更内容

### 言語仕様の説明

`docs/spec.md` の7.2節で、同じ場所への複数の ref mut と読み取り用 ref が共存できることを明記した。
ref mut は書き換えの許可であり、Rust の排他的借用のような別名なしの保証ではない。
9.1節では「2つのスレッドが同じ値を指すことはない」という過度に広い説明を修正した。
chan は共有して同期し、送るデータは所有権の移動と古い ref の無効化で保護する。
文字列は送信時に複製し、変更されない定数は共有できる（`runtime/jw_rt.c` の `jw_str_unshare`）。

新しい文法・型・演算は追加していない。直前の調査で挙げた対象日・代入先・定義順と、const の未実装表記は、基準コミットまでに修正済みなので再変更しなかった。

### 速度の改善

従来は IR の `Rvalue::Call` を通るたびに、全 `RefElem`（リスト要素の参照が生存するという事実）を捨てていた。
今回、次の条件をすべて満たす直接呼び出しだけは、その事実を保持する。

1. 呼び出す関数が IR 上で特定できる。
2. 取り込みがない（`captures == 0`）。
3. 全引数と戻り値が整数・f64・bool・str・値なし・Never のいずれか。
4. 引数を評価するときに失われた事実は復活させない。

この条件を `scalar_call` として関数ごとに一度計算し、基本ブロックの固定点計算と、実際に Check を書き込む処理の両方に渡す。
ジェネリックな関数は単相化後の具体型で判定する。暗黙の this も引数として検査する。
自己ホスト版 `jwc/analyze.jw` にも同じ判定と事実の保持を実装した。

これは一般的な純粋関数解析ではない。関数内に print・エラー・ローカルなリスト操作があってもよい。
逆に、読むだけの ref、構造体、enum、own、配列、リスト、辞書、関数値、task、chan を渡す呼び出しは、影響がなくても今回の対象外。
関数値の呼び出しと interface の呼び出しも、従来どおり全 `RefElem` を消す。

## 安全性の根拠とレビューの要点

- 現在の言語には可変のグローバル変数や任意のポインタを作るキャストがない。この引数だけでは、呼び出し先が呼び出し元のリストに到達できない。
  関数内で作った参照・クロージャ・チャンネルも、呼び出し元のリストへ到達する値を最初に受け取っていなければそのリストを変更できない。
- str は言語上は値としてコピーされ、ref・own・リストを内部に格納できない。文字列の参照カウントや copy-on-write はリスト要素の生存には影響しない。
- 「リストを所有しない型」と「呼び出し元のリストを変更できない型」は別。既存の `may_hold_list` は ref に false を返すため流用してはいけない。
- 引数の中に `change(ref mut xs)` のような呼び出しがあれば、外側が数値の関数でも内側で事実が消える。`operands` の処理後に事実を新しく追加せず、残ったものを保持するだけ。
- `pinned` の引数判定、`is_live`、借用印、ref の4要素表現、世代の進め方、検査失敗の表示、AST の解析は変更していない。LLVM の noalias も追加していない。
- 未知の関数番号は安全側に扱う。Rust は `get`、jwc は範囲を調べてから表を読む。
- 関数が失敗する場合の出力と診断も保存する。呼び出しを移動・削除したり、エラーをループの前へ移したりしない。
- 将来、可変グローバル、FFI、ポインタを作るキャスト、隠れた実行コンテキストなどを追加したら、この証明を見直すこと。

## テスト

- `tests/ir.rs`: 直接呼ぶ数値・真偽・文字列・単相化した関数の後で、ループと分岐を越えて検査が消えること。
- 同ファイル: 引数評価中の push、ref mut の引数、コールバック、捕捉した参照、関数値の呼び出し後の push では検査が残り、ダングリングになること。
- 同ファイル: 数値関数内の0割りより前の出力と、診断全文が解析なしの場合と一致すること。
- `tests/borrow.rs`: 同じ場所への2つの ref mut と1つの ref が共存し、書き換えた値がすべてから見えること。
- `tests/jwc_cases.txt`: 直接呼び出しの保持、引数中の変更、コールバック経由の変更を追加。Rust と jwc の AST・型・解析前後の IR・LLVM を照合する既存テストが読む。
- デバッグビルドの run_source は interp と IR を自動照合する。`JW_NATIVE=1` でネイティブとも照合する。`JW_NATIVE=o2` は -O2 と LTO を用いる追加確認。

初回の全体テストは338成功・1失敗だった。失敗は jwc の `typed_and_ir_of_the_repository_files`。
Rust 版だけの改善で解析済み IR の完全一致が崩れたため、jwc に同じ変更を入れた。テストの比較を緩めたり、スキップしたりする解決はしていない。
最終実行は下記のとおり341成功・0失敗。jwc の9テストも成功した。

## 測定の再現

マイクロベンチマークは `bench/experiments/scalar-call.jw`。3億回、数値の補助関数を呼んでから同じリスト要素の ref を読み書きする。
通常の言語間比較スクリプトが列挙する `bench/jabberwock/` には入れず、他言語の対応ベンチマークを要求しない独立した実験にした。

変更前・変更後の release コンパイラを別々のパスに保存し、同じ clang・runtime・ソースで比較する。
今回の変更前バイナリは基準コミットの release build を変更前に保存したもの。

```sh
# リポジトリのルートで実行。BEFORE_JW と AFTER_JW には各コンパイラのパスを設定する。
mkdir -p /tmp/scalar-call-bench
"$BEFORE_JW" build --stats -o /tmp/scalar-call-bench/before bench/experiments/scalar-call.jw
"$AFTER_JW" build --stats -o /tmp/scalar-call-bench/after bench/experiments/scalar-call.jw
"$BEFORE_JW" build --count-checks -o /tmp/scalar-call-bench/before-count bench/experiments/scalar-call.jw
"$AFTER_JW" build --count-checks -o /tmp/scalar-call-bench/after-count bench/experiments/scalar-call.jw
/tmp/scalar-call-bench/before-count
/tmp/scalar-call-bench/after-count
```

期待する標準出力は両方 `151199992962`。計数版は時間測定に使わない。
時間は通常版をそれぞれ1回ウォームアップし、その後7組を交互順で実行する。

```sh
python3 - /tmp/scalar-call-bench/before /tmp/scalar-call-bench/after <<'PY'
import statistics, subprocess, sys, time
paths = dict(zip(('before', 'after'), sys.argv[1:]))
samples = {k: [] for k in paths}
for i in range(8):
    order = ('before', 'after') if i % 2 == 0 else ('after', 'before')
    for k in order:
        started = time.perf_counter()
        p = subprocess.run([paths[k]], capture_output=True, text=True, check=True)
        elapsed = time.perf_counter() - started
        assert p.stdout == '151199992962\n', p.stdout
        if i:
            samples[k].append(elapsed)
for k, values in samples.items():
    print(k, 'samples:', values)
    print('min/median/max:', min(values), statistics.median(values), max(values))
PY
```

比較中は他のテスト・ビルドを止める。周波数や CPU の割り当ては今回固定していないので、短い差は繰り返し確認する。
Windows と他の CPU での速度は未測定。clang が LTO に失敗すると通常 build は LTO なしへ戻るため、LTO の有無もそろえる。

## 検証結果

計測環境は Linux x86_64、AMD EPYC 9V45、Rust 1.90.0、Debian clang/lld 19.1.7。
clang はシステムへインストールせず、署名検証された Debian パッケージ索引に従い取得したパッケージを `/workspace/.setup/llvm` へ展開して使用した。
`/workspace/.setup/bin/clang` を PATH に加えるラッパーを使う。これはリポジトリの変更ではない。

確認コマンド:

```sh
export CARGO_HOME=/workspace/.cargo RUSTUP_HOME=/workspace/.rustup
export PATH=/workspace/.setup/bin:/workspace/.cargo/bin:$PATH
cargo +1.90.0 build --release --locked -j 4
JW_NATIVE=1 cargo +1.90.0 test --locked -j 4 --no-fail-fast -- --test-threads=4
JW_NATIVE=o2 cargo +1.90.0 test --locked --test ir --test borrow
```

| 検査数（scalar-call） | 変更前 | 変更後 |
|---|---:|---:|
| 世代チェックの箇所 | 2 | 0 |
| 解析で消した箇所 | 1 | 3 |
| 実行した世代チェック | 600,000,000 | 0 |
| 実行時に省略したチェック | 1 | 600,000,001 |
| 実行した添字チェック | 2 | 2 |

| 実行時間（秒、中央値 [最小–最大]） | 変更前 | 変更後 |
|---|---|---|
| scalar-call、7回 | 0.183816 [0.181432–0.184584] | 0.161268 [0.158096–0.164284] |
| 既存 refmut、5回 | 0.208221 [0.123310–0.213640] | 0.126324 [0.123580–0.215026] |
| 既存 nbody、5回 | 0.230930 [0.220396–0.239964] | 0.230402 [0.221960–0.241690] |

テスト・ビルドの終了後に計測した。各組で前後の順番を交互にし、ウォームアップを集計から除外した。
scalar-call はこの環境で約12.3%短縮した。これはこの形だけを強調するマイクロベンチマークであり、言語全体の高速化率ではない。
refmut と nbody は変更前後で生成 LLVM IR が完全一致した。refmut の中央値には大きな差が出たが、範囲が重なり、生成コードも同じなので改善の根拠にしない。
今回の最適化が効かないコードは、共有環境の測定ばらつきを見分ける対照として扱う。

丸めた各試行の秒数（上の表と同じ実行）:

```text
scalar-call before: 0.183816, 0.181432, 0.183946, 0.184426, 0.184584, 0.181740, 0.182928
scalar-call after:  0.161268, 0.161424, 0.159317, 0.164284, 0.158096, 0.161932, 0.160214
refmut before:      0.123310, 0.126733, 0.213640, 0.208260, 0.208221
refmut after:       0.123580, 0.124132, 0.132829, 0.126324, 0.215026
nbody before:       0.220396, 0.235458, 0.230930, 0.230900, 0.239964
nbody after:        0.241690, 0.222172, 0.221960, 0.230402, 0.235412
```

すべての試行で変更前後の標準出力が一致した。refmut は `34159`、nbody は `-169075163` と `-169083133` の2行。
LTO が使えることは、生成した LLVM IR と `runtime/jw_rt.c` を `clang -O2 -flto -fuse-ld=lld -Wno-override-module ... -pthread -lm` で明示的にリンクし、scalar-call の出力一致を確認した。

最終テスト結果:

- `cargo +1.90.0 build --release --locked -j 4`: 成功。
- `JW_NATIVE=1 cargo +1.90.0 test --locked -j 4 --no-fail-fast -- --test-threads=4`: 341成功・0失敗・0 ignored。
  clang 不在による早期終了ではない。jwc 9件、ネイティブ専用18件、既存の差分 fuzz も実行された。
- `JW_NATIVE=o2 cargo +1.90.0 test --locked --test ir --test borrow`: 28成功（IR 9・borrow 19）・0失敗。
- `git diff --check`: 成功。引き継ぎ文書・設計記録・bench README の相対リンクの参照先も確認した。
- 実行ログと未丸めの計測値は現在の環境の `/workspace/.setup/scalar-call-*.log` と
  `/workspace/.setup/scalar-call-bench/final-results.json` にある。環境外でレビューするときは上記の手順で取り直す。

## 未対応・レビューで見てほしいこと

1. 上記の到達不能の証明と、取り込み・暗黙の this を含む判定に抜けがないか。
2. 将来型が増えた場合に、デフォルトで対象外のままになること。参照を含む構造体を「小さい値だから」と対象へ追加しないこと。
3. Rust と jwc の2つの判定が同じ条件であること。今回のように IR の完全一致テストも必ず動かすこと。
4. jwc の Cx は従来の arity と同じく判定表をコピーする。関数数が多いと解析時間・一時メモリが増える可能性がある。コンパイラ全体の速度改善は主張しない。
5. 既存 refmut はもともと世代チェックが0回なので、その主要な遅さは今回解消しない。ref の生成時の借用印・表現削減、範囲解析、チャンネル、区画配置は未対応。
6. jwc を Rust 版でビルドして行う差分テストと、自己ホストの第2・第3世代のブートストラップは別。後者は今回の必須テストに含めていない。
7. Rust 1.65 の最低対応版、Windows、長時間の競合テストはこの環境では未検証。言語仕様の意味を変える必要が出たら、この最適化の拡張と混ぜずに判断する。
