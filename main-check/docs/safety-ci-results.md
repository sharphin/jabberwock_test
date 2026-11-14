# 安全性テストの再開結果（2026-10-25）

## 対象と記録の区別

`claude/project-thread-yo9do9` の `eca370faa25b7f0ce0f802dbffe913ead6f33ece` を対象に、
`docs/safety-ci-handoff.md` の未完了テストから再開した。
今回の検証中に、処理系・ランタイム・テストのソースは変更していない。

引き継ぎ文書にある「TSan の thread leak 3件」「ASan の130本の失敗」「MSan の calc の指摘」は、
Claude が残した過去の記録であり、今回の実測ではない。修正前のコードでの再現試験は実施していない。
今回の結果は、`quit` / `_Exit` と `jw_view_target_nocheck` の修正を含む上記コミットでの実測である。
修正前後を同条件で比較した結果としては扱わない。

## 環境

- Linux x86_64、Debian 13 のクラウド環境。
- Rust / Cargo 1.99.1、Debian clang / lld 19.1.7、対応する compiler-rt。
- clang 関連パッケージは Debian の署名付き索引で検証して取得し、作業領域に展開。
- `CARGO_BUILD_JOBS=2`、`RUST_TEST_THREADS=2`。
- ASan/UBSan、TSan、MSan は単純な C プログラムでコンパイルと実行の成功を確認してから使用。
- `vm.mmap_rnd_bits` の参照はこの環境では拒否された。sysctl は変更せず実行できた。
- ログと終了コードは `/workspace/safety-results/<実行名>.log` / `<実行名>.exit` に保存。
  このパスは今回の環境内の記録で、リポジトリに含まれる成果物ではない。

## 集計

すべてのコマンドが終了コード0で完了した。今回のテスト失敗は0件。

| 実行（ログ名） | 成功 | 失敗 | 備考 |
|---|---:|---:|---|
| 通常テスト（`normal`） | 350 | 0 | ランナー集計。下記の早期returnの注意を参照 |
| 通常ネイティブ比較（`native`） | 350 | 0 | ランナー集計。下記の早期returnの注意を参照 |
| MSan 全体（`msan`） | 350 | 0 | ランナー集計。下記の早期returnの注意を参照 |
| ASan/UBSan 全体（`asan-all`） | 350 | 0 | ランナー集計。下記の早期returnの注意を参照 |
| TSan（native / threads / system）（`tsan`） | 39 | 0 |  |
| ASan/UBSan 堅牢性（`robust-asan`） | 6 | 0 | 変異入力150本、手書き攻撃16本、例、極端な入力、LLVMの守り |
| ASan/UBSan 変異入力のネイティブ実行（`robust-native`） | 1 | 0 | 変異入力100本。5テストはフィルタで対象外 |
| ASan/UBSan fuzz -O1（`asan-fuzz`） | 1 | 0 | 300本中278本を比較、22本は型エラー |
| ASan/UBSan fuzz -O2＋LTO（`asan-fuzz-o2`） | 1 | 0 | 100本中92本を比較、8本は型エラー |
| jwc 差分fuzz（`jwc-fuzz`） | 1 | 0 | 50本中49本を比較、1本は型エラー |
| Clippy all-targets（`clippy`） | 完了 | 0 | 警告なし |

350件の全体実行では、`sanitized_examples_are_clean` は環境変数の条件により早期returnする。
その検証は別の `robust-asan` 実行で実際に走らせて成功した。ランナーの `ignored` は全実行0件。
lib・bin・doc-testには0テストのターゲットがあるが、上記件数は実行済みintegration testの合計。

引き継ぎに記載された次の3件は、今回のTSan実行でそれぞれ成功した。

- `blocks_freed_on_other_threads`
- `compiled_programs_run_threads`
- `channel_misuse_is_an_error`

これは修正後のテスト成功であり、修正前の再現や、修正との因果関係を比較実験で確定したという意味ではない。

準備時にclangが未導入だったため、導入後にテストを開始した。未導入状態での空実行を成功件数には含めていない。

## 判定の範囲

`docs/safety-testing.md` の1〜7は目標とする検証範囲であり、CIの終了コードだけで完了とはしない。

