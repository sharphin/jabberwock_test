//! 堅牢性のテスト。壊れた入力・極端な入力を与えても、処理系が落ちない（panic・スタックあふれ・内部エラーにならない）ことを確かめる。
//! 正しくエラーを出して止まるのはよい（終了コード 1）。プログラムがいつまでも回るのも、時間で打ち切ってよしとする。
//!
//! 2 種類の入力を使う。
//! - `HOSTILE`: 手で集めた極端な入力（深い入れ子・巨大な数・巨大な配列や文字列など）。見つかった穴をここに足していく
//! - 変異: examples/*.jw と tests/jwc_cases.txt のプログラムを、種から決まる手順で壊したもの（消す・重ねる・極端な部品を差し込む）
//!
//! - `ATTACKS`: メモリ破壊・情報漏洩をねらうプログラム（解放した区画を使い回させてから古い ref で読む・範囲の外を読む・ほかのスレッドへ送った値を読む など）。
//!   必ず決まったエラーで止まり、出力に秘密の値（`SECRET`）が出ないことを、interp・IR・実行ファイルで確かめる
//!
//! 落ちないかは、コマンド（`jabberwock --ir` と `jabberwock`）を別のプロセスで動かして見る（スタックあふれは同じプロセスでは受け止められないため）。
//! ふだんは `CASES` 本。`JW_ROBUST=本数`・`JW_ROBUST_SEED=始めの種`・`JW_ROBUST_NATIVE=1`（実行ファイルも作って動かす。clang が要る）・
//! `JW_ROBUST_DUMP=フォルダ`（落ちた入力を書き出す）・`JW_ROBUST_ASAN=1`（実行ファイルを AddressSanitizer と UBSan つきで作り、
//! 実行時ライブラリと書き出したコードの範囲外・解放後の読み書き・未定義の動作を見つける。clang の compiler-rt が要る）。
//! 落ちた入力は、種と一緒に失敗の文に出るので、HOSTILE に足して直す。

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const CASES: u64 = 150;
/// 1 回の実行の時間の上限（プログラムが回り続けるのは失敗にしない）
const TIMEOUT: Duration = Duration::from_secs(3);

/// xorshift の乱数（外部クレートなし。fuzz.rs と同じ）
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// 差し込む部品（壊れやすいところを突く）
const PIECES: &[&str] = &[
    "(", ")", "{", "}", "[", "]", ";", ",", ".", "own ", "ref ", "ref mut ", "*", "-", "!", "~", "?", "->", ">>", "<", ">",
    "9223372036854775807", "-9223372036854775808", "18446744073709551615", "340282366920938463463374607431768211456",
    "1e308", "0.0 / 0.0", "1 / 0", "0", "\"", "\"{", "}\"", "\"\"\"", "\\u{110000}", "\u{feff}", "é", "日本",
    "match x { ", "case ", "default -> ", "fn ", "struct ", "enum ", "interface ", "const ", "use ", "pub ",
    "as u8", "as i64", ".repeat(9223372036854775807)", "[0; 9223372036854775807]", "[0; 131072]", "spawn(() -> 1)",
    ".push(1)", ".pop()", ".sort()", ".len()", "chan()", "drop(", "assert(false)", "exit(3)", "T", "<T>", "Option<", "Some(",
    "break", "continue", "return", "loop {", "while true {", "for i in range(", "if ", "else ", "x++", "x += ",
];

