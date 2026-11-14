//! 差分テスト（小さなファジング）。種から小さなプログラムを作り、interp と実行ファイルで出力とエラーを全文比べる
//! （デバッグビルドでは interp が IR でも動かして突き合わせるので、3 つを比べることになる）。
//! 作るプログラムは、読むだけの写し（for の要素・クロージャの取り込み）・ジェネリクス（`Box<T>`）・match ref・
//! リストの操作・リストの要素を借りた ref・メソッドの this（場所・一時的な値・`fn take`）・辞書の値の置き換えを組み合わせ、
//! own を取り出す・借りる・読む（docs/design-decisions.md の「写しの印」の表の部品）。
//! 新しい取り出し経路や組み込みの操作を足したら、ここの部品にも足すこと。
//! ふだんは `CASES` 本。`JW_FUZZ=1000` で本数、`JW_FUZZ_SEED=n` で始めの種を変えられる。
//! `JW_FUZZ_JWC=jwc の実行ファイル` なら、jwc で作った実行ファイルとも比べる。`JW_FUZZ_DUMP=フォルダ` なら、比べたプログラムを書き出す。clang がなければ何もしない。

use std::process::Command;

use jabberwock::{llvm_build, llvm_build_with, native, no_modules};

#[path = "support/attacks.rs"]
mod attacks;

const CASES: u64 = 40;

fn have_clang() -> bool {
    let available = Command::new(native::clang()).arg("--version").output().map(|o| o.status.success()).unwrap_or(false);
    assert!(available || std::env::var("JW_SAFETY_STRICT").is_err(), "required fuzz verification cannot skip clang");
    available
}

fn selected_jwc() -> Option<String> {
    let compiler = std::env::var("JW_FUZZ_JWC").ok();
    if std::env::var("JW_SAFETY_COMPILER").map(|s| s == "jwc").unwrap_or(false) {
        assert!(compiler.as_ref().map(|p| std::path::Path::new(p).is_file()).unwrap_or(false),
                "jwc subject requires an actual JW_FUZZ_JWC executable");
    }
    compiler
}

/// xorshift の乱数（外部クレートなし）
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
        (self.next() % n as u64) as usize
    }
    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[self.below(xs.len())]
    }
}

const HEAD: &str = "struct P { i64 x; }
struct Box<T> { T v; }
struct S { own P b; i64 n; (own P)[] ps; Option<own P> o; Box<own P> bx;
    fn i64 bval() { b.x }
    fn ref P first() { ref ps[0] }
    fn ref i64 nref() { ref n }
    fn mut bump() { n = n + 1; b.x = b.x + 1; }
    fn take own P into() { b }
}
fn T get<T>(ref Box<T> b) { b.v }
fn i64 look(ref P p) { p.x }
fn i64 take(own P p) { p.x }
fn S mk(i64 n) { S { b: own P { x: n }, n: n, ps: [own P { x: n + 1 }, own P { x: n + 2 }], o: Some(own P { x: n + 3 }), bx: Box { v: own P { x: n + 4 } } } }
";

/// main の始めに借りて、あとで使う ref（リストの要素・メソッドが返す ref）。(宣言, 使う文)。
/// 辞書の値のメソッドはまだ実行ファイルに書き出せないので使わない
const REFS: [(&str, &str); 11] = [
    ("let e0 = ref ss[0];", "print(e0.n, e0.b.x);"),
    ("let e1 = ref ss[1].b;", "print(e1.x);"),
    ("let e2 = ref ss[0].n;", "print(*e2);"),
    ("let e3 = ref ss[1].ps[0];", "print(e3.x);"),
    ("let e4 = ss[0].first();", "print(e4.x);"),
    ("let e5 = ss[1].nref();", "print(*e5);"),
    ("let e6 = ref ss[0].ps;", "print(e6.len());"),
    ("let e7 = ref mut ss[0];", "print(e7.n);"),
    ("let e8 = ref ns[0];", "print(*e8);"),
    ("let e9 = ref s0.b;", "print(e9.x);"),
    ("let e10 = ref s0;", "print(e10.n);"),
];

