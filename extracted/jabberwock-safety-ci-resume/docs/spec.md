# jabberwock 言語仕様（作成中）

いま実装されている言語の決まりをまとめたもの。なぜそう決めたかは [design-decisions.md](design-decisions.md) にある。
旧名は ward（2026-10-24 に jabberwock へ改名。拡張子は `.jw`）。

- 対象: 最新の main（言語を変えたら、同じコミットでここも直す）
- 「未実装」と書いたものは、決めてあってもまだ動かない
- 所有権・参照の形式化の範囲と世代上限での退役規則は [formal-semantics.md](formal-semantics.md)。機械証明は抽象モデルの一部で、実装全体の証明ではない。
- [形式検証の到達目標と責任範囲](formal-semantics.md#到達目標と責任範囲) は、意味保存・メモリ安全性・型安全性・所有権/参照・実行時チェックの5項目。到達目標と現在の保証済み範囲を区別する。

## 1. プログラムの形

```
struct Point { i64 x; i64 y; }

enum Shape {
    Circle(i64 r);
    Empty;
}

fn i64 add(i64 a, i64 b) {
    a + b
}

fn main() {
    print(add(1, 2));
}
```

- トップレベルに書けるのは `struct`・`enum`・`interface`・`fn`・`const` の定義と `use` だけ。順番は自由（後ろで定義したものも使える）。ただし定数の値の中で使えるのは、それより前に書いた定数だけ（下）。
- `const 名前 = 値;` は定数。値に書けるのは数・小数・文字列・真偽値と、その計算（`60 * 60`）・`as`・それより前に書いた定数。
  使う場所に値をそのまま書いたのと同じなので、数の型は使う場所で決まる（`const MASK = 0xff;` は `let u8 b = MASK;` にも書ける）。
  定数はそのファイルの中だけで使える（`pub` はつけられない）。変数・引数・関数に定数と同じ名前はつけられない。
- 実行は `fn main()` から始まる。`main` は引数も戻り値も持たない。
- 文の終わりには `;` をつける。改行はただの空白（式を途中で改行してよい）。

### 1.1 モジュール

```
// geom.jw
pub struct Point { i64 x; i64 y; }
pub fn i64 dist2(ref Point p) { sq(p.x) + sq(p.y) }
fn i64 sq(i64 x) { x * x }          // pub がないので geom の中だけで使える

// main.jw
use geom;                           // 同じフォルダの geom.jw を使う
fn main() {
    let p = geom.Point { x: 3, y: 4 };
    print(geom.dist2(p));           // 25
}
```

- 1 ファイルが 1 モジュール。`use 名前;` で、最初のファイルと同じフォルダの `名前.jw` を使う。
  パッケージ（alice.toml のあるフォルダ。`alice` コマンド）では、src の `名前.jw` と依存先のライブラリ（依存先の src/lib.jw がパッケージの名前のモジュール）を使う（[docs/packages.md](packages.md)）。
  jwcは `alice package-map` が出力するローカル依存情報を `--package-map` で読み、同じ規則で名前を解決する。aliceのbuild/run/testからのjwc呼び出しはまだ未対応。
  名前の空間はパッケージごと: 依存先の src のほかのファイルはそのパッケージの中からだけ見え、`use 名前;` は自分のパッケージ → 依存先の順に探す。
  使えるパッケージは alice.toml に直接書いた依存先だけ。
- ほかのモジュールの名前は、いつも `モジュール名.名前` と書く（関数・構造体・enum・interface・静的メソッド `geom.Point.at(1, 2)`・enum の値 `geom.Shape.Box(1, 2)`・型 `geom.Point[]`）。
- ほかのファイルから使えるのは `pub` をつけた定義だけ。構造体のメソッド・フィールド、enum の種類は、型が `pub` なら使える。
- モジュールの中では、自分の定義は名前だけで書ける。`use` は書いたファイルの中でだけ効く（使うモジュールが使っているモジュールは見えない）。
- モジュールの中から、最初のファイル（main）の関数と型（構造体・enum・interface）は使えない（関数は値としても使えない）。
  使うものはモジュールの中に置くか、別のモジュールに分けて use する（関数なら、関数の値として引数で渡してもよい）。
  どのプログラムにも最初からある `Option`・`Result`・`ok_or` は使える。jwc だけ（Rust 版は使えてしまう。凍結のため直さない）。
- 違うモジュールに同じ名前の定義があってよい。
- `use` が輪になる（a が b を使い、b が a を使う）のはエラー。
- 変数や引数に、そのファイルで `use` したモジュールと同じ名前はつけられない。構造体のフィールドにはつけてよい（`b.geom.x` の `geom` はフィールド）。
  ほかのファイルが use しているだけのモジュールの名前はつけてよい（jwc だけ。Rust 版はプログラムのどこかで use したモジュールの名前を全部禁じる）。
- 構造体の値を print すると、型の名前はモジュール名つきで出る（`geom.Point { x: 3, y: 4 }`）。
- エラーの位置は、そのモジュールのファイルの中で示す。

### 1.2 標準ライブラリ（jwc だけ）

```
use std.math;
use std.text;
fn main() {
    print(math.gcd(12, 18), text.pad_left("7", 3, "0"));   // 6 007
}
```

- `use std.名前;` で標準ライブラリのモジュールを使う。使う側では末尾の名前で `math.gcd(...)` と書く（ほかのモジュールと同じ規則）。
- 標準ライブラリは jwc に埋め込まれている（ソースはリポジトリの `std/*.jw`）。ファイルを探さないので、jwc の置き場所によらず使える。
- 同じフォルダやパッケージの `math.jw` は標準の math を置き換えない。`use math;` と `use std.math;` を同じファイルに書くとエラー（別名の `as` はまだない）。
  `use std;` は今までどおり同じフォルダの std.jw を使う。
- 標準ライブラリの中では `use std.名前;` だけ使える。ない名前はエラー（使える名前を示す）。エラーの位置は `std/math.jw` のように示す。
- Rust 版（`jabberwock`）は対応しない（凍結。[design-decisions.md](design-decisions.md)）。

| モジュール | 関数 |
|---|---|
| `std.math` | `gcd(a, b)`・`lcm(a, b)`・`pow(base, exp)`（exp は 0 以上）・`clamp(x, lo, hi)`（どれも i64）、`pi()`・`floor(x)`・`ceil(x)`・`round(x)`（f64。round はちょうど半分なら 0 から遠い方） |
| `std.text` | `pad_left(s, width, fill)`・`pad_right(s, width, fill)`（fill は 1 文字）・`reverse(s)`・`count(s, sub)`（重ならずに数える。sub が空なら実行時エラー）・`is_digits(s)`（空でなく全部 0〜9）・`from_utf8(ref bytes)`（`Option<str>`。正しい UTF-8 でなければ None） |
| `std.fs` | `open(path)`・`create(path)`・`append(path)`・`close(f)` と資源の型 `File`、`exists(path)`・`list_dir(path)`・`make_dir(path)`・`remove(path)`・`rename(from, to)`・`write_atomic(path, bytes)`・`write_text_atomic(path, s)`・`stat(path)` と `Info`・`is_file(path)`・`is_dir(path)`・`copy(from, to)`・`remove_all(path)`（[1.3](#13-資源と-stdfsjwc-だけ)） |
| `std.env` | `get(name)`（`Option<str>`）（[1.6](#16-stdenvstdtimestdjsonjwc-だけ)） |
| `std.time` | `now()`・`sleep(ms)`・`unix_ms()`・`local_offset_ms(t)` と `Instant`（[1.6](#16-stdenvstdtimestdjsonjwc-だけ)） |
| `std.json` | `parse(s)`・`stringify(ref v)`・`quote(s)` と `Json`（[1.6](#16-stdenvstdtimestdjsonjwc-だけ)） |
| `std.http` | `get(url)`・`post(url, content_type, body)` と `Response`、サーバーの `listen(host, port)`・`stop(srv)` と `Server`・`Request`（[1.5](#15-stdhttpjwc-だけ)） |
| `std.url` | `parse(s)`・`encode(s)`・`decode(s, plus)` と `Url`（[1.7](#17-stdurljwc-だけ)） |
| `std.browser` | `open(cfg)`・`close(e)` と `Engine`・`Config`・`Event`・`KeyCode`・`Info`（[1.8](#18-stdbrowserjwc-だけ)） |
| `std.net` | `connect(host, port)`・`listen(host, port)`・`close(s)`・`stop(l)` と資源の型 `TcpStream`・`TcpListener`（[1.4](#14-stdnetjwc-だけ)） |
| `std.tls` | `connect(host, port)`・`upgrade(s, host)`・`close(s)` と資源の型 `TlsStream`（[1.9](#19-stdtlsjwc-だけ)） |
| `std.gzip` | `decompress(ref data, limit)`・`compress(ref data)`・`crc32(ref data)`（[1.10](#110-stdgzipjwc-だけ)） |
| `std.pg` | `connect(cfg)`・`close(db)` と `Config`・`Conn`・`Rows`（[1.11](#111-stdpgjwc-だけ)） |
| `std.hash` | `sha256(ref data)`・`hmac_sha256(ref key, ref msg)`・`pbkdf2_sha256(ref password, ref salt, iters)`・`hex(ref data)`・`equal(ref a, ref b)`（[1.12](#112-stdhashstdbase64stdrandomjwc-だけ)） |
| `std.base64` | `encode(ref data)`・`decode(s)`（[1.12](#112-stdhashstdbase64stdrandomjwc-だけ)） |
| `std.random` | `bytes(n)`・`int(lo, hi)`（[1.12](#112-stdhashstdbase64stdrandomjwc-だけ)） |
| `std.date` | `now()`・`from_unix_ms(t)`・`make(...)`・`parse(s)`・`is_leap(y)`・`days_in_month(y, m)` と `DateTime`（[1.13](#113-stddatestdcsvstdregexjwc-だけ)） |
| `std.csv` | `parse(s)`・`format(ref rows)`・`quote(v)`（[1.13](#113-stddatestdcsvstdregexjwc-だけ)） |
| `std.regex` | `compile(pattern)`・`escape(s)` と `Regex`・`Match`（[1.13](#113-stddatestdcsvstdregexjwc-だけ)） |

- 詳しい動き（一周・実行時エラーになる条件）は `std/*.jw` の各関数の説明に書いてある。

### 1.3 資源と std.fs（jwc だけ）

```
use std.fs;
fn Result<(), str> save() {
    let mut f = fs.create("out.txt")?;   // fs.File
    f.write_text("こんにちは\n")?;
    f.write([65, 66])?;                  // u8[]（ムーブ）
    fs.close(f)?;                        // 閉じた結果を受け取る
    let mut r = fs.open("out.txt")?;
    print(r.read(3)?, r.read_text()?);  // 最大 3 バイト（終わりなら []）、残り全部
    Ok(())
}
```

- **資源**は外のもの（今は開いたファイルだけ）を持つ値。`resource 名前;` で決めるが、書けるのは標準ライブラリの中だけで、
  コンパイラが知っている種類（`std.fs` の `File`、`std.net` の `TcpStream`・`TcpListener`、`std.tls` の `TlsStream`、`std.browser` の `Handle`）に限る。数や文字列から作る方法はない（作るのは `fs.open` などだけ）。
- 値として読むといつもムーブ（2 つ目の持ち主はできない）。`ref` / `ref mut` で借りられる。`for x in` の x などの読むだけの写しは、閉じない。
- 持ち主がいなくなると黙って閉じる（構造体・リストの中でも）。閉じたときの失敗を知りたいときは `fs.close(f)`（`Result<(), str>`。失敗しても f はもう使えない）。
  `exit` と実行時エラーでは閉じない（ほかの後片づけと同じ。書いた内容は close で確かめる）。
- ほかのスレッドへは渡せない（構造体などに包んでも）。`==` などで比べられない。print すると `<fs.File>`。
- メソッド（どれも `Result<_, str>` を返し、持ち主が mut であること）: `read(n)`（`u8[]`。n が負なら実行時エラー）・`read_text()`（`str`。UTF-8 でなければ Err）・
  `write(bytes)`（`u8[]` をムーブして全部書く）・`write_text(s)`。
- 開く: `fs.open(path)`（読む）・`fs.create(path)`（書く。なければ作り、あれば空にする）・`fs.append(path)`（後ろに足す）。パスに NUL 文字があれば Err。
- 書いた内容は、close するか持ち主がいなくなるまで、ほかの開き方からは見えないことがある（まとめて書くため）。
- ディレクトリ: `fs.exists(path)`（`bool`。ファイルでもディレクトリでも）・`fs.list_dir(path)`（`Result<str[], str>`。`.` と `..` を除き、名前の順。UTF-8 でない名前があれば Err）・
  `fs.make_dir(path)`（親はあること。もうあれば Err）・`fs.remove(path)`（ファイルか空のディレクトリ）。
- `fs.rename(from, to)`: 名前を変える。to があれば置き換える。
- 情報: `fs.stat(path)`（`Result<Info, str>`。なければ Err）。`Info` は `size`（中身のバイト数。ファイルでなければ 0）・`is_file`・`is_dir`・
  `is_link`（path そのものがシンボリックリンクか。Windows ではリパースポイント）・`modified_ms`（更新時刻。1970-01-01 UTC からのミリ秒。Windows は秒の単位）。
  リンクなら is_link 以外はたどった先のもの（先がなければ is_file も is_dir も false で、size は 0）。
  `fs.is_file(path)`・`fs.is_dir(path)`（`bool`。なければ false）。
- `fs.copy(from, to)`: ファイルを写す（to があれば上書き。安全な上書きではない）。from と to が同じ文字列、または from がファイルでなければ Err。
- `fs.remove_all(path)`: ファイルか、ディレクトリを中身ごと消す。リンクはたどらず、リンクそのものを消す（リンクの先は残る）。なければ Err。途中で失敗すると、そこまでに消したものは戻らない。
- 安全な上書き: `fs.write_atomic(path, bytes)`・`fs.write_text_atomic(path, s)`。`path + ".tmp"` に全部書いて閉じてから path へ名前を変える。
  途中で失敗すれば path は元のまま（一時ファイルは消す）。読む側には古い中身か新しい中身のどちらかが見える。電源断まで耐えるとは約束しない。
- 実行時ライブラリは資源を表で持ち、値は「札」（場所の番号と世代）。閉じた札を使うと実行時エラー（型検査を通ったプログラムでは起きないはずの、念のための検査）。
  場所を使い回すときは世代を進め、世代が上限に達した場所は退役させる。
- テスト用: 環境変数 `JW_RES_FAIL=open` / `read` / `write` / `close` / `list` / `make_dir` / `remove` / `rename`（std.net は `connect` / `listen` / `accept` も）で、
  その操作をわざと失敗させる（`--release` で作った実行ファイルでは効かない）。

### 1.4 std.net（jwc だけ）

```
use std.net;
let mut l = net.listen("127.0.0.1", 0)?;   // 0 なら OS が番号を選ぶ（l.port()）
let mut c = l.accept()?;                   // TcpStream
c.set_timeout(2000)?;                      // ミリ秒（0 なら待ち続ける）
let got = c.read(4096)?;                   // 最大 4096 バイト。相手が閉じたら []
c.write_text("ok\n")?;
net.close(c)?;
```

- TCP だけ（同期。UDP はまだ。TLS は std.tls、HTTP は std.http）。`TcpStream`・`TcpListener` は資源で、規則は 1.3 と同じ（ムーブだけ・送れない・持ち主がいなくなると閉じる）。
- `net.connect(host, port)`・`net.listen(host, port)`（`Result`）。host は名前か番号。port が 0〜65535 の外なら Err。
- TcpStream: `read(n)`（`u8[]`）・`write(bytes)`・`write_text(s)`・`set_timeout(ms)`。時間切れは `Err("時間切れです")`。相手が閉じた接続へ書くと Err（プログラムは止まらない）。
- TcpListener: `accept()`（`Result<TcpStream, str>`）・`port()`（`i64`）。やめるのは `net.stop(l)`。
- Linux・macOS・Windows で使える。Windows は Winsock（ws2_32.dll）を初めて使うときに読み込む（リンクの指定は要らない。読み込めなければ connect / listen が Err）。

### 1.5 std.http（jwc だけ）

```
use std.http;
let r = http.get("http://127.0.0.1:8080/a?b=1")?;          // http.Response
print(r.status, r.headers["content-type"], r.text()?);
let p = http.post("http://127.0.0.1:8080/p", "application/json", "{{}}")?;
```

- HTTP/1.1 のクライアント。std.net・std.tls の上に .jw だけで書いている（`std/http.jw`）。http:// と https://（[1.9](#19-stdtlsjwc-だけ) の規則で証明書を確かめる）。ほかの scheme は Err。
  Host ヘッダーは、port がその scheme の決まりの番号（http は 80、https は 443）なら port を書かない。
- 1 回ごとにつなぎ、`Connection: close` を送って、相手が閉じるまで読む。本文は chunked なら読み解き、`Content-Length` があればその長さにする（足りなければ Err）。
  状態の番号が 404 などでも Ok（`status` を見る）。
- `Accept-Encoding: gzip` を送る。`Content-Encoding: gzip` の本文は std.gzip で展開し（展開して 256 MiB を超えれば Err）、
  `headers` から `content-encoding` と `content-length` を除く。
- リダイレクト（301・302・303・307・308 で `Location` があるもの）は 10 回までたどる（超えたら Err）。303 と、POST への 301・302 は、本文なしの GET でたどる。
  Location は、`scheme://` で始まればそのまま、`//` で始まれば同じ scheme、`/` で始まれば同じ host と port、ほかは今の path の最後の `/` のあとにつなぐ。
  https から http へのリダイレクトもたどる。
- `Response`: `status`（`i64`）・`url`（たどったあとの URL。たどらなければ頼んだ URL）・`headers`（`str[str]`。名前は小文字。同じ名前が二つ以上なら `", "` でつなぐ）・`body`（`u8[]`）・`text()`（本文を `str` に。UTF-8 でなければ Err）。
- URL は std.url の `parse` で分ける（同じ規則で Err）。URL に空白・改行、content_type に改行があれば Err（ヘッダーを足されないように）。

サーバー（HTTP/1.1。1 度に 1 つの要求を順に扱う）:

```
let mut srv = http.listen("127.0.0.1", 8080)?;      // http.Server（port が 0 なら OS が選ぶ。番号は srv.port()）
loop {
    let mut req = srv.next()?;                       // http.Request
    req.respond(200, "text/plain; charset=utf-8", "こんにちは {req.path}")?;
}
```

- `srv.next()`（`Result<Request, str>`）: 次のつながりを受け、要求を 1 つ読む。読めない要求にはここで答えて（本文は理由の文）、次のつながりを待つ:
  要求の行の形が違う・ヘッダーが UTF-8 でない・HTTP/1.1 で Host がない・target が `/` で始まらない（`OPTIONS *` はよい）・%XX が壊れている・Content-Length が数でない（二つあるときも）なら 400、
  `Transfer-Encoding` があれば 411（chunked の要求は受けない）、ヘッダーが 64 KiB を超えれば 431、本文が 16 MiB を超えれば 413、HTTP/1.0・1.1 でなければ 505。
  1 つの要求を読み終えるまでの上限は 30 秒（受け取るたびに残りの時間だけ待つ）で、超えたり相手が途中で閉じたりしたら、答えずに閉じる。Err は待ち受けそのものが失敗したときだけ。
  `Expect: 100-continue` には本文を読む前に `100 Continue` を送る。
- `Request`: `method`（送られたまま）・`target`（要求の行のまま）・`path`（target の `?` の前を %XX から戻したもの。`..` や `/` も入りうる）・
  `query`（`str[str]`。`?` のあとを `&` と `=` で分け、%XX と `+` を戻したもの。同じ名前は後のもの）・`headers`（名前は小文字。同じ名前は `", "` でつなぐ）・`body`（`u8[]`）・`text()`。
  資源の TcpStream を持つ（ムーブだけ。持ち主がいなくなるとつながりを閉じる）。
- `req.respond(status, content_type, body)`（本文は str）・`req.respond_bytes(status, headers, body)`（`str[str]` と `u8[]`）: 答えを送る。
  いつも `Content-Length` と `Connection: close` を付ける（1 つのつながりで 1 つの要求）。HEAD の要求には本文を送らない（Content-Length は本文の長さ）。
  答えるのは 1 回だけ（2 回目は Err）。status が 100〜599 でない・ヘッダーの名前がトークンでない・値に改行か NUL がある・`Content-Length`・`Connection`・`Transfer-Encoding` を渡したなら、送らずに Err。
- `http.stop(srv)`: 待ち受けをやめる。TLS（https）の待ち受けはまだない。

### 1.6 std.env・std.time・std.json（jwc だけ）

```
use std.env;
use std.time;
use std.json;
print(env.get("HOME"));                         // Some(/home/...)。なければ None
let t = time.now();                             // time.Instant
time.sleep(100);                                // ミリ秒待つ
print(t.elapsed_ms());                          // 100 くらい
let v = json.parse("{{\"a\": [1, \"x\"]}}")?;     // json.Json
print(v.get("a").at(1).as_str(), json.stringify(ref v));   // Some(x) {"a":[1,"x"]}
```

- `env.get(name)`: 環境変数（名前が空・`=` や NUL を含む・値が UTF-8 でないなら None）。設定する関数はまだない。
- `time.now()`: 単調な時計の読み（`Instant { ns }`）。壁時計の修正に影響されず、差だけに意味がある。`t.elapsed_ns()`・`t.elapsed_ms()`。
  `time.sleep(ms)`（負なら実行時エラー。待つ前に print の出力を書き出す）。
- `time.unix_ms()`: 壁時計（1970-01-01 UTC からのミリ秒）。時刻の修正で戻ることがあるので、かかった時間は now で測る。暦（年月日）への直しは std.date（1.13）。
- `time.local_offset_ms(t)`（`Result<i64, str>`）: OS に設定したタイムゾーンが、t（unix_ms と同じミリ秒）の時点で UTC からどれだけずれているか（ミリ秒。東が正。夏時間もその時点のもの）。OS の localtime（Windows は _localtime64_s）で求め、答えがなければ Err（Windows は 1970 年より前に答えないので Err。環境変数 TZ を設定していればそれに従う）。
- `Json` は enum: `Null`・`Bool(b)`・`Num(n)`（いつも f64）・`Str(s)`・`Arr(own Json[] items)`・`Obj(own Json[str] fields)`（キーは書いた順。同じキーは後ろの値）。
  メソッド: `get(key)`・`at(i)`（写しを返す。なければ Null）・`len()`・`keys()`・`is_null()`・`as_bool()`・`as_num()`・`as_str()`（`Option`）・`clone()`。
  中を借りて読むなら `match ref v { case Obj(fields) -> ... }`。
- `json.parse(s)`（`Result<Json, str>`）: RFC 8259 のとおり（前後の空白はよい。余分な文字・先頭の 0・制御文字・対になっていないサロゲートは Err）。
  入れ子は 512 段まで。Err の文には何文字目かが入る。
- `json.stringify(ref v)`: 空白なしで書く。整数の値（2^53 未満）は `3`、ほかは小数の表示。NaN・inf は `null`。`json.quote(s)` は文字列 1 つ。

### 1.7 std.url（jwc だけ）

```
use std.url;
let u = url.parse("http://example.com:8080/a?q=1#top")?;   // url.Url
print(u.scheme, u.host, u.port, u.path, u.query, u.fragment, u.target());
// http example.com 8080 /a q=1 top /a?q=1
print(url.encode("a b&日"));                               // a%20b%26%E6%97%A5
print(url.decode("a%20b+c", true)?);                       // a b c
```

- `url.parse(s)`（`Result<Url, str>`）: `scheme://host[:port]/path?query#fragment` の形だけ分ける。scheme は小文字にする。path がなければ `/`。
  port を書いていなければ http は 80、https は 443、ほかは 0。`[::1]` の書き方の host は角かっこを外す。
  空白・改行・タブ、利用者情報（`user@`）、0〜65535 でない port、空の host（`file:///...` も）は Err。
- `Url`: `scheme`・`host`・`port`（`i64`）・`path`・`query`・`fragment`（なければ空の文字列）と `target()`（path と、あれば `?query`）。
- `url.encode(s)`: 英数字と `-` `.` `_` `~` のほかは、UTF-8 のバイトごとに `%XX`（大文字）にする。
- `url.decode(s, plus)`: `%XX` を戻す（plus が true なら `+` を空白にする）。`%` のあとが 2 桁の 16 進数でない・戻した結果が UTF-8 でないなら Err。

### 1.8 std.browser（jwc だけ）

```
use std.browser;
let mut e = browser.open(browser.Config { title: "試し", data_dir: "data", width: 1024, height: 768, ephemeral: true })?;
let tab = e.tab_new()?;
e.navigate(tab, "https://example.com/")?;
loop {
    match e.wait(-1) {                    // Option<browser.Event>
        case Some(CloseRequested) -> break;
        case Some(ev) -> print(ev);       // NavStarted(1, https://example.com/) など
        case None -> {}
    }
}
browser.close(e);
```

- ブラウザのエンジン（ページを表示する部品）を使う。エンジンとの境目は `runtime/engine/jw_engine.h`（C の関数だけ。版 1）。
  エンジンの実装は jabberwock の外にあり、`jwc build --link エンジン.c`（`.o`・`-lole32` なども。何度でも書ける）で一緒にリンクする。
  std.browser を使うプログラムで `--link` がなければ、作る前にエラー。試験用の偽エンジンは `runtime/engine/mock/jwe_mock.c`（画面なし）。
- 呼び返しはしない。エンジンがイベントを溜め、`e.poll()`（待たない）・`e.wait(ms)`（-1 は無期限。-1 より小さいと実行時エラー。待つ前に print の出力を書き出す）で 1 つずつ取る。
  エンジンは開いたスレッドだけで使う（`Engine` は資源 `Handle` を持つので、ほかのスレッドへ渡せない）。
- `browser.open(cfg)`（`Result<Engine, str>`）: width・height は 1〜100000。エンジンが使えない・境目の版が違うなら Err。
  持ち主がいなくなるか `browser.close(e)` で閉じる（タブも全部。ephemeral なら置き場所の中も消す）。
- `Engine` のメソッド（持ち主が mut であること）: `info()`（`Info { name, script, chrome_view, persist }`）・`poll()`・`wait(ms)`・
  `tab_new()`（`Result<i64, str>`。タブの番号は 1 から、使い回さない）・`tab_close(tab)`・`tab_show(tab)`・`navigate(tab, url)`・`back(tab)`・`forward(tab)`・
  `reload(tab)`・`stop(tab)`・`chrome_load(html, height)`・`chrome_post(msg)`（外枠の HTML。`chrome_view` のときだけ）・`set_title(s)`・`clear_site_data()`。
  操作は `Result<(), str>`（ないタブ・できない操作は Err）。
- `Event`: `Ready`・`CloseRequested`・`NavStarted(tab, url)`・`NavFinished(tab, url, code)`（code 0 は成功）・`UrlChanged(tab, url)`・`TitleChanged(tab, title)`・
  `HistoryChanged(tab, back, forward)`・`NewTabRequested(tab, url)`・`ChromeMessage(text)`・`Key(code)`（`KeyCode.of(code)` で読む）・`PermissionDenied(tab, what)`・`Failed(code, text)`。
  エンジンが UTF-8 でない文字列を返したら `Failed` にする。

### 1.9 std.tls（jwc だけ）

```
use std.tls;
let mut s = tls.connect("example.com", 443)?;   // TlsStream。証明書と名前を確かめてから Ok
s.write_text("GET / HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n")?;
let got = s.read(4096)?;                         // 相手が終わりを知らせたら []
tls.close(s)?;
```

- TLS のクライアントだけ（待ち受けはまだ）。版は TLS 1.2 以上（Windows は 1.2）。`TlsStream` は資源で、規則は 1.3 と同じ。メソッドは TcpStream と同じ（`read(n)`・`write(bytes)`・`write_text(s)`・`set_timeout(ms)`）。
- `tls.connect(host, port)`: TCP でつなぎ、TLS の握手をして、相手の証明書を OS の信頼の置き場所までたどれるか・`host` と名前が合うかを確かめる。
  どちらかがだめなら `Err("TLS の証明書を確かめられません（理由）: host")`。理由の文は OS の部品による（下）。host が IP の番号なら、証明書の IP と照らす。
  確かめるのをやめる指定はない。`tls.close(s)` は終わりの知らせ（close_notify）を送ってから閉じる。持ち主がいなくなったときも同じ。
- OS の部品は初めて使うときに読み込む（リンクの指定は要らない。読み込めなければ connect が Err）。
  - Linux など: OpenSSL 3（`libssl.so.3`、なければ `libssl.so`）。信頼の置き場所は OpenSSL の決まりのもの（環境変数 `SSL_CERT_FILE`・`SSL_CERT_DIR` で変えられる）。理由の文は OpenSSL のもの（英語）。
  - Windows: Schannel（secur32.dll）と crypt32.dll。信頼の置き場所は Windows のもの。理由の文は「信頼できる発行元につながりません」「名前が合いません」「期限が切れています」「取り消されています」か、番号（`0x...`）。
- `tls.upgrade(s, host)`（`Result<TlsStream, str>`）: すでにつないだ `net.TcpStream`（ムーブ）の上で握手をする（平文で少しやりとりしてから TLS にする手順のため。std.pg が使う）。
  確かめ方とエラーは connect と同じ（host は照らす名前か IP）。だめなら Err で、TcpStream は閉じる。
- 取り消し（失効）は調べない。

### 1.10 std.gzip（jwc だけ）

```
use std.gzip;
let packed = gzip.compress(ref data);                  // u8[]（gzip の形）
let back = gzip.decompress(ref packed, 10000000)?;     // 展開して 10000000 バイトを超えるなら Err
print(gzip.crc32(ref data));                           // i64（0〜4294967295）
```

- gzip（RFC 1952）と、その中身の deflate（RFC 1951）。.jw だけで書いている（`std/gzip.jw`）。
- `decompress(ref data, limit)`（`Result<u8[], str>`）: deflate の 3 種類のブロック（圧縮しない・固定の符号・動的な符号）を読む。
  gzip の頭の名前・注釈・追加の欄は読み飛ばす。最後に CRC32 と長さを確かめる。続けてつないだ gzip は、順につなげて返す。
  limit は展開した大きさの上限（バイト）で、超えたら Err（小さなデータが大きく広がる攻撃を防ぐため）。limit が負なら実行時エラー。
  壊れた・途中で切れた・gzip でないデータは Err。
- `compress(ref data)`（`u8[]`）: 固定の符号のブロック 1 つにする（同じ並びは後ろ 32768 バイトまでから探す）。縮まないデータは圧縮しないブロックにする。
  頭の時刻は 0、OS は 255（不明）。同じ入力からはいつも同じ出力。
- `crc32(ref data)`: gzip と同じ CRC32。

### 1.11 std.pg（jwc だけ）

```
use std.pg;
let mut db = pg.connect(pg.Config { host: "localhost", port: 5432, user: "app", password: "secret", database: "mydb", tls: false })?;
let r = db.query("SELECT id, name FROM t WHERE id = $1", ["1"])?;   // Rows
for row in r.rows { print(row[0], row[1]); }                         // 値は Option<str>（NULL なら None）
let n = db.exec("UPDATE t SET name = $1 WHERE id = $2", ["a", "1"])?; // 変わった行の数
db.exec_opt("INSERT INTO t VALUES ($1, $2)", [Some("2"), None])?;     // None は NULL
pg.close(db)?;
```

- PostgreSQL のクライアント（フロントエンド・バックエンドの手順の版 3.0）。std.net の上に .jw だけで書いている（`std/pg.jw`）。同期だけ。
- `pg.connect(cfg)`（`Result<Conn, str>`）: `Config` の host・port・user・password・database でつなぎ、ログインする。database が空なら user と同じ名前（サーバーの決まり）。
  文字の符号は UTF8 にする。ログインの方式は SCRAM-SHA-256（サーバーの署名も確かめ、合わなければ Err。チャネルバインディングは使わない）・md5・パスワードなし（trust）・
  パスワードをそのまま送る方式（password）。password は TLS のときだけで、平文のつながりでサーバーが求めたら Err。
- `tls`（`bool`）が true なら、ログインの前に SSLRequest を送り、サーバーが受けたら `tls.upgrade` で TLS にする（1.9。証明書と host の名前を確かめる）。
  サーバーが断ったら `Err("PostgreSQL のサーバーが TLS を使えません（…）")` で、平文には戻らない。false なら平文でつなぐ。サーバーのエラーは `Err("PostgreSQL のエラー（重さ SQLSTATE）: 本文")`。
  設定に NUL 文字があるか、user が空なら Err。
- `Conn` は資源の TcpStream か TlsStream を持つ構造体（ムーブだけ。ほかのスレッドへは渡せない。持ち主がいなくなると黙って閉じる）。メソッドは持ち主が mut であること。
  - `query(sql, params)`（`Result<Rows, str>`）: params（`str[]`）を `$1`・`$2`… の値として SQL とは別に送る（拡張問い合わせ。SQL に埋め込まない）。
    値は文字列で送り、型はサーバーが決める。結果の値も文字列（テキスト形式）で、NULL は None。文は 1 つだけ。
    サーバーのエラー（構文・制約など）は Err で、そのあとも同じ Conn を使える。トランザクションは `exec("BEGIN", [])` などで書く。
  - `exec(sql, params)`（`Result<i64, str>`）: query と同じで、変わった行の数を返す。
  - `query_opt(sql, params)`・`exec_opt(sql, params)`: params が `Option<str>[]` で、None を NULL として送る。ほかは query・exec と同じ。
- `Rows`: `columns`（`str[]`。列の名前）・`rows`（`Option<str>[][]`）・`affected`（INSERT・UPDATE・DELETE・SELECT などの行の数。数のない命令なら 0）。
  `get(row, col)`（`Option<str>`）は row 行目の名前が col の列の値（その列がなければ None）。
- `pg.close(db)`（`Result<(), str>`）: 終わりの知らせ（Terminate）を送ってから閉じる。送れなければ Err だが、どちらでも閉じる。
- `tls: false` のつながりは暗号にしない（同じ機械か、信頼できるネットワークの中で使う）。パスワードは SASLprep（Unicode の正規化）をせずにそのまま使う。

### 1.12 std.hash・std.base64・std.random（jwc だけ）

```
use std.hash;
use std.base64;
use std.random;
let d = hash.sha256(ref data);                    // u8[]（32 バイト）
print(hash.hex(ref d), base64.encode(ref d));
let back = base64.decode("aGk=")?;                 // [104, 105]
let key = random.bytes(32);                        // OS の乱数
let die = random.int(1, 7);                        // 1〜6
```

- `std.hash`（.jw だけで書いている）: `sha256(ref data)`（SHA-256。32 バイト）・`hmac_sha256(ref key, ref msg)`（HMAC-SHA-256。key は何バイトでもよい）・
  `pbkdf2_sha256(ref password, ref salt, iters)`（PBKDF2-HMAC-SHA-256 の最初の 32 バイト。iters が 1 未満なら実行時エラー）・
  `hex(ref data)`（小文字の 16 進の `str`）・`equal(ref a, ref b)`（`bool`。長さが同じなら、どこで違っても同じだけ時間をかけて比べる）。
- `std.base64`: `encode(ref data)`（標準の文字と `=` の埋め）・`decode(s)`（`Result<u8[], str>`）。decode は、長さが 4 の倍数でない・
  ほかの文字（空白や改行も）がある・`=` が終わり以外にある・終わりの余りのビットが 0 でないなら Err。
- `std.random`（暗号に使える OS の乱数。Linux・macOS は getentropy、Windows は BCryptGenRandom。取り出せなければ実行時エラー）:
  `bytes(n)`（`u8[]`。n が負なら実行時エラー）・`int(lo, hi)`（lo 以上 hi 未満の i64 を同じ確からしさで。hi <= lo か、hi - lo があふれるなら実行時エラー）。
  種を決めて同じ並びを出す乱数はない。

### 1.13 std.date・std.csv・std.regex（jwc だけ）

```
use std.date;
use std.csv;
use std.regex;
let d = date.parse("2026-10-31T19:37:52+09:00")?;    // DateTime（UTC に直す）
print(d.format(), d.weekday(), d.unix_ms());          // 2026-10-31T10:37:52Z 6 1793443072000
let rows = csv.parse("a,\"b,c\"\r\n1,2\r\n")?;      // [[a, b,c], [1, 2]]
let re = regex.compile("(\\d+)-(\\d+)")?;
print(re.replace_all("1-2 3-4", "$2-$1"));            // 2-1 4-3
```

- `std.date`（.jw だけ。地域の時刻のずれだけ std.time で OS に聞く）: DateTime はいつも UTC で、グレゴリオ暦を過去にも延ばす。年は 0〜9999。うるう秒は扱わない。タイムゾーンの名前（Asia/Tokyo など）からは引けない。
  - `DateTime`: `year`・`month`（1〜12）・`day`・`hour`・`minute`・`second`（0〜59）・`ms`（0〜999）。メソッドは `unix_ms()`（1970-01-01T00:00:00Z からのミリ秒。前なら負）・
    `weekday()`（ISO 8601 の 1＝月〜7＝日）・`day_of_year()`（1 月 1 日が 1）・`format()`（`2026-10-31T10:37:52Z`。ミリ秒が 0 でなければ `.123` を付ける）・`format_date()`（`2026-10-31`）・
    `format_at(offset_ms)`（時差 offset_ms の地域で見た形。`2026-10-31T19:37:52+09:00`。0 なら `+00:00`）・`format_local()`（`Result<str, str>`。OS のタイムゾーンで format_at）・
    `wall(offset_ms)`（その地域の時計の年月日と時刻を入れた DateTime。UTC ではないので unix_ms や format には使わない）。
    offset_ms は分の倍数で -23:59〜+23:59、結果が 0〜9999 年に入ること（でなければ実行時エラー）。
  - `now()`（今の UTC）・`from_unix_ms(t)`（0〜9999 年に入らなければ実行時エラー）・`make(year, month, day, hour, minute, second, ms)`（`Result`。ない日付・範囲の外なら Err）・
    `is_leap(year)`・`days_in_month(year, month)`（month が 1〜12 でなければ実行時エラー）。
  - `parse(s)`（`Result<DateTime, str>`）: `2026-10-31`（0 時 UTC）か `2026-10-31T10:37:52`＋秒の小数（1〜9 桁。ミリ秒より細かい分は切り捨て）＋`Z` か `±HH:MM`。
    `T` と `Z` は小文字でもよく、`T` の代わりに空白でもよい。時差は UTC に直す（直して範囲を出れば Err）。時差のない日時・ほかの形・ない日付は Err。
- `std.csv`（.jw だけ。RFC 4180）: 区切りは `,`、行の終わりは CRLF か LF、値はどれも文字列。
  - `parse(s)`（`Result<str[][], str>`）: `"` で囲んだ値の中には `,`・改行・`""`（`"` 1 つ）を書ける。終わりの改行のあとに空の行は作らない（空の行は `[""]` の 1 行）。
    囲んでいない値の中の `"`・閉じた `"` のあとの余計な文字・閉じていない `"` は Err（何行目かを書く）。
  - `format(ref rows)`: `,`・`"`・改行を含む値は `"` で囲み `"` を `""` にして、どの行も CRLF で終える。値のない行（`[]`）は空の行になり、読み直すと `[""]` になる。`quote(v)` は値 1 つ分。
- `std.regex`（.jw だけ）: 後戻りしない方式（Pike の VM）で、かかる時間はいつも「文字列の長さ × パターンの大きさ」に比べた程度。位置は文字単位（`slice` と同じ）。
  - 書ける形: 文字・`.`（改行のほかの 1 文字）・`[abc]` `[a-z]` `[^...]`（先頭の `]` と端の `-` はその文字）・`\d` `\w` `\s`（ASCII の数字・英数字と `_`・空白と \t〜\r）と `\D` `\W` `\S`（`[...]` の中には書けない）・
    `^` `$`（文字列の始めと終わり。行ごとではない）・`\b` `\B`（単語の境目。単語の文字は `\w`）・`(...)`・`(?:...)`・`|`・
    `*` `+` `?` `{m}` `{m,}` `{m,n}`（回数は 1000 まで。後ろに `?` で短い方を選ぶ）・`\n` `\t` `\r`・`\` と記号。後方参照・先読み・フラグはない。
    形が違う・繰り返しが重なる（`a**`）・組み立てた命令が 20000 を超えるなら、compile が Err（何文字目かを書く）。
  - 合うところは、いちばん左から始まるもの。同じ位置なら、`|` は左、繰り返しは長い方（`?` を付けたものは短い方）を先に選ぶ（Perl と同じ選び方）。
    ただし、空に合いうる中身の繰り返しの中の組が何を受け取るかは、Perl・Python と違うことがある。
  - `Regex` のメソッド: `is_match(s)`・`find(s)`（`Option<Match>`）・`find_all(s)`（重ならずに左から。空に合ったら 1 文字進める）・
    `replace_all(s, rep)`（rep の `$0`〜`$9` は全体と組の中身、`$$` は `$`。ない組・合わなかった組は空）・`split(s)`（空に合うところでは分けない）。
  - `Match`: `start`・`end`・`text`・`groups`（`Option<str>[]`。[0] は全体）・`group(k)`。`regex.escape(s)` は記号に `\` を付ける。

## 2. 字句

| 種類 | 書き方 |
|---|---|
| コメント | `//` から行の終わりまで |
| 名前 | 英字か `_` で始まり、英数字と `_` が続く（日本語などの文字も使える） |
| 整数 | `0`〜`18446744073709551615`。`0xff`（16 進）・`0b1010`（2 進）・`0o17`（8 進）とも書ける。`1_000_000` のように `_` で区切ってよい。型は使う先で決まる（[3.2](#32-数の書き方と型)） |
| 小数 | `1.5`、`0.25`。`.` の前後に数字が要る（`1.` や `.5` は書けない）。型は `f64` |
| 文字列 | `"..."`。使えるエスケープは `\n` `\t` `\r` `\0` `\"` `\\` と `\u{1F600}`（16 進で書いた文字の番号）。途中で改行はできない（改行を含むなら `"""..."""`。開きの `"""` の直後の改行は含めない。中では `"` をそのまま書ける）。`{式}` で値を埋め込める（`{{` `}}` は `{` `}` そのもの） |
| 真偽値 | `true` `false` |

予約語: `fn` `let` `mut` `if` `else` `while` `loop` `for` `in` `break` `continue` `match` `case` `default` `return` `struct` `enum` `interface` `static` `own` `ref` `as` `true` `false` `use` `pub` `const`

特別な名前: `this`（メソッドの中の自分）、`Some` `None`（Option の値）、`Ok` `Err`（Result の値）、`Option` `Result`（最初からある enum）、`print` `eprint` `drop` `assert` `read_line` `args` `exit` `read_file` `write_file` `run_command` `chr` `spawn` `chan`（組み込み関数）

## 3. 型

### 3.1 型の一覧

| 型 | 中身 |
|---|---|
| `i8` `i16` `i32` `i64` | 符号つき整数 |
| `u8` `u16` `u32` `u64` | 符号なし整数 |
| `usize` | 符号なし整数（添字用。64 ビットだが u64 とは別の型） |
| `f64` | 小数（64 ビットの浮動小数点数。IEEE 754） |
| `bool` | `true` / `false` |
| `str` | 文字列（1 種類だけ） |
| 構造体名・enum 名 | 定義した型。型の引数があれば `Pair<i64>` のように書く |
| `Option<T>` | `T` の値があるか、ないか（最初からある enum） |
| `Result<T, E>` | 成功の値 `T` か、失敗の値 `E`（最初からある enum） |
| `own T` | ヒープにある `T` の唯一の持ち主 |
| `ref T` | `T` を借りた参照（読むだけ） |
| `ref mut T` | `T` を借りた参照（書き換えてよい） |
| `T[N]` | 長さ N の配列（N はそのまま書いた数）。`i64[3][2]` は「長さ 3 の配列」が 2 つ |
| `T[]` | 伸び縮みするリスト |
| `V[K]` | 辞書（キー K から値 V を引く）。K は整数・`str`・`bool`（[8.10](#810-辞書)） |
| `fn R(A, B)` | 関数の値（引数 A, B を受け取り R を返す）。戻り値がなければ `fn(A, B)`（[5.1](#51-関数の値とクロージャ)） |
| `task<R>` | `spawn` で動かしたスレッド。`join()` で R を受け取る（[9.1](#91-スレッド)）。値を返さないスレッドは `task<()>` |
| `()` | 値なし。型の引数に書くときに使う（`task<()>`・`Result<(), str>`）。値も `()` と書く（`Ok(())`）。値なしの変数は作れない |
| `chan<T>` | スレッドのあいだで T を送るチャンネル（[9.1](#91-スレッド)） |
| 資源（`fs.File`） | 開いたファイルなど、外のものを持つ値。標準ライブラリだけが作れる（[1.3](#13-資源と-stdfsjwc-だけ)） |

- `own` / `ref` は右側の型全体にかかる（`ref i64[5]` は配列を借りた ref、`own i64[]` はリストの own）。
  要素にかけるときはかっこでまとめる（`(own Node)[]` は own を要素に持つリスト）。

- `f32` は未実装。

### 3.2 数の書き方と型

- 書いた数は、使う先の型になる。`let u8 b = 200;` の `200` は u8、`b + 1` の `1` も u8。
- 使う先の型が決まらなければ i64。
- そのまま書いた数が型に入らなければエラー（`let u8 b = 300;`）。
- 数だけの計算（`200 + 100` など）の結果が型に入らなければ警告。実行すると一周する。
- 使う先が `f64` なら、整数の書き方でも小数になる。`let f64 x = 1;` は 1.0、`let f64 h = 7 / 2;` は 3.5（小数として計算する）。

### 3.2.1 小数

- `f64` どうしで `+ - * / %`・比べる（`<` など、`==`）ができる。`<T: ordered>` にも当てはまる。
- 整数と小数をまぜて計算するとエラー。`n as f64` / `x as i64` と書いてそろえる。
- 小数 → 整数の `as` は 0 の方へ切り捨て、型の幅で一周する（`-2.9 as i64` は `-2`、`300.7 as u8` は `44`）。NaN は 0。
- 0 で割ってもエラーにならない（IEEE 754 のとおり `inf` / `NaN`）。
- 表示は整数と見分けがつくよう、`3.0` のように必ず小数点をつける。
- `x.to_bits()` は小数のビットの並びを `u64` で返す（IEEE 754。`1.5.to_bits()` は 4609434218613702656）。
- 文字列から小数へは `s.to_float()`（`Option<f64>`。`"2.5e3"`・`"inf"`・`"NaN"` も読める）。

### 3.2.2 数のメソッド

| メソッド | 使える型 | 意味 |
|---|---|---|
| `x.sqrt()` | `f64` | 平方根（負の数は `NaN`） |
| `x.abs()` | `f64`・整数 | 絶対値。整数は型の幅で一周する（`i64` の最小の値はそのまま）。符号なしの整数はそのまま |
| `x.max(y)`・`x.min(y)` | `f64`・整数 | 大きいほう・小さいほう。`y` は `x` と同じ型（数の書き方は `x` の型になる。`a.max(0)`） |
| `x.to_bits()` | `f64` | 上のとおり |

- 小数の `max` / `min` は、片方が `NaN` なら他方を返す（IEEE 754 の maxNum / minNum。Rust の `f64::max` と同じ。Go の `math.Max` は `NaN` を返すので違う）。

### 3.3 整数の変換

- 値が欠けない変換は自動でする。

| 元 | 自動で変換できる先 |
|---|---|
| 符号つき（i8〜i64） | 同じか広い符号つき |
| 符号なし（u8〜u64） | 同じか広い符号なし、自分より広い符号つき |
| u8 / u16 / u32 | usize |
| usize | なし |

- 欠けうる変換は `x as u8` と書く。範囲の外の値は一周する（`300 as u8` は `44`）。
- 型の違う整数どうしの演算は、片方へ欠けずにそろえられるなら広い方の型でする（`u8 + i64` は `i64`）。そろえられない組み合わせ（`i32` と `u32` など）はエラー。
- own や ref の中の型は変換しない（`ref u8` を `ref i64` として渡せない）。

### 3.4 型の推論

- 関数の引数・戻り値・構造体のフィールド・enum の中身には型を書く。
- `let` は型を省略できる。省略すると初期値の型になる（前から順に決めるだけ）。
- `ref mut T` は `ref T` が要るところにそのまま渡せる（逆はできない）。

## 4. 変数

```
let x = 1;            // 型は初期値から（i64）
let i64 y = 2;        // 型を書く
let mut n = 0;        // 書き換えられる
let u8 mut m = 0;     // 型を書くときは「型 mut 名前」
```

- 変数は普段は書き換えられない。書き換えるものだけ `mut` をつける。
- **シャドーイングはできない。** 見えている変数（同じブロック・外のブロック・引数）と同じ名前で宣言するとエラー。
  `let`・`for` の変数・match の case で受け取る名前・引数のどれでも同じ。別々のブロックでなら、同じ名前を使ってよい。
- 変数はブロック（`{ }`）の終わりで消える。そのとき中の own は解放される。

### 4.1 代入

| 書き方 | 意味 |
|---|---|
| `x = e;` | 代入。古い値の中の own は解放される |
| `x += e;` `-=` `*=` `/=` `%=` | `x = x + e;` などと同じ |
| `x++;` `x--;` | `x = x + 1;` などと同じ。**文としてだけ**書ける（`print(i++)` はエラー） |

- 代入先に書けるのは、変数・`*p`・`p.f`・添字 `a[i]`（配列・リスト・辞書）と、それらを重ねたもの（`ps[i].x`・`p.f[i]`）。
  辞書の `m[k] = v` だけは、ないキーなら足す（[8.10](#810-辞書)）。
- 書き換えてよいかは持ち主で決まる（[7.3](#73-書き換えてよいか)）。

## 5. 関数

```
fn i64 add(i64 a, i64 b) {   // fn、戻り値の型、名前、引数は「型 名前」
    return a + b;
}
fn i64 square(i64 x) {
    x * x                    // 最後の ; のない式が戻り値
}
fn greet(str name) {         // 戻り値がなければ型を書かない
    print("hello", name);
}
fn inc(i64 mut n) { ... }    // 書き換える引数は「型 mut 名前」
```

- 戻り値のある関数は、`return 値;` か、本体の最後の `;` のない式で値を返す。
- 値を返さないまま終わる道があるとエラー。`while` は 0 回で終わりうるので、後ろに return が要る。抜けない `loop` で終わるのはよい。
- 引数は値渡し。own や構造体を渡すとムーブする。
- 引数の型が `ref T` なら、呼ぶ側は `ref` を書かなくてよい。場所（変数・`p.f`・`a[i]`・`*p`）を渡すと自動で借りる（ムーブしない）。
  `show(a)` は `show(ref a)` と同じ。`a` が `own T` なら、`ref a` と同じく own の先を借りる。
- 場所でない式（関数の結果・`[1, 2]`・`P { x: 1 }`・`None` など、一時的な値）も `ref T` の引数に渡せる。値は呼び出しが終わるまで置いておき、
  終わったら捨てる（`own` の一時的な値なら own の先を借りる）。借りた ref を戻り値で持ち出すと、使うときに無効な参照のエラーになる。
  `[...]`・`P {...}`・enum の値・数は `T` を入れる先として型を決める（`count([1, 2])` は `i64[]`）。
- 引数の型が `ref mut T` なら、呼ぶ側でも `ref mut x` と書く（どの引数が書き換えられるかを、呼ぶ側で見えるようにするため）。

```
fn i64 sum(ref i64[] v) { ... }
fn add(ref mut i64[] v, i64 x) { v.push(x); }

let i64[] mut v = [1, 2];
add(ref mut v, 3);
print(sum(v));          // 借りて渡す。v はそのまま使える
```
- 呼び出しの深さが 5000 を超えると実行時エラー。
- 変数や引数に、関数・組み込み関数と同じ名前はつけられない。比べる関数は、同じファイル（モジュール）の関数だけ
  （ほかのモジュールの関数の名前はつけてよい。いつも `モジュール名.名前` と書くので紛れない）。
  jwc だけ。Rust 版は、モジュールの中でも最初のファイルの関数の名前を禁じる（凍結のため直さない）。

### 5.1 関数の値とクロージャ

```
fn i64 apply(fn i64(i64) f, i64 x) { f(x) }   // 関数の値を受け取る

let k = 10;
let add = (i64 x) -> x + k;      // その場で作る関数（クロージャ）。外の k を取り込む
print(add(1));                   // 11
print(apply((x) -> x * 2, 21));  // 42。入れる先の型が決まっていれば引数の型は書かなくてよい
let g = () -> { print("hi"); };  // 本体はブロックでもよい
v.sort_by(desc);                 // 名前のある関数もそのまま値として渡せる
fn fn i64(i64) adder(i64 n) { (x) -> x + n }   // クロージャを返す
```

- 書き方は `(引数, ...) -> 式` か `(引数, ...) -> { ... }`。引数は `型 名前` か、名前だけ。
  名前だけのときは、入れる先（関数の引数・型を書いた変数・戻り値）の関数の型から決める。決められなければエラー。
- 戻り値の型は、入れる先の関数の型があればそれ、なければ本体の最後の式の型。入れる先の型がないときは `return` は使えない。
- **外の変数は、クロージャを作ったときに値として取り込む。**
  - コピーできる値（数・文字列・ref など）はコピー。own や構造体などはムーブ（元の変数は使えなくなる）。引数に書いたクロージャは下のとおり借りる。
  - 取り込んだ変数は書き換えられない（`n = 2` はエラー）。書き換えたいときは外で `ref mut` を取り、それを取り込んで `*r = ...` と書く
    （取り込んだ own の `*o = ...` は、写しの中なのでエラー）。
  - 取り込んだ own を、クロージャの中から値として取り出すことはできない（何度も呼べるので）。`*a` や `ref` で読むのはよい。
  - 取り込んだ ref は世代チェックで守られる。借りた先が先に消えていれば、呼んだときに実行時エラー。
  - **関数・メソッドの引数にその場で書いたクロージャは、ムーブする値（own・構造体・リスト・辞書など）を借りて取り込む**
    （`ref 名前` を取り込むのと同じ。中では `ref T` に見える）。なので元の変数はそのあとも使える。
    関数の値と task は借りずに、今までどおり取り込む。`spawn` に渡すクロージャは借りない（ほかのスレッドへ移す）。

    ```
    let m = ["a": 3, "b": 1];
    let mut ks = m.keys();
    ks.sort_by((x, y) -> m[x] < m[y]);   // m を借りるだけ
    print(m);                            // まだ使える
    ```

    呼ばれた関数がそのクロージャを持ち続け、借りた変数が消えたあとに呼ぶと、ダングリングの実行時エラー。
- 関数の値は、取り込んだ値にムーブするものがあればムーブ、なければコピー。
- 呼べるのは、関数の値を入れた変数・引数（`f(x)`）。構造体のフィールドに関数の値を置くのは未実装。
- メソッドと型の引数のある関数は、まだ値にできない（クロージャで包む）。
- 外のループへの `break` / `continue` は、クロージャの中には書けない。
- print では `<関数>` と出る。

## 6. 式と文

### 6.1 ブロック

- `{ 文; 文; 式 }`。最後の `;` のない式がブロックの値になる。
- `}` で終わる文（`if` `while` `for` `loop` `match`）の後ろには `;` は要らない。

### 6.2 演算子（強い順）

| 強さ | 演算子 | 型 |
|---|---|---|
| 1 | `.f`（フィールド）、`f(...)`（呼び出し）、`e?` | `?` は Result / Option（[8.4](#84-result-と-)） |
| 2 | `-x` `!x` `~x` `*p` `ref x` `ref mut x` | `-` は符号つき整数と f64。`!` は bool。`~` は整数（ビットの反転） |
| 3 | `x as T` | 数 → 数（整数と f64） |
| 4 | `*` `/` `%` | 整数・f64 |
| 5 | `+` `-` | 整数・f64。`+` は str どうしの連結にも使える |
| 6 | `<<` `>>` | 整数。結果は左の型。右はずらす数（どの整数の型でもよい） |
| 7 | `&` | 整数（ビットの and） |
| 8 | `^` | 整数（ビットの xor） |
| 9 | `\|` | 整数（ビットの or） |
| 10 | `<` `<=` `>` `>=` | 整数・f64・str（辞書順）・`<T: ordered>` の T |
| 11 | `==` `!=` | 整数・f64・bool・str（同じ型どうし） |
| 12 | `&&` | bool（左が false なら右は評価しない） |
| 13 | `\|\|` | bool（左が true なら右は評価しない） |

- `own e` は右側の式全体を取る（`own n * 2` は `own (n * 2)`）。
- 整数のあふれは、その型の幅で一周する（`i8` の 127 + 1 は -128）。
- 0 で割ると実行時エラー。
- `&` `|` `^` の両辺は `+` と同じく型をそろえる。ビット演算は比べるより強い（Rust と同じ。`x & 1 == 0` は `(x & 1) == 0`）。
- `a << n` / `a >> n`: `>>` は符号つきなら符号を残す（算術シフト）、符号なしなら 0 を入れる。n が型の幅以上なら、`<<` は 0、`>>` は 0 か -1（Go と同じ）。n が負なら実行時エラー。
- `&=` などのビット演算の複合代入はない（`x = x & m;` と書く）。

### 6.3 if

```
if x > 0 { print("正"); } else if x == 0 { print("ゼロ"); } else { print("負"); }
let m = if a > b { a } else { b };
```

- 条件は `( )` で囲まない。`{ }` は必須。条件は bool。
- 値として使うときは `else` が必須。値は各ブロックの最後の `;` のない式。
- 値として使う if の中でも `return` は書ける（ブロックの最後の文にすれば、そのブロックは値を持たなくてよい）。外のループへの `break` / `continue` は使えない。
- 条件の中に構造体リテラルを直接は書けない（括弧で囲めば書ける）。三項演算子はない。

### 6.4 ループ

| 書き方 | 意味 |
|---|---|
| `loop { }` | 無限ループ |
| `while 条件 { }` | 条件が true の間 |
| `for i in range(終わり) { }` | 0 から 終わり-1 まで |
| `for i in range(始め, 終わり) { }` | 始め から 終わり-1 まで |
| `for i in range(始め, 終わり, 刻み) { }` | 刻みずつ。負の刻みで逆向き（`range(10, 0, -1)`） |
| `for x in a { }` | 配列・リストの要素を順に（[8.9](#89-リスト)） |
| `for ... { } else { }` | **1 回も回らなかったときだけ** else を実行（Python の for-else とは違う） |

- `break` で抜け、`continue` で次の周回へ。内側のループだけに効く。
- else のある for の本体に（そのループを抜ける）`break` があると警告する（Python の for-else と読み違えやすいため。実行は続く）。
- ループ変数は書き換えられない。型は始めと終わりの型（刻みはいつも i64）。刻みが 0 だと実行時エラー。
- do-while はない。

### 6.5 match

```
match n {                         // 文として
    case 0 -> print("zero");
    case 1, 2 -> print("small");
    case 3 -> {
        print("three");
    }
}
let name = match n {              // 値として
    case 0 -> "zero";
    default -> "many";
};
```

- 上から順に調べ、最初に当てはまった case だけを実行する（fall-through はない）。
- case に書けるのは、整数・bool・文字列の値と、enum の種類（[8.7](#87-match-で中身を受け取る)。形を重ねてもよい）。
- 同じ値の case が二回あるとエラー。
- 整数・bool・文字列の match: 値として使うときは default が必須（true と false を両方書いたときを除く）。文なら省略できる。
- enum の match: 全部の種類を書くか default が必須（文でも）。

### 6.6 文字列の埋め込み

```
let x = 3;
print("x = {x}, 次は {x + 1}, p = {p.show()}");   // x = 3, 次は 4, p = ...
print("{{x}} はそのまま");                          // {x} はそのまま
```

- `"..."` の中の `{式}` は、その値を文字にして埋め込む。print と同じく、どんな値でも埋め込める。
  own と ref は、指している中身を出す（`{o}` と `{*o}` は同じ）。無効になった ref を埋め込むと実行時エラー。
- 式は借りて読むだけ（own を埋め込んでもムーブしない）。結果は `str`。
- `{ }` の中には文字列（`"`）を書けない（先に変数に入れる）。`{` / `}` そのものは `{{` / `}}`。

### 6.7 文字列のメソッド

```
let s = "日本語テキスト";
print(s.len(), s.slice(0, 3));        // 7 日本語
for w in "a b c".split(" ") { ... }   // w は "a"、"b"、"c"
match "42".to_int() {
    case Some(n) -> print(n + 1);
    case None -> print("数ではない");
}
```

- 長さと位置は**文字単位**（バイトではない）。`"日本語".len()` は 3。バイト数は `byte_len()`。
- 文字列を返すメソッドは新しい文字列を返す（元の文字列は変わらない）。
- 文字列は `+` でつなぐ。`==` `<` などで比べる（`<` は辞書順）。
- `s += x`（`s = s + x`）は、s がほかと共有されていなければその場で後ろに足すので、くり返しても速い（毎回まるごと複製しない）。
- 番号から文字を作るのは組み込み関数 `chr(n)`（[9](#9-組み込み関数)）。

| メソッド | 戻り値 | 動き |
|---|---|---|
| `len()` | `i64` | 文字数 |
| `byte_len()` | `i64` | バイト数（UTF-8）。`"日本語".byte_len()` は 9 |
| `chars()` | `str[]` | 1 文字ずつのリスト（文字型はまだないので、1 文字の str） |
| `bytes()` | `u8[]` | バイト（UTF-8）のリスト |
| `code()` | `i64` | 1 文字目の番号（Unicode）。`"a".code()` は 97。空の文字列なら実行時エラー |
| `lines()` | `str[]` | 行ごとのリスト（改行は含まない） |
| `split(sep)` | `str[]` | `sep` で分けたリスト。`sep` が空ならエラー |
| `trim()` | `str` | 前後の空白を取る |
| `to_upper()` / `to_lower()` | `str` | 大文字・小文字にする |
| `contains(x)` / `starts_with(x)` / `ends_with(x)` | `bool` | x を含むか・x で始まるか・x で終わるか |
| `find(x)` | `Option<i64>` | x が最初に出てくる位置（文字単位） |
| `slice(a, b)` | `str` | a 文字目から b 文字目の手前まで。範囲の外なら実行時エラー |
| `replace(a, b)` | `str` | a をすべて b に置き換える |
| `repeat(n)` | `str` | n 回くり返す（n が負、または長さ × n が 2^62 バイトを超えると実行時エラー） |
| `to_int()` | `Option<i64>` | 整数に変換する（前後の空白は無視）。変換できなければ None |
| `to_float()` | `Option<f64>` | 小数に変換する（前後の空白は無視。`1e5`・`.5`・`inf`・`nan` も読める）。変換できなければ None |

## 7. 所有権と参照

### 7.1 own

```
let a = own 5;      // ヒープに 5 を置き、a が唯一の持ち主
let b = a;          // ムーブ。a はもう使えない
print(*b);          // * で中身を読む
```

- own を値として読むとムーブする（代入・引数・return・構造体に入れる）。ムーブした変数を使うと実行時エラー（どこでムーブしたかを示す）。
- 持ち主がスコープを抜ける・`drop` される・上書きされると、中身は解放される。中にある own も連鎖して解放される。
- 構造体と、own や構造体を中に持ちうる enum も、値として読むとムーブする。
- own や構造体を、`*p` や `p.f` から値として取り出すことはできない（`ref` で借りるか、`*p = ...` で置き換える）。
  for の要素・クロージャに取り込んだ値の中の own（own の配列）も同じで、読むだけの写しでも取り出せない（実行時エラー。`ref s.b` で借りる）。
  ジェネリックな関数の中で型の引数 `T` として読んでも同じ（`T` が own なら取り出せない）。

### 7.2 ref（世代つき参照）

```
let mut a = own 1;
let r = ref a;          // 読むだけ
let w = ref mut a;      // 書き換えてよい
*w += 1;
```

- `ref` で借りられるのは場所（変数・`*p`・`p.f`・`a[i]`）。
  - own を持つ場所なら、own の先を借りる（`ref a` は `a` の中身を指す）。
  - ref を持つ場所なら、同じものを指す（`ref r` は r の先を借りる）。
  - それ以外（整数・構造体・enum・配列・リストをそのまま持つ場所）なら、その場所そのものを借りる。
    変数に再代入すると、借りた ref からも新しい値が見える。
- リストの要素（とその中のフィールド）も借りられる（`ref mut ps[i]`・`ref ps[i].x`・`ps[i].bump()`）。要素を借りた ref は、
  - そのリストの**要素が動く**と無効になる: 長さが変わる（`push`・`pop`・`insert`・`remove`・`drain`）、並びが変わる（`sort`・`sort_by`・`sort_unstable`・`sort_unstable_by`・`reverse`）。
  - リストが解放される（スコープを抜ける・再代入・`drop`）か、ほかのスレッドへ送られても無効になる。
  - リストを別の変数へムーブしても、要素に代入しても有効なまま（own の先を借りた ref と同じ。代入した新しい値が見える）。
  - リストの中のリスト（`i64[][]`）の要素を借りた ref は、内側のリストだけを見る（外側に `push` しても有効）。
- 辞書の値の中は借りられない（キーを取り除くと値の場所が詰まるため）。値を借りたいときは、値を own にする（`(own T)[K]`）。
- 変数を借りた ref は、その変数がスコープを抜けるか、ムーブされると無効になる。
- ref の型に寿命は書かない。借りた先が解放されたあとに ref を使うと、**必ず**実行時エラーになる。エラーは「借りた → 解放された → 使った」の時系列で出る。
  - エラーは捕まえられず、プログラムはその場で止まる（終了コード 1。[10](#10-エラーと警告)）。`--release` で作った実行ファイルは、使った位置だけを出す。
  - 実行ファイルは「解放された」の位置を、区画を使い回したあとも直近 4096 回分だけ覚えている。それより古いと「解放された」の行は出ない（止まり方と残りの表示は同じ。[native.md](native.md)）。
- 安全と証明できた箇所の検査は、静的解析が実行前に消す（速さのため）。
- `ref mut` はいくつ同時にあってもよい。同じ場所を指す `ref` と `ref mut` も共存できる。
  `ref mut` は書き換えの許可であり、その場所をほかの参照が指していないという保証ではない。
- **世代は周回させず、上限で区画を退役させる（2026-10-26実装）。** 論理世代は64ビットに収まる範囲で単調に進める。
  状態bitを詰める実装は使用可能な上限を小さくしてよいが、加算・送信・リストの更新等であふれる前に確認する。0や過去の値へ戻してはならない。
  退役した区画の識別子は再利用せず、世代値が一致してもそこへのrefは無効とする。
  生きた値の使用を続ける必要があれば新しい区画へ所有責任を移し、安全に移せない場合は資源エラーで停止する。
  古いrefが読む検査用metadataは、そのrefを使用できる間は安全に保持する。詳細と証明範囲は [退役の規則](formal-semantics.md#有限世代と退役の規則)。
  Rust/IR/C/native/jwcへ反映した。[実装と検証範囲](generation-retirement.md) を参照する。
  生きたC区画の上限では今回は資源エラーで停止し、移し替えて継続する処理は実装していない。
  これは言語全体・コンパイラ全体の形式検証の完了ではない。
- 今は ref と own を比べたり、辞書のキーにしたり、文字にしたりできない。将来足すときは、読むだけの写しの印（実行ファイルではポインタの下の 1 ビット）を落として本物と同じに扱うこと。

### 7.3 書き換えてよいか

| 場所 | 書き換えてよい条件 |
|---|---|
| 変数 `x` | `x` が `mut` |
| own の先（`*a`、`a.f`） | 持ち主 `a` が書き換えてよい |
| ref の先（`*r`、`r.f`） | `r` が `ref mut` |
| 構造体のフィールド `s.f` | `s` が書き換えてよい |

- `ref mut x` で借りるには、`x` が書き換えてよい場所であること。
- 違反は実行前のエラー。

## 8. 構造体と enum

### 8.1 構造体

```
struct Player { str name; own Point pos; i64 score; }

let p = own Player { name: "勇者", pos: own Point { x: 0, y: 0 }, score: 0 };
print(p.name, p.pos.x);
```

- フィールドは「型 名前;」。リテラルではフィールドを書く順番は自由だが、全部書く。
- `p.f` は、`p` が構造体・own・ref のどれでも使える（own / ref は自動でたどる。1 段まで）。
- 自分自身を（own を通さず）じかに含む構造体はエラー。

### 8.2 enum と Option

```
enum Shape {
    Circle(i64 r);
    Rect(i64 w, i64 h);
    Empty;
}
let s = Shape.Circle(5);

fn Option<i64> find(i64 n) { ... }
let x = Some(5);
let Option<i64> y = None;
```

- 種類の中身は「型 名前」。値は `型名.種類(中身)`。
- Option の値は `Some(x)` と `None`。`None` の型は入れる先の型から決める（決まらなければエラー）。
- Option と Result には次のメソッドがある。

| メソッド | 結果 | 動き |
|---|---|---|
| `o.is_some()` / `o.is_none()` | `bool` | 中身は動かさない |
| `o.unwrap_or(d)` | `T` | `Some(x)` なら x、`None` なら d。o はムーブ |
| `o.ok_or(e)` | `Result<T, E>` | `Some(x)` → `Ok(x)`、`None` → `Err(e)`。o はムーブ |
| `o.map(f)` | `Option<U>` | `Some(x)` → `Some(f(x))`。o はムーブ |
| `r.is_ok()` / `r.is_err()` | `bool` | 中身は動かさない |
| `r.unwrap_or(d)` | `T` | `Ok(x)` なら x、`Err` なら d。r はムーブ |
| `r.ok()` | `Option<T>` | `Ok(x)` → `Some(x)`、`Err` → `None`。r はムーブ |
| `r.map(f)` | `Result<U, E>` | `Ok(x)` → `Ok(f(x))`。r はムーブ |

  ```
  let n = "12".to_int().unwrap_or(0);
  let m = s.to_int().ok_or("数ではない")?;
  ```
- 中身が数・bool・str・ref だけの enum は、値として読むとコピー。own や構造体を持ちうるならムーブ。
- enum どうしの `==` はない（match を使う）。

### 8.3 ジェネリクス（型の引数）

```
fn T pick<T>(bool first, T a, T b) { if first { a } else { b } }
struct Pair<T> { T a; T b; }
enum Either<L, R> { Left(L value); Right(R value); }

let x = pick(true, 1, 2);              // T は i64
let p = Pair { a: "x", b: "y" };       // Pair<str>
let Pair<u8> q = Pair { a: 1, b: 2 };  // 入れる先の型からも決まる
```

- 型の引数は名前のあと `<T, U>` に書く。関数・構造体・enum につけられる。
- 使うときに `<i64>` は書かない。引数・フィールド・入れる先の型から決める。決まらなければエラー。
  - 数どうしなら、欠けずにそろう広い方の型になる（`pick(true, a_u8, b_i64)` の T は i64）。
  - 数の書き方や `None` は、ほかの引数で決まった型に合わせる（`or_else(None, 5)` は T = i64）。
- 型の名前として書くときは型の引数が必要（`Pair` ではなく `Pair<i64>`）。
- 関数の中では `T` が何か分からないので、条件をつけないと `T` の値は渡す・返す・しまう・print しかできない。比べたりメソッドを呼んだりするには条件をつける（[8.6](#86-interface-と型の引数の条件)）。
- `T` が own や構造体なら値として読むとムーブ、数ならコピー（中身しだい）。
- `Option<T>` は最初から `enum Option<T> { Some(T value); None; }` として定義されている。
  `Some(x)` / `None` は `Option.Some(x)` / `Option.None` の短い書き方。

### 8.4 Result と `?`

```
fn Result<i64, str> div(i64 a, i64 b) {
    if b == 0 { Err("0 では割れません") } else { Ok(a / b) }
}

fn Result<i64, str> calc(i64 a, i64 b, i64 c) {
    let x = div(a, b)?;        // Err なら、それをそのまま calc から返す
    let y = div(x, c)?;
    Ok(x + y)
}
```

- 例外はない。失敗するかもしれない関数は `Result<T, E>` を返す。
- `o.ok_or(e)` は Option を Result にする（`Some(x)` → `Ok(x)`、`None` → `Err(e)`）。Result を返す関数の中で Option に `?` を使いたいときに:
  `let n = s.to_int().ok_or("数ではない")?;`（関数の形 `ok_or(o, e)` も使える）
  `Result` は最初から `enum Result<T, E> { Ok(T value); Err(E error); }` として定義されている。`Ok(x)` / `Err(e)` は短い書き方。
- `e?` は、e が `Ok(x)` なら `x`。`Err(e)` なら、関数からその `Err(e)` を返す。
  - 使えるのは Result を返す関数の中だけ。Err の型は関数の Err の型と同じでなければならない（変換はしない）。
  - Option にも使える（`None` ならそのまま返す）。Option を返す関数の中だけ。
  - 式のどこに書いてもよい（`f(a()?, b()?)`）。抜けるとき、それまでに作った値や変数の own はすべて解放する。

### 8.5 メソッド

```
struct Point {
    i64 x;
    i64 y;

    fn i64 len2() {                 // 書き換えないメソッド
        x * x + y * y               // フィールドは this. なしで使える（this.x とも書ける）
    }

    fn mut move_by(i64 dx, i64 dy) { // 書き換えるメソッドは fn mut
        x += dx;
        y += dy;
    }
}

let mut p = Point { x: 3, y: 4 };
p.move_by(1, 1);
print(p.len2());
```

- メソッドは構造体の中、フィールドのあとに書く。形は関数と同じで、`fn mut` なら書き換えるメソッド。
- メソッドの中では、フィールドと同じ構造体のメソッドを `this.` なしで使える。メソッドの引数やローカル変数には、フィールドと同じ名前をつけられない（メソッドの中の `x` はいつもフィールド）。静的メソッドには this がないので、つけてよい。
- `this` は呼び出し元を借りた ref。ふつうのメソッドでは `ref 構造体`、`fn mut` では `ref mut 構造体`。
  - `fn mut` を呼べるのは、`ref mut` で借りられる相手だけ（`let mut` の変数、mut の持ち主の own、`ref mut`）。
  - 書き換えないメソッドの中でフィールドに代入するとエラー。
- `p.m()` の `p` は、構造体の値・own・ref のどれでもよい（`p.f` と同じく自動でたどる）。
  - 構造体の値（own に入っていないもの）が変数やフィールドにあるなら、その場所を借りて `this` にする。
    `this` を返せば、その変数が生きているあいだ使える。
  - リストの要素なら、その要素を借りて `this` にする（呼び出しの中でそのリストの要素が動けば、`this` は無効になる）。
  - 一時的な値と辞書の値は、呼び出しのあいだだけヒープに移して借り、終わったら元に戻す（一時的な値は解放する）。
    この場合、`this` を借りた ref を返しても、呼び出しが終わった時点で無効になる（使うと実行時エラー）。
  - 評価の順: `p` が変数や `a.b` なら、引数を先に評価してから借りる。`make().m()` のような一時的な値なら、それを先に評価する。
- 型の引数のある構造体のメソッドは、構造体の型の引数をそのまま使える（`struct Stack<T> { ... fn mut push(T v) { ... } }`）。メソッド自身の型の引数もつけられる。
- メソッドはフィールドと同じ名前にできない。
- **静的メソッド**（this のないメソッド）は `static fn` と書き、`構造体名.名前(...)` で呼ぶ。

  ```
  struct Point {
      i64 x;
      i64 y;
      static fn Point at(i64 x, i64 y) { Point { x: x, y: y } }
  }
  let p = Point.at(1, 2);
  ```

  - 同じ構造体のメソッドの中からは `at(1, 2)` と `構造体名.` なしで呼べる。静的メソッドの中にはフィールドも this もない。
  - 静的メソッドを値から（`p.at()`）、this のあるメソッドを構造体名から（`Point.len2()`）は呼べない。
- **enum にもメソッドを書ける**。決まりは構造体と同じ（`fn` / `fn mut` / `static fn`、`this` は呼び出し元を借りた ref）。

  ```
  enum Shape {
      Circle(f64 r);
      Rect(f64 w, f64 h);

      fn f64 area() {
          match this {
              case Circle(r) -> 3.0 * r * r;
              case Rect(w, h) -> w * h;
          }
      }
      fn mut double() {                       // this の先を丸ごと置き換えるときは *this = ...
          match this {
              case Circle(r) -> *this = Shape.Circle(r * 2.0);
              case Rect(w, h) -> *this = Shape.Rect(w * 2.0, h * 2.0);
          }
      }
      static fn Shape square(f64 s) { Shape.Rect(s, s) }
  }
  let mut s = Shape.square(3.0);
  s.double();
  print(s.area());
  ```

  - メソッドは種類のあと（最後の種類の `;` のあと）に書く。種類と同じ名前のメソッドは作れない。
  - enum にはフィールドがないので、中身は `match this` で取り出す。`this = ...` とは書けない（`*this = ...`）。
  - `Shape.名前(...)` は、名前が種類なら enum の値、そうでなければ静的メソッドの呼び出し。
  - enum は interface に当てはめられない（interface は構造体だけ）。
- **this を値で受け取るメソッド**は `fn take` と書く（構造体にも enum にも書ける）。呼ぶと receiver はムーブする（コピーできる値ならコピー）。

  ```
  enum Slot {
      Full(own Item item);
      Empty;
      fn take Option<own Item> open() {     // 中身の own を取り出して返せる
          match this {
              case Full(item) -> Some(item);
              case Empty -> None;
          }
      }
  }
  let item = slot.open();                  // slot はもう使えない
  ```

  - `this` は値（`Slot`）なので、`match this` は値の match になり、中身をムーブして受け取れる。
  - own / ref の先にある値には呼べない（`p.f` から値を取り出せないのと同じ）。ふつうのメソッドの中から this に呼ぶこともできない。
  - 構造体の `fn take` でも、フィールドの own を取り出すことはできない（今までどおり）。
  - `fn take` のメソッドは interface のメソッドにはなれない。
  - `fn take(...)` と書くと、take という名前のふつうのメソッドになる。

### 8.6 interface と型の引数の条件

```
interface Shape {
    fn i64 area();
    fn mut grow(i64 n);          // 書き換えるメソッドは fn mut
}

fn describe<T: Shape>(ref T s) {
    print(s.area());             // 条件の interface にあるメソッドは呼べる
}

fn T max<T: ordered>(T a, T b) { // ordered: 大小を比べられる（整数・f64・str）
    if a > b { a } else { b }
}
```

- interface はメソッドの形だけを並べる（本体は書かず `;`）。
- **構造体は何も宣言しなくても、メソッドがそろっていれば interface に当てはまる**（Go と同じ）。
  - 引数と戻り値の型が同じであること。interface で書き換えないメソッドを、構造体が `fn mut` で持っているときは当てはまらない。
- 条件は型の引数のあとに `: 名前` で書く。いくつもつけるなら `<T: Shape + ordered>`。
- 組み込みの条件 `ordered`: 整数・f64・str。`<` `<=` `>` `>=` `==` `!=` が使える。
- 条件は呼び出しの場所で確かめる。満たさなければ、足りないメソッドを示してエラー。
  型の引数のまま別のジェネリックな関数へ渡すときは、こちらにも同じ条件が要る。
- 条件は関数とメソッドの型の引数にだけつけられる（構造体・enum の型の引数にはつけられない）。
- **interface は型としても使える**（own か ref を通して）。当てはまる構造体なら何でも入れられ、メソッドは中身の構造体のものが呼ばれる。

  ```
  fn i64 area_of(ref Shape s) { s.area() }
  fn own Shape pick(bool big) {
      if big { own Rect { w: 10, h: 10 } } else { own Square { side: 1 } }
  }
  let ref Shape s = if c { ref sq } else { ref r };   // 入れる先の型で、枝の違う構造体をまとめる
  ```

  - `own Square` は `own Shape` に、`ref Square` は `ref Shape` に、そのまま渡せる（逆はできない）。
  - interface の値には、interface に書いたメソッドだけ呼べる（フィールドは読めない）。
  - そのままの `Shape`（own / ref なし）は書けない。中身の構造体によって大きさが違うため。

### 8.7 match で中身を受け取る

```
match s {
    case Circle(r) -> print(r);       // 中身を名前で受け取る
    case Rect(w, _) -> print(w);      // 受け取らない中身は _
    case Empty -> print("empty");
}
match ref t { ... }                    // 借りて match（t は own か ref）
```

- `match x` は値の match。x がムーブする値なら x は使えなくなり、中身は受け取った名前へ移る。
- `match ref x` / `match ref mut x` は借りるだけ。中身の own は `ref T` / `ref mut T` として受け取り、数などはコピーで受け取る。
  own を通さずに持つ構造体などの中身は受け取れない（`_` にする）。
- 種類をまとめた case（`case Circle(_), Rect(_, _) ->`）では中身を受け取れない。

**形を重ねる**: 中身の位置に、種類・数・文字列・真偽値を書ける。

```
match peek() {                          // Option<Tok>
    case Some(Op("-")) -> ...;          // Some で、中身が Op で、その中身が "-"
    case Some(Op(op)) -> ...;           // それ以外の Op
    case Some(Num(n)) -> ...;
    case Some(End), None -> ...;
}
match ref e {                           // 借りた match なら own の先にも重ねられる
    case Add(Lit(0), r) -> ...;         // e が Add で、左が own E.Lit(0)
    default -> ...;
}
```

- 上から順に調べ、形が全部当たった最初の case を選ぶ。調べるだけでは値は動かない（当たった case だけ中身を受け取る）。
- 中身の位置の名前は、小文字で始まれば受け取る名前、大文字で始まれば種類の名前（`Some(End)`）。`名前(...)` は種類。
- 値の match では、own の中には重ねられない（`match ref x` で借りれば重ねられる）。ref の中にも重ねられない。
- 全部の場合を書いたかは、重ねた形もたどって調べる（`Some(Some(x))` と `Some(None)` と `None` で全部）。足りなければ default が要る。
- 先に重ねていない `case Some(x)` があると、あとの `case Some(1)` は決して選ばれないのでエラー。

### 8.8 配列

```
let mut a = [1, 2, 3];          // i64[3]
let u8[3] b = [250, 0, 1];      // 型を書けば、数はその型になる
let z = [0; 4];                 // 同じ値を 4 つ（コピーできる値だけ）
a[0] = 10;
a[2] += 5;
print(a[1], a.len());           // 長さは len()（i64）
for i in range(0, a.len()) { print(a[i]); }
let grid = [[0; 3]; 2];         // i64[3][2]
let r = ref own_list[1];        // 要素の own は借りられる
```

- 長さは型の一部で、変わらない（伸び縮みさせるならリスト）。
- 配列 1 つの大きさは 1 MiB まで（`i64[131072]` まで）。値なので、実行ファイルではスタックに置くため。中の構造体・enum の大きさも数える。
  超えると実行前のエラー（大きいものはリストにする）。`[x; n]` で作るリストは 2^62 バイトまで。
- 添字はどの整数型でもよい。範囲の外は実行時エラー。
  **添字が定数（数だけの式）なら、範囲の外は実行前のエラー**（`a[3]`、`a[1 + 2]`、`a[-1]`）。
- 要素がコピーできる型（数・bool・str・ref）ならコピー、own や構造体を持つならムーブ（Rust と同じ）。
- 要素の own や構造体は値として取り出せない（`ref a[i]` で借りるか、`a[i] = ...` で置き換える）。
  置き換えると古い要素は解放され、それを借りた ref は無効になる。
- 書き換えられるかは持ち主で決まる（`let mut` の配列、mut の持ち主の own、`ref mut`）。
- `a` が own / ref なら `a[i]` は自動で先をたどる（`p.f` と同じく 1 段まで）。
- 空の配列 `[]` は型を書く（`let i64[0] e = [];`）。

### 8.9 リスト

```
let i64[] mut v = [];            // 入れる先が T[] なら、[...] はリストになる
v.push(10);
v.push(20);
print(v[0], v.len());            // 10 2
print(v.pop());                  // Some(20)（空なら None）
v.insert(0, 5);                  // 0 番目に 5 を入れる（後ろはずれる）
print(v.remove(1));              // 1 番目を取り出す（後ろは詰まる）
let (own Node)[] mut nodes = [];  // own を要素に持つリスト

for x in v { print(x); }         // 配列・リストを回す
for x in v { ... } else { print("空でした"); }
```

- メソッドは `len()`（i64）、`push(x)`、`pop()`（`Option<T>`）、`insert(i, x)`、`remove(i)`（取り出した要素 `T`）、
  `reverse()`（逆順にする）、`drain()`（全部の要素を新しいリストに移して返す。元は空になる）、`sort()`、`sort_by(f)`、`sort_unstable()`、`sort_unstable_by(f)`、
  `map(f)`（要素ごとに f を呼んだ結果の新しいリスト）、`filter(f)`（f が true を返した要素だけの新しいリスト）、
  `join(sep)`（`str[]` だけ。要素を sep でつないだ文字列。元のリストは変えない）。
- `map` / `filter` は配列にも使え、元のリストは変えない（mut でなくてよい）。f は要素を `for x in a` の x と同じ形で受け取る
  （コピーできればコピー、own なら `ref T`、構造体などは読むだけの `ref T`）。`filter` はコピーできる要素だけ（残す要素をコピーするため）。

  ```
  let names = people.map((p) -> p.name);            // str[]
  let adults = ages.filter((a) -> a >= 20);          // i64[]
  ```
  `len` 以外は書き換えなので、持ち主が mut であること。
- `remove(i)` の i は 0〜長さ-1、`insert(i, x)` の i は 0〜長さ（長さなら末尾に足す）。範囲の外は実行時エラー。
- リストは値として読むといつもムーブ（中身の数でもコピーしない）。
- 添字の範囲は実行時に調べる（長さが決まっていないので、実行前には調べない）。
- **`for x in a` は a を借りて回す**（ムーブしない）。回すのは、ループを始めた時点の要素。
  - 要素がコピーできる型なら x はそのコピー、own なら x はその要素を借りた `ref T`。
  - ループの中で要素を取り出したり置き換えたりすると、それを借りた x は無効になる（使うと実行時エラー）。
  - 構造体など、コピーできない要素なら、x はループを始めた時点の要素を読むだけの `ref T`（書き換えられない）。
    要素の中の own は元の要素のものなので、ループの中でその要素が取り除かれると、たどったときに実行時エラーになる。
    書き換えたいときは `for i in range(a.len())` と添字で回す。
- `for (i, x) in a` と書くと、i に添字（i64、0 から）も入る。
- **その場で作ったリスト（`a.drain()`・リストを返す関数の呼び出し）を回すときは、要素を前から値として受け取る**（x の型は `T`）。
  構造体や own もムーブして受け取れるので、関数やスレッドへそのまま渡せる。途中で抜けると、残りの要素は捨てられる。

  ```
  for job in jobs.drain() {          // jobs の要素を 1 つずつ取り出す（jobs は空になる）
      ts.push(spawn(() -> run(job)));
  }
  ```

並べ替え（配列にも使える）:

```
v.sort();                              // 小さい順。要素は整数・f64・文字列
ps.sort_by((a, b) -> a.age < b.age);   // 「a を b より前に置くか」を返す関数で並べる
v.sort_unstable();                     // 安定でない（同じ値の順は決めない）ぶん速い
ps.sort_unstable_by((a, b) -> a.age < b.age);
```

- `sort` / `sort_by` は安定（同じと見なされた要素は元の順のまま）。`sort_unstable` / `sort_unstable_by` は安定でない
  （同じと見なされた要素の順は決めない。ただし同じプログラムなら毎回同じ）。どれも持ち主が mut であること。
- どれも最悪でも n log n 回ほどの比べで終わる。すでに並んでいる（逆順も）ところは速い。
  `sort` / `sort_by` は並んだ部分を合わせていく手順（Timsort の仲間）で比べる回数が少ない。`sort_unstable` / `sort_unstable_by` は
  pdqsort に近い手順で、作業用の場所を使わない（比べる回数は少し多い）。整数の `sort()` / `sort_unstable()` は比べずに並べることがある（結果は同じ）。
  関数が矛盾した答えを返しても（`a < b` と `b < a` が両方真など）止まり、要素はなくならない（並び方は決めない）。
- 比べる関数を呼ぶ回数と順番は決まっている（interp と実行ファイルで同じ）。
- `sort_by` の関数が受け取る要素は `for x in a` の x と同じ（コピーできればコピー、own なら `ref T`、それ以外は読むだけの `ref T`）。
- 並べ替えの途中でリストの長さが変わると実行時エラー。

### 8.10 辞書

```
let i64[str] mut ages = [];          // 空の辞書（入れる先の型が辞書なら [] は空の辞書）
ages["tom"] = 30;                    // 入れる（同じキーがあれば置き換え）
ages["tom"] += 1;
let m = ["a": 1, "b": 2];            // 書いて作る。型は最初の組から決まる（i64[str]）
print(m["a"]);                       // 1。キーがなければ実行時エラー
print(m.get("z"));                   // None（あれば Some(値)）
for (k, v) in m { print(k, v); }     // 入れた順に回る
let (own Node)[i64] mut nodes = [];  // own を値に持つ辞書
```

- キーにできるのは整数・`str`・`bool`。
- 辞書は値として読むといつもムーブ（リストと同じ）。
- 回す順・表示の順は入れた順。`remove` すると後ろが詰まる。表示は `[a: 1, b: 2]`。
- メソッド:

| メソッド | 結果 | 意味 |
|---|---|---|
| `len()` | `i64` | 組の数 |
| `contains(k)` | `bool` | キーがあるか |
| `get(k)` | `Option<V>` | 値の写し。V がコピーできる型のときだけ使える |
| `remove(k)` | `Option<V>` | 取り出す（持ち主が mut であること） |
| `keys()` | `K[]` | キーのリスト（入れた順） |

- `m[k] = v` と `remove` は書き換えなので、持ち主が mut であること。
- `m[k].f = ...` のように、中の値の場所に書き込める。`ref m[k]` で借りることはできない（リストと同じく、中の場所が変わるため）。
- `for (k, v) in m` の v は、`for x in a` の x と同じ規則で受け取る（コピーできればコピー、own なら `ref T`、それ以外は読むだけの `ref T`）。
  キーだけ回すなら `for k in m.keys()`。`for x in m` はエラー。
- `m[k]` の k は、もう一つの `[...]` に続けて書ける（`mm["a"]["b"] = 1`）。ただし外側のキー `"a"` がなければ実行時エラー。

## 9. 組み込み関数

| 関数 | 動き |
|---|---|
| `print(a, b, ...)` | 値を空白区切りで 1 行に出す。引数はムーブしない。own と ref は指している中身を出す（無効な ref なら実行時エラー）。実行ファイルでは、標準出力が端末なら 1 行ごとに書き出し、端末でなければためてまとめて書き出す（`eprint`・`read_line`・終わるときにも書き出す） |
| `eprint(a, b, ...)` | print と同じだが、標準エラーへ出す |
| `chr(n)` | 番号 n（Unicode）の文字 1 つの文字列。`chr(97)` は `"a"`。文字の番号でなければ実行時エラー |
| `exit(n)` | プログラムをすぐ終わらせる（`i64` の終了コード。0〜255 にすること）。ほかのスレッドも止まる。後片づけはしない。型は「戻らない」なので、値が要る場所（match の case など）にも書ける |
| `read_file(path)` | ファイルを読む（`Result<str, str>`）。読めない・UTF-8 でないなら `Err` |
| `write_file(path, text)` | ファイルに書く（`Result<(), str>`。あれば上書き）。書けなければ `Err` |
| `run_command(cmd, args)` | コマンドを動かして終わるのを待つ（`Result<i64, str>`。Ok は終了コード、シグナルで止まったら -1）。`args` は `str[]`（ムーブ）。起動できなければ `Err`。出力はそのまま画面へ |
| `drop(x)` | x を今すぐ解放する（x はムーブされる） |
| `assert(条件)` | 条件が false なら実行時エラー |
| `read_line()` | 標準入力から 1 行読む（`Option<str>`。改行は含まない）。入力が終わっていれば `None` |
| `args()` | コマンドライン引数（`str[]`）。`jabberwock prog.jw a b` なら `["a", "b"]` |
| `spawn(f)` | f を新しいスレッドで動かし、`task<R>` を返す（[9.1](#91-スレッド)） |
| `chan()` | 新しいチャンネル。型は入れる先から決める（`let chan<i64> c = chan();`） |

### 9.1 スレッド

```
fn i64 sum(i64[] xs) { ... }

let i64[] a = [1, 2, 3];
let t = spawn(() -> sum(a));     // a は新しいスレッドへムーブ（もう使えない）
print(t.join());                 // 終わるのを待って、戻り値を受け取る

let chan<str> c = chan();
let w = spawn(() -> {
    for i in range(3) { c.send("msg {i}"); }
    c.close();
});
loop {
    match c.recv() {             // Option<str>。閉じて空なら None
        case Some(m) -> print(m);
        case None -> break;
    }
}
```

- **ほかのスレッドへ渡せるのは「送れる型」だけ。** 数・真偽・小数・文字列・own・構造体・enum・配列・リスト・辞書・`chan`・`task` のうち、
  中に ref・関数の値・型の引数・interface を含まないもの。ref を渡そうとすると実行前のエラー。
  書き換えられるデータの持ち主はムーブして渡し、送った側に残る ref は無効にするので、そのデータを両方のスレッドから触ることはできない。
  ただし `chan` はコピーを通じてスレッド間で共有でき、その操作は実装側で同期する。
  文字列は送るときに複製する（変更されない文字列定数は共有してよい）。共有そのものを禁止するのではなく、データの取り合い（data race）を防ぐ。
- `spawn(f)` の f は、その場で書いた引数のないクロージャか、引数のない関数の名前。取り込んだ値は新しいスレッドへ移る
  （own はムーブ。送った側に残った ref は、使うとダングリングのエラー）。f は 1 回だけ呼ばれるので、取り込んだ own を中から取り出してよい。
  戻り値も送れる型であること。
- `t.join()` は 1 回だけ（2 回目は実行時エラー）。join しないまま task を捨てると、捨てたところで終わるのを待つ（戻り値は捨てる）。
  `task` はムーブする値。
- `c.send(x)` は x をムーブして送る（待たない）。`c.recv()` は値が来るまで待つ。閉じて空なら `None`。`c.close()` で閉じる。
  閉じたチャンネルへの send と、2 回目の close は実行時エラー。`chan` はコピーしても同じチャンネルを指す。
- スレッドを作れないとき（OS のスレッドの数かメモリが足りない）は、spawn の位置で実行時エラー。
  環境変数 `JW_MAX_THREADS` で、同時に動かせるスレッドの数（main のほか）の上限を決められる（テスト用。`--release` で作った実行ファイルでは効かない）。
- どれかのスレッドが実行時エラーで止まると、プログラム全体がそのエラーで止まる。
- すべてのスレッドが待ちのまま進めなくなったら、実行時エラー（デッドロック）。
- main が終わると、ほかのスレッドが終わるのを待ってから終わる。
- 1 回の print の行は、ほかのスレッドの出力と混ざらない。行の順番は実行のたびに変わりうる。

## 10. エラーと警告

| いつ | 例 |
|---|---|
| 実行前のエラー（構文・型） | `;` がない、型が合わない、mut でない変数への代入、match の書き忘れ、入らない数、入れ子が深すぎる（式・ブロック・型・形と、演算子や `.` の鎖を合わせて 256 段まで） |
| 実行前の警告（実行は続く） | 数だけの計算のあふれ、数だけの計算での 0 割り、else のある for の本体の break |
| 実行時のエラー | ダングリングした ref の使用、ムーブ済みの変数の使用、0 割り、assert の失敗、呼び出しが深すぎる、デッドロック、世代番号の上限（[7.2](#72-ref世代つき参照)） |

- 型や mut のエラーがあるプログラムは、1 行も実行されない。
- 実行時のエラーは捕まえられない。エラーを出してプログラム全体が止まり、終了コードは 1（ほかのスレッドも止まる。[9.1](#91-スレッド)）。

## 11. 未実装（決定済みのもの）

| 項目 | 決まっていること |
|---|---|
| あふれの確認 | `checked_add` のような明示的な関数 |
| interface の値から元の構造体を取り出す（型で分岐） | これから |
| `f32`・文字型 | これから |
| フォルダを分けたモジュール（`use a.b;`。`use std.名前;` は 1.2）・メソッドやフィールドごとの `pub`・`use ... as 別名` | これから |
| 辞書のキーに構造体を使う | これから |
| 写しの中の own を取り出すのを、実行前（型検査）のエラーにする | これから（今は実行時エラー。7.1） |
| 構造体のフィールドに関数の値を置く・メソッドを値にする | これから |
