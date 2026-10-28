# 仕様に基づく3つの安全性判定機構

保証対象は、定義した実行条件の下で到達可能な実行に安全性違反が生じないこと。
危険になり得る要素の存在自体をすべて禁止することではない。
既存の12項目・コンパイラ別の5目標・Lean検査・証拠管理を利用する。

## 分割した実装の状態

| 段階 | 担当する判定 | 今回の状態 |
|---|---|---|
| ① 仕様に基づく対象の絞り込み | 何を違反とし、どの保護条件を確認すべきか | 実装追加。実行未確認 |
| ② 実装との対応検証 | 実際の型検査・停止処理・不変条件・省略条件が成立するか | 登録済み契約の検証・条件への適用・独立再実行を追加。実行未確認 |
| ③ 網羅性・合成・受入 | 到達可能な対象経路に抜けがなく、5目標が成立するか | 独立再検証・条件の網羅性・合成契約・5目標別受入を追加。公開Actionsで実行確認 |

①の終了値0は対象整理ができたという意味で、言語の安全性合格ではない。
今回、既存の受入基準や実用テストは緩めていない。新しいテストケースも作っていない。

## ① 今回追加した内容

`formal/safety-scope-policy.json` は、仕様の文章を根拠にしたレビュー対象の規則集。
`scripts/safety_scope.py` が規則を検査し、コンパイラ別の判定対象を出力する。
自然言語の仕様を自動で数学的に証明するプログラムではない。

| 要素 | 仕様による分類 | 残すべき確認 |
|---|---|---|
| 寿命後のrefの保持 | それ自体は違反ではない | 使用前の拒否、検査metadataの生存、退役後の非復活 |
| 複数のref mutやrefとの共存 | 仕様上許される | 権限、参照の有効性、並行操作の安全性 |
| 明示的な整数縮小変換の周回 | 仕様どおりなら違反ではない | 変換の意味保存。世代やレイアウトのオーバーフローに一般化しない |
| 規定の安全な停止 | 停止したこと自体は違反ではない | 不正操作より前の停止と、停止処理自体の安全性 |

実行前の拒否、安全な停止、不変条件、チェック省略の成立条件、到達不能、意味保存を
別々の証明経路として扱う。どの経路も、選択しただけで証明済みにはならない。
特に `unreachable` を宣言しても、実装の経路は除外されない。

安全条件は11種類の共通定義に整理した。これはテスト11ケースではなく、後続が検証する契約。
同じ条件を複数の目標で共有する。jwcとRustには別の適用対象ID・ソース識別を付ける。
ソースファイルの一覧は来歴確認に使い、ファイル数×5を新しい証明作業件数にはしない。

5目標の根の命題は既存の `acceptance_requirements.GOAL_CRITERIA` をそのまま使う。
条件の一覧だけでは全言語の網羅性を証明したことにならないため、各目標に
`REQUIRES_COVERAGE_AND_COMPOSITION` を残す。全体の判定は必要な実装対応・前提・網羅性・合成が確認されるまで `NOT_PROVEN`。

## 変更・不正な除外の扱い

- 仕様2ファイルをLF正規化したSHA-256と、各規則の一意な引用を照合する。
  仕様が変わったら `INVALID_SCOPE` とし、規則の再レビューを要求する。
- 未知の欄、重複ID、存在しない目標・保護経路・引用・依存条件を拒否する。
- 各規則・命題・コンパイラ・ソース版に識別値を付け、後続で別対象の証拠を混ぜない。
- 取得途中に仕様・規則・ソースが変わったら拒否する。
- レポートの `validate_scope()` は現在の入力から再生成して完全比較する。
  目標・条件の削除や、片方のコンパイラの脱落を成功扱いしない。
- これはローカルの仕様・規則をレビュー済みとして使う仕組み。
  ハッシュは同一性を確認するもので、仕様の正しさや実装の安全性の証明ではない。
- 外部ツールを新たに信頼する前提、未対応機能の除外、無検査モードの安全認定を追加しない。

## コマンド

今回の単独コマンドはソース・仕様の読み取りとJSON出力だけを行う。
Clang、Lean、コンパイラテストは起動しない。

```sh
# 両コンパイラの仕様判定対象
python scripts/safety_scope.py --output target/safety-scope.json

# jwcだけを対象整理
python scripts/safety_scope.py --compiler jwc --output target/jwc-safety-scope.json
```

WindowsでPythonランチャーを使う場合は `python` を `py` に置き換える。

`SCOPE_READY` なら終了値0、仕様・入力が不正なら `INVALID_SCOPE` で終了値2。
ここで `SCOPE_READY` と安全性の `PASS` / `PROVEN` を同一視しない。

既存の `python scripts/formal_tool.py` にも `safety-scope` 工程を追加した。
一括コマンドは従来どおりLean等も起動するため、実行再開時に使用する。
出力は `target/formal-tool/safety-scope.json` と `summary.json`。

## 検証状況

実装時にはPythonの構文、JSON、仕様引用の対応、差分の整合性を確認する。
新しい判定器・Lean・既存テスト・CIの実行結果は別に記録する。
判定機構の追加と、そこで扱う実装対応証明の成立・言語全体の証明完了は区別する。

## ② 実装との対応検証

`scripts/safety_correspondence.py` は①のレポートを現在の仕様・ソースから再生成して照合し、
固定したローカルの検証器を実行する。入力レポートのコマンド、定理名、許可する公理、
前提の解除指定を実行・採用することはない。

