# PR #30 引き継ぎ仕様：未完了4機構の実装と受入条件

## 目的と守るべき作業順序

Jabberwock の **jwc セルフホスト処理系と Rust 処理系の両方**について、単なるソースハッシュ照合・サンプルテスト・Lean の限定モデル定理を「コンパイラ全体の数学的証明」と誤認しない検証基盤を完成させる。

1. 実装・数学的証明・検証器・負例テスト・セルフレビューを開発側が完成させる。
2. **4機構すべてが完成するまではユーザーに実機テストを依頼しない**。
3. 完成後、ユーザーが実機で `py scripts/verify_all_local.py` を実行して結果を渡す。
4. 失敗した場合は開発側が原因を特定し、**コンパイラ本体・ランタイム・形式仕様・検証器を必要に応じて修正**する。検査条件の緩和や根拠のない PASS への変更は禁止。
5. 以降「修正 → ユーザー実機再テスト」を合格まで繰り返す。

**完成報告は機構ごとに1回のみ、合計4回。途中の実装・コミット・監査強化は完成報告に数えない。** 実際に証明できない要件は理由と証明上の前提を明確にして未完了として扱う。

## 完成させる4機構（それぞれ独立に受入）

### 1. コンパイラ全体の数学的対応証明
- 実際の言語仕様、字句解析、構文、型、所有権・借用・move/drop、エラーと停止、副作用、FFI と並行性に対する操作的意味論を形式化する。
- AST → 型付け → IR → 変換 → 出力の各段階について型保存、進行（規定の停止を含む）、観測的意味保存を証明する。
- G1〜G5（意味保存・メモリ安全・型安全・所有権/参照整合性・実行時チェック）を、jwc/Rust の実装に**別個に**対応づける。
- 現存する `formal/JabberwockFormal/ScopedGoals.lean` は定数・加算・世代付き参照の小言語のみ。機械整数オーバーフロー・メモリ生存期間・並行 atomic・実コンパイラの全経路を証明したとみなさない。
- 完成条件：サポートする全構文と全実装パスを対象とする kernel-checkable な定理、依存公理/除外一覧、実装との対応証拠が揃い、未証明義務がゼロ。対象外機能を黙って省かない。

### 2. LLVM 意味論との完全な対応証明
- IR生成器の全出力経路、制御フロー、エラー/トラップ、メモリ操作、LLVM の poison/undef・UB・alias/provenance・lifetime・アラインメント・atomic ordering・リンクと最適化の意味を対象にする。
- `scripts/proof_emitter_correspondence.py` の文字列テンプレート一致や `llvm_ir_audit.py` の IR verifier 成功は証明ではない。生成された実 IR と意味論、各 lowering/optimization pass との refinement を証明する。
- ターゲット・LLVM バージョン・許容する外部仮定を固定して記録し、変化時は証拠を失効させる。
- 完成条件：両処理系から生成し得る全 IR と最適化後の動作について、対応定理と独立検査可能な証拠が存在する。未対応経路を拒否する負例検証がある。

### 3. ビルド依存関係の完全性検証
- `scripts/build_closure_graph.py`, `build_dependency_audit.py`, `build_depfile_verifier.py`, `build_observation_correlation.py`, `verify_traced_build.py` 等を統合する。
- ソース・生成コード・依存ライブラリ・トランジティブな実行ファイル・ツールチェーン・環境変数・OS/CPU/リンカ・ビルドスクリプトを追跡。ビルド中の実際の reads/execs との照合を行い、depfile に現れないものも扱う。
- 動的入力と生成物の因果関係、再生成の再現性、変更された依存・未追跡ファイル・symlink/大小文字/パス別名・重複出力・循環・プロセス外入力の欠落を拒否する。
- 宣言した DAG のハッシュが合っただけでは `observed_inputs_complete` を真にしない。完全性は独立した trusted tracing と trace coverage の証拠が必要。
- 完成条件：**実ビルドの全入力の閉包**が監視により裏付けられ、未観測・未追跡の依存が存在すると失敗する。Windows と対象環境での検証を実施する仕組みを整備し、負例テストが通る。

### 4. 実装と形式証明を結ぶ独立検証器
- `scripts/whole_compiler_proof.py` の source-derived 義務一覧と `scripts/refinement_transition.py` の独立 verification gate を、実在する意味論証明 checker に接続する。
- 証拠は義務 ID・対象 compiler/phase/goal・完全なソース/ビルドスナップショット・target semantics・公理・kernel-checked 定理を cryptographically bind する。
- 文字列 PASS、ユーザー提出 JSON の自己申告、定理名、hash 一致、限定モデルの Lean ビルド成功だけで `REFINEMENT_PROVEN` に遷移させない。
- 偽造、重複、改ざん、欠落、古い証拠、証明にない仮定、検査対象外のソース、新規コード追加を fail-closed に拒否する。
- `scripts/acceptance_report.py` と `run_everything.py` を実際の検証結果に接続し、34件（jwc/Rust × S01〜S12 の実証24件、G1〜G5 の形式10件）を個別に判定。証明義務の網羅性と数学的妥当性を混同しない。
- 完成条件：第三者が再現可能な checker によってのみ全義務が解消され、両処理系34件の根拠が生成される。偽証拠が成功を生まない。

## 現状・誤解してはいけない点
- PR #30 は多数の検証補助コード、Lean 小モデル、証明義務の生成、負例テストを含むが、**4機構すべて未完成**。完成報告は **0/4**。
- `whole_compiler_proof.py` の全件インベントリは source coverage の監査であり、意味的正しさの証明ではない。
- `refinement_transition.py` は production `VERIFIERS = {}` のため、正式な全実装 refinement を認証できない。
- `build_closure_graph.py` の `DECLARED_CLOSURE_VERIFIED` は宣言された DAG の検証だけ。`observed_inputs_complete = False`。
- LLVM generation gate の現行検査は限定的なソーステンプレート照合。コンパイラ全体の LLVM semantics proof ではない。
- PR に追加した局所監査改善もすべて「部分実装」であり、機構完成・ユーザー実機テスト開始の根拠にはならない。
- 最終受入では個々のチェックの戻り値と「数学的証明が成立した」という主張を明確に分離する。証明不足なら最終受入は成功にしない。

## 実装担当向けの進め方
1. まず jwc/Rust/ランタイム/LLVM emitters/テストの実行可能な実装表面を確定し、全件の proof coverage と明示的な仕様上の制約を整理。
2. 各 proof boundary に対する形式仕様・pass relation・implementation extraction/correspondence の設計と実装。
3. 独立 checker、ビルド閉包 monitor、LLVM semantics への接続を実装し、生成証拠の鮮度・真正性・全義務の網羅性を監査。
4. 単体テスト、改ざん負例、仕様変動、反例、クロスコンパイラ差分、CI/Lean build、セルフレビューを実施して修正。
5. 4機構の完成条件が成立して初めて各1回の完成報告。その後にユーザー実機テスト。テスト結果に応じ、コードと証明を直す。

### 最低限のユーザー側実機入口（4機構完成後のみ）
```powershell
py scripts/verify_all_local.py
```
ログ/レポートを分析して欠陥が出た箇所を修正。報告やテスト用証拠を PASS に書き換えるだけの対処は絶対に行わない。
