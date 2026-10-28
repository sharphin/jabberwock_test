> 現行の実用テストは [コンパイラ別検査](compiler-safety-matrix.md) を参照。
> OSごとに12項目×2コンパイラ＋5目標×2。形式証明は別ツール。以下の旧混合集計は全コンパイラの合格を表さない。

# ローカル安全性テスト（GitHub Actions不要）

この仕組みは [verification-plan.md](verification-plan.md) の **実用安全性12項目** を、
**Windows用12項目**と**Linux用12項目**の独立したセットとして実行します。
各OSの番号・判定・証拠は別に管理し、片方の成功で他方の不足を埋めません。
**12項目の合格を自動認定するものではありません。** 合格条件には、網羅性、OS、検出器の負例、実行時の副作用など、別途判定すべき内容が残ります。

## 前提

- Python 3.10以上、Rust/Cargo、Cコンパイラの clang
- 包括検証には Linux x86_64 推奨。clangの sanitizer runtime（ASan/UBSan、TSan、MSan）、lld
- 形式モデルの検査には `lake` と Lean 4.24.0（`formal/lean-toolchain`）
- WindowsはWindows向けASan runtimeとCリンカ・SDKも必要。Linuxと同じ検出器構成を利用できるとは限りません
- `clang --version`、`cargo --version`、`lake env lean --version` で準備状況を確認してください
- sanitizerがランタイム起動に失敗する環境ではその試験は失敗です。スクリプトはOSの `sysctl` を勝手に書き換えません

### Windowsで未実行が多いとき

`tools.clang: null` / `clang unavailable` は、実行したシェルからclangを見つけられなかったことを示します。
LLVMのclangと、対象に合うCリンカ・Windows SDKを準備し、同じシェルで `clang --version` が動くようにしてください。
別の場所のclangを使う場合は `JW_CLANG` に実行ファイルのパスを設定できます。
Leanは `formal/lean-toolchain` に合わせて用意し、同じシェルから `lake` が見える必要があります。

Windows版はWindows用ASan・native・巨大配列・副作用検査を実行します。
Windows用の競合検出・未定義動作検出・未初期化メモリ検出・資源枯渇検査には未対応があり、該当する検査は `NOT_RUN` のままです。
LinuxでのTSan/MSan/RLIMIT結果を、Windows側の未対応検査の成功として扱いません。
Rustの通常テストとreleaseテストはclangなしでも実行しますが、native/jwcを使う一部のテストは内部でスキップされます。
単一JSONの `coverage_notes` にその制限を記録し、native専用の退役検査は `NOT_RUN` にします。

## Windowsの12項目とLinuxの12項目を別々に実行する

リポジトリルートで、そのOSの入口を使います。

Windows（PowerShell）:

```powershell
.\scripts\safety_windows.cmd
```

Windowsの永続PATH設定と起動時のx64準備は [windows-setup.md](windows-setup.md)。
準備済み端末での直接実行は `py -3 scripts/safety_windows.py`。

Linux（WSL2を含む）:

```sh
python3 scripts/safety_linux.py
```

| 検証セット | 項目一覧 | アップロードするファイル |
|---|---|---|
| Windows用12項目 | [Windowsの1〜12](safety-windows.md) | `target/safety-windows-report.json` |
| Linux用12項目 | [Linuxの1〜12](safety-linux.md) | `target/safety-linux-report.json` |

各OSでRust Debug/Releaseコンパイラと自己ホスト版を準備し、1番から12番まで順に検査を起動します。
そのOS内で共通するプローブは一度だけ実行します。Windows用の12件とLinux用の12件を混ぜて合格を数えません。

