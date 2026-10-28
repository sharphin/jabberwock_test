//! ファイルの読み書き・外部コマンド・`exit`・`eprint`・値なしの値 `()` のテスト
//! （デバッグビルドでは IR と、`JW_NATIVE=1` なら実行ファイルとも突き合わせる）

use jabberwock::run_source;

fn ok(src: &str) -> jabberwock::Outcome {
    let o = run_source(src, false);
    if let Some(d) = &o.error {
        panic!("unexpected error:\n{}", d.render("test.jw", src));
    }
    o
}

fn err(src: &str) -> jabberwock::diag::Diagnostic {
    run_source(src, false).error.expect("expected an error")
}

/// テストごとの作業用フォルダ（jabberwock の文字列にそのまま書ける形）
fn scratch(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("jw-system-test-{}-{}", std::process::id(), name));
    std::fs::create_dir_all(&d).unwrap();
    d.to_str().unwrap().replace('\\', "/")
}

#[test]
fn unit_value_in_result() {
    let o = ok("
fn Result<(), str> check(i64 n) {
    if n < 0 { return Err(\"負の数 {n}\"); }
    Ok(())
}
fn Result<(), str> all() {
    check(1)?;
    check(-2)?;
    print(\"ここには来ない\");
    Ok(())
}
fn main() {
    print(check(3), all());
    match check(5) {
        case Ok(u) -> print(\"ok\");
        case Err(e) -> print(e);
    }
}");
    assert_eq!(o.output, "Ok(()) Err(負の数 -2)\nok\n");
    assert_eq!(o.live_objects, 0);
}

#[test]
fn unit_value_is_not_a_variable() {
    assert!(err("fn main() { let u = (); }").title.contains("値"));
}

#[test]
fn write_then_read_a_file() {
    let dir = scratch("rw");
    let o = ok(&format!(
        "
fn Result<i64, str> go(str dir) {{
    let path = dir + \"/a.txt\";
    write_file(path, \"一行目\\n二行目\\n\")?;
    let t = read_file(path)?;
    print(t.lines());
    write_file(path, \"\")?;
    print(read_file(path)?.len());
    Ok(t.len())
}}
fn main() {{
    print(go(\"{dir}\"));
}}"
    ));
    assert_eq!(o.output, "[一行目, 二行目]\n0\nOk(8)\n");
    assert_eq!(o.live_objects, 0);
}

/// 項目10: Debug での AST/IR 差分検証が外部効果を二重実行しない。
/// 一回目の read_file -> write_file で x、二回実行なら xx になる。
#[test]
fn external_file_effect_runs_exactly_once() {
    let dir = scratch("effect-once");
    let path = format!("{dir}/effect.txt");
    std::fs::write(&path, "").unwrap();
    let o = ok(&format!(r#"
fn main() {{
    let before = read_file("{path}").unwrap_or("");
    write_file("{path}", before + "x");
}}
"#));
    assert_eq!(o.live_objects, 0);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "x");
}

#[test]
fn file_errors_are_values() {
    let dir = scratch("errs");
    std::fs::write(format!("{}/bad.txt", dir), [0x61u8, 0xff, 0x62]).unwrap();
    let o = ok(&format!(
        "
fn main() {{
    print(read_file(\"{dir}/ない.txt\"));
    print(read_file(\"{dir}/bad.txt\"));
    print(write_file(\"{dir}/ないフォルダ/a.txt\", \"x\"));
    print(read_file(\"{dir}\").is_err());
}}"
    ));
    assert_eq!(
        o.output,
        format!(
            "Err(ファイルを読めません: {d}/ない.txt)\nErr(ファイルが UTF-8 ではありません: {d}/bad.txt)\nErr(ファイルに書けません: {d}/ないフォルダ/a.txt)\ntrue\n",
            d = dir
        )
    );
}

#[cfg(unix)]
#[test]
fn run_a_command() {
    let dir = scratch("cmd");
    let o = ok(&format!(
        "
fn main() {{
    print(run_command(\"true\", []), run_command(\"sh\", [\"-c\", \"exit 3\"]));
    print(run_command(\"sh\", [\"-c\", \"printf '%s' \\\"$0\\\" > {dir}/arg.txt\", \"空白 と \\\"かっこ\\\"\"]));
    print(read_file(\"{dir}/arg.txt\"));
    print(run_command(\"no-such-command-jw\", [\"x\"]));
}}"
    ));
    assert_eq!(
        o.output,
        "Ok(0) Ok(3)\nOk(0)\nOk(空白 と \"かっこ\")\nErr(コマンドを起動できません: no-such-command-jw)\n"
    );
    assert_eq!(o.live_objects, 0);
}

#[test]
fn exit_stops_the_program() {
    let o = ok("
struct Big { i64[] xs; }
fn i64 pick(Option<i64> o) {
    match o {
        case Some(n) -> n;
        case None -> exit(7);
    }
}
fn main() {
    let b = own Big { xs: [1, 2] };
    print(pick(Some(3)));
    eprint(\"止まります\", b.xs.len());
    print(pick(None));
    print(\"ここには来ない\");
}");
    assert_eq!(o.output, "3\n");
    assert_eq!(o.errput, "止まります 2\n");
    assert_eq!(o.exit_code, Some(7));
}

#[test]
fn exit_from_another_thread() {
    let o = ok("
fn main() {
    let t = spawn(() -> {
        print(\"thread\");
        exit(2);
    });
    t.join();
    loop {}
}");
    assert_eq!(o.output, "thread\n");
    assert_eq!(o.exit_code, Some(2));
}

#[test]
fn eprint_goes_to_stderr() {
    let o = ok("
fn main() {
    print(\"out\");
    eprint(\"err\", 1, [2, 3]);
    eprint();
}");
    assert_eq!(o.output, "out\n");
    assert_eq!(o.errput, "err 1 [2, 3]\n\n");
    assert_eq!(o.exit_code, None);
}

#[test]
fn system_builtins_check_types() {
    assert!(err("fn main() { read_file(1); }").title.contains("型"));
    assert!(err("fn main() { exit(\"a\"); }").title.contains("型"));
    assert!(err("fn main() { run_command(\"a\", \"b\"); }").title.contains("型"));
    assert!(err("fn main() { write_file(\"a\"); }").title.contains("引数"));
    assert!(err("fn read_file() {} fn main() {}").title.contains("組み込み"));
}
