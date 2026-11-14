> 現行の実用テストは [コンパイラ別検査](compiler-safety-matrix.md) を参照。
> OSごとに12項目×2コンパイラ＋5目標×2。形式証明は別ツール。以下の旧混合集計は全コンパイラの合格を表さない。

# Windows用の安全性12項目

Windowsでビルド・実行した結果だけを、この12項目の証拠として記録する。
Linux版の12項目とは独立した検証セットで、WSL2での実行結果はWindowsの合格に充当しない。
共通の受け入れ基準は [verification-plan.md](verification-plan.md)、実行方法全般は [local-safety.md](local-safety.md)。

永続PATH設定とx64環境の準備は [windows-setup.md](windows-setup.md) を参照する。
通常のPowerShell/コマンドプロンプトから、環境準備を含む入口を利用できる。

```powershell
.\scripts\safety_windows.cmd
```

すでに環境を準備した端末で直接起動する場合は `py -3 scripts/safety_windows.py`。

アップロードするファイル: **`target/safety-windows-report.json`**。
JSONの `suite.id` は `windows-12`、項目IDは `windows-01`〜`windows-12`。
`--scale N`、`--output PATH`、`--dry-run` を指定できる。
実行ホストがWindowsでなければ、検査開始前に終了する（`--dry-run` での計画表示は可能）。

## Windowsの12項目と実行内容

| 番号 | 項目 | Windowsで収集する証拠・残る不足 |
|---|---|---|
| 1 | 検査省略の解析 | AST/IR・native差分、O1/O2、型の通る入力、ASan。Leanモデルは補助証拠 |
| 2 | 型の通る攻撃 | 攻撃ケース、ASanとその負例、変異入力、native退役検査 |
| 3 | データ競合 | Windowsスレッド関連の実行＋暫定Intel Inspector接続。未導入ならNOT_RUN |
| 4 | 生成LLVMの未定義動作 | 暫定UBSanの負例・正常ケース＋LLVM opt verifier。対応ツール/実行環境が必要 |
| 5 | 最適化後の動作 | Windows nativeのO1/O2、LTO、Rust/jwcの生成コードのASan付き差分 |
| 6 | 未初期化メモリ | 暫定Dr. Memoryの未初期化負例・正常ケース。未導入ならNOT_RUN |
| 7 | 自己ホスト版 | Windowsでjwcをビルドし、Rust/jwcの生成コードをASan付きで比較。Leanモデルは補助証拠 |
| 8 | 資源枯渇 | Windowsのメモリ制限・確保失敗注入プローブは未対応でNOT_RUN |
| 9 | 巨大な配列型 | Rust/jwcのtyped・IR raw・IR・LLVMの4入口。上限超過と上限ちょうどを計16ケース確認 |
| 10 | 仕様・型検査・副作用 | debug/release、通常終了とエラーまでの副作用が1回かをCLIとRust/jwc nativeで確認 |
| 11 | 所有権・回収・途中失敗 | 既存の回収・退役・スレッド・攻撃検査、nativeの生存数、副作用の途中停止。全経路の失敗注入網羅性は別途確認 |
| 12 | Windowsの実環境・実行構成 | Windowsのdebug/release、生成nativeのO1/O2・LTO、ABI/リンク、スレッド、ASanの実行記録 |

**3・4・6・8には未対応の検査があるため、現時点のWindows用セットは全項目成功で終了しない。**
その項目で別の検査が失敗すればFAIL、実行できた検査が成功しても未対応が残ればNOT_RUNとする。
例えばスレッドの通常実行が成功しても、競合が検出できることを確かめた扱いにはしない。
これは12項目への分離とWindows用の実行内容の整理であり、未対応検査の実装完了を意味しない。

## 必要な環境

- Python 3.10以上、Rust/Cargo、clang、Cリンカ・Windows SDK、LTOに使うlld。
- Windows向けASan runtime。`-fsanitize=address` を利用し、WindowsでのUBSan/TSan/MSanの同等性を仮定しない。
- Leanモデルを確認する場合はLean 4.24.0/lake。プロジェクトは `formal/lean-toolchain` で版を固定する。
- 実行に必要なツールは、同じPowerShellのPATHから見えること。clangは `JW_CLANG` で指定可能。

LeanはPowerShellからelan（Leanのバージョン管理ツール）を導入する。

```powershell
winget install --id Lean.Elan --exact --source winget
```

PowerShellを開き直し、プロジェクト指定の版とコマンドを確認する。
Windows TerminalやVS Code内の端末は、アプリごと終了して開き直す。

```powershell
elan default leanprover/lean4:v4.24.0
lean --version
lake --version
```

`lake` もLeanのツールチェーンに含まれる。
`lake` が見つからない場合は `winget list --id Lean.Elan --exact` で導入済みか確認する。
導入済みでPATHが反映されていない場合、現在のPowerShellには次で登録済みPATHを追加できる。

```powershell
$env:Path += ";" + [Environment]::GetEnvironmentVariable("Path", "User") + ";" + [Environment]::GetEnvironmentVariable("Path", "Machine")
Get-Command elan, lean, lake
```

配布元: [elan](https://github.com/leanprover/elan)、[WinGetのLean.Elan定義](https://github.com/microsoft/winget-pkgs/tree/master/manifests/l/Lean/Elan)。

ASanのランタイムがない、負例が検出されない、リンクできない等の結果は失敗として残す。
ASanの検査に失敗したとき、検査を外して成功に置き換える動作はしない。
`PROBES_PASS_ONLY` は実行したプローブの成功であり、項目全体の認定は常に `acceptance: NOT_ASSESSED`。

### 合意した基準（agreed、2026-10-30）

各項目と 5 目標には、`acceptance` とは別の欄 `agreed` がある（`MET` / `NOT_MET`）。
条件は `scripts/safety_goals.py` の `AGREED`。項目ごとに必要な probe を決め、全部 `PROBE_PASS` のときだけ `MET`。
`coverage_gaps` は消さずに残し、`acceptance` は `BLOCKED` のまま（34 の義務の全体の受け入れとは別の判定）。
01 は `jwc-elision-certificate`（Lean の検査器で消した検査を確かめ直す。lake が要る）、
10 は `*-spec-coverage`（`tests/spec_coverage.json` の、spec.md の節ごとの正しい例・拒否される例）。
12 はこのレポートの OS の分だけ。12 全体は Linux と Windows（オーナーの実機）の両方のレポートで `MET` のとき。

## この変更の確認状況

作業分担に従い、変更担当者はビルド・テスト・clippy・Lean検査を実行していない。
Windows向けに追加したASanプローブ、巨大配列・副作用検査も、ユーザー側での再テスト待ち。

ASanのDLL起動失敗については [診断コマンドとJSON項目](windows-asan-diagnostics.md) を参照。通常の実用レポートにも実読み込みパスと要求関数の照合結果を格納する。

項目3・4・6の暫定構成とツール指定方法は [暫定検出器](provisional-detectors.md)。後で専用検出器を実装して置き換える。