/// s（S の場所。写しのことも本物のこともある）について、1 つの文を作る。`muts` はリスト ss と辞書 m を書き換えてよいか、
/// `refs` は始めに借りた ref（`REFS` の番号）
fn action(r: &mut Rng, s: &str, muts: bool, refs: &[usize], k: usize) -> String {
    let place = r.pick(&["{s}.b", "{s}.ps[0]", "{s}.ps[1]", "{s}.bx.v"]).replace("{s}", s);
    // 取り出す文・必ずダングリングになる文（たいていエラーで止まる）は 8 回に 1 回。ほかは借りる・読む・書き換える
    const RISKY: [usize; 8] = [4, 5, 6, 7, 9, 12, 23, 24];
    const SAFE: [usize; 17] = [0, 1, 2, 3, 8, 10, 11, 13, 14, 15, 19, 20, 21, 22, 25, 26, 27];
    // 仕様（spec の 7.1・7.2）で ref を無効にする操作をすべて入れる: 要素が動く（push・pop・insert・remove・drain・sort・sort_by・sort_unstable・sort_unstable_by・reverse）、
    // リストが解放される（再代入・drop・スレッドへ送る）、own が解放される（再代入・drop）。無効にしない操作（ムーブ・要素への代入）も入れる
    const MUTS: [usize; 21] = [16, 17, 18, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45];
    let which = if r.below(8) == 0 {
        RISKY[r.below(RISKY.len())]
    } else if muts && r.below(3) == 0 {
        MUTS[r.below(MUTS.len())]
    } else {
        SAFE[r.below(SAFE.len())]
    };
    match which {
        0 => format!("print({}.x);", place),
        1 => format!("print(look({}));", place),
        2 => format!("let r{k} = ref {}; print(r{k}.x);", place),
        3 => format!("let h{k} = Box {{ v: ref {} }}; print(get(ref h{k}).x);", place),
        4 => format!("let q{k} = {}; print(q{k}.x);", place),
        5 => format!("print(take({}));", place),
        6 => format!("print(get({s}.bx).x);"),
        7 => format!("let g{k} = get(ref {s}.bx); print(g{k}.x);"),
        8 => format!("match ref {s}.o {{\n        case Some(p{k}) -> {{ let h{k} = Box {{ v: p{k} }}; print(get(ref h{k}).x); }}\n        case None -> print(0);\n    }}"),
        9 => format!("match {s}.o {{\n        case Some(p{k}) -> print(p{k}.x);\n        case None -> print(0);\n    }}"),
        10 => format!("for p{k} in {s}.ps {{ let h{k} = Box {{ v: p{k} }}; print(get(ref h{k}).x); }}"),
        11 => format!("let m{k} = {s}.ps.map((p) -> p); let h{k} = Box {{ v: m{k}[0] }}; print(get(ref h{k}).x);"),
        12 => format!("let h{k} = Box {{ v: {s} }}; print(get(ref h{k}).b.x);"),
        13 => format!("print({s}.n, {s}.ps.len());"),
        14 => format!("let f{k} = () -> look({}); print(f{k}());", place),
        15 => format!("let f{k} = () -> {}.x; print(f{k}());", place),
        16 => "ss.pop();".to_string(),
        17 => format!("ss.push(mk({}));", 100 + k),
        18 => "ss.sort_by((a, c) -> a.n > c.n);".to_string(),
        // 始めに借りた ref を使う（要素が動いたあとならダングリング）
        19 => match refs.get(r.below(refs.len() + 1)) {
            Some(&i) => REFS[i].1.to_string(),
            None => format!("print({s}.n);"),
        },
        // メソッドの this: 場所・一時的な値・辞書の値
        20 => format!("print({s}.bval());"),
        21 => format!("print(mk({}).bval());", 200 + k),
        22 => format!("let w{k} = {s}.first(); print(w{k}.x);"),
        23 => format!("let w{k} = mk({}).nref(); print(*w{k});", 200 + k),
        24 => format!("let t{k} = mk({}).first(); print(t{k}.x);", 200 + k),
        25 => format!("let o{k} = mk({}).into(); print(o{k}.x);", 200 + k),
        26 => format!("print(look({s}.first()));"),
        27 => format!("let u{k} = {s}.nref(); print(*u{k});"),
        28 => "ss[0].bump();".to_string(),
        29 => "ss[1].bump();".to_string(),
        30 => format!("match ss.pop() {{\n        case Some(t{k}) -> {{ let o{k} = t{k}.into(); print(o{k}.x); }}\n        case None -> print(0);\n    }}"),
        31 => format!("m[\"a\"] = mk({});", 300 + k),
        32 => format!("ss[0] = mk({});", 300 + k),
        33 => "ss.remove(0);".to_string(),
        34 => format!("ss.insert(0, mk({}));", 300 + k),
        35 => "ss.reverse();".to_string(),
        36 => format!("for d{k} in ss.drain() {{ print(d{k}.n); }}"),
        37 => format!("ss = [mk({})];", 400 + k),
        38 => "drop(ss);".to_string(),
        39 => format!("let mv{k} = ss; print(mv{k}.len());"),
        40 => format!("let tk{k} = spawn(() -> ss.len()); print(tk{k}.join());"),
        41 => "ns.sort();".to_string(),
        42 => format!("s0 = mk({});", 400 + k),
        43 => "ss.sort_unstable_by((a, c) -> a.n < c.n);".to_string(),
        44 => "ns.sort_unstable();".to_string(),
        _ => "drop(s0);".to_string(),
    }
}

