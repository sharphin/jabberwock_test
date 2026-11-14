# 実用テスト1〜12の追加実行

2026-10-26。対象のコンパイラ本体は`9250e0e75d87eecaae4830fafcdf6769b6652236`時点。
今回追加したのは副作用の検査script・fixture・記録で、本体の修正は行っていない。
環境はローカルLinux、Rust 1.99.1、Clang 19.1.7。
結果・コンパイラの指紋・ケース別の観測値は [機械可読記録](practical-retest-2026-10-26.json)。

## 実行結果

| 検証 | 結果 |
|---|---|
| コンパイラ自体のreleaseビルドで全体試験 | `cargo test --release --no-fail-fast`、runner集計366件成功・失敗0 |
| 外部効果の回数 | 16ケース中12成功・4不合格、基盤エラー0。script終了1 |
| release CLIで項目8/9を再テスト | 19ケース中9成功・10不合格、基盤エラー0。script終了1 |

全体試験は`JW_NATIVE=1 JW_REQUIRE_RETIREMENT_TESTS=1`を指定したが、
`run_inner`の自動AST→IR/native照合はdebug専用なので、releaseではその経路は実行されない。
明示的にnativeを作るnative/fuzz/jwc/退役テストは実行された。
366というrunnerの集計を、366件全てのAST/IR/native比較が行われたという意味にはしない。
今回の副作用scriptは、releaseコンパイラのLLVM生成からnative実行までを明示的に行う。

## 項目10: 外部効果を一度だけ起こす

新しい [script](../scripts/retest_practical_effects.py) は、
[正常終了](../tests/safety/effect-once.jw) と [効果の後に無効な参照で停止](../tests/safety/effect-before-error.jw) を調べる。
ファイルを読んで`x`を1文字追加するため、同じ内容で上書きする試験では見えなかった二重実行を検出できる。
エラーケースでは、エラー後の追加操作が行われないことも確認する。

| 実行構成 | 正常終了 | エラー終了 |
|---|---|---|
| debug CLI・検査あり | `xx`、不合格 | `xx`、不合格 |
| debug CLI・検査省略あり | `xx`、不合格 | `xx`、不合格 |
| release CLI・検査あり | `x`、成功 | `x`、成功 |
| release CLI・検査省略あり | `x`、成功 | `x`、成功 |
| Rust版生成native・O1 | `x`、成功 | `x`、成功 |
| Rust版生成native・O2＋LTO | `x`、成功 | `x`、成功 |
| jwc生成native・O1 | `x`、成功 | `x`、成功 |
| jwc生成native・O2＋LTO | `x`、成功 | `x`、成功 |

各ケースは別の一時ファイルを使い、期待値は1文字の`x`、標準出力なし、終了0または規定の参照エラー終了1。
正常終了のnativeでは`[live] 0`も確認する。通常のCLI実行を確認するため、子プロセスの`JW_NATIVE`は解除する。
今回のscriptは計装なしのLinux用であり、Windowsやsanitizerで成功したことにはしない。
この2種類の入力で項目10/11/12全体が完成したとは扱わない。

```sh
cargo build
cargo build --release
python3 scripts/retest_practical_effects.py --jwc /path/to/current/jwc --output /tmp/practical-effects.json
python3 scripts/retest_safety_edges.py --compiler target/release/jabberwock --jwc /path/to/current/jwc --output /tmp/safety-8-9-release.json
```

scriptは全ケース成功0、動作不一致1、コンパイラ不足等の検証基盤エラー2。
コンパイラ不足が終了2になることも確認した。現状の不合格をexpected-passに置き換えない。

## 残る状態

- 8: releaseでもinterpのメモリエラー終了1と、Rust/jwc nativeの内部エラー終了2が一致せず不合格。
- 9: releaseでも具体化後に1 MiBを超える配列を8経路が受け付け、不合格。上限ちょうどの8経路は成功。
- 10: debug CLIの再実行による二重の外部効果が残り、不合格。
- 11: releaseで`locals_are_freed_on_break_and_continue`、`question_mark_frees_what_was_built_so_far`、
  genericのown回収・task破棄・送受信等の既存試験は成功。全経路の台帳と制御された失敗注入は未完。
- 12: Rustコンパイラ自身のrelease実行を追加した。全構成の照合が完了したわけではなく、現変更のWindowsとLSanは未検証。

この追加実行の終了を「1〜12全項目合格」とは報告しない。
対象commitのGitHub [test](https://github.com/sharphin/jabberwock/actions/runs/40261928238) と
[safety](https://github.com/sharphin/jabberwock/actions/runs/40261928153) は全ジョブが実行ステップ0・runner未割当で終了している。
ローカル成功をGitHub Actionsの成功へ読み替えない。形式証明の結果は [専用様式](formal-verification.md) で別に管理する。
