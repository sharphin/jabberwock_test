# Windows ASan起動失敗の診断

通常の実用テストに組み込み済み。リポジトリ直下で実行する。

```powershell
.\scripts\safety_windows.cmd
```

`target/safety-windows-report.json` の `results` → `windows-asan-preflight` →
`case_report.results[].diagnostics` に診断を丸ごと格納する。このJSON一つをアップロードすればよい。
別ファイル `target/safety-windows-report-work/windows-asan-diagnostic.json` にも保存する。

環境設定済みのx64 Developer端末で、診断だけ行う場合:

```powershell
py -3 scripts/check_windows_asan.py --output target/windows-asan-diagnostic.json
```

## 記録内容

- 元の実行の終了コード（10進・16進）、stdout/stderr。
- `loader_trace.modules`: 診断用プロセスをWindowsデバッグAPIで再実行し、LOAD_DLL_DEBUG_EVENTのファイルハンドルから取得した実際のDLLパス。PATH候補と区別する。
- `loader_trace.exceptions` / `debug_strings`: ローダー例外、取得できたデバッグ文字列、診断プロセスの終了コード、トレース完了の有無。
- `pe_analysis.modules`: 対象exeと観測したDLLのSHA256、machine、要求DLL名と関数名/ordinal。
- `pe_analysis.missing_exports`: 実際に観測した同名DLLが要求関数を公開していない組み合わせ。要求元・要求関数・読み込み先を記録。
- `unresolved_imports` / `errors`: API-setの別名、取得失敗、転送先未観測、同名DLL複数等。推測で読み込み先を埋めない。

PEのimport/exportは標準ライブラリで静的に読み取る。公開関数の転送は観測済みDLLまで追跡する。
API-setのホスト解決、delay-load、動的GetProcAddressの全経路は未対応。
Windowsデバッガ起動権限がない場合や途中でローダーが停止した場合も、取得できた情報と失敗理由を残す。
診断用再実行は通常実行とは異なるため、元の実行結果と診断プロセスの結果を別に保持する。
この仕組みでも不足関数を特定できなかった場合は未解決と明記する。

64-bit Pythonが必要。管理者化、レジストリ変更、DLLのコピー/削除は行わない。
診断プロセスには時間制限を設け、タイムアウト時に終了させる。
起動・検出器の失敗はINFRA_ERRORになり、ASan依存テストを合格にしない。

実装時にはWindows実行・テストを行っていない。回帰テストには合成PEの名前/ordinal/転送と一ファイル報告への格納を追加した。

## clangに対応するASan DLLの選択

0xC0000139の報告で、`C:\Program Files\LLVM\bin\clang.exe` が生成したexeに対して
MSVC 14.38.33130の `clang_rt.asan_dynamic-x86_64.dll` が読み込まれ、13関数が不足していた。
例は `__sanitizer_cov_load1__dll`、`__sanitizer_get_dtls_size__dll`。

事前検査は選択clangの `--print-runtime-dir`、`--print-resource-dir`、
`--print-file-name=clang_rt.asan_dynamic-x86_64.dll` とclangのbinディレクトリからDLLを探索する。
PATH上の別コンパイラのDLLは代用しない。
x64と要求exportを確認したDLLを、診断/正常起動/負例、および後続ASan検査の子プロセスPATHで優先する。
正常起動と負例は同じDLLを使用し、後続検査の前にもSHA256を再確認する。
システム/ユーザーの恒久PATHは変更せず、DLLもコピーしない。

`case_report.runtime_discovery` に問い合わせ結果と存在有無、`runtime_selections` にexport照合結果、
`runtime_environment` に選択DLLとhashを記録する。
一致するDLLがインストールされていなければ `No compatible ASan DLL found...` と明示して停止する。
その場合は選択LLVMと同じ配布・バージョンのcompiler-rt（ASan runtime）の導入が必要。
任意バージョンのDLLを流用して合格にはしない。必要な具体的パスは診断JSONに残る。

実行コマンドは引き続き `.\scripts\safety_windows.cmd`。