/// 手で集めた極端な入力
fn hostile() -> Vec<String> {
    let deep = |open: &str, mid: &str, close: &str, n: usize| format!("{}{}{}", open.repeat(n), mid, close.repeat(n));
    let n = 100_000;
    vec![
        format!("fn main() {{ print({}); }}", deep("(", "1", ")", n)),
        format!("fn main() {{ print({}1); }}", "- ".repeat(n)),
        format!("fn main() {{ print({}true); }}", "!".repeat(n)),
        format!("fn main() {{ {} }}", deep("if true { ", "", "}", n)),
        format!("fn main() {{ {} }}", deep("loop { ", "break;", "}", n)),
        format!("fn main() {{ let x = {}; }}", deep("[", "1", "]", n)),
        format!("fn main() {{ let x = {}; }}", deep("Some(", "1", ")", n)),
        format!("fn main() {{ match o {{ case {} -> 1; }} }}", deep("Some(", "x", ")", n)),
        format!("fn f({}i64 x) {{}}\nfn main() {{}}", "own ".repeat(n)),
        format!("fn f({} x) {{}}\nfn main() {{}}", deep("Option<", "i64", ">", n)),
        format!("fn f(i64{} x) {{}}\nfn main() {{}}", "[]".repeat(n)),
        format!("fn main() {{ let f = {}1; }}", "() -> ".repeat(n)),
        format!("fn main() {{ print(\"{}\"); }}", deep("{\"", "", "\"}", 1000)),
        format!("fn main() {{ print(1{}); }}", " + 1".repeat(n)),
        format!("fn main() {{ let x = 1; print(x{}); }}", ".abs()".repeat(n)),
        format!("fn main() {{ {}}}", (0..20000).map(|i| format!("let a{} = {}; ", i, i)).collect::<String>()),
        "fn main() { print(\"abc\".repeat(9223372036854775807)); }".to_string(),
        "fn main() { print(\"\".repeat(9223372036854775807)); }".to_string(),
        "fn main() { let z = [0; 9223372036854775807]; }".to_string(),
        "fn main() { let i64[] z = [0; 9223372036854775807]; }".to_string(),
        // 上限（2^59）の中でも確保できない大きさ（interp が確保で落ちていた）
        "fn main() { let i64[] z = [0; 9236854775807]; }".to_string(),
        "fn main() { let z = [[[0; 1000]; 1000]; 1000]; }".to_string(),
        "fn main() { print(340282366920938463463374607431768211456); }".to_string(),
        "fn main() { print(-9223372036854775808 / -1, -9223372036854775808 % -1); }".to_string(),
        "fn main() { let i64 a = -9223372036854775807 - 1; print(a / -1, a.abs()); }".to_string(),
        "fn main() { print(1 << 64, 1 >> 64, 1 << -1); }".to_string(),
        "fn main() { let s = 70; print(1 << s, 1 >> s); }".to_string(),
        "fn main() { print(1e308 * 10.0, (0.0 / 0.0) as i64, 1e300 as u8); }".to_string(),
        "fn main() { print(\"日本\".slice(0, 9223372036854775807), \"ab\".slice(-1, 1)); }".to_string(),
        "fn main() { let a = [1, 2]; print(a[9223372036854775807]); }".to_string(),
        "fn main() { let mut l = [1]; l.insert(9223372036854775807, 1); }".to_string(),
        "fn main() { let mut l = [1]; l.remove(-1); }".to_string(),
        "fn main() { for i in range(0, 9223372036854775807, 9223372036854775807) { print(i); } }".to_string(),
        "fn main() { for i in range(9223372036854775806, 9223372036854775807) { print(i); } }".to_string(),
        "fn main() { print(chr(1114112), chr(-1), chr(55296)); }".to_string(),
        "fn f() { f(); }\nfn main() { f(); }".to_string(),
        "fn main() { let c = chan(); c.recv(); }".to_string(),
        "fn main() { exit(9223372036854775807); }".to_string(),
        format!("fn main() {{ let {} = 1; }}", "x".repeat(n)),
        format!("fn main() {{ print({}); }}", vec!["1"; 20_000].join(", ")),
        format!("const A0 = 1;\n{}fn main() {{ print(A{}); }}", (1..2000).map(|i| format!("const A{} = A{} + A{};\n", i, i - 1, i - 1)).collect::<String>(), 1999),
        "\u{feff}fn main() { print(1); }".to_string(),
        "fn main() { print(\"\\u{110000}\"); }".to_string(),
        "fn main() { print(\"\\u{d800}\"); }".to_string(),
        "fn main() { /* ".to_string(),
        "fn main() { \"".to_string(),
        "\"\"\"".to_string(),
        String::new(),
    ]
}

