//! 静的解析で ref の検査を消す仕組みのテスト。
//!
//! 大事なのは「消してよい所だけ消す」こと。デバッグビルドでは、消した箇所が実はダングリング
//! だった場合に panic するので、危ないプログラムを流して panic せずエラーになることも確かめる。

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
    o
}

/// 解析あり・なしで同じ出力・同じエラーになること
fn same_with_and_without_elision(src: &str) {
    let on = run_source_with(src, false, true);
    let off = run_source_with(src, false, false);
    assert_eq!(on.output, off.output);
    assert_eq!(on.error.map(|d| d.title), off.error.map(|d| d.title));
    assert_eq!(off.ref_checks_elided, 0);
}

/// ダングリングが（消されずに）実行時に検出されること
fn dangling(src: &str) {
    same_with_and_without_elision(src);
    let d = run_source(src, false).error.expect("expected an error");
    assert!(d.title.contains("dangling"), "unexpected error: {}", d.title);
}

// ---------------------------------------------------------------- 消せる場合

#[test]
fn local_borrow_is_elided() {
    let src = "
fn main() {
    let mut a = own 1;
    let r = ref mut a;
    *r = *r + 1;
    print(*r)
}";
    let o = ok(src);
    assert_eq!(o.output, "2\n");
    assert_eq!(o.ref_checks, 0);
    assert_eq!(o.ref_checks_elided, 3);
    assert_eq!(o.analysis.required, 0);
}

#[test]
fn ref_param_is_elided_when_every_caller_passes_a_live_ref() {
    let o = ok(&example("counter"));
    assert_eq!(o.output, "count = 3\n");
    assert_eq!(o.ref_checks, 0);
    assert_eq!(o.ref_checks_elided, 6);
}

#[test]
fn pinned_ref_survives_being_passed_on_and_recursion() {
    let src = "
fn add(ref mut i64 r, i64 n) {
    if n > 0 {
        *r = *r + 1;
        add(ref mut r, n - 1)
    }
}

fn main() {
    let mut total = own 0;
    let r = ref mut total;
    add(r, 100);
    print(*total)
}";
    let o = ok(src);
    assert_eq!(o.output, "100\n");
    assert_eq!(o.ref_checks, 0);
}

#[test]
fn borrow_in_loop_is_elided() {
    let src = "
fn main() {
    let mut sum = own 0;
    let r = ref mut sum;
    let mut i = 0;
    while i < 10 {
        *r = *r + i;
        i = i + 1;
    }
    print(*sum)
}";
    let o = ok(src);
    assert_eq!(o.output, "45\n");
    assert_eq!(o.ref_checks, 0);
}

// ---------------------------------------------------------------- 消してはいけない場合

#[test]
fn ref_into_inner_scope_is_still_checked() {
    dangling(&example("dangling_scope"));
    dangling(&example("dangling_return"));
}

#[test]
fn reassigning_the_owner_keeps_the_check() {
    dangling(
        "
fn main() {
    let mut a = own 1;
    let r = ref a;
    a = own 2;
    print(*r)
}",
    );
}

#[test]
fn moving_the_owner_keeps_the_check() {
    dangling(
        "
fn eat(own i64 c) {
}

fn main() {
    let a = own 1;
    let r = ref a;
    eat(a);
    print(*r)
}",
    );
}

#[test]
fn dropping_the_owner_keeps_the_check() {
    dangling(
        "
fn main() {
    let a = own 1;
    let r = ref a;
    drop(a);
    print(*r)
}",
    );
}

#[test]
fn drop_later_in_the_loop_keeps_the_check() {
    // 1周目は安全でも、2周目には a が解放されている
    let src = "
fn main() {
    let a = own 1;
    let r = ref a;
    let mut i = 0;
    while i < 2 {
        print(*r);
        if i == 0 {
            drop(a)
        }
        i = i + 1;
    }
}";
    dangling(src);
}

#[test]
fn drop_in_one_branch_keeps_the_check() {
    dangling(
        "
fn main() {
    let a = own 1;
    let r = ref a;
    if 1 < 2 {
        drop(a)
    }
    print(*r)
}",
    );
}

#[test]
fn ref_and_its_owner_passed_together_keeps_the_check() {
    // ref を作った後、同じ呼び出しの中で own もムーブして関数側で解放する
    dangling(
        "
fn f(ref i64 r, own i64 c) {
    drop(c);
    print(*r)
}

fn main() {
    let a = own 1;
    f(ref a, a)
}",
    );
}

#[test]
fn one_bad_caller_keeps_the_param_checked() {
    dangling(
        "
fn show(ref i64 r) {
    print(*r)
}

fn ref i64 stale() {
    let a = own 1;
    return ref a;
}

fn main() {
    let b = own 2;
    show(ref b);
    show(stale())
}",
    );
}

#[test]
fn same_names_in_sibling_scopes_do_not_confuse_the_analysis() {
    // シャドーイングは禁止だが、別々のブロックで同じ名前を使うのはよい。二つの `a` は別の変数
    dangling(
        "
fn main() {
    let z = own 0;
    let mut r = ref z;
    if true {
        let a = own 1;
        r = ref a;
    }
    if true {
        let a = own 2;
        print(*a);
    }
    print(*r)
}",
    );
}

#[test]
fn stale_ref_passed_through_ref_of_ref_keeps_the_check() {
    dangling(
        "
fn show(ref i64 r) {
    print(*r)
}

fn main() {
    let r = r0();
    show(ref r)
}

fn ref i64 r0() {
    let x = own 5;
    return ref x;
}",
    );
}

#[test]
fn self_reassignment_through_its_own_ref_is_a_type_error() {
    // 型がなかったころは実行時に検出していた。今は own と ref の型が違うので実行前に止まる
    let d = run_source("fn main() {\n    let mut a = own 1;\n    a = ref a;\n}", false).error.expect("error");
    assert!(d.title.contains("型が合いません"), "{}", d.title);
}

// ---------------------------------------------------------------- 全体

#[test]
fn all_examples_behave_the_same_with_and_without_elision() {
    for name in ["counter", "dangling_return", "dangling_scope", "fib", "move", "use_after_move"] {
        same_with_and_without_elision(&example(name));
    }
}

#[test]
fn a_name_taken_in_a_case_is_a_new_variable() {
    // case の `r` は前の case の `r`（生きている ref）とは別の変数。前の事実を使って検査を消してはいけない
    dangling(
        "
enum E {
    A(ref i64 r);
}

fn ref i64 stale() {
    let x = own 1;
    return ref x;
}

fn main() {
    let a = own 1;
    match E.A(ref a) {
        case A(r) -> print(*r);
    }
    let e = E.A(stale());
    match e {
        case A(r) -> print(*r);
    }
}",
    );
}
