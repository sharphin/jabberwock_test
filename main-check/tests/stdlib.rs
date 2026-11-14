//! 標準ライブラリ（`use std.名前;`。ソースはリポジトリの std/）のテスト

use jabberwock::run_with_modules;

fn ok(src: &str) -> String {
    ok_with(src, &[])
}

fn ok_with(src: &str, mods: &[(&str, &str)]) -> String {
    let o = run_with_modules(src, mods);
    if let Some(d) = &o.error {
        panic!("unexpected error:\n{}", jabberwock::render(d, "main.jw", &o.files));
    }
    assert_eq!(o.live_objects, 0, "heap objects leaked");
    o.output
}

fn err(src: &str) -> String {
    let o = run_with_modules(src, &[]);
    let d = o.error.expect("expected an error");
    jabberwock::render(&d, "main.jw", &o.files)
}

#[test]
fn text_module() {
    let src = r#"
use std.text;

fn main() {
    print(text.pad_left("42", 5, "0"));
    print("[" + text.pad_right("ab", 4, ".") + "]");
    print("[" + text.center("ab", 7, "*") + "]");
    print("[" + text.pad_left("abcdef", 3, " ") + "]");
    print("[" + text.trim_start("  hi ") + "]");
    print("[" + text.trim_end("  hi \n") + "]");
    let ws = text.words("  a bc\tdef  ");
    let j = ws.join(",");
    print("{ws.len()} {j}");
    print(text.rfind("abcabc", "bc").unwrap_or(-1), text.rfind("abc", "x").is_none());
    print(text.count("aaaa", "aa"), text.reverse("日本語"), text.is_digits("12"), text.is_digits(""));
    print(text.hex(255), text.to_base(-10, 2), text.to_base(0, 7));
    print(text.parse_int("-ff", 16).unwrap_or(0), text.parse_int("Z", 36).unwrap_or(0));
    print(text.parse_int("12a", 10).is_none(), text.parse_int("-", 10).is_none());
}
"#;
    assert_eq!(
        ok(src),
        "00042\n[ab..]\n[**ab***]\n[abcdef]\n[hi ]\n[  hi]\n3 a,bc,def\n4 true\n2 語本日 true false\nff -1010 0\n-255 35\ntrue true\n"
    );
}

#[test]
fn lists_module() {
    let src = r#"
use std.lists;

fn bool big(i64 x) { x > 8 }

fn main() {
    let i64[] mut xs = [3, 1, 4, 1, 5];
    print(lists.sum(xs), lists.max(xs).unwrap_or(0), lists.min(xs).unwrap_or(0));
    let i64[] empty = [];
    print(lists.max(empty).is_none(), lists.sum(empty));
    let i64[] mut ys = [9, 2];
    lists.extend(ref mut xs, ref mut ys);
    print("{xs.len()} {ys.len()} {xs[5]} {xs[6]}");
    let part = lists.slice(xs, 1, 3);
    print(part[0], part[1], lists.slice(xs, -5, 100).len());
    print(lists.any(xs, big), lists.all(xs, (x) -> x > 0));
    let s = lists.range_of(0, 10);
    print(lists.binary_search(s, 7).unwrap_or(-1));
    match lists.binary_search(s, 70) { case Ok(i) -> print(i); case Err(i) -> print("ins {i}"); }
    let f64[] fs = [0.5, 1.25];
    print(lists.sum_f64(fs));
    // own の要素も extend で動かせる
    let (own str)[] mut a = [own "x"];
    let (own str)[] mut b = [own "y", own "z"];
    lists.extend(ref mut a, ref mut b);
    print(*a[2], b.len());
}
"#;
    assert_eq!(ok(src), "14 5 1\ntrue 0\n7 0 9 2\n1 4 7\ntrue true\n7\nins 10\n1.75\nz 0\n");
}

