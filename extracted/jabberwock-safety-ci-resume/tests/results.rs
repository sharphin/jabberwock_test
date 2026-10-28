//! Result と `?` のテスト

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

fn err(src: &str) -> jabberwock::diag::Diagnostic {
    run_source(src, false).error.expect("expected an error")
}

#[test]
fn result_values_and_match() {
    let src = "
fn Result<i64, str> div(i64 a, i64 b) {
    if b == 0 { Err(\"0 では割れません\") } else { Ok(a / b) }
}

fn main() {
    match div(10, 2) {
        case Ok(v) -> print(\"答え\", v);
        case Err(e) -> print(\"失敗\", e);
    }
    match div(1, 0) {
        case Ok(v) -> print(\"答え\", v);
        case Err(e) -> print(\"失敗\", e);
    }
    print(div(9, 3));
    let Result<u8, str> r = Result.Ok(255);
    print(r);
}";
    assert_eq!(ok(src), "答え 5\n失敗 0 では割れません\nOk(3)\nOk(255)\n");
}

#[test]
fn question_mark_returns_the_error() {
    let src = "
fn Result<i64, str> div(i64 a, i64 b) {
    if b == 0 { Err(\"0 では割れません\") } else { Ok(a / b) }
}

fn Result<i64, str> calc(i64 a, i64 b, i64 c) {
    let x = div(a, b)?;
    let y = div(x, c)?;
    print(\"ここまで来た\", y);
    Ok(x + y)
}

fn main() {
    print(calc(100, 5, 2));
    print(calc(100, 0, 2));
    print(calc(100, 5, 0));
}";
    assert_eq!(ok(src), "ここまで来た 10\nOk(30)\nErr(0 では割れません)\nErr(0 では割れません)\n");
}

#[test]
fn question_mark_in_the_middle_of_an_expression() {
    let src = "
fn Result<i64, str> num(i64 n) {
    if n < 0 { Err(\"負の数\") } else { Ok(n) }
}

fn i64 add(i64 a, i64 b) {
    a + b
}

fn Result<i64, str> f(i64 a, i64 b) {
    Ok(add(num(a)?, num(b)?) * 2)
}

fn main() {
    print(f(1, 2), f(-1, 2), f(1, -2));
}";
    assert_eq!(ok(src), "Ok(6) Err(負の数) Err(負の数)\n");
}

#[test]
fn question_mark_frees_what_was_built_so_far() {
    // `?` で抜けるとき、変数・途中まで作った引数・構造体の own をすべて解放する（リークしない）
    let src = "
struct Two {
    own i64 a;
    own i64 b;
}

fn Result<i64, str> fail(bool b) {
    if b { Err(\"だめ\") } else { Ok(1) }
}

fn i64 sum(own i64 a, own i64 b) {
    *a + *b
}

fn Result<i64, str> run(bool b) {
    let keep = own 10;
    let total = if true {
        let inner = own 20;
        sum(own 1, own fail(b)?)
    } else {
        0
    };
    loop {
        let inner = own 30;
        if fail(b)? == 1 {
            break;
        }
    }
    let t = Two { a: own 3, b: own fail(b)? };
    print(*t.a + *t.b);
    print(own 5, fail(b)?);
    Ok(*keep + total)
}

fn main() {
    print(run(false));
    print(run(true));
}";
    assert_eq!(ok(src), "4\n5 1\nOk(12)\nErr(だめ)\n");
}

#[test]
fn question_mark_on_option() {
    let src = "
fn Option<i64> half(i64 n) {
    if n % 2 == 0 { Some(n / 2) } else { None }
}

fn Option<i64> quarter(i64 n) {
    let h = half(n)?;
    Some(half(h)?)
}

fn main() {
    print(quarter(8), quarter(6), quarter(3));
}";
    assert_eq!(ok(src), "Some(2) None None\n");
}

#[test]
fn result_holding_own_moves() {
    let src = "
fn Result<own i64, str> make(bool b) {
    if b { Ok(own 42) } else { Err(\"なし\") }
}

fn Result<i64, str> use_it(bool b) {
    let p = make(b)?;
    Ok(*p + 1)
}

fn main() {
    print(use_it(true), use_it(false));
    let r = make(true);
    let s = r;
    print(r);
}";
    let d = err(src);
    assert!(d.title.contains("ムーブ"), "{}", d.title);
    assert_eq!(ok("
fn Result<own i64, str> make(bool b) {
    if b { Ok(own 42) } else { Err(\"なし\") }
}

fn Result<i64, str> use_it(bool b) {
    let p = make(b)?;
    Ok(*p + 1)
}

fn main() {
    print(use_it(true), use_it(false));
}"), "Ok(43) Err(なし)\n");
}

#[test]
fn question_mark_errors() {
    let e = |src: &str| err(src);
    let d = e("
fn Result<i64, str> f() { Ok(1) }
fn main() {
    let x = f()?;
}");
    assert!(d.title.contains("ここでは `?` を使えません"), "{}", d.title);
    let d = e("
fn Result<i64, str> f() { Ok(1) }
fn Option<i64> g() {
    Some(f()?)
}
fn main() {}");
    assert!(d.title.contains("ここでは `?` を使えません"), "{}", d.title);
    let d = e("
fn Result<i64, str> f() { Ok(1) }
fn Result<i64, i64> g() {
    Ok(f()?)
}
fn main() {}");
    assert!(d.title.contains("エラーの型が合いません"), "{}", d.title);
    let d = e("
fn Result<i64, str> g() {
    let i64 x = 1;
    Ok(x?)
}
fn main() {}");
    assert!(d.title.contains("Result か Option に使います"), "{}", d.title);
    let d = e("
fn main() {
    let x = Ok(1);
}");
    assert!(d.title.contains("決まりません"), "{}", d.title);
    let d = e("
fn Result<i64, str> g() {
    Ok(1)
}
fn main() {
    match g() {
        case Ok(v) -> print(v);
    }
}");
    assert!(d.title.contains("Err の場合が書かれていません"), "{}", d.title);
}

#[test]
fn question_mark_changes_the_ok_type() {
    // `?` で返す Err は関数の戻り値の型になる（Result<i64, str> はコピーできるので 2 回読める）
    let src = "
fn Result<i64[], str> a() { Err(\"x\") }
fn Result<i64, str> b() { let v = a()?; Ok(v.len()) }
fn Option<str> c() { let i64[] v = None?; Some(\"no\") }
fn main() {
    let r = b();
    let s = r;
    print(r, s, c());
}";
    assert_eq!(ok(src), "Err(x) Err(x) None\n");
}

#[test]
fn ok_or_turns_an_option_into_a_result() {
    let src = "
fn Result<i64, str> parse(str s) {
    let n = ok_or(s.to_int(), \"数ではない: {s}\")?;
    Ok(n * 2)
}
fn main() {
    print(parse(\"21\"), parse(\"x\"));
    let Option<own i64> a = None;
    print(ok_or(a, own 5));
}";
    assert_eq!(ok(src), "Ok(42) Err(数ではない: x)\nErr(5)\n");
}
