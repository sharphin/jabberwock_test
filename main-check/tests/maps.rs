//! 辞書 `i64[str]` と `for (k, v) in m` のテスト

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
fn basic_operations() {
    let src = "
fn main() {
    let i64[str] mut m = [];
    m[\"a\"] = 1;
    m[\"b\"] = 2;
    m[\"a\"] += 10;
    print(m, m.len(), m[\"a\"], m.get(\"z\"), m.get(\"b\"), m.contains(\"b\"));
    for (k, v) in m { print(k, v); }
    print(m.keys());
    print(m.remove(\"a\"), m.remove(\"a\"), m);
    let ages = [\"x\": 3, \"y\": 4, \"x\": 5];
    print(ages);
}";
    assert_eq!(
        ok(src),
        "[a: 11, b: 2] 2 11 None Some(2) true\na 11\nb 2\n[a, b]\nSome(11) None [b: 2]\n[x: 5, y: 4]\n"
    );
}

#[test]
fn structs_and_owns_as_values() {
    let src = "
struct P {
    i64 hp;
}

fn i64[str] make() { [\"k\": 42] }

fn main() {
    let mut ps = [1: P { hp: 5 }];
    ps[1].hp = 7;
    ps[2] = P { hp: 1 };
    for (k, p) in ps { print(k, p.hp); }
    print(ps.remove(1));
    let (own i64)[i64] mut os = [];
    os[3] = own 9;
    os[3] = own 10;
    os[4] = own 11;
    print(os, *os[4]);
    print(make()[\"k\"]);
    let i64[str][str] mut mm = [];
    mm[\"a\"] = [\"b\": 1];
    mm[\"a\"][\"c\"] = 2;
    print(mm);
}";
    assert_eq!(ok(src), "1 7\n2 1\nSome(P { hp: 7 })\n[3: 10, 4: 11] 11\n42\n[a: [b: 1, c: 2]]\n");
}

#[test]
fn passing_maps() {
    let src = "
fn add(ref mut i64[str] m, str k) { m[k] = m.len(); }

fn i64 total(ref i64[str] m) {
    let mut t = 0;
    for (k, v) in m { t += v; }
    t
}

fn main() {
    let i64[str] mut m = [];
    add(ref mut m, \"x\");
    add(ref mut m, \"y\");
    print(m, total(m));
    let n = m;
    print(n);
}";
    assert_eq!(ok(src), "[x: 0, y: 1] 1\n[x: 0, y: 1]\n");
}

#[test]
fn for_with_index() {
    let src = "
fn main() {
    let a = [10, 20];
    for (i, x) in a { print(i, x); }
    let str[] mut l = [];
    l.push(\"p\");
    for (i, s) in l { print(\"{i}:{s}\"); }
}";
    assert_eq!(ok(src), "0 10\n1 20\n0:p\n");
}

#[test]
fn missing_key_is_a_runtime_error() {
    let o = run_source("fn main() {\n    let m = [\"a\": 1];\n    print(m[\"zz\"]);\n}", false);
    let d = o.error.unwrap();
    assert!(d.title.contains("キー \"zz\" がありません"), "{}", d.title);
}

#[test]
fn map_errors() {
    let e = |body: &str| err(&format!("struct P {{\n    i64 x;\n}}\nfn main() {{\n{}\n}}", body));
    assert!(e("let i64[str] m = [\"a\": 1];\nm[\"b\"] = 2;").title.contains("mut ではない"));
    assert!(e("let m = [\"a\": 1];\nm.remove(\"a\");").title.contains("mut ではない"));
    assert!(e("let f64[f64] m = [];").title.contains("キーにできません"));
    assert!(e("let m = [\"a\": 1];\nfor k in m {\n}").title.contains("for (k, v) in m"));
    assert!(e("let m = [1: P { x: 1 }];\nlet p = m.get(1);").title.contains("get"));
    assert!(e("let m = [\"a\": 1];\nlet x = m[1];").title.contains("型が合いません"));
    assert!(e("let m = [\"a\": 1, 2: 3];").title.contains("型が合いません"));
    assert!(e("let i64[str] mut m = [];\nm[\"a\"] = 1;\nlet r = ref m[\"a\"];").title.contains("借りられません"));
    assert!(e("let m = [\"a\": 1];\nm.clear();").title.contains("メソッド `clear`"));
    assert!(e("for (i, x) in range(3) {\n}").title.contains("range"));
}
