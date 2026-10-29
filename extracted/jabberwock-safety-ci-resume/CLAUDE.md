# CLAUDE.md

自作言語 jabberwock（`own` と世代つき `ref`）。Rust 版コンパイラ（`src/`）・自己ホスト版 `jwc/`・C の実行時ライブラリ（`runtime/jw_rt.c`）・Lean の形式化（`formal/`）。
ここは毎回読む最小限だけ。詳しいことは必要になったときに下の文書を読む。
今の主な目的は形式検証の実行基盤（ツール本体）の完成と、結果の正しい判定（`docs/formal-tool-completion.md`）。

## 守ること（オーナーの指示）

- オーナーとは日本語・平易な言葉で、短く報告する。実行していない検証を「成功」と言わない（成功・失敗・未実行を分ける）。
- 作業ブランチは `safety-ci-resume`。許可なく `main` へマージしない。ドラフト PR は作らない。
- GitHub Actions は公開リポジトリ `jabberwock_test` で回す（公開なら時間の上限がないため）。
- push 前に `cargo test --no-fail-fast` と `cargo clippy --all-targets`。IR・llvm.rs・jw_rt.c を変えたら `JW_NATIVE=1 cargo test --no-fail-fast` も。
- 外部クレートは使わない。Rust 1.65 で動くこと（`is_some_and`・`div_ceil`・`is_multiple_of` などは使わない）。`cargo fmt` は全体にかけない。周りの書き方（日本語のコメント）に合わせる。
- テストケースや形式証明のケースを頼まれずに量産しない。既存テストの実行・検証器の実行・新しいケースの作成は別のこととして扱う。
- 安全性の合格は、合意した基準（定めた仕様・条件の下で全テスト合格。jwc 版を優先、jwc と Rust・Windows と Linux の結果は混ぜない）で判定する。
  別の証明要求を勝手に足さない。未証明は `NOT_PROVEN`、検証器の故障は `TOOL_ERROR` / `INVALID_EVIDENCE`。
- Rust 版（src/）は凍結。新しい機能は jwc にだけ足す。Rust 版は jwc を最初にコンパイルする役と突き合わせの正解の役（詳しくは docs/design-decisions.md）。
- 仕様を変えたら同じコミットで `docs/spec.md` を直す。決めたことは `docs/design-decisions.md` に書く。

## よく使うコマンド

- `jw_rt.c` を変えたら `jwc/runtime.jw` を作り直す: `JWC_REGEN=1 cargo test --test jwc runtime_is_up_to_date`（または `python3 scripts/sync_runtime.py`）
- 形式検証の一括: `python scripts/formal_tool.py`（結果は `target/formal-tool/summary.json`）
- ベンチマーク: `python scripts/benchmark_report.py [--full]`、検査の数の見張り: `bench/checks.sh`

## 壊してはいけない前提（全文は docs/claude-reference-archive.md の「大事な不変条件」）

- interp・IR（exec）・実行ファイル（llvm.rs + jw_rt.c）・jwc は、出力・エラーの文と位置まで一字一句そろえる（今ある機能について）。片方を変えたら全部直す
  （デバッグビルドは全テストで interp と IR を突き合わせる。`JW_NATIVE=1` で実行ファイルとも）。
- 世代の検査を消すのは、解析が証明できたときだけ。分からなければ実行時検査を残す（安全側）。
- 世代が上限に達した区画は退役させ、使い回さない（`GEN_RETIRED` は全 1。ref には発行しない）。
- `--no-gen-checks` / `--no-bounds-checks` / `--unchecked` は安全でない測定用。ふつうのビルドの速さと混ぜない。

## 必要なときに読む文書

| 知りたいこと | 文書 |
|---|---|
| コードの地図・不変条件の全文（どのファイルが何をするか） | `docs/claude-reference-archive.md` |
| 形式検証・ベンチマークの運用、検証スクリプトの役割、レビューの注意 | `docs/agent-guide.md` |
| 言語仕様 / 設計の決めごと / IR の設計 | `docs/spec.md` / `docs/design-decisions.md` / `docs/ir-design.md` |
| 安全性の確かめ方と残り / 合格の判定の仕組み | `docs/safety-testing.md` / `docs/safety-judgement-mechanisms.md` |
| 世代の退役 / 形式化の範囲 / 検査を消す判断の証明 | `docs/generation-retirement.md` / `docs/formal-semantics.md` / `docs/elision-proof.md` |
| ベンチマークの手順 | `docs/benchmark-operations.md` |
