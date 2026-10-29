# パッケージ（alice）

ライブラリ全体の現状・既存設計・jwc対応の順序は [ライブラリシステム](library-system.md) を参照。
このページは現在のaliceの使い方を記す。現行のbuild/run/testはRust実装を使用し、jwcとのパッケージ連携は未対応。

パッケージの道具 `alice`（jabberwock の cargo にあたるもの。コンパイラ `jabberwock` とは別のコマンド）。今は手元のフォルダの依存先だけを扱う（ネットからの取得・版の指定はまだ）。

## 作る

```
alice new app          # 実行ファイルのパッケージ（src/main.jw）
alice new mathx --lib  # ライブラリ（src/lib.jw と、関数ごとのテスト tests/add.jw）
```

```
app/
  alice.toml
  .gitignore        /target
  src/main.jw       入口（ライブラリなら src/lib.jw）
  src/*.jw          モジュール（`use geom;` で src/geom.jw）
  tests/*.jw        alice test で 1 本ずつ動かす
  target/           build の出力
```

パッケージの名前はフォルダの名前（英字・数字・_ で、英字か _ から始める）。

## alice.toml

```toml
[package]
name = "app"

[dependencies]
mathx = { path = "../mathx" }   # 手元のフォルダ（alice.toml からの道筋）
```

- 読むのは `[package]` の `name` と、`[dependencies]` の `名前 = { path = "..." }`（`名前 = "..."` も path とみなす）だけ。ほかの節とキーは読み飛ばす。`#` から行末は注釈。
- 依存先の名前は、依存先の alice.toml の `name` と同じにする。依存先は `src/lib.jw` を持つライブラリであること。
- 依存先の依存先も読む。同じ名前の依存先が 2 つの場所にあるとエラー。

## モジュールの名前

名前の空間はパッケージごと。

- 自分の `src/*.jw` は、ファイル名がモジュール名（`main` と `lib` は入口なので除く）。
- 依存先 `mathx` の `src/lib.jw` がモジュール `mathx`（`use mathx;` → `mathx.add(1, 2)`）。
- 依存先の `src` のほかのファイルは、そのパッケージの中からだけ見える（`mathx` の中で `use sq;` と書ける。外からは `use sq;` で読めない）。
- `use 名前;` は、まず自分のパッケージの中を探し、なければ依存先のパッケージを探す。なので app の `util` と mathx の `util` は別のモジュールとして共存できる。
  mathx の中の `use util;` が、mathx にない util を app から借りることはない。
- 自分のモジュールと依存先のパッケージが同じ名前ならエラー（`use 名前;` がどちらか決まらないため）。
- `use` できるパッケージは、自分の alice.toml の `[dependencies]` に書いたものだけ。依存先の依存先を使いたいときは、自分の alice.toml にも書く
  （同じフォルダを指していれば 1 つのパッケージとして読む）。

## コマンド

| コマンド | すること |
|---|---|
| `alice build [--release]` | src/main.jw から target/debug/名前（`--release` は target/release）を作る。実行ファイルがソース・依存先・alice.toml より新しければ作り直さない |
| `alice run [-- 引数...]` | src/main.jw をインタプリタで動かす（clang がいらない） |
| `alice run --native [-- 引数...]` | build してから実行ファイルを動かす（`--release` も使える） |
| `alice test [名前の一部]` | tests/*.jw を名前の順にインタプリタで動かす（下の「テスト」）。名前の一部を書くと、その文字を含むテストだけ |
| `alice package-map [-o ファイル] [入口.jw]` | 現在のパッケージとpath依存を解決し、jwc用の情報を書き出す。省略時の入口はsrc/main.jw、出力は標準出力 |

どれも、今のフォルダから上へたどって alice.toml を探す。パッケージを使わないときは今までどおり `jabberwock ファイル.jw` / `jabberwock build ファイル.jw`。
エラーの位置は、パッケージの中のファイルならパッケージからの道筋で出す（`src/util.jw:3:5`）。

## jwcでローカルライブラリを使う

パッケージのフォルダで、構築済みのaliceとjwcを使う:

```sh
alice package-map -o packages.map
jwc --ir --package-map packages.map
jwc build --package-map packages.map -o app
./app
```

`--package-map` は入口のファイルも指定するので、`.jw` ファイルを別に渡さない。
`--ast` / `--typed` / `--ir-raw` / `--ir` / `--ll` / `--ll-checked` とbuildで使える。
別の入口を使う場合は `alice package-map -o packages.map tests/example.jw` のように指定する。
入口は現在のパッケージ内の実在する `.jw` ファイル。mainのないテスト関数の実行は、まだalice側でjwcへ接続していない。

依存先の探索、名前の重複、パスの正規化には既存の `Project::load` を使う。
jwcは、パッケージ内のモジュールを先に探し、なければそのパッケージの**直接依存**を探す。
他の依存先が読み込み済みでも、この確認を省略しない。
異なるパッケージの同名モジュールは別の型・関数として扱い、`pub` と循環の検査はjwcで行う。
パッケージ使用時の診断には実際のソースのパスを表示する。

マップは `jwc-package-map-v1` を先頭に持つUTF-8のタブ区切りで、`entry`、`module`（既存の問い合わせ名と絶対パス）、
`allow`（使用元の空間と直接依存の名前）の行からなる。ルートの空間は空文字。
タブ・改行・NULやUTF-8以外を含むパスは書き出せない。
これはローカルのビルド入力であり、権限を制限する仕組みではない。
ファイル追加・移動、manifest・依存変更時にはマップを作り直す。ソース本体は毎回jwcが読む。
絶対パスを含むので配布物としてコミットせず、移動先で再生成する。

現段階では、aliceのbuild/run/testは従来どおりRust実装を使う。
上記はjwcによるコンパイルの接続であり、aliceの自動選択・実行・テストへの接続は次の段階。

## テスト

tests/*.jw には 2 つの書き方がある。

```
// tests/add.jw: main のないファイル。fn test_名前() を 1 本ずつ別に動かす
use mathx;

fn test_add() {
    assert(mathx.add(2, 3) == 5);
}

fn test_zero() {
    assert(mathx.add(0, 0) == 0);
}
```

```
// tests/hello.jw: main のあるファイルは 1 本のテスト。tests/hello.out があれば、出力が同じでなければ失敗
fn main() {
    print("hi");
}
```

- 失敗するのは、エラー（`assert` の失敗・ダングリングなどの実行時エラー・型のエラー）と、0 でない `exit`。
- テストの関数はトップレベルの `fn test_名前()`（引数も戻り値もなし）。ほかの関数は手伝いとして使える。
- 1 本ずつ別に動かすので、あるテストの失敗やリークはほかのテストに響かない。表示は `tests/add.jw::test_add`。
- main と `fn test_...()` は同じファイルに書けない。

## まだやっていないこと

将来の配布・管理は **P2P＋ブロックチェーン方式**で構築する方針。
詳細と現在のローカル依存との関係は [ライブラリ配布・管理の方針](library-system.md#ライブラリ配布管理の方針) を参照。

- git やネット上の置き場からの取得、版の指定と依存の解決、lockfile
- aliceのbuild/run/testからjwcを選択する接続
