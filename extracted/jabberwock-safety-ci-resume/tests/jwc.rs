//! jwc（jwc/ にある、jabberwock で書いた jabberwock のコンパイラ）を Rust 版と突き合わせる。
//! jwc は clang で実行ファイルにして動かす（clang がなければ何もしない）。

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::Instant;

use jabberwock::{llvm_build, native};

#[path = "support/retirement.rs"]
mod retirement_guard;

#[test]
fn jwc_generated_checks_reject_a_matching_retired_generation() {
    if !have_clang() {
        assert!(std::env::var("JW_REQUIRE_RETIREMENT_TESTS").is_err(), "退役のjwc検証にclangが必要です");
        return;
    }
    let source = std::env::temp_dir().join(format!("jw-retirement-jwc-{}.jw", std::process::id()));
    std::fs::write(&source, retirement_guard::SOURCE).unwrap();
    for mode in ["--ll", "--ll-checked"] {
        let ll = run_jwc(&[mode, source.to_str().unwrap()]);
        retirement_guard::check_retired_marker(&ll, if mode == "--ll" { "jwc-elided" } else { "jwc-checked" });
    }
    std::fs::remove_file(source).unwrap();
}

fn have_clang() -> bool {
    let available = Command::new(native::clang()).arg("--version").output().map(|o| o.status.success()).unwrap_or(false);
    assert!(available || std::env::var("JW_SAFETY_STRICT").is_err(), "required jwc tests cannot skip clang");
    available
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Rust 版を大きなスタックで動かす（デバッグビルドは、長い式（llvm.jw の PRELUDE）でテストのスレッドのスタックがあふれる）
fn big<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new().stack_size(256 << 20).spawn(f).unwrap().join().unwrap()
}

/// jwc の実行ファイル（テストの中で 1 回だけ作る）
fn jwc() -> PathBuf {
    if let Ok(path) = std::env::var("JW_SAFETY_JWC") {
        let path = PathBuf::from(path);
        assert!(path.is_file(), "selected jwc executable missing: {}", path.display());
        return path;
    }
    static EXE: Mutex<Option<PathBuf>> = Mutex::new(None);
    let mut g = EXE.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(|| {
        let dir = root().join("jwc");
        let src = std::fs::read_to_string(dir.join("main.jw")).unwrap();
        let read = move |name: &str| std::fs::read_to_string(dir.join(format!("{}.jw", name))).ok();
        let start = Instant::now();
        let ll = big(move || llvm_build(&src, &read, "jwc/main.jw", false)).expect("jwc を LLVM に書き出せる");
        eprintln!("JWC_BUILD_PHASE llvm_emission elapsed_ms={} ir_bytes={}", start.elapsed().as_millis(), ll.len());
        let out = std::env::temp_dir().join(format!("jw-jwc-test-{}", std::process::id()));
        // The parity integration tests exercise compiler behavior, not optimized
        // code generation. Build the very large self-hosted compiler at -O1 without LTO;
        // optimized native output remains covered by native/safety tests.
        let opt = std::env::var("JW_JWC_TEST_OPT").unwrap_or_else(|_| "-O1".to_owned());
        let start = Instant::now();
        let exe = native::compile(&ll, &out, "jwc", &opt).expect("jwc を clang でコンパイルできる");
        eprintln!("JWC_BUILD_PHASE clang_link elapsed_ms={} opt={}", start.elapsed().as_millis(), opt);
        exe
    })
    .clone()
}

fn run_jwc(args: &[&str]) -> String {
    let out = Command::new(jwc()).args(args).output().expect("jwc を起動できる");
    assert!(out.status.success(), "jwc command failed: {:?}: {}", out.status, String::from_utf8_lossy(&out.stderr));
    assert!(!out.stdout.is_empty(), "jwc の出力がありません: {:?}: {}", out.status, String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// `--tokens` の出力が Rust 版（`token_dump`）と同じか
fn same_tokens(path: &Path) {
    let src = std::fs::read_to_string(path).unwrap();
    let name = path.to_str().unwrap();
    let want = jabberwock::token_dump(&src, name) + "\n";
    assert_eq!(run_jwc(&["--tokens", name]), want, "{}", name);
}

fn jw_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension() == Some(std::ffi::OsStr::new("jw")))
        .collect();
    v.sort();
    v
}

#[test]
fn tokens_of_the_repository_files() {
    if !have_clang() {
        return;
    }
    for dir in ["examples", "jwc"] {
        for p in jw_files(&root().join(dir)) {
            same_tokens(&p);
        }
    }
}

