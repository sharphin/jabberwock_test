# 形式検証の実行コマンド

リポジトリ直下で実行する。Python 3.10以上、elan/lake、Lean 4.24.0が必要。
実用安全性テストとは別ツールで、コンパイラごとに別の証明モジュールを検査する。
初回はelanが固定版のLeanをダウンロードする場合がある。

## Windows（PowerShell）

```powershell
# 実用検査：jwc → Rust、それぞれ12項目＋5目標
.\scripts\safety_windows.cmd

# 形式検証：jwcの実装済みの局所定理
py -3 scripts/formal_report.py --compiler jwc --output target/formal-jwc-report.json

# 形式検証：Rust版の実装済みの局所定理
py -3 scripts/formal_report.py --compiler rust --output target/formal-rust-report.json

# 両コンパイラの形式検証を一括実行し、一つのJSONに保存
py -3 scripts/formal_report.py --compiler all --output target/formal-report.json
```

## Linux

```sh
python3 scripts/safety_linux.py
python3 scripts/formal_report.py --compiler jwc --output target/formal-jwc-report.json
python3 scripts/formal_report.py --compiler rust --output target/formal-rust-report.json
# 両方をまとめる場合
python3 scripts/formal_report.py --compiler all --output target/formal-report.json
```

共有抽象モデル（世代/退役/所有位置）の追加監査は `--abstract-model` を指定する。
コンパイラ全体の5目標を必須条件にした判定は `--require-full` を指定する。

```powershell
py -3 scripts/formal_report.py --compiler all --abstract-model --require-full --output target/formal-full-report.json
```

## レポートの完全性と限界

レポートはschema 3。各コンパイラのG1〜G5について、証明モジュールの帰属、登録定理の欠落・重複・不正な完全修飾名、対象ソースhashの整合を検査する。`summary` に目標件数、局所カーネル検査済み件数、処理系全体の証明済み件数を明示する。これは**登録内容の整合性検査**であり、形式モデルが言語仕様に十分対応することの証明ではない。

`--require-full`は現段階で常に未完了（終了1）とする。将来、実装対応の機械的証拠とG1〜G5の充足を検証するロジックが実装されるまで、この判定を緩めない。

## 結果と終了コード

- `proof_check: KERNEL_CHECKED`: このソースhashに紐付けた局所モデルの定理がLeanのカーネル監査を通過。
- `STALE`: 対象の実装ソースhashが変わり、モデルとの対応の見直しが必要。hashを自動更新して成功扱いにはしない。
- `FAIL` / `INFRA_ERROR`: 証明不成立・Lean不足など。未検査を成功にしない。
- `acceptance: NOT_PROVEN`: 実装全体との対応証明と全言語の保証は未完成。

通常の終了0は、選択した実装済み局所定理の検査成功のみを表す。
終了1は検査失敗・古いhash・未実装、または `--require-full` の全体証明未完了。
環境エラーは2、中断は130。現状 `--require-full` は終了1となる。

## 実装した証明の範囲

`JabberwockFormal.Jwc.ReferenceAccess` と `JabberwockFormal.Rust.ReferenceAccess` は
それぞれ8定理を持つ。両版の成功を代用せず、別々にビルド・監査する。
64ビットの世代値、all-onesの退役マーカー、比較と分岐、read-only権限、所有位置moveを扱う。

|5目標|今回の局所定理|
|---|---|
|意味保存|世代比較のBoolean決定とValid仕様の一致|
|メモリ安全性|モデル上のpayload読出し成功には世代検査成功が必要|
|型安全性|モデル上のread-onlyアクセスの書込み拒否、書込み成功時の権限|
|所有権・参照|checkedMove後も所有位置の一意性を保持|
|実行時チェック|世代検査の必要十分条件、同世代でも退役済みなら拒否|

jwcモデルは `jwc/llvm.jw`、Rustモデルは `src/ir/llvm.rs` の世代比較の意図を手動で形式化したもの。
元のコードをLeanへ自動抽出したものではない。ソースhashは変更検出であり、実装との同値性の証明ではない。
所有位置moveの補題は共有抽象モデルを依存として使用するが、各コンパイラの定理入口を独立に監査する。
実ポインタの生存・メモリ配置・並行実行・FFI・全LLVM意味論・言語全体の型保存/進行は未証明。
この変更では定理本体を実装したが、エージェントによるLean実行はしていない。
