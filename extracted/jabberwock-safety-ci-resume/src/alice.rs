//! `alice`: jabberwock のパッケージの道具（cargo を小さくしたもの）。説明は docs/packages.md、中身は src/project.rs
use std::{env, fs, process};

const USAGE: &str = "使い方: alice <コマンド>
  new <名前> [--lib]              新しいパッケージを作る（--lib はライブラリ）
  build [--release]               target/debug（release）に実行ファイルを作る（clang が要る）
  package-map [-o ファイル] [入口.jw]  jwc用のローカル依存情報を書き出す
  run [--native] [-- 引数...]      src/main.jw を動かす（--native は実行ファイルを作って動かす）
  test [名前の一部]                tests/*.jw を動かす（main のないファイルは fn test_名前() を 1 本ずつ。tests/名前.out があれば出力と比べる）";

fn main() {
    jabberwock::main_in_big_stack(real_main);
}

fn real_main() {
    let rest: Vec<String> = env::args().skip(2).collect();
    match env::args().nth(1).as_deref() {
        Some("new") => project_new(rest),
        Some("build") => project_build(rest),
        Some("run") => project_run(rest),
        Some("test") => project_test(rest),
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
fn project_exe(p: &jabberwock::project::Project, release: bool) -> std::path::PathBuf {
    let main = p.main_file();
    if !main.is_file() {
        eprintln!("{} がありません（ライブラリは build できません。tests は `alice test` で動かせます）", shown(p, &main));
        process::exit(1);
    }
    let exe = p.exe_path(release);
    if !p.stale(&exe) {
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
fn project_build(args: Vec<String>) {
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
    let exe = p.exe_path(release);
    let fresh = !p.stale(&exe);
    let exe = project_exe(&p, release);
    if fresh {
        eprintln!("最新です: {}", shown(&p, &exe));
    }
}

/// `alice run [--native] [--release] [-- 引数...]`
fn project_run(args: Vec<String>) {
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
    let p = current_project();
    if native {
        let exe = project_exe(&p, release);
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
fn project_test(args: Vec<String>) {
    if args.len() > 1 || args.iter().any(|a| a.starts_with('-')) {
        eprintln!("使い方: alice test [名前の一部]");
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
