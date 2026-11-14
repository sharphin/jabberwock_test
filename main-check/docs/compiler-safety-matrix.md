# コンパイラ別の実用検査と形式証明

安全性の確認対象は jwc を最優先とする。Windows と Linux それぞれで、
jwc の12項目と Rust 版の12項目を独立に実行・集計する。
さらに各コンパイラに対して最終目標5項目の追加テストを実行する。
既定では OS ごとに24項目と10目標を一つの JSON に保存する。
形式証明は別ツール・別JSONであり、実用テストから Lean を呼び出さない。

## 実行

Windows（x64の環境初期化・リンク確認を含む）:

```powershell
.\scripts\safety_windows.cmd
# jwcだけを先に確認する場合
.\scripts\safety_windows.cmd --compiler jwc
```

Linux:

```sh
python3 scripts/safety_linux.py
python3 scripts/safety_linux.py --compiler jwc
```

準備のRustビルドはjwcのブートストラップに必要。準備成功をjwc検査の合格として数えない。
jwc検査が終わってからRust版を検査する。各プローブの開始・経過時間・ログパスを表示する。
実行するテストは利用者側で行う。この変更時にエージェントによるテスト実行はしていない。

34件の正式受入条件・証拠の対応表・実際のLean証明の残課題は [34件の正式受入基準](acceptance-34.md) を参照。\n\n## ローカル一括監査（GitHub Actionsは使用しない）

Windows（PowerShell / リポジトリ直下）:

```powershell
py scripts/verify_all_local.py
py -m unittest discover -s scripts -p "test_*.py"
# Rust製コンパイラの最適化チェック数をWindows上で監査する場合（セルフホストjwcとは別）
py scripts/check_bench_checks.py
```

Linux / WSL2:

```sh
python3 scripts/verify_all_local.py
python3 -m unittest discover -s scripts -p "test_*.py"
```

既存のJSONだけを集約する場合は `--practical` と `--formal` にそれぞれのファイルを指定する。
Windowsの新規実証試験は `scripts/safety_windows.cmd` 経由で実行し、
その中でVisual Studio/Clang x64の環境初期化とツールチェーン検査を行う。
Lean監査は独立したPythonプロセスとして実行し、Windowsではユーザーの
`.elan\\bin`、`.cargo\\bin` もコマンド検索経路に加える。
**既存の結果ファイルを誤って新規実行結果と誤認しないよう、実行前に前回のJSONを消去する。**

`target/local-verification.json` にOS別の12項目×2コンパイラと
G1〜G5×2コンパイラを別々に記録する。最終安全性認定は
局所的なLean定理と実コンパイラ・LLVM・ランタイム全体の数学的対応が未完成のため
**`NOT_PROVEN`** のままとする。欠落・重複・別コンパイラの混入は
`INVALID_EVIDENCE`、実証試験の失敗は `PROBE_FAILURE`、
必要なテスト未実行は `NOT_RUN`、Lean監査失敗は `PROOF_CHECK_FAILURE`、
古い証明は `PROOF_STALE` と区別する。
JSON中の `PASS`/`PROVEN` という文字列を安全性認定の証拠として信頼しない。
将来の完全証明と正式な実証受入条件が機械的に検証されるまで合格判定を緩めない。

## 12項目の検査対象

以下は各コンパイラに適用する。ケース・構成・実行対象を混在集計しない。

|項目|現在の実行経路|残る確認範囲の例|
|---|---|---|
|1 検査省略|当該コンパイラのchecked/elided LLVM、仕様期待値、fuzz|全IR命令と制御フロー|
|2 型の通る攻撃|無効参照・境界・型拒否・ASan・退役負例|全公開機能の攻撃パターン|
|3 データ競合|スレッド/所有権移動/channel、Linux TSan・Windows暫定Inspectorと負例|全共有経路、専用検出器|
|4 LLVM未定義動作|数値・境界期待値、暫定UBSan＋LLVM verifier|LLVM poison/undef・属性・alias（UBSanでは網羅しない）|
|5 最適化|O1/O2+LTO、検査省略あり/なし、ASan fuzz|サポートする全構成|
|6 未初期化|Linux MSan・Windows暫定Dr. Memoryと当該コンパイラLLVMの負例|専用検出器、全生成経路|
|7 自己ホスト|jwc stage1→2→3、LLVM一致、生成物実行。Rust側はbootstrap元の回帰|stage2/3への全検査適用|
|8 資源枯渇|runtime malloc/calloc/reallocのN回目失敗注入、Linux資源制限|コンパイラ内部・スレッド・スタック・OS資源|
|9 巨大配列型|各コンパイラの型境界・上限内/外|全入れ子型・レイアウト演算|
|10 仕様/型/副作用|直接期待値、型拒否、順序、ファイル副作用|全仕様の正負ケース対応表|
|11 所有権/回収|move/drop、list変更、途中return/?/break、live=0、失敗注入|全所有型・失敗経路・FFI|
|12 実環境|OS別、O1/O2+LTO、通常CLI、副作用、bootstrap|実機/CPU/ツールチェーン構成表|

