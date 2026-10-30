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

/// std.net の TCP（Windows はまだ）。待ち受け・接続・送受信・時間切れ
#[cfg(unix)]
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
#[cfg(unix)]
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
    assert_eq!(String::from_utf8_lossy(&got.stdout), "200 text/plain 1, 2 こんに\n404 abcde\n\
        Ok(GET /a?b=1 HTTP/1.1|POST /p HTTP/1.1|)\nNone None None\nhttps はまだ使えません: https://x\nok\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(unix)]
const HTTP_SRC: &str = r#"use std.net;
use std.http;
use std.text;

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
        } else {
            c.write_text("HTTP/1.1 404 Not Found\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2;x=y\r\nde\r\n0\r\n\r\n")?;
        }
        net.close(c)?;
    }
    Ok(seen)
}

fn Result<(), str> run() {
    let mut probe = net.listen("127.0.0.1", 0)?;
    let port = probe.port();
    net.stop(probe)?;
    let t = spawn(() -> serve(port, 2));
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
    print(b.status, b.text()?);
    print(t.join());
    let u8[] x1 = [237, 160, 128];
    let u8[] x2 = [192, 128];
    let u8[] x3 = [230, 151];
    print(text.from_utf8(ref x1), text.from_utf8(ref x2), text.from_utf8(ref x3));
    match http.get("https://x") { case Ok(x) -> print("?"); case Err(e) -> print(e); }
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
