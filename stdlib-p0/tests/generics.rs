//! ジェネリクス（型の引数）のテスト

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
fn generic_functions() {
    let src = "
fn T pick<T>(bool first, T a, T b) {
    if first { a } else { b }
}

fn show<T>(T x) {
    print(\"値:\", x);
}

fn main() {
    print(pick(true, 1, 2));
    print(pick(false, \"a\", \"b\"));
    show(3);
    show(true);
    let u8 small = 200;
    let big = pick(false, small, 100);
    print(big + 100);
}";
    assert_eq!(ok(src), "1\nb\n値: 3\n値: true\n200\n");
}

#[test]
fn integer_arguments_widen_to_a_common_type() {
    let src = "
fn T pick<T>(bool first, T a, T b) {
    if first { a } else { b }
}

fn main() {
    let u8 a = 250;
    let i64 b = 10;
    let c = pick(true, a, b);
    print(c + 10);
    let u8 d = pick(true, a, 3);
    print(d + 10);
}";
    // c は i64（u8 と i64 をそろえた型）、d は u8 なので一周する
    assert_eq!(ok(src), "260\n4\n");
}

#[test]
fn generic_structs() {
    let src = "
struct Pair<T> {
    T a;
    T b;
}

fn Pair<T> swap<T>(Pair<T> p) {
    Pair { a: p.b, b: p.a }
}

fn main() {
    let p = Pair { a: 1, b: 2 };
    let q = swap(p);
    print(q.a, q.b);
    let Pair<str> s = Pair { a: \"x\", b: \"y\" };
    print(swap(s).a);
    let Pair<u8> u = Pair { a: 255, b: 0 };
    print(u.a + 1);
}";
    assert_eq!(ok(src), "2 1\ny\n0\n");
}

#[test]
fn generic_structs_holding_own_are_freed() {
    let src = "
struct Box2<T> {
    T first;
    T second;
}

fn main() {
    let b = Box2 { first: own 1, second: own 2 };
    print(*b.first + *b.second);
    let c = b;
    let Box2<own i64> d = Box2 { first: own 3, second: own 4 };
}";
    assert_eq!(ok(src), "3\n");
}

#[test]
fn generic_enums() {
    let src = "
enum Either<L, R> {
    Left(L value);
    Right(R value);
}

fn describe(Either<i64, str> e) {
    match e {
        case Left(n) -> print(\"数\", n);
        case Right(s) -> print(\"文字\", s);
    }
}

fn Either<i64, str> parse(bool b) {
    if b { Either.Left(42) } else { Either.Right(\"no\") }
}

fn main() {
    describe(Either.Left(1));
    describe(Either.Right(\"a\"));
    describe(parse(true));
    describe(parse(false));
}";
    assert_eq!(ok(src), "数 1\n文字 a\n数 42\n文字 no\n");
}

#[test]
fn option_is_an_ordinary_generic_enum() {
    let src = "
fn T or_else<T>(Option<T> o, T other) {
    match o {
        case Some(v) -> v;
        case None -> other;
    }
}

fn Option<T> wrap<T>(T x) {
    Some(x)
}

fn main() {
    print(or_else(Some(1), 5));
    print(or_else(None, 5));
    let Option<str> s = Option.Some(\"long\");
    print(s);
    let o = wrap(own 7);
    match o {
        case Some(p) -> print(*p);
        case None -> print(\"none\");
    }
}";
    assert_eq!(ok(src), "1\n5\nSome(long)\n7\n");
}

#[test]
fn generic_values_move_when_they_hold_own() {
    // T が own なら、渡した変数はもう使えない
    let src = "
fn T id<T>(T x) {
    x
}

fn main() {
    let a = own 1;
    let b = id(a);
    print(*a);
}";
    let d = err(src);
    assert!(d.title.contains("ムーブ"), "{}", d.title);
    // T が i64 ならコピー
    assert_eq!(ok("
fn T id<T>(T x) {
    x
}

fn main() {
    let a = 1;
    let b = id(a);
    print(a, b);
}"), "1 1\n");
}

#[test]
fn generic_errors() {
    let d = err("
fn T pick<T>(T a, T b) {
    a
}

fn main() {
    print(pick(1, \"a\"));
}");
    assert!(d.title.contains("型が合いません"), "{}", d.title);
    // 型の引数を使ってできないこと
    let d = err("
fn T add<T>(T a, T b) {
    a + b
}

fn main() {
}");
    assert!(d.title.contains("型が合わない演算"), "{}", d.title);
    let d = err("
struct Pair<T> {
    T a;
    T b;
}

fn main() {
    let Pair p = Pair { a: 1, b: 2 };
}");
    assert!(d.title.contains("型の引数が必要"), "{}", d.title);
    let d = err("
struct Pair<T> {
    T a;
    T b;
}

fn main() {
    let Pair<i64, i64> p = Pair { a: 1, b: 2 };
}");
    assert!(d.title.contains("型の引数の数が違います"), "{}", d.title);
    let d = err("
fn Option<T> nothing<T>() {
    None
}

fn main() {
    let x = nothing();
}");
    assert!(d.title.contains("決まりません"), "{}", d.title);
}