fn body(r: &mut Rng, s: &str, muts: bool, refs: &[usize], base: usize) -> String {
    let n = 1 + r.below(6);
    (0..n).map(|i| format!("    {}\n", action(r, s, muts, refs, base + i))).collect()
}

fn program(seed: u64) -> String {
    let mut r = Rng::new(seed);
    let mut out = String::from(HEAD);
    out.push_str("fn main() {\n    let S[] mut ss = [mk(1), mk(10)];\n    let S[str] mut m = [\"a\": mk(30)];\n    let mut s0 = mk(20);\n    let i64[] mut ns = [3, 1, 2];\n");
    let mut refs: Vec<usize> = Vec::new();
    for _ in 0..r.below(4) {
        let i = r.below(REFS.len());
        if !refs.contains(&i) {
            refs.push(i);
            out.push_str(&format!("    {}\n", REFS[i].0));
        }
    }
    match r.below(6) {
        // for の要素（写し）
        0 => out.push_str(&format!("    for s in ss {{\n{}    }}\n", body(&mut r, "s", true, &refs, 0))),
        // 添字と for (i, s)
        1 => out.push_str(&format!("    for (i, s) in ss {{\n{}    }}\n", body(&mut r, "s", true, &refs, 0))),
        // クロージャに取り込んだ構造体（写し）
        2 => out.push_str(&format!("    let f = () -> {{\n{}    0\n    }};\n    print(f());\n", body(&mut r, "s0", false, &refs, 0))),
        // for の中のクロージャ（写しの写し）
        3 => out.push_str(&format!(
            "    for s in ss {{\n    let f = () -> {{\n{}    0\n    }};\n    print(f());\n    }}\n",
            body(&mut r, "s", false, &refs, 0)
        )),
        // 本物の構造体
        4 => out.push_str(&body(&mut r, "s0", true, &refs, 0)),
        // リストの要素（本物）
        _ => out.push_str(&body(&mut r, "ss[0]", true, &refs, 0)),
    }
    out.push_str(&format!("    {}\n", action(&mut r, "s0", false, &refs, 50)));
    for &i in &refs {
        out.push_str(&format!("    {}\n", REFS[i].1));
    }
    out.push_str("}\n");
    out
}