#[test]
fn tokens_and_lexer_errors() {
    if !have_clang() {
        return;
    }
    let cases = [
        "let x = 0xFF_ff + 0b1010 + 0o17;",
        "let y = 0xZZ;",
        "let z = 0x;",
        "let big = 18446744073709551615;",
        "let bigger = 18446744073709551616;",
        "let h = 0x1_0000_0000_0000_0000;",
        "let f = 1.5 + 1_0.25 + 3.x + 1_;",
        "let s = \"a\\tb\\n\\r\\0\\\"\\\\ \\u{1F600} \\u{41}\";",
        "let s = \"bad \\q\";",
        "let s = \"bad \\u{110000}\";",
        "let s = \"bad \\u{D800}\";",
        "let s = \"bad \\u{}\";",
        "let s = \"bad \\u{41\";",
        "let s = \"open\nnext",
        "let s = \"x = {x}, {a + b} {{lit}} {f(1, 2)}\";",
        "let s = \"{x\";",
        "let s = \"{ }\";",
        "let s = \"a } b\";",
        "let s = \"{x \"y\"}\";",
        "let s = \"\"\"\nmulti\nline {x}\n\"\"\" ;",
        "let s = \"\"\"never closed\n",
        "let 名前 = 3; // コメント\tタブ",
        "\tlet a = b @ c;",
        "let a = b << 2 >> 1 && c || d -> e += f -= g *= h /= i %= j ++ -- != == <= >= & | ^ ~ ? !;",
        "let s = \"日本語{x}語\";",
        "",
    ];
    let dir = std::env::temp_dir().join(format!("jw-jwc-cases-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (i, c) in cases.iter().enumerate() {
        let p = dir.join(format!("case{}.jw", i));
        std::fs::write(&p, c).unwrap();
        same_tokens(&p);
    }
}

/// `--ast` の出力が Rust 版（`ast_dump`）と同じか。モジュールは同じフォルダから読む
fn same_ast(path: &Path) {
    let src = std::fs::read_to_string(path).unwrap();
    let name = path.to_str().unwrap();
    let dir = path.parent().unwrap().to_path_buf();
    let read = move |m: &str| std::fs::read_to_string(dir.join(format!("{}.jw", m))).ok();
    let n = name.to_string();
    let want = big(move || jabberwock::ast_dump(&src, &read, &n)) + "\n";
    assert_eq!(run_jwc(&["--ast", name]), want, "{}", name);
}

#[test]
fn ast_of_the_repository_files() {
    if !have_clang() {
        return;
    }
    for dir in ["examples", "jwc"] {
        for p in jw_files(&root().join(dir)) {
            same_ast(&p);
        }
    }
}

#[test]
fn ast_and_parser_errors() {
    if !have_clang() {
        return;
    }
    let cases = [
        // 書き方いろいろ
        "fn Option<i64> pick<T: Show + ordered, U>(ref mut T a, U mut b) { a >> 2; a > > b; return None; }",
        "fn main() { let f = (a, i64 b) -> a + b; let g = (x) -> { x }; apply((y) -> y, 1); spawn(() -> 1); f(()); }",
        "fn main() { let m = [\"a\": 1, \"b\": 2,]; let a = [0; 5]; let b = [1, 2,]; let c = (1 + 2) as u8 as f64; }",
        "fn main() { match x { case Some(Circle(r, _)), None -> 1; case -1, -0, 0 -> 2; case \"s\" -> {} default -> 3; } }",
        "fn main() { for (k, v) in m { continue; } else { x++; } for i in range(1, 2, 3) {} else {} s.f += 1; a[i] -= 2; *p *= 3; }",
        "fn main() { let x = if a { 1 } else if b { 2 } else { 3 }; let y = match b { case true -> 1; case false -> 2; }; }",
        "fn i64 f() { let x = match o { case Some(v) -> v; case None -> return 0; }; while x < 3 && !done { loop { break; } } x }",
        "fn main() { let s = \"x = {a.b(c)}, {Point { x: 1 }.x} {-n}\"; let t = ref mut *p; let u = own Point { x: 1, y: 2 }; f()?.g?; }",
        "fn main() { let fn i64(i64, str) mut f = g; let (own i64)[] a = []; let i64[str][] b = []; let fn(i64) h = k; }",
        "struct P<T> { T x; i64 y;\n fn i64 get(i64 x) { x + y + helper(1) + mk().y + (() -> y)() }\n fn mut set() { let y = 1; x = y; match o { case Some(y) -> y; case None -> y; } }\n fn take P<T> me() { this }\n static fn P<i64> mk() { P { x: 1, y: 2 } }\n fn i64 helper(i64 n) { for y in a { y; } y }\n}\nfn main() { let p = P.mk(); }",
        "enum Shape { Circle(i64 r); Empty;\n fn i64 area() { match this { case Circle(r) -> r; case Empty -> zero(); } }\n static fn i64 zero() { 0 }\n static fn Shape make() { Shape.Circle(1) }\n}\nfn main() { let s = Shape.make(); let t = Shape.Empty; let u = Shape.Circle(2); }",
        "const N = 3;\nconst M = N * 2 + -1;\nconst S = \"s\";\nconst F = 1.5 as f64;\nfn main() { let x = N + M; print(S, F, \"{N}\"); }",
        "interface Shape { fn i64 area(); fn mut grow(i64 n); }\npub fn main() {}",
        // 構文エラー
        "interface I<T> { fn f(); }",
        "interface I { fn f<T>(); }",
        "interface I { fn f(); fn f(); }",
        "interface I { fn f() {} }",
        "struct S { fn f() {} i64 x; }",
        "struct S { i64 x, i64 y; }",
        "struct S { x; }",
        "struct S { i64 x; i64 x; }",
        "struct S { i64 x; fn x() {} }",
        "struct S<T> { fn g<T>() {} }",
        "struct S<T: I> { T x; }",
        "struct S { fn f() {} static fn f() {} }",
        "enum E { A; fn f() {} fn f() {} }",
        "enum E { A(i64 x, i64 x); }",
        "enum E { A(x); }",
        "enum E { A, B }",
        "enum E { A; A; }",
        "enum E { fn f() {} A; }",
        "enum E { A; fn A() {} }",
        "fn f<T, T>() {}",
        "fn f(x) {}",
        "fn f(mut i64 x) {}",
        "pub",
        "pub const N = 1;",
        "const N = 1; const N = 2;",
        "const N = M; const M = 1;",
        "const N = f(1);",
        "const N = 1; fn N() {}",
        "let x = 1;",
        "fn main() { 1 = 2; }",
        "fn main() { let x = 1 }",
        "fn main() { let x = 1 let y = 2; }",
        "fn main() {",
        "fn main() { let x = match 1 { case 1 -> let y = 2; default -> 3; }; }",
        "fn main() { let x = match 1 { case 1 -> 2; }; }",
        "fn main() { let x = if a { 1 }; }",
        "fn main() { let x = if a { 1; } else { 2 }; }",
        "fn main() { match x { case 1 -> 1; case 1 -> 2; } }",
        "fn main() { match x { case Some(a) -> 1; case Some(b) -> 2; } }",
        "fn main() { match x { case 1, 1 -> 1; } }",
        "fn main() { match x { default -> 1; default -> 2; } }",
        "fn main() { match x { case 1.5 -> 1; } }",
        "fn main() { match x { case - a -> 1; } }",
        "fn main() { match x { case 1 -> 1 } }",
        "fn main() { match x { 1 -> 1; } }",
        "fn main() { break; }",
        "fn main() { continue; }",
        "fn main() { loop { let x = if a { break; 1 } else { 2 }; } }",
        "fn main() { for i in range() {} }",
        "fn main() { for i in range(1, 2, 3, 4) {} }",
        "fn main() { for (i, x) in range(3) {} }",
        "fn main() { for i in a b {} }",
        "fn main() { Point.x; }",
        "fn main() { let s = \"{a b}\"; }",
        "fn main() { let s = \"{break}\"; }",
        "fn main() { let x = ; }",
        "fn main() { let i64 = 1; }",
        "fn main() { let x = [1; n]; }",
        "fn main() { let x = a.; }",
        "fn main() { f(1, 2; }",
        "fn main() { let x = (1 + 2; }",
        "fn main() { let x = 1 as; }",
        "fn main() { let i64[x y] a = 1; }",
    ];
    let dir = std::env::temp_dir().join(format!("jw-jwc-ast-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // 入れ子の深さの上限（ちょうど上限まではよく、超えるとエラー）
    let deep = |n: usize| format!("fn main() {{ print({}1{}); }}", "(".repeat(n), ")".repeat(n));
    let mut cases: Vec<String> = cases.iter().map(|c| c.to_string()).collect();
    cases.push(deep(jabberwock::parser::MAX_NEST - 3));
    cases.push(deep(jabberwock::parser::MAX_NEST - 2));
    cases.push(deep(10000));
    cases.push(format!("fn f({}i64 x) {{}}", "own ".repeat(10000)));
    cases.push(format!("fn main() {{ match o {{ case {}x{} -> 1; }} }}", "Some(".repeat(10000), ")".repeat(10000)));
    cases.push(format!("fn main() {{ {}{} }}", "if a { ".repeat(10000), "}".repeat(10000)));
    cases.push(format!("fn main() {{ print(1{}); }}", " + 1".repeat(10000)));
    cases.push(format!("fn main() {{ print(x{}); }}", ".abs()".repeat(10000)));
    cases.push(format!("fn main() {{ print(x{}); }}", " as u8".repeat(10000)));
    cases.push(format!("fn main() {{ print(1{}); }}", " + 1".repeat(200)));
    for (i, c) in cases.iter().enumerate() {
        let p = dir.join(format!("case{}.jw", i));
        std::fs::write(&p, c).unwrap();
        same_ast(&p);
    }
}

#[test]
fn ast_with_modules() {
    if !have_clang() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("jw-jwc-mods-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let files = [
        ("geom", "use util;\nconst K = 2;\npub struct Point { i64 x; i64 y;\n fn i64 len2() { util.sq(x) + util.sq(y) + K }\n static fn Point at(i64 x, i64 y) { Point { x: x, y: y } }\n}\npub enum Shape { Dot(Point p); Box(i64 w, i64 h);\n static fn Shape square(i64 n) { Shape.Box(n, n) }\n static fn Shape unit() { square(1) }\n}\npub interface Sized { fn i64 size(); }\npub fn i64 area(ref Shape s) { match s { case Dot(_) -> 0; case Box(w, h) -> helper(w, h); } }\nfn i64 helper(i64 a, i64 b) { a * b }\npub fn i64 apply<T: Sized>(fn i64(i64) f, ref T t, Point p) { let Shape[] a = []; let g = helper; f(t.size()) }\n"),
        ("util", "pub fn i64 sq(i64 x) { x * x }\nfn hidden() {}\n"),
        ("loop1", "use loop2;\npub fn f() {}\n"),
        ("loop2", "use loop1;\npub fn g() {}\n"),
        ("bad", "fn main() { let x = ; }\n"),
        ("ok_main", "use geom;\nuse util;\nfn main() { let p = geom.Point.at(1, 2); let s = geom.Shape.square(2); print(\"{geom.area(s)} {util.sq(3)}\", p.len2()); let q = geom.Point { x: 1, y: 2 }; let geom.Shape[] v = [geom.Shape.Box(1, 2)]; let w = W { geom: p }; print(w.geom.x); }\nstruct W { geom.Point geom; }\n"),
        ("missing", "use nothere;\nfn main() {}\n"),
        ("cycle", "use loop1;\nfn main() {}\n"),
        ("not_pub", "use util;\nfn main() { util.hidden(); }\n"),
        ("no_name", "use util;\nfn main() { util.nope(); }\n"),
        ("dot_end", "use util;\nfn main() { util.; }\n"),
        ("twice", "use util;\nuse util;\nfn main() {}\n"),
        ("bad_mod", "use bad;\nfn main() {}\n"),
    ];
    for (n, s) in files {
        std::fs::write(dir.join(format!("{}.jw", n)), s).unwrap();
    }
    for n in ["ok_main", "missing", "cycle", "not_pub", "no_name", "dot_end", "twice", "bad_mod"] {
        same_ast(&dir.join(format!("{}.jw", n)));
        same_typed(&dir.join(format!("{}.jw", n)));
        same_irs(&dir.join(format!("{}.jw", n)));
        same_ll(&dir.join(format!("{}.jw", n)));
    }
}

/// 解決済みの依存をjwcが読み、空間・pub・直接依存をキャッシュ後も守る。
#[test]
fn local_packages_with_jwc() {
    if !have_clang() { return; }
    let dir = std::env::temp_dir().join(format!("jw-jwc packages-日本語-{}", std::process::id()));
    let write = |path: &str, text: &str| {
        let p = dir.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    };
    write("app/alice.toml", "[package]\nname = \"app\"\n[dependencies]\nshapes = { path = \"../shapes\" }\n");
    write("shapes/alice.toml", "[package]\nname = \"shapes\"\n[dependencies]\nnums = { path = \"../nums\" }\n");
    write("nums/alice.toml", "[package]\nname = \"nums\"\n");
    write("app/src/util.jw", "pub fn str who() { \"app\" }");
    write("shapes/src/util.jw", "pub fn str who() { \"shapes\" } fn hidden() {}\npub struct Point<T> { T x; }");
    write("nums/src/lib.jw", "pub fn i64 zero() { 0 }");
    write("shapes/src/lib.jw", "use util; use nums; pub fn str who() { util.who() }\npub fn i64 value() { let p = util.Point { x: 9 }; p.x + nums.zero() }");
    write("app/src/main.jw", "use shapes; use util; fn main() { print(util.who(), shapes.who(), shapes.value()); }");
    let project = jabberwock::project::Project::load(&dir.join("app")).unwrap();
    let map = dir.join("packages.map");
    std::fs::write(&map, project.package_map(&project.main_file()).unwrap()).unwrap();
    let run = |mode: &str| Command::new(jwc()).args([mode, "--package-map"]).arg(&map).output().unwrap();
    let source = std::fs::read_to_string(project.main_file()).unwrap();
    let reader = project.reader();
    let want = big(move || jabberwock::ir_dump(&source, &reader, "main.jw", true));
    let got = run("--ir");
    assert!(got.status.success(), "{}", String::from_utf8_lossy(&got.stderr));
    assert_eq!(String::from_utf8_lossy(&got.stdout).trim_end(), want.trim_end());
    let exe = dir.join("app-native");
    let output = Command::new(jwc()).args(["build", "--package-map"]).arg(&map)
        .args(["--clang", &native::clang(), "-o"]).arg(&exe).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let output = Command::new(exe).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "app shapes 9\n");
    // shapesを先に読んでnumsがキャッシュにあっても、appからは使えない。
    write("app/src/main.jw", "use shapes; use nums; fn main() { print(nums.zero()); }");
    let out = run("--ll");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("直接依存"));
    write("app/src/main.jw", "use shapes; fn main() { print(shapes.who()); }");
    write("shapes/src/lib.jw", "use util; pub fn str who() { util.hidden(); \"x\" }");
    let out = run("--ll");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("pub ではありません"));
    write("shapes/src/lib.jw", "use util; pub fn str who() { util.who() }");
    write("shapes/src/util.jw", "use shapes; pub fn str who() { \"cycle\" }");
    // shapesの直接依存へ自分を加えた不正なマップでも、循環を診断する。
    let text = std::fs::read_to_string(&map).unwrap() + "allow\tshapes\tshapes\n";
    std::fs::write(&map, text).unwrap();
    let out = run("--ll");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("輪"));
    std::fs::write(&map, "jwc-package-map-v99\n").unwrap();
    let out = run("--ll");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("対応していない"));
    let _ = std::fs::remove_dir_all(dir);
}

