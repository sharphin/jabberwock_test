//! 型検査のテスト。型の違反は実行する前に見つかる。

use jabberwock::run_source;

fn ok(src: &str) -> String {
    let o = run_source(src, false);
    if let Some(d) = &o.error {
        panic!("unexpected error:\n{}", d.render("test.jw", src));
    }
    assert_eq!(o.live_objects, 0, "heap objects leaked");
    o.output
}

/// 型エラーになり、何も実行されないこと
fn type_err(src: &str) -> jabberwock::diag::Diagnostic {
    let o = run_source(src, false);
    assert_eq!(o.output, "", "a program with a type error must not run");
    o.error.expect("expected a type error")
}

fn main_err(body: &str) -> jabberwock::diag::Diagnostic {
    type_err(&format!("fn main() {{\n    print(0);\n{}\n}}", body))
}

// ---------------------------------------------------------------- 書き方

#[test]
fn types_are_written_before_names() {
    let src = "
struct Point { i64 x; i64 y; }

fn i64 add(i64 a, i64 b) {
    return a + b;
}

fn own Point origin() {
    return own Point { x: 0, y: 0 };
}

fn shift(ref mut Point p, i64 dx) {
    p.x += dx;
}

fn main() {
    let i64 mut n = add(1, 2);
    n++;
    let str s = \"n =\";
    let bool b = n > 3;
    let mut p = origin();
    shift(ref mut p, n);
    print(s, n, b, p.x);
}";
    assert_eq!(ok(src), "n = 4 true 4\n");
}