- **1回の報告は、そのOSのJSON 1ファイル**です。12項目の判定、コマンド、環境変数、Git SHA、OS、ツールの版、終了コード、stdout/stderr、実行時間、個別ケースJSONを含みます。
- schema 2の `suite.id` は `windows-12` / `linux-12`。項目にも `windows-01` / `linux-01` のようなIDと `target_os` を付けます。
- WSL2はLinuxです。Windows用入口をLinuxで実行する等、OSの取り違えは検査開始前にエラーにします。
- Windowsの3・4・6・8には未対応の検査があり、現段階では全項目成功で終了しません。未対応の理由もJSONに残します。
- 失敗後も独立した検査を継続します。準備ビルドに失敗した実行ファイルを必要とする検査はNOT_RUNにし、古い生成物を流用しません。
- 各検査の開始時にコマンド・制限時間・ログ保存先を表示し、15秒ごとに検査名・経過秒数・PIDを表示します。経過表示だけでは、検査内部が正常に進んでいることまでは保証しません。
- stdout/stderrの全出力は作業フォルダ内の `<検査名>.stdout.log` / `<検査名>.stderr.log` に逐次保存します。JSONには先頭・末尾を合計最大24 KiB収録し、切り詰めた場合はサイズとSHA-256を記録します。
- JSONは検査開始前と各検査の前後に更新します。途中でも12項目を収録し、未実行分は `pending_probe_names` に残します。`run_status` はRUNNING / COMPLETE / INTERRUPTED、`active_probe` は実行中の検査名です。COMPLETEは全検査の処理終了を表し、合格を意味しません。
- Windows等でJSONの置き換えがアクセス拒否になった場合は、合計0.75秒の待機を挟んで5回試みます。拒否が続く場合は同じフォルダの一意な別名JSONを保持し、以後その保存先を更新します。`Report redirected to:` と最後の `Report:` に出た実際のファイルを提出してください。元のファイルを削除して回避しません。新規ファイルも書けない場合は保存エラーを表示し、検査を止めて終了コード2にします。
- Ctrl+Cでは実行中の検査をINTERRUPTEDとして記録し、途中結果を保存して終了コード130で終了します。タイムアウト・中断時はWindowsではtaskkill /T、Linuxでは専用プロセスグループへ終了を要求し、後片付けにも制限時間を設けます。強制終了・電源断では最後の保存時点のJSONとログが残ります。
- `NOT_RUN`、`FAIL`、`CRASH`、`TIMEOUT`、`INFRA_ERROR`を区別します。全プローブ成功でも`PROBES_PASS_ONLY`であり、12項目の形式的な認定ではありません。
- 同じ検査を複数項目が共有する場合、1件の失敗が複数項目の `FAIL` に現れます。欠陥の件数は `results` のログで確認してください。項目8と9は別コマンド・別ケースJSONで判定します。
- nativeを必須とするCargo検査には `JW_REQUIRE_RETIREMENT_TESTS=1` を設定します。通常のCargo検査の成功件数だけでnative実行を確認したことにはなりません。
- 必要なclang・Lean・sanitizer等がない場合も判定に記録します。各OSで全プローブが成功しても、12項目全体の受け入れ判定は別です。
- 中間ケースJSONやファズ入力は `target/safety-windows-report-work/` / `target/safety-linux-report-work/` に分けて保存します。ファジングで生成した巨大な入力ファイルそのものは別保存です。
- ケースJSONは毎回新しく作られたものだけを取り込みます。欠落・ケース数不足を成功扱いにしません。
- 概要だけ見る場合は各入口に `--dry-run` を付けます。検査・バージョン取得コマンドを実行せず、レポートも生成しません。
- ランナーの基本動作確認: `python3 scripts/test_safety_report.py`。

`jwc/runtime.jw` の同期不一致が出た場合、修正側は `python3 scripts/sync_runtime.py` で生成ファイルを更新してコミットします。
報告用ランナーは `JWC_REGEN` を子プロセスから外し、検査中に同期不一致を自動修正して隠さないようにします。
2026-10-26のWindows結果の読み取りと修正範囲は [今回の記録](safety-report-2026-10-26-windows.md) を参照してください。