/// Rust 版（凍結）が、古い名前の決まり（モジュールの中の変数に、最初のファイルの関数と同じ名前をつけられない）だけで断るファイルか。
/// jwc では名前をモジュールごとに閉じたので、このファイルは比べられない（docs/design-decisions.md）。jwc が通すことだけ確かめる
fn rust_rejects_by_old_name_rule(path: &Path) -> bool {
    let src = std::fs::read_to_string(path).unwrap();
    let name = path.to_str().unwrap();
    let dir = path.parent().unwrap().to_path_buf();
    let read = move |m: &str| std::fs::read_to_string(dir.join(format!("{}.jw", m))).ok();
    let n = name.to_string();
    let want = big(move || jabberwock::typed_dump(&src, &read, &n));
    let old_rule = want.starts_with("error: `") && want.contains("変数や引数に、関数と同じ名前はつけられない");
    if old_rule {
        assert!(!run_jwc(&["--typed", name]).starts_with("error"), "jwc も {} を断りました", name);
    }
    old_rule
}

/// `--typed` の出力が Rust 版（`typed_dump`）と同じか
fn same_typed(path: &Path) {
    let src = std::fs::read_to_string(path).unwrap();
    let name = path.to_str().unwrap();
    let dir = path.parent().unwrap().to_path_buf();
    let read = move |m: &str| std::fs::read_to_string(dir.join(format!("{}.jw", m))).ok();
    let n = name.to_string();
    let want = big(move || jabberwock::typed_dump(&src, &read, &n)) + "\n";
    assert_eq!(run_jwc(&["--typed", name]), want, "{}", name);
}

/// IR（`--ir-raw` は解析の前、`--ir` は検査を消す解析のあと）が Rust 版と同じか
fn same_irs(path: &Path) {
    same_ir(path, false);
    same_ir(path, true);
}

/// `--ll` の出力が Rust 版（`llvm_text`）と同じか。書けないときは、どちらも標準エラーに同じ文を出す
fn same_ll(path: &Path) {
    let src = std::fs::read_to_string(path).unwrap();
    let name = path.to_str().unwrap();
    let dir = path.parent().unwrap().to_path_buf();
    let read = move |m: &str| std::fs::read_to_string(dir.join(format!("{}.jw", m))).ok();
    let out = Command::new(jwc()).args(["--ll", name]).output().expect("jwc を起動できる");
    let n = name.to_string();
    let want = big(move || jabberwock::llvm_text(&src, &read, &n));
    match want {
        Ok(want) => assert_eq!(String::from_utf8_lossy(&out.stdout), want, "{}", name),
        Err(e) => assert_eq!(String::from_utf8_lossy(&out.stderr), e + "\n", "{}", name),
    }
}

fn same_ir(path: &Path, analyze: bool) {
    let src = std::fs::read_to_string(path).unwrap();
    let name = path.to_str().unwrap();
    let dir = path.parent().unwrap().to_path_buf();
    let read = move |m: &str| std::fs::read_to_string(dir.join(format!("{}.jw", m))).ok();
    let n = name.to_string();
    let want = big(move || jabberwock::ir_dump(&src, &read, &n, analyze));
    let got = run_jwc(&[if analyze { "--ir" } else { "--ir-raw" }, name]);
    assert_eq!(got.strip_suffix('\n').unwrap_or(&got), want.strip_suffix('\n').unwrap_or(&want), "{}", name);
}

#[test]
fn typed_and_ir_of_the_repository_files() {
    if !have_clang() {
        return;
    }
    for dir in ["examples", "jwc"] {
        for p in jw_files(&root().join(dir)) {
            if rust_rejects_by_old_name_rule(&p) { continue; }
            same_typed(&p);
            same_irs(&p);
            same_ll(&p);
        }
    }
}

/// テストで動かすプログラム（tests/jwc_cases.txt に `#####` の行で区切って集めたもの。型のエラーも含む）
#[test]
fn ast_typed_and_ir_of_the_test_programs() {
    if !have_clang() {
        return;
    }
    let all = std::fs::read_to_string(root().join("tests/jwc_cases.txt")).unwrap();
    let dir = std::env::temp_dir().join(format!("jw-jwc-progs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (i, c) in all.split("#####\n").enumerate() {
        let p = dir.join(format!("case{}.jw", i));
        std::fs::write(&p, c).unwrap();
        same_ast(&p);
        same_typed(&p);
        same_irs(&p);
        same_ll(&p);
    }
}

/// jwc/runtime.jw（実行時ライブラリを jabberwock の文字列にしたもの）の中身。runtime/jw_rt.c から作る
fn runtime_jw() -> String {
    let body = native::RUNTIME_C.replace('\\', "\\\\").replace('{', "{{").replace('}', "}}");
    format!(
        "// 実行時ライブラリ（runtime/jw_rt.c）を文字列にしたもの。`jwc build` が書き出して一緒にコンパイルする。\n\
         // 手で書き換えない: runtime/jw_rt.c を直したら `python3 scripts/sync_runtime.py` で作り直す。\n\
         pub fn str source() {{\n    \"\"\"\n{}\"\"\"\n}}\n",
        body
    )
}

#[test]
fn runtime_is_up_to_date() {
    let path = root().join("jwc/runtime.jw");
    let want = runtime_jw();
    if std::env::var("JWC_REGEN").is_ok() {
        std::fs::write(&path, &want).unwrap();
    }
    let got = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(got == want, "jwc/runtime.jw が runtime/jw_rt.c と合っていません。`python3 scripts/sync_runtime.py` で作り直してください");
}

/// scripts/sync_stdlib.py と同じ形で std/*.jw を埋め込んだ jwc/stdlib.jw
fn stdlib_jw() -> String {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(root().join("std")).unwrap().map(|e| e.unwrap().path())
        .filter(|p| p.extension() == Some(std::ffi::OsStr::new("jw"))).collect();
    paths.sort();
    let mut names = String::new();
    let mut bodies = String::new();
    for p in &paths {
        let name = p.file_stem().unwrap().to_str().unwrap();
        let src = std::fs::read_to_string(p).unwrap().replace("\r\n", "\n");
        let body = src.replace('\\', "\\\\").replace('{', "{{").replace('}', "}}");
        names += &format!("    out.push(\"{}\");\n", name);
        bodies += &format!("    if name == \"{}\" {{\n        return Some(\"\"\"\n{}\"\"\");\n    }}\n", name, body);
    }
    format!(
        "// 標準ライブラリ（std/*.jw）を文字列にしたもの。`use std.名前;` で modules.jw が読む。\n\
         // 手で書き換えない: std/ を直したら `python3 scripts/sync_stdlib.py` で作り直す。\n\
         pub fn str[] names() {{\n    let str[] mut out = [];\n{}    out\n}}\n\npub fn Option<str> source(str name) {{\n{}    None\n}}\n",
        names, bodies
    )
}

#[test]
fn stdlib_is_up_to_date() {
    let path = root().join("jwc/stdlib.jw");
    let want = stdlib_jw();
    if std::env::var("JWC_REGEN").is_ok() {
        std::fs::write(&path, &want).unwrap();
    }
    let got = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(got == want, "jwc/stdlib.jw が std/ と合っていません。`python3 scripts/sync_stdlib.py` で作り直してください");
}

/// `use std.名前;` の標準ライブラリ（jwc だけの機能なので、期待する出力を書いて確かめる）
#[test]
fn jwc_standard_library() {
    if !have_clang() { return; }
    let dir = std::env::temp_dir().join(format!("jw-jwc-std-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let build = |name: &str, src: &str| {
        let path = dir.join(format!("{}.jw", name));
        std::fs::write(&path, src).unwrap();
        let exe = dir.join(name);
        let out = Command::new(jwc()).args(["build", "--clang", &native::clang(), "-o"]).arg(&exe).arg(&path).output().unwrap();
        (out, exe)
    };
    let (out, exe) = build("main", "use std.math;\nuse std.text;\nfn main() {\n    \
        print(math.gcd(12, -18), math.lcm(4, 6), math.pow(2, 10), math.clamp(15, 0, 10));\n    \
        print(math.floor(-2.5), math.ceil(2.1), math.round(2.5), math.round(-2.5), math.round(0.49999999999999994));\n    \
        print(text.pad_left(\"7\", 3, \"0\"), text.pad_right(\"ab\", 4, \".\"), text.reverse(\"日本語\"), text.count(\"abababa\", \"aba\"), text.is_digits(\"12\"), text.is_digits(\"\"));\n}\n");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let got = Command::new(&exe).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&got.stdout), "6 12 1024 10\n-3.0 3.0 3.0 -3.0 0.0\n007 ab.. 語本日 2 true false\n");
    // 同じフォルダの math.jw は標準の math を置き換えない（両方を use するとエラー）
    std::fs::write(dir.join("math.jw"), "pub fn i64 gcd(i64 a, i64 b) { 0 }\n").unwrap();
    let (out, _) = build("both", "use math;\nuse std.math;\nfn main() {}\n");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("`math` という名前のモジュールを二つ使っています"));
    let (out, _) = build("nope", "use std.nope;\nfn main() {}\n");
    assert!(String::from_utf8_lossy(&out.stderr).contains("標準ライブラリに `nope` はありません"));
    let _ = std::fs::remove_dir_all(dir);
}

