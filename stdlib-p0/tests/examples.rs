use std::fs;

use jabberwock::run_source;

fn example(name: &str) -> String {
    let path = format!("{}/examples/{}.jw", env!("CARGO_MANIFEST_DIR"), name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {}", path, e))
}

fn ok(src: &str) -> String {
    let o = run_source(src, false);
    if let Some(d) = &o.error {
        panic!("unexpected error:\n{}", d.render("test.jw", src));
    }
    assert_eq!(o.live_objects, 0, "heap objects leaked");
    o.output
}

fn err(src: &str) -> jabberwock::diag::Diagnostic {
    run_source(src, false).error.expect("expected an error")
}

// ---------------------------------------------------------------- 正常系

#[test]
fn counter_example() {
    assert_eq!(ok(&example("counter")), "count = 3\n");
}

#[test]
fn move_example() {
    assert_eq!(ok(&example("move")), "a = 42\nconsume: 42\n");
}

#[test]
fn fib_example() {
    let out = ok(&example("fib"));
    assert!(out.ends_with("fib( 15 ) = 610\n"));
}

#[test]
fn reassigning_an_own_frees_the_old_value() {
    let src = "
fn main() {
    let mut a = own 1;
    a = own 2;
    print(*a)
}";
    assert_eq!(ok(src), "2\n");
}

#[test]
fn nested_own_is_freed_recursively() {
    let src = "
fn main() {
    let outer = own own 7;
    print(**outer)
}";
    assert_eq!(ok(src), "7\n");
}

#[test]
fn early_return_inside_loop_frees_locals() {
    let src = "
fn i64 find() {
    let mut i = 0;
    loop {
        let tmp = own i;
        if *tmp == 3 { return i; }
        i = i + 1;
    }
}
fn main() {
    print(find())
}";
    assert_eq!(ok(src), "3\n");
}

#[test]
fn temporaries_do_not_leak() {
    let src = "
fn main() {
    print(*own 5);
    own 6;
    let x = own 1;
    drop(x)
}";
    assert_eq!(ok(src), "5\n");
}

#[test]
fn ref_of_ref_points_at_the_same_cell() {
    let src = "
fn main() {
    let mut a = own 5;
    let r = ref mut a;
    let q = ref mut r;
    *q = 9;
    print(*a)
}";
    assert_eq!(ok(src), "9\n");
}

#[test]
fn else_if_chain() {
    let src = "
fn i64 sign(i64 n) {
    if n < 0 {
        return -1;
    } else if n == 0 {
        return 0;
    } else {
        return 1;
    }
}
fn main() {
    print(sign(-5), sign(0), sign(7))
}";
    assert_eq!(ok(src), "-1 0 1\n");
}

// ---------------------------------------------------------------- 安全性

#[test]
fn dangling_via_return_is_detected_with_timeline() {
    let src = example("dangling_return");
    let d = err(&src);
    assert!(d.title.contains("dangling"));
    let text = d.render("t.jw", &src);
    assert!(text.contains("ここで `a` を借りた"), "{}", text);
    assert!(text.contains("ここで解放された"), "{}", text);
    assert!(text.contains("ここで使ったが"), "{}", text);
}

#[test]
fn dangling_via_scope_end_has_ordered_timeline() {
    let src = example("dangling_scope");
    let d = err(&src);
    assert!(d.title.contains("dangling"));
    let primary = d.labels.iter().find(|l| l.primary).unwrap().span.line;
    let mut others: Vec<usize> = d.labels.iter().filter(|l| !l.primary).map(|l| l.span.line).collect();
    others.sort();
    // 借りた(7行) → 解放された(8行) → 使った(9行)
    assert_eq!(others, vec![7, 8]);
    assert_eq!(primary, 9);
}

#[test]
fn stale_ref_is_still_caught_after_slot_reuse() {
    // a のスロットは b に再利用される。世代がなければ r は b の中身を読んでしまう。
    let src = "
fn main() {
    let a = own 1;
    let r = ref a;
    drop(a);
    let b = own 2;
    print(*r)
}";
    let d = err(src);
    assert!(d.title.contains("dangling"));
}

#[test]
fn use_after_move_is_detected() {
    let src = example("use_after_move");
    let o = run_source(&src, false);
    assert_eq!(o.output, "1\n"); // consume までは実行される
    let d = o.error.expect("error");
    assert!(d.title.contains("use after move"));
    assert_eq!(d.labels.len(), 2);
}

#[test]
fn moving_inside_a_loop_is_detected_on_second_iteration() {
    let src = "
fn eat(own i64 c) {
}
fn main() {
    let a = own 1;
    let mut i = 0;
    while i < 2 {
        eat(a);
        i = i + 1;
    }
}";
    let d = err(src);
    assert!(d.title.contains("use after move"));
    assert_eq!(d.labels.len(), 1);
}

#[test]
fn cannot_move_an_own_out_through_a_pointer() {
    let src = "
fn main() {
    let outer = own own 1;
    let inner = *outer;
}";
    let d = err(src);
    assert!(d.title.contains("取り出せません"));
}

// ---------------------------------------------------------------- その他

#[test]
fn runaway_recursion_is_an_error_not_a_crash() {
    let src = "
fn i64 f(i64 n) {
    return f(n + 1);
}
fn main() {
    f(0)
}";
    let d = err(src);
    assert!(d.title.contains("深すぎ"));
}

#[test]
fn div_zero_is_an_error_and_overflow_wraps() {
    assert!(err("fn main() {\n print(1 / 0)\n}").title.contains("0 で割"));
    // あふれは一周する（C++ の未定義動作と違い、結果は決まっている）
    let src = "fn i64 big() {\n 9223372036854775807\n}\nfn main() {\n print(big() + 1, -(big() + 1) == big() + 1, big() * 2);\n}";
    let o = jabberwock::run_source(src, false);
    assert!(o.error.is_none());
    assert!(o.warnings.is_empty(), "values unknown before running give no warning");
    assert_eq!(o.output, "-9223372036854775808 true -2\n");
}

#[test]
fn parse_error_points_at_the_right_line() {
    let d = err("fn main() {\n    let = 5;\n}");
    let p = d.labels.iter().find(|l| l.primary).unwrap();
    assert_eq!(p.span.line, 2);
}

#[test]
fn renders_japanese_source_without_panicking() {
    let src = "fn main() {\n    let あ = own 1;\n    drop(あ);\n    print(*あ)\n}";
    let d = err(src);
    let text = d.render("t.jw", src);
    assert!(text.contains("use after move"));
}

// ---------------------------------------------------------------- 試しに書いたプログラム

#[test]
fn bst_example() {
    assert_eq!(ok(&example("bst")), "size 7: [1, 3, 4, 5, 7, 8, 9]\ntrue false\n");
}

#[test]
fn words_example() {
    assert_eq!(ok(&example("words")), "the: 3\ncat: 1\nand: 2\ndog: 1\nbird: 1\n");
}

#[test]
fn battle_example() {
    let out = ok(&example("battle"));
    assert!(out.starts_with("1: スライム の残り 3\n"), "{}", out);
    assert!(out.ends_with("ドラゴン を倒した\n"), "{}", out);
}

#[test]
fn parallel_words_example() {
    let out = ok(&example("parallel_words"));
    assert!(out.starts_with("the: 3\ndog: 2\nfox: 2\nquick: 2\na: 1\n"), "{}", out);
}

#[test]
fn calc_example() {
    assert_eq!(
        ok(&example("calc")),
        "1 + 2 * 3 = 7\n(1 + 2) * 3 = 9\n-4 + 10 / 2 = 1\n2 * (3 + : エラー: 式が途中で終わっています\n\
         8 / (4 - 4): エラー: 0 で割りました\n7 % 2: エラー: 知らない文字 '%'（2 文字目）\n"
    );
}

#[test]
fn shop_example() {
    let out = ok(&example("shop"));
    assert!(out.contains("売上 708 円\n"), "{}", out);
    assert!(out.ends_with("在庫が少ない: [りんご, みかん]\n"), "{}", out);
}

#[test]
fn deep_nesting_is_an_error_not_a_crash() {
    use jabberwock::parser::MAX_NEST;
    let parens = |n: usize| format!("fn main() {{ print({}1{}); }}", "(".repeat(n), ")".repeat(n));
    // fn の本体のブロック・print の引数・いちばん内側の 1 で 3 段
    assert_eq!(ok(&parens(MAX_NEST - 3)), "1\n");
    for src in [
        parens(MAX_NEST - 2),
        parens(100_000),
        format!("fn main() {{ print({}1); }}", "- ".repeat(100_000)),
        format!("fn main() {{ {}{} }}", "if true { ".repeat(100_000), "}".repeat(100_000)),
        format!("fn f({}i64 x) {{}}\nfn main() {{}}", "own ".repeat(100_000)),
        format!("fn main() {{ let x = {}1{}; }}", "[".repeat(100_000), "]".repeat(100_000)),
        format!("fn main() {{ match o {{ case {}x{} -> 1; }} }}", "Some(".repeat(100_000), ")".repeat(100_000)),
    ] {
        let d = err(&src);
        assert_eq!(d.title, "入れ子が深すぎます", "{}", &src[..40]);
    }
    // コマンドも（構文解析を大きなスタックで動かすので）エラーで止まる
    let dir = std::env::temp_dir().join(format!("jw-deep-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let f = dir.join("deep.jw");
    fs::write(&f, parens(100_000)).unwrap();
    let o = std::process::Command::new(env!("CARGO_BIN_EXE_jabberwock")).arg(&f).output().unwrap();
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("入れ子が深すぎます"));
    let _ = fs::remove_dir_all(&dir);
}
