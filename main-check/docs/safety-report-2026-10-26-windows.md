# 2026-10-26 Windowsアップロード結果の確認

## 入力と確認方法

- ユーザーが実行してアップロードした `safety-report.json` を読み取り、ソースと照合した。
- レポート対象: `8cda15abba7430ec77d5937a2c3e2f2f3a5a0e37`（PR #7 の元ブランチ `safety-ci-resume`）。
- 実行時刻: 2026-10-26 20:00:31〜20:01:18 JST（レポート内はUTC）。
- Windows 11 AMD64、Python 3.13.2、Rust/Cargo 1.99.1。
- 入力SHA-256: `f1b1a5fae991a4168d937a5174d86eb61024ef1c3181825f95f9fd0d7d2eaf6e`。
- この修正作業では、ユーザーとの分担に従い **ビルド・テスト・clippy・Lean検査を実行していない**。
  生成ファイルの更新と差分レビューのみ実施した。以下の成功・失敗はアップロードされた変更前の結果であり、修正後の結果ではない。

## 実測された結果

レポートは12項目すべてを含む。25プローブの表示は `PROBE_PASS` 3、`FAIL` 1、`NOT_RUN` 21。
ただし成功表示の1つ（native退役検査）は内部でスキップされていたため、native実行の成功証拠にならない。

| 項目 | レポートの表示 | 読み取り |
|---|---|---|
| 1・2・4・7・10・11・12 | FAIL | 共通の `cargo-test` の失敗1件が波及。7つの独立した欠陥という意味ではない |
| 3・5・6・8・9 | NOT_RUN | 対応OSまたは必要ツールを欠き未実行 |

Rustのdebug/releaseビルドは成功した。通常の `cargo test --no-fail-fast` は終了101で、
ログに表示された集計は362成功・1失敗。唯一の失敗名は `runtime_is_up_to_date`。
stdout/stderrに切り詰めはない。ただしclangなしで早期returnしたnative/jwc検査も成功件数に含まれるため、
362件をnativeや自己ホスト版の合格件数として扱わない。

`retirement` のstderrには次の両方がある。

```text
retirement native test not executed: clang unavailable
retired guard test not executed: clang unavailable
```

そのため、このプローブの `PROBE_PASS` は未実行の誤表示。
別途、Rust内部の退役テスト5件と `external_file_effect_runs_exactly_once` は、アップロードされたログでは成功している。
専用の16構成の副作用検査や、資源枯渇・巨大配列の再試験は今回実行されていない。

## 今回の修正

1. `jwc/runtime.jw` を現在の `runtime/jw_rt.c` から再生成した。
   PR #13 の `jw_oom()` と7か所の確保失敗処理を自己ホスト版にも埋め込む。
2. `scripts/sync_runtime.py` を追加し、ビルド・テストを伴わずに再生成できるようにした。
   既存の同期テストは比較を続ける。レポート用ランナーから `JWC_REGEN` を除き、検査中に不一致を修復して隠さない。
3. native退役プローブをclang必須とし、ない場合は `NOT_RUN` にする。
   clangを必須とするCargoプローブには `JW_REQUIRE_RETIREMENT_TESTS=1` を付ける。
4. 通常のRust debug/release検査はclangなしでも実行し、単一JSONの `coverage_notes` にnative検証の欠落を残す。
   Windowsでの `jwc-local.exe` の存在確認を共通化した。
5. clang不在の退役検査・通常Rust検査、厳密な退役実行、検査中の自動再生成禁止について回帰テストを追加した（未実行）。

## 再確認に必要なこと

修正ブランチまたはマージ後のブランチで、ユーザー側から実行する。

```sh
python scripts/test_safety_report.py
python scripts/safety_report.py --output target/safety-report.json
```

Windowsのnative検証にはclangとCのリンク環境、形式モデルにはLean 4.24.0/lakeが必要。
Linux専用検査はLinux環境で別途実行し、Windowsの結果と併せて確認する。
clangがインストール済みでも、実行したシェルのPATHから見つからなければ未実行になる。

**修正後は再テスト待ち。12項目全体の合格、項目8・9の解消、形式証明の完了を今回の結果から認定しない。**
