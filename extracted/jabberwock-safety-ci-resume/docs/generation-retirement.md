# 世代上限での退役: 実装と再検証

2026-10-26。[形式仕様](formal-semantics.md) の退役規則をRust/IR/C runtime/生成LLVM/jwcへ反映した。
対象課題は [M3/O4/C1/C3](verification-plan.md)。言語全体の形式検証の完了ではない。
実行先はPR #7の `safety-ci-resume`。Windowsを含むGitHub Actionsの結果と、ローカルLinuxの結果を区別する。

## 実装した規則

- 世代の無効化に周回する加算を使わない。
- 解放時に上限へ達した識別子は退役させ、空き表へ戻さない。別の区画を確保して処理を継続できる。
- 生きた区画の参照をこれ以上安全に無効化できない場合は、資源エラー「世代番号の上限に達しました」で停止する。
  生きたC payloadの移し替えは今回実装しない。停止は採用した仕様で許す動作であり、継続したと偽らない。
- 退役先のrefは世代値が一致しても拒否する。古いrefが見る検査用metadataは安全に保持する。
- 通常のownのムーブや要素代入など、もともとrefを無効化しない操作の規則は変えない。

Rustでは `Slot.retired` を持ち、`Heap.advance_generation` が上限前の加算と退役を分ける。
`free`・`vacate` は退役したslotを再利用しない。`vacate` は中の所有値を壊さず取り出すため、
Rustの送信は退役した古いslotを残して新しいHeapのslotへ所有責任を渡せる。
`list_moved` の上限到達はDiagnosticとして返し、ASTとIRの両方がそのエラーを伝播する。
slot番号とmetadataはHeapの終了まで保持し、その間に新しい割当てへ同じ番号を使わない。

Cでは8 byteヘッダーとrefの配置を保つ。bit62は既存のLIST_SHARED、bit0/1は既存の解放/借用印。
下位62 bitのcounterが状態印へはみ出さないよう確認し、全1のwordを退役専用markerにする。
markerは正規の生きたrefの世代として発行しない。通常の世代一致検査にもmarkerの明示的な拒否を加えた。

| 経路 | 実装した上限処理 |
|---|---|
| `block_free` | 解放+次の再利用の余地がなければ早期退役。空き表へ戻さない |
| `remember_reuse` / 小区画・大区画の割当て | 再利用の加算を確認し、上限の区画は退役させて飛ばす |
| `jw_send_block` | +2を確認し、不能なら退役後に資源エラーで停止 |
| `jw_list_moved` | 借用印を消して+4する従来の式と等しい、借用中のlive wordへの+2を上限確認付きで行う |
| Rust版LLVM / jwc版LLVMの `check_ref` | 世代の不一致に加え、退役markerそのものを拒否 |
| `jw_view_target` | ownの写しも同じく退役markerを拒否 |
| `freed_at` | markerと一致する壊れたrefで加算を周回させず、退役payloadを診断の値として読まない |

世代ヘッダーは引き続きatomicのrelaxed/LLVM monotonicで読み書きする。
退役したC区画はプロセス終了まで保持する表へ登録する。
既に無効化されたpayloadの先頭語を保持表のリンクに転用し、他のスレッドの退役もatomic CASで公開する。
保持表の削除や識別子の再割当ては行わず、古いrefが参照し得るアドレスをOSへ返さない。
全スレッドの退役区画数をatomicで別計上し、`JW_RETIREMENTS=1` では正常終了時に `[retired] N` を表示する。
これはlive payloadの数やRSSの上限とは別。保持メモリの上限制御は既存項目8に残る。

`jwc/runtime.jw` は既存の生成テストでCから再生成する。手で二つの実装を別々に修正しない。
新しい演算・FFI・利用者向けの世代変更APIは追加していない。

## 境界テスト

Rustの小さい上限は `cfg(test)` のHeapだけで設定し、プロセス環境変数を変更しない。
Cはharnessを専用にコンパイルし、`JW_TEST_GENERATION_LIMIT` を3・7に制限した場合と実際の上限で確認する。
状態bitを表現できない上限はコンパイル時に拒否する。通常の処理系ではこのテスト用defineを指定しない。

