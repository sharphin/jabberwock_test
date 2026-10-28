//! interface（Go 風）と型の引数の条件のテスト

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

const SHAPES: &str = "interface Shape {
    fn i64 area();
    fn str name();
}

struct Square {
    i64 side;

    fn i64 area() { side * side }
    fn str name() { \"正方形\" }
}

struct Rect {
    i64 w;
    i64 h;

    fn i64 area() { w * h }
    fn str name() { \"長方形\" }
    fn mut grow(i64 n) { w += n; h += n; }
}

fn describe<T: Shape>(ref T s) {
    print(s.name(), s.area());
}

fn i64 twice<T: Shape>(T s) {
    s.area() * 2
}
";

#[test]
fn structs_satisfy_interfaces_without_declaring() {
    let src = format!("{}
fn main() {{
    let sq = own Square {{ side: 3 }};
    let r = own Rect {{ w: 2, h: 5 }};
    describe(ref sq);
    describe(ref r);
    print(twice(Square {{ side: 2 }}));
}}", SHAPES);
    assert_eq!(ok(&src), "正方形 9\n長方形 10\n8\n");
}

#[test]
fn mutable_interface_methods() {
    let src = "
interface Growable {
    fn mut grow(i64 n);
    fn i64 size();
}

struct Ball {
    i64 r;

    fn mut grow(i64 n) { r += n; }
    fn i64 size() { r }
}

fn grow_twice<T: Growable>(ref mut T g) {
    g.grow(1);
    g.grow(1);
}

fn main() {
    let mut b = own Ball { r: 1 };
    grow_twice(ref mut b);
    print(b.size());
}";
    assert_eq!(ok(src), "3\n");
}

#[test]
fn ordered_compares_numbers_and_strings() {
    let src = "
fn T max<T: ordered>(T a, T b) {
    if a > b { a } else { b }
}

fn T max3<T: ordered>(T a, T b, T c) {
    max(max(a, b), c)
}

fn main() {
    print(max(3, 7), max(\"apple\", \"banana\"));
    print(max3(5, 9, 2));
    let u8 small = 200;
    print(max(small, 100));
    print(\"a\" < \"b\");
}";
    assert_eq!(ok(src), "7 banana\n9\n200\ntrue\n");
}

#[test]
fn bound_errors() {
    let d = err(&format!("{}
fn main() {{
    describe(ref (own 5));
}}", SHAPES));
    assert!(d.title.contains("借用できない") || d.title.contains("条件"), "{}", d.title);
    let d = err(&format!("{}
struct Circle {{
    i64 r;
    fn i64 area() {{ 3 * r * r }}
}}
fn main() {{
    let c = own Circle {{ r: 1 }};
    describe(ref c);
}}", SHAPES));
    assert!(d.title.contains("条件 `Shape` を満たしません"), "{}", d.title);
    assert!(d.note.unwrap().contains("name"));
    let d = err(&format!("{}
struct Odd {{
    i64 r;
    fn str area() {{ \"x\" }}
    fn str name() {{ \"odd\" }}
}}
fn main() {{
    print(twice(Odd {{ r: 1 }}));
}}", SHAPES));
    assert!(d.title.contains("満たしません"), "{}", d.title);
    assert!(d.note.unwrap().contains("形と違います"));
    let d = err("
fn T max<T>(T a, T b) {
    if a > b { a } else { b }
}
fn main() {}");
    assert!(d.title.contains("型が合わない演算"), "{}", d.title);
    assert!(d.note.unwrap().contains("ordered"));
    let d = err("
fn T max<T: ordered>(T a, T b) {
    if a > b { a } else { b }
}
fn main() {
    print(max(true, false));
}");
    assert!(d.title.contains("条件 `ordered` を満たしません"), "{}", d.title);
    let d = err("
fn f<T>(T x) {
    x.area();
}
fn main() {}");
    assert!(d.title.contains("あるか分かりません"), "{}", d.title);
    let d = err("
fn f<T: Nothing>(T x) {
}
fn main() {}");
    assert!(d.title.contains("定義されていません"), "{}", d.title);
    // 型の引数のまま渡すときは、同じ条件が要る
    let d = err("
fn T max<T: ordered>(T a, T b) {
    if a > b { a } else { b }
}
fn T pass<T>(T a) {
    max(a, a)
}
fn main() {}");
    assert!(d.title.contains("満たしません"), "{}", d.title);
    let d = err(&format!("{}
fn main() {{
    let r = own Rect {{ w: 1, h: 1 }};
    describe(ref r);
}}
interface Bad {{
    fn i64 area();
}}
fn h<T: Bad>(ref T x) {{
    x.grow(1);
}}", SHAPES));
    assert!(d.title.contains("あるか分かりません"), "{}", d.title);
}

// ---------------------------------------------------------------- interface を型として使う

#[test]
fn interface_values_hold_any_matching_struct() {
    let src = format!("{}
struct Scene {{
    own Shape main;
    Option<own Shape> extra;
}}

fn i64 area_of(ref Shape s) {{
    s.area()
}}

fn own Shape pick(bool big) {{
    if big {{ own Rect {{ w: 10, h: 10 }} }} else {{ own Square {{ side: 1 }} }}
}}

fn main() {{
    let sq = own Square {{ side: 3 }};
    let r = own Rect {{ w: 2, h: 5 }};
    print(area_of(ref sq), area_of(ref r));
    let ref Shape s = if true {{ ref sq }} else {{ ref r }};
    print(s.name());
    let a = pick(true);
    let b = pick(false);
    print(a.name(), a.area(), b.name(), b.area());
    let scene = own Scene {{ main: own Square {{ side: 4 }}, extra: Some(own Rect {{ w: 1, h: 2 }}) }};
    print(scene.main.area());
    let Option<own Shape> extra = Some(own Rect {{ w: 1, h: 2 }});
    match extra {{
        case Some(e) -> print(e.name());
        case None -> print(\"なし\");
    }}
}}", SHAPES);
    assert_eq!(ok(&src), "9 10\n正方形\n長方形 100 正方形 1\n16\n長方形\n");
}

#[test]
fn mutable_methods_through_interface_refs() {
    let src = "
interface Growable {
    fn mut grow(i64 n);
    fn i64 size();
}

struct Ball {
    i64 r;

    fn mut grow(i64 n) { r += n; }
    fn i64 size() { r }
}

fn grow_all(ref mut Growable g) {
    g.grow(2);
}

fn main() {
    let mut b = own Ball { r: 1 };
    grow_all(ref mut b);
    print(b.size());
}";
    assert_eq!(ok(src), "3\n");
}

#[test]
fn interface_refs_dangle_safely() {
    let src = format!("{}
fn main() {{
    let first = own Square {{ side: 1 }};
    let ref Shape mut s = ref first;
    let mut a = own Square {{ side: 2 }};
    s = ref a;
    a = own Square {{ side: 3 }};
    print(s.area());
}}", SHAPES);
    let o = run_source(&src, false);
    let d = o.error.expect("dangling");
    assert!(d.title.contains("dangling"), "{}", d.title);
}

#[test]
fn interface_type_errors() {
    let d = err(&format!("{}
fn f(Shape s) {{}}
fn main() {{}}", SHAPES));
    assert!(d.title.contains("own か ref を通して"), "{}", d.title);
    let d = err(&format!("{}
struct Circle {{
    i64 r;
    fn i64 area() {{ r }}
}}
fn main() {{
    let c = own Circle {{ r: 1 }};
    let ref Shape s = ref c;
}}", SHAPES));
    assert!(d.title.contains("型が合いません"), "{}", d.title);
    let d = err(&format!("{}
fn main() {{
    let r = own Rect {{ w: 1, h: 1 }};
    let ref Shape s = ref r;
    s.grow(1);
}}", SHAPES));
    assert!(d.title.contains("あるか分かりません"), "{}", d.title);
    let d = err(&format!("{}
fn main() {{
    let r = own Rect {{ w: 1, h: 1 }};
    let ref Shape s = ref r;
    print(s.w);
}}", SHAPES));
    assert!(d.title.contains("フィールド"), "{}", d.title);
}

#[test]
fn for_over_a_list_of_owned_interfaces() {
    let src = "
interface Named { fn str name(); }
struct A { fn str name() { \"a\" } }
struct B { i64 n; fn str name() { \"b{n}\" } }
fn main() {
    let (own Named)[] mut xs = [];
    xs.push(own A {});
    xs.push(own B { n: 2 });
    for x in xs { print(x.name()); }
}";
    assert_eq!(ok(src), "a\nb2\n");
}
