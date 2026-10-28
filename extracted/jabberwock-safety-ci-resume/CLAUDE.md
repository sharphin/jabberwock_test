# CLAUDE.md

> **開発指示は現在のユーザーの依頼を優先する。** 長い技術メモは `docs/claude-reference-archive.md` に保存。ここは常時参照する簡潔な入口。

## プロジェクト

- Jabberwock は `own` と世代付き `ref` を中心とした自作言語。
- Rust製コンパイラ、セルフホスト `jwc/`、Cランタイム、Lean形式化を持つ。
- 日本語で簡潔に報告する。実施していない検証を成功と呼ばない。
- 不明な仕様・実装状態は現行のソースコードで確認する。

## 現在の作業範囲

- 本体は `jabberwock` の **`safety-ci-resume`**。許可なく本体の `main` へマージしない。
- 公開テスト側は `jabberwock_test` の `main`。対象は `extracted/jabberwock-safety-ci-resume/`。
- 公開側固有の修正を保持し、依頼対象の変更だけを同期する。
- 現段階の主目的は形式検証の実行基盤の整備と、証拠の正確な判定。
- 安全性の受入は、下記の合意した判定基準に従う。
- 未証明は `NOT_PROVEN`、検証器の故障は `TOOL_ERROR` / `INVALID_EVIDENCE` と区別。
- 根拠のない `PROVEN` 表示や見せかけの安全性評価は禁止。
- 追加の形式証明テストケースや回帰テストの量産を勝手に行わない。
- 既存テストの実行、検証器の実行、新規テストケースの作成は別の行為として扱う。
- ドラフトPRを作らない。必要なPRは通常PRのみ。
- 仕様・既存証明・ランタイムの不変条件を保護し、安易に消さない。
- ブランチ・CIの起動条件は明示された作業目的とユーザー指示に従う。

## 安全性の判定基準（ユーザーとの合意）

- 定めた言語仕様・対象条件の下で、全テスト合格をもって「言語全体の安全性に合格」と判定する。
- 対象は既存基盤のコンパイラ別12項目と最終5目標の検証。jwc版を最優先にする。
- 合意した受入基準へ、別の無制限な証明要求を勝手に追加しない。
- jwcとRust、WindowsとLinuxの結果を分離し、他方の合格を転用しない。
- 各検証器の出力はそのまま記録し、成功・失敗・未実行を正確に報告する。
- 実行中・失敗・欠落が残る段階では、全テスト合格とは報告しない。

## 重要なコマンド

抽出フォルダへ移動して実行する。WindowsでPythonランチャーを使う場合は `python` を `py` に置き換える。
```sh
# 形式検証の一括実行
python scripts/formal_tool.py

# Linuxの12項目と5目標（コンパイラごと）
python scripts/safety_linux.py --compiler jwc --output target/safety-linux-jwc-report.json
python scripts/safety_linux.py --compiler rust --output target/safety-linux-rust-report.json

# Windowsの開発環境初期化と安全性検査
.\scripts\safety_windows.cmd

# 既存の回帰検査
cargo test
cargo clippy --all-targets
```
- 形式検証結果: `target/formal-tool/summary.json`。
- 安全性検査結果: コマンドの `--output` で指定したJSON。
- 形式検証基盤の完成基準: `docs/formal-tool-completion.md`。
- 仕様に基づく3段階の判定機構: `docs/safety-judgement-mechanisms.md`。
- ① `scripts/safety_scope.py`: 仕様に基づく対象整理。
- ② `scripts/safety_correspondence.py`: 登録契約の実装対応検証。
- ③ `scripts/safety_acceptance.py`: 独立再検証とコンパイラ別5目標の受入判定。
- 単独工程の成功を、未実行の全項目の合格に転用しない。
- 実装進捗の根拠: `scripts/tool_implementation_progress.py` / `docs/tool-capabilities.json`。

## ディレクトリの地図

| 場所 | 役割 |
|---|---|
| `src/` | Rust実装・インタプリタ・型検査・コンパイラ |
| `src/lexer.rs` | 字句解析 |
| `src/parser.rs` | 構文解析 |
| `src/ast.rs` | 構文木・型情報 |
| `src/types.rs` | 型検査 |
| `src/analyze.rs` | ASTの安全性解析 |
| `src/ir/` | 中間表現・解析・LLVM出力 |
| `src/ir/lower.rs` | ASTからIR |
| `src/ir/exec.rs` | IR実行 |
| `src/ir/llvm.rs` | LLVMテキスト生成 |
| `src/native.rs` | Clang呼び出しとネイティブビルド |
| `runtime/jw_rt.c` | 世代管理・所有権に関わるランタイム |
| `jwc/` | セルフホストコンパイラ |
| `formal/` | Lean 4.24.0の形式化 |
| `scripts/` | 検証・計測・監査・運用コマンド |
| `bench/` | ベンチマーク入力・旧方式・履歴資料 |
| `docs/` | 仕様・設計・詳細な操作手順 |
| `tests/` | 既存の回帰テスト |

## 言語・ランタイムの重要な不変条件

