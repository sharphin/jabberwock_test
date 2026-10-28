# ライブラリシステム：現状・設計・次の作業

更新日: 2026-10-28

ライブラリ開発の入口。今後の開発対象は **jwc版** とし、既存のaliceとモジュール機能を利用して進める。
この整理はドキュメントのみ。下記の設計済み機能を、実装済みとは扱わない。

## 現在使えるもの

| 対象 | 実装状況 | 確認先 |
|---|---|---|
| jwcのモジュール | 同じディレクトリの `.jw` を `use` で読み、`pub` の公開範囲を検査する | `jwc/modules.jw` |
| jwcの単体ビルド | LLVM生成とClangによる実行ファイルのビルド | `jwc/main.jw` |
| aliceのローカル依存 | `alice.toml`、path依存、`src/lib.jw`、パッケージごとの名前空間 | [現在のパッケージ仕様](packages.md)、`src/project.rs` |
| aliceのbuild/run/test | 現在はRust実装のコンパイル・実行APIを使用 | `src/alice.rs` |
| ファイルの一括入出力 | 組み込みの `read_file` / `write_file` | [言語仕様](spec.md) |

jwcとaliceのパッケージ連携は未実装。`alice new --lib` でライブラリを作れることと、
そのパッケージをjwcでビルド・テストできることは区別する。

## 既存の詳細設計を読む順序

詳細設計は `docs/stdlib-v1-design` ブランチにある。
下表は確認済みのコミット `68e5977df5dda141423233fab7982068949edaa7` に固定した参照。
現在の作業ブランチに存在しない設計を、リンク切れや未設計と取り違えないために掲載する。

| 順序・用途 | 設計書 |
|---|---|
| 全体像・設計成果物の一覧 | [全体方針][overview]、[設計状況][status] |
| 読み込み・名前空間・配布 | [標準ライブラリの読み込み][loading]、[カタログと登録表][catalog] |
| aliceとjwcの接続 | [alice設定][config]、[compiler連携][compiler]、[ネイティブ依存とビルド][native] |
| 所有・解放・外部境界 | [共通資源管理][resources]、[資源ABI][abi] |
| 公開API・型・操作ID | [公開API][apis]、[個別C操作][operations]、[初期版の契約][contracts] |
| ファイル・通信・DB | [ファイル入出力][files]、[通信][net]、[PostgreSQL][postgres] |
| Windowsの描画・GUI | [描画・GUI][drawing]、[GUI内部仕様][gui] |

設計内の `use std.fs`、`as`、`bundled`、`--compiler-info`、新しいalice設定・コマンドは提案仕様。
現在利用可能なコマンドは [packages.md](packages.md) を参照する。
登録表・schema・Cヘッダーなどの設計データは、上表のカタログ設計から辿れる。

### 過去の設計から更新する方針

- 新機能の開発先はjwc。過去の「Rust/IR/native/jwcすべてへ実装」は、今後の一律の実装義務としない。
  Rust版は既存機能の比較・構築に使い、新機能をRustへ重複実装することを前提にしない。
- alice自体の実装言語を変更することと、aliceがjwcを呼ぶことは別作業。まずjwcとの連携を扱う。
- 過去の「安全性1〜7」は、現行のコンパイラ別12項目と5目標の検査へ読み替える。
  形式検証は既存の別ツールで扱い、コンパイラ別の結果を保持する。
- 現在の作業順序は次節を優先する。既存API・資源ABIの詳細は設計資料を使い、変更時は理由を記録する。
- GUIはWindows先行。ゲーム描画にも利用できるウィンドウ・入力・描画層をGUI部品から分離する。
  Linux GUIは今後検討する。

## 次の実装順序と到達点

| 段階 | 作業 | 到達点 |
|---|---|---|
| 1. ローカルライブラリ | jwcへパッケージの所属・直接依存・公開範囲を渡す。既存の解決規則を利用する | 別パッケージの `.jw` ライブラリをjwcでコンパイルでき、非公開・未宣言依存・循環を拒否できる |
| 2. alice連携 | jwcを選択したbuild/run/test、診断、出力管理を接続する | ライブラリを使うアプリとテストをjwc生成物で実行できる。Rustへ黙って切り替えない |
| 3. 標準配布・外部資源の基盤 | カタログ、標準モジュール、資源型、所有・解放、ABI、リンク・DLL配置を接続する | 必要な依存だけを選び、途中失敗やABI不一致を診断し、資源を正しく回収できる |
| 4. 個別ライブラリ | ファイル入出力を先に実装し、通信・PostgreSQL、Windowsウィンドウ・描画・GUIへ進む | 各設計の公開APIと実環境での動作を確認できる |

段階3では共通資源管理を模擬資源で確認してから実OS資源へ接続する。
PostgreSQLは既存設計のlibpq接続、Windows GUIはWin32・Direct3D 11・DirectWrite等を土台にする。
初期版にネット配布・依存バージョン解決・任意FFI・HTTP等を追加することは、この整理では決めない。

