//! loop / while / for / break / continue / if 式のテスト

use jabberwock::{run_source, run_source_with, Outcome};

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

fn out(body: &str) -> String {
    ok(&format!("fn main() {{\n{}\n}}", body)).output
}

fn err(src: &str) -> jabberwock::diag::Diagnostic {
    let d = run_source(src, false).error.expect("expected an error");
    let off = run_source_with(src, false, false).error.expect("expected an error without elision");
    assert_eq!(d.title, off.title, "elision changed the error");
    d
}

fn err_in_main(body: &str) -> jabberwock::diag::Diagnostic {
    err(&format!("fn main() {{\n{}\n}}", body))
}

fn dangling(body: &str) {
    let d = err_in_main(body);
    assert!(d.title.contains("dangling"), "unexpected error: {}", d.title);
}

// ---------------------------------------------------------------- ループ

#[test]
fn loop_with_break() {
    let o = out("
    let mut i = 0;
    loop {
        i = i + 1;
        if i == 3 { break; }
    }
    print(i);");
    assert_eq!(o, "3\n");
}

#[test]
fn while_with_continue() {
    let o = out("
    let mut i = 0;
    let mut sum = 0;
    while i < 6 {
        i = i + 1;
        if i % 2 == 0 { continue; }
        sum = sum + i;
    }
    print(sum);");
    assert_eq!(o, "9\n");
}

#[test]
fn for_range_forms() {
    let o = out("
    for i in range(3) { print(i) }
    for i in range(1, 10, 2) { print(i) }
    for i in range(10, 0, -4) { print(i) }");
    assert_eq!(o, "0\n1\n2\n1\n3\n5\n7\n9\n10\n6\n2\n");
}

#[test]
fn for_else_runs_only_when_the_body_never_ran() {
    let o = out("
    for i in range(5, 5) {
        print(\"body\")
    } else {
        print(\"empty\")
    }
    for i in range(2) {
        print(i)
    } else {
        print(\"never\")
    }
    for i in range(3) {
        break;
    } else {
        print(\"not after break either\")
    }");
    assert_eq!(o, "empty\n0\n1\n");
}

#[test]
fn for_else_with_break_in_the_body_warns() {
    // Python の for-else と読み違えやすいので、本体に break があれば警告（実行は止めない）
    let o = ok("fn main() {
    for x in [1, 0, 2] {
        if x == 0 {
            break;
        }
    } else {
        print(\"empty\");
    }
    print(\"done\");
}");
    assert_eq!(o.output, "done\n");
    assert_eq!(o.warnings.len(), 1);
    assert!(o.warnings[0].title.contains("1 回も回らなかったとき"), "{}", o.warnings[0].title);
    // 内側のループの break・else のない for は警告しない
    let o = ok("fn main() {
    for i in range(2) {
        while true {
            break;
        }
    } else {
        print(\"empty\");
    }
    for i in range(2) {
        break;
    }
}");
    assert!(o.warnings.is_empty(), "unexpected warning: {}", o.warnings[0].title);
}

#[test]
fn nested_loops_break_only_the_inner_one() {
    let o = out("
    for i in range(3) {
        for j in range(3) {
            if j == 1 { break; }
            print(i, j)
        }
    }");
    assert_eq!(o, "0 0\n1 0\n2 0\n");
}

#[test]
fn locals_are_freed_on_break_and_continue() {
    out("
    for i in range(4) {
        let a = own i;
        if i == 1 { continue; }
        if i == 2 { break; }
    }
    loop {
        let b = own 1;
        break;
    }");
}

#[test]
fn return_from_inside_a_loop() {
    let src = "
fn i64 find(i64 n) {
    for i in range(100) {
        if i * i >= n { return i; }
    }
    return -1;
}

fn main() {
    print(find(50))
}";
    assert_eq!(ok(src).output, "8\n");
}

// ---------------------------------------------------------------- if 式

#[test]
fn if_as_a_value() {
    let o = out("
    let a = 3;
    let b = 7;
    let m = if a > b { a } else { b };
    print(m);
    print(if a < 0 { \"負\" } else if a == 0 { \"ゼロ\" } else { \"正\" });
    let n = if true {
        let t = a * 10;
        t + 1
    } else {
        0
    };
    print(n);");
    assert_eq!(o, "7\n正\n31\n");
}

#[test]
fn nested_if_as_the_last_line_gives_the_value() {
    let o = out("
    let x = 5;
    let s = if x > 0 {
        if x > 3 { \"big\" } else { \"small\" }
    } else {
        \"neg\"
    };
    print(s);");
    assert_eq!(o, "big\n");
}

#[test]
fn if_value_can_be_an_own_and_is_freed() {
    let src = "
fn own i64 pick(bool c) {
    return if c { own 1 } else { own 2 };
}

fn main() {
    let x = if 1 < 2 { own 10 } else { own 20 };
    print(*x, *pick(false));
    if true { own 3 } else { own 4 }
}";
    assert_eq!(ok(src).output, "10 2\n");
}

// ---------------------------------------------------------------- エラー

#[test]
fn value_if_needs_else() {
    let d = err_in_main("let x = if true { 1 };");
    assert!(d.title.contains("else が必要"), "{}", d.title);
}

#[test]
fn break_and_continue_outside_a_loop() {
    assert!(err_in_main("break;").title.contains("ループの外"));
    assert!(err_in_main("if true { continue; }").title.contains("ループの外"));
    // for の else はループ本体ではない
    assert!(err_in_main("for i in range(0) { } else { break; }").title.contains("ループの外"));
}

#[test]
fn return_but_no_outer_break_inside_a_value_if() {
    let o = ok("fn i64 f(i64 a) {\n let x = if a > 0 { return a * 2; } else { 2 };\n x + 100\n}\nfn main() {\n print(f(5), f(-1));\n}");
    assert_eq!(o.output, "10 102\n");
    let d = err_in_main("loop {\n let x = if true { break; } else { 2 };\n}");
    assert!(d.title.contains("外のループへ break"), "{}", d.title);
    // 値として使う if の中のループで break するのはよい
    out("let x = if true {\n loop { break; }\n 1\n} else { 2 };");
}

#[test]
fn range_errors() {
    assert!(err_in_main("for i in range(1, 5, 0) { }").title.contains("刻みが 0"));
    assert!(err_in_main("for i in range(\"a\") { }").title.contains("型が合いません"));
    assert!(err_in_main("for i in range() { }").title.contains("1〜3"));
    assert!(err_in_main("for i in range(1, 2, 3, 4) { }").title.contains("1〜3"));
    let d = err_in_main("let x = 3;\nfor i in x { }");
    assert!(d.title.contains("配列・リスト・辞書") && d.note.as_deref().unwrap_or("").contains("range"), "{}", d.title);
}

// ---------------------------------------------------------------- 静的解析が安全側を保つこと

#[test]
fn ref_to_a_loop_local_escaping_through_break_is_checked() {
    dangling("
    let slot = own 0;
    let mut r = ref slot;
    loop {
        let a = own 1;
        r = ref a;
        break;
    }
    print(*r);");
}

#[test]
fn drop_before_continue_is_seen_on_the_next_iteration() {
    dangling("
    let a = own 1;
    let r = ref a;
    let mut i = 0;
    while i < 3 {
        i = i + 1;
        if i == 2 {
            drop(a);
            continue;
        }
        print(*r)
    }");
}

#[test]
fn drop_before_break_is_seen_after_the_loop() {
    dangling("
    let a = own 1;
    let r = ref a;
    loop {
        drop(a);
        break;
    }
    print(*r);");
}

#[test]
fn drop_in_for_else_is_seen_after_the_loop() {
    dangling("
    let a = own 1;
    let r = ref a;
    for i in range(0) {
    } else {
        drop(a)
    }
    print(*r);");
}

#[test]
fn drop_in_one_branch_of_an_if_value() {
    dangling("
    let a = own 1;
    let r = ref a;
    let x = if true {
        drop(a);
        1
    } else {
        2
    };
    print(*r);");
}

#[test]
fn checks_are_still_elided_in_loops_with_break() {
    let o = ok("
fn main() {
    let mut s = own 0;
    let r = ref mut s;
    for i in range(10) {
        *r = *r + i;
        if i == 5 { break; }
    }
    print(*r)
}");
    assert_eq!(o.output, "15\n");
    assert_eq!(o.ref_checks, 0);
}
