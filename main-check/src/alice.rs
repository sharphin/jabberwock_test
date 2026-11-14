//! `alice`: jabberwock のパッケージの道具（cargo を小さくしたもの）。説明は docs/packages.md、中身は src/project.rs
//! Rust 版（src/）は凍結だが、alice だけは例外（2026-11-14、オーナーの選択）。jwc を呼ぶ道（`--compiler jwc`）を足した
use std::path::{Path, PathBuf};
use std::{env, fs, process};

const USAGE: &str = "使い方: alice <コマンド>
  new <名前> [--lib]              新しいパッケージを作る（--lib はライブラリ）
  build [--release]               target/debug（release）に実行ファイルを作る（clang が要る）
  package-map [-o ファイル] [入口.jw]  jwc用のローカル依存情報を書き出す
  run [--native] [-- 引数...]      src/main.jw を動かす（--native は実行ファイルを作って動かす）
  test [--native] [名前の一部]     tests/*.jw を動かす（main のないファイルは fn test_名前() を 1 本ずつ。tests/名前.out があれば出力と比べる）
build・run・test には --compiler rust|jwc（既定 rust）と --jwc jwcの実行ファイル（既定は alice と同じフォルダの jwc）をつけられる。
jwc は実行ファイルを作るだけなので、jwc の run・test には --native が要る";

/// どのコンパイラで作るか。jwc は実行ファイルの場所
enum Compiler {
    Rust,
    Jwc(PathBuf),
}

/// 引数から `--compiler rust|jwc` と `--jwc 実体` を取り出す（run の `--` より後は見ない）。残りを返す
fn take_compiler(args: Vec<String>) -> (Compiler, Vec<String>) {
    let mut name = None;
    let mut jwc = None;
    let mut rest = Vec::new();
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--compiler" | "--jwc" => {
                let Some(v) = it.next() else {
                    eprintln!("{} のあとに値を書いてください", a);
                    process::exit(2);
                };
                let slot = if a == "--compiler" { &mut name } else { &mut jwc };
                if slot.is_some() {
                    eprintln!("{} は 1 回だけ書けます", a);
                    process::exit(2);
                }
                *slot = Some(v);
            }
            "--" => {
                rest.push(a);
                rest.extend(it.by_ref());
            }
            _ => rest.push(a),
        }
    }
    let c = match name.as_deref() {
        None | Some("rust") => {
            if jwc.is_some() {
                eprintln!("--jwc は --compiler jwc のときだけ書けます");
                process::exit(2);
            }
            Compiler::Rust
        }
        Some("jwc") => {
            let path = match jwc {
                Some(j) => PathBuf::from(j),
                None => {
                    let exe = env::current_exe().unwrap_or_default();
                    exe.with_file_name(format!("jwc{}", env::consts::EXE_SUFFIX))
                }
            };
            // 区切りのない名前は PATH から探させる。道筋なら、ここで有無を確かめる
            if path.components().count() > 1 && !path.is_file() {
                eprintln!("jwc が見つかりません: {}（--jwc で場所を指定してください）", path.display());
                process::exit(1);
            }
            Compiler::Jwc(path)
        }
        Some(other) => {
            eprintln!("--compiler は rust か jwc です（`{}` は知りません）", other);
            process::exit(2);
        }
    };
    (c, rest)
}

/// jwc の出力の中の、パッケージのフォルダの絶対パスを外す（Rust 版と同じく、パッケージからの道筋で見せる）
fn relative_in(p: &jabberwock::project::Project, text: &str) -> String {
    let root = p.root.canonicalize().unwrap_or_else(|_| p.root.clone());
    let prefix = format!("{}{}", root.display(), std::path::MAIN_SEPARATOR);
    text.replace(&prefix, "")
}

/// jwc で `entry` から実行ファイル `exe` を作る（パッケージマップは exe の隣に書く）。失敗なら jwc のエラーの文
fn jwc_build(p: &jabberwock::project::Project, jwc: &Path, entry: &Path, exe: &Path, release: bool) -> Result<(), String> {
    let map_text = p.package_map(entry)?;
    if let Some(d) = exe.parent() {
        fs::create_dir_all(d).map_err(|e| format!("{}: {}", d.display(), e))?;
    }
    let map = exe.with_extension("map");
    fs::write(&map, map_text).map_err(|e| format!("{}: {}", map.display(), e))?;
    let mut cmd = process::Command::new(jwc);
    cmd.arg("build");
    if release {
        cmd.arg("--release");
    }
    cmd.arg("--package-map").arg(&map).arg("--clang").arg(jabberwock::native::clang()).arg("-o").arg(exe);
    let o = cmd.stdin(process::Stdio::null()).output().map_err(|e| format!("jwc を起動できません: {}: {}", jwc.display(), e))?;
    let err = relative_in(p, &String::from_utf8_lossy(&o.stderr));
    if !o.status.success() {
        return Err(if err.is_empty() { format!("jwc が失敗しました（{}）\n", o.status) } else { err });
    }
    // jwc 自身の「作りました」は出さない（alice が出す）
    for line in err.lines() {
        if !line.starts_with("作りました: ") {
            eprintln!("{}", line);
        }
    }
    Ok(())
}

/// jwc の build の出力（Rust 版の target/debug と混ぜない）
fn jwc_exe_path(p: &jabberwock::project::Project, release: bool) -> PathBuf {
    let rust = p.exe_path(release);
    let name = rust.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    p.root.join("target").join("jwc").join(if release { "release" } else { "debug" }).join(name)
}

fn exe_of(p: &jabberwock::project::Project, release: bool, c: &Compiler) -> (PathBuf, bool) {
    match c {
        Compiler::Rust => {
            let exe = p.exe_path(release);
            let stale = p.stale(&exe);
            (exe, stale)
        }
        Compiler::Jwc(j) => {
            let exe = jwc_exe_path(p, release);
            let stale = p.stale_with(&exe, std::slice::from_ref(j));
            (exe, stale)
        }
    }
}

fn main() {
    jabberwock::main_in_big_stack(real_main);
}

fn real_main() {
    let rest: Vec<String> = env::args().skip(2).collect();
    match env::args().nth(1).as_deref() {
        Some("new") => project_new(rest),
        Some("build") => {
            let (c, rest) = take_compiler(rest);
            project_build(rest, c)
        }
        Some("run") => {
            let (c, rest) = take_compiler(rest);
            project_run(rest, c)
        }
        Some("test") => {
            let (c, rest) = take_compiler(rest);
            project_test(rest, c)
        }
        Some("package-map") => project_package_map(rest),
        Some("-h") | Some("--help") => eprintln!("{}", USAGE),
        _ => {
            eprintln!("{}", USAGE);
            process::exit(2);
        }
    }
}

fn project_package_map(args: Vec<String>) {
    let p = current_project();
    let mut output = None;
    let mut entry = None;
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        if a == "-o" && output.is_none() {
            output = it.next();
            if output.is_some() { continue; }
        } else if !a.starts_with('-') && entry.is_none() {
            entry = Some(std::path::PathBuf::from(a));
            continue;
        }
        eprintln!("使い方: alice package-map [-o ファイル] [入口.jw]");
        process::exit(2);
    }
    let entry = entry.unwrap_or_else(|| p.main_file());
    let result = p.package_map(&entry).and_then(|text| match output {
        Some(path) => fs::write(&path, text).map_err(|e| format!("{}: {}", path, e)),
        None => { print!("{}", text); Ok(()) }
    });
    if let Err(e) = result { eprintln!("{}", e); process::exit(1); }
}

/// 今のフォルダから上へたどってパッケージを読む（なければ終わる）
fn current_project() -> jabberwock::project::Project {
    let cwd = env::current_dir().unwrap_or_default();
    let Some(root) = jabberwock::project::Project::find(&cwd) else {
        eprintln!("{} が見つかりません（このフォルダか上のフォルダに要ります。`alice new 名前` で作れます）", jabberwock::project::MANIFEST);
        process::exit(1);
    };
    match jabberwock::project::Project::load(&root) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}", e);
            process::exit(1);
        }
    }
}

/// パッケージでのエラーの表示（モジュールのファイルはパッケージからの道筋で出す）
fn render_in(p: &jabberwock::project::Project, d: &jabberwock::diag::Diagnostic, main: &str, files: &[(String, String)]) -> String {
    jabberwock::render_named(d, main, files, &|m| match p.modules.get(m) {
        Some(f) => shown(p, f),
        None => format!("{}.jw", m.trim_start_matches('*')),
    })
}

/// パッケージの中のファイルを、表示用にパッケージからの道筋で書く
fn shown(p: &jabberwock::project::Project, f: &std::path::Path) -> String {
    f.strip_prefix(&p.root).unwrap_or(f).display().to_string()
}

fn read_or_exit(f: &std::path::Path) -> String {
    match fs::read_to_string(f) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("ファイルを読めません: {}: {}", f.display(), e);
            process::exit(1);
        }
    }
}

/// `alice new 名前 [--lib]`
fn project_new(args: Vec<String>) {
    let usage = "使い方: alice new <名前> [--lib]";
    let mut lib = false;
    let mut name = None;
    for a in args {
        match a.as_str() {
            "--lib" => lib = true,
            _ if name.is_none() && !a.starts_with('-') => name = Some(a),
            _ => {
                eprintln!("{}", usage);
                process::exit(2);
            }
        }
    }
    let Some(name) = name else {
        eprintln!("{}", usage);
        process::exit(2);
    };
    match jabberwock::project::create(std::path::Path::new(&name), lib) {
        Ok(()) => eprintln!("作りました: {}（{}）", name, if lib { "ライブラリ" } else { "実行ファイル" }),
        Err(e) => {
            eprintln!("{}", e);
            process::exit(1);
        }
    }
}

/// パッケージの src/main.jw から実行ファイルを作る（新しければ作り直さない）。作った場所を返す
fn project_exe(p: &jabberwock::project::Project, release: bool, c: &Compiler) -> std::path::PathBuf {
    let main = p.main_file();
    if !main.is_file() {
        eprintln!("{} がありません（ライブラリは build できません。tests は `alice test` で動かせます）", shown(p, &main));
        process::exit(1);
    }
    let (exe, stale) = exe_of(p, release, c);
    if !stale {
        return exe;
    }
    if let Compiler::Jwc(j) = c {
        if let Err(e) = jwc_build(p, j, &main, &exe, release) {
            eprint!("{}", e);
            process::exit(1);
        }
        eprintln!("作りました: {}", shown(p, &exe));
        return exe;
    }
    let src = read_or_exit(&main);
    let read = p.reader();
    let name = shown(p, &main);
    let (warnings, files) = jabberwock::warnings(&src, &read);
    for w in warnings {
        eprint!("{}", render_in(p, &w, &name, &files));
    }
    let ll = match jabberwock::llvm_build(&src, &read, &name, release) {
        Ok(t) => t,
        Err(e) => {
            eprint!("{}", e);
            process::exit(1);
        }
    };
    if let Some(d) = exe.parent() {
        let _ = fs::create_dir_all(d);
    }
    match jabberwock::native::build(&ll, &exe, if release { "-O2" } else { "-O0" }) {
        Ok(e) => {
            eprintln!("作りました: {}", shown(p, &e));
            e
        }
        Err(e) => {
            eprintln!("{}", e);
            if e.starts_with("clang を起動できません") {
                eprintln!("clang が要ります。入れ方は docs/native.md を見てください（別の clang を使うなら JW_CLANG）");
            }
            process::exit(1);
        }
    }
}

/// `alice build [--release]`
fn project_build(args: Vec<String>, c: Compiler) {
    let mut release = false;
    for a in args {
        match a.as_str() {
            "--release" => release = true,
            _ => {
                eprintln!("使い方: alice build [--release]");
                process::exit(2);
            }
        }
    }
    let p = current_project();
    let fresh = !exe_of(&p, release, &c).1;
    let exe = project_exe(&p, release, &c);
    if fresh {
        eprintln!("最新です: {}", shown(&p, &exe));
    }
}

/// `alice run [--native] [--release] [-- 引数...]`
fn project_run(args: Vec<String>, c: Compiler) {
    let mut native = false;
    let mut release = false;
    let mut prog_args = Vec::new();
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--native" => native = true,
            "--release" => {
                native = true;
                release = true;
            }
            "--" => {
                prog_args.extend(it.by_ref());
            }
            _ => {
                eprintln!("使い方: alice run [--native] [--release] [-- プログラムへの引数...]");
                process::exit(2);
            }
        }
    }
    if let (Compiler::Jwc(_), false) = (&c, native) {
        eprintln!("--compiler jwc の run には --native が要ります（jwc は実行ファイルを作るだけで、インタプリタがありません）");
        process::exit(2);
    }
    let p = current_project();
    if native {
        let exe = project_exe(&p, release, &c);
        let status = process::Command::new(&exe).args(&prog_args).status();
        match status {
            Ok(s) => process::exit(s.code().unwrap_or(1)),
            Err(e) => {
                eprintln!("{} を動かせません: {}", exe.display(), e);
                process::exit(1);
            }
        }
    }
    let main = p.main_file();
    if !main.is_file() {
        eprintln!("{} がありません（ライブラリは run できません。tests は `alice test` で動かせます）", shown(&p, &main));
        process::exit(1);
    }
    let src = read_or_exit(&main);
    let read = p.reader();
    let name = shown(&p, &main);
    let (warnings, files) = jabberwock::warnings(&src, &read);
    for w in warnings {
        eprint!("{}", render_in(&p, &w, &name, &files));
    }
    let input = Box::new(std::io::BufReader::new(std::io::stdin()));
    let outcome = jabberwock::run_program(&src, &read, true, true, input, prog_args);
    if let Some(d) = &outcome.error {
        eprint!("{}", render_in(&p, d, &name, &outcome.files));
        process::exit(1);
    }
    if let Some(code) = outcome.exit_code {
        process::exit(code);
    }
}

/// テスト 1 本: (表示の名前, 動かすソースか、動かせない理由, 期待する出力とそのファイルの名前)
type Case = (String, Result<String, String>, Option<(String, String)>);

/// `alice test [名前の一部]`: tests/*.jw を interp で動かす。
/// main のあるファイルは 1 本のテスト（tests/名前.out があれば出力も比べる）。main のないファイルは、`fn test_名前()` を 1 本ずつ別に動かす。
/// エラー（assert の失敗も）か 0 でない exit で失敗
fn project_test(args: Vec<String>, c: Compiler) {
    let native = args.iter().any(|a| a == "--native");
    let args: Vec<String> = args.into_iter().filter(|a| a != "--native").collect();
    if args.len() > 1 || args.iter().any(|a| a.starts_with('-')) {
        eprintln!("使い方: alice test [--native] [--compiler rust|jwc] [--jwc 実体] [名前の一部]");
        process::exit(2);
    }
    if let (Compiler::Jwc(_), false) = (&c, native) {
        eprintln!("--compiler jwc の test には --native が要ります（jwc は実行ファイルを作るだけで、インタプリタがありません）");
        process::exit(2);
    }
    let filter = args.first().cloned();
    let p = current_project();
    let tests = p.tests();
    if tests.is_empty() {
        eprintln!("テストがありません（tests/*.jw）");
        return;
    }
    let read = p.reader();
    let (mut ok, mut failed) = (0, Vec::new());
    let mut ran = 0;
    // 1 本動かして、失敗ならその理由
    let work = p.root.join("target").join("alice-test").join(if let Compiler::Jwc(_) = c { "jwc" } else { "rust" });
    // --native: 実行ファイルを作って動かす（入力は空。stderr に何か出るか 0 でない終わり方なら失敗）
    let run_native = |src: &str, t: &Path, case: &str, want: Option<(String, String)>| -> Option<String> {
        let file = shown(&p, t);
        let exe = work.join(format!("{}{}", case.replace("::", "__").replace(&['/', '\\', '.'][..], "_"), env::consts::EXE_SUFFIX));
        if let Some(d) = exe.parent() {
            if let Err(e) = fs::create_dir_all(d) {
                return Some(format!("{}: {}\n", d.display(), e));
            }
        }
        // jwc にはファイルで渡す（関数ごとのテストは main を足した写しを作り、エラーの位置は元のファイルの名前で見せる）
        let mut copy = None;
        let built = match &c {
            Compiler::Jwc(j) => {
                let entry = if src == read_or_exit(t) {
                    t.to_path_buf()
                } else {
                    let e = exe.with_extension("jw");
                    if let Err(err) = fs::write(&e, src) {
                        return Some(format!("{}: {}\n", e.display(), err));
                    }
                    copy = Some(e.clone());
                    e
                };
                jwc_build(&p, j, &entry, &exe, false)
            }
            Compiler::Rust => jabberwock::llvm_build(src, &read, &file, false)
                .and_then(|ll| jabberwock::native::build(&ll, &exe, "-O0").map(|_| ())),
        };
        let fix = |text: &str| -> String {
            let mut t = text.to_string();
            if let Some(e) = &copy {
                t = t.replace(&e.canonicalize().unwrap_or_else(|_| e.clone()).display().to_string(), &file);
            }
            relative_in(&p, &t)
        };
        if let Err(e) = built {
            return Some(fix(&e));
        }
        let o = match process::Command::new(&exe).stdin(process::Stdio::null()).output() {
            Ok(o) => o,
            Err(e) => return Some(format!("{} を動かせません: {}\n", exe.display(), e)),
        };
        let out = String::from_utf8_lossy(&o.stdout).to_string();
        let err = fix(&String::from_utf8_lossy(&o.stderr));
        if !o.status.success() || !err.is_empty() {
            let code = o.status.code().map(|c| format!("exit({}) で終わりました\n", c)).unwrap_or_else(|| format!("{}\n", o.status));
            return Some(if err.is_empty() { code } else { err });
        }
        match want {
            Some((w, wname)) if w != out => Some(format!("出力が違います\n--- 期待（{}）\n{}--- 実際\n{}", wname, w, out)),
            _ => None,
        }
    };
    let run = |src: &str, name: &str, want: Option<(String, String)>| -> Option<String> {
        let input = Box::new(std::io::Cursor::new(Vec::new()));
        let o = jabberwock::run_program(src, &read, false, true, input, Vec::new());
        if let Some(d) = &o.error {
            Some(render_in(&p, d, name, &o.files))
        } else if o.exit_code.unwrap_or(0) != 0 {
            Some(format!("exit({}) で終わりました\n", o.exit_code.unwrap_or(0)))
        } else {
            match want {
                Some((w, wname)) if w != o.output => Some(format!("出力が違います\n--- 期待（{}）\n{}--- 実際\n{}", wname, w, o.output)),
                _ => None,
            }
        }
    };
    for t in &tests {
        let file = shown(&p, t);
        let src = read_or_exit(t);
        let mut cases: Vec<Case> = Vec::new();
        match jabberwock::project::test_file(&src) {
            Err(e) => cases.push((file.clone(), Err(e), None)),
            Ok(tf) if tf.has_main => {
                if !tf.fns.is_empty() {
                    cases.push((file.clone(), Err("main のあるファイルに `fn test_...()` は書けません（関数ごとのテストは main なしのファイルに）".to_string()), None));
                } else {
                    let out = t.with_extension("out");
                    let want = fs::read_to_string(&out).ok().map(|w| (w, shown(&p, &out)));
                    cases.push((file.clone(), Ok(src.clone()), want));
                }
            }
            Ok(tf) if tf.fns.is_empty() => {
                cases.push((file.clone(), Err("`fn main()` も `fn test_名前()` もありません".to_string()), None))
            }
            Ok(tf) => {
                for f in tf.fns {
                    // 最後に main を足して、その関数だけ呼ぶ（位置は元のファイルのまま）
                    cases.push((format!("{}::{}", file, f), Ok(format!("{}\nfn main() {{\n    {}();\n}}\n", src, f)), None));
                }
            }
        }
        for (name, src, want) in cases {
            if let Some(f) = &filter {
                if !name.contains(f.as_str()) {
                    continue;
                }
            }
            ran += 1;
            let problem = match src {
                Err(e) => Some(format!("{}\n", e)),
                Ok(src) if native => run_native(&src, t, &name, want),
                Ok(src) => run(&src, &file, want),
            };
            match problem {
                None => {
                    println!("test {} ... ok", name);
                    ok += 1;
                }
                Some(why) => {
                    println!("test {} ... 失敗", name);
                    failed.push((name, why));
                }
            }
        }
    }
    if ran == 0 {
        println!("`{}` に当てはまるテストがありません", filter.unwrap_or_default());
        process::exit(1);
    }
    for (name, why) in &failed {
        println!("\n---- {} ----\n{}", name, why);
    }
    println!("\n結果: {} 本成功・{} 本失敗", ok, failed.len());
    if !failed.is_empty() {
        process::exit(1);
    }
}