/// interp と実行ファイルを比べる。型エラーなら None（比べない）
fn compare(name: &str, src: &str, jwc: Option<&str>) -> Option<Result<String, String>> {
    let o = jabberwock::run_source(src, false);
    let ll = match llvm_build(src, &no_modules, "main.jw", false) {
        Ok(ll) => ll,
        Err(e) => {
            if std::env::var("JW_FUZZ_SHOW").is_ok() {
                eprintln!("型エラー: {}", e.lines().next().unwrap_or(""));
            }
            return None;
        }
    };
    let unchecked = jabberwock::run_source_with(src, false, false);
    let diagnostic = |o: &jabberwock::Outcome| o.error.as_ref().map(|d| jabberwock::render(d, "main.jw", &o.files));
    if (o.output.as_str(), diagnostic(&o), o.live_objects) != (unchecked.output.as_str(), diagnostic(&unchecked), unchecked.live_objects) {
        return Some(Err("解析あり・なしのインタプリタが一致しない".to_string()));
    }
    let want_err = o.error.as_ref().map(|d| jabberwock::render(d, "main.jw", &o.files));
    // 実行時ライブラリの .o を使い回すので、フォルダは全部の種で同じ（最後に消す）
    let dir = fuzz_dir();
    let mut exes = vec![native::compile_with_cached_runtime(&ll, &dir, name).expect("clang compiles")];
    let (checked_ll, _) = llvm_build_with(src, &no_modules, "main.jw", false, false, Default::default()).expect("checked LLVM");
    exes.push(native::compile_with_cached_runtime(&checked_ll, &dir, &format!("{}-checked", name)).expect("clang compiles checked LLVM"));
    if let Some(jwc) = jwc {
        // エラーに出るファイル名を interp と同じ main.jw にする
        let sub = dir.join(format!("{}-jwc-input", name));
        std::fs::create_dir_all(&sub).expect("temp dir");
        std::fs::write(sub.join("main.jw"), src).expect("writes source");
        // 自己ホストが書いた LLVM を同じ native 経路で計装する。
        // jwc build は JW_CFLAGS を読まないため、sanitizer を付けたと誤認しない。
        let b = Command::new(jwc).arg("--ll").arg("main.jw").current_dir(&sub).output().expect("runs jwc");
        if !b.status.success() {
            return Some(Err(format!("jwc --ll: {:?}: {}", b.status, String::from_utf8_lossy(&b.stderr))));
        }
        let ll = String::from_utf8(b.stdout).expect("jwc LLVM is UTF-8");
        if !ll.contains("define void @jw_main(") {
            return Some(Err(format!("jwc が LLVM を出さなかった: {}", ll)));
        }
        let exe = native::compile_with_cached_runtime(&ll, &dir, &format!("{}-jwc", name)).expect("clang compiles jwc LLVM");
        exes.push(exe);
    }
    for exe in &exes {
        let out = Command::new(exe).env("JW_LEAKS", "1").current_dir(&dir).output().expect("runs");
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        let what = exe.display().to_string();
        if stdout != o.output {
            return Some(Err(format!("{}: 出力が違う\ninterp:\n{}\nexe:\n{}", what, o.output, stdout)));
        }
        match &want_err {
            Some(e) if out.status.code() != Some(1) || &stderr != e => {
                return Some(Err(format!("{}: エラーが違う\ninterp:\n{}\nexe ({:?}):\n{}", what, e, out.status.code(), stderr)));
            }
            None if out.status.code() != Some(0) || !stderr.contains("[live] 0") || o.live_objects != 0 => {
                return Some(Err(format!("{}: 正常に終わらない・リーク (interp {})\n{}", what, o.live_objects, stderr)));
            }
            _ => {}
        }
    }
    for exe in &exes { let _ = std::fs::remove_file(exe); }
    for suffix in [name.to_string(), format!("{}-checked", name), format!("{}-jwc", name)] {
        let _ = std::fs::remove_file(dir.join(format!("{}.ll", suffix)));
    }
    let _ = std::fs::remove_dir_all(dir.join(format!("{}-jwc-input", name)));
    Some(Ok(o.error.map(|d| d.title).unwrap_or_else(|| "（正常に終わった）".to_string())))
}

fn fuzz_dir() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("jw-fuzz-{}", std::process::id()))
}