/// プログラムを 1 つ壊す（1〜3 回）
fn mutate(src: &str, rng: &mut Rng) -> String {
    let mut s: Vec<char> = src.chars().collect();
    for _ in 0..1 + rng.below(3) {
        let n = s.len();
        let at = rng.below(n + 1);
        match rng.below(6) {
            // 消す
            0 if n > 0 => {
                let len = 1 + rng.below(20.min(n - at.min(n - 1)));
                let at = at.min(n - 1);
                s.drain(at..(at + len).min(n));
            }
            // 重ねる
            1 if n > 0 => {
                let at = at.min(n - 1);
                let end = (at + 1 + rng.below(40)).min(n);
                let piece: Vec<char> = s[at..end].to_vec();
                let times = if rng.below(4) == 0 { 50 + rng.below(500) } else { 1 };
                for _ in 0..times {
                    s.splice(at..at, piece.iter().cloned());
                }
            }
            // 数を極端な数にする
            2 => {
                if let Some(i) = (0..n).map(|k| (at + k) % n.max(1)).find(|&i| s[i].is_ascii_digit()) {
                    let mut j = i;
                    while j < n && s[j].is_ascii_digit() {
                        j += 1;
                    }
                    let big = ["9223372036854775807", "0", "18446744073709551615", "131073", "-1"][rng.below(5)];
                    s.splice(i..j, big.chars());
                }
            }
            // 途中で切る
            3 => s.truncate(at),
            // 部品を差し込む
            _ => {
                let p = PIECES[rng.below(PIECES.len())];
                s.splice(at..at, p.chars());
            }
        }
    }
    s.into_iter().collect()
}

/// 種のプログラム（ファイルやコマンドを触るものは除く）
fn seeds() -> Vec<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    let mut files: Vec<PathBuf> =
        std::fs::read_dir(root.join("examples")).unwrap().filter_map(|e| e.ok().map(|e| e.path())).collect();
    files.sort();
    for f in files {
        if f.extension().and_then(|e| e.to_str()) == Some("jw") {
            out.push(std::fs::read_to_string(&f).unwrap());
        }
    }
    let cases = std::fs::read_to_string(root.join("tests/jwc_cases.txt")).unwrap();
    out.extend(cases.split("\n#####\n").map(|c| c.to_string()));
    out.retain(|s| !s.contains("run_command") && !s.contains("write_file") && !s.contains("read_file") && !s.contains("use "));
    out
}

#[path = "support/attacks.rs"]
mod attacks;
use attacks::{ATTACKS, SECRET};

/// 落ちたか（落ちたならその様子）。時間切れは、打ち切ってよしとする
fn crashed(cmd: &mut Command) -> Option<String> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().expect("runs");
    let mut err = child.stderr.take().expect("piped");
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err.read_to_end(&mut buf);
        String::from_utf8_lossy(&buf).to_string()
    });
    let start = Instant::now();
    let status = loop {
        if let Some(st) = child.try_wait().expect("waits") {
            break Some(st);
        }
        if start.elapsed() > TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let stderr = reader.join().unwrap_or_default();
    let status = status?;
    let bad = ["panicked at", "overflowed its stack", "内部エラー", "RUST_BACKTRACE", "fatal runtime error", "Sanitizer", "runtime error:"];
    if let Some(b) = bad.iter().find(|b| stderr.contains(*b)) {
        return Some(format!("{}: {}", b, stderr.lines().take(6).collect::<Vec<_>>().join(" / ")));
    }
    match status.code() {
        None => Some(format!("シグナルで止まった（{:?}）", status)),
        Some(101) => Some("終了コード 101（panic）".to_string()),
        _ => None,
    }
}

/// 1 つの入力を、型検査・IR まで（`--ir`）と、実行（`jabberwock`）で試す。clang があれば実行ファイルも
fn check(src: &str, dir: &Path, name: &str, native: bool) -> Option<String> {
    let f = dir.join(format!("{}.jw", name));
    std::fs::write(&f, src).unwrap();
    let jw = env!("CARGO_BIN_EXE_jabberwock");
    if let Some(why) = crashed(Command::new(jw).arg("--ir").arg(&f).current_dir(dir)) {
        return Some(format!("--ir: {}", why));
    }
    if let Some(why) = crashed(Command::new(jw).arg(&f).current_dir(dir)) {
        return Some(format!("実行: {}", why));
    }
    if native {
        let exe = dir.join(name);
        if let Some(why) = crashed(build_cmd(jw, &exe, &f).current_dir(dir)) {
            return Some(format!("build: {}", why));
        }
        if exe.exists() {
            if let Some(why) = crashed(Command::new(&exe).current_dir(dir)) {
                return Some(format!("実行ファイル: {}", why));
            }
            let _ = std::fs::remove_file(&exe);
        }
    }
    let _ = std::fs::remove_file(&f);
    None
}

/// `jabberwock build`（`JW_ROBUST_ASAN` なら AddressSanitizer と UBSan つき）
fn build_cmd(jw: &str, exe: &Path, src: &Path) -> Command {
    let mut c = Command::new(jw);
    c.arg("build").arg("-o").arg(exe).arg(src);
    if std::env::var("JW_ROBUST_ASAN").is_ok() {
        c.env("JW_CFLAGS", "-fsanitize=address,undefined -fno-sanitize-recover=undefined -fno-omit-frame-pointer");
    }
    c
}

