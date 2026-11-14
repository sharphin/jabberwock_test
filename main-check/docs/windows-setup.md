# Windowsの永続設定と安全性テストの起動

PATHはユーザー設定へ保存し、MSVC/Windows SDKのINCLUDE・LIB・x64設定は毎回VsDevCmd.batから読み込む。
SDKの版番号を含むINCLUDE・LIBを永続保存すると更新後に古い版を参照するため、ここでは保存しない。

## 一度だけ行うPATH設定

LLVM、elan、Rustが導入済みの状態で、通常のPowerShellへ以下を貼り付ける。
`.ps1`ファイルやPowerShellプロファイルにはしない。管理者権限や実行ポリシーの変更は不要。
既存のユーザーPATHを保ち、実在する不足分だけ追加する。システムPATHには書き込まない。

```powershell
& {
$ErrorActionPreference = 'Stop'
$oldUserPath = [Environment]::GetEnvironmentVariable('Path', 'User')
$oldClang = [Environment]::GetEnvironmentVariable('JW_CLANG', 'User')
$backupDir = Join-Path $env:LOCALAPPDATA 'Jabberwock'
New-Item -ItemType Directory -Force -Path $backupDir | Out-Null
$backupFile = Join-Path $backupDir ('environment-' + [guid]::NewGuid() + '.json')
@{ Path = $oldUserPath; JW_CLANG = $oldClang } | ConvertTo-Json | Set-Content -Encoding UTF8 $backupFile

$extraPaths = @('C:\Program Files\LLVM\bin', "$env:USERPROFILE\.elan\bin", "$env:USERPROFILE\.cargo\bin")
$userParts = @($oldUserPath -split ';' | Where-Object { $_ })
foreach ($entry in $extraPaths) {
    if ((Test-Path -LiteralPath $entry -PathType Container) -and ($userParts -notcontains $entry)) {
        $userParts += $entry
    }
}
[Environment]::SetEnvironmentVariable('Path', ($userParts -join ';'), 'User')
if (Test-Path -LiteralPath 'C:\Program Files\LLVM\bin\clang.exe') {
    [Environment]::SetEnvironmentVariable('JW_CLANG', 'C:\Program Files\LLVM\bin\clang.exe', 'User')
    $env:JW_CLANG = 'C:\Program Files\LLVM\bin\clang.exe'
}
$env:Path = ($extraPaths -join ';') + ';' + $env:Path
Write-Host "設定前のバックアップ: $backupFile"
}
```

他の既存端末には反映されない。Windows TerminalやVS Codeはアプリごと開き直す。
保存先のJSONを使えば元のユーザーPath/JW_CLANGに戻せる。戻すと、その後追加した設定も戻るため内容を確認する。

```powershell
# 実際に表示されたバックアップファイルを指定する
$saved = Get-Content -Raw 'バックアップファイルの完全なパス' | ConvertFrom-Json
[Environment]::SetEnvironmentVariable('Path', $saved.Path, 'User')
[Environment]::SetEnvironmentVariable('JW_CLANG', $saved.JW_CLANG, 'User')
```

## 毎回の起動は1コマンド

リポジトリのルートで、通常のPowerShellまたはコマンドプロンプトから実行する。

```powershell
.\scripts\safety_windows.cmd
```

この起動ファイルは次を順に行う。

1. vswhereでC++ツールを含むVisual Studioを探す。見つからない場合は2022 BuildToolsの標準配置を確認する。
2. 実在するVsDevCmd.batを呼び、target/hostの両方がx64か確認する。
3. LLVM・elan・Cargoの検索先をそのプロセス内に追加する。
4. `py -3`でPython 3.10以上が実行できるか確認する。Windows Storeのpython実行エイリアスを使わない。
5. Cプログラムで標準ヘッダー・64ビット・LTO/lld・shell32と、64 KiBのvolatile配列によるスタック拡張処理のリンクを確認する。
6. 準備が通った場合だけ12項目のランナーを起動し、終了コードをそのまま返す。

初期化・基本リンクの失敗時はテストを開始せず終了コード2。リンク診断は `target/windows-toolchain-probe.log`。
同じログにclangの詳細、INCLUDE・LIB・LIBPATH・target/host設定を残す。空のmainのリンク成功だけではchkstk等のアーキテクチャ不一致を検出できない場合がある。
基本リンク成功はASanや12項目の合格ではない。環境準備後のレポートは `target/safety-windows-report.json`。
準備で止まった場合、そのJSONは新しく生成されないので、残っている古いJSONを今回の結果として提出しない。
起動ファイル自身はレジストリや親PowerShellの環境を変更しない。
ランナーの引数は渡せるが、`--dry-run`でも起動ファイルの環境準備・リンク確認は実行する。
コマンドを一切実行せず検査計画のみ見る場合は、直接 `py -3 scripts/safety_windows.py --dry-run` を使う。

標準外の配置では、JW_VSDEVCMDにVsDevCmd.bat、JW_CLANGにclang.exe、JW_PYTHONに実際のpython.exeの完全なパスを設定できる。
これらは引数を含めず、値自体に引用符を含めない。

## 今回確認された環境の問題

- `math.h`未検出: Windows SDK/UCRTのヘッダーが見つかっていない。
- `machine type x86 conflicts with x64`: x64生成物へx86ライブラリが選ばれている。
- `.ps1`の実行拒否: PowerShellの実行制限。この起動経路では.batを使う。
- `python`が「Python」とだけ表示して終了: 本物のPythonが起動していない可能性。`py -3`または実体パスを使う。
- `mainCRTStartup`未解決: CRTのリンクに失敗。x64初期化と基本リンクのログを確認する。

Windows用の項目3・4・6・8には未対応検査が残る。環境設定では解消しない。
この起動ファイルは静的レビューのみで、変更担当者によるWindows実行・テスト・ビルドは未実施。