| 項目 | 今回の確認方法 | なお残る範囲 |
|---|---|---|
| 1. 検査を消す解析 | ASan/UBSanの生成プログラム比較、解析の既存テスト | 全生成入力についてネイティブの解析あり・なしを突き合わせてはいない |
| 2. 型の通る攻撃 | fuzzの借用後の無効化操作、手書き攻撃、変異入力 | 操作ごとの実行到達率は未計測。型エラーや先行する実行時エラーで後続操作に届かないことがある |
| 3. データ競合 | TSanでnative・threads・systemの39テスト | 今回は1回の実行。毎晩の反復実行は別途必要 |
| 4. 生成LLVMの未定義動作 | 割り算・剰余・シフトの守りのテスト、sanitizer実行 | アドレス計算などは守りの形の検査対象外。C向けUBSanでは生成LLVM全体を検査できない |
| 5. 最適化 | ASan/UBSanの`-O2`＋LTO差分100本、robustのbuild経路 | 全入力・全最適化構成を網羅したものではない |
| 6. 未初期化メモリ | MSanで全体テスト350件成功 | 実行した経路に限る |
| 7. 自己ホスト版 | jwc生成物の差分50本、既存jwcテスト | robustの攻撃一式をjwcで走らせる検証は未実施。jwc生成物のsanitizer化も別途必要 |

したがって今回の結果は、引き継ぎ時点のテストを再実行した証拠であり、1〜7の未確認範囲がすべて解消したという判定ではない。

- fuzz は型エラーで実行に進まなかった入力と、実行比較した入力を分けて数える。
- 正常終了と、期待する言語エラーで停止したケースは、ともに比較が一致すればテスト成功。
  言語エラーの期待どおりの検出を sanitizer の失敗に数えない。
- `robust` は3秒の時間切れを成功扱いにする既存仕様。時間切れの本数は既存ランナーが報告しない。
  また `sanitized_examples_are_clean` は `JW_ROBUST_ASAN` がない実行では早期returnする。
- 生成LLVMの守りのテストは割り算・剰余・シフトが対象。アドレス計算などを網羅する証拠にはならない。
- 既存fuzzは解析ありの生成LLVMをインタプリタと比較する。すべての生成入力について
  ネイティブの解析あり・なしを比較するテストにはなっていない。
- jwcの差分テストは生成実行ファイルの出力・エラーを比較するが、jwcのbuildは
  `JW_CFLAGS` を使わないため、その生成物までsanitizer付きであるとは主張しない。
- GitHub Actionsのrunnerでの動作、毎晩20倍の試験、Windowsは今回のローカル検証とは別。

## 実行手順

作業ディレクトリはリポジトリのルート。下記の `cargo` は今回 `cargo +1.99.1` を使い、
すべてのテストコマンドに `--locked` を付けた。`JW_CLANG` と `PATH` で導入済みのclangとlldを選ぶ。

```sh
cargo test --no-run
cargo clippy --all-targets

JW_NATIVE=1 JW_CFLAGS='-fsanitize=thread' TSAN_OPTIONS=halt_on_error=1 \
  cargo test --no-fail-fast --test threads --test native --test system
JW_NATIVE=1 JW_CFLAGS='-fsanitize=memory -fno-omit-frame-pointer' MSAN_OPTIONS=halt_on_error=1 \
  cargo test --no-fail-fast

ASAN_FLAGS='-fsanitize=address,undefined -fno-sanitize-recover=undefined -fno-omit-frame-pointer'
JW_CFLAGS="$ASAN_FLAGS" JW_FUZZ=300 cargo test --test fuzz -- --nocapture
JW_NATIVE=o2 JW_CFLAGS="$ASAN_FLAGS" JW_FUZZ=100 JW_FUZZ_SEED=501 \
  cargo test --test fuzz -- --nocapture
JW_NATIVE=1 JW_CFLAGS="$ASAN_FLAGS" cargo test --no-fail-fast

JW_ROBUST_ASAN=1 cargo test --test robust -- --nocapture
JW_ROBUST=100 JW_ROBUST_NATIVE=1 JW_ROBUST_ASAN=1 \
  cargo test --test robust mutated -- --nocapture

cargo test --no-fail-fast
JW_NATIVE=1 cargo test --no-fail-fast
JW_FUZZ_JWC=/absolute/path/to/jwc JW_FUZZ=50 cargo test --test fuzz -- --nocapture
```

fuzz の種は指定がなければ1から、robustの変異入力も1から。
jwc差分検証には、通常の `tests/jwc.rs` が生成した `-O1` の自己ホストコンパイラをコピーして使用した。
fuzzでjwcが生成する実行ファイルのビルドは `jwc build` の `-O2`＋LTO の経路を使用する。
