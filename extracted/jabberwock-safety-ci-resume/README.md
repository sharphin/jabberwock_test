# jabberwock

`own` と世代付き `ref` だけで「速い・簡単・安全」の隙間を狙う、小さな言語。インタプリタと、clang を使って実行ファイルを作るコンパイラ（`jabberwock build`）がある。

## 動かす

```
cargo run -- examples/counter.jw
cargo run -- --stats examples/counter.jw     # ref の検査をどれだけ消せたか表示
cargo run -- --no-elide examples/counter.jw  # 静的解析なし（全部実行時に検査）
cargo run -- --ir examples/fib.jw            # 実行せず IR（中間表現）を表示
cargo run -- --ll examples/fib.jw            # 実行せず LLVM IR を表示（clang でコンパイルできる）
cargo run -- build examples/fib.jw           # 実行ファイル fib を作る（clang が要る。docs/native.md）
cargo run -- build --release examples/fib.jw # 実行時エラーは使った位置だけ出す
cargo run -- examples/words.jw "a b a"        # ファイル名のあとの引数は args() で受け取れる
cargo test
```

パッケージは別のコマンド `alice`（cargo にあたるもの。`cargo build --release` で target/release/alice もできる。くわしくは [docs/packages.md](docs/packages.md)）:

```
alice new app                 # app/alice.toml と src/main.jw を作る（--lib でライブラリ）
cd app
alice run                     # src/main.jw を動かす（--native で実行ファイルを作って動かす）
alice build --release         # target/release/app を作る（変わっていなければ作り直さない）
alice test                    # tests/*.jw を動かし、tests/名前.out があれば出力と比べる
```

ライブラリ開発の現状・設計書一覧・jwc対応の順序は [ライブラリシステム](docs/library-system.md) にまとめています。

## 安全性テスト・形式検証

リポジトリ直下から実行する。Windows:

```powershell
.\scripts\safety_windows.cmd
py -3 scripts/formal_report.py --compiler jwc --output target/formal-jwc-report.json
py -3 scripts/formal_report.py --compiler rust --output target/formal-rust-report.json
```

Linux:

```sh
python3 scripts/safety_linux.py
python3 scripts/formal_report.py --compiler jwc --output target/formal-jwc-report.json
python3 scripts/formal_report.py --compiler rust --output target/formal-rust-report.json
```

言語機能として保証を目指す安全性と、アプリケーション側の責任範囲は [言語安全性契約](docs/language-safety-contract.md) に定義する。
形式検証は実装済みの局所定理を検査する。全コンパイラの安全性を証明済みとはしない。
必要環境・一括実行・終了コードは [形式検証コマンド](docs/formal-commands.md)、
実用12項目×コンパイラ＋5目標は [検査構成](docs/compiler-safety-matrix.md) を参照。

## 言語の見た目

```
fn inc(ref mut i64 r) {   // 型は「型 名前」の順。ref mut i64 = 書き換えてよい i64 の借用
    *r += 1;
}

fn main() {
    let mut count = own 0;   // own: 唯一の所有者。書き換えるので mut
    inc(ref mut count);      // ref mut: 書き換えてよい借用（ライフタイム注釈は存在しない）
    print(*count);           // 1
}
```

- ポインタは `own` と `ref` の2種類だけ。生ポインタはない。
- `own x` を読むと **ムーブ**。元の変数はそれ以降使えない。
- `ref p` は **世代付き参照**。所有者が解放されると、使った瞬間に確実に検出される。
- スコープを抜ける・`drop(x)`・再代入で `own` は解放される。中の `own` も連鎖して解放。
- 型は「型 名前」の順で、関数の引数・戻り値・構造体のフィールドに書く（`fn i64 add(i64 a, i64 b)`）。
  `let` は書かなくてよい（初期値から決まる）。書くなら `let i64 mut x = 0;`。
