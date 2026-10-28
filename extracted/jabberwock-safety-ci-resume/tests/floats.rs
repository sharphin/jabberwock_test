//! 小数（f64）のテスト

use jabberwock::{run_source, run_source_with};

fn ok(src: &str) -> String {
    let o = run_source(src, false);
    if let Some(d) = &o.error {
        panic!("unexpected error:\n{}", d.render("test.jw", src));
    }
    assert_eq!(o.live_objects, 0, "heap objects leaked");
    let off = run_source_with(src, false, false);
    assert_eq!(o.output, off.output, "elision changed the output");
    o.output
}

fn main_ok(body: &str) -> String {
    ok(&format!("fn main() {{\n{}\n}}", body))
}

fn err(body: &str) -> jabberwock::diag::Diagnostic {
    let o = run_source(&format!("fn main() {{\n{}\n}}", body), false);
    assert_eq!(o.output, "", "a program with a type error must not run");
    o.error.expect("expected an error")
}

#[test]
fn float_arithmetic_and_printing() {
    let o = main_ok("
    let y = 1.5;
    print(y + 0.25, y * 2.0, y / 2.0, -y, 7.5 % 2.0);
    print(3.0, 0.1 + 0.2, 1.0 / 0.0, \"y = {y}\");
    print(y < 2.0, y >= 1.5, y == 1.5, y != 1.5);");
    assert_eq!(o, "1.75 3.0 0.75 -1.5 1.5\n3.0 0.30000000000000004 inf y = 1.5\ntrue true true false\n");
}

#[test]
fn integer_literals_become_floats_where_a_float_is_wanted() {
    // 数の書き方は相手の型に合わせる。小数の場所では、整数の書き方でも小数として計算する
    let o = ok("
fn f64 area(f64 r) {
    3.14 * r * r
}

fn main() {
    let f64 x = 7 / 2;
    let f64 mut s = 0;
    s += 1;
    print(x, s, area(2), 2 * 1.5);
}");
    assert_eq!(o, "3.5 1.0 12.56 3.0\n");
}

#[test]
fn casts_between_integers_and_floats() {
    let o = main_ok("
    let n = 10;
    print(n as f64 / 4.0, 2.9 as i64, -2.9 as i64, 300.7 as u8);
    let f64 mut s = 0.0;
    for i in range(4) {
        s += i as f64 * 0.5;
    }
    print(s);");
    assert_eq!(o, "2.5 2 -2 44\n3.0\n");
}

#[test]
fn floats_are_ordered_and_copyable() {
    let o = ok("
fn T max<T: ordered>(T a, T b) {
    if a > b { a } else { b }
}

struct P {
    f64 x;
}

fn main() {
    let p = P { x: 0.5 };
    let f64[] mut v = [1.5, 2.5];
    v.push(p.x);
    print(max(1.5, 2.5), v, p.x);
}");
    assert_eq!(o, "2.5 [1.5, 2.5, 0.5] 0.5\n");
}

#[test]
fn float_errors() {
    let d = err("let x = 1.5;\nlet n = 3;\nprint(x + n);");
    assert!(d.title.contains("型が合わない演算"), "{}", d.title);
    assert!(d.note.as_deref().unwrap_or("").contains("as f64"));
    assert!(err("let i64 n = 1.5;").title.contains("型が合いません"));
    assert!(err("let b = true as f64;").title.contains("as"));
    assert!(err("let f32 x = 1.0;").title.contains("まだ使えません"));
}

#[test]
fn parse_floats_and_read_bits() {
    let o = main_ok("
    for t in [\"1.5\", \" -2.25e3 \", \"1.\", \".5\", \"1e\", \"e5\", \"inf\", \"-Infinity\", \"NaN\", \"abc\", \"\", \"+0.1\", \"1_0\", \"0x10\", \"7\", \"1E-2\"] {
        print(t.to_float());
    }
    let x = 1.5;
    let own f64 p = own 2.0;
    print(x.to_bits(), (0.1).to_bits(), (-0.0).to_bits(), p.to_bits());");
    assert_eq!(
        o,
        "Some(1.5)\nSome(-2250.0)\nSome(1.0)\nSome(0.5)\nNone\nNone\nSome(inf)\nSome(-inf)\nSome(NaN)\nNone\nNone\nSome(0.1)\nNone\nNone\nSome(7.0)\nSome(0.01)\n\
4609434218613702656 4591870180066957722 9223372036854775808 4611686018427387904\n"
    );
    assert!(err("let x = 1.5; print(x.round());").title.contains("メソッド"));
}

#[test]
fn sqrt_abs_max_min() {
    let o = main_ok("
    let x = 2.0;
    let own f64 p = own 9.0;
    print(x.sqrt(), (x - 5.0).abs(), x.max(3), x.min(1.5), p.sqrt(), (16.0).sqrt());
    print((-1.0).sqrt(), (0.0 / 0.0).max(1.0), x.max(0.0 / 0.0));");
    assert_eq!(o, "1.4142135623730951 3.0 3.0 1.5 3.0 4.0\nNaN 1.0 2.0\n");
    assert!(err("let x = 1.5; print(x.max());").title.contains("引数"));
    assert!(err("let x = 1.5; print(x.max(true));").title.contains("型"));
}
