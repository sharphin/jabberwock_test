# パッケージ（alice）

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

どれも、今のフォルダから上へたどって alice.toml を探す。パッケージを使わないときは今までどおり `jabberwock ファイル.jw` / `jabberwock build ファイル.jw`。
エラーの位置は、パッケージの中のファイルならパッケージからの道筋で出す（`src/util.jw:3:5`）。

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

- git やネット上の置き場からの取得、版の指定と依存の解決、lockfile
- jwc（自己ホスト版）のパッケージ
