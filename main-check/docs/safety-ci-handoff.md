# 作業メモ・引き継ぎ: 安全性の検証

## 実用テストの追加実行（2026-10-26追記）

- [追加実行の記録](practical-retest.md): Rustコンパイラ自身のrelease全体試験366件が成功。
  debug専用の自動IR/native比較がreleaseで動くとは扱わず、明示的なnative試験の結果と分ける。
- 正常終了/エラー終了の外部効果を、debug/release CLIとRust/jwc生成nativeで比較するscriptを追加。
  16ケース中12成功・4不合格。debugの二重書込みを検出し、未修正のまま失敗として記録した。
- releaseで8/9も再実行。19ケース中10不合格で、従来の課題は残る。
  11の失敗注入と全経路の対応、12のWindows等の未実行も残る。1〜12全項目合格ではない。
- 追加したものは検証用script・fixture・記録。本体ソースは変更していない。

## 形式証明専用の様式（2026-10-26追記）

- 実用テスト1〜12と別に、[形式証明専用のF1〜F5](formal-verification.md) を用意した。
  命題、前提、対象外、実際の公理依存、信頼する部分、実装との対応、機械検査の証拠を記録する。
- `formal/records/TEMPLATE.md` と、既存の世代・退役/所有位置のムーブの2記録を追加。
  「モデル内証明済み」と「実装接続済み」を分け、現在の2記録はともに実装接続が未証明とする。
- 既存22定理のビルドと公理監査をローカルで再実行し成功。証拠JSONに対象commit・入力ハッシュ・定理別の依存と実出力を記録した。
  定義・証明・検査script・コンパイラ本体は変更していない。新しい定理の追加やGitHub Actionsの成功とは扱わない。
- 現行の`check_formal.py`はLeanのビルドと公理依存を検査する。記録欄の完全性や自然言語の主張との一致を自動検査する機能は未実装。

## 実用上の確認を12項目へ拡張（2026-10-26追記）