重い包括テストです。メモリ・CPUの余裕がある環境で実行してください。詳細をすべて含む独立したJSONは大きくなるため、必要に応じて圧縮して送れます。

`scripts/safety_report.py` から直接呼ぶ場合も、現在のOSのセットを自動選択します。
`--platform windows` / `--platform linux` で明示できますが、実行OSとの不一致は許可しません。
既存の `--output PATH`、`--scale N` は両方の入口で利用できます。

## 従来の共通ランナー（補助用）

以下の `local_safety.py` は互換用に残しています。OS別12項目の提出には上記の専用入口を使ってください。

リポジトリルートで実行します。

```sh
# 日常の高速な開発確認（対象を絞ったテスト）
python3 scripts/local_safety.py quick

# 実際にコマンドを起動せず、実行計画を確認
python3 scripts/local_safety.py full --dry-run

# Linuxで既存の包括的な安全試験を再現（未実行も終了コード非0）
python3 scripts/local_safety.py full --strict

# 例：未解決の8・9だけを再テスト
python3 scripts/local_safety.py full --items 8,9 --strict

# 例：生成プログラム数を20倍にする
python3 scripts/local_safety.py full --scale 20 --strict

# 出力先を固定
python3 scripts/local_safety.py full --strict --report-dir target/local-safety/manual
```

Windowsでは `python` に読み替えられます。サニタイザ非対応の試験を含む `full --strict` は、Windowsで成功終了しないのが意図した挙動です。

## 実行内容

スクリプトは既存の `safety.yml`、`formal-spec.yml`、`test.yml`、`bench-checks.yml` と専用再試験スクリプトをもとに、以下を統合します。

- Rust通常テストと `JW_NATIVE=1` 差分テスト、退役テスト、ベンチマークの検査回数
- sanitizer自身が実際に欠陥を検出できることを確認する負例
- ASan/UBSanでの堅牢性テスト、変異入力のnative実行、型の通るファジング（O1とO2）
- TSanスレッド試験、MSan全体試験
- 自己ホスト `jwc` のビルド、jwc生成実行ファイルのASan/MSan差分比較
- 項目8・9の専用再テスト（資源枯渇と巨大配列）
- 項目10〜12の外部副作用を検証する部分テスト、Rust releaseテスト
- Leanモデルのビルドと公理監査

**重要:** `jwc` の生成物への `JW_CFLAGS` の伝播は既存実装の制約を受けます。Rust側でsanitizerフラグを指定しただけで、jwc生成物まで完全に計装されたとみなさないでください。

## 結果の読み方

ログと `report.json` は既定で `target/local-safety/<UTC時刻>/` に保存します。

- `PROBE_PASS`：そのコマンドが終了コード0になりました
- `FAIL`：コマンド失敗・タイムアウトなど
- `NOT_RUN`：必要なツールや対応OSがなく実行できませんでした
- `PROBES_PASS_ONLY`：項目に紐付く今回の試験は成功しましたが、**項目全体の合格ではありません**
- `acceptance: NOT_ASSESSED`：12項目の包括的な合格判定は実施していません

`--strict` は試験失敗だけでなく `NOT_RUN` も非0終了にします。未解決の8・9や10の副作用問題が再現する場合は `FAIL` のまま記録し、無理に緑にしません。

`quick` は短い開発用プロファイルで、全項目を実行しません。OS別12項目の証拠は `safety_windows.py` / `safety_linux.py` でそれぞれ収集してください。

### 注意と限界

- GitHub Actionsを停止・削除する変更は含みません。既存CIと並行運用した後、必要なら無効化してください
- 本変更はテスト実行器のみで、コンパイラ本体・言語仕様・既存テストの合格基準は変更しません
- コマンドが環境によって条件付きでテストをスキップすることがあります。終了コード0やテスト件数だけで実行到達を保証できません。各ログと `docs/verification-plan.md` の受け入れ条件を照合してください
- `full` はファジングやsanitizerを多用するため高負荷・長時間です。独立した作業環境での実行を推奨します