pub const JSON: &str = "use std.json;
fn main() {
    let text = \"\"\"{{\"name\": \"jab\\\\\"ber\", \"n\": [1, 2.5, -3e2, true, null], \"o\": {{}}, \"u\": \"\\\\u00e9\\\\ud83d\\\\ude00\", \"e\": []}}\"\"\";
    match json.parse(text) {
        case Ok(v) -> {
            print(json.stringify(v));
            print(v.get(\"name\").as_str(), v.get(\"n\").at(1).as_num(), v.get(\"zz\").as_str().is_none());
        }
        case Err(e) -> print(e);
    }
    for bad in [\"\", \"[1,\", \"tru\", \"\\\"abc\", \"[1] x\"] {
        match json.parse(bad) {
            case Ok(v) -> print(\"ok?\", json.stringify(v));
            case Err(e) -> print(e);
        }
    }
    print(json.quote(\"a\\nb\"));
}
";

#[test]
fn json_module() {
    assert_eq!(ok(JSON), "{\"name\":\"jab\\\"ber\",\"n\":[1,2.5,-300,true,null],\"o\":{},\"u\":\"é😀\",\"e\":[]}\nSome(jab\"ber) Some(2.5) true\nJSON が読めません（1 文字目）: 値がありません\nJSON が読めません（4 文字目）: 値がありません\nJSON が読めません（4 文字目）: `true` のつもりの書き方が違います\nJSON が読めません（5 文字目）: 文字列が閉じていません\nJSON が読めません（5 文字目）: 値のあとに余分な文字があります\n\"a\\nb\"\n");
}

pub const RAND: &str = "use std.rand;
fn main() {
    let mut r = rand.Rng.seeded(42);
    print(r.next(), r.next());
    let mut counts = [0, 0, 0, 0, 0, 0];
    for i in range(6000) { let k = r.below(6); counts[k] += 1; }
    print(counts);
    let mut lo = 100;
    let mut hi = -100;
    for i in range(1000) {
        let x = r.range(-5, 5);
        lo = lo.min(x);
        hi = hi.max(x);
    }
    print(lo, hi, r.range(-9223372036854775807, 9223372036854775807) != 0);
    let f = r.float();
    print(f >= 0.0 && f < 1.0, r.chance(0.0), r.chance(1.0));
    let str[] mut a = [\"a\", \"b\", \"c\", \"d\", \"e\"];
    r.shuffle(ref mut a);
    print(a, a.len());
    let mut t = rand.Rng.from_time();
    print(t.below(10) < 10);
}
";

/// std.csv・std.base64・std.hash（tests/text_formats_example.jw）
#[test]
fn csv_and_base64_modules() {
    assert_eq!(ok(include_str!("text_formats_example.jw")), "5\n2 [name, age]\n2 [Smith, J, 42]\n2 [say \"hi\", ]\n2 [two\nlines, x]\n2 [, ]\nname,age\r\n\"Smith, J\",42\r\n\"say \"\"hi\"\"\",\r\n\"two\nlines\",x\r\n,\r\n\n\"Smith, J\",42\n[a: 1, b: 2]\n[a: 3, b: ]\n0 1\nCSV の 1 行目の `\"` が閉じていません\nCSV の 1 行目: 閉じた `\"` のあとに `,` か改行がありません\nCSV の 1 行目: 囲んでいない値の中に `\"` があります（`\"` で囲んで `\"\"` と書いてください）\n[]  \n[Zg==] f 66\n[Zm8=] fo 666f\n[Zm9v] foo 666f6f\n[Zm9vYg==] foob 666f6f62\n[Zm9vYmE=] fooba 666f6f6261\n[Zm9vYmFy] foobar 666f6f626172\n[5pel5pys6Kqe] 日本語 e697a5e69cace8aa9e\nbase64 の長さが 4 の倍数ではありません\nbase64 の長さが 4 の倍数ではありません\nbase64 にない文字があります: =\nAP+AAQ== 00ff8001 [0, 255, 128, 1] Some([0, 255, 128, 1])\n");
}

/// std.sets（tests/sets_example.jw）
#[test]
fn sets_module() {
    assert_eq!(ok(include_str!("sets_example.jw")), "true true false 2 false\ntrue false [b, a]\n[a, c] 2\n[b, a, c] [a] [b] [c]\ntrue false [b] true false\n[3, 1, 2, -5] true 4\n[h, e, l, o] true false\ndup 5\ndup 3\n");
}

/// std.cli（tests/cli_example.jw）
#[test]
fn cli_module() {
    assert_eq!(ok(include_str!("cli_example.jw")), "true false true x None [a.txt, b] false
true true true file Some(12) [-v, --x] false
true true true y None [-, -5] false
Err: 知らないオプションです: --nope
Err: 知らないオプションです: -z
Err: --out に値がありません
Err: --verbose は値をとりません
false false false - None [] true
false false false - None [] true
使い方: wc [オプション] <file>
行を数える

引数:
  <file>            読むファイル

オプション:
  -v, --verbose     詳しく出す
  -o, --out <値>    出力先（既定: -）
      --count <値>  回数
  -h, --help        この説明を出す
使い方: x

オプション:
  -h, --help  この説明を出す
");
}

/// 同じ種なら同じ並び（splitmix64 の最初の値は、知られている値と同じ）
#[test]
fn rand_module() {
    assert_eq!(ok(RAND), "13679457532755275413 2949826092126892291\n[985, 991, 1014, 1026, 985, 999]\n-5 4 true\ntrue false true\n[d, a, b, e, c] 5\ntrue\n");
}

/// 同じ名前のファイルがあっても、標準ライブラリはコンパイラの中のものを使う。ふつうの `use text;` はそのファイル
#[test]
fn std_is_not_overridden_by_local_files() {
    let local = "pub fn str pad_left(str s, i64 w, str f) { \"local\" }";
    let src = "use std.text;\nfn main() { print(text.pad_left(\"1\", 3, \"0\")); }";
    assert_eq!(ok_with(src, &[("text", local), ("std:text", local)]), "001\n");
    let src = "use text;\nfn main() { print(text.pad_left(\"1\", 3, \"0\")); }";
    assert_eq!(ok_with(src, &[("text", local)]), "local\n");
}

/// ふつうのモジュールの中からも使える
#[test]
fn std_from_a_module() {
    let fmt = "use std.text;\npub fn str money(i64 c) { text.pad_left(\"{c}\", 6, \" \") }";
    let src = "use fmt;\nuse std.text;\nfn main() { print(\"[\" + fmt.money(42) + \"]\", text.hex(16)); }";
    assert_eq!(ok_with(src, &[("fmt", fmt)]), "[    42] 10\n");
}

/// 別のスレッドでも同じ標準ライブラリを読み直せる
#[test]
fn std_in_threads() {
    let src = r#"
use std.text;
fn main() {
    let t = spawn(() -> text.pad_left("7", 3, "0"));
    print(t.join());
}
"#;
    assert_eq!(ok(src), "007\n");
}

#[test]
fn errors() {
    let e = err("use std.nope;\nfn main() {}");
    assert!(e.contains("標準ライブラリに `nope` はありません"), "{}", e);
    assert!(e.contains("使えるのは base64・browser・cli"), "{}", e);
    let e = err("use std.text;\nfn main() { print(text.nothing(1)); }");
    assert!(e.contains("モジュール `text` に `nothing` はありません"), "{}", e);
    let e = err("use std.text;\nfn main() { print(text.hex(\"x\")); }");
    assert!(e.contains("main.jw:2"), "{}", e);
    let e = err("use std.text;\nfn main() { let text = 1; }");
    assert!(e.contains("text"), "{}", e);
    let e = err("use std;\nfn main() {}");
    assert!(e.contains("モジュール `std` が見つかりません"), "{}", e);
    let e = err("use std.text;\nuse std.text;\nfn main() {}");
    assert!(e.contains("二回"), "{}", e);
    // 標準のモジュールと同じ名前のふつうのモジュールは、両方は使えない（jwc と同じ）
    let o = run_with_modules("use text;\nuse std.text;\nfn main() {}", &[("text", "pub fn i64 f() { 1 }")]);
    let e = jabberwock::render(&o.error.expect("expected an error"), "main.jw", &o.files);
    assert!(e.contains("`text` という名前のモジュールを二つ使っています"), "{}", e);
    // jwc だけの機能（資源）を使うモジュールは、凍結した Rust 版では読めない
    let e = err("use std.fs;\nfn main() {}");
    assert!(e.contains("構文エラー"), "{}", e);
}