- `own` と `ref` の寿命の違いは安全性の中核。参照世代の検査を消す場合は根拠が必要。
- 一度無効化した参照が、メモリ再利用や世代一周で有効にならないことを目指す。
- 世代上限に達した区画は退役し、再利用に戻さない。
- `GEN_RETIRED` は全1の退役専用マーカーであり、有効な参照には発行しない。
- `GEN_COUNTER_MASK` は下位62ビットの世代カウンタを抽出する。
- Cの退役区画は保持表に残し、退役したペイロードを参照しない。
- 区画の世代は必要なアトミック操作で読み書きする。
- コンパイラの型検査、静的解析、IR、LLVMとCランタイムの前提を揃える。
- `--no-gen-checks` / `--no-bounds-checks` / `--unchecked` は安全性を弱める測定用設定。
- 上記オプションの性能を安全な通常ビルドの性能と混同しない。
- `runtime/jw_rt.c` 変更後は `python3 scripts/sync_runtime.py` の同期要否を確認する。
- `jwc/runtime.jw` は同期対象。自動生成の関係を破壊しない。
- スレッド、FFI、OS効果、LLVMの未定義動作やポインタ来歴を無視して証明済みとしない。

## 形式検証の運用

- `formal/` のLean定理と実コードの対応は別個の証明義務。
- `scripts/lean_evidence_engine.py` はLeanカーネル検証のための証拠処理。
- `scripts/refinement_transition.py` は独立した実装対応の状態遷移を扱う。
- `scripts/whole_compiler_proof.py` は実装全体の証明義務を列挙する。
- `scripts/semantic_composition.py` は段階間の合成の必要条件を記述する。
- `scripts/runtime_generation_binding.py` はC/Leanの世代番号関連の構造照合。
- `scripts/runtime_preprocessing_evidence.py` はClang前処理結果を記録する。
- `scripts/formal_tool.py` は各工程の一括起動と集約を担当する。
- ソースハッシュ、名前、構文一致、テストPASSだけでは実装の正当性証明にならない。
- 証拠は対象ソース・ビルド条件・ツールの実行結果に結び付ける。
- 無効な証拠、欠落、ソースの変化、矛盾を成功扱いしない。
- 検証結果は実行記録と区別して報告する。未実行は未実行と記す。
- GitHub Actionsの結果が失敗なら原因確認まで成功・完走とは書かない。
- Leanが証明する範囲とC/LLVM実装について未証明の範囲を明示する。
- 形式検証基盤の運用手順は `docs/formal-tool-completion.md` に集約する。

## コンパイラ間の互換性とCI

- 今後の開発はjwc版を中心に進める。
- Rust版とjwc版の挙動・仕様の対応を維持する。
- 字句・構文・型・IR・LLVM・診断の対応は既存の比較検査で確認する。
- 深さ制限など共通の保護条件を修正する場合は両版を確認する。
- ランタイム変更はCと埋め込み側の同期を確認する。
- Rust側のIR確認では、副作用を重複実行しないことにも注意する。
- Rust 1.65以上で動作する前提を保持する。外部クレートを勝手に追加しない。
- リポジトリ全体へ無関係な整形をかけない。
- 公開Actionsはリポジトリ直下の `.github/workflows/jabberwock-tests.yml` を使う。
- 失敗時はジョブログとartifactのJSON・詳細ログを確認する。
- 結果には対象コミット、OS、コンパイラ、実行条件を対応付ける。
- 検出器の未導入・非対応・負例検出失敗を成功扱いしない。
- ベンチマークの旧方式と記録は `bench/` を参照する。
- 詳細なコードの地図・各機能の不変条件は保存した旧詳細版を参照する。

## 仕様・設計資料

- 言語仕様: `docs/spec.md`。
- 設計判断: `docs/design-decisions.md`。
- IR設計: `docs/ir-design.md`。
- 形式化の対象と限界: `docs/formal-semantics.md`。
- 世代の退役経路: `docs/generation-retirement.md`。
- コンパイラ安全性マトリクス: `docs/compiler-safety-matrix.md`。
- Windows/Linux別の安全性検証: `scripts/safety_windows.py` / `scripts/safety_linux.py`。
- 安全性検査の共通レポート: `scripts/safety_report.py`。
- 暫定検出器の制限: `docs/provisional-detectors.md`。
- 仕様の変更時は同一変更で `docs/spec.md` も更新する。
- 既存の詳細開発メモ: `docs/claude-reference-archive.md`。

## 変更・レビュー時の注意

- 指定されたブランチでのみ修正する。
- 対象ファイルと関連ファイルを読んでから編集する。
- 破壊的な置換や大量削除を避ける。
- 依存する生成コード・ヘッダ・仕様の更新漏れを確認する。
- 実行したコマンドと実行していないコマンドを分けて報告する。
- テストケースを追加しない段階では、その制約を守る。
- 必要な確認で失敗したら、失敗内容を根拠つきで報告する。
- 新しい安全性主張には証明の範囲と前提を付記する。
- 速度の比較では正確性・安全性を損なう設定を区別する。
- 既存の文書を短縮する際は詳細を別ファイルへ退避して参照を残す。
- 最後に変更したファイル・コミット・未確認事項を簡潔に報告する。

## 優先度

- 第一に安全性の保持と正確な状態判定。
- 第二に、目的に直結する実装を完了させること。
- 第三に、再現可能な検証結果と記録。
- テスト数やコミット数を実装完成度の代替指標にしない。
- 明示的な依頼なく本体の `main` にマージしない。