#[test]
fn interp_and_executables_agree_on_generated_programs() {
    if !have_clang() {
        return;
    }
    let n: u64 = std::env::var("JW_FUZZ").ok().and_then(|s| s.parse().ok()).unwrap_or(CASES);
    let first: u64 = std::env::var("JW_FUZZ_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
    let jwc = selected_jwc();
    let (mut ran, mut failed) = (0, Vec::new());
    let mut kinds: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for seed in first..first + n {
        let src = program(seed);
        // デバッグビルドでは IR とのずれは panic になるので、種を添えて出す
        let r = std::panic::catch_unwind(|| compare(&format!("s{}", seed), &src, jwc.as_deref()));
        match r {
            Ok(None) => {}
            Ok(Some(Ok(kind))) => {
                ran += 1;
                // JW_FUZZ_DUMP=フォルダ なら、比べたプログラムを書き出す（どの部品が実際に走ったかを見るため）
                if let Ok(d) = std::env::var("JW_FUZZ_DUMP") {
                    let _ = std::fs::create_dir_all(&d);
                    let _ = std::fs::write(std::path::Path::new(&d).join(format!("s{}.jw", seed)), &src);
                }
                *kinds.entry(kind).or_insert(0) += 1;
            }
            Ok(Some(Err(e))) => failed.push(format!("種 {}:\n{}\n{}", seed, src, e)),
            Err(_) => failed.push(format!("種 {}（IR とのずれ）:\n{}", seed, src)),
        }
    }
    // cached runtime の .o は同じプロセスの別テストでも使うため残す。
    eprintln!("fuzz: {} 本のうち {} 本を比べた（残りは型エラー）", n, ran);
    for (k, c) in &kinds {
        eprintln!("  {:5} {}", c, k);
    }
    assert!(failed.is_empty(), "{} 本ずれた:\n\n{}", failed.len(), failed.join("\n\n"));
    // 型エラーで比べられないものばかりにならないように
    assert!(ran * 2 >= n as usize, "比べられたのは {} / {} 本だけ", ran, n);
}

/// 各操作を必ず実行し、期待する言語エラーで止まることを全バックエンドで確認する。
#[test]
fn typed_attacks_reach_their_checks() {
    if !have_clang() { return; }
    let jwc = selected_jwc();
    let seed = std::env::var("JW_FUZZ_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
    let mut cases: Vec<(String, String, &str)> = attacks::ATTACKS.iter().map(|(n, s, w)| (n.to_string(), s.to_string(), *w)).collect();
    cases.extend(attacks::generated(seed));
    let mut checked = 0;
    for (name, src, want) in &cases {
        if let Ok(d) = std::env::var("JW_FUZZ_DUMP") {
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(std::path::Path::new(&d).join(format!("attack-{}.jw", name)), src).unwrap();
        }
        let o = jabberwock::run_source_with(src, false, false);
        assert!(!o.output.contains(attacks::SECRET), "{}: 秘密が漏れた", name);
        if src.contains("reached") { assert_eq!(o.output, "reached\n", "{}: 攻撃まで到達しない: {:?}", name, o.error); }
        let result = compare(&format!("attack-{}", name), src, jwc.as_deref()).unwrap_or_else(|| panic!("{}: 型エラーまたは未対応でスキップ: {}", name, src));
        let kind = result.unwrap_or_else(|e| panic!("{}:\n{}\n{}", name, src, e));
        assert!(kind.contains(want), "{}: 期待 {}、結果 {}", name, want, kind);
        checked += 1;
    }
    eprintln!("typed attacks: {}/{} operations reached and checked", checked, cases.len());
    // cached runtime の .o は同じプロセスの別テストでも使うため残す。
}

/// LLVM の演算の端（0・最小値/-1・幅以上のシフト・NaN/無限大の変換）。
#[test]
fn arithmetic_edges_agree_with_checked_execution() {
    if !have_clang() { return; }
    let jwc = selected_jwc();
    let mut cases = Vec::new();
    for (ty, min, max) in [("i8", "-128", "127"), ("i16", "-32768", "32767"), ("i32", "-2147483648", "2147483647"),
                           ("i64", "-9223372036854775808", "9223372036854775807"), ("u8", "0", "255"),
                           ("u16", "0", "65535"), ("u32", "0", "4294967295"), ("u64", "0", "18446744073709551615")] {
        for (a, b) in [(min, "1"), (max, "0"), (max, "3")] {
            cases.push(format!("fn f({ty} a, {ty} b) {{ print(a + b, a - b, a * b, a / b, a % b); }} fn main() {{ f({a}, {b}); }}"));
        }
        if ty.starts_with('i') { cases.push(format!("fn f({ty} a, {ty} b) {{ print(a / b, a % b, a.abs()); }} fn main() {{ f({min}, -1); }}")); }
        for shift in ["-1", "0", "63", "64", "9223372036854775807"] {
            cases.push(format!("fn f({ty} a, i64 b) {{ print(a << b, a >> b); }} fn main() {{ f({max}, {shift}); }}"));
        }
        cases.push(format!("fn f(f64 a) {{ print(a as {ty}); }} fn main() {{ f(0.0 / 0.0); f(1.0 / 0.0); f(-1.0 / 0.0); }}"));
    }
    for (i, src) in cases.iter().enumerate() {
        compare(&format!("arithmetic-{}", i), src, jwc.as_deref()).unwrap_or_else(|| panic!("型エラー: {}", src)).unwrap_or_else(|e| panic!("{}\n{}", src, e));
    }
    eprintln!("arithmetic edges: {} cases compared", cases.len());
}
