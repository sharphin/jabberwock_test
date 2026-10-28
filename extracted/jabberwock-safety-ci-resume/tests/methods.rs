//! メソッドのテスト

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

const POINT: &str = "struct Point {
    i64 x;
    i64 y;

    fn i64 len2() {
        x * x + y * y
    }

    fn mut move_by(i64 dx, i64 dy) {
        x += dx;
        this.y = y + dy;
    }

    fn i64 plus(i64 d) {
        d + x + len2()
    }
}
";

#[test]
fn methods_read_and_write_fields() {
    let src = format!("{}
fn main() {{
    let mut p = Point {{ x: 3, y: 4 }};
    print(p.len2());
    p.move_by(1, 1);
    print(p.x, p.y, p.len2());
    print(Point {{ x: 2, y: 0 }}.len2());
}}", POINT);
    assert_eq!(ok(&src), "25\n4 5 41\n4\n");
}

#[test]
fn fields_and_methods_are_used_without_this() {
    let src = format!("{}
fn main() {{
    let p = Point {{ x: 3, y: 4 }};
    print(p.plus(100));
}}", POINT);
    // d は引数の 100、x は this.x の 3、len2() は this.len2()
    assert_eq!(ok(&src), "128\n");
}

#[test]
fn methods_through_own_and_ref() {
    let src = format!("{}
fn bump(ref mut Point r) {{
    r.move_by(1, 1);
}}

fn main() {{
    let mut o = own Point {{ x: 1, y: 2 }};
    o.move_by(10, 10);
    print(o.len2());
    bump(ref mut o);
    print(o.x, o.y);
    let r = ref o;
    print(r.len2());
}}", POINT);
    assert_eq!(ok(&src), "265\n12 13\n313\n");
}

#[test]
fn nested_struct_fields_are_written_back() {
    let src = format!("{}
struct Box2 {{
    Point a;
    own Point b;
}}

fn main() {{
    let mut bx = Box2 {{ a: Point {{ x: 0, y: 0 }}, b: own Point {{ x: 5, y: 5 }} }};
    bx.a.move_by(1, 2);
    bx.b.move_by(1, 1);
    print(bx.a.x, bx.a.y, bx.b.x);
    let o = own Box2 {{ a: Point {{ x: 1, y: 1 }}, b: own Point {{ x: 0, y: 0 }} }};
    print(o.a.len2());
}}", POINT);
    assert_eq!(ok(&src), "1 2 6\n2\n");
}

#[test]
fn generic_methods() {
    let src = "
struct Stack2<T> {
    Option<T> top;
    i64 count;

    fn mut push(T v) {
        top = Some(v);
        count++;
    }

    fn bool empty() {
        count == 0
    }
}

fn main() {
    let Stack2<own i64> mut s = Stack2 { top: None, count: 0 };
    s.push(own 1);
    s.push(own 2);
    print(s.empty(), s.count);
    let mut t = Stack2 { top: Some(\"a\"), count: 1 };
    t.push(\"b\");
    print(t.top);
}";
    assert_eq!(ok(src), "false 2\nSome(b)\n");
}

#[test]
fn methods_with_own_fields_and_results() {
    let src = "
struct Account {
    str name;
    own i64 balance;

    fn mut Result<i64, str> withdraw(i64 n) {
        if n > *balance {
            return Err(\"残高が足りません\");
        }
        *balance = *balance - n;
        Ok(*balance)
    }
}

fn Result<i64, str> twice(ref mut Account a, i64 n) {
    a.withdraw(n)?;
    a.withdraw(n)
}

fn main() {
    let mut a = own Account { name: \"x\", balance: own 100 };
    print(a.withdraw(30));
    print(twice(ref mut a, 30));
    print(twice(ref mut a, 30));
}";
    assert_eq!(ok(src), "Ok(70)\nOk(10)\nErr(残高が足りません)\n");
}

#[test]
fn ref_to_this_does_not_outlive_the_receiver() {
    // 変数にある構造体の値に対するメソッドは、その変数を借りて this にする。
    // this を返せば、変数が生きているあいだは使える
    let src = "
struct P {
    i64 x;

    fn ref P me() {
        this
    }
}

fn main() {
    let p = P { x: 1 };
    let r = p.me();
    print(r.x);
}";
    assert_eq!(ok(src), "1\n");
    // 一時的な値は呼び出しのあいだだけヒープに置くので、呼び出しが終わればダングリング
    let src = "
struct P {
    i64 x;

    fn ref P me() {
        this
    }
}

fn main() {
    let r = P { x: 1 }.me();
    print(r.x);
}";
    let o = run_source(src, false);
    let d = o.error.expect("dangling");
    assert!(d.title.contains("dangling"), "{}", d.title);
    assert_eq!(o.live_objects, 0);
}

#[test]
fn method_errors() {
    let e = |body: &str| err(&format!("{}\nfn main() {{\n{}\n}}", POINT, body));
    let d = e("let p = Point { x: 1, y: 1 };\np.move_by(1, 1);");
    assert!(d.title.contains("mut ではない"), "{}", d.title);
    let d = e("let mut o = own Point { x: 1, y: 1 };\nlet r = ref o;\nr.move_by(1, 1);");
    assert!(d.title.contains("読むだけの ref"), "{}", d.title);
    let d = e("let p = Point { x: 1, y: 1 };\np.fly();");
    assert!(d.title.contains("メソッド `fly` はありません"), "{}", d.title);
    assert!(d.note.unwrap().contains("len2"));
    let d = e("let p = Point { x: 1, y: 1 };\np.len2(1);");
    assert!(d.title.contains("引数"), "{}", d.title);
    let d = e("let i64 n = 1;\nn.len2();");
    assert!(d.title.contains("`i64` にメソッド `len2` はありません"), "{}", d.title);
    let d = e("let b = true;\nb.len2();");
    assert!(d.title.contains("構造体と enum だけ"), "{}", d.title);
    let d = err("
struct P {
    i64 x;

    fn set() {
        x = 1;
    }
}
fn main() {}");
    assert!(d.title.contains("書き換えないメソッド"), "{}", d.title);
    assert!(d.note.unwrap().contains("fn mut"));
    let d = err("
struct P {
    i64 x;

    fn i64 x() {
        1
    }
}
fn main() {}");
    assert!(d.title.contains("フィールドと同じ名前"), "{}", d.title);
}

#[test]
fn this_checks_are_elided_when_the_receiver_is_an_own_variable() {
    let src = format!("{}
fn main() {{
    let mut o = own Point {{ x: 1, y: 2 }};
    o.move_by(1, 1);
    print(o.len2(), o.plus(1));
}}", POINT);
    let o = run_source(&src, false);
    assert!(o.error.is_none());
    assert_eq!(o.analysis.required, 0, "{:?}", o.analysis);
    assert!(o.analysis.elided > 0);
    // 構造体の値の receiver は、解析では分からないので検査が残る（安全側）
    let src = format!("{}
fn main() {{
    let p = Point {{ x: 1, y: 2 }};
    print(p.len2());
}}", POINT);
    let o = run_source(&src, false);
    assert!(o.error.is_none());
    assert!(o.analysis.required > 0);
}

#[test]
fn static_methods() {
    let src = "
struct Point {
    i64 x;
    i64 y;

    static fn Point origin() {
        Point { x: 0, y: 0 }
    }

    static fn Point at(i64 x, i64 y) {
        Point { x: x, y: y }
    }

    fn Point moved(i64 d) {
        at(x + d, y + d)
    }
}

struct Pair<T> {
    T a;
    T b;

    static fn Pair<T> same(T v) {
        Pair { a: v, b: v }
    }
}

fn main() {
    let p = Point.origin();
    let q = Point.at(1, 2).moved(10);
    print(p.x, q.x, q.y);
    let s = Pair.same(own 3);
    print(*s.a + 0);
}";
    // own を 2 か所に入れようとしているので、2 つ目でムーブ済みエラーになる
    let o = run_source(src, false);
    assert!(o.error.unwrap().title.contains("ムーブ"));
    let ok_src = src.replace("Pair.same(own 3)", "Pair.same(\"w\")").replace("*s.a + 0", "s.a, s.b");
    assert_eq!(ok(&ok_src), "0 11 12\nw w\n");
}

#[test]
fn static_method_errors() {
    let src = |body: &str| format!("{}\nstruct S {{\n    i64 v;\n\n    static fn S make() {{\n        S {{ v: 1 }}\n    }}\n}}\nfn main() {{\n{}\n}}", POINT, body);
    let d = err(&src("let p = Point { x: 1, y: 1 };\nPoint.len2(ref p);"));
    assert!(d.title.contains("this のあるメソッド"), "{}", d.title);
    let d = err(&src("let s = S.make();\ns.make();"));
    assert!(d.title.contains("静的メソッドです"), "{}", d.title);
    let d = err(&src("S.nothing();"));
    assert!(d.title.contains("未定義の関数"), "{}", d.title);
    let d = err(&src("let v = S.v;"));
    assert!(d.title.contains("静的メソッドの呼び出し"), "{}", d.title);
    // 静的メソッドの中には this がない
    let d = err("
struct S {
    i64 v;

    static fn i64 get() {
        v
    }
}
fn main() {}");
    assert!(d.title.contains("未定義の変数") || d.title.contains("v"), "{}", d.title);
}
