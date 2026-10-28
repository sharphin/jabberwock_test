//! `;`・`mut`・`ref mut`・複合代入・`++` のテスト

use jabberwock::run_source;

fn out(body: &str) -> String {
    let src = format!("fn main() {{\n{}\n}}", body);
    let o = run_source(&src, false);
    if let Some(d) = &o.error {
        panic!("unexpected error:\n{}", d.render("test.jw", &src));
    }
    assert_eq!(o.live_objects, 0, "heap objects leaked");
    o.output
}

fn err(src: &str) -> jabberwock::diag::Diagnostic {
    run_source(src, false).error.expect("expected an error")
}

fn err_in_main(body: &str) -> jabberwock::diag::Diagnostic {
    err(&format!("fn main() {{\n{}\n}}", body))
}

// ---------------------------------------------------------------- mut

#[test]
fn mut_errors_are_found_before_running() {
    let o = run_source("fn main() {\n    print(1);\n    let x = 1;\n    x = 2;\n}", false);
    assert!(o.error.unwrap().title.contains("mut ではない"));
    assert_eq!(o.output, "", "nothing runs when the program has a type error");
}

#[test]
fn variables_are_read_only_unless_mut() {
    let src = "fn main() {\n    let x = 1;\n    x = 2;\n}";
    let d = err(src);
    assert!(d.title.contains("`x` は mut ではない"), "{}", d.title);
    let r = d.render("t.jw", src);
    assert!(r.contains("ここで `x` を宣言した（mut がない）"), "{}", r);
    assert!(r.contains("let mut x"), "{}", r);
    assert_eq!(out("let mut x = 1;\nx = 2;\nprint(x);"), "2\n");
}

#[test]
fn params_and_loop_variables_are_read_only_unless_mut() {
    assert!(err("fn f(i64 a) {\n a = 1;\n}\nfn main() {\n f(0);\n}").title.contains("`a` は mut ではない"));
    let o = run_source("fn f(i64 mut a) {\n a += 1;\n print(a);\n}\nfn main() {\n f(1);\n}", false);
    assert_eq!(o.output, "2\n");
    assert!(err_in_main("for i in range(3) {\n i = 5;\n}").title.contains("`i` は mut ではない"));
}

#[test]
fn struct_fields_follow_the_owner() {
    let p = "struct P { i64 x; }\n";
    assert!(err(&format!("{p}fn main() {{\n let a = P {{ x: 1 }};\n a.x = 2;\n}}")).title.contains("`a` は mut ではない"));
    // own の先も持ち主が mut でないと書き換えられない
    assert!(err(&format!("{p}fn main() {{\n let a = own P {{ x: 1 }};\n a.x = 2;\n}}")).title.contains("`a` は mut ではない"));
    assert!(err_in_main("let a = own 1;\n*a = 2;").title.contains("`a` は mut ではない"));
}

// ---------------------------------------------------------------- ref mut

#[test]
fn writing_through_a_plain_ref_is_an_error() {
    let src = "fn main() {\n    let mut a = own 1;\n    let r = ref a;\n    *r = 2;\n}";
    let d = err(src);
    assert!(d.title.contains("読むだけの ref"), "{}", d.title);
    let r = d.render("t.jw", src);
    assert!(r.contains("`r` は読むだけの ref（mut がない）"), "{}", r);
    assert!(r.contains("ref mut"), "{}", r);
}

#[test]
fn ref_mut_needs_a_mut_owner() {
    let d = err_in_main("let a = own 1;\nlet r = ref mut a;");
    assert!(d.title.contains("`a` は mut ではない"), "{}", d.title);
    assert!(d.labels.iter().any(|l| l.text.contains("ref mut で借りようとした")));
}

#[test]
fn a_plain_ref_cannot_be_upgraded_to_ref_mut() {
    let d = err_in_main("let mut a = own 1;\nlet r = ref a;\nlet q = ref mut r;");
    assert!(d.title.contains("読むだけの ref"), "{}", d.title);
    // ref mut から読むだけの ref に借り直すのはよい
    assert_eq!(out("let mut a = own 1;\nlet r = ref mut a;\nlet q = ref r;\nprint(*q);"), "1\n");
}

#[test]
fn ref_mut_into_a_field() {
    let src = "struct B { own i64 inner; }
fn main() {
    let mut b = own B { inner: own 1 };
    let r = ref mut b.inner;
    *r += 41;
    print(*b.inner);
}";
    let o = run_source(src, false);
    assert!(o.error.is_none());
    assert_eq!(o.output, "42\n");
}

// ---------------------------------------------------------------- 複合代入と ++

#[test]
fn compound_assignment_and_increment() {
    let o = out("
    let mut x = 10;
    x += 5;
    x -= 3;
    x *= 2;
    x /= 4;
    x %= 4;
    print(x);
    let mut i = 0;
    i++;
    i++;
    i--;
    print(i);
    let mut p = own 1;
    *p += 1;
    *p++;
    print(*p);");
    assert_eq!(o, "2\n1\n3\n");
}

#[test]
fn increment_is_a_statement_only() {
    assert!(err_in_main("let mut i = 0;\nprint(i++);").title.contains("構文エラー"));
}

// ---------------------------------------------------------------- ; とブロックの値

#[test]
fn statements_need_semicolons() {
    let d = err_in_main("let x = 1\nprint(x);");
    assert!(d.title.contains("`;` が必要"), "{}", d.title);
    let d = err_in_main("let x = 1\n");
    assert!(d.title.contains("`;` が必要"), "{}", d.title);
    // { } で終わる文には要らない
    out("if true { print(1); }\nwhile false { }\nfor i in range(1) { }\nprint(2);");
}

#[test]
fn newlines_do_not_end_expressions() {
    assert_eq!(out("let x = 1\n    + 2\n    + 3;\nprint(x);"), "6\n");
}

#[test]
fn block_value_is_the_last_expression_without_semicolon() {
    assert_eq!(out("let v = if true { let t = 2; t * 3 } else { 0 };\nprint(v);"), "6\n");
    let d = err_in_main("let v = if true { 1; } else { 2 };");
    assert!(d.title.contains("最後に値がありません"), "{}", d.title);
    assert!(d.note.unwrap().contains("`;` を外す"));
}

#[test]
fn match_arms_end_with_semicolons() {
    let d = err_in_main("match 1 {\n case 1 -> print(1)\n}");
    assert!(d.title.contains("構文エラー"), "{}", d.title);
    let d = err_in_main("let x = match 1 {\n case 1 -> print(1);\n default -> { print(2); }\n};");
    assert!(d.title.contains("最後に値がありません"), "{}", d.title);
}