## コンパイラへの影響

| 範囲 | 主な変更箇所 | 中核への影響 |
|---|---|---|
| ローカル `.jw` ライブラリ | モジュール探索、パッケージ所属、名前解決、alice連携 | 既存の型・所有権・参照規則、最適化、コード生成を利用する方針。名前解決は型の同一性にも影響するため回帰確認する |
| 標準importと別名 | 字句・構文、モジュールID、公開範囲、標準配布 | `use std.fs` / `as` を導入する段階では、読み込み以外に構文解析も変更する |
| 外部資源・DLL | 資源型、内部操作、IR/LLVM、Cランタイム、リンク | 型検査・所有／参照の扱い・コード生成にも変更が及び得る。中核に無影響とはしない |
| ライブラリ本体 | `.jw` のAPIとOS／libpq等への接続 | 通常の言語機能で書ける部分はそれを使い、外部境界で必要な処理を共通基盤へ集約する |

## 変更後の検査

**変更後のjwcで、追加機能も含めた全検査が通れば、安全性検査に合格したと判断する。**
既存の12項目と5目標を使い、ライブラリ機能をその検査対象へ組み込む。
新しいテスト基盤を一から作り直す計画にはしない。

- パッケージの隔離・型の同一性・公開範囲、依存変更時の再ビルドを確認する。
- 資源の二重解放・解放後利用・途中失敗・DLL不足／不一致・副作用の重複実行を検査する。
- WindowsとLinux、jwcとRustの結果を分ける。新機能未対応の実行器の結果をjwcの代わりにしない。
- 各段階で対象ソースと実行条件を記録する。以前の合格を変更後の実装へ転用しない。

既存の実行コマンド（リポジトリ直下）:

```powershell
# Windows：jwcの実用検査
.\scripts\safety_windows.cmd --compiler jwc

# 形式検証・判定基盤の一括実行（コンパイラ別の結果を保持）
py -3 scripts/formal_tool.py
```

```sh
# Linux：jwcの実用検査
python3 scripts/safety_linux.py --compiler jwc
python3 scripts/formal_tool.py
```

環境・出力・結果の読み方は [コンパイラ別検査](compiler-safety-matrix.md)、
[形式検証基盤](formal-tool-completion.md)、[言語安全性契約](language-safety-contract.md) を参照する。
ライブラリ向けの検査は実装に伴って組み込むもので、上記コマンドで未実装機能を検査済みにはならない。

[overview]: https://github.com/sharphin/jabberwock/blob/68e5977df5dda141423233fab7982068949edaa7/docs/standard-libraries.md
[status]: https://github.com/sharphin/jabberwock/blob/68e5977df5dda141423233fab7982068949edaa7/docs/stdlib-design-status.md
[loading]: https://github.com/sharphin/jabberwock/blob/68e5977df5dda141423233fab7982068949edaa7/docs/standard-library-loading.md
[catalog]: https://github.com/sharphin/jabberwock/blob/68e5977df5dda141423233fab7982068949edaa7/docs/catalog-registry.md
[config]: https://github.com/sharphin/jabberwock/blob/68e5977df5dda141423233fab7982068949edaa7/docs/alice-configuration.md
[compiler]: https://github.com/sharphin/jabberwock/blob/68e5977df5dda141423233fab7982068949edaa7/docs/compiler-integration.md
[native]: https://github.com/sharphin/jabberwock/blob/68e5977df5dda141423233fab7982068949edaa7/docs/alice-native.md
[resources]: https://github.com/sharphin/jabberwock/blob/68e5977df5dda141423233fab7982068949edaa7/docs/resource-management.md
[abi]: https://github.com/sharphin/jabberwock/blob/68e5977df5dda141423233fab7982068949edaa7/docs/resource-abi.md
[apis]: https://github.com/sharphin/jabberwock/blob/68e5977df5dda141423233fab7982068949edaa7/docs/public-apis.md
[operations]: https://github.com/sharphin/jabberwock/blob/68e5977df5dda141423233fab7982068949edaa7/docs/native-operations.md
[contracts]: https://github.com/sharphin/jabberwock/blob/68e5977df5dda141423233fab7982068949edaa7/docs/stdlib-v1-contracts.md
[files]: https://github.com/sharphin/jabberwock/blob/68e5977df5dda141423233fab7982068949edaa7/docs/file-io.md
[net]: https://github.com/sharphin/jabberwock/blob/68e5977df5dda141423233fab7982068949edaa7/docs/networking.md
[postgres]: https://github.com/sharphin/jabberwock/blob/68e5977df5dda141423233fab7982068949edaa7/docs/postgresql.md
[drawing]: https://github.com/sharphin/jabberwock/blob/68e5977df5dda141423233fab7982068949edaa7/docs/gui-rendering.md
[gui]: https://github.com/sharphin/jabberwock/blob/68e5977df5dda141423233fab7982068949edaa7/docs/gui-internals.md
