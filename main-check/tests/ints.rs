//! 整数型（i8〜i64、u8〜u64、usize）のテスト

use jabberwock::run_source;

fn out(body: &str) -> String {
    ok(&format!("fn main() {{\n{}\n}}", body))
}

fn ok(src: &str) -> String {
    let o = run_source(src, false);
    if let Some(d) = &o.error {
        panic!("unexpected error:\n{}", d.render("test.jw", src));
    }
    assert!(o.warnings.is_empty(), "unexpected warning: {}", o.warnings[0].title);
    assert_eq!(o.live_objects, 0, "heap objects leaked");
    o.output
}

fn err(body: &str) -> jabberwock::diag::Diagnostic {
    let o = run_source(&format!("fn main() {{\n{}\n}}", body), false);
    assert_eq!(o.output, "", "a program with a type error must not run");
    o.error.expect("expected an error")
}

// ---------------------------------------------------------------- 数の書き方は相手の型に合わせる

#[test]
fn literals_take_the_type_they_are_used_as() {
    assert_eq!(out("let u8 b = 200;\nlet u64 m = 18446744073709551615;\nlet i8 n = -128;\nprint(b, m, n);"),
        "200 18446744073709551615 -128\n");
    // `b + 100` の 100 は u8。計算も u8 なので一周する
    assert_eq!(out("let u8 b = 200;\nprint(b + 100);"), "44\n");
    // 型を書かなければ i64
    assert_eq!(out("let x = 200;\nprint(x + 100);"), "300\n");
}

#[test]
fn a_literal_that_does_not_fit_is_an_error() {
    let d = err("let u8 b = 300;");
    assert!(d.title.contains("`300` は `u8` に入りません"), "{}", d.title);
    assert!(d.labels[0].text.contains("0〜255"));
    assert!(err("let u8 b = -1;").title.contains("`-1` は `u8` に入りません"));
    assert!(err("let i8 b = 128;").title.contains("`128` は `i8` に入りません"));
    assert!(err("let u8 a = 1;\nprint(a + 1000);").title.contains("`1000` は `u8`"));
    assert!(err("let u8 a = 1;\nmatch a {\n case 300 -> print(1);\n}").title.contains("`300` は `u8`"));
}

#[test]
fn constant_arithmetic_that_overflows_its_type_is_a_warning() {
    let o = run_source("fn main() {\n    let u8 b = 200 + 100;\n    print(b);\n}", false);
    assert!(o.error.is_none());
    assert_eq!(o.output, "44\n");
    assert_eq!(o.warnings.len(), 1);
    assert!(o.warnings[0].labels[0].text.contains("u8 の範囲を超える"));
}

// ---------------------------------------------------------------- 一周する

#[test]
fn every_type_wraps_at_its_own_width() {
    assert_eq!(out("let i8 mut x = 127;\nx++;\nprint(x);"), "-128\n");
    assert_eq!(out("let u8 mut a = 0;\na -= 1;\nprint(a);"), "255\n");
    assert_eq!(out("let i32 a = 2147483647;\nprint(a + 1);"), "-2147483648\n");
    assert_eq!(out("let u64 m = 18446744073709551615;\nprint(m + 1, m * m);"), "0 1\n");
    assert_eq!(out("let i8 m = -128;\nprint(m / -1, m % -1);"), "-128 0\n");
    assert_eq!(out("let u16 a = 65535;\nprint(a * a);"), "1\n");
}

// ---------------------------------------------------------------- 変換

#[test]
fn widening_is_automatic() {
    let src = "
fn i64 scale(u8 a) {
    a
}

fn main() {
    let u8 a = 200;
    let i64 b = 1000;
    let i64 x = a;
    print(a + b, x * 1000, scale(a) * 1000);
    let u32 c = 70000;
    let usize n = c;
    let i16 s = a;
    print(n, s);
}";
    assert_eq!(ok(src), "1200 200000 200000\n70000 200\n");
}

