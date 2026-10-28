//! LLVM の書き出し（`ir::llvm`）から、clang で実行ファイルを作る。
//!
//! 実行時ライブラリ `runtime/jw_rt.c` はコンパイラの中に入れてあり、作るたびに書き出して一緒にコンパイルする。
//! clang は `JW_CLANG` で別のものを指定できる（なければ `clang`）。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

/// 実行時ライブラリのソース
pub const RUNTIME_C: &str = include_str!("../runtime/jw_rt.c");

pub fn clang() -> String {
    std::env::var("JW_CLANG").unwrap_or_else(|_| "clang".to_string())
}

fn exe_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{}.exe", name)
    } else {
        name.to_string()
    }
}

/// clang に足す引数（環境変数 `JW_CFLAGS`。空白で区切る）。堅牢性のテストで `-fsanitize=address,undefined` を足すため
fn extra_flags() -> Vec<String> {
    std::env::var("JW_CFLAGS").map(|v| v.split_whitespace().map(|s| s.to_string()).collect()).unwrap_or_default()
}

/// `JW_CFLAGS` に sanitizer（address / thread / memory）があれば、書き出した関数にも印をつけて検査させる
/// （`.ll` の関数は印がないと検査されない。C の実行時ライブラリは clang がそのまま検査する）
fn instrument(ll: &str) -> std::borrow::Cow<'_, str> {
    let flags = extra_flags();
    let attrs: Vec<&str> = [("address", "sanitize_address"), ("thread", "sanitize_thread"), ("memory", "sanitize_memory")]
        .iter()
        .filter(|(k, _)| flags.iter().any(|f| f.starts_with("-fsanitize=") && f[11..].split(',').any(|x| x == *k)))
        .map(|(_, a)| *a)
        .collect();
    if attrs.is_empty() {
        std::borrow::Cow::Borrowed(ll)
    } else {
        let mut ll = ll.replace(") {\nentry:", &format!(") {} {{\nentry:", attrs.join(" ")));
        if attrs.contains(&"sanitize_memory") {
            // clang が作る C の関数は noundef の引数を受け取る。宣言にも同じ約束を付け、
            // MSan に呼び出しの手前で検査させる（ないと C 側が引数を初期化済みと見なす）。
            ll = ll.lines().map(|line| {
                if line.starts_with("declare ") && !line.contains("@llvm.") {
                    if let Some((head, rest)) = line.split_once('(') {
                        if let Some((params, tail)) = rest.split_once(')') {
                            let params = params.split(", ").map(|p| {
                                if p.is_empty() || p == "..." || p.contains("noundef") { p.to_string() }
                                else { format!("{} noundef", p) }
                            }).collect::<Vec<_>>().join(", ");
                            return format!("{}({}){}", head, params, tail);
                        }
                    }
                }
                line.to_string()
            }).collect::<Vec<_>>().join("\n");
        }
        std::borrow::Cow::Owned(ll)
    }
}

fn run_clang(args: &[&str]) -> Result<(), String> {
    let out = Command::new(clang())
        .args(args)
        .args(extra_flags())
        .output()
        .map_err(|e| format!("clang を起動できません（{}）: {}", clang(), e))?;
    if !out.status.success() {
        return Err(format!("clang が失敗しました:\n{}", String::from_utf8_lossy(&out.stderr)));
    }
    Ok(())
}

/// `.ll` のテキストと実行時ライブラリをコンパイルして、`dir` に実行ファイル `name` を作る。`opt` は `-O2` など
pub fn compile(ll: &str, dir: &Path, name: &str, opt: &str) -> Result<PathBuf, String> {
    let exe = dir.join(exe_name(name));
    compile_to(ll, dir, name, &exe, opt)?;
    Ok(exe)
}

/// `jabberwock build`: 途中のファイルは一時フォルダに置き、実行ファイルを `out` に作る（Windows で `.exe` がなければ足す）
pub fn build(ll: &str, out: &Path, opt: &str) -> Result<PathBuf, String> {
    let mut out = out.to_path_buf();
    if cfg!(windows) && out.extension().is_none() {
        out.set_extension("exe");
    }
    let dir = std::env::temp_dir().join(format!("jw-build-{}", std::process::id()));
    let r = compile_to(ll, &dir, "main", &out, opt);
    let _ = fs::remove_dir_all(&dir);
    r.map(|_| out)
}

