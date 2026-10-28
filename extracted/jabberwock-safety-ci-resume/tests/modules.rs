//! モジュール（`use geom;` と `geom.area(p)`、`pub`）のテスト

use jabberwock::run_with_modules;

const GEOM: &str = "
use util;

pub struct Point {
    i64 x;
    i64 y;

    fn i64 len2() { util.sq(x) + util.sq(y) }
    static fn Point at(i64 x, i64 y) { Point { x: x, y: y } }
}

pub enum Shape {
    Dot(Point p);
    Box(i64 w, i64 h);

    fn i64 corners() {
        match this {
            case Dot(_) -> 1;
            case Box(_, _) -> helper(2, 2);
        }
    }
    static fn Shape square(i64 n) { Shape.Box(n, n) }
    // モジュールの中で、enum の静的メソッドを型名なしで・型名つきで呼ぶ
    static fn Shape unit() { square(1) }
    static fn Shape unit2() { Shape.unit() }
}

pub interface Sized {
    fn i64 size();
}

pub fn i64 area(ref Shape s) {
    match s {
        case Dot(_) -> 0;
        case Box(w, h) -> helper(w, h);
    }
}

fn i64 helper(i64 a, i64 b) { a * b }

pub fn i64 apply(fn i64(i64) f, i64 x) { f(x) }

pub fn i64 twice(i64 x) { helper(x, 2) }
";

const UTIL: &str = "
pub fn i64 sq(i64 x) { x * x }
";

fn ok(src: &str) -> String {
    let o = run_with_modules(src, &[("geom", GEOM), ("util", UTIL)]);
    if let Some(d) = &o.error {
        panic!("unexpected error:\n{}", jabberwock::render(d, "main.jw", &o.files));
    }
    assert_eq!(o.live_objects, 0, "heap objects leaked");
    o.output
}

fn err(src: &str) -> (jabberwock::diag::Diagnostic, String) {
    let o = run_with_modules(src, &[("geom", GEOM), ("util", UTIL)]);
    let d = o.error.expect("expected an error");
    let shown = jabberwock::render(&d, "main.jw", &o.files);
    (d, shown)
}

#[test]
fn uses_names_from_a_module() {
    let src = "
use geom;

struct Mine {
    i64 w;

    fn i64 size() { w }
}

fn i64 need<T: geom.Sized>(ref T t) { t.size() }

fn main() {
    let p = geom.Point.at(3, 4);
    let q = geom.Point { x: 1, y: 1 };
    print(p.len2(), q.len2());
    let s = geom.Shape.Box(2, 5);
    print(geom.area(s));
    let geom.Point[] ps = [p, q];
    print(ps.len());
    print(geom.apply(geom.twice, 7), geom.apply((x) -> x + 1, 1));
    let mine = Mine { w: 9 };
    print(need(ref mine));
    print(\"{geom.twice(3)}\");
    let sq = geom.Shape.square(3);
    print(geom.area(sq), sq.corners(), geom.Shape.Dot(geom.Point.at(0, 0)).corners());
    let u = geom.Shape.unit2();
    print(geom.area(u));
}";
    assert_eq!(ok(src), "25 2\n10\n2\n14 2\n9\n6\n9 4 1\n1\n");
}

/// `x.geom` のように、フィールドの名前がモジュールと同じでもよい
#[test]
fn a_field_can_share_a_module_name() {
    let src = "
use geom;

struct Box { geom.Point geom; }

fn main() {
    let b = Box { geom: geom.Point.at(3, 4) };
    print(b.geom.x, b.geom.len2());
}";
    assert_eq!(ok(src), "3 25\n");
}

#[test]
fn module_errors() {
    let (d, _) = err("use geom;\nfn main() {\n    print(geom.helper(1, 2));\n}");
    assert!(d.title.contains("pub ではありません"), "{}", d.title);
    let (d, _) = err("use geom;\nfn main() {\n    print(geom.nope(1));\n}");
    assert!(d.title.contains("`nope` はありません"), "{}", d.title);
    let (d, _) = err("use nothing;\nfn main() {\n}");
    assert!(d.title.contains("見つかりません"), "{}", d.title);
    let (d, _) = err("use geom;\nfn main() {\n    let geom = 1;\n}");
    assert!(d.title.contains("モジュール"), "{}", d.title);
    // 使っていないモジュールの名前は見えない
    let (d, _) = err("use geom;\nfn main() {\n    print(util.sq(2));\n}");
    assert!(d.title.contains("未定義"), "{}", d.title);
}

#[test]
fn errors_point_into_the_module_file() {
    let o = run_with_modules("use bad;\nfn main() {\n    bad.f();\n}", &[("bad", "pub fn f() {\n    let x = 1 + true;\n}\n")]);
    let d = o.error.expect("type error in the module");
    let shown = jabberwock::render(&d, "main.jw", &o.files);
    assert!(shown.contains("--> bad.jw:2:"), "{}", shown);
    assert!(shown.contains("let x = 1 + true;"), "{}", shown);
}

#[test]
fn cycles_are_errors() {
    let o = run_with_modules("use a;\nfn main() {\n}", &[("a", "use b;\n"), ("b", "use a;\n")]);
    assert!(o.error.unwrap().title.contains("輪"));
}

#[test]
fn modules_can_have_the_same_names() {
    let o = run_with_modules(
        "use a;\nuse b;\nfn main() {\n    print(a.f(), b.f());\n}",
        &[("a", "pub fn i64 f() { g() }\nfn i64 g() { 1 }\n"), ("b", "pub fn i64 f() { g() }\nfn i64 g() { 2 }\n")],
    );
    assert!(o.error.is_none(), "{:?}", o.error.map(|d| d.title));
    assert_eq!(o.output, "1 2\n");
}