#[test]
fn the_last_expression_is_the_return_value() {
    let src = "
fn i64 square(i64 x) {
    x * x
}

fn str sign(i64 n) {
    if n < 0 { \"負\" } else if n == 0 { \"ゼロ\" } else { \"正\" }
}

fn main() {
    print(square(7), sign(-3), sign(0), sign(5));
}";
    assert_eq!(ok(src), "49 負 ゼロ 正\n");
}

#[test]
fn a_function_may_end_in_a_loop_it_never_leaves() {
    let src = "
fn i64 first_square_over(i64 n) {
    let mut i = 0;
    loop {
        if i * i > n { return i; }
        i++;
    }
}

fn main() {
    print(first_square_over(50));
}";
    assert_eq!(ok(src), "8\n");
}

#[test]
fn ref_mut_can_be_passed_where_ref_is_expected() {
    let src = "
fn show(ref i64 r) {
    print(*r);
}

fn main() {
    let mut a = own 5;
    let r = ref mut a;
    show(r);
    show(ref a);
}";
    assert_eq!(ok(src), "5\n5\n");
}

#[test]
fn missing_types_in_signatures_get_a_friendly_error() {
    let d = type_err("fn f(a) {\n}\nfn main() {\n}");
    assert!(d.title.contains("引数には型が必要"), "{}", d.title);
    assert!(d.note.unwrap().contains("i64 a"));
    let d = type_err("struct P { x; }\nfn main() {\n}");
    assert!(d.title.contains("フィールドには型が必要"), "{}", d.title);
    let d = type_err("struct P { i64 x, i64 y }\nfn main() {\n}");
    assert!(d.title.contains("区切りは `;`"), "{}", d.title);
}

#[test]
fn unknown_and_not_yet_supported_types() {
    assert!(type_err("fn f(Nope n) {\n}\nfn main() {\n}").title.contains("未定義の型 `Nope`"));
    assert!(type_err("fn f(f32 n) {\n}\nfn main() {\n}").title.contains("`f32` はまだ使えません"));
}

// ---------------------------------------------------------------- 型の食い違い

#[test]
fn mismatches_are_found_before_running() {
    let d = main_err("    let i64 x = \"a\";");
    assert!(d.title.contains("型が合いません"));
    assert!(d.labels.iter().any(|l| l.text.contains("`str` ですが、`i64` が必要")));
    assert!(d.labels.iter().any(|l| l.text.contains("ここで型を決めた")));

    assert!(main_err("    let mut x = 1;\n    x = true;").title.contains("型が合いません"));
    assert!(main_err("    print(1 + \"a\");").title.contains("型が合わない演算"));
    assert!(main_err("    print(true < false);").title.contains("型が合わない演算"));
    assert!(main_err("    print(1 == true);").title.contains("比較"));
    assert!(main_err("    if 1 { }").title.contains("型が合いません"));
    assert!(main_err("    while \"x\" { }").title.contains("型が合いません"));
    assert!(main_err("    print(!3);").title.contains("型が合いません"));
    assert!(main_err("    assert(1);").title.contains("型が合いません"));
}

#[test]
fn arguments_and_return_values_are_checked() {
    let f = "fn i64 twice(i64 n) {\n    return n * 2;\n}\n";
    assert!(type_err(&format!("{f}fn main() {{\n    twice(true);\n}}")).title.contains("型が合いません"));
    assert!(type_err(&format!("{f}fn main() {{\n    twice(1, 2);\n}}")).title.contains("引数の数"));
    assert!(type_err(&format!("{f}fn main() {{\n    let str s = twice(1);\n}}")).title.contains("型が合いません"));
    assert!(type_err("fn i64 f() {\n    return \"x\";\n}\nfn main() {\n}").title.contains("型が合いません"));
    assert!(type_err("fn i64 f() {\n    return;\n}\nfn main() {\n}").title.contains("return に値がありません"));
    assert!(type_err("fn f() {\n    return 1;\n}\nfn main() {\n}").title.contains("値を返さない関数で値を return"));
    assert!(type_err("fn f() {\n}\nfn main() {\n    let x = f();\n}").title.contains("値のない式"));
}

#[test]
fn every_path_must_return_a_value() {
    let d = type_err("fn i64 f(i64 n) {\n    if n > 0 {\n        return 1;\n    }\n}\nfn main() {\n}");
    assert!(d.title.contains("値を返さないまま関数が終わる道"), "{}", d.title);
    // while は条件しだいで 0 回なので、その後ろに return が要る
    let d = type_err("fn i64 f() {\n    while true {\n        return 1;\n    }\n}\nfn main() {\n}");
    assert!(d.title.contains("値を返さないまま"), "{}", d.title);
    // ; をつけると値ではなくなる
    let d = type_err("fn i64 f() {\n    1;\n}\nfn main() {\n}");
    assert!(d.title.contains("値を返さないまま"), "{}", d.title);
}

#[test]
fn branches_of_a_value_must_agree() {
    let d = main_err("    let x = if true { 1 } else { \"one\" };");
    assert!(d.title.contains("if の枝で値の型が違います"), "{}", d.title);
    let d = main_err("    let x = match 1 {\n        case 1 -> 1;\n        default -> false;\n    };");
    assert!(d.title.contains("match の枝で値の型が違います"), "{}", d.title);
    assert!(main_err("    match 1 {\n        case \"a\" -> print(1);\n    }").title.contains("case の値"));
}

// ---------------------------------------------------------------- own / ref と構造体

#[test]
fn pointers_are_checked() {
    assert!(main_err("    let r = ref 5;").title.contains("借用できない式"));
    assert!(main_err("    let x = 5;\n    print(*x);").title.contains("`*` でたどれるのは own と ref だけ"));
    let d = main_err("    let a = own 1;\n    let ref bool r = ref a;");
    assert!(d.labels.iter().any(|l| l.text.contains("`ref i64` ですが、`ref bool` が必要")));
    // ref は ref mut の代わりにはならない
    let src = "fn inc(ref mut i64 r) {\n    *r += 1;\n}\nfn main() {\n    let mut a = own 1;\n    inc(ref a);\n}";
    assert!(type_err(src).labels.iter().any(|l| l.text.contains("`ref i64` ですが、`ref mut i64` が必要")));
}

#[test]
fn struct_fields_are_typed() {
    let p = "struct P { i64 x; own i64 o; }\n";
    let d = type_err(&format!("{p}fn main() {{\n    let a = P {{ x: \"s\", o: own 1 }};\n}}"));
    assert!(d.title.contains("型が合いません"));
    let d = type_err(&format!("{p}fn main() {{\n    let mut a = P {{ x: 1, o: own 1 }};\n    a.o = 2;\n}}"));
    assert!(d.labels.iter().any(|l| l.text.contains("`i64` ですが、`own i64` が必要")));
    let d = type_err(&format!("{p}fn main() {{\n    let a = P {{ x: 1, o: own 1 }};\n    print(a.y);\n}}"));
    assert!(d.title.contains("フィールド `y` はありません"));
}

#[test]
fn a_struct_cannot_contain_itself_directly() {
    let d = type_err("struct Node { i64 v; Node next; }\nfn main() {\n}");
    assert!(d.title.contains("自分自身をじかに含んで"), "{}", d.title);
    let d = type_err("struct A { B b; }\nstruct B { A a; }\nfn main() {\n}");
    assert!(d.title.contains("自分自身"), "{}", d.title);
    // own を通せばよい（作り方は Option が入ってから）
    ok("struct Node { i64 v; own Node next; }\nfn main() {\n}");
}

#[test]
fn definition_errors() {
    assert!(type_err("fn f() {\n}\nfn f() {\n}\nfn main() {\n}").title.contains("二重"));
    assert!(type_err("fn f(i64 a, i64 a) {\n}\nfn main() {\n}").title.contains("二回"));
    assert!(type_err("fn i64 main() {\n    return 0;\n}").title.contains("`main` は引数も戻り値もない"));
    assert!(type_err("fn main() {\n    print(nope);\n}").title.contains("未定義の変数"));
    assert!(type_err("fn main() {\n    nope();\n}").title.contains("未定義の関数"));
}

// ---------------------------------------------------------------- 警告

#[test]
fn constant_overflow_is_warned_but_runs() {
    let src = "fn main() {\n    let x = 9223372036854775807 + 1;\n    print(x);\n}";
    let o = run_source(src, false);
    assert!(o.error.is_none());
    assert_eq!(o.output, "-9223372036854775808\n");
    assert_eq!(o.warnings.len(), 1);
    let w = &o.warnings[0];
    assert!(w.title.contains("整数があふれます"));
    assert!(w.render("t.jw", src).starts_with("warning:"));
    // 定数どうしの計算なら、式の途中でも見つかる。一度だけ警告する
    let o = run_source("fn main() {\n    print((4611686018427387904 * 2 - 1) + 5);\n}", false);
    assert_eq!(o.warnings.len(), 1);
    // 0 で割る定数も警告（実行するとエラー）
    let o = run_source("fn main() {\n    print(1 % 0);\n}", false);
    assert!(o.warnings[0].title.contains("0 で割"));
    assert!(o.error.is_some());
    // 範囲内なら警告なし
    assert!(run_source("fn main() {\n    print(-9223372036854775807 - 1);\n}", false).warnings.is_empty());
}

// ---------------------------------------------------------------- シャドーイングの禁止

#[test]
fn shadowing_is_an_error() {
    let shadow = |d: jabberwock::diag::Diagnostic| {
        assert!(d.title.contains("すでにある名前"), "{}", d.title);
        assert!(d.labels.iter().any(|l| l.text.contains("ここで宣言した")));
    };
    // 同じスコープ・内側のスコープ・ループの変数・case で受け取る名前・引数
    shadow(main_err("    let a = 1;\n    let a = 2;"));
    shadow(main_err("    let a = 1;\n    if true {\n        let a = 2;\n    }"));
    shadow(main_err("    let i = 0;\n    for i in range(3) { }"));
    shadow(main_err("    let x = 0;\n    for x in [1, 2] { }"));
    shadow(main_err("    let v = 0;\n    match Some(1) {\n        case Some(v) -> print(v);\n        case None -> {}\n    }"));
    shadow(type_err("fn f(i64 n) {\n    let n = 2;\n}\nfn main() {\n    print(0);\n}"));
}

#[test]
fn same_names_in_separate_blocks_are_fine() {
    let src = "
fn main() {
    if true {
        let a = 1;
        print(a);
    }
    if true {
        let a = \"two\";
        print(a);
    }
    for i in range(1) { print(i); }
    for i in range(1) { print(i); }
}";
    assert_eq!(ok(src), "1\ntwo\n0\n0\n");
}

#[test]
fn method_params_and_locals_cannot_use_field_names() {
    let p = "struct P {\n    i64 x;\n";
    let d = type_err(&format!("{p}    fn i64 f(i64 x) {{\n        x\n    }}\n}}\nfn main() {{\n    print(0);\n}}"));
    assert!(d.title.contains("フィールドの名前"), "{}", d.title);
    let d = type_err(&format!("{p}    fn i64 f() {{\n        let x = 1;\n        x\n    }}\n}}\nfn main() {{\n    print(0);\n}}"));
    assert!(d.title.contains("フィールドの名前"), "{}", d.title);
    // 静的メソッドには this がないので、フィールドと同じ名前の引数でよい
    let src = format!("{p}    static fn P at(i64 x) {{\n        P {{ x: x }}\n    }}\n}}\nfn main() {{\n    print(P.at(5).x);\n}}");
    assert_eq!(ok(&src), "5\n");
}
