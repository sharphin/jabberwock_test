//! パッケージ（alice.toml と `alice` コマンドの new / build / run / test）

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use jabberwock::project::{parse_manifest, Manifest, Project};

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("jw-project-test-{}-{}", std::process::id(), name));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn write(p: &Path, s: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, s).unwrap();
}

fn jw(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_alice")).args(args).current_dir(dir).output().expect("runs");
    (o.status.code().unwrap_or(-1), String::from_utf8_lossy(&o.stdout).to_string(), String::from_utf8_lossy(&o.stderr).to_string())
}

#[test]
fn manifest() {
    let m = parse_manifest(
        "# 注釈\n[package]\nname = \"app\"  # 名前\nversion = \"0.1.0\"\n\n[dependencies]\ngeom = { path = \"../geom\" }\nutil = \"lib/util\"\n",
    )
    .unwrap();
    assert_eq!(
        m,
        Manifest { name: "app".into(), deps: vec![("geom".into(), "../geom".into()), ("util".into(), "lib/util".into())] }
    );
    let e = |t: &str| parse_manifest(t).unwrap_err();
    assert!(e("[dependencies]\n").contains("name"));
    assert!(e("[package]\nname = \"1a\"\n").contains("使えません"));
    assert!(e("[package]\nname = \"a\"\n[dependencies]\nx = { git = \"u\" }\n").contains("手元のフォルダだけ"));
    assert!(e("[package]\nname = \"a\"\n[dependencies]\nx = \"a\"\nx = \"b\"\n").contains("2 回"));
    assert!(e("[package\n").contains("1 行目"));
}

#[test]
fn package_map_exports_existing_resolution() {
    let d = tmp("package-map");
    let dep = d.with_extension("dep");
    write(&d.join("alice.toml"), &format!("[package]\nname = \"app\"\n[dependencies]\nmathx = {{ path = \"../{}\" }}\n", dep.file_name().unwrap().to_str().unwrap()));
    write(&d.join("src/main.jw"), "use mathx; fn main() { print(mathx.one()); }");
    write(&dep.join("alice.toml"), "[package]\nname = \"mathx\"\n");
    write(&dep.join("src/lib.jw"), "pub fn i64 one() { 1 }");
    let p = Project::load(&d).unwrap();
    let want = p.package_map(&p.main_file()).unwrap();
    let (code, out, err) = jw(&d, &["package-map"]);
    assert_eq!(code, 0, "{}", err);
    assert_eq!(out, want);
    assert!(out.contains("allow\t\tmathx\n"));
    let (code, _, err) = jw(&d, &["package-map", "-o", "deps.map"]);
    assert_eq!(code, 0, "{}", err);
    assert_eq!(fs::read_to_string(d.join("deps.map")).unwrap(), want);
    assert!(p.package_map(&dep.join("src/lib.jw")).is_err());
    let _ = fs::remove_dir_all(d);
    let _ = fs::remove_dir_all(dep);
}