- オーナーは今回の水準について「5項目すべてとは言わない」と補足。5保証の形式証明完成を今回の合格条件にしない。
  現行コンパイラの改良やライブラリ実装に進むための実用試験として [合格条件](verification-plan.md#実用上の安全確認) を追加した。
- コンパイラの追加10は仕様動作・型検査・副作用、11は所有権・回収・途中失敗、12は実際の実行構成。
  実行構成の確認も独立した12番として追跡する。
- 5領域すべてを実用試験の対象とする。全て通った場合も、検証した機能・構成での試験合格という判定とする。
  詳細23課題や5保証全体の形式証明の完成とは区別する。
- 実装36d4df2のdebug CLIで、外部コマンドの追記が1回の指示に対して2回起きることを一時ファイルで再確認した。
  既知のdebug照合の挙動を項目10の修正対象に明示。項目8/9と同様、未解決のまま合格にしない。
- 今回は検証条件・文書だけの追加。新設10〜12のテストを実装・完了したという意味ではない。
  次は8/9と10の具体的な不一致を解消し、11の経路別試験、12のrelease/Windows等の未検証構成を埋める。
- ライブラリ向けの旧10〜14はL1〜L5へ改称。過去の実行記録・1〜9の番号は変更しない。

## 退役実装と1〜9の再検証（2026-10-26追記）

- オーナーの指示で退役実装を再開。[実装と検証記録](generation-retirement.md) を正本とする。
  Rust/IRのretired状態、Cの上限確認・退役marker・保持表、Rust/jwc生成LLVMのmarker拒否を実装した。
- 小さい上限と実際の上限で、解放・再利用・所有値の移動・リスト・送信・別スレッドのmetadataを検証する。
  退役のnative検証は必須flagでclang不足を成功扱いにしない。
- 「1〜95」はオーナー回答により「既存1〜9すべて」と確認。8/9の再現fixtureとLinuxの隔離再テストscriptも追加。
  8はinterp/nativeのメモリ不足診断不一致、9は具体化後の配列上限のすり抜けを再現し、未解決・不合格のまま記録する。
- GitHub Actionsは支払い失敗または利用上限の表示で全ジョブ未開始。Windowsの現変更の結果を合格に含めない。
- ローカルLinuxのLSanは実行基盤の制限で異常終了。該当実行を中断し、LSanを無効にしたASan/UBSanを別検証する。
  MSan/TSanの負例・本試験、残存区画数の検査、LSan未実施を分けて報告する。
- ローカルの通常native・ASan/UBSan・MSan全体は各366件成功。TSanは45件×3回成功。
  最適化版（`JW_NATIVE=o2`）の全体試験も366件成功。
  Rust/jwc生成物の比較、攻撃78件、演算76件、LLVM390モジュール、堅牢性、ベンチ21件、Lean22定理も成功。
  8/9の再現試験は19ケース中10不合格。1〜9全項目クリアとは判定しない。実行条件と本数の内訳は正本に記載。

## 検証体系の整理（2026-10-26、退役実装前の記録）

- [検証体系と合格条件](verification-plan.md) を追加。5目標の下にS/M/T/O/Cの23課題を置き、既存1〜9と対応づけた。
  各課題の既存の入口・不足・合格条件、言語機能と実行構成、独立した期待値、証拠の記録方法を定める。
- 既存1〜7は定めた範囲の実行試験。8/9は未解決。22定理は部分モデルの証明。
  23課題の計画を追加しても、新しい試験や5保証が完了したことにはしない。
- オーナーの指示で、退役実装は検証体系の整理を先に行うため停止中。本体ソースは未変更。
  次は実行中CIの終了確認、退役実装と専用試験、変更SHAでの既存試験の再実行へ進む。
- 検証はPR #7元ブランチ `safety-ci-resume` で行う。ライブラリ向けL1〜L5（当時10〜14）とはIDを混ぜない。

## 形式検証の到達目標（2026-10-26追記）

- オーナーが示した責任範囲を [形式仕様の到達目標](formal-semantics.md#到達目標と責任範囲) として採用。
  コンパイラの意味保存、メモリ安全性、型安全性、所有権・参照の整合性、実行時チェックの正しさを保証対象にする。
- アプリの認可・認証ロジック、ビジネスロジック、アルゴリズムの正しさはプログラマーが担う。
  提供ライブラリ自身の証明書検証やSQLパラメータ処理等の不備まで、利用者の責任とはしない。
- 5項目ごとの証明目標と現在の到達点を対応づける。22個の部分モデルの定理だけで5項目全体を保証済みとは扱わない。

## 形式化と退役仕様（2026-10-26、退役実装前の記録）

- [形式仕様](formal-semantics.md) と [Leanプロジェクト](../formal/README.md) を追加。
  世代・退役・参照検査と所有位置間のムーブを初版の機械証明対象とする。
- [言語仕様](spec.md) と決定記録へ「周回禁止・上限で退役」を反映。
  Rust/IR/native/jwc本体は未変更で、旧周回処理の置換・境界試験は次の実装作業。
- `safety-ci-resume` 上でLean 4.24.0の22定理が成功。独自公理・証明の穴を許さず、公理依存も検査。
  言語全体の型健全性、検査省略の正当性、LLVMとの対応、スレッドは未証明。
- `.github/workflows/formal-spec.yml` で同じ検査を継続する。既存の1〜7のCIとは別の検証として扱う。
- ローカルで `scripts/check_formal.py`、証明の穴・偽の命題・検証器なしの拒否3件を確認。
  通常の `cargo test` はrunner集計351件成功、Clippyは警告0。clang依存の検証はローカルでは未実施で、合格数へ混ぜない。
- 本体の退役実装では、小さい上限を使った強制退役と再利用、使用中の移し替え、確保失敗、送信元の古いrefを検証する。
  保持する退役metadataはlive payloadと別計上し、項目8の検討対象へ含める。

## 現在の判断と次の作業（2026-10-25追記）

作業先は [PR #7](https://github.com/sharphin/jabberwock/pull/7) の元ブランチ
`safety-ci-resume`。このメモの更新と今回のテストはこのブランチで行う。
標準ライブラリの設計は [PR #8](https://github.com/sharphin/jabberwock/pull/8) に分離してあり、
ライブラリ本体はまだ実装していない。

### コンパイラの合格範囲

- 項目1〜7は [合格基準と実行結果](safety-validation.md) に記録した実装・機能・構成について合格した。
  現在のコンパイラの基礎検証として維持し、変更時には攻撃ケース・生成入力・負例を拡充する。
- 「7項目だけでコンパイラ全体の安全性が十分」とは判定しない。
  [既存の残課題8・9](safety-testing.md#項目1〜7の検証完了と残る課題) は未解決のまま。
- **8: メモリ不足・資源枯渇時の扱い。** メモリ上限、確保失敗の扱い、時間の使い切りを別途整理する。
- **9: 巨大な配列型のサイズ制限。** ジェネリックな型の引数を通じて上限をすり抜ける経路を修正・検証する。
- この追記は作業計画の記録であり、8・9を修正済み・検証済みにするものではない。

### ライブラリ実装時に追加する検証案（L1〜L5、旧10〜14は未実施）

| 項目 | 確認内容 |
|---|---|
| L1. C ABI・資源管理（旧10） | 型と配置、allocator、所有権、二重解放、古いハンドル、別context・別スレッドでの使用、終了との競合 |
| L2. 途中失敗（旧11） | 通信切断、timeout、ディスク満杯、確保失敗を注入し、資源回収と部分結果を確認 |
| L3. 外部入力・認証（旧12） | 壊れた画像・応答、不正な長さ・文字列、TLS証明書・名前検証、SQLパラメータを確認 |
| L4. 外部への副作用（旧13） | debug比較や再試行による二重書込み・二重送信を防ぎ、DBの既知の確定結果を保持 |
| L5. 配布・実環境（旧14） | DLL・ABI・version不一致の拒否、移動先起動、Windowsの実API・GPU・IME・UIAを確認 |

各項目に合格条件と対象OS・実行器・依存ライブラリの計装範囲を定める。
新しい操作を追加するたびにコンパイラの1〜12へ対応するケースを追加し、該当するL1〜L5も実行する。
未実施・Unsupported・skipを合格数へ含めず、模擬backendの成功と実OS・実機の成功を分ける。
既存の1〜7の合格を、新ライブラリの合格へ流用しない。

### 今回の確認

今回の変更対象はこの作業メモだけ。機能実装・コンパイラ本体は変更しない。
以下は `safety-ci-resume`、実装が `7fc6c7e` と同一の状態で実行した結果。

- `cargo test`: 成功。runner集計は351件成功・失敗0件。
  ローカルにclangがなく、clang依存の試験は早期returnするため、351件すべてを実処理の検証数とは扱わない。
- `cargo +1.99.1 clippy --all-targets`: 成功、警告0件。
- `git diff --check`、このメモのローカルリンク検査: 成功。
- native・jwc生成物・sanitizerの実行検証は今回ローカルでは未実施。
  1〜7の既存の合格根拠は `safety-validation.md` のCI結果と区別し、
  この更新に対するCIの実行状況はPR #7のChecksで確認する。

## 以下は一時停止時点の履歴（現在の合否は上記とsafety-validation.mdを参照）

2026-10-25 にオーナーの指示で一時停止した作業。ブランチ `claude/project-thread-yo9do9` にある（main には入れていない）。
全体の考え方と項目 1〜9 は docs/safety-testing.md。

## できていること（このブランチ）

- `.github/workflows/safety.yml`: 項目 1〜7 を流す CI。push・PR は短く、毎晩（日本時間 3:00）は 20 倍の本数。
  仕事は robust / fuzz-asan / tsan / msan / jwc の 5 つ。**まだ GitHub で一度も動かしていない**（apt で入れる
  `libclang-rt-<版>-dev`・`lld`、`sysctl vm.mmap_rnd_bits=28` が runner で通るかは未確認）。
- `src/native.rs` の `instrument`: `JW_CFLAGS` の `-fsanitize=` が address / thread / memory なら、書き出した関数に
  `sanitize_address` / `sanitize_thread` / `sanitize_memory` を付ける。
- `tests/robust.rs` の `generated_code_guards_undefined_operations`（項目 4）: 書き出した LLVM の割り算・ずらしの前に守りがあるかを見る。
  llvm.rs の守りをわざと外すと 55 箇所で失敗することを確かめた。
- `runtime/jw_rt.c`（`jwc/runtime.jw` は作り直し済み）:
  - エラー・`exit`・内部エラーで終わるときは `quit`（`fflush` → `_Exit`）。片づけずに終わるので、LeakSanitizer が
    「漏れ」と報告していた（fuzz.rs をASan で流したときの 130 本の失敗はすべてこれ。エラーで止まったプログラムの辞書など）。
    atexit の処理（`llvm.global_dtors` の `@jw.count.report`）はエラー・exit のときは動かなくなる。
  - `jw_view_target_nocheck` は写しでなければ v のまま返す。llvm.rs の `place` の中（写しかどうかを見る前に呼んで select で選ぶ書き出し）が、
    own の中身の最初の 8 バイトを読んでいた。MSan が calc の例で「初期化していない値」と報告した。

## まだのこと

1. 上の jw_rt.c の変更のあとで、手元でもう一度流す（止めたので結果がない）:
   - `JW_NATIVE=1 JW_CFLAGS="-fsanitize=thread" TSAN_OPTIONS=halt_on_error=1 cargo test --no-fail-fast --test threads --test native --test system`
   - `JW_NATIVE=1 JW_CFLAGS="-fsanitize=memory -fno-omit-frame-pointer" MSAN_OPTIONS=halt_on_error=1 cargo test --no-fail-fast`
   - `JW_CFLAGS="-fsanitize=address,undefined -fno-sanitize-recover=undefined -fno-omit-frame-pointer" JW_FUZZ=300 cargo test --test fuzz`
2. TSan の「thread leak」（`blocks_freed_on_other_threads`・`compiled_programs_run_threads`・`channel_misuse_is_an_error`）。
   join も detach もしていないスレッドが終わりに残っていた。エラーで終わる道なら `_Exit` で出なくなるかもしれない。
   正常に終わる道で出るなら、task を捨てずに終わる道がある（`jw_task_drop` が呼ばれていない）ので調べる。
3. ふつうの確認: `cargo test --no-fail-fast`・`JW_NATIVE=1 cargo test --no-fail-fast`・`cargo clippy --all-targets`。
4. main に入れて、safety.yml が GitHub で通るのを見る（落ちたら runner の準備の手順を直す）。
5. CLAUDE.md の不変条件に、エラーで終わるときは `_Exit`（`quit`）であることを足す。
