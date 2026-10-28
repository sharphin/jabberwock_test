//! 構造体のテスト

use std::fs;

use jabberwock::{run_source, run_source_with, Outcome};

fn example(name: &str) -> String {
    let path = format!("{}/examples/{}.jw", env!("CARGO_MANIFEST_DIR"), name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {}", path, e))
}

fn ok(src: &str) -> Outcome {
    let o = run_source(src, false);
    if let Some(d) = &o.error {
        panic!("unexpected error:\n{}", d.render("test.jw", src));
    }
    assert_eq!(o.live_objects, 0, "heap objects leaked");
    let off = run_source_with(src, false, false);
    assert_eq!(o.output, off.output, "elision changed the output");
    o
}

fn err(src: &str) -> jabberwock::diag::Diagnostic {
    let d = run_source(src, false).error.expect("expected an error");
    let off = run_source_with(src, false, false).error.expect("expected an error without elision");
    assert_eq!(d.title, off.title, "elision changed the error");
    d
}

const POINT: &str = "struct Point { i64 x; i64 y; }\n";

// ---------------------------------------------------------------- 正常系

#[test]
fn structs_example() {
    let o = ok(&example("structs"));
    assert_eq!(
        o.output,
        "勇者 は Point { x: 3, y: 4 } にいる。スコア 10\nPoint { x: 9, y: 9 }\n"
    );
    assert_eq!(o.ref_checks, 0);
}

#[test]
fn literal_fields_can_be_written_in_any_order() {
    let src = format!("{POINT};
fn main() {{
    let p = Point {{ y: 2, x: 1 }};
    print(p, p.x + p.y)
}}");
    assert_eq!(ok(&src).output, "Point { x: 1, y: 2 } 3\n");
}

#[test]
fn field_assignment_frees_the_old_own() {
    let src = "
struct Box2 { own i64 inner; }

fn main() {
    let mut b = Box2 { inner: own 1 };
    b.inner = own 2;
    b.inner = own 3;
    print(*b.inner)
}";
    assert_eq!(ok(src).output, "3\n");
}

#[test]
fn own_fields_are_freed_with_their_struct() {
    let src = "
struct Pair { own i64 a; own own i64 b; }

fn own Pair make() {
    return own Pair { a: own 1, b: own own 2 };
}

fn main() {
    let p = make();
    print(*p.a, **p.b);
    let q = Pair { a: own 3, b: own own 4 };
    drop(q);
    print(Pair { a: own 5, b: own own 6 })
}";
    // print の一時的な構造体の中の own も解放される（リークしない）
    let o = ok(src);
    assert!(o.output.starts_with("1 2\n"));
}

#[test]
fn field_of_a_temporary_does_not_leak() {
    let src = format!("{POINT};
fn own Point make() {{
    return own Point {{ x: 7, y: 8 }};
}}

fn main() {{
    print(make().x + Point {{ x: 1, y: 1 }}.y)
}}");
    assert_eq!(ok(&src).output, "8\n");
}

#[test]
fn struct_field_in_if_and_while_conditions() {
    let src = format!("{POINT};
fn main() {{
    let mut p = own Point {{ x: 0, y: 3 }};
    while p.x < p.y {{
        p.x = p.x + 1;
    }}
    if p.x == 3 {{
        print(\"ok\")
    }}
    if (Point {{ x: 1, y: 2 }}).x == 1 {{
        print(\"paren\")
    }}
}}");
    assert_eq!(ok(&src).output, "ok\nparen\n");
}

#[test]
fn ref_to_a_struct_lets_a_function_update_it() {
    let src = format!("{POINT};
fn shift(ref mut Point p) {{
    p.x = p.x + 10;
}}

fn main() {{
    let mut pt = own Point {{ x: 1, y: 2 }};
    shift(ref mut pt);
    shift(ref mut pt);
    print(pt.x)
}}");
    let o = ok(&src);
    assert_eq!(o.output, "21\n");
    assert_eq!(o.ref_checks, 0, "pinned ref param should be elided");
}

// ---------------------------------------------------------------- 安全性

#[test]
fn struct_value_moves_like_own() {
    let d = err(&format!("{POINT};
fn main() {{
    let p = Point {{ x: 1, y: 2 }};
    let q = p;
    print(p.x)
}}"));
    assert!(d.title.contains("use after move"), "{}", d.title);
}

#[test]
fn ref_into_a_field_dangles_after_overwrite() {
    let src = "
struct Box2 { own i64 inner; }

fn main() {
    let mut b = own Box2 { inner: own 1 };
    let r = ref b.inner;
    b.inner = own 2;
    print(*r)
}";
    let d = err(src);
    assert!(d.title.contains("dangling"), "{}", d.title);
    let out = d.render("t.jw", src);
    let borrowed = out.find("を借りた").unwrap();
    let freed = out.find("上書きされて").unwrap();
    let used = out.find("ここで使った").unwrap();
    assert!(borrowed < freed && freed < used, "{}", out);
    assert!(out.contains("`b.inner` を借りた"), "{}", out);
}

#[test]
fn field_through_a_dangling_ref_is_detected() {
    let d = err(&format!("{POINT};
fn ref Point leak() {{
    let p = own Point {{ x: 1, y: 2 }};
    return ref p;
}}

fn main() {{
    let r = leak();
    print(r.x)
}}"));
    assert!(d.title.contains("dangling"), "{}", d.title);
}

#[test]
fn cannot_move_an_own_out_of_a_field() {
    let d = err("
struct Box2 { own i64 inner; }

fn main() {
    let b = Box2 { inner: own 1 };
    let o = b.inner;
}");
    assert!(d.title.contains("取り出せません"), "{}", d.title);
}

#[test]
fn cannot_move_a_struct_out_through_a_pointer() {
    let d = err(&format!("{POINT};
fn main() {{
    let p = own Point {{ x: 1, y: 2 }};
    let q = *p;
}}"));
    assert!(d.title.contains("取り出せません"), "{}", d.title);
}

// ---------------------------------------------------------------- エラーの親切さ

#[test]
fn literal_errors() {
    let missing = err(&format!("{POINT}fn main() {{\n let p = Point {{ x: 1 }};\n}};"));
    assert!(missing.title.contains("足りません"));
    assert!(missing.labels[0].text.contains("y"));

    let unknown = err(&format!("{POINT}fn main() {{\n let p = Point {{ x: 1, y: 2, z: 3 }};\n}};"));
    assert!(unknown.title.contains("`z`"));
    assert!(unknown.note.unwrap().contains("x, y"));

    let twice = err(&format!("{POINT}fn main() {{\n let p = Point {{ x: 1, x: 2 }};\n}};"));
    assert!(twice.title.contains("二回"));

    let undefined = err("fn main() {\n let p = Nope { x: 1 };\n}");
    assert!(undefined.title.contains("未定義の構造体"));
}

#[test]
fn definition_errors() {
    assert!(err("struct A { i64 x; bool x; }\nfn main() {\n}").title.contains("二重"));
    assert!(err("struct A { i64 x; }\nstruct A { i64 y; }\nfn main() {\n}").title.contains("二重"));
}

#[test]
fn field_access_on_a_non_struct() {
    let d = err("fn main() {\n let n = 5;\n print(n.x)\n}");
    assert!(d.title.contains("フィールドを持たない"));
    let d = err("fn main() {\n let n = own 5;\n print(n.x)\n}");
    assert!(d.title.contains("フィールドを持たない"));
}

#[test]
fn temporary_own_cannot_be_used_as_a_place() {
    let d = err(&format!("{POINT};
fn own Point make() {{
    return own Point {{ x: 1, y: 2 }};
}}

fn main() {{
    print((*make()).x)
}}"));
    assert!(d.title.contains("一時的な own"), "{}", d.title);
}