| テスト | 確認すること |
|---|---|
| Rustの解放 | 上限0/1/3/u64::MAXで、世代が戻らず、古いslotが空き表・再割当てへ戻らない |
| Rustのvacate・連鎖解放 | 所有値のムーブと子の回収を保ち、各識別子を一度だけ退役させる |
| Rust/IRの生存中の上限 | リスト更新のDiagnosticを伝播し、同じ世代のrefも退役後には拒否 |
| Cの小/大区画 | 最終世代の解放・再利用候補・繰返し割当てで、退役アドレスを再利用しない |
| Cのflag・送信・リスト | 状態bitを保った正常な更新と、上限直前/上限での資源エラーを確認 |
| Cの別スレッドでの解放 | 送信元の古い世代が復活せず、解放・退役と同時のmetadata読取りが安全 |
| Cの保持表 | 退役markerだけを保持し、リストの件数と全スレッドの計数が一致 |
| Cのownの写し | 捕捉世代をmarkerに一致させた不正な写しでもpayloadを読ませない |
| Rust/jwc生成LLVM | 既に無効なrefへmarker一致の破損を注入し、実際の生成コードが拒否して値を出力しない |

生成LLVMへの注入はテスト内だけで行う。通常の言語機能に世代を変更する抜け道は作らない。
参照検査を正当に省略した状態へ任意の破損を注入して「省略した検査がない」と判定する方法は使わない。
必要な検査が残る無効参照のケースで、解析あり/なしのRust版とjwc版を検査する。
既存の固定攻撃・生成入力・verifier・sanitizerの負例も維持する。

```sh
cargo test --lib retirement_tests
JW_REQUIRE_RETIREMENT_TESTS=1 cargo test --test retirement
JW_REQUIRE_RETIREMENT_TESTS=1 cargo test --test jwc jwc_generated_checks_reject_a_matching_retired_generation
JW_REQUIRE_RETIREMENT_TESTS=1 JW_NATIVE=1 cargo test --no-fail-fast
```

`JW_REQUIRE_RETIREMENT_TESTS=1` ではclang不足をskipで成功にしない。
CIの通常・native試験と安全性試験にもこの指定を加え、TSanの3回反復には退役のテストも含める。
clangの標準ヘッダー・runtime・linkerが不足した場合も、言語エラーに置き換えない。
Lean配布に同梱されたclangを誤って選ばないよう、Leanとnative検証のPATHは分離する。

## 項目8・9の再テスト

退役実装と、以前から未解決の資源枯渇・巨大型の問題を分けて記録する。
再現fixtureは [資源枯渇](../tests/safety/resource-exhaustion.jw) と [具体化後の配列](../tests/safety/generic-array-limit.jw)。
配列は [上限ちょうどの正常系](../tests/safety/generic-array-limit-ok.jw) も同じ8経路で受け付けることを確認する。
Linuxの隔離子プロセスでアドレス空間を4 GiBに制限し、ホスト全体を枯渇させずに検証する。
Rust CLIは1 GiBのstackを予約するので、起動前の失敗とプログラム実行中の確保失敗を混ぜない。

```sh
python3 scripts/retest_safety_edges.py --compiler target/debug/jabberwock --jwc /path/to/current/jwc --output /tmp/safety-8-9.json
```

このコマンドは計装なしで実行する。MSan/ASanのshadow領域を制限して起動不能にする試験ではない。
失敗した再現ケースは失敗のまま報告し、未解決事項の検出を「安全性合格」に変換しない。
終了コードは全ケース成功0、ケース不合格1、実行基盤の失敗2。
ケースが成功するよう修正されても、資源上限全体等の契約が未完なら項目全体の完了とは別に扱う。
同じscriptを実行する手動workflow `safety-edges-8-9` も追加した。GitHub側ではまだ実行していない。
GitHubの手動実行はworkflowがdefault branchへ登録された後に利用する。今回のPR元ブランチでは上記scriptをローカルで実行した。
既存1〜7のworkflowと別の結果であり、8/9を再現する間は成功へ変換しない。

