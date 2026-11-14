# Windows実機の検証環境セットアップ

## 自動セットアップ（先に実行）

Windows PowerShellでリポジトリルートから次を実行します。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\setup_lean_windows.ps1
```

このスクリプトは `formal/lean-toolchain` のLean 4.24.0を確認し、必要ならwingetでelanを導入、Leanツールチェーン取得、`lake build`、`scripts/check_formal.py` による公理監査を順番に行います。
結果は `target/lean-setup/status.json` と `target/lean-setup/setup.log` に保存します。
環境構築後にすでにelanがある環境でインストールを禁止したいときは `-SkipInstall` を指定します。
ネットワークに接続できず必要なツールチェーンが未導入なら失敗します。成功しても実コンパイラ全体の証明完了は意味しません。


この手順はWindows x64用です。環境の構築と、34項目の正式受入合格は別物です。

## 1. Windows x64に公式ツールをインストール

PowerShellを開き、次を一行ずつ実行します。提供元を確認してください。

~~~powershell
winget install -e --id Git.Git
winget install -e --id Python.Python.3.12
winget install -e --id Rustlang.Rustup
winget install -e --id Lean.Elan
winget install -e --id LLVM.LLVM
winget install -e --id GoLang.Go
~~~

wingetで見つからなければwinget searchでIDを確認し、公式配布元を利用してください。
Visual Studio InstallerからBuild Tools for Visual Studio 2022の Desktop development with C++、MSVC x64/x86、Windows SDKをインストールしてください。
新しいPowerShellまたはx64 Native Tools Command Promptを起動してください。

## 2. バージョン固定のLeanを導入

リポジトリのルートで以下を実行します。

~~~powershell
cd formal
elan toolchain install leanprover/lean4:v4.24.0
lake env lean --version
lake build
cd ..
~~~

Lean 4.24.0以外なら中断してください。Leanの実行ファイルがPATHにない場合は、現在のPowerShellセッション内だけで $env:PATH = "$HOME\.elan\bin;$env:PATH" を実行してください。
Rustについては $env:PATH = "$HOME\.cargo\bin;$env:PATH" です。

## 3. 環境診断と実機検証

リポジトリルートで以下を順に実行してください。

~~~powershell
py scripts/toolchain_preflight.py
py scripts/check_formal.py
cargo test
cargo clippy --all-targets
py scripts/verify_all_local.py
py scripts/package_verification.py
~~~

環境診断は target/toolchain-preflight.json、全体判定は target/verification-complete/summary.json、共有候補の証拠は target/verification-evidence.zip に保存されます。
未証明の義務が残る限り verify_all_local.py は非ゼロ終了し得ます。非ゼロと実行不能、テスト失敗、数学的未証明を区別してください。

## 4. ログに基づく修正

summary.json の stages と evidence を読み、失敗工程のstdout/stderrログを照合します。
ZIPにはローカルパス、秘密情報等が含まれることがあるため、共有前に内容を確認・除去してください。
WindowsのASanには適合するclang runtime DLLが必要です。Intel InspectorやDr. Memoryなど未導入の暫定検出器はNOT_RUNとして保持し、合格に変更しません。

公式資料:
- https://lean-lang.org/install/manual/
- https://github.com/leanprover/vscode-lean4/blob/master/vscode-lean4/media/guide-installLean-windows.md
- https://learn.microsoft.com/ja-jp/windows/dev-environment/rust/setup
- https://rust-lang.github.io/rustup/installation/windows.html