#[test]
fn narrowing_needs_as() {
    let d = err("let i64 x = 5;\nlet u8 y = x;");
    assert!(d.title.contains("型が合いません"));
    assert!(d.note.unwrap().contains("`x as u8`"));
    // 符号つきと符号なしの同じ幅は、どちらにも欠けずには変換できない
    let d = err("let i32 a = 1;\nlet u32 b = 2;\nprint(a + b);");
    assert!(d.title.contains("型の違う整数どうし"), "{}", d.title);
    assert!(d.note.unwrap().contains("`as`"));
    // usize は u64 とは別の型
    assert!(err("let u64 a = 1;\nlet usize b = a;").title.contains("型が合いません"));
    assert!(err("let u8 a = 1;\nlet u8 b = 2;\nfor i in range(a, 300) { }").title.contains("`300` は `u8`"));
}

#[test]
fn as_converts_and_wraps() {
    assert_eq!(out("let i64 x = 300;\nprint(x as u8, -1 as u8, 255 as i8, x as i16);"), "44 255 -1 300\n");
    assert_eq!(out("let u8 a = 250;\nprint(a as i64 + 10, (a + 10) as i64);"), "260 4\n");
    assert!(err("print(true as u8);").title.contains("`as` で変換できるのは数"));
    assert!(err("print(1 as bool);").title.contains("数の型"));
}

#[test]
fn negation_is_only_for_signed_types() {
    let d = err("let u8 a = 1;\nprint(-a);");
    assert!(d.title.contains("符号なしの型 `u8` に `-` は使えません"), "{}", d.title);
    assert_eq!(out("let i8 a = -128;\nprint(-a);"), "-128\n");
}

// ---------------------------------------------------------------- いろいろな場所

#[test]
fn integers_in_loops_branches_and_structs() {
    let src = "
struct Pixel { u8 r; u8 g; u8 b; }

fn main() {
    let u8 s = 253;
    for i in range(s, 255) {
        let u8 x = i;
        print(x);
    }
    for i in range(s, 250, -1) {
        print(i);
    }
    let u8 a = 1;
    let wide = if true { a } else { 1000 };
    print(wide * 1000);
    let u8 narrow = if false { a } else { 2 };
    let p = own Pixel { r: 255, g: narrow, b: 0 };
    print(p.r + 1, p.g);
    let own u8 q = own 7;
    print(*q);
}";
    assert_eq!(ok(src), "253\n254\n253\n252\n251\n1000\n0 2\n7\n");
}

#[test]
fn pointers_do_not_convert() {
    let src = "fn show(ref i64 r) {\n    print(*r);\n}\nfn main() {\n    let a = own (5 as u8);\n    show(ref a);\n}";
    let d = run_source(src, false).error.expect("error");
    assert!(d.labels.iter().any(|l| l.text.contains("`ref u8` ですが、`ref i64` が必要")));
}

// ---------------------------------------------------------------- ビット演算・16 進数・定数

