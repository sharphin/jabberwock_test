> 現行の実用テストは [コンパイラ別検査](compiler-safety-matrix.md) を参照。
> OSごとに12項目×2コンパイラ＋5目標×2。形式証明は別ツール。以下の旧混合集計は全コンパイラの合格を表さない。

# Linux用の安全性12項目

Linuxでビルド・実行した結果だけを、この12項目の証拠として記録する。
Windows版の12項目とは独立した検証セット。WSL2からの実行もこちらに分類する。
共通の受け入れ基準は [verification-plan.md](verification-plan.md)、実行方法全般は [local-safety.md](local-safety.md)。

```sh
python3 scripts/safety_linux.py
```

アップロードするファイル: **`target/safety-linux-report.json`**。
JSONの `suite.id` は `linux-12`、項目IDは `linux-01`〜`linux-12`。
`--scale N`、`--output PATH`、`--dry-run` を指定できる。
Linux以外の実行ホストでは検査開始前に終了する（`--dry-run` での計画表示は可能）。

## Linuxの12項目と実行内容

| 番号 | 項目 | Linuxで収集する証拠・限界 |
|---|---|---|
| 1 | 検査省略の解析 | AST/IR・native差分、型の通る入力、O1/O2、検査回数。Leanモデルは補助証拠 |
| 2 | 型の通る攻撃 | 攻撃ケース、異常入力、ASan/UBSan、退役検査、検出器の負例 |
| 3 | データ競合 | TSanの負例、スレッド・native・system・退役検査 |
| 4 | 生成LLVMの未定義動作 | LLVMの守り、変異入力、ASan/UBSan、CのUBSan負例 |
| 5 | 最適化後の動作 | O1/O2・LTOでの差分、検査回数、ASan付きのRust/jwc生成物 |
| 6 | 未初期化メモリ | MSanの負例、全体差分、jwc生成物のMSan付き差分 |
| 7 | 自己ホスト版 | jwcビルド、ASan/MSan付き生成物、O1/O2での比較。Leanモデルは補助証拠 |
| 8 | 資源枯渇 | RLIMIT_AS下でinterp・Rust native・jwc生成nativeの3ケース。全確保経路の網羅性は別途確認 |
| 9 | 巨大な配列型 | Rust/jwcの4入口で上限超過と上限ちょうどを計16ケース。項目8とは別に集計 |
| 10 | 仕様・型検査・副作用 | debug/releaseと通常終了・エラーまでの外部副作用。16構成の専用検査 |
| 11 | 所有権・回収・途中失敗 | 回収・退役・異常入力・副作用停止。全経路の失敗注入網羅性は別途確認 |
| 12 | Linuxの実環境・実行構成 | Linuxのdebug/release、nativeのO1/O2・LTO、ABI/リンク、各sanitizerの実行記録 |

## 必要な環境と結果の扱い

- Python 3.10以上、Rust/Cargo、clang、lld、ASan/UBSan/TSan/MSan runtime、bash。
- LeanモデルにはLean 4.24.0/lake。
- sanitizerの起動に失敗する環境では、その実行は失敗または未実行として残す。OS設定を自動で変更しない。
- 原則としてLinux x86_64で既存の検査構成を利用する。別アーキテクチャの対応状況は実行ログと受け入れ基準で判断する。

Windowsの成功結果を、この12項目の代わりとして取り込まない。
既知の欠陥や実行環境不足を自動で成功に変えない。
`PROBES_PASS_ONLY` は実行したプローブの成功であり、項目全体の認定は常に `acceptance: NOT_ASSESSED`。

## この変更の確認状況

作業分担に従い、変更担当者はビルド・テスト・clippy・Lean検査を実行していない。
実行器の変更、項目8/9の分離、レポート形式の変更はユーザー側での再テスト待ち。
