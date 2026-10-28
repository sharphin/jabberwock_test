//! 文字列の埋め込み `"x = {x}"` のテスト

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
fn embeds_expressions() {
    let src = "
struct Point {
    i64 x;
    i64 y;

    fn i64 len2() { x * x + y * y }
    fn str show() { \"({x}, {y})\" }
}

fn main() {
    let x = 3;
    let name = \"jabberwock\";
    print(\"x = {x}, x+1 = {x + 1}, {name}!\");
    let p = own Point { x: 1, y: 2 };
    print(\"p = {p.show()}, len2 = {p.len2()}, ok = {p.x < p.y}\");
    let o = Some(5);
    print(\"o = {o}\");
    let u8 big = 250;
    print(\"{big + 10}\");
    let s = \"a{x}b\" + \"c\";
    print(s);
    print(\"{{x}} はそのまま {x}\");
}";
    assert_eq!(
        ok(src),
        "x = 3, x+1 = 4, jabberwock!\np = (1, 2), len2 = 5, ok = true\no = Some(5)\n4\na3bc\n{x} はそのまま 3\n"
    );
}

#[test]
fn embedding_does_not_move() {
    let src = "
fn main() {
    let a = own 7;
    print(\"a = {*a}, {a}\");
    print(*a);
    let t = \"{own 1}\";
}";
    assert_eq!(ok(src), "a = 7, 7\n7\n");
}

#[test]
fn embedding_errors() {
    let e = |body: &str| err(&format!("fn main() {{\n{}\n}}", body));
    assert!(e("print(\"{x}\");").title.contains("未定義の変数"));
    assert!(e("print(\"{}\");").title.contains("空"));
    assert!(e("print(\"{1 + }\");").title.contains("構文エラー"));
    assert!(e("print(\"a { b\");").title.contains("閉じていません"));
    assert!(e("print(\"a } b\");").title.contains("対応する"));
    let d = e("let s = \"x\";\nprint(\"{s + \"y\"}\");");
    assert!(d.note.unwrap().contains("文字列を書けない"));
    // エラーの位置は元のファイルの中の位置
    let d = e("let a = 1;\nprint(\"value: {a + nope}\");");
    assert!(d.render("t.jw", "fn main() {\nlet a = 1;\nprint(\"value: {a + nope}\");\n}").contains("^^^^"));
}

#[test]
fn own_and_ref_show_what_they_point_to() {
    let src = "
struct Box2 {
    own i64 a;
    i64 b;
}

fn main() {
    let o = own 5;
    let i64[] v = [1, 2];
    let r = ref v;
    let x = Box2 { a: own 7, b: 8 };
    print(o, r, x);
    print(\"{o} {r} {x}\");
}";
    assert_eq!(ok(src), "5 [1, 2] Box2 { a: 7, b: 8 }\n5 [1, 2] Box2 { a: 7, b: 8 }\n");
}

#[test]
fn printing_a_dangling_ref_is_an_error() {
    let o = run_source("
fn main() {
    let z = 0;
    let mut r = ref z;
    if true {
        let x = 1;
        r = ref x;
    }
    print(\"r = {r}\");
}", false);
    assert!(o.error.unwrap().title.contains("dangling"));
    assert_eq!(o.output, "");
}

#[test]
fn multi_line_strings() {
    // `"""..."""` は途中で改行できる。開きの直後の改行は含めない。埋め込みも使える
    let src = "fn main() {\n    let n = 3;\n    let s = \"\"\"\nこんにちは \"世界\"\n  n = {n}\n{{}}\"\"\";\n    print(s);\n    print(s.lines().len());\n}\n";
    assert_eq!(ok(src), "こんにちは \"世界\"\n  n = 3\n{}\n3\n");
    // 後ろの行の位置はずれない
    let d = jabberwock::run_source("fn main() {\n    let s = \"\"\"\nx\"\"\";\n    let i64 a = \"no\";\n}\n", false).error.expect("error");
    assert_eq!(d.labels[0].span.line, 4);
}
