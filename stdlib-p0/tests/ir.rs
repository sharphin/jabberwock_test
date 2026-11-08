//! IR（中間表現）への書き出し。実行結果の突き合わせは、デバッグビルドのすべてのテストで
//! `run_source` などが自動でしている（`lib.rs` の `ir_mismatch`）。ここでは表示と、書き出せる範囲を確かめる

use jabberwock::{ir_text, no_modules, run_source};

fn ir(src: &str) -> String {
    ir_text(src, &no_modules).expect("lowers to IR")
}

#[test]
fn moves_drops_and_checks_are_visible() {
    let t = ir("fn main() {\n    let a = own 5;\n    let r = ref a;\n    print(*r);\n    drop(a);\n}");
    let expected = "\
fn main() -> 値なし {
    let _0: own i64;
    let _1: own i64;  // a
    let _2: ref i64;
    let _3: ref i64;  // r
    let _4: str;
    let _5: 値なし;
    let _6: own i64;
  bb0:
    _0 = own 5i64
    _1 = move _0
    _2 = ref _1
    _3 = move _2
    _4 = show!((*_3))
    _5 = print!(move _4)
    discard move _5
    _6 = move _1
    discard move _6
    drop _3
    drop _1
    return ()
}
";
    assert_eq!(t, expected);
}

#[test]
fn a_ref_that_may_dangle_keeps_its_check() {
    let src = "fn i64 get(ref i64 p) { *p }\nfn main() {\n    let a = own 1;\n    print(get(ref a));\n}";
    let t = ir(src);
    // 検査を消せたかどうかは `(*_0)`（消せた）と `(*!_0)`（実行時に検査）で見分けられる
    assert!(t.contains("(*_0)") || t.contains("(*!_0)"), "{}", t);
    assert_eq!(run_source(src, false).output, "1\n");
}

#[test]
fn loops_and_early_exits_drop_what_they_leave() {
    let src = "\
fn main() {
    let mut n = 0;
    for i in range(10) {
        let x = own i;
        if i == 3 { continue; }
        if i == 6 { break; }
        n += *x;
    }
    print(n);
}";
    let t = ir(src);
    assert!(t.contains("step "), "{}", t);
    let o = run_source(src, false);
    assert_eq!(o.output, "12\n");
    assert_eq!(o.live_objects, 0);
}

#[test]
fn unsupported_programs_say_why() {
    let src = "fn main() {\n    let ms = [[\"a\": 1]];\n    for (k, v) in ms[0] {\n        print(k, v);\n    }\n}";
    let e = ir_text(src, &no_modules).unwrap_err();
    assert!(e.contains("要素の中の辞書"), "{}", e);
    // 書き出せなくてもインタプリタでは動く
    assert_eq!(run_source(src, false).output, "a 1\n");
}

#[test]
fn refs_to_list_elements_skip_the_check_until_the_list_may_change() {
    // 要素を借りた ref（`ref mut ps[i]`）は、リストの要素が動きうる操作までは世代の検査がいらない
    let head = "struct P { i64 x; }\nfn bump(ref mut P[] ps) { ps.push(P { x: 0 }); }\n";
    let checked = |body: &str| {
        let src = format!("{}fn main() {{\n    let P[] mut ps = [P {{ x: 1 }}, P {{ x: 2 }}];\n    let p = ref mut ps[1];\n    p.x = 5;\n{}\n    p.x = 6;\n}}", head, body);
        let t = ir(&src);
        (t.matches("(*!").count(), t.matches("(*_").count())
    };
    assert_eq!(checked("    let n = ps.len();\n    print(n, p.x);"), (0, 3));
    // 要素が動く・リストを丸ごと替える・関数に渡す・別の ref から替える、のあとは検査を残す
    for body in [
        "    ps.push(P { x: 3 });",
        "    ps.sort_unstable_by((a, b) -> a.x < b.x);",
        "    ps = [P { x: 9 }];",
        "    bump(ref mut ps);",
        "    let q = ref mut ps;\n    *q = [];",
    ] {
        let (c, _) = checked(body);
        assert!(c >= 1, "{}", body);
    }
    // 引数の ref の先のリストでも同じ
    let t = ir(&format!("{}fn set(ref mut P[] ps) {{\n    let p = ref mut ps[0];\n    p.x = 1;\n    p.x = p.x + 1;\n}}\nfn main() {{\n    let P[] mut ps = [P {{ x: 1 }}];\n    set(ref mut ps);\n}}", head));
    assert_eq!(t.matches("(*!").count(), 0, "{}", t);
}