| 接続した契約 | 実行する検証 | 結論の範囲 |
|---|---|---|
| 世代ガードの容量条件 | 実C関数の型付きClang ASTから全64ビット入力のLean命題を生成・検査 | Cフロントエンドと抽出器を信頼する条件付きの関数契約 |
| 参照ゲート | コンパイラ別のエミッタ構造照合＋既存Lean命題の型・公理検査 | 認識したテンプレートとモデルの判定条件。ポインタ生存・全アクセス経路は別 |
| チェック省略 | 既存の `sound_elision_preserves_read` の命題・公理検査 | `FactSoundAtAccess` を前提とするモデル。実データフローでの成立は別 |
| 書込み権限 | 各コンパイラの既存モデル定理の命題・公理検査 | モデル上の許可条件。実型検査全体との接続は別 |
| 所有者の移動 | 各コンパイラの既存モデル定理の命題・公理検査 | 移動前の一意所有と成功を前提にしたモデルの保存。実move/drop全体は別 |

既存の `lean_evidence_engine.py` と `lean_refinement_contracts.py` を利用し、
`formal/refinement-contracts.json` に今回使う既存7定理の期待する命題を登録した。
定理名だけが一致しても、期待する命題の検査が欠けた場合は拒否する。
共有定理のカーネル検査はまとめて行い、コンパイラ固有の適用先は分離する。

容量条件はPR #39の実ソース証明生成処理を再利用し、Rustにも独立した対象IDを付けた。
共通C関数について同じ性質を検査するが、jwcの結果をRustへ転記しない。
jwcの埋め込み文字列は同期元と照合する。Rustの埋め込み・生成処理まで証明したとは扱わない。
Leanの当該ソースは一時プロジェクトでビルドする。

検証器、仕様、ソース、Lean入力の変化、別コンパイラの結果、欠けた条件、
未承認の公理、期待する命題との不一致、タイムアウトは成功にしない。
Clang用CヘッダーとLean 4.24.0/lakeが必要。Windowsでは既存のx64開発者環境を使う。

```sh
# ①と②。②はClang/Leanを実行するが、実用テストケースは実行しない。
python scripts/safety_scope.py --output target/safety-scope.json
python scripts/safety_correspondence.py --scope target/safety-scope.json --output target/safety-correspondence.json

# レポートの文字列を信用せず、現在のローカル環境で検証を再実行して完全比較する。
python scripts/safety_correspondence.py --scope target/safety-scope.json --replay target/safety-correspondence.json --output target/safety-correspondence-replayed.json
```

jwcだけなら両コマンドで `--compiler jwc` を指定する。
`formal_tool.py` にも②を接続した。出力は `target/formal-tool/safety-correspondence.json`。

`CHECKED_WITH_OPEN_CONDITIONS`（終了値0）は登録済みの局所契約の検査が完了したという意味。
各条件に、確認できた契約・証拠水準・前提・未確認部分を出力する。
未対応の条件は `OPEN_NO_IMPLEMENTATION_PROOF`、部分的証拠がある条件は
`OPEN_WITH_CHECKED_PARTS`。全条件や言語全体の合格とはしない。
検証失敗は `INVALID_EVIDENCE`（終了値2）。

`validate_report_binding()` は同じ実行器が生成したレポートの構造・帰属確認。
アップロードされたレポートを証明として認証する場合は必ず `replay()` を使う。
③もこの再検証経路を利用し、残る前提を文字列だけで解除しない。

②の追加時点では、実際のClang抽出・Lean検証・Windows/Linux実行は未確認。
既存テストやCIは再開していない。

## ③ 網羅性・合成・受入判定

`scripts/safety_acceptance.py` が②を独立再実行し、①の各目標へ対応付ける。
jwcとRustの各5目標を分離し、必要な条件、確認した局所契約、未解決の前提、
網羅性・合成の証拠、受入結果を一つのJSONに出力する。
目標数・受入数は実際の判定から計算する。片方の証拠による他方の合格は認めない。

- 到達不能や仕様上の非違反という文字列だけでは条件を解除しない。
- 全条件が成立しても、到達可能な実行の網羅性・合成契約が欠ければ `NOT_PROVEN`。
- 合成の接続はリポジトリ内の `formal/safety-composition-contracts.json` で管理する。
  未登録は未証明として扱う。登録には目標ID、同一context、全条件の一覧、対象命題、
  既存 `refinement-contracts.json` でレビューした期待命題の定理、前提を指定する。
  定理の期待する型と公理をLeanで検査する。未解決の前提は受入を阻止する。
  自然言語の対象と形式命題の意味対応はレビュー対象であり、自動翻訳ではない。
- 現在の②は局所契約の検証であり、条件全体の成立を出力しない。
  合成証明も未登録なので、現段階の言語全体の結果は `NOT_PROVEN`。

```sh
# ①→②→③（両コンパイラ）。③は②の検証を再実行する。
python scripts/safety_scope.py --output target/safety-scope.json
python scripts/safety_correspondence.py --scope target/safety-scope.json --output target/safety-correspondence.json
python scripts/safety_acceptance.py --scope target/safety-scope.json --correspondence target/safety-correspondence.json --output target/safety-acceptance.json

# 既存の形式検証工程も含む一括実行
python scripts/formal_tool.py --output target/formal-tool
```

③の終了値は0=`ACCEPTED`、1=`NOT_PROVEN`、2=`INVALID_EVIDENCE`。
`formal_tool.py` は③の終了値1を正常な未証明判定として扱い、工程障害と区別する。
一括出力の `safety_acceptance` に同じ10目標の結果を収録する。
実用12項目のテスト・既存34項目監査とは別の形式判定経路であり、既存監査を緩めない。

公開 `jabberwock_test` のActionsでは、既存Rust/Python/Lean検査、コンパイラ別Linux安全性検査に加え、
この一括実行を行いJSONとログを保存する。形式判定工程の正常完走と
`ACCEPTED` / `PROVEN` は区別する。Windowsでの実行結果には代用しない。
