//! 文字列のメソッドと入力（`read_line()` / `args()`）のテスト

use jabberwock::{run_source, run_source_with, run_source_with_input};

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

fn main_ok(body: &str) -> String {
    ok(&format!("fn main() {{\n{}\n}}", body))
}

fn err(body: &str) -> jabberwock::diag::Diagnostic {
    run_source(&format!("fn main() {{\n{}\n}}", body), false).error.expect("expected an error")
}

#[test]
fn repeat_checks_the_length_before_allocating() {
    assert_eq!(main_ok("print(\"ab\".repeat(3), \"\".repeat(9223372036854775807), \"x\".repeat(0), \"|\");"), "ababab   |\n");
    let d = err("print(\"abc\".repeat(9223372036854775807));");
    assert_eq!(d.title, "くり返した文字列が長すぎます");
    assert_eq!(d.note.as_deref(), Some("文字列は 2^62 バイトまで"));
    assert_eq!(err("print(\"ab\".repeat(-1));").title, "負の回数はくり返せません");
}

#[test]
fn string_methods_count_characters() {
    let o = main_ok("
    let s = \"日本語テキスト\";
    print(s.len(), s.byte_len(), s.slice(0, 3), s.slice(3, 7), s.find(\"テ\"), s.find(\"x\"));
    print(s.chars());
    print(\"  a b  \".trim(), \"Hi\".to_upper(), \"Hi\".to_lower(), \"ab\".repeat(3));
    print(s.contains(\"語\"), s.starts_with(\"日本\"), s.ends_with(\"本\"));
    print(\"a-b-c\".replace(\"-\", \"+\"));");
    assert_eq!(o, "7 21 日本語 テキスト Some(3) None\n[日, 本, 語, テ, キ, ス, ト]\na b HI hi ababab\ntrue true false\na+b+c\n");
}

#[test]
fn split_lines_and_to_int() {
    let o = main_ok("
    let words = \"the cat  sat\".split(\" \");
    print(words, words.len());
    print(\"one\\ntwo\\n\".lines());
    print(\" 42 \".to_int(), \"-7\".to_int(), \"x\".to_int());
    let mut total = 0;
    for w in \"1,2,3\".split(\",\") {
        match w.to_int() {
            case Some(n) -> total += n;
            case None -> {}
        }
    }
    print(total);");
    assert_eq!(o, "[the, cat, , sat] 4\n[one, two]\nSome(42) Some(-7) None\n6\n");
}

#[test]
fn methods_on_borrowed_strings() {
    let o = ok("
fn i64 count(ref str s) {
    s.len()
}

fn main() {
    let s = \"abc\";
    let r = ref s;
    print(count(s), r.len(), r.to_upper());
}");
    assert_eq!(o, "3 3 ABC\n");
}

#[test]
fn string_errors() {
    assert!(err("print(\"a\".nope());").title.contains("メソッド `nope` はありません"));
    assert!(err("print(\"a\".split(1));").title.contains("型が合いません"));
    assert!(err("print(\"abc\".slice(1, 5));").title.contains("範囲の外"));
    assert!(err("print(\"abc\".split(\"\"));").title.contains("空の文字列"));
    assert!(err("print(\"abc\".len(1));").title.contains("引数"));
}

// ---------------------------------------------------------------- 入力

#[test]
fn read_line_until_the_end() {
    let src = "
fn main() {
    let mut n = 0;
    loop {
        match read_line() {
            case None -> break;
            case Some(line) -> {
                n += 1;
                print(\"{n}: {line}\");
            }
        }
    }
    print(read_line());
}";
    let o = run_source_with_input(src, "first\r\nsecond\nlast", &[]);
    assert!(o.error.is_none(), "{:?}", o.error.map(|d| d.title));
    assert_eq!(o.output, "1: first\n2: second\n3: last\nNone\n");
    assert_eq!(o.live_objects, 0);
}

#[test]
fn args_are_a_list_of_strings() {
    let src = "
fn main() {
    let a = args();
    print(a.len(), a);
}";
    let o = run_source_with_input(src, "", &["x", "日本"]);
    assert_eq!(o.output, "2 [x, 日本]\n");
    // 引数がなければ空のリスト、入力がなければ None
    assert_eq!(ok("fn main() {\n    print(args(), read_line());\n}"), "[] None\n");
}

#[test]
fn builtin_input_errors() {
    assert!(err("print(read_line(1));").title.contains("引数"));
    let d = run_source("fn args() {\n}\nfn main() {\n}", false).error.expect("error");
    assert!(d.title.contains("組み込み関数"), "{}", d.title);
}

#[test]
fn bytes_and_char_codes() {
    let o = main_ok("
    print(\"aé語\".bytes(), \"\".bytes().len());
    print(\"a\".code(), \"é\".code(), \"語\".code(), \"😀\".code(), \"ab\".code());
    print(chr(97), chr(233), chr(35486), chr(128512), chr(0).byte_len());
    let mut n = 0;
    for c in \"x9_\".chars() {
        let k = c.code();
        if k >= 48 && k <= 57 { n += 1; }
    }
    let u8[] b = \"AZ\".bytes();
    print(n, b[0] + 1, b[1] as i64 - 65);");
    assert_eq!(o, "[97, 195, 169, 232, 170, 158] 0\n97 233 35486 128512 97\na é 語 😀 1\n1 66 25\n");
}

#[test]
fn bad_char_codes_are_errors() {
    assert!(err("print(\"\".code());").title.contains("空の文字列"));
    assert!(err("print(chr(55296));").title.contains("文字の番号"));
    assert!(err("print(chr(-1));").title.contains("文字の番号"));
    assert!(err("print(chr(1114112));").title.contains("文字の番号"));
    assert!(err("print(chr(\"a\"));").title.contains("型"));
}

#[test]
fn appending_to_strings() {
    // `s += x` はその場で足すことがある。ほかに同じ文字列を持っているもの（コピー・リスト・ref）は変わらない
    let o = ok("
struct P { str name; }
fn str add(str s) {
    let mut t = s;
    t += \"!\";
    t
}
fn main() {
    let mut s = \"a\";
    let keep = s;
    s += \"b\";
    s = s + s;
    s += \"{s.len()}\";
    let r = ref s;
    s += \"c\";
    print(s, keep, *r, add(s), s);
    let str[] mut v = [];
    let mut u = \"x\";
    for i in range(3) {
        v.push(u);
        u += \"y\";
    }
    let mut p = P { name: \"p\" };
    p.name += \"q\";
    let str[str] mut m = [\"k\": \"v\"];
    m[\"k\"] += \"w\";
    print(v, u, p.name, m);
    let mut big = \"\";
    for i in range(5000) {
        big += \"0123456789\";
    }
    print(big.byte_len(), big.slice(49990, 50000));
}");
    assert_eq!(o, "abab4c a abab4c abab4c! abab4c\n[x, xy, xyy] xyyy pq [k: vw]\n50000 0123456789\n");
}

#[test]
fn more_escapes() {
    let o = main_ok("
    print(\"a\\rb\".bytes(), \"x\\0y\".bytes(), \"\\u{41}\\u{3042}\\u{1F600}\", \"\\u{41}{1 + 1}\");");
    assert_eq!(o, "[97, 13, 98] [120, 0, 121] Aあ😀 A2\n");
    assert!(err("print(\"\\q\");").title.contains("エスケープ"));
    assert!(err("print(\"\\u{D800}\");").title.contains("エスケープ"));
    assert!(err("print(\"\\u{zz}\");").title.contains("エスケープ"));
    assert!(err("print(\"\\u{41\");").title.contains("エスケープ"));
}