/// std.fs の File（資源）。正しい使い方、わざと失敗させたとき、誤用がはじかれること
#[test]
fn jwc_std_fs() {
    if !have_clang() { return; }
    let dir = std::env::temp_dir().join(format!("jw-jwc-fs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let build = |name: &str, src: &str| {
        let path = dir.join(format!("{}.jw", name));
        std::fs::write(&path, src).unwrap();
        let exe = dir.join(name);
        let out = Command::new(jwc()).args(["build", "--clang", &native::clang(), "-o"]).arg(&exe).arg(&path).output().unwrap();
        (out, exe)
    };
    let (out, exe) = build("main", "use std.fs;\n\
        fn Result<(), str> run() {\n    \
            let mut f = fs.create(\"t.txt\")?;\n    f.write_text(\"日本\\n\")?;\n    f.write([65, 66])?;\n    fs.close(f)?;\n    \
            let mut a = fs.append(\"t.txt\")?;\n    a.write_text(\"!\")?;\n    fs.close(a)?;\n    \
            let mut r = fs.open(\"t.txt\")?;\n    print(r.read(3)?);\n    print(r.read_text()?, r.read(8)?.len(), r);\n    \
            match fs.open(\"nai.txt\") { case Ok(x) -> print(\"?\"); case Err(e) -> print(e); }\n    \
            fs.make_dir(\"d\")?;\n    fs.create(\"d/b\")?;\n    fs.create(\"d/a\")?;\n    print(fs.list_dir(\"d\")?, fs.exists(\"d/a\"));\n    \
            for n in fs.list_dir(\"d\")? { fs.remove(\"d/\" + n)?; }\n    fs.remove(\"d\")?;\n    print(fs.exists(\"d\"));\n    Ok(())\n}\n\
        fn main() { match run() { case Ok(u) -> print(\"ok\"); case Err(e) -> print(\"err\", e); } }\n");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let run = |fail: &str| {
        let got = Command::new(&exe).current_dir(&dir).env("JW_RES_FAIL", fail).output().unwrap();
        String::from_utf8_lossy(&got.stdout).to_string()
    };
    assert_eq!(run(""), "[230, 151, 165]\n本\nAB! 0 <fs.File>\nファイルを開けません: nai.txt\n[a, b] true\nfalse\nok\n");
    assert_eq!(run("write"), "err ファイルに書けません\n");
    assert_eq!(run("close"), "err ファイルを閉じられません（書いた内容が残っていないかもしれません）\n");
    for (name, src, want) in [
        ("inner", "fn main() { __fs_open(\"a\", 0); }", "`__fs_open` は標準ライブラリの中だけで使えます"),
        ("decl", "resource X;\nfn main() {}", "`resource` は標準ライブラリの中だけで書けます"),
        ("send", "use std.fs;\nfn main() { match fs.open(\"a\") { case Ok(f) -> { let t = spawn(() -> { let g = f; }); } case Err(e) -> {} } }",
            "ほかのスレッドへ渡せません"),
        ("readonly", "use std.fs;\nfn main() { match fs.open(\"a\") { case Ok(f) -> { f.read(1); } case Err(e) -> {} } }", "mut ではない"),
    ] {
        let (out, _) = build(name, src);
        assert!(!out.status.success(), "{}", name);
        assert!(String::from_utf8_lossy(&out.stderr).contains(want), "{}: {}", name, String::from_utf8_lossy(&out.stderr));
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// std.net の TCP。待ち受け・接続・送受信・時間切れ
#[test]
fn jwc_std_net() {
    if !have_clang() { return; }
    let dir = std::env::temp_dir().join(format!("jw-jwc-net-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("main.jw");
    std::fs::write(&path, "use std.net;\n\
        fn Result<str, str> client(i64 port) {\n    let mut c = net.connect(\"127.0.0.1\", port)?;\n    c.write_text(\"ping\")?;\n    \
            let mut s = \"\";\n    for b in c.read(100)? { s += chr(b as i64); }\n    net.close(c)?;\n    Ok(s)\n}\n\
        fn Result<(), str> run() {\n    let mut l = net.listen(\"127.0.0.1\", 0)?;\n    let port = l.port();\n    \
            let t = spawn(() -> client(port));\n    let mut c = l.accept()?;\n    print(c.read(100)?.len());\n    c.write([112, 111, 110, 103])?;\n    \
            print(c.read(10)?.len(), t.join());\n    let q = net.connect(\"127.0.0.1\", port)?;\n    let mut d = l.accept()?;\n    d.set_timeout(100)?;\n    \
            match d.read(1) { case Ok(x) -> print(\"?\", x); case Err(e) -> print(e); }\n    net.stop(l)?;\n    Ok(())\n}\n\
        fn main() { match run() { case Ok(u) -> print(\"ok\"); case Err(e) -> print(\"err\", e); } }\n").unwrap();
    let exe = dir.join("main");
    let out = Command::new(jwc()).args(["build", "--clang", &native::clang(), "-o"]).arg(&exe).arg(&path).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let got = Command::new(&exe).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&got.stdout), "4\n0 Ok(pong)\n時間切れです\nok\n");
    let got = Command::new(&exe).env("JW_RES_FAIL", "listen").output().unwrap();
    assert_eq!(String::from_utf8_lossy(&got.stdout), "err 待ち受けられません: 127.0.0.1\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// std.http（std.net の上の HTTP/1.1 クライアント）。同じプログラムの中に小さなサーバーを立てて試す
#[test]
fn jwc_std_http() {
    if !have_clang() { return; }
    let dir = std::env::temp_dir().join(format!("jw-jwc-http-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("main.jw");
    std::fs::write(&path, HTTP_SRC).unwrap();
    let exe = dir.join("main");
    let out = Command::new(jwc()).args(["build", "--clang", &native::clang(), "-o"]).arg(&exe).arg(&path).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let got = Command::new(&exe).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&got.stdout), "200 text/plain 1, 2 こんに\n404 abcde true\n200 gzipped None\n\
        Ok(GET /a?b=1 HTTP/1.1|POST /p HTTP/1.1|GET /g HTTP/1.1|GET /z HTTP/1.1|)\nNone None None\nhttp:// か https:// で始まる URL ではありません: ftp://x\nok\n");
    let _ = std::fs::remove_dir_all(dir);
}

const HTTP_SRC: &str = r#"use std.net;
use std.http;
use std.text;
use std.gzip;

fn Result<str, str> serve(i64 port, i64 n) {
    let mut l = net.listen("127.0.0.1", port)?;
    let mut seen = "";
    for i in range(n) {
        let mut c = l.accept()?;
        let got = c.read(4096)?;
        match text.from_utf8(ref got) {
            case Some(s) -> seen += s.lines()[0] + "|";
            case None -> {}
        }
        if i == 0 {
            c.write_text("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 9\r\nX-A: 1\r\nx-a: 2\r\n\r\nこんにちはxx")?;
        } else if i == 1 {
            c.write_text("HTTP/1.1 303 See Other\r\nLocation: /g\r\nContent-Length: 0\r\n\r\n")?;
        } else if i == 2 {
            c.write_text("HTTP/1.1 404 Not Found\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2;x=y\r\nde\r\n0\r\n\r\n")?;
        } else {
            let u8[] plain = [103, 122, 105, 112, 112, 101, 100];
            let packed = gzip.compress(ref plain);
            c.write_text("HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: {packed.len()}\r\n\r\n")?;
            c.write(packed)?;
        }
        net.close(c)?;
    }
    Ok(seen)
}

fn Result<(), str> run() {
    let mut probe = net.listen("127.0.0.1", 0)?;
    let port = probe.port();
    net.stop(probe)?;
    let t = spawn(() -> serve(port, 4));
    let mut tries = 0;
    loop {
        match http.get("http://127.0.0.1:{port}/a?b=1") {
            case Ok(a) -> {
                print(a.status, a.headers["content-type"], a.headers["x-a"], a.text()?);
                break;
            }
            case Err(e) -> {
                tries += 1;
                if tries > 1000 { return Err(e); }
            }
        }
    }
    let b = http.post("http://127.0.0.1:{port}/p", "text/plain", "日本")?;
    print(b.status, b.text()?, b.url == "http://127.0.0.1:{port}/g");
    let z = http.get("http://127.0.0.1:{port}/z")?;
    print(z.status, z.text()?, z.headers.get("content-encoding"));
    print(t.join());
    let u8[] x1 = [237, 160, 128];
    let u8[] x2 = [192, 128];
    let u8[] x3 = [230, 151];
    print(text.from_utf8(ref x1), text.from_utf8(ref x2), text.from_utf8(ref x3));
    match http.get("ftp://x") { case Ok(x) -> print("?"); case Err(e) -> print(e); }
    Ok(())
}

fn main() { match run() { case Ok(u) -> print("ok"); case Err(e) -> print("err", e); } }
"#;

/// std.env・std.time・std.json
#[test]
fn jwc_std_env_time_json() {
    if !have_clang() { return; }
    let dir = std::env::temp_dir().join(format!("jw-jwc-json-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("main.jw");
    std::fs::write(&path, JSON_SRC).unwrap();
    let exe = dir.join("main");
    let out = Command::new(jwc()).args(["build", "--clang", &native::clang(), "-o"]).arg(&exe).arg(&path).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let got = Command::new(&exe).env("JW_TEST_X", "はい").env_remove("NO_SUCH_VAR_123").output().unwrap();
    assert_eq!(String::from_utf8_lossy(&got.stdout), "Some(はい) None None\ntrue true\nSome(上書き) Some(-125.0) Some(3.0) 5 [name, n, ok, o, a]\n{\"name\":\"上書き\",\"n\":-125,\"ok\":[true,false,null,3,0.1],\"o\":{},\"a\":[]}\ntrue true\nJSON が読めません（1 文字目）: 値がありません\nJSON が読めません（4 文字目）: 値がありません\nJSON が読めません（6 文字目）: キーのあとに `:` がありません\nJSON が読めません（2 文字目）: 値のあとに余分な文字があります\nJSON が読めません（3 文字目）: 小数点のあとに数字が必要です\nJSON が読めません（8 文字目）: 対になっていないサロゲートです\nJSON が読めません（5 文字目）: 値のあとに余分な文字があります\nJSON が読めません（4 文字目）: `true` のつもりの書き方が違います\nJSON が読めません（4 文字目）: 文字列の中に制御文字があります（\\n などと書きます）\nJSON が読めません（2 文字目）: 数の書き方が違います\nok [100000,0.002,null]\nJSON が読めません（513 文字目）: 入れ子が深すぎます\n\"a\\\"b\\\\c\\u0001\"\n");
    let _ = std::fs::remove_dir_all(dir);
}

const JSON_SRC: &str = r#"use std.env;
use std.time;
use std.json;

fn main() {
    print(env.get("JW_TEST_X"), env.get(""), env.get("NO_SUCH_VAR_123"));
    let t = time.now();
    time.sleep(30);
    print(t.elapsed_ms() >= 30, t.elapsed_ns() > 0);
    match json.parse(" {{\"name\": \"太郎\\n\\u00e9\\ud83d\\ude00\", \"n\": -12.5e1, \"ok\": [true, false, null, 3, 0.1], \"o\": {{}}, \"a\": [], \"name\": \"上書き\"}} ") {
        case Ok(v) -> {
            print(v.get("name").as_str(), v.get("n").as_num(), v.get("ok").at(3).as_num(), v.get("ok").len(), v.keys());
            print(json.stringify(ref v));
            print(v.get("nai").is_null(), v.at(0).is_null());
        }
        case Err(e) -> print(e);
    }
    for s in ["", "[1,", "{{\"a\" 1}}", "01", "1.", "\"\\ud800\"", "[1] x", "tru", "\"a\nb\"", "-", "[1e5, 2E-3, 1e400]"] {
        match json.parse(s) {
            case Ok(v) -> print("ok", json.stringify(ref v));
            case Err(e) -> print(e);
        }
    }
    let mut deep = "";
    for i in range(600) { deep += "["; }
    match json.parse(deep) { case Ok(v) -> print("?"); case Err(e) -> print(e); }
    print(json.quote("a\"b\\c" + chr(1)));
}
"#;

/// std.tls と https。`openssl s_server` を試験用の証明書（tests/tls）で立て、確かめられる場合と確かめられない場合を見る。
/// Windows（Schannel）は信頼の置き場所に試験用の発行元を入れないと試せないので、ここでは試さない（手で wine と Windows で確かめる）
#[cfg(unix)]
#[test]
fn jwc_std_tls() {
    if !have_clang() { return; }
    let dir = std::env::temp_dir().join(format!("jw-jwc-tls-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("main.jw");
    std::fs::write(&path, TLS_SRC).unwrap();
    let exe = dir.join("main");
    let out = Command::new(jwc()).args(["build", "--clang", &native::clang(), "-o"]).arg(&exe).arg(&path).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let certs = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/tls");
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut server = Command::new("openssl")
        .args(["s_server", "-quiet", "-www", "-accept"])
        .arg(port.to_string())
        .arg("-cert").arg(certs.join("server.pem"))
        .arg("-key").arg(certs.join("server.key"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("この試験には openssl のコマンドが要ります");
    let start = Instant::now();
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(start.elapsed().as_secs() < 20, "openssl s_server が立ち上がりません");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let trusted = Command::new(&exe).arg(port.to_string()).env("SSL_CERT_FILE", certs.join("ca.pem")).output().unwrap();
    let unknown = Command::new(&exe).arg(port.to_string()).env_remove("SSL_CERT_FILE").output().unwrap();
    let _ = server.kill();
    let _ = server.wait();
    assert_eq!(String::from_utf8_lossy(&trusted.stdout), "200 true\n\
        TLS の証明書を確かめられません（IP address mismatch）: 127.0.0.1\nOk([72, 84, 84, 80])\nok\n");
    assert_eq!(String::from_utf8_lossy(&unknown.stdout), "TLS の証明書を確かめられません（unable to get local issuer certificate）: localhost\n\
        TLS の証明書を確かめられません（unable to get local issuer certificate）: 127.0.0.1\n\
        TLS の証明書を確かめられません（unable to get local issuer certificate）: localhost\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(unix)]
const TLS_SRC: &str = r#"use std.http;
use std.tls;

fn Result<(), str> direct(i64 port) {
    let mut s = tls.connect("localhost", port)?;
    s.set_timeout(5000)?;
    s.write_text("GET / HTTP/1.0\r\n\r\n")?;
    print(s.read(4));
    tls.close(s)?;
    Ok(())
}

fn main() {
    let a = args();
    let port = a[0].to_int().unwrap_or(0);
    for u in ["https://localhost:{port}/", "https://127.0.0.1:{port}/"] {
        match http.get(u) {
            case Ok(r) -> print(r.status, r.body.len() > 0);
            case Err(e) -> print(e);
        }
    }
    match direct(port) { case Ok(_) -> print("ok"); case Err(e) -> print(e); }
}
"#;

/// std.gzip。gzip のコマンドが作ったもの（動的なハフマン符号のブロックを含む）を展開し、作ったものを gzip のコマンドで戻す
#[cfg(unix)]
#[test]
fn jwc_std_gzip() {
    if !have_clang() { return; }
    let dir = std::env::temp_dir().join(format!("jw-jwc-gzip-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("main.jw");
    std::fs::write(&path, GZIP_SRC).unwrap();
    let exe = dir.join("main");
    let out = Command::new(jwc()).args(["build", "--clang", &native::clang(), "-o"]).arg(&exe).arg(&path).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    // 縮む部分（文章）と縮まない部分（擬似乱数）をつないだデータ
    let mut data = Vec::new();
    for i in 0..3000 { data.extend_from_slice(format!("行 {} の内容は {} です。\n", i, i * 7 % 13).as_bytes()); }
    let mut x: u32 = 12345;
    for _ in 0..40000 {
        x = x.wrapping_mul(1103515245).wrapping_add(12345);
        data.push((x >> 16) as u8);
    }
    std::fs::write(dir.join("in.bin"), &data).unwrap();
    let gz = Command::new("gzip").args(["-9", "-c"]).arg(dir.join("in.bin")).output().expect("この試験には gzip のコマンドが要ります");
    assert!(gz.status.success());
    std::fs::write(dir.join("in.gz"), &gz.stdout).unwrap();
    let got = Command::new(&exe).current_dir(&dir).output().unwrap();
    let mut crc: u32 = !0;
    for b in &data {
        crc ^= *b as u32;
        for _ in 0..8 { crc = if crc & 1 == 1 { 0xEDB8_8320 ^ (crc >> 1) } else { crc >> 1 }; }
    }
    assert_eq!(String::from_utf8_lossy(&got.stdout), format!("{} {} true\n0 0\n\
        gzip のデータが途中で切れています\n展開した大きさが上限（10 バイト）を超えます\ngzip のデータではありません\nCRC32 が合いません（データが壊れています）\nok\n", data.len(), !crc));
    let back = Command::new("gzip").args(["-d", "-c"]).arg(dir.join("my.gz")).output().unwrap();
    assert!(back.status.success(), "{}", String::from_utf8_lossy(&back.stderr));
    assert!(back.stdout == data, "gzip -d で戻したものが元と違います");
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(unix)]
const GZIP_SRC: &str = r#"use std.gzip;
use std.fs;

fn Result<u8[], str> slurp(str p) {
    let mut f = fs.open(p)?;
    let u8[] mut all = [];
    loop {
        let got = f.read(65536)?;
        if got.len() == 0 { break; }
        for x in got { all.push(x); }
    }
    Ok(all)
}

fn show(Result<u8[], str> r) {
    match r { case Ok(b) -> print("ok", b.len()); case Err(e) -> print(e); }
}

fn Result<(), str> run() {
    let packed = slurp("in.gz")?;
    let plain = slurp("in.bin")?;
    let got = gzip.decompress(ref packed, 100000000)?;
    let mut same = got.len() == plain.len();
    for (i, b) in got { if b != plain[i] { same = false; } }
    print(got.len(), gzip.crc32(ref got), same);
    let mut f = fs.create("my.gz")?;
    f.write(gzip.compress(ref plain))?;
    fs.close(f)?;
    let u8[] empty = [];
    let e = gzip.compress(ref empty);
    match gzip.decompress(ref e, 0) { case Ok(b) -> print(b.len(), gzip.crc32(ref b)); case Err(m) -> print(m); }
    let u8[] mut cut = [];
    for i in range(packed.len() / 2) { cut.push(packed[i]); }
    show(gzip.decompress(ref cut, 100000000));
    show(gzip.decompress(ref packed, 10));
    show(gzip.decompress(ref plain, 100000000));
    let mut bad = gzip.compress(ref plain);
    let at = bad.len() - 6;
    bad[at] = bad[at] ^ 1;
    show(gzip.decompress(ref bad, 100000000));
    Ok(())
}

fn main() { match run() { case Ok(_) -> print("ok"); case Err(e) -> print(e); } }
"#;

/// std.pg。試験用の PostgreSQL をその場で立て（initdb・pg_ctl）、ログインの方式ごとと、問い合わせ・エラー・トランザクションを見る。
/// root で動かすときは postgres の利用者で立てる（PostgreSQL は root では動かない）
#[cfg(unix)]
#[test]
fn jwc_std_pg() {
    if !have_clang() { return; }
    let bin = pg_bindir().expect("この試験には PostgreSQL（initdb・pg_ctl）が要ります");
    let dir = std::env::temp_dir().join(format!("jw-jwc-pg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("main.jw");
    std::fs::write(&path, PG_SRC).unwrap();
    let exe = dir.join("main");
    let out = Command::new(jwc()).args(["build", "--clang", &native::clang(), "-o"]).arg(&exe).arg(&path).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let as_root = Command::new("id").arg("-u").output().map(|o| String::from_utf8_lossy(&o.stdout).trim() == "0").unwrap_or(false);
    let run = |cmd: String| {
        let st = if as_root {
            Command::new("su").args(["postgres", "-s", "/bin/sh", "-c"]).arg(&cmd).status()
        } else {
            Command::new("sh").arg("-c").arg(&cmd).status()
        };
        assert!(st.map(|s| s.success()).unwrap_or(false), "うまく動きません: {}", cmd);
    };
    let data = dir.join("data");
    if as_root {
        run_ok(Command::new("chown").arg("postgres").arg(&dir));
    }
    run(format!("{}/initdb -D {} -A trust -U postgres -E UTF8 --no-sync >/dev/null", bin.display(), data.display()));
    std::fs::write(
        data.join("pg_hba.conf"),
        "host all postgres 127.0.0.1/32 trust\nhost all t_trust 127.0.0.1/32 trust\nhost all t_md5 127.0.0.1/32 md5\n\
         host all t_scram 127.0.0.1/32 scram-sha-256\nhost all t_plain 127.0.0.1/32 password\n",
    )
    .unwrap();
    // TLS 用の証明書と鍵（鍵は持ち主だけが読めること）
    let tls = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/tls");
    std::fs::copy(tls.join("server.pem"), data.join("server.crt")).unwrap();
    std::fs::copy(tls.join("server.key"), data.join("server.key")).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(data.join("server.key"), std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    if as_root {
        run_ok(Command::new("chown").arg("postgres").arg(data.join("server.crt")).arg(data.join("server.key")));
    }
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let ctl = format!("{}/pg_ctl -D {}", bin.display(), data.display());
    run(format!(
        "{} -w -l {}/log -o '-p {} -k {} -c listen_addresses=127.0.0.1 -c fsync=off -c ssl=on' start >/dev/null",
        ctl, dir.display(), port, dir.display()
    ));
    let got = Command::new(&exe).arg(port.to_string()).env("SSL_CERT_FILE", tls.join("ca.pem")).output();
    run(format!("{} -m immediate stop >/dev/null", ctl));
    let got = got.unwrap();
    assert_eq!(String::from_utf8_lossy(&got.stderr), "");
    assert_eq!(
        String::from_utf8_lossy(&got.stdout),
        "t_trust [x, n, s, tls] [[Some(42), None, Some(こんにちは), Some(false)]] 1 Some(こんにちは) None\n\
         t_md5 [x, n, s, tls] [[Some(42), None, Some(こんにちは), Some(false)]] 1 Some(こんにちは) None\n\
         t_scram [x, n, s, tls] [[Some(42), None, Some(こんにちは), Some(false)]] 1 Some(こんにちは) None\n\
         t_scram Err PostgreSQL のエラー（FATAL 28P01）: password authentication failed for user \"t_scram\"\n\
         t_plain Err PostgreSQL のサーバーがパスワードをそのまま送るよう求めています（TLS なしでは送りません。tls: true にするか、md5 か scram-sha-256 にしてください）\n\
         nobody Err PostgreSQL のエラー（FATAL 28000）: no pg_hba.conf entry for host \"127.0.0.1\", user \"nobody\", database \"tdb\", no encryption\n\
         t_plain [x, n, s, tls] [[Some(42), None, Some(こんにちは), Some(true)]] 1 Some(こんにちは) None\n\
         t_scram [x, n, s, tls] [[Some(42), None, Some(こんにちは), Some(true)]] 1 Some(こんにちは) None\n\
         t_scram Err TLS の証明書を確かめられません（IP address mismatch）: 127.0.0.1\n\
         0 2\n\
         1\n\
         Err PostgreSQL のエラー（ERROR 23505）: duplicate key value violates unique constraint \"t_pkey\"（Key (id)=(1) already exists.）\n\
         Err PostgreSQL のエラー（ERROR 42601）: syntax error at or near \"SELEC\"\n\
         Err PostgreSQL のエラー（ERROR 08P01）: bind message supplies 0 parameters, but prepared statement \"\" requires 1\n\
         1 0\n\
         [id, name] [[Some(1), Some(a'; DROP TABLE t; --)], [Some(2), Some(b)], [Some(3), None]] 3\n\
         5000 5000 1000\n\
         0 0\n\
         3\n\
         ok\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(unix)]
fn run_ok(c: &mut Command) {
    assert!(c.status().map(|s| s.success()).unwrap_or(false), "うまく動きません: {:?}", c);
}

/// PostgreSQL のコマンドの置き場所（pg_config か、Debian 系の /usr/lib/postgresql/版/bin。新しい版を選ぶ）
#[cfg(unix)]
fn pg_bindir() -> Option<PathBuf> {
    if let Ok(o) = Command::new("pg_config").arg("--bindir").output() {
        let p = PathBuf::from(String::from_utf8_lossy(&o.stdout).trim());
        if o.status.success() && p.join("initdb").exists() { return Some(p); }
    }
    let mut best: Option<(u32, PathBuf)> = None;
    for e in std::fs::read_dir("/usr/lib/postgresql").ok()?.flatten() {
        let v: u32 = match e.file_name().to_string_lossy().parse() { Ok(v) => v, Err(_) => continue };
        let p = e.path().join("bin");
        if p.join("initdb").exists() && best.as_ref().map(|b| v > b.0).unwrap_or(true) { best = Some((v, p)); }
    }
    best.map(|b| b.1)
}

#[cfg(unix)]
const PG_SRC: &str = r#"use std.pg;

fn pg.Config cfg(i64 port, str user, str pw) { pg.Config { host: "127.0.0.1", port: port, user: user, password: pw, database: "tdb", tls: false } }

fn Result<(), str> setup(i64 port) {
    let mut db = pg.connect(pg.Config { host: "127.0.0.1", port: port, user: "postgres", password: "", database: "postgres", tls: false })?;
    db.exec("CREATE USER t_trust", [])?;
    db.exec("SET password_encryption = 'md5'", [])?;
    db.exec("CREATE USER t_md5 PASSWORD 'md5pw'", [])?;
    db.exec("SET password_encryption = 'scram-sha-256'", [])?;
    db.exec("CREATE USER t_scram PASSWORD 'scrampw'", [])?;
    db.exec("CREATE USER t_plain PASSWORD 'plainpw'", [])?;
    db.exec("CREATE DATABASE tdb OWNER t_scram", [])?;
    pg.close(db)?;
    Ok(())
}

fn Result<(), str> login(pg.Config c) {
    let user = c.user;
    let mut db = pg.connect(c)?;
    let r = db.query("SELECT 1 + $1::int AS x, NULL::text AS n, 'こんにちは' AS s, (SELECT ssl::text FROM pg_stat_ssl WHERE pid = pg_backend_pid()) AS tls", ["41"])?;
    print(user, r.columns, r.rows, r.affected, r.get(0, "s"), r.get(0, "nope"));
    pg.close(db)?;
    Ok(())
}

fn Result<(), str> work(i64 port) {
    let mut db = pg.connect(cfg(port, "t_scram", "scrampw"))?;
    let made = db.exec("CREATE TABLE t (id int PRIMARY KEY, name text)", [])?;
    // 値は SQL に埋め込まずに送るので、引用符などはそのまま値になる
    print(made, db.exec("INSERT INTO t VALUES ($1, $2), ($3, $4)", ["1", "a'; DROP TABLE t; --", "2", "b"])?);
    print(db.exec_opt("INSERT INTO t VALUES ($1, $2)", [Some("3"), None])?);
    // エラーのあとも同じつながりを使える
    match db.exec("INSERT INTO t VALUES ($1, $2)", ["1", "dup"]) {
        case Ok(n) -> print("?", n);
        case Err(e) -> print("Err", e);
    }
    match db.query("SELEC 1", []) {
        case Ok(r) -> print("?", r.rows.len());
        case Err(e) -> print("Err", e);
    }
    match db.query("SELECT $1::int", []) {
        case Ok(r) -> print("?", r.rows.len());
        case Err(e) -> print("Err", e);
    }
    db.exec("BEGIN", [])?;
    let changed = db.exec("UPDATE t SET name = $1 WHERE id = $2", ["c", "2"])?;
    print(changed, db.exec("ROLLBACK", [])?);
    let r = db.query("SELECT id, name FROM t ORDER BY id", [])?;
    print(r.columns, r.rows, r.affected);
    let big = db.query("SELECT g, repeat('x', 1000) FROM generate_series(1, 5000) g", [])?;
    let last = match big.get(4999, "repeat") { case Some(v) -> v.len(); case None -> -1; };
    print(big.rows.len(), big.affected, last);
    let empty = db.query("", [])?;
    print(empty.rows.len(), empty.affected);
    print(db.exec("DELETE FROM t", [])?);
    pg.close(db)?;
    Ok(())
}

fn main() {
    let a = args();
    let port = a[0].to_int().unwrap_or(0);
    match setup(port) { case Ok(_) -> {} case Err(e) -> print("setup", e); }
    let us = ["t_trust", "t_md5", "t_scram", "t_scram", "t_plain", "nobody"];
    let ps = ["", "md5pw", "scrampw", "wrong", "plainpw", ""];
    for (i, u) in us {
        match login(cfg(port, u, ps[i])) { case Ok(_) -> {} case Err(e) -> print(u, "Err", e); }
    }
    // TLS（証明書は localhost のもの。IP で照らすと合わない）
    let ts = ["localhost", "localhost", "127.0.0.1"];
    let tu = ["t_plain", "t_scram", "t_scram"];
    let tp = ["plainpw", "scrampw", "scrampw"];
    for (i, h) in ts {
        let c = pg.Config { host: h, port: port, user: tu[i], password: tp[i], database: "tdb", tls: true };
        match login(c) { case Ok(_) -> {} case Err(e) -> print(tu[i], "Err", e); }
    }
    match work(port) { case Ok(_) -> print("ok"); case Err(e) -> print("Err", e); }
}
"#;

/// std.hash・std.base64・std.random。ハッシュは決まった値（FIPS 180-4・RFC 4231・RFC 7914 の例）と比べ、乱数は範囲と長さだけ見る
#[test]
fn jwc_std_hash_random() {
    if !have_clang() { return; }
    let dir = std::env::temp_dir().join(format!("jw-jwc-hash-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("main.jw");
    std::fs::write(&path, HASH_SRC).unwrap();
    let exe = dir.join("main");
    let out = Command::new(jwc()).args(["build", "--clang", &native::clang(), "-o"]).arg(&exe).arg(&path).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let got = Command::new(&exe).output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&got.stdout),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\n\
         ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\n\
         248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1\n\
         cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0\n\
         b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7\n\
         60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54\n\
         55ac046e56e3089fec1691c22544b605f94185216dde0465e68b9d57c20dacbc\n\
         true false false\n\
         ,Zg==,Zm8=,Zm9v,Zm9vYg==,Zm9vYmE=,Zm9vYmFy\n\
         [102, 111, 111, 98, 97]\n0\n\
         base64 の長さが 4 の倍数ではありません\nbase64 にない文字があります: =\nbase64 にない文字があります: *\n\
         base64 の終わりの余りのビットが 0 ではありません\nbase64 の終わりの余りのビットが 0 ではありません\n\
         32 true true true\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

const HASH_SRC: &str = r##"use std.hash;
use std.base64;
use std.random;

fn show(ref u8[] b) { print(hash.hex(ref b)); }

fn main() {
    for s in ["", "abc", "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"] {
        let b = s.bytes();
        let d = hash.sha256(ref b);
        show(ref d);
    }
    let u8[] mut million = [];
    for i in range(1000000) { million.push(97); }
    let dm = hash.sha256(ref million);
    show(ref dm);
    // RFC 4231 の 1 と 6（鍵がブロックより長い）
    let u8[] mut k1 = [];
    for i in range(20) { k1.push(11); }
    let m1 = "Hi There".bytes();
    let h1 = hash.hmac_sha256(ref k1, ref m1);
    show(ref h1);
    let u8[] mut k6 = [];
    for i in range(131) { k6.push(170); }
    let m6 = "Test Using Larger Than Block-Size Key - Hash Key First".bytes();
    let h6 = hash.hmac_sha256(ref k6, ref m6);
    show(ref h6);
    // RFC 7914 の PBKDF2-HMAC-SHA256 の例（最初の 32 バイト）
    let pw = "passwd".bytes();
    let salt = "salt".bytes();
    let p = hash.pbkdf2_sha256(ref pw, ref salt, 1);
    show(ref p);
    let other = h1.map((x) -> x);
    let short = "Hi".bytes();
    print(hash.equal(ref h1, ref other), hash.equal(ref h1, ref h6), hash.equal(ref h1, ref short));
    let str[] mut parts = [];
    for s in ["", "f", "fo", "foo", "foob", "fooba", "foobar"] {
        let b = s.bytes();
        parts.push(base64.encode(ref b));
    }
    print(parts.join(","));
    match base64.decode("Zm9vYmE=") { case Ok(b) -> print(b); case Err(e) -> print(e); }
    match base64.decode("") { case Ok(b) -> print(b.len()); case Err(e) -> print(e); }
    for s in ["Zm9", "Zm=v", "Zm*v", "Zm9=", "Zh=="] {
        match base64.decode(s) { case Ok(b) -> print("?", b); case Err(e) -> print(e); }
    }
    let key = random.bytes(32);
    let mut in_range = true;
    let mut seen_lo = false;
    let mut seen_hi = false;
    for i in range(2000) {
        let n = random.int(1, 7);
        if n < 1 || n > 6 { in_range = false; }
        if n == 1 { seen_lo = true; }
        if n == 6 { seen_hi = true; }
        let w = random.int(-9223372036854775807, 9223372036854775807);
        if w == 9223372036854775807 { in_range = false; }
    }
    print(key.len(), in_range, seen_lo, seen_hi);
}
"##;

/// std.fs の stat・is_file・is_dir・copy・remove_all。remove_all がリンクの先を消さないことも見る（リンクは Rust 側で作るので unix だけ）
#[cfg(unix)]
#[test]
fn jwc_std_fs_tree() {
    if !have_clang() { return; }
    let dir = std::env::temp_dir().join(format!("jw-jwc-fstree-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("keep")).unwrap();
    std::fs::write(dir.join("keep/precious.txt"), "消えてはいけない").unwrap();
    std::fs::create_dir_all(dir.join("t/sub")).unwrap();
    std::fs::write(dir.join("t/a.txt"), "abc").unwrap();
    std::fs::write(dir.join("t/sub/b.txt"), "").unwrap();
    std::os::unix::fs::symlink(dir.join("keep"), dir.join("t/to_keep")).unwrap();
    std::os::unix::fs::symlink(dir.join("nowhere"), dir.join("t/broken")).unwrap();
    let path = dir.join("main.jw");
    std::fs::write(&path, FS_TREE_SRC).unwrap();
    let exe = dir.join("main");
    let out = Command::new(jwc()).args(["build", "--clang", &native::clang(), "-o"]).arg(&exe).arg(&path).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let before = std::fs::metadata(dir.join("t/a.txt")).unwrap().modified().unwrap();
    let ms = before.duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();
    let got = Command::new(&exe).current_dir(&dir).arg(ms.to_string()).output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&got.stdout),
        "3 true false false true\n0 false true false\n0 false true true\n0 false false true\n\
         true false false false\n情報を読めません: nai\n\
         ok abc 3\n写す元と先が同じです: t/a.txt\n写す元がファイルではありません: t\n写す元がファイルではありません: nai\n\
         false true\n情報を読めません: nai\n"
    );
    assert_eq!(std::fs::read_to_string(dir.join("keep/precious.txt")).unwrap(), "消えてはいけない");
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(unix)]
const FS_TREE_SRC: &str = r##"use std.fs;

fn show(str p, i64 ms) {
    match fs.stat(p) {
        case Ok(i) -> print(i.size, i.is_file, i.is_dir, i.is_link, i.modified_ms == ms);
        case Err(e) -> print(e);
    }
}

fn Result<(), str> run() {
    let a = args();
    let ms = a[0].to_int().unwrap_or(-1);
    show("t/a.txt", ms);
    match fs.stat("t") { case Ok(i) -> print(i.size, i.is_file, i.is_dir, i.is_link); case Err(e) -> print(e); }
    match fs.stat("t/to_keep") { case Ok(i) -> print(i.size, i.is_file, i.is_dir, i.is_link); case Err(e) -> print(e); }
    match fs.stat("t/broken") { case Ok(i) -> print(i.size, i.is_file, i.is_dir, i.is_link); case Err(e) -> print(e); }
    print(fs.is_file("t/a.txt"), fs.is_file("t"), fs.is_dir("t/a.txt"), fs.is_dir("nai"));
    show("nai", ms);
    fs.copy("t/a.txt", "c.txt")?;
    let mut f = fs.open("c.txt")?;
    let body = f.read_text()?;
    fs.close(f)?;
    print("ok", body, body.len());
    let froms = ["t/a.txt", "t", "nai"];
    for (i, to) in ["t/a.txt", "x", "x"] {
        match fs.copy(froms[i], to) { case Ok(_) -> print("?"); case Err(e) -> print(e); }
    }
    fs.remove_all("t")?;
    print(fs.exists("t"), fs.exists("keep/precious.txt"));
    match fs.remove_all("nai") { case Ok(_) -> print("?"); case Err(e) -> print(e); }
    Ok(())
}

fn main() { match run() { case Ok(_) -> {} case Err(e) -> print("err", e); } }
"##;

#[test]
fn jwc_std_url_fs_time() {
    if !have_clang() { return; }
    let dir = std::env::temp_dir().join(format!("jw-jwc-url-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("main.jw");
    std::fs::write(&path, URL_SRC).unwrap();
    let exe = dir.join("main");
    let out = Command::new(jwc()).args(["build", "--clang", &native::clang(), "-o"]).arg(&exe).arg(&path).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let got = Command::new(&exe).current_dir(&dir).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&got.stdout), "http Example.com 8080 /a/b q=1&r=2 top /a/b?q=1&r=2\n::1 443 /x\nURL に利用者情報（@）は使えません: http://a@b/\nポートの番号が読めません: http://h:99999/\nscheme:// で始まる URL ではありません: nope\nホストの名前がありません: http://:80/\nポートの番号が読めません: http://h:8x/\nホストの名前がありません: file:///tmp/a\na%20b%26c%3D%E6%97%A5%E6%9C%AC~\na b&c=日本~ x\n% のあとに 2 桁の 16 進数がありません: %\n% のあとに 2 桁の 16 進数がありません: %2\n% のあとに 2 桁の 16 進数がありません: %zz\n戻した結果が UTF-8 になりません: %FF\n二回目\n\nfalse\nファイルを書くために開けません: nodir/a.txt.tmp\n名前を変えられません: none\ntrue\n");
    let _ = std::fs::remove_dir_all(dir);
}

const URL_SRC: &str = r##"use std.url;
use std.fs;
use std.time;

fn main() {
    match url.parse("HTTP://Example.com:8080/a/b?q=1&r=2#top") {
        case Ok(u) -> print(u.scheme, u.host, u.port, u.path, u.query, u.fragment, u.target());
        case Err(e) -> print(e);
    }
    match url.parse("https://[::1]/x") { case Ok(u) -> print(u.host, u.port, u.path); case Err(e) -> print(e); }
    for s in ["http://a@b/", "http://h:99999/", "nope", "http://:80/", "http://h:8x/", "file:///tmp/a"] {
        match url.parse(s) { case Ok(u) -> print("ok", u.host, u.port); case Err(e) -> print(e); }
    }
    print(url.encode("a b&c=日本~"));
    match url.decode("a%20b%26c%3D%E6%97%A5%E6%9C%AC~+x", true) { case Ok(t) -> print(t); case Err(e) -> print(e); }
    for s in ["%", "%2", "%zz", "%FF"] {
        match url.decode(s, false) { case Ok(t) -> print("ok", t); case Err(e) -> print(e); }
    }
    match fs.write_text_atomic("a.txt", "一回目\n") { case Ok(_) -> {} case Err(e) -> print(e); }
    match fs.write_text_atomic("a.txt", "二回目\n") { case Ok(_) -> {} case Err(e) -> print(e); }
    match fs.open("a.txt") {
        case Ok(f) -> { let mut g = f; match g.read_text() { case Ok(t) -> print(t); case Err(e) -> print(e); } }
        case Err(e) -> print(e);
    }
    print(fs.exists("a.txt.tmp"));
    match fs.write_text_atomic("nodir/a.txt", "x") { case Ok(_) -> print("?"); case Err(e) -> print(e); }
    match fs.rename("none", "b") { case Ok(_) -> print("?"); case Err(e) -> print(e); }
    let t = time.unix_ms();
    print(t > 1700000000000 && t < 4000000000000);
}
"##;

#[test]
fn jwc_std_browser() {
    if !have_clang() { return; }
    let dir = std::env::temp_dir().join(format!("jw-jwc-browser-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("main.jw");
    std::fs::write(&path, BROWSER_SRC).unwrap();
    std::fs::write(dir.join("script.txt"), "chrome {\"a\":1}\nkey 2\nnewtab http://n/\nclose\n").unwrap();
    let exe = dir.join("main");
    // エンジンがなければ断る
    let out = Command::new(jwc()).args(["build", "--clang", &native::clang(), "-o"]).arg(&exe).arg(&path).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("エンジンの実装が要ります"));
    let mock = root().join("runtime/engine/mock/jwe_mock.c");
    let out = Command::new(jwc()).args(["build", "--clang", &native::clang(), "--link"]).arg(&mock).arg("-o").arg(&exe).arg(&path).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let got = Command::new(&exe).current_dir(&dir).env("JWE_MOCK_SCRIPT", "script.txt").env("JWE_MOCK_LOG", "log.txt").output().unwrap();
    assert_eq!(String::from_utf8_lossy(&got.stdout), "窓の大きさは 1〜100000 にしてください\nmock false true false\n1 Err(そのタブはありません) Ok(()) Ok(()) Ok(())\nOk(()) Ok(()) Ok(()) Ok(()) Err(そのタブはありません)\nSome(Ready)\nNavStarted(1, http://a/)\nUrlChanged(1, http://a/)\nTitleChanged(1, title:http://a/)\nNavFinished(1, http://a/, 0)\nHistoryChanged(1, false, false)\nNavStarted(1, http://b/)\nUrlChanged(1, http://b/)\nTitleChanged(1, title:http://b/)\nNavFinished(1, http://b/, 0)\nHistoryChanged(1, true, false)\nNavStarted(1, http://a/)\nUrlChanged(1, http://a/)\nTitleChanged(1, title:http://a/)\nNavFinished(1, http://a/, 0)\nHistoryChanged(1, false, true)\nChromeMessage({\"a\":1})\nkey NewTab\nNewTabRequested(0, http://n/)\n閉じる\n");
    assert_eq!(std::fs::read_to_string(dir.join("log.txt")).unwrap(), "open 0 試し\ntab_new 1\nnavigate 1 http://a/\nnavigate 1 http://b/\nback 1\nchrome_load 40\nchrome_post 0 こんにちは\nset_title 0 題\ntab_close 1\nclose 0\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// 実行時ライブラリの中のエンジンとの境目は runtime/engine/jw_engine.h の写し
#[test]
fn runtime_engine_header_is_a_copy() {
    let rt = std::fs::read_to_string(root().join("runtime/jw_rt.c")).unwrap().replace("\r\n", "\n");
    let h = std::fs::read_to_string(root().join("runtime/engine/jw_engine.h")).unwrap().replace("\r\n", "\n");
    let a = rt.find("// ---- jw_engine.h ここから\n").unwrap() + "// ---- jw_engine.h ここから\n".len();
    let b = rt.find("// ---- jw_engine.h ここまで\n").unwrap();
    assert_eq!(&rt[a..b], h, "runtime/jw_rt.c の写しを runtime/engine/jw_engine.h に合わせてください");
}

const BROWSER_SRC: &str = r##"use std.browser;

fn main() {
    match browser.open(browser.Config { title: "試し", data_dir: "", width: 0, height: 10, ephemeral: true }) {
        case Ok(_) -> print("?");
        case Err(e) -> print(e);
    }
    let mut e = match browser.open(browser.Config { title: "試し", data_dir: "data", width: 800, height: 600, ephemeral: true }) {
        case Ok(x) -> x;
        case Err(m) -> { print(m); return; }
    };
    let info = e.info();
    print(info.name, info.script, info.chrome_view, info.persist);
    let tab = match e.tab_new() { case Ok(t) -> t; case Err(m) -> { print(m); return; } };
    print(tab, e.tab_show(99), e.navigate(tab, "http://a/"), e.navigate(tab, "http://b/"), e.back(tab));
    print(e.chrome_load("<p>枠</p>", 40), e.chrome_post("こんにちは"), e.set_title("題"), e.tab_close(tab), e.tab_close(tab));
    print(e.poll());
    loop {
        match e.wait(-1) {
            case Some(CloseRequested) -> { print("閉じる"); break; }
            case Some(Key(k)) -> print("key", browser.KeyCode.of(k));
            case Some(ev) -> print(ev);
            case None -> { print("なし"); break; }
        }
    }
    browser.close(e);
}
"##;



/// `jwc build` で作った実行ファイルが、interp と同じものを出す
#[test]
fn jwc_build_makes_an_executable() {
    if !have_clang() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("jw-jwc-build-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for name in ["fib", "bst", "shop"] {
        let path = root().join(format!("examples/{}.jw", name));
        let exe = dir.join(name);
        let clang = native::clang();
        let out = Command::new(jwc())
            .args(["build", "--clang", &clang, "-o", exe.to_str().unwrap(), path.to_str().unwrap()])
            .output()
            .expect("jwc を起動できる");
        assert!(out.status.success(), "{}: {}", name, String::from_utf8_lossy(&out.stderr));
        let src = std::fs::read_to_string(&path).unwrap();
        let want = big(move || jabberwock::run_source(&src, false).output);
        let got = Command::new(&exe).output().expect("作った実行ファイルを起動できる");
        assert!(got.status.success(), "jwc generated executable failed: {}", String::from_utf8_lossy(&got.stderr));
        assert_eq!(String::from_utf8_lossy(&got.stdout), want, "{}", name);
    }
}

/// LTO が使えない環境でも、引数リストをムーブ済みにせず通常リンクへ戻れる。
#[cfg(unix)]
#[test]
fn jwc_build_retries_without_lto() {
    if !have_clang() { return; }
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("jw-jwc-fallback-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let wrapper = dir.join("clang-no-lto");
    std::fs::write(&wrapper, "#!/bin/sh\nfor arg do\n  if [ \"$arg\" = -flto ]; then exit 1; fi\ndone\nexec \"$JW_TEST_CLANG\" \"$@\"\n").unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    let src = dir.join("main.jw");
    let exe = dir.join("main");
    std::fs::write(&src, "fn main() { print(7); }").unwrap();
    let output = Command::new(jwc()).arg("build").arg("--clang").arg(&wrapper).arg("-o").arg(&exe).arg(&src)
        .env("JW_TEST_CLANG", native::clang()).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let output = Command::new(exe).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "7\n");
    let _ = std::fs::remove_dir_all(dir);
}