- 型は整数（`i8` `i16` `i32` `i64` `u8` `u16` `u32` `u64` `usize`）・`f64`・`bool`・`str`・構造体・enum・配列 `i64[3]`・リスト `i64[]`・辞書 `i64[str]`・関数の型・`own T`・`ref T`・`ref mut T`。
  型の違反と mut の違反は実行前に見つかる。
- 数の書き方は使う先の型に合わせる（`let u8 b = 200;`）。入らない数（`let u8 b = 300;`）はエラー。型が決まらなければ `i64`。
- 値が欠けない整数の変換（`u8` → `i64` など）は自動。欠けうる変換は `x as u8` と書く（範囲の外は一周する）。
- 戻り値のある関数は、本体の最後の `;` のない式が戻り値になる（`return 値;` でもよい）。
- 文の終わりには `;`。改行はただの空白。
- 変数は普段は書き換え不可。書き換えるものだけ `let mut`（引数は `fn f(i64 mut a)`）。
- `ref` は読むだけ、`ref mut` は書き換えてよい借用。`ref mut` で借りられるのは mut の持ち主だけ。
- `x += 1` などの複合代入と、文としての `i++` / `i--` がある（`print(i++)` のように式の中では使えない）。
- 組み込み関数は `print` `eprint` `drop` `assert` `read_line` `args` `spawn` `chan` `exit` `read_file` `write_file` `run_command` `chr`。全部の決まりは [docs/spec.md](docs/spec.md)。
- 数のメソッドは `x.sqrt()`・`x.abs()`・`x.max(y)`・`x.min(y)`（整数は `abs` / `max` / `min`）。

## ループと if 式

```
for i in range(1, 10, 2) {            // range(終わり) / range(始め, 終わり) / range(始め, 終わり, 刻み)
    let kind = if i % 3 == 0 { "3の倍数" } else { "ふつう" };   // if は値にもなる
    print(i, kind);
}

for i in range(0) {
} else {
    print("1 回も回らなかったときだけ実行");   // Python の for-else とは意味が違う
}

loop {                                // 無限ループ。break / continue が使える
    break;
}
```

- 条件は `( )` で囲まない。`{ }` は必須。
- 値として使う if には else が必要。値はブロックの最後の **`;` のない式**（Rust と同じ）。
- `}` で終わる文（if・while・for・loop・match）のあとには `;` は要らない。
- do-while と三項演算子 `? :` はない。

```
let name = match n {                  // Java の switch の矢印形式。値にも文にもなる
    case 0 -> "zero";
    case 1, 2 -> "small";
    default -> "many";
};
```

言語仕様は [docs/spec.md](docs/spec.md)、決めた理由は [docs/design-decisions.md](docs/design-decisions.md) にまとめている。

## 構造体

```
struct Point { i64 x; i64 y; }
struct Player { str name; own Point pos; i64 score; }

fn move_by(ref mut Player p, i64 dx, i64 dy) {
    p.pos.x += dx;   // p は ref mut、p.pos は own。どちらも自動でたどる
}

fn main() {
    let mut hero = own Player { name: "勇者", pos: own Point { x: 0, y: 0 }, score: 0 };
    move_by(ref mut hero, 3, 4);
    hero.pos = own Point { x: 9, y: 9 };  // 古い Point はここで解放される
}
```

- フィールドを書き換えられるかは持ち主で決まる（`let mut` の変数か、`ref mut` を通しているか）。
- フィールドごとに所有する。構造体が解放されると、own のフィールドも連鎖して解放される。
- 構造体は値として読むとムーブ（中に own を持ちうるため）。
- `p.f` は `p` が own / ref なら自動で先をたどる。ref をたどるときの世代チェックも静的解析の対象。
- `ref p.f` でフィールドの own を借りられる。上書きされた後に使うと、時系列つきで検出される。
- フィールドから own や構造体を値として取り出すことはできない（`ref` で借りるか、`p.f = ...` で置き換える）。
- `if` / `while` の条件では構造体リテラルを書けない（`{` があいまいになるため）。括弧で囲めば書ける。

## enum と Option

