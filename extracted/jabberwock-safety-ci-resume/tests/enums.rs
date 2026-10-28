//! enum と Option のテスト

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

const SHAPE: &str = "enum Shape {
    Circle(i64 r);
    Rect(i64 w, i64 h);
    Empty;
}
";

// ---------------------------------------------------------------- enum

#[test]
fn enums_carry_data_and_match_takes_it_out() {
    let src = format!("{SHAPE}
fn i64 area(Shape s) {{
    match s {{
        case Circle(r) -> 3 * r * r;
        case Rect(w, h) -> w * h;
        case Empty -> 0;
    }}
}}

fn main() {{
    print(area(Shape.Circle(2)), area(Shape.Rect(3, 4)), area(Shape.Empty));
    let s = Shape.Rect(5, 6);
    print(s, Shape.Empty);
    print(area(s), area(s));
}}");
    // 中身が数だけの enum はコピーされる（2 回渡せる）
    assert_eq!(ok(&src), "12 12 0\nRect(5, 6) Empty\n30 30\n");
}

#[test]
fn match_must_cover_every_variant() {
    let d = err(&format!("{SHAPE}fn main() {{\n    let s = Shape.Empty;\n    match s {{\n        case Circle(r) -> print(r);\n    }}\n}}"));
    assert!(d.title.contains("Rect・Empty の場合が書かれていません"), "{}", d.title);
    assert!(d.note.unwrap().contains("default"));
    // default でまとめてよい
    let src = format!("{SHAPE}fn main() {{\n    let s = Shape.Empty;\n    match s {{\n        case Circle(r) -> print(r);\n        default -> print(\"other\");\n    }}\n}}");
    assert_eq!(ok(&src), "other\n");
    // 種類をまとめた case
    let src = format!("{SHAPE}fn main() {{\n    let s = Shape.Rect(1, 2);\n    let n = match s {{\n        case Circle(_), Rect(_, _) -> 1;\n        case Empty -> 0;\n    }};\n    print(n);\n}}");
    assert_eq!(ok(&src), "1\n");
}

#[test]
fn enum_errors() {
    let e = |body: &str| err(&format!("{SHAPE}fn main() {{\n{}\n}}", body));
    assert!(e("let s = Shape.Square(1);").title.contains("種類 `Square` はありません"));
    assert!(e("let s = Shape.Circle(1, 2);").title.contains("引数の数"));
    assert!(e("let s = Shape.Circle(true);").title.contains("型が合いません"));
    let d = e("let s = Shape.Empty;\nmatch s {\n case Circle -> print(1);\n default -> print(2);\n}");
    assert!(d.title.contains("中身の数が違います"), "{}", d.title);
    assert!(e("let s = Shape.Empty;\nmatch s {\n case Box -> print(1);\n default -> print(2);\n}").title.contains("種類 `Box` はありません"));
    assert!(e("let s = Shape.Empty;\nmatch s {\n case 1 -> print(1);\n default -> print(2);\n}").title.contains("種類の名前を書きます"));
    assert!(e("match 1 {\n case Empty -> print(1);\n default -> print(2);\n}").title.contains("enum ではない値"));
    assert!(e("let s = Shape.Empty;\nmatch s {\n case Empty -> print(1);\n case Empty -> print(2);\n default -> print(3);\n}").title.contains("同じ値の case"));
    assert!(err("enum A { X; X; }\nfn main() {\n}").title.contains("二重"));
    assert!(err("enum A { X(i64); }\nfn main() {\n}").title.contains("型と名前"));
    assert!(err("enum A { X, Y }\nfn main() {\n}").title.contains("区切りは `;`"));
    assert!(err("enum A { X(A a); }\nfn main() {\n}").title.contains("自分自身をじかに含んで"));
    assert!(err("struct A { i64 x; }\nenum A { X; }\nfn main() {\n}").title.contains("二重"));
}

// ---------------------------------------------------------------- 所有権

#[test]
fn enums_holding_own_move_and_are_freed() {
    let src = "
enum Slot {
    Full(own i64 v);
    Vacant;
}

fn main() {
    let a = Slot.Full(own 5);
    let b = a;
    match b {
        case Full(v) -> print(*v);
        case Vacant -> print(\"vacant\");
    }
    let c = Slot.Full(own 6);
    match c {
        case Full(_) -> print(\"dropped\");
        case Vacant -> print(\"vacant\");
    }
    let d = Slot.Full(own 7);
    match d {
        case Vacant -> print(\"vacant\");
        default -> print(\"default\");
    }
    let e = Slot.Full(own 8);
}";
    assert_eq!(ok(src), "5\ndropped\ndefault\n");
    let d = err("enum Slot {\n    Full(own i64 v);\n    Vacant;\n}\nfn main() {\n    let a = Slot.Full(own 5);\n    let b = a;\n    print(a);\n}");
    assert!(d.title.contains("use after move"), "{}", d.title);
}

#[test]
fn match_ref_borrows_the_contents() {
    let src = "
enum Slot {
    Full(own i64 v, i64 tag);
    Vacant;
}

fn main() {
    let mut s = own Slot.Full(own 5, 1);
    match ref mut s {
        case Full(v, tag) -> {
            *v += 10;
            print(*v, tag);
        }
        case Vacant -> print(\"vacant\");
    }
    match ref s {
        case Full(v, _) -> print(*v);
        case Vacant -> print(\"vacant\");
    }
    match s {
        case Full(v, _) -> print(*v + 1);
        case Vacant -> print(\"vacant\");
    }
}";
    assert_eq!(ok(src), "15 1\n15\n16\n");
}

#[test]
fn a_ref_taken_in_match_ref_is_checked_after_the_owner_is_replaced() {
    let src = "
enum Slot {
    Full(own i64 v);
    Vacant;
}

fn main() {
    let mut s = own Slot.Full(own 5);
    let r = match ref s {
        case Full(v) -> Some(v);
        case Vacant -> None;
    };
    *s = Slot.Vacant;
    match r {
        case Some(v) -> print(*v);
        case None -> print(\"none\");
    }
}";
    let d = err(src);
    assert!(d.title.contains("dangling"), "{}", d.title);
    let out = d.render("t.jw", src);
    assert!(out.contains("`s の中身 v` を借りた"), "{}", out);
}

#[test]
fn match_ref_needs_own_contents() {
    let src = "struct P { i64 x; }\nenum E {\n    A(P p);\n}\nfn main() {\n    let e = own E.A(P { x: 1 });\n    match ref e {\n        case A(p) -> print(1);\n    }\n}";
    assert!(err(src).title.contains("借りて match するとき"));
}

// ---------------------------------------------------------------- Option

#[test]
fn option_replaces_null() {
    let src = "
fn Option<i64> find(i64 n) {
    for i in range(10) {
        if i * i == n { return Some(i); }
    }
    None
}

fn main() {
    for n in range(8, 10) {
        match find(n) {
            case Some(i) -> print(n, \"の平方根は\", i);
            case None -> print(n, \"は平方数ではない\");
        }
    }
    let Option<i64> x = None;
    let y = Some(3);
    print(x, y);
    let z = if true { Some(\"a\") } else { None };
    print(z);
}";
    assert_eq!(ok(src), "8 は平方数ではない\n9 の平方根は 3\nNone Some(3)\nSome(a)\n");
}

#[test]
fn option_errors() {
    let e = |body: &str| err(&format!("fn main() {{\n{}\n}}", body));
    let d = e("let x = None;");
    assert!(d.title.contains("None の型が決まりません"), "{}", d.title);
    assert!(d.note.unwrap().contains("Option<i64>"));
    assert!(e("let Option<i64> x = Some(\"a\");").title.contains("型が合いません"));
    assert!(e("let Option<i64> x = None;\nprint(x + 1);").title.contains("型が合わない演算"));
    assert!(e("let Option<i64> x = None;\nmatch x {\n case Some(v) -> print(v);\n}").title.contains("None の場合が書かれていません"));
    assert!(e("let Option x = None;").title.contains("型の引数が必要"));
    assert!(e("let Vec<i64> x = None;").title.contains("未定義の型"));
}

#[test]
fn option_of_own_moves_and_frees() {
    let src = "
fn Option<own i64> make(bool b) {
    if b { Some(own 42) } else { None }
}

fn main() {
    let a = make(true);
    let b = make(false);
    match a {
        case Some(p) -> print(*p);
        case None -> print(\"none\");
    }
    print(b);
    let c = make(true);
}";
    assert_eq!(ok(src), "42\nNone\n");
}

#[test]
fn integer_payloads_follow_the_declared_type() {
    let src = "
enum Px {
    Gray(u8 v);
}

fn main() {
    let p = Px.Gray(250);
    match p {
        case Gray(v) -> print(v + 10);
    }
    let Option<u8> o = Some(255);
    match o {
        case Some(v) -> print(v + 1);
        case None -> print(0);
    }
}";
    assert_eq!(ok(src), "4\n0\n");
}

#[test]
fn nested_patterns() {
    // `Some(Op(op))` のように形を重ねる。数・文字列・真偽値も書ける。借りた match では own の先にも重ねられる
    let src = "enum Tok { Num(i64 n); Op(str op); End; }
enum E { Lit(i64 n); Add(own E l, own E r); Neg(own E e); }
struct Box2 { own i64 v; }

fn str show(Option<Tok> t) {
    match t {
        case Some(Num(0)) -> \"zero\";
        case Some(Num(n)) -> \"num {n}\";
        case Some(Op(\"+\")) -> \"plus\";
        case Some(Op(o)) -> \"op {o}\";
        case Some(End), None -> \"end\";
    }
}

fn i64 simp(ref E e) {
    match e {
        case Add(Lit(0), r) -> simp(r);
        case Add(l, Lit(0)) -> simp(l);
        case Add(l, r) -> simp(l) + simp(r);
        case Neg(Neg(x)) -> simp(x);
        case Neg(x) -> -simp(x);
        case Lit(n) -> n;
    }
}

fn main() {
    print(show(Some(Tok.Num(0))), show(Some(Tok.Num(5))), show(Some(Tok.Op(\"+\"))), show(Some(Tok.Op(\"*\"))), show(None), show(Some(Tok.End)));
    let e = own E.Add(own E.Lit(0), own E.Neg(own E.Neg(own E.Add(own E.Lit(3), own E.Lit(0)))));
    print(simp(e));
    let Option<Option<own i64>> x = Some(Some(own 4));
    match x {
        case Some(Some(v)) -> print(*v);
        case Some(None) -> print(\"inner none\");
        case None -> print(\"none\");
    }
    let Result<Option<Box2>, str> y = Ok(Some(Box2 { v: own 9 }));
    match ref y {
        case Ok(Some(_)) -> print(\"ref\");
        default -> print(\"other\");
    }
    match y {
        case Ok(Some(b)) -> print(*b.v);
        case Ok(None) -> {}
        case Err(_) -> {}
    }
    let Option<bool> z = Some(true);
    match z { case Some(true) -> print(\"t\"); case Some(false) -> print(\"f\"); case None -> {} }
}
";
    assert_eq!(ok(src), "zero num 5 plus op * end end\n3\n4\nref\n9\nt\n");
}

#[test]
fn nested_pattern_errors() {
    let d = err("
fn main() {
    let Option<Option<i64>> x = None;
    match x { case Some(Some(v)) -> print(v); case None -> {} }
}");
    assert_eq!(d.title, "match で Some の場合が書かれていません");
    let d = err("
fn main() {
    let Option<own Option<i64>> x = None;
    match x { case Some(Some(v)) -> print(v); default -> {} }
}");
    assert_eq!(d.title, "値の match では、own の中に形を重ねられません");
    let d = err("
fn main() {
    let Option<i64> x = None;
    match x { case Some(y) -> {} case Some(1) -> {} case None -> {} }
}");
    assert_eq!(d.title, "同じ値の case が二回あります");
    let d = err("
fn main() {
    let Option<i64> x = None;
    match x { case Some(\"a\") -> {} default -> {} }
}");
    assert_eq!(d.title, "case の値と中身の型が合いません");
}

// ---------------------------------------------------------------- enum のメソッド

#[test]
fn enum_methods() {
    let src = "
enum Shape {
    Circle(f64 r);
    Rect(f64 w, f64 h);

    fn f64 area() {
        match this {
            case Circle(r) -> 3.0 * r * r;
            case Rect(w, h) -> w * h;
        }
    }
    fn mut grow() {
        match this {
            case Circle(r) -> *this = Shape.Circle(r * 2.0);
            case Rect(w, h) -> *this = Shape.Rect(w * 2.0, h * 2.0);
        }
    }
    static fn Shape square(f64 s) { Shape.Rect(s, s) }
    fn bool big() { area() > 100.0 }
}

enum Tree<T> {
    Leaf(T v);
    Node(own Tree<T> l, own Tree<T> r);

    fn i64 count() {
        match this {
            case Leaf(_) -> 1;
            case Node(l, r) -> l.count() + r.count();
        }
    }
}

fn main() {
    let mut s = Shape.square(3.0);
    print(s.area());
    s.grow();
    print(s.area(), s.big());
    let o = own Shape.Circle(10.0);
    print(o.big(), Shape.Rect(1.0, 2.0).area());
    let mut xs = [Shape.Circle(1.0), Shape.Circle(1.0)];
    xs[1].grow();
    print(xs[0].area(), xs[1].area());
    let t = Tree.Node(own Tree.Leaf(\"a\"), own Tree.Node(own Tree.Leaf(\"b\"), own Tree.Leaf(\"c\")));
    print(t.count());
}";
    assert_eq!(ok(src), "9.0\n36.0 false\ntrue 2.0\n3.0 12.0\n3\n");
}

#[test]
fn option_and_result_methods() {
    let src = "
fn Result<i64, str> half(i64 n) {
    if n % 2 == 0 { Ok(n / 2) } else { Err(\"odd\") }
}
fn main() {
    let a = Some(3);
    let Option<str> b = None;
    print(a.is_some(), a.is_none(), b.is_some(), b.is_none());
    print(half(4).is_ok(), half(3).is_err());
    let s = Some(own 5);
    if s.is_some() { print(\"yes\"); }
    let Option<i64>[] xs = [Some(1), None, Some(3)];
    let mut n = 0;
    for x in xs { if x.is_some() { n += 1; } }
    print(n);
}";
    assert_eq!(ok(src), "true false false true\ntrue true\nyes\n2\n");
}

#[test]
fn enum_method_errors() {
    let d = err("enum E { A; B; fn bool A() { true } } fn main() {}");
    assert_eq!(d.title, "メソッド `A` が種類と同じ名前です");
    let d = err("enum E { A; B; fn bool x() { true } } fn main() { print(E.x()); }");
    assert_eq!(d.title, "`E.x` は this のあるメソッドです");
    let d = err("enum E { A; B; static fn E mk() { E.A } } fn main() { let e = E.mk(); print(e.mk()); }");
    assert_eq!(d.title, "`E.mk` は静的メソッドです");
    let d = err("enum E { A; B; fn mut flip() { *this = E.B; } } fn main() { let e = E.A; e.flip(); }");
    assert_eq!(d.title, "`e` は mut ではないので書き換えられません");
    let d = err("enum E { A; B; fn mut flip() { this = E.B; } } fn main() {}");
    assert_eq!(d.title, "`this` そのものは書き換えられません");
    let d = err("enum E { A; B; fn flip() { *this = E.B; } } fn main() {}");
    assert_eq!(d.title, "書き換えないメソッドの中で書き換えようとしました");
    let d = err("enum E { A; fn x() {} B; } fn main() {}");
    assert_eq!(d.title, "種類はメソッドより前に書いてください");
    // 借りた match の中身は、this を上書きすると無効になる
    let d = err("
enum E {
    A(own i64 x);
    B;
    fn mut f() {
        match this {
            case A(x) -> { *this = E.B; print(*x); }
            case B -> {}
        }
    }
}
fn main() { let mut e = E.A(own 1); e.f(); }");
    assert_eq!(d.title, "無効になった参照を使いました (dangling reference)");
}

// ---------------------------------------------------------------- this を値で受け取るメソッド（fn take）

#[test]
fn take_methods() {
    let src = "
struct Tag {
    str name;
    i64 n;

    fn take str label() { \"{name}:{n}\" }
}

enum Shape {
    Circle(f64 r);
    Rect(f64 w, f64 h);

    fn take Shape scaled(f64 k) {
        match this {
            case Circle(r) -> Shape.Circle(r * k);
            case Rect(w, h) -> Shape.Rect(w * k, h * k);
        }
    }
    fn f64 area() { match this { case Circle(r) -> 3.0 * r * r; case Rect(w, h) -> w * h; } }
}

enum Slot {
    Full(own i64 v);
    Empty;

    fn take i64 open() { match this { case Full(v) -> *v; case Empty -> 0; } }
}

fn main() {
    let t = Tag { name: \"a\", n: 1 };
    print(t.label());
    print(Shape.Rect(1.0, 2.0).scaled(2.0).scaled(3.0).area());
    let c = Shape.Circle(1.0);
    let d = c.scaled(2.0);
    print(d.area(), c.area());
    let s = Slot.Full(own 7);
    print(s.open(), Slot.Empty.open());
}";
    assert_eq!(ok(src), "a:1\n72.0\n12.0 3.0\n7 0\n");
}

#[test]
fn option_and_result_take_methods() {
    let src = "
struct P { str name; }
fn Result<i64, str> half(i64 n) { if n % 2 == 0 { Ok(n / 2) } else { Err(\"odd\") } }
fn Result<i64, str> parse(str s) {
    let n = s.to_int().ok_or(\"数ではない\")?;
    Ok(n * 10)
}
fn i64 d(i64 x) { x }
fn main() {
    print(\"12\".to_int().unwrap_or(0), \"x\".to_int().unwrap_or(d(-1)));
    print(parse(\"4\"), parse(\"q\"));
    print(half(4).unwrap_or(0), half(3).unwrap_or(0), half(3).ok(), half(8).ok());
    print(Some(3).map((x) -> x * 2), half(6).map((x) -> \"v{x}\"));
    let Option<own P> op = Some(own P { name: \"a\" });
    let p = op.unwrap_or(own P { name: \"b\" });
    let Option<own P> none = None;
    let q = none.unwrap_or(own P { name: \"c\" });
    print(p.name, q.name, Some(own P { name: \"z\" }).map((pp) -> pp.name));
}";
    assert_eq!(ok(src), "12 -1\nOk(40) Err(数ではない)\n2 0 None Some(4)\nSome(6) Ok(v3)\na c Some(z)\n");
}

#[test]
fn take_method_errors() {
    let take = "enum E { A(own i64 x); B; fn take i64 get() { match this { case A(x) -> *x; case B -> 0; } } }\n";
    // 値を渡すのでムーブする
    let o = run_source(&format!("{take}fn main() {{ let e = E.A(own 3); print(e.get()); print(e.get()); }}"), false);
    assert_eq!(o.error.expect("moved").title, "ムーブ済みの値 `e` を使いました (use after move)");
    // ムーブした値を借りた ref は無効になる
    let o = run_source(
        &format!("{take}fn main() {{ let e = E.A(own 3); let r = ref e; print(e.get()); match r {{ case A(x) -> print(*x); case B -> {{}} }} }}"),
        false,
    );
    assert_eq!(o.error.expect("dangling").title, "無効になった参照を使いました (dangling reference)");
    // own / ref の先からは呼べない
    let d = err(&format!("{take}fn main() {{ let e = own E.A(own 3); print(e.get()); }}"));
    assert_eq!(d.title, "`E.get` は this を値で受け取るメソッドです");
    let d = err("enum E { A; fn take i64 get() { 1 } fn i64 peek() { get() } } fn main() {}");
    assert_eq!(d.title, "`E.get` は this を値で受け取るメソッドです");
    // interface のメソッドにはなれない
    let d = err("interface G { fn i64 get(); }
struct S { i64 v; fn take i64 get() { v } }
fn i64 need<T: G>(ref T t) { t.get() }
fn main() { let s = S { v: 1 }; print(need(s)); }");
    assert_eq!(d.title, "`S` は条件 `G` を満たしません");
    // `fn take(...)` は take という名前のメソッド
    let src = "struct S { i64 v; fn i64 take() { v } } fn main() { let s = S { v: 1 }; print(s.take()); }";
    assert_eq!(ok(src), "1\n");
}
