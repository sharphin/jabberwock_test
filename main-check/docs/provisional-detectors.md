# 項目3・4・6の暫定検出器

**方針：この3項目は既存ツールによる暫定対応。後でJabberwock専用検出器を実装して置き換える。**
暫定ツールが成功しても、全言語・全実装経路の検証完了とはしない。
コンパイラ別・OS別の記録を維持し、Linux結果でWindowsを充当しない。

|項目|Windows暫定ツール|Linux既存ツール|専用化で残る作業|
|---|---|---|---|
|3 データ競合|Intel Inspector CLIのti3収集に接続|TSan|言語の同期/所有権移動/回収と対応した競合検出|
|4 未定義動作|UBSan＋LLVM opt verifier|UBSan＋LLVM opt verifier|poison/undef、属性、alias、言語意味論との対応検査|
|6 未初期化|Dr. Memory|MSan|生成LLVMとruntimeの初期化状態を追跡する専用検査|

Intel Inspectorは既存の利用可能な環境向けの接続口。製品の入手・ライセンス・現行Windowsとの互換性を保証しない。
未導入ならNOT_RUN。新規導入ができない環境を合格にするために並列ストレステストへ置き換えない。
このPRで製品のインストールや取得は行わない。

## 実行コマンド

既存のコマンドに組み込み済み:

```powershell
.\scripts\safety_windows.cmd
# jwcのみ先に確認
.\scripts\safety_windows.cmd --compiler jwc
```

PATHにないツールはPowerShellで実際のインストール先を指定する（以下のパスは例）:

```powershell
$env:JW_INSPECTOR = 'C:\tools\Inspector\bin64\inspxe-cl.exe'
$env:JW_DRMEMORY = 'C:\tools\DrMemory\bin64\drmemory.exe'
$env:JW_OPT = 'C:\Program Files\LLVM\bin\opt.exe'
.\scripts\safety_windows.cmd
```

UBSanは `JW_CLANG` のclangを使用する。対応runtimeがない/リンクできない場合はINFRA_ERROR。
Inspector/Dr. Memory/optが未導入ならNOT_RUN。版の不一致やレポート書式不明も成功にしない。
Dr. Memory/Inspectorの実機互換性とCLI動作は未確認。実行結果を受けて対応を詰める。

## 検出能力と対象コンパイラ

各コンパイラ自身のLLVMを生成してから、意図的な不具合を注入する。

- Inspector: 生成したwork関数に共有値の非atomic読み書きを注入し、2スレッドのData raceを検出。
- Dr. Memory: 生成したmain関数に未初期化heapのvolatile loadを注入し、UNINITIALIZED READを検出。
- UBSan: 生成LLVMから計測済みCのゼロ除算を呼び、runtime errorを検出。LLVM自体のUBを全て検出した扱いにはしない。
- LLVM verifier: 各コンパイラのLLVMに不正なphiを追加して拒否を確認し、その後通常LLVMをverifyする。

負例を検出できなければ通常ケースへ進まない。
通常ケースはInspectorがスレッド/channel関連3ケース、Dr. Memory/UBSanが所有値・配列・リスト・クロージャ・enum・map・thread・channel・`?`の9ケース。
各々checked/elided × O1/O2+LTOを実行する。LLVM verifierは全てのコード生成可能な登録fixtureをchecked/elidedで検査する。

検出器の終了0だけで成功にしない。対象exe内の専用Cドライバがstdout/stderrを別ファイルへ保存し、
runtime正常復帰の終了コードを記録する。期待stdout、live=0、正常復帰、検出器の無問題報告を必須にする。
ドライバはテスト用で製品runtimeを変更しない。ツールの診断を削除・自動抑制しない。
InspectorはProblem Type列を持つCSVを解析し、不明な書式をゼロ件と解釈しない。
Dr. Memoryはresults.txtのNO ERRORS FOUNDを要求するため、他の種類の異常も失敗として残る。
ケースIDの欠落はINFRA_ERROR。結果・検出器ログ・ツール版は通常のOS別一ファイルJSONへ格納する。

## 限界と専用実装への引継ぎ

- LLVM verifierはIRの整形式を検証するもので、意味保存や全UBの証明ではない。Alive2の導入は今後の検討対象で、このPRには含めない。
- 動的検出は実行した経路・スケジュールだけが対象。有限fixtureの成功では網羅完成にならない。
- 専用検出器を作る際も意図的な負例、欠落検出、OS/コンパイラ識別を維持する。
- 既存検出器と専用検出器を比較し、検出能力を確認してから切り替える。未完成の専用版で暫定検査を削除しない。

実装時はPython構文・差分の静的確認のみ。テスト・ビルド・検出器の実行は利用者が行う。