```
enum Shape {
    Circle(i64 r);          // 中身は「型 名前」
    Rect(i64 w, i64 h);
    Empty;
}

fn i64 area(Shape s) {
    match s {                         // 全部の種類を書かないとエラー（default でまとめてもよい）
        case Circle(r) -> 3 * r * r;  // 中身を名前で受け取る
        case Rect(w, h) -> w * h;
        case Empty -> 0;
    }
}

fn Option<i64> find(i64 n) { ... }   // null はない。ないかもしれない値は Option
let x = Some(5);
let Option<i64> y = None;            // None だけでは型が決まらないので型を書く
```

- 値は `Shape.Circle(2)` のように作る。Option は `Some(x)` と `None`。
- 中身に own や構造体を持ちうる enum は、構造体と同じく値として読むとムーブ。中身が数などだけならコピー。
- `match ref s`（`s` は own か ref）は借りるだけ。中身の own は ref として受け取る。
- enum にもメソッドを書ける（構造体と同じ決まり）。this を値で受け取るメソッドは `fn take`。
  Option / Result には `is_some()`・`unwrap_or(d)`・`ok_or(e)`・`map(f)` などがある。
- Option は最初からある普通の enum（`enum Option<T> { Some(T value); None; }`）。

## メソッド

```
struct Point {
    i64 x;
    i64 y;

    fn i64 len2() { x * x + y * y }          // this. は書かなくてよい
    fn mut move_by(i64 dx) { x += dx; }      // 書き換えるメソッドは fn mut
}
let mut p = Point { x: 3, y: 4 };
p.move_by(1);
print(p.len2());
```

## interface（Go 風）

```
interface Shape { fn i64 area(); }
fn describe<T: Shape>(ref T s) { print(s.area()); }   // area() を持つ構造体なら何でも渡せる
fn T max<T: ordered>(T a, T b) { if a > b { a } else { b } }
fn i64 area_of(ref Shape s) { s.area() }               // interface は型としても使える
```

## Result と `?`

```
fn Result<i64, str> div(i64 a, i64 b) {
    if b == 0 { Err("0 では割れません") } else { Ok(a / b) }
}
fn Result<i64, str> calc(i64 a, i64 b) {
    let x = div(a, b)?;    // Err ならそのまま返す
    Ok(x * 2)
}
```

- 例外はない。失敗は Result で返し、`?` で上へ渡す。

## ジェネリクス

```
fn T pick<T>(bool first, T a, T b) { if first { a } else { b } }
struct Pair<T> { T a; T b; }
let p = Pair { a: 1, b: 2 };          // Pair<i64>。型の引数は使う側では書かない
let Pair<str> q = Pair { a: "x", b: "y" };
```

- 型の引数は、引数・フィールド・入れる先の型から決まる。`T` を比べたりメソッドを呼んだりするには条件（`<T: ordered>`・`<T: Shape>`）をつける。

## リスト・辞書・クロージャ・文字列の埋め込み

```
let i64[] mut xs = [3, 1, 2];
xs.push(5);
xs.sort();
let i64[str] mut ages = ["mio": 7];       // 辞書は「値の型[キーの型]」
ages["ken"] = 9;
for (name, age) in ages { print("{name} は {age} 歳"); }   // 入れた順に回る
let k = 10;
print(xs.filter((x) -> x > 2).map((x) -> x * k));        // [30, 50]
```

- リストの要素を借りた ref は、push・pop・sort など要素が動く操作のあとで使うとエラーになる（時系列つき）。
- 文字列には `split`・`trim`・`lines`・`find`・`replace` などのメソッドがある。

## スレッド

```
let chan<i64> c = chan();
let t = spawn(() -> { c.send(42); "done" });
print(c.recv(), t.join());                // Some(42) done
```

- スレッドへ送れるのは、ほかのスレッドと共有されない値だけ（型検査が調べる）。own を送ると送った側では使えなくなる。
- みんなが待ちに入ったら、デッドロックとしてエラーになる。

## モジュール

