# ベンチマーク運用手順（safety-ci-resume）

## 目的と対象

現行の主計測器は `scripts/benchmark_report.py`。セルフホストの `jwc` が書き出したネイティブ実行ファイルを、Rust (`rustc`)、C++ (`clang++`)、Go (`go build`) と同一ケースで比較する。

Rust製 Jabberwock コンパイラは `jwc` の初回ビルドに利用するが、比較対象には含めない。各言語の成果物を4者で比較する。セキュリティ検査の成否や言語全体の優劣をベンチマークの数値から導かない。

## 一括実行

リポジトリのルートから実行する。LinuxまたはWSL2を基本とする。

```sh
python3 scripts/benchmark_report.py
python3 scripts/benchmark_report.py --full
```

先頭は簡易5ケース、`--full` は全21ケース。既定は各ケース8反復。自動で jwc を構築する構成のため、初回実行にはコンパイル環境が必要。

```sh
# 既存のセルフホスト jwc を利用
python3 scripts/benchmark_report.py --full --jwc path/to/jwc

# 対象と反復数を限定
python3 scripts/benchmark_report.py --cases fib sort --runs 7

# 同一条件の過去結果と性能回帰を比較
python3 scripts/benchmark_report.py --full --baseline path/to/previous-benchmark-report.json
```

Windows（PowerShell、Visual Studio/Clang x64環境、構築済みjwcを使用する例）:

```powershell
.\scripts\benchmark_windows.cmd --full --jwc .\target\release\jwc-local.exe
```

必要な実行環境は Python 3、Clang/Clang++、Rust (`rustc`)、Go。Linuxでは `cc` と `ld.lld` も必要。jwc の自動構築時には `cargo` も必要。Windowsでは開発用ツールチェーンが利用できる必要がある。Windowsでの全ケース完走は未確認。

## 計測と判定

- **対象**: `jwc`、`rust`、`cpp`、`go` の4種類。通常の安全性チェックを省いた `--unchecked` 等は主比較に含めない。
- **時間**: 各反復を測定し、最小値・中央値・最大値（秒）を記録。
- **メモリ**: 最大RSS（Linux、`wait4`）またはピークワーキングセット（Windows）。異なるOSのメモリ指標を同じ値として直接比較しない。
- **順序**: 実行順を反復ごとに交替。8反復なら4者がそれぞれ先頭を2回担当。反復回数が4の倍数でない場合、レポートに部分的な均衡であることを記録。
- **PASS**: 全反復で出力が安定し、4者の標準出力SHA-256が一致したときのみ。空出力、異常終了、無効な計測値、出力不一致は成功扱いしない。
- **速度比**: `jwcの中央値 ÷ 比較対象の中央値`。1より大きい値はjwcが遅く、1未満は速い。
- **回帰比較**: `--baseline` で環境・ワークロード・フラグ・反復数等が比較可能であることを確認し、閾値超過を記録。環境が異なるレポートを比較可能と偽らない。

レポートは `target/benchmark-report.json`。実行環境、バージョン、フラグ、ソースのハッシュ、使用したjwcとランタイムの識別情報、ケース別の測定結果・出力ハッシュを保存する。実行中にも結果が更新される。

## 旧方式と安全性チェックの数

`bench/run.sh` はRust製Jabberwockコンパイラも含めた旧方式であり、**現行セルフホストjwcの主比較に使用しない**。歴史的な測定表は `bench/README.md` に残す。

`bench/checks.sh` は速度計測器ではない。世代・添字のチェック挿入数と実行回数を `bench/checks.txt` と比較する別の回帰監視器。既存の `.github/workflows/bench-checks.yml` は手動起動のこの検査を提供する。

```sh
# Rust製コンパイラのリリースビルド後に検査数を確認
cargo build --release
bench/checks.sh
```

`UPDATE=1 bench/checks.sh` は比較基準を書き換えるため、意図した基準更新以外では使わない。

## 判定の限界と現状

この文書は**実行方法と判定の仕様**であり、現行ブランチで全21ケースが成功したという記録ではない。最新の実測値と正否は、各環境で生成されたJSONを参照すること。異なるコンパイラの標準ライブラリ、アルゴリズム、最適化器、並列実装の差は残る。

形式検証基盤は別の `python scripts/formal_tool.py` が入口で、ベンチマークのPASSを形式証明のPROVENへ変換しない。形式検証の実装完了と実装の安全性証明も別に判断する。

詳しいケース一覧と既存の測定記録は [bench/README.md](../bench/README.md)、形式検証ツールの完成基準は [formal-tool-completion.md](formal-tool-completion.md) を参照。

## 保存した測定記録

- [2026-10-29：Linux・全21ケース、各8回、全件PASS](benchmarks/2026-10-29.md)。測定条件・結果JSON・ソース識別情報を保存。