| 項目 | 再現した結果 |
|---|---|
| 8: 巨大な文字列確保の失敗 | interpは規定のメモリエラーで終了1。Rust版/jwc版のnativeは内部エラーで終了2となり、契約が一致せず不合格 |
| 9: 型引数の展開後に大きくなる配列 | Rust/jwcのtyped・IR-raw・IR・LLVMの8経路で1 MiB超を受け付けた。不合格 |

この2項目を今回修正済みとは扱わない。退役による新しい資源エラーと、従来の確保失敗の扱いも混同しない。

## 検証の状態と保証の境界

2026-10-26、PR #7元ブランチで、以下のローカルLinux再検証が完了した。
環境はDebian 13 / Linux 6.18.55、Rust 1.99.1、Clang 19.1.7、Lean 4.24.0。
対象ソース・ログのSHA-256と項目8/9のケース別結果は [機械可読の記録](retirement-validation.json) に残す。
これはローカル実行の記録であり、GitHub Actionsの実行証拠ではない。

| 構成 | 結果 |
|---|---|
| 通常のnative全体試験 | runner集計366件成功、失敗0。Rustの退役5件、nativeの退役2件、jwcの退役1件を含む |
| -O2＋LTOの全体試験 | `JW_NATIVE=o2 cargo test --no-fail-fast`、366件成功、失敗0 |
| ASan/UBSan全体 | 366件成功、失敗0。LSanは無効 |
| MSan全体 | 366件成功、失敗0 |
| TSan | 退役を含む45件×3回、計135件成功 |
| Rust生成物のASan/UBSan比較 | -O1、-O2＋LTOの両方で成功 |
| Rust/jwc生成物のASan/UBSan・MSan比較 | 両sanitizer・両最適化構成で成功。jwc自身のbuildも全体試験に含む |
| 固定攻撃・演算の端 | 各生成物比較構成で攻撃78/78件、演算76/76件成功。スキップなし |
| 生成入力 | 各構成で-O1は182/200本、-O2＋LTOは88/100本を実行比較して成功。残る18本・12本は型エラーで、実行比較数へ足さない |
| LLVM監査・verifier | 390モジュールで成功 |
| 堅牢性 | ASan/UBSanで7テスト＋native変異の1テストが成功。変異は150本とnative100本。既存の3秒打切り条件を維持 |
| 検出器の負例 | 生成LLVMのASan/MSan/TSan、CのUBSan/TSanが意図的な欠陥を検出 |
| その他 | Clippy警告0、ベンチの検査21件成功、Leanの22定理と公理依存検査成功 |
| 項目8/9の再現試験 | 19ケース中9成功・10不合格、基盤エラー0、script終了1。上限ちょうどの配列8経路とinterpの確保失敗は成功 |

生成入力のseedは-O1が `20261026000`、-O2が `20261026500`。
全体試験の366件はrunnerの集計であり、各sanitizerの検出例が366個あるという意味ではない。
環境変数で早期returnする既存テストは、対応するsanitizer・堅牢性の専用実行でも確認した。
通常の残存区画数と退役metadataの保持数は別物であり、LSanの代わりにはしない。
Rustのtest runnerはdebugビルド。表の-O1/-O2は生成nativeコードの最適化を指す。

上記のLinux実行範囲で項目1〜7の再検証は成功した。項目8/9は不合格で、**1〜9全項目クリアではない**。
GitHub Actionsは、アカウントの支払い失敗または利用上限に関する表示でジョブが開始されていない。
その赤表示はテスト実行による不合格とは区別し、Windowsの現変更での実行は未実施とする。

ローカルLinuxではLSanが実行基盤の制限でfatal errorとなるため、その実行を中断し、
`ASAN_OPTIONS=detect_leaks=0:halt_on_error=1` でASan/UBSanを別に実行する。
LSanを実行できたことにはせず、言語の残存区画数の検査とも分けて記録する。
MSan/TSanの負例も先に実行し、検出器が欠陥を検出することを確認した。

Leanの22定理は引き続き抽象モデルの証明。
本実装のbyte表現・保持表・具体的な全遷移をLeanへ接続した証明はまだない。
項目8/9、言語全体の型健全性、全コンパイル変換、標準ライブラリ全体も未完了のままである。