fn compile_to(ll: &str, dir: &Path, name: &str, exe: &Path, opt: &str) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let llp = dir.join(format!("{}.ll", name));
    let rt = dir.join("jw_rt.c");
    fs::write(&llp, &*instrument(ll)).map_err(|e| e.to_string())?;
    fs::write(&rt, RUNTIME_C).map_err(|e| e.to_string())?;
    let mut args = vec![opt, path_str(&llp)?, path_str(&rt)?, "-o", path_str(exe)?, "-Wno-override-module"];
    link_flags(&mut args);
    // LTO is needed for the release (-O2/-O3) path, but not for
    // development (-O0/-O1). A large self-hosted compiler generates
    // substantial LLVM IR: whole-program LTO at -O1 dominates its build time.
    // Keep the -O1 runtime and output checks while avoiding unnecessary LTO.
    if opt == "-O0" || opt == "-O1" {
        return run_clang(&args);
    }
    let mut lto = args.clone();
    lto.extend(["-flto", "-fuse-ld=lld"]);
    run_clang(&lto).or_else(|_| run_clang(&args))
}

/// Windows では引数を読むのに shell32 が要る。ほかは数学の関数（libm）
fn link_flags(args: &mut Vec<&str>) {
    if cfg!(windows) {
        args.push("-lshell32");
        // jwc が大きな構文木を再帰でたどるためのスタック。
        let target = Command::new(clang()).arg("-dumpmachine").output().ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
        args.push(if target.contains("gnu") { "-Wl,--stack,268435456" } else { "-Wl,/stack:268435456" });
    } else {
        args.push("-lm");
        args.push("-pthread");
    }
}

fn path_str(p: &Path) -> Result<&str, String> {
    p.to_str().ok_or_else(|| format!("パスが UTF-8 ではありません: {}", p.display()))
}

static RUNTIME_OBJECT: Mutex<Option<(bool, PathBuf)>> = Mutex::new(None);

/// テストの突き合わせ用: 実行時ライブラリを一度だけコンパイルしておき、`.ll` だけをコンパイルしてつなぐ。
/// ふだんは速く作れる `-O1`。`JW_NATIVE=o2` なら `jabberwock build` と同じ `-O2` と LTO で作る（最適化に頼る書き出しの確かめ）
pub fn compile_with_cached_runtime(ll: &str, dir: &Path, name: &str) -> Result<PathBuf, String> {
    let o2 = std::env::var("JW_NATIVE").map(|v| v == "o2").unwrap_or(false);
    let opt: &[&str] = if o2 { &["-O2", "-flto"] } else { &["-O1"] };
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let obj = {
        let mut g = RUNTIME_OBJECT.lock().unwrap_or_else(|e| e.into_inner());
        // Callers may delete their own test directories; keep the cached object
        // outside those directories so subsequent links cannot use a stale path.
        if g.as_ref().map_or(true, |(cached_o2, path)| *cached_o2 != o2 || !path.exists()) {
            let cache_dir = std::env::temp_dir().join(format!("jw-runtime-cache-{}", std::process::id()));
            fs::create_dir_all(&cache_dir).map_err(|e| e.to_string())?;
            let rt = cache_dir.join("jw_rt.c");
            let o = cache_dir.join(if o2 { "jw_rt_o2.o" } else { "jw_rt_o1.o" });
            fs::write(&rt, RUNTIME_C).map_err(|e| e.to_string())?;
            let mut args = vec!["-c", path_str(&rt)?, "-o", path_str(&o)?];
            args.extend(opt);
            run_clang(&args)?;
            *g = Some((o2, o));
        }
        g.as_ref().expect("compiled").1.clone()
    };
    let llp = dir.join(format!("{}.ll", name));
    fs::write(&llp, &*instrument(ll)).map_err(|e| e.to_string())?;
    let exe = dir.join(exe_name(name));
    let mut args = vec![path_str(&llp)?, path_str(&obj)?, "-o", path_str(&exe)?, "-Wno-override-module"];
    args.extend(opt);
    if o2 {
        args.push("-fuse-ld=lld");
    }
    link_flags(&mut args);
    run_clang(&args)?;
    Ok(exe)
}