#[test]
fn scalar_direct_calls_preserve_element_refs_across_blocks() {
    let src = "
fn T id<T>(T x) { x }
fn i64 add(i64 x) { x + 1 }
fn str label(str s) { s + \"!\" }
fn main() {
    let i64[] mut xs = [1];
    let p = ref mut xs[0];
    let mut n = 0;
    for i in range(4) {
        n = add(n);
        if id(true) { *p += n; } else { *p = 0; }
    }
    print(*p, label(\"ok\"));
}";
    let t = ir(src);
    assert!(!t.contains("(*!"), "{}", t);
    let on = run_source(src, false);
    let off = jabberwock::run_source_with(src, false, false);
    assert!(on.error.is_none(), "{:?}", on.error);
    assert_eq!(on.output, "11 ok!\n");
    assert_eq!(on.output, off.output);
    assert_eq!(on.live_objects, 0);
}

#[test]
fn scalar_call_does_not_hide_mutation_while_evaluating_arguments() {
    let src = "
fn i64 id(i64 n) { n }
fn i64 change(ref mut i64[] xs) { xs.push(2); 3 }
fn main() {
    let i64[] mut xs = [1];
    let p = ref xs[0];
    print(id(change(ref mut xs)));
    print(*p);
}";
    let t = ir(src);
    assert!(t.contains("(*!"), "{}", t);
    let on = run_source(src, false);
    let off = jabberwock::run_source_with(src, false, false);
    assert_eq!(on.output, "3\n");
    assert_eq!(on.output, off.output);
    let a = on.error.expect("element ref must dangle");
    let b = off.error.expect("unoptimized must also fail");
    assert!(a.title.contains("dangling"), "{}", a.title);
    assert_eq!(a.render("test.jw", src), b.render("test.jw", src));
}

#[test]
fn callbacks_and_ref_receivers_still_invalidate_element_facts() {
    for (head, body) in [
        ("fn change(ref mut i64[] xs) { xs.push(2); }", "change(ref mut xs);"),
        ("fn change(ref mut i64[] xs) { xs.push(2); } fn call(fn() f) { f(); }", "let q = ref mut xs; call(() -> change(q));"),
        ("fn change(ref mut i64[] xs) { xs.push(2); }", "let q = ref mut xs; let f = () -> change(q); f();"),
        ("", "let f = () -> 7; print(f()); xs.push(2);"),
    ] {
        let src = format!("{}\nfn main() {{\n    let i64[] mut xs = [1];\n    let p = ref xs[0];\n    {}\n    print(*p);\n}}", head, body);
        let t = ir(&src);
        assert!(t.contains("(*!"), "{}", t);
        let on = run_source(&src, false);
        let off = jabberwock::run_source_with(&src, false, false);
        assert_eq!(on.output, off.output);
        let a = on.error.expect("element ref must dangle");
        let b = off.error.expect("unoptimized must also fail");
        assert!(a.title.contains("dangling"), "{}", a.title);
        assert_eq!(a.render("test.jw", &src), b.render("test.jw", &src));
    }
}

#[test]
fn scalar_call_errors_keep_output_and_diagnostics() {
    let src = "
fn i64 divide(i64 n) { print(\"helper\"); 10 / n }
fn main() {
    let i64[] mut xs = [1];
    let p = ref mut xs[0];
    print(\"before\", *p);
    let n = divide(0);
    *p += n;
}";
    let on = run_source(src, false);
    let off = jabberwock::run_source_with(src, false, false);
    assert_eq!(on.output, "before 1\nhelper\n");
    assert_eq!(on.output, off.output);
    assert_eq!(
        on.error.expect("division by zero").render("test.jw", src),
        off.error.expect("division by zero").render("test.jw", src)
    );
}