`use geom;` で同じフォルダの `geom.jw` を読み、`pub` をつけた名前を `geom.area(p)` のように使う。

## エラーは時系列で出る

```
error: 無効になった参照を使いました (dangling reference)
 --> examples/dangling_scope.jw:9:11
  |
7 |         r = ref a;
  |             ----- ここで `a` を借りた
8 |     }
  |     - ここで解放された: `a` はスコープを抜けたので解放された
9 |     print(*r)
  |           ^^ ここで使ったが、借りた先はもう存在しない
  |
  = note: 参照は、借りた元の所有者より長くは生きられません
```

## 静的解析で検査を消す

実行前にプログラムを読み（インタプリタは `src/analyze.rs`、実行ファイルは IR の上の `src/ir/analyze.rs`）、「借りた先は必ず生きている」と証明できた `*r` の
世代チェックを消す。証明できない箇所だけ実行時の検査が残る。

- `let r = ref a;` のあと、`a` が再代入・ムーブ・`drop`・スコープ終了されていない間の `*r` は安全。
  （`a` のスロットを解放できるのは `a` という名前を使う操作だけなので）
- 関数の引数 `r` は、すべての呼び出し元が「生きている ref」を渡しているなら、関数の中ずっと安全。
  関数をまたいで繰り返し計算するので、再帰や ref の受け渡しにも効く。
- `if` の合流や `while` では、どちらの道でも成り立つことだけを残す（安全側）。
- デバッグビルドでは、消した検査も念のため確かめ、解析の誤りがあれば panic する。

```
$ cargo run -- --stats examples/counter.jw
count = 3
[stats] `*p` と `p.f` の箇所: 検査を消せた 3 / 実行時に残る 0
[stats] ref の世代チェック: 実行 0 回 / 省略 6 回
```

## 現状の正直な限界

- 証明できなかった箇所の検査は **実行時**。ダングリングはコンパイルエラーではなく、確実に検出される実行時エラー。
- 解析は保守的。分からないところは検査を残す（安全側）。
- 整数のあふれはその型の幅で一周する（定数どうしの計算なら警告が出る）。
- 小数は `f64` だけ（`f32` はまだ）。未実装のものは [docs/spec.md の 11 章](docs/spec.md#11-未実装決定済みのもの)。

## 速さ

`jabberwock build` の実行ファイルを Go・C++（clang++ -O2）と比べたものは [bench/README.md](bench/README.md)。
Rust の版も足した 21 本（2026-10-25）では、15 本で Go より速い。計算だけのものは C++・Rust とほぼ同じで、確保と解放が多いもの（shortlived・binarytrees）は一番速い。
Go に負けているのは bst・chan・pingpong（スレッドの待ち合わせ）・refmut・linkedlist。

## セルフホスト

`jwc/` は jabberwock で書いた jabberwock のコンパイラ。Rust 版と同じ IR と LLVM のテキストを一字一句同じに出し、jwc 自身もコンパイルできる。

```
cargo run --release -- build -o jwc1 jwc/main.jw   # Rust 版で 1 代目の jwc を作る
./jwc1 build -o jwc2 jwc/main.jw                    # 1 代目で 2 代目を作る（数分かかる。ほとんどは clang）
./jwc2 build examples/fib.jw
```

くわしくは [docs/self-hosting.md](docs/self-hosting.md)。

## 次の一手

1. 実行時に見つけているエラーの一部を、型検査で実行前に見つけるようにする
2. ベンチマークの残りの項目（[bench/README.md](bench/README.md) の「まだやっていないこと」）
3. パッケージ（`alice`）: 手元の依存先だけはできた（docs/packages.md）。ネットからの取得・版の指定と、標準ライブラリ

Windows ASanの起動エラーは [DLL診断](docs/windows-asan-diagnostics.md) を参照。診断情報は通常の安全性JSONにも含まれる。

項目3・4・6には [既存ツールによる暫定検出器](docs/provisional-detectors.md) を接続。後で専用検出器を実装して置き換える方針。未導入や検出能力未確認を合格にはしない。