#[test]
fn bit_operations() {
    assert_eq!(
        out("print(12 & 10, 12 | 10, 12 ^ 10, ~5, 1 << 4, -16 >> 2, 255 >> 9, -1 >> 70, 1 << 64);
let u8 a = 200;
print(a & 15, a | 1, a ^ 255, ~a, a << 1, a >> 3, a >> 8);
let u64 big = 18446744073709551615;
print(big >> 63, big << 63, big >> (64 as u8), 3 + 4 << 1, 1 | 2 == 3, 6 & 3 ^ 1 | 8);
let i32 k = 5;
print(k << 30, k << 31, (k << 29) >> 29);
let Option<Option<i64>> o = Some(Some(1 << 2));
print(o);"),
        "8 14 6 -6 16 -4 0 -1 0\n8 201 55 55 144 25 0\n1 9223372036854775808 0 14 true 11\n1073741824 -2147483648 -3\nSome(Some(4))\n"
    );
}

#[test]
fn hash_with_bits() {
    // FNV-1a（64 ビット）
    assert_eq!(
        ok("
fn u64 fnv(str s) {
    let u64 mut h = 0xcbf29ce484222325;
    for b in s.bytes() {
        h = (h ^ b) * 0x100000001b3;
    }
    h
}
fn main() {
    print(fnv(\"\"), fnv(\"a\"), fnv(\"abc\"));
}"),
        "14695981039346656037 12638187200555641996 16654208175385433931\n"
    );
}

#[test]
fn bit_operation_errors() {
    assert!(err("print(true & false);").title.contains("型"));
    assert!(err("print(1.5 << 1);").title.contains("整数"));
    assert!(err("print(~true);").title.contains("整数"));
    let o = run_source("fn main() {\n    let n = -1;\n    print(1 << n);\n}", false);
    assert_eq!(o.error.expect("error").title, "負の数だけはずらせません");
}

#[test]
fn hex_binary_octal_and_underscores() {
    assert_eq!(out("let u64 all = 0xFFFF_FFFF_FFFF_FFFF;
let u8 m = 0x0f;
print(0xff, 0xFF + 0, 0b1010, 0o17, 1_000_000, 0x_ff, all, m);"), "255 255 10 15 1000000 255 18446744073709551615 15\n");
    assert!(err("print(0xfg);").title.contains("数の書き方"));
    assert!(err("print(0b102);").title.contains("数の書き方"));
    assert!(err("print(0x);").title.contains("数の書き方"));
    assert!(err("print(0x1_0000_0000_0000_0000);").title.contains("大きすぎ"));
}

#[test]
fn constants() {
    assert_eq!(
        ok("
const MAX = 5000;
const NAME = \"jw\";
const HOUR = 60 * 60;
const BIG = HOUR * 1000;
const RATE = 1.5;
const ON = true;
const MASK = 0xff;
fn f() {
    let u8 b = MASK;
    print(b, MAX + 1, NAME, HOUR, BIG, RATE * 2.0, ON, MASK & 0x0f);
}
fn main() {
    f();
    let u16 w = HOUR;
    let g = () -> MAX * 2;
    print(w, \"{NAME}-{MAX}\", g());
}"),
        "255 5001 jw 3600 3600000 3.0 true 15\n3600 jw-5000 10000\n"
    );
}

#[test]
fn constant_errors() {
    let e = |src: &str| run_source(src, false).error.expect("expected an error").title;
    assert!(e("const A = B; const B = 1; fn main() {}").contains("使えない名前"));
    assert!(e("const A = 1; fn main() { let A = 2; }").contains("定数の名前"));
    assert!(e("const A = 1; fn main() { for A in range(3) {} }").contains("定数の名前"));
    assert!(e("const A = 1; fn A() {} fn main() {}").contains("定数の名前"));
    assert!(e("fn i64 f() { 1 } const A = f(); fn main() {}").contains("定数ではありません"));
    assert!(e("pub const A = 1; fn main() {}").contains("pub"));
    assert!(e("const A = 1; const A = 2; fn main() {}").contains("二重"));
    assert!(e("const A = 300; fn main() { let u8 b = A; }").contains("入りません"));
}

#[test]
fn abs_max_min() {
    let o = out("
    let i32 a = -7;
    let u8 b = 200;
    let i64 m = -9223372036854775807 - 1;
    let i8 n = -128;
    let ref i32 r = ref a;
    print(a.abs(), a.max(3), a.min(-10), b.max(100), b.min(201), b.abs(), m.abs(), n.abs(), r.abs());
    let u64 big = 18446744073709551615;
    print(big.max(1), big.min(1), (3 - 10).abs());");
    assert_eq!(o, "7 3 -10 200 200 200 -9223372036854775808 -128 7\n18446744073709551615 1 7\n");
    assert!(err("let i64 a = 1; print(a.sqrt());").title.contains("メソッド"));
    let d = err("let i32 a = 1; let i64 b = 2; print(a.max(b));");
    assert!(d.title.contains("型"), "{}", d.title);
}