検査の具体的な一覧は `scripts/safety_compilers.py`、期待値は
`scripts/compiler_contracts.py`、不足は `scripts/safety_goals.py` に置く。
NULL注入はruntimeの5つの単一スレッドfixtureだけを対象とし、
生成LLVMやコンパイラ自身の確保を検査した扱いにはしない。
検出器負例は当該コンパイラのLLVMに意図的な欠陥を挿入して検出を確認する。
差分fuzzではRust側を比較先に使うが、jwcのコード生成・実行を必須にする。
直接期待値テストは他コンパイラの出力を正解として使用しない。

## 最終目標5項目の追加テスト

|目標|独立に実行する追加ケース|合わせて必要な12項目の証拠|
|---|---|---|
|意味保存|評価順序と戻り値|1,5,7,9,10,12|
|メモリ安全性|所有値を反復確保・解放しlive=0|2,3,4,6,8,11|
|型安全性|戻り値型の不一致をコード生成前に拒否|2,9,10|
|所有権・参照の整合性|連続move後の参照先の保持|1,2,3,8,11|
|実行時チェックの正しさ|drop後の参照を指定エラーで拒否|1,2,4,5,8,9|

これらは5目標の代表ケースから開始した検査で、目標全体の網羅完成ではない。
各目標の直接テストと関連項目の両方が成功して初めて `PROBES_PASS_ONLY` とする。
不足が残る間は `acceptance: BLOCKED`。未実装・未実行・失敗を合格にしない。
プロセス終了0は実行したプローブの成功であり、安全性認定を意味しない。

## 形式証明：別ツール

```powershell
py -3 scripts/formal_report.py --compiler jwc --output target/formal-jwc-report.json
py -3 scripts/formal_report.py --compiler rust --output target/formal-rust-report.json
```

両コンパイラの参照アクセスについて各8定理を実装した。
証明対象・必要環境・Linuxコマンド・終了コードは [形式検証コマンド](formal-commands.md) を参照。
共有抽象モデルは `--abstract-model` で追加実行する。コンパイラ全体の5目標には `--require-full` を指定する。
局所定理の成功は `KERNEL_CHECKED`、実装全体の状態は `NOT_PROVEN` と区別する。

## レポート

schema 3: `items` はOS・compiler・項目番号の複合ID、`goals` はOS・compiler・目標の複合ID。
ネストしたケースのsubject不一致、必要ケースIDの欠落/重複、検出器が0件実行は `INFRA_ERROR`。
古いschema 2の混合集計と互換ではない。旧レポートを全コンパイラの合格証拠として移行しない。

`jwc --ll-checked file.jw` は検査省略解析を飛ばしたLLVMを出力する追加CLI。
通常の `--ll` と `build` の挙動は変更しない。

Rust CLIの `--no-elide --ll` が以前はフラグを無視していたため修正した。
両コンパイラでcheckedとelidedを区別して生成する。CLI回帰テストも追加したが未実行。
共有C runtimeの退役検査とUB検出器負例は `evidence_scope` に共有基盤と記録する。
これだけでコンパイラ生成コードを検証したとは扱わない。

### コンパイラ別形式証明の入口

`formal/compilers/<compiler>/obligations.json` に実装ソースhash、局所的な証明範囲と除外範囲、定理入口を記録する。
名前空間は `JabberwockFormal.Jwc.*` と `JabberwockFormal.Rust.*`。
別コンパイラの定理や古い実装hashの証明を流用しない。hashの一致だけでは実装との同値性を証明しない。

項目3・4・6には [既存ツールによる暫定検出器](provisional-detectors.md) を接続。後で専用検出器を実装して置き換える方針。未導入や検出能力未確認を合格にはしない。