fn have_clang() -> bool {
    Command::new(jabberwock::native::clang()).arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn run_all(inputs: Vec<(String, String)>, label: &str) {
    let dir = std::env::temp_dir().join(format!("jw-robust-{}-{}", label, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let native = std::env::var("JW_ROBUST_NATIVE").is_ok();
    let dump = std::env::var("JW_ROBUST_DUMP").ok();
    // 4 本ずつ並べて動かす
    let inputs = std::sync::Arc::new(std::sync::Mutex::new(inputs));
    let found = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let workers: Vec<_> = (0..4)
        .map(|w| {
            let (inputs, found, dir) = (inputs.clone(), found.clone(), dir.clone());
            std::thread::spawn(move || loop {
                let Some((name, src)) = inputs.lock().unwrap().pop() else { break };
                if let Some(why) = check(&src, &dir, &format!("{}-{}", name, w), native) {
                    found.lock().unwrap().push((name, src, why));
                }
            })
        })
        .collect();
    for w in workers {
        w.join().unwrap();
    }
    let found = found.lock().unwrap();
    if let Some(d) = &dump {
        std::fs::create_dir_all(d).unwrap();
        for (name, src, _) in found.iter() {
            std::fs::write(Path::new(d).join(format!("{}.jw", name)), src).unwrap();
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    let report: Vec<String> = found
        .iter()
        .map(|(name, src, why)| {
            let head: String = src.chars().take(300).collect();
            format!("{}: {}\n  入力（先頭）: {:?}", name, why, head)
        })
        .collect();
    assert!(report.is_empty(), "{} 本で落ちました:\n{}", report.len(), report.join("\n"));
}

#[test]
fn hostile_inputs_do_not_crash() {
    run_all(hostile().into_iter().enumerate().map(|(i, s)| (format!("hostile{}", i), s)).collect(), "hostile");
}

#[test]
fn mutated_programs_do_not_crash() {
    let seeds = seeds();
    let n = env_u64("JW_ROBUST", CASES);
    let seed0 = env_u64("JW_ROBUST_SEED", 1);
    let inputs = (seed0..seed0 + n)
        .map(|seed| {
            let mut rng = Rng::new(seed);
            let base = &seeds[rng.below(seeds.len())];
            (format!("seed{}", seed), mutate(base, &mut rng))
        })
        .collect();
    run_all(inputs, "mutated");
}

#[test]
fn memory_attacks_stop_with_an_error() {
    let dir = std::env::temp_dir().join(format!("jw-robust-attack-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let native = have_clang();
    let mut bad = Vec::new();
    let mut cases: Vec<(String, String, &str)> = ATTACKS.iter().map(|(n, s, w)| (n.to_string(), s.to_string(), *w)).collect();
    cases.extend(attacks::generated(env_u64("JW_ROBUST_SEED", 1)));
    for (name, src, want) in &cases {
        // interp（デバッグビルドでは IR でも動かして突き合わせる）。解析あり・なしの両方
        for elide in [true, false] {
            let o = jabberwock::run_source_with(src, false, elide);
            match &o.error {
                Some(d) if d.title.contains(want) => {}
                other => bad.push(format!("{}（解析 {}）: 「{}」で止まらなかった: {:?}", name, elide, want, other.as_ref().map(|d| &d.title))),
            }
            if o.output.contains(SECRET) {
                bad.push(format!("{}（解析 {}）: 秘密の値が出た: {:?}", name, elide, o.output));
            }
        }
        // 実行ファイル（JW_ROBUST_ASAN なら AddressSanitizer つき）
        if native {
            let f = dir.join(format!("{}.jw", name));
            std::fs::write(&f, src).unwrap();
            let exe = dir.join(name);
            let b = build_cmd(env!("CARGO_BIN_EXE_jabberwock"), &exe, &f).output().unwrap();
            if !b.status.success() {
                bad.push(format!("{}: 実行ファイルを作れない: {}", name, String::from_utf8_lossy(&b.stderr)));
                continue;
            }
            let o = Command::new(&exe).stdin(Stdio::null()).output().unwrap();
            let (out, err) = (String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
            if o.status.code() != Some(1) || !err.contains(want) || err.contains("Sanitizer") || err.contains("runtime error:") {
                bad.push(format!("{}: 実行ファイルが「{}」で止まらなかった（{:?}）: {}", name, want, o.status.code(), err.lines().take(4).collect::<Vec<_>>().join(" / ")));
            }
            if out.contains(SECRET) {
                bad.push(format!("{}: 実行ファイルで秘密の値が出た: {:?}", name, out));
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// AddressSanitizer つきの実行ファイルで、正しいプログラム（examples）を動かしても何も見つからないこと（JW_ROBUST_ASAN のときだけ）
#[test]
fn sanitized_examples_are_clean() {
    if std::env::var("JW_ROBUST_ASAN").is_err() || !have_clang() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("jw-robust-asan-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut bad = Vec::new();
    let mut files: Vec<PathBuf> = std::fs::read_dir(root.join("examples")).unwrap().filter_map(|e| e.ok().map(|e| e.path())).collect();
    files.sort();
    for f in files.iter().filter(|f| f.extension().and_then(|e| e.to_str()) == Some("jw")) {
        let name = f.file_stem().unwrap().to_str().unwrap();
        let exe = dir.join(name);
        let b = build_cmd(env!("CARGO_BIN_EXE_jabberwock"), &exe, f).output().unwrap();
        if !b.status.success() {
            continue;
        }
        if let Some(why) = crashed(Command::new(&exe).current_dir(&dir)) {
            bad.push(format!("{}: {}", name, why));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// 見張りそのものが、落ちたことを見分けられること
#[cfg(unix)]
#[test]
fn the_detector_sees_crashes() {
    let sh = |script: &str| crashed(Command::new("sh").arg("-c").arg(script));
    assert!(sh("kill -SEGV $$").is_some());
    assert!(sh("echo 'thread main panicked at x' >&2; exit 101").is_some());
    assert!(sh("echo '内部エラー: メモリが足りません' >&2; exit 2").is_some());
    assert!(sh("echo 'error: 型が合いません' >&2; exit 1").is_none());
    assert!(sh("sleep 10").is_none());
}

/// 書き出した LLVM の中の、LLVM では未定義になりうる命令（0 や -1 での割り算・幅以上のずらし）の前に、
/// 守りの計算があるかを見る。UBSan は C から作るときに足す検査なので、書き出した LLVM には効かない。その代わり。
/// 守りの形は llvm.rs の `binary` / `shift` が書くもの（割る数は 0 と比べてから、符号つきは -1 を 1 に置き換える `select`、
/// ずらす数は幅 - 1 で頭打ちにする `select`）。新しく危ない命令を書き出したら、ここにも形を足す
const DANGER: [&str; 7] = ["sdiv", "srem", "udiv", "urem", "shl", "lshr", "ashr"];

#[test]
fn generated_code_guards_undefined_operations() {
    let mut seen: std::collections::BTreeMap<&str, usize> = Default::default();
    let mut bad = Vec::new();
    let mut extra: Vec<String> = Vec::new();
    // 型ごとに、変数と定数で割る・ずらす
    for t in ["i8", "i16", "i32", "i64", "u8", "u32", "u64"] {
        extra.push(format!(
            "fn f({t} a, {t} b) {{ print(a / b, a % b, a << b, a >> b, a / 3, a % 7, a << 3, a >> 2); }}\nfn main() {{ f(100, 3); }}"
        ));
    }
    for (n, src) in seeds().iter().chain(extra.iter()).enumerate() {
        let Ok(ll) = jabberwock::llvm_build(src, &jabberwock::no_modules, "main.jw", false) else { continue };
        for f in ll.split("\ndefine ").skip(1) {
            let lines: Vec<&str> = f.lines().map(|l| l.trim()).collect();
            let def = |v: &str| lines.iter().find_map(|l| l.strip_prefix(&format!("{} = ", v)));
            for (i, l) in lines.iter().enumerate() {
                let Some((_, rhs)) = l.split_once(" = ") else { continue };
                let mut w = rhs.split_whitespace();
                let word = w.next().unwrap_or("");
                let Some(&op) = DANGER.iter().find(|&&d| d == word) else { continue };
                let (Some(ty), Some(_), Some(y)) = (w.next(), w.next(), w.next()) else { continue };
                let bits: i128 = ty[1..].parse().unwrap_or(0);
                *seen.entry(op).or_default() += 1;
                let before = &lines[..i];
                let has = |s: &str| before.iter().any(|b| b.ends_with(s));
                let ok = match op {
                    "udiv" | "urem" => y.parse::<i128>().map(|c| c != 0).unwrap_or(false) || has(&format!("icmp eq {} {}, 0", ty, y)),
                    "sdiv" | "srem" => {
                        // %s = select i1 %m, T 1, T %y0  かつ  %m = icmp eq T %y0, -1  かつ  %y0 を 0 と比べてある
                        let s = def(y).unwrap_or("");
                        let p: Vec<&str> = s.split([' ', ',']).filter(|p| !p.is_empty()).collect();
                        p.len() == 7 && p[0] == "select" && p[4] == "1" && {
                            let y0 = p[6];
                            def(p[2]) == Some(&format!("icmp eq {} {}, -1", ty, y0))
                                && (y0.parse::<i128>().map(|c| c != 0).unwrap_or(false) || has(&format!("icmp eq {} {}, 0", ty, y0)))
                        }
                    }
                    _ => {
                        // 定数なら幅より小さい。変数なら（trunc を通した）幅 - 1 で頭打ちにする select
                        let small = |c: &str| c.parse::<i128>().map(|c| (0..bits).contains(&c)).unwrap_or(false);
                        let mut s = def(y).unwrap_or("");
                        if let Some(r) = s.strip_prefix("trunc i64 ") {
                            s = def(r.split_whitespace().next().unwrap_or("")).unwrap_or("");
                        }
                        let p: Vec<&str> = s.split([' ', ',']).filter(|p| !p.is_empty()).collect();
                        small(y) || (p.len() == 7 && p[0] == "select" && p[4] == (bits - 1).to_string())
                    }
                };
                if !ok {
                    bad.push(format!("種 {}: 守りのない {}: {}", n, op, l));
                }
            }
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
    // 調べる命令が本当に出ていること（形が変わって何も見なくなっていないか）
    for op in DANGER {
        assert!(seen.get(op).copied().unwrap_or(0) > 0, "{} を 1 つも見ていません: {:?}", op, seen);
    }
}

/// GEP は範囲を調べる前でも poison にせず、整数は一周するという言語の規則を守る。
/// 範囲の検査自体は typed attacks（負の数・最大値・read/write/ref）で実行して確かめる。
#[test]
fn generated_llvm_does_not_assume_unproved_address_or_integer_properties() {
    let mut checked = 0;
    let dir = std::env::temp_dir().join(format!("jw-llvm-verify-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let verify = have_clang();
    for src in seeds().into_iter().chain(ATTACKS.iter().map(|(_, src, _)| src.to_string())).chain(attacks::generated(1).into_iter().map(|(_, src, _)| src)) {
        let Ok(ll) = jabberwock::llvm_build(&src, &jabberwock::no_modules, "main.jw", false) else { continue };
        for line in ll.lines().filter(|l| !l.trim_start().starts_with(';')) {
            for forbidden in ["getelementptr inbounds", " nsw ", " nuw ", " exact ", " = fptosi ", " = fptoui "] {
                assert!(!line.contains(forbidden), "未証明の LLVM の前提 {}: {}", forbidden, line);
            }
        }
        // 送信元の古い ref が読むため、区画ヘッダーは読み手も書き手も atomic にする。
        for function in ll.split("\ndefine ").skip(1) {
            let lines: Vec<&str> = function.lines().collect();
            for header in lines.iter().filter(|l| l.contains(" = getelementptr i8, ptr ") && l.ends_with(", i64 -8")) {
                let ptr = header.split_once(" = ").unwrap().0.trim();
                for access in lines.iter().filter(|l| l.split(|c: char| c.is_whitespace() || c == ',').any(|v| v == ptr) && (l.contains("load ") || l.trim_start().starts_with("store "))) {
                    assert!(access.contains("load atomic ") || access.contains("store atomic "), "世代番号の非atomicアクセス: {}", access);
                }
            }
        }
        if verify {
            let input = dir.join("case.ll");
            std::fs::write(&input, &ll).unwrap();
            let output = Command::new(jabberwock::native::clang()).args(["-c", "-O0", "-Xclang", "-llvm-verify-each"])
                .arg(&input).arg("-o").arg(dir.join("case.o")).output().unwrap();
            assert!(output.status.success(), "LLVM verifier: {}", String::from_utf8_lossy(&output.stderr));
        }
        checked += 1;
    }
    assert!(checked >= 50, "LLVMの監査対象が足りません: {}", checked);
    eprintln!("LLVM audit: {} modules, verifier enabled: {}", checked, verify);
    let _ = std::fs::remove_dir_all(dir);
}