#[test]
fn modules_come_from_src_and_dependencies() {
    let d = tmp("load");
    write(&d.join("app/alice.toml"), "[package]\nname = \"app\"\n[dependencies]\nshapes = { path = \"../shapes\" }\n");
    write(&d.join("app/src/main.jw"), "use shapes;\nuse util;\nfn main() { print(shapes.area(2), util.one()); }\n");
    write(&d.join("app/src/util.jw"), "pub fn i64 one() { 1 }\n");
    // 依存先の依存先も読む
    write(&d.join("shapes/alice.toml"), "[package]\nname = \"shapes\"\n[dependencies]\nnums = { path = \"../nums\" }\n");
    write(&d.join("shapes/src/lib.jw"), "use nums;\nuse sq;\npub fn i64 area(i64 x) { sq.sq(x) + nums.zero() }\n");
    write(&d.join("shapes/src/sq.jw"), "pub fn i64 sq(i64 x) { x * x }\n");
    write(&d.join("nums/alice.toml"), "[package]\nname = \"nums\"\n");
    write(&d.join("nums/src/lib.jw"), "pub fn i64 zero() { 0 }\n");
    let p = Project::load(&d.join("app")).unwrap();
    let names: Vec<&str> = p.modules.keys().map(|s| s.as_str()).collect();
    assert_eq!(names, ["*nums", "*shapes", "shapes/sq", "util"]);
    assert_eq!(Project::find(&d.join("app/src")).as_deref(), Some(d.join("app").as_path()));
    let read = p.reader();
    let src = fs::read_to_string(p.main_file()).unwrap();
    let o = jabberwock::run_program(&src, &read, false, true, Box::new(std::io::Cursor::new(Vec::new())), Vec::new());
    assert!(o.error.is_none(), "{:?}", o.error.map(|d| d.title));
    assert_eq!(o.output, "4 1\n");

    // 依存先の名前と同じ名前のモジュール・名前の違う依存先・ライブラリでない依存先はエラー
    write(&d.join("app/src/nums.jw"), "pub fn i64 zero() { 0 }\n");
    assert!(Project::load(&d.join("app")).unwrap_err().contains("依存先のパッケージ `nums` が同じ名前"));
    fs::remove_file(d.join("app/src/nums.jw")).unwrap();
    write(&d.join("nums/alice.toml"), "[package]\nname = \"numbers\"\n");
    assert!(Project::load(&d.join("app")).unwrap_err().contains("名前が `numbers`"));
    write(&d.join("nums/alice.toml"), "[package]\nname = \"nums\"\n");
    fs::remove_file(d.join("nums/src/lib.jw")).unwrap();
    assert!(Project::load(&d.join("app")).unwrap_err().contains("src/lib.jw がありません"));
    write(&d.join("app/alice.toml"), "[package]\nname = \"app\"\n[dependencies]\nnone = { path = \"../none\" }\n");
    assert!(Project::load(&d.join("app")).unwrap_err().contains("フォルダがありません"));
    let _ = fs::remove_dir_all(&d);
}

fn run_app(p: &Project) -> Result<String, String> {
    let read = p.reader();
    let src = fs::read_to_string(p.main_file()).unwrap();
    let o = jabberwock::run_program(&src, &read, false, true, Box::new(std::io::Cursor::new(Vec::new())), Vec::new());
    match o.error {
        Some(e) => Err(format!("{}: {:?}", e.title, e.note)),
        None => Ok(o.output),
    }
}

#[test]
fn each_package_has_its_own_names() {
    let d = tmp("space");
    write(&d.join("app/alice.toml"), "[package]\nname = \"app\"\n[dependencies]\nshapes = { path = \"../shapes\" }\n");
    // app の util と shapes の util は別のモジュール
    write(&d.join("app/src/util.jw"), "pub fn str who() { \"app\" }\n");
    write(&d.join("app/src/main.jw"), "use shapes;\nuse util;\nfn main() { print(util.who(), shapes.who(), shapes.area(3)); }\n");
    write(&d.join("shapes/alice.toml"), "[package]\nname = \"shapes\"\n[dependencies]\nnums = { path = \"../nums\" }\n");
    write(
        &d.join("shapes/src/lib.jw"),
        "use util;\nuse nums;\npub struct Sq { i64 n }\npub fn str who() { util.who() }\npub fn i64 area(i64 x) { let s = Sq { n: util.sq(x) }; s.n + nums.zero() }\n",
    );
    write(&d.join("shapes/src/util.jw"), "pub fn str who() { \"shapes\" }\npub fn i64 sq(i64 x) { x * x }\n");
    write(&d.join("nums/alice.toml"), "[package]\nname = \"nums\"\n");
    write(&d.join("nums/src/lib.jw"), "pub fn i64 zero() { 0 }\n");
    let p = Project::load(&d.join("app")).unwrap();
    assert_eq!(run_app(&p).unwrap(), "app shapes 9\n");

    // 依存先の中のモジュールは外から見えない
    write(&d.join("app/src/main.jw"), "use util;\nfn main() { print(util.sq(2)); }\n");
    let e = run_app(&p).unwrap_err();
    assert!(e.contains("`sq`"), "{}", e);

    // 依存先の中の `use` は、自分にないモジュールを app から借りない
    fs::remove_file(d.join("shapes/src/util.jw")).unwrap();
    write(&d.join("app/src/util.jw"), "pub fn str who() { \"app\" }\npub fn i64 sq(i64 x) { x * x }\n");
    write(&d.join("app/src/main.jw"), "use shapes;\nfn main() { print(shapes.who()); }\n");
    let p = Project::load(&d.join("app")).unwrap();
    let e = run_app(&p).unwrap_err();
    assert!(e.contains("見つかりません"), "{}", e);
    write(&d.join("shapes/src/util.jw"), "pub fn str who() { \"shapes\" }\npub fn i64 sq(i64 x) { x * x }\n");

    // 依存先の依存先は、alice.toml に書かなければ使えない
    write(&d.join("app/src/main.jw"), "use nums;\nfn main() { print(nums.zero()); }\n");
    let e = run_app(&p).unwrap_err();
    assert!(e.contains("`nums` が見つかりません"), "{}", e);
    write(
        &d.join("app/alice.toml"),
        "[package]\nname = \"app\"\n[dependencies]\nshapes = { path = \"../shapes\" }\nnums = { path = \"../nums\" }\n",
    );
    let p = Project::load(&d.join("app")).unwrap();
    assert_eq!(run_app(&p).unwrap(), "0\n");

    // スレッドの中で読み直しても同じモジュールになる（nums は app と shapes の両方から使う）
    write(
        &d.join("app/src/main.jw"),
        "use shapes;\nuse util;\nuse nums;\nfn main() {\n    let t = spawn(() -> shapes.who() + \"/\" + util.who());\n    print(t.join(), shapes.area(2), nums.zero());\n}\n",
    );
    let p = Project::load(&d.join("app")).unwrap();
    assert_eq!(run_app(&p).unwrap(), "shapes/app 4 0\n");
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn new_run_and_test_commands() {
    let d = tmp("cli");
    let (c, _, err) = jw(&d, &["new", "app"]);
    assert_eq!(c, 0, "{}", err);
    let (c, _, err) = jw(&d, &["new", "mathx", "--lib"]);
    assert_eq!(c, 0, "{}", err);
    assert_eq!(jw(&d, &["new", "app"]).0, 1);
    let app = d.join("app");
    let (c, out, _) = jw(&app.join("src"), &["run"]);
    assert_eq!((c, out.as_str()), (0, "Hello, world!\n"));

    // ライブラリの tests は最初から通る
    let (c, out, _) = jw(&d.join("mathx"), &["test"]);
    assert_eq!(c, 0, "{}", out);
    assert!(out.replace('\\', "/").contains("test tests/add.jw::test_add ... ok") && out.contains("2 本成功・0 本失敗"), "{}", out);

    // 依存先を足して、引数も渡す
    write(&app.join("alice.toml"), "[package]\nname = \"app\"\n\n[dependencies]\nmathx = { path = \"../mathx\" }\n");
    write(&app.join("src/main.jw"), "use mathx;\nfn main() {\n    print(mathx.add(2, 3), args());\n}\n");
    let (c, out, err) = jw(&app, &["run", "--", "x", "y"]);
    assert_eq!((c, out.as_str()), (0, "5 [x, y]\n"), "{}", err);

    // エラーの位置はパッケージからの道筋で出す
    write(&d.join("mathx/src/lib.jw"), "pub fn i64 add(i64 a, i64 b) {\n    a + true\n}\n");
    let (c, _, err) = jw(&app, &["run"]);
    assert_eq!(c, 1);
    assert!(err.contains("mathx/src/lib.jw:2:5") || err.contains("lib.jw:2:5"), "{}", err);
    write(&d.join("mathx/src/lib.jw"), "pub fn i64 add(i64 a, i64 b) {\n    a + b\n}\n");

    // test: 出力の違い・実行時エラーは失敗
    write(&app.join("tests/good.jw"), "use mathx;\nfn main() { print(mathx.add(1, 1)); }\n");
    write(&app.join("tests/good.out"), "2\n");
    write(&app.join("tests/wrong.jw"), "fn main() { print(1); }\n");
    write(&app.join("tests/wrong.out"), "2\n");
    write(&app.join("tests/boom.jw"), "fn main() { let v = [1]; let i = 3; print(v[i]); }\n");
    let (c, out, _) = jw(&app, &["test"]);
    assert_eq!(c, 1);
    let out = out.replace('\\', "/");
    assert!(out.contains("test tests/good.jw ... ok"), "{}", out);
    assert!(out.contains("test tests/wrong.jw ... 失敗") && out.contains("出力が違います"), "{}", out);
    assert!(out.contains("test tests/boom.jw ... 失敗") && out.contains("tests/boom.jw:1:"), "{}", out);
    assert!(out.contains("1 本成功・2 本失敗"), "{}", out);

    // 関数ごとのテスト（main のないファイル）。1 本ずつ別に動かし、名前の一部で選べる
    for f in ["good", "wrong", "boom"] {
        fs::remove_file(app.join(format!("tests/{}.jw", f))).unwrap();
    }
    write(
        &app.join("tests/fns.jw"),
        "use mathx;\n\nfn i64 two() { 2 }\n\nfn test_ok() {\n    assert(mathx.add(1, 1) == two());\n}\n\nfn test_ng() {\n    assert(mathx.add(1, 1) == 3);\n}\n\nfn test_exit() {\n    exit(3);\n}\n",
    );
    let (c, out, _) = jw(&app, &["test"]);
    assert_eq!(c, 1);
    let out = out.replace('\\', "/");
    assert!(out.contains("test tests/fns.jw::test_ok ... ok"), "{}", out);
    assert!(out.contains("test tests/fns.jw::test_ng ... 失敗") && out.contains("assert に失敗しました") && out.contains("tests/fns.jw:10:"), "{}", out);
    assert!(out.contains("test tests/fns.jw::test_exit ... 失敗") && out.contains("exit(3)"), "{}", out);
    assert!(out.contains("1 本成功・2 本失敗"), "{}", out);
    let (c, out, _) = jw(&app, &["test", "test_ok"]);
    assert_eq!(c, 0, "{}", out);
    assert!(out.contains("1 本成功・0 本失敗"), "{}", out);
    assert_eq!(jw(&app, &["test", "nothing"]).0, 1);
    // 形の違うテストの関数・main と混ぜる・何もない
    for (src, want) in [
        ("fn test_a(i64 x) {}\n", "引数なし"),
        ("fn i64 test_a() { 1 }\n", "戻り値なし"),
        ("fn main() {}\nfn test_a() {}\n", "main のあるファイル"),
        ("fn helper() {}\n", "ありません"),
    ] {
        write(&app.join("tests/fns.jw"), src);
        let (c, out, _) = jw(&app, &["test"]);
        assert_eq!(c, 1, "{}", src);
        assert!(out.contains(want), "{}: {}", src, out);
    }

    // パッケージの外
    let (c, _, err) = jw(&std::env::temp_dir(), &["test"]);
    assert_eq!(c, 1);
    assert!(err.contains("alice.toml が見つかりません"), "{}", err);
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn build_makes_an_executable_once() {
    if Command::new(std::env::var("JW_CLANG").unwrap_or_else(|_| "clang".into())).arg("--version").output().is_err() {
        return;
    }
    let d = tmp("build");
    assert_eq!(jw(&d, &["new", "hello"]).0, 0);
    let p = d.join("hello");
    let (c, _, err) = jw(&p, &["build"]);
    assert_eq!(c, 0, "{}", err);
    assert!(err.contains("作りました"), "{}", err);
    let exe = jabberwock::project::Project::load(&p).unwrap().exe_path(false);
    let out = Command::new(&exe).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "Hello, world!\n");
    // 変わっていなければ作り直さない
    let (_, _, err) = jw(&p, &["build"]);
    assert!(err.contains("最新です"), "{}", err);
    let (c, out, _) = jw(&p, &["run", "--native"]);
    assert_eq!((c, out.as_str()), (0, "Hello, world!\n"));
    let _ = fs::remove_dir_all(&d);
}
