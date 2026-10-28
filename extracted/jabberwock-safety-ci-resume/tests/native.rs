//! LLVM に書き出して clang で作った実行ファイル。clang がなければ何もしない。
//! テスト全体を実行ファイルでも突き合わせるには `JW_NATIVE=1 cargo test`（`lib.rs` の `native_mismatch`）

use std::process::Command;

use jabberwock::{llvm_build, native, no_modules};

fn have_clang() -> bool {
    Command::new(native::clang()).arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}

/// 実行ファイルを作って走らせる（終了コード, 標準出力, 標準エラー）
fn run(name: &str, src: &str) -> (Option<i32>, String, String) {
    run_with(name, src, false)
}

fn run_with(name: &str, src: &str, release: bool) -> (Option<i32>, String, String) {
    let ll = llvm_build(src, &no_modules, "main.jw", release).expect("lowers to LLVM");
    // テストは並んで走るので、フォルダはテストごとに分ける（Windows は使用中のファイルを書き換えられない）
    let dir = std::env::temp_dir().join(format!("jw-native-test-{}-{}", std::process::id(), name));
    let exe = native::compile(&ll, &dir, name, "-O1").expect("clang compiles");
    let out = Command::new(&exe).env("JW_LEAKS", "1").output().expect("runs");
    (out.status.code(), String::from_utf8_lossy(&out.stdout).to_string(), String::from_utf8_lossy(&out.stderr).to_string())
}

#[test]
fn compiled_programs_print_the_same() {
    if !have_clang() {
        return;
    }
    let src = "struct P { i64 x; own i64 y; }\n\
               fn i64 fib(i64 n) { if n < 2 { return n; } fib(n - 1) + fib(n - 2) }\n\
               fn main() {\n    let p = P { x: 1, y: own 2 };\n    let r = ref p;\n    let xs = [1.5, 0.1, 100000000000000000000.0];\n    \
               print(fib(20), r.x, *r.y, xs, \"{p}\");\n}";
    let (code, out, err) = run("print", src);
    assert_eq!(code, Some(0), "{}", err);
    assert_eq!(out, "6765 1 2 [1.5, 0.1, 1e20] P { x: 1, y: 2 }\n");
    assert!(err.contains("[live] 0"), "{}", err);
}

#[test]
fn compiled_programs_catch_dangling_refs() {
    if !have_clang() {
        return;
    }
    let src = "fn main() {\n    let a = own 5;\n    let r = ref a;\n    drop(a);\n    print(*r);\n}";
    let (code, out, err) = run("dangling", src);
    assert_eq!(code, Some(1));
    assert_eq!(out, "");
    // インタプリタと同じ、時系列のエラー表示
    let want = "error: 無効になった参照を使いました (dangling reference)
 --> main.jw:5:11
  |
3 |     let r = ref a;
  |             ----- ここで `a` を借りた
4 |     drop(a);
  |     ------- ここで解放された: `a` が drop で明示的に解放された
5 |     print(*r);
  |           ^^ ここで使ったが、借りた先はもう存在しない
  |
  = note: 参照は、借りた元の所有者より長くは生きられません
";
    assert_eq!(err, want);
    // --release では使った位置だけ
    let (code, _, err) = run_with("dangling_release", src, true);
    assert_eq!(code, Some(1));
    assert_eq!(
        err,
        "error: 無効になった参照を使いました (dangling reference)\n --> main.jw:5:11\n  = ここで使ったが、借りた先はもう存在しない\n"
    );
}

#[test]
fn compiled_programs_run_threads() {
    if !have_clang() {
        return;
    }
    let src = "fn i64 fib(i64 n) { if n < 2 { return n; } fib(n - 1) + fib(n - 2) }\n\
               fn main() {\n    let chan<str> c = chan();\n    let task<i64>[] mut ts = [];\n    \
               for i in range(4) { ts.push(spawn(() -> { c.send(\"done {i}\"); fib(20) })); }\n    \
               let mut total = 0;\n    for t in ts { total += t.join(); }\n    \
               let mut n = 0;\n    for i in range(4) { match c.recv() { case Some(m) -> { n += 1; } case None -> {} } }\n    \
               print(total, n);\n    print(c.recv());\n}";
    let (code, out, err) = run("threads", src);
    // 最後の recv は、送るスレッドがもうないのでデッドロック（main の位置で知らせる）
    assert_eq!(code, Some(1), "{}", err);
    assert_eq!(out, "27060 4\n");
    assert!(err.starts_with("error: すべてのスレッドが待ちのまま止まりました (deadlock)\n  --> main.jw:11:11\n"), "{}", err);
}

// ---------------------------------------------------------------- 区画の確保（実行時ライブラリ）
// 小さい区画は塊から切り出して使い回し、頭は世代だけ。解放の位置は解放した区画の中身・使い回しの輪の表・送った区画の輪の表にある。
// どれもインタプリタと同じ出力・同じエラー表示・残った区画 0 になることを、-O1 と、jabberwock build と同じ -O2（LTO）で確かめる

/// インタプリタと実行ファイルの結果が同じか（出力・エラー表示の全文・終了コード・残った区画の数）
fn same_as_interp(name: &str, src: &str) {
    if !have_clang() {
        return;
    }
    let o = jabberwock::run_source(src, false);
    let want_err = o.error.as_ref().map(|d| jabberwock::render(d, "main.jw", &o.files));
    for opt in ["-O1", "-O2"] {
        let ll = llvm_build(src, &no_modules, "main.jw", false).expect("lowers to LLVM");
        // テストは並んで走るので、フォルダはテストごと（実行時ライブラリのソースを同じ場所に書く）
        let dir = std::env::temp_dir().join(format!("jw-native-test-{}-{}", std::process::id(), name));
        let exe = native::compile(&ll, &dir, &format!("{}{}", name, opt), opt).expect("clang compiles");
        let out = Command::new(&exe).env("JW_LEAKS", "1").output().expect("runs");
        let _ = std::fs::remove_dir_all(&dir);
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        assert_eq!(stdout, o.output, "{} {}: 出力", name, opt);
        match &want_err {
            Some(e) => {
                assert_eq!(out.status.code(), Some(1), "{} {}", name, opt);
                assert_eq!(&stderr, e, "{} {}: エラー表示", name, opt);
            }
            None => {
                assert_eq!(out.status.code(), Some(0), "{} {}: {}", name, opt, stderr);
                assert_eq!(o.live_objects, 0, "{}: interp のリーク", name);
                assert!(stderr.contains("[live] 0"), "{} {}: {}", name, opt, stderr);
            }
        }
    }
}

#[test]
fn reused_blocks_keep_detecting_old_refs() {
    // 解放した区画は同じ大きさの次の own に使い回される。古い ref は世代で見つかり、「解放された」位置も正しい
    let src = "fn main() {\n    let a = own 5;\n    let r = ref a;\n    drop(a);\n    let b = own 6;\n    let c = own 7;\n    print(*b, *c);\n    print(*r);\n}";
    same_as_interp("reuse", src);
    // 何度も使い回したあと（解放の位置は使い回しの輪の表から探す）
    let src = "fn main() {\n    let a = own 1;\n    let r = ref a;\n    drop(a);\n    let mut s = 0;\n    for i in range(1000) {\n        let x = own i;\n        s += *x;\n    }\n    print(s);\n    print(*r);\n}";
    same_as_interp("reuse_many", src);
}

#[test]
fn blocks_of_mixed_sizes_are_kept_apart() {
    // 大きさの違う区画（8 バイト刻みの小さいもの・2048 バイトを超える大きいもの）を混ぜて確保・解放・使い回す
    let src = "struct Big { i64[60][5] xs; i64 tag; }\n\
               struct Pair { i64 a; own i64 b; }\n\
               fn main() {\n    let mut total = 0;\n    for round in range(50) {\n        \
               let (own i64)[] mut small = [];\n        let (own Pair)[] mut mid = [];\n        let (own Big)[] mut big = [];\n        \
               for i in range(20) {\n            small.push(own i);\n            mid.push(own Pair { a: i, b: own i * 2 });\n            \
               if i % 5 == 0 { big.push(own Big { xs: [[round; 60]; 5], tag: i }); }\n        }\n        \
               for x in small { total += *x; }\n        for p in mid { total += p.a + *p.b; }\n        for b in big { total += b.xs[4][59] + b.tag; }\n        \
               if round % 2 == 0 { small.pop(); big.pop(); }\n    }\n    print(total);\n    \
               let keep = own Big { xs: [[7; 60]; 5], tag: 1 };\n    let r = ref keep;\n    drop(keep);\n    let other = own Big { xs: [[8; 60]; 5], tag: 2 };\n    \
               print(other.tag);\n    print(r.tag);\n}";
    same_as_interp("mixed_sizes", src);
}

#[test]
fn blocks_freed_on_other_threads() {
    // 送った区画は受け取ったスレッドで解放される（そのスレッドの空き区画の鎖に入る）。送った側の古い ref は「ほかのスレッドへ送られた」
    let src = "struct Job { i64 id; own i64 load; }\n\
               fn main() {\n    let chan<own Job> jobs = chan();\n    let chan<i64> done = chan();\n    \
               let w = spawn(() -> {\n        loop {\n            match jobs.recv() {\n                \
               case Some(j) -> { let x = own *j.load + j.id; done.send(*x); }\n                case None -> break;\n            }\n        }\n    });\n    \
               for i in range(200) { jobs.send(own Job { id: i, load: own i }); }\n    jobs.close();\n    \
               let mut s = 0;\n    for i in range(200) { match done.recv() { case Some(v) -> s += v; case None -> {} } }\n    w.join();\n    print(s);\n    \
               let mut extra = 0;\n    for i in range(100) { let y = own i; extra += *y; }\n    print(extra);\n    \
               let last = own Job { id: 1, load: own 2 };\n    let r = ref last;\n    let chan<own Job> c2 = chan();\n    c2.send(last);\n    print(*r.load);\n}";
    same_as_interp("other_threads", src);
}

#[test]
fn option_of_own_is_one_pointer() {
    // Option<own T> と、同じ形の enum（中身なし・own 1 つ）はポインタ 1 つで表す。作る・取り出す・表示・解放・中身の型が決まらない None
    let src = "struct Node { i64 v; Option<own Node> next; }\n\
               enum Slot { Empty; Full(own i64 v); }\n\
               fn i64 sum(ref Option<own Node> l) {\n    let mut s = 0;\n    let mut cur = l;\n    loop {\n        \
               match cur {\n            case None -> return s;\n            case Some(n) -> { s += n.v; cur = ref n.next; }\n        }\n    }\n}\n\
               fn Option<own Node> build(i64 n) {\n    let Option<own Node> mut head = None;\n    \
               for i in range(n) { head = Some(own Node { v: i, next: head }); }\n    head\n}\n\
               fn main() {\n    let l = build(100);\n    print(sum(l));\n    \
               let Option<own i64> mut o = None;\n    print(o);\n    o = Some(own 3);\n    print(o, o.is_some());\n    \
               match o {\n        case Some(x) -> print(*x + 1);\n        case None -> print(\"none\");\n    }\n    \
               let Slot[] mut slots = [Slot.Empty, Slot.Full(own 4), Slot.Empty];\n    slots[2] = Slot.Full(own 5);\n    \
               for s in slots {\n        match s {\n            case Empty -> print(\"empty\");\n            case Full(v) -> print(*v);\n        }\n    }\n    \
               let short = build(1);\n    match short {\n        case Some(n) -> print(n.v, n.next);\n        case None -> {}\n    }\n}";
    same_as_interp("niche", src);
    // 取り出した中身を借りた ref は、Some を None で上書きすると無効になる
    let src = "fn main() {\n    let Option<own i64> mut o = Some(own 9);\n    let a = own 0;\n    let mut r = ref a;\n    \
               match ref o {\n        case Some(x) -> r = x;\n        case None -> {}\n    }\n    o = None;\n    print(*r);\n}";
    same_as_interp("niche_dangling", src);
}

#[test]
fn strings_of_many_lengths() {
    // 文字列は 16 バイト刻みの空き領域の鎖から取る（1024 バイトを超えれば malloc）。組み立て中の文字は 48 バイトを超えると広げる
    let src = "fn main() {\n    let str[] mut keep = [];\n    let mut total = 0;\n    for i in range(300) {\n        \
               let s = \"x\".repeat(i * 7) + \"{i}\";\n        let t = \"[{s}|{i}|{-i}]\";\n        total += t.len();\n        \
               if i % 3 == 0 { keep.push(t); }\n    }\n    print(total, keep.len(), keep[keep.len() - 1].len());\n    \
               let mut joined = \"\";\n    for k in keep { if k.len() < 40 { joined = joined + k; } }\n    print(joined);\n    \
               print(\"{9223372036854775807} {-9223372036854775807 - 1} {0}\");\n    let u64 big = 18446744073709551615;\n    print(\"{big}\");\n}";
    same_as_interp("strings", src);
}

#[test]
fn freed_site_is_forgotten_after_4096_reuses() {
    // 同じ区画を 4096 回より多く使い回すと「解放された」の行は出せない（docs/native.md に書いた決まり）。止まり方と残りの表示は同じ
    if !have_clang() {
        return;
    }
    let src = "fn main() {\n    let a = own 1;\n    let r = ref a;\n    drop(a);\n    for i in range(5000) {\n        let x = own i;\n    }\n    print(*r);\n}";
    let (code, out, err) = run("forgotten", src);
    assert_eq!(code, Some(1));
    assert_eq!(out, "");
    let want = "error: 無効になった参照を使いました (dangling reference)
 --> main.jw:8:11
  |
3 |     let r = ref a;
  |             ----- ここで `a` を借りた
  ...
8 |     print(*r);
  |           ^^ ここで使ったが、借りた先はもう存在しない
  |
  = note: 参照は、借りた元の所有者より長くは生きられません
";
    assert_eq!(err, want);
}

#[test]
fn own_inside_a_for_element_cannot_be_taken_out() {
    // for の要素の中の own は読むだけの写し。取り出すと、型は own なのに写しが外へ出てしまう（検査を消す解析・spawn・書き換えの穴）
    let base = "struct P { i64 x; }\nstruct S { own P b; }\n";
    let freed = "fn main() {\n    let S[] ss = [S { b: own P { x: 1 } }];\n    let mut ss2 = ss;\n    for s in ss2 {\n        \
                 let mut q = s.b;\n        ss2.pop();\n        let n = own P { x: 777 };\n        print(q.x);\n        q.x = 4242;\n        \
                 print(n.x);\n    }\n}";
    same_as_interp("view_freed", &format!("{}{}", base, freed));
    let write = "fn main() {\n    let ss = [S { b: own P { x: 1 } }];\n    for s in ss {\n        let mut q = s.b;\n        q.x = 4242;\n    }\n    \
                 print(ss[0].b.x);\n}";
    same_as_interp("view_write", &format!("{}{}", base, write));
    let send = "fn main() {\n    let ss = [S { b: own P { x: 1 } }];\n    for s in ss {\n        let q = s.b;\n        \
                let t = spawn(() -> q.x);\n        print(t.join());\n    }\n}";
    same_as_interp("view_send", &format!("{}{}", base, send));
    let closure = "fn main() {\n    let s = S { b: own P { x: 1 } };\n    let f = () -> { let q = s.b; q.x };\n    print(f());\n}";
    same_as_interp("view_closure", &format!("{}{}", base, closure));
    let array = "struct Q { (own P)[2] a; }\nfn main() {\n    let (own P)[2] arr = [own P { x: 1 }, own P { x: 2 }];\n    \
                 let qs = [Q { a: arr }];\n    for q in qs {\n        let a = q.a;\n        print(a[0].x);\n    }\n}";
    same_as_interp("view_array", &format!("{}{}", base, array));
    // 借りるのはよい
    let borrow = "fn main() {\n    let ss = [S { b: own P { x: 5 } }];\n    for s in ss {\n        let q = ref s.b;\n        print(q.x, s.b.x);\n    }\n}";
    same_as_interp("view_borrow", &format!("{}{}", base, borrow));
}

#[test]
fn own_in_a_view_gives_the_same_error_on_every_path() {
    // 写しの中の own を取り出す経路はどれも同じエラー（借りる経路 arg_ref / match_ref は通る）。interp と実行ファイルで突き合わせる
    let base = "struct P { i64 x; }\nstruct S { own P b; }\nenum E { A(own P p); N; }\nstruct T { own P p; }\nstruct W { E e; }\n";
    let cases = [
        ("view_index", "fn main() {\n    let ss = [S { b: own P { x: 1 } }];\n    let mut ss2 = ss;\n    for s in ss2 {\n        let q = ss2[0].b;\n        print(q.x);\n    }\n}"),
        ("view_arg_own", "fn i64 take(own P p) { p.x }\nfn main() {\n    let ss = [S { b: own P { x: 1 } }];\n    for s in ss {\n        print(take(s.b));\n    }\n}"),
        ("view_arg_ref", "fn i64 look(ref P p) { p.x }\nfn main() {\n    let ss = [S { b: own P { x: 3 } }];\n    for s in ss {\n        print(look(s.b));\n    }\n}"),
        ("view_ret", "fn own P get(ref S s) { return s.b; }\nfn main() {\n    let ss = [S { b: own P { x: 1 } }];\n    for s in ss {\n        let q = get(s);\n        print(q.x);\n    }\n}"),
        ("view_struct_init", "fn main() {\n    let ss = [S { b: own P { x: 1 } }];\n    for s in ss {\n        let t = T { p: s.b };\n        print(t.p.x);\n    }\n}"),
        ("view_variant", "fn main() {\n    let ss = [S { b: own P { x: 1 } }];\n    for s in ss {\n        let o = Some(s.b);\n        print(o.is_some());\n    }\n}"),
        ("view_push", "fn main() {\n    let ss = [S { b: own P { x: 1 } }];\n    let (own P)[] mut out = [];\n    for s in ss {\n        out.push(s.b);\n    }\n    print(out.len());\n}"),
        ("view_match_val", "fn main() {\n    let ws = [W { e: E.A(own P { x: 1 }) }];\n    for w in ws {\n        match w.e {\n            case A(p) -> print(p.x);\n            case N -> print(0);\n        }\n    }\n}"),
        ("view_match_ref", "fn main() {\n    let ws = [W { e: E.A(own P { x: 1 }) }];\n    for w in ws {\n        match ref w.e {\n            case A(p) -> print(p.x);\n            case N -> print(0);\n        }\n    }\n}"),
        ("view_ifval", "fn main() {\n    let ss = [S { b: own P { x: 1 } }];\n    for s in ss {\n        let q = if true { s.b } else { own P { x: 2 } };\n        print(q.x);\n    }\n}"),
        ("view_assign", "fn main() {\n    let ss = [S { b: own P { x: 1 } }];\n    let mut q = own P { x: 0 };\n    for s in ss {\n        q = s.b;\n    }\n    print(q.x);\n}"),
        ("view_lambda_ret", "fn main() {\n    let s = S { b: own P { x: 1 } };\n    let f = () -> s.b;\n    let q = f();\n    print(q.x);\n}"),
    ];
    for (name, body) in cases.iter() {
        let src = format!("{}{}", base, body);
        let o = jabberwock::run_source(&src, false);
        let borrows = name.ends_with("_ref");
        assert_eq!(o.error.is_none(), borrows, "{}", name);
        if let Some(d) = &o.error {
            assert!(d.title.contains("は取り出せません"), "{}: {}", name, d.title);
        }
        same_as_interp(name, &src);
    }
}

#[test]
fn own_in_a_view_through_generics_and_builtins() {
    // 型の引数 T を通っても（interp は値の `RefVal.view` で見る）、組み込みの操作・関数やクロージャを通しても、写しの own は取り出せない
    let runs = [
        ("g_field", "struct P { i64 x; }\nstruct S { own P b; }\nstruct Box<T> { T v; }\ninterface HasX { fn i64 getx(); }\nstruct Q { i64 x; fn i64 getx() { x } }\nfn T get<T>(ref Box<T> b) { b.v }\nfn main() {\n    let Box<own P>[] bs = [Box { v: own P { x: 1 } }];\n    for b in bs { let q = get(b); print(q.x); }\n}"),
        ("g_elem_view", "struct P { i64 x; }\nstruct S { own P b; }\nstruct Box<T> { T v; }\ninterface HasX { fn i64 getx(); }\nstruct Q { i64 x; fn i64 getx() { x } }\nstruct L { (own P)[] ps; }\nfn T first<T>(ref T[] a) { a[0] }\nfn main() {\n    let L[] ls = [L { ps: [own P { x: 1 }] }];\n    for l in ls { let q = first(l.ps); print(q.x); }\n}"),
        ("g_struct_view", "struct P { i64 x; }\nstruct S { own P b; }\nstruct Box<T> { T v; }\ninterface HasX { fn i64 getx(); }\nstruct Q { i64 x; fn i64 getx() { x } }\nfn T get<T>(ref Box<T> b) { b.v }\nstruct H { Box<own P> bx; }\nfn main() {\n    let H[] hs = [H { bx: Box { v: own P { x: 1 } } }];\n    for h in hs { let q = get(h.bx); print(q.x); }\n}"),
        ("g_closure", "struct P { i64 x; }\nstruct S { own P b; }\nstruct Box<T> { T v; }\ninterface HasX { fn i64 getx(); }\nstruct Q { i64 x; fn i64 getx() { x } }\nfn T get<T>(ref Box<T> b) { b.v }\nfn main() {\n    let bx = Box { v: own P { x: 1 } };\n    let f = () -> get(bx);\n    let q = f();\n    print(q.x);\n}"),
        ("g_arr", "struct P { i64 x; }\nstruct S { own P b; }\nstruct Box<T> { T v; }\nfn T get<T>(ref Box<T> b) { b.v }\nstruct H { Box<(own P)[2]> bx; }\nfn main() {\n    let H[] hs = [H { bx: Box { v: [own P { x: 1 }, own P { x: 2 }] } }];\n    for h in hs { let a = get(h.bx); print(a[1].x); }\n}"),
        ("g_opt", "struct P { i64 x; }\nstruct S { own P b; }\nstruct Box<T> { T v; }\nfn T get<T>(ref Box<T> b) { b.v }\nstruct H { Box<Option<own P>> bx; }\nfn main() {\n    let H[] hs = [H { bx: Box { v: Some(own P { x: 1 }) } }];\n    for h in hs { let a = get(h.bx); print(a.is_some()); }\n}"),
        ("ok_ref", "struct P { i64 x; }\nstruct S { own P b; }\nstruct Box<T> { T v; }\nfn T get<T>(ref Box<T> b) { b.v }\nfn main() {\n    let S[] ss = [S { b: own P { x: 4 } }];\n    for s in ss {\n        let r = ref s.b;\n        let bx = Box { v: r };\n        let q = get(ref bx);\n        print(q.x);\n        let bs = [Box { v: ref s.b }];\n        for b in bs { print(get(b).x); }\n    }\n}"),
        ("ok_refelem", "struct P { i64 x; }\nstruct S { own P b; }\nstruct Box<T> { T v; }\nfn T get<T>(ref Box<T> b) { b.v }\nfn main() {\n    let (own P)[] ps = [own P { x: 6 }];\n    for p in ps { let bx = Box { v: p }; print(get(ref bx).x); }\n}"),
        ("l_map", "struct P { i64 x; }\nstruct S { (own P)[] ps; }\nfn main() {\n    let S[] ss = [S { ps: [own P { x: 1 }] }];\n    for s in ss { let k = s.ps.map((p) -> p); print(k.len()); }\n}"),
        ("m_for", "struct P { i64 x; }\nstruct S { (own P)[] ps; }\nstruct M { (own P)[str] m; }\nfn main() {\n    let M[] ms = [M { m: [\"a\": own P { x: 1 }] }];\n    for x in ms { for (k, v) in x.m { let q = v; print(k, q.x); } }\n}"),
        ("l_whole", "struct P { i64 x; }\nstruct S { (own P)[] ps; }\nfn main() {\n    let S[] ss = [S { ps: [own P { x: 1 }] }];\n    for s in ss { let k = s.ps; print(k.len()); }\n}"),
        ("l_elem", "struct P { i64 x; }\nstruct S { (own P)[] ps; }\nfn main() {\n    let S[] ss = [S { ps: [own P { x: 1 }] }];\n    for s in ss { let q = s.ps[0]; print(q.x); }\n}"),
        ("c_whole", "struct P { i64 x; }\nstruct S { (own P)[] ps; }\nfn main() {\n    let (own P)[] ps = [own P { x: 1 }];\n    let f = () -> ps;\n    let k = f();\n    print(k.len());\n}"),
        ("m_index", "struct P { i64 x; }\nstruct S { (own P)[] ps; }\nstruct M { (own P)[str] m; }\nfn main() {\n    let M[] ms = [M { m: [\"a\": own P { x: 1 }] }];\n    for x in ms { let q = x.m[\"a\"]; print(q.x); }\n}"),
        ("ret_view", "struct P { i64 x; }\nstruct S { own P b; }\nstruct Box<T> { T v; }\ninterface HasX { fn i64 getx(); }\nstruct Q { i64 x; fn i64 getx() { x } }\nfn ref S keep(ref S s) { s }\nfn own P take(ref S s) { s.b }\nfn main() {\n    let S[] ss = [S { b: own P { x: 1 } }];\n    for s in ss { let r = keep(s); let q = take(r); print(q.x); }\n}"),
        ("clo_out", "struct P { i64 x; }\nstruct S { own P b; }\nstruct Box<T> { T v; }\ninterface HasX { fn i64 getx(); }\nstruct Q { i64 x; fn i64 getx() { x } }\nfn own P take(ref S s) { s.b }\nfn main() {\n    let s = S { b: own P { x: 1 } };\n    let f = () -> take(s);\n    let q = f();\n    print(q.x);\n}"),
        ("clo_param", "struct P { i64 x; }\nstruct S { own P b; }\nstruct Box<T> { T v; }\ninterface HasX { fn i64 getx(); }\nstruct Q { i64 x; fn i64 getx() { x } }\nfn main() {\n    let S[] ss = [S { b: own P { x: 1 } }];\n    let g = (ref S s) -> s.b;\n    for s in ss { let q = g(s); print(q.x); }\n}"),
    ];
    for (name, src) in runs.iter() {
        let o = jabberwock::run_source(src, false);
        let ok = name.starts_with("ok_") || *name == "l_map" || *name == "m_for";
        assert_eq!(o.error.is_none(), ok, "{}", name);
        if let Some(d) = &o.error {
            assert!(d.title.contains("は取り出せません"), "{}: {}", name, d.title);
        }
        same_as_interp(name, src);
    }
    // 書き換える組み込みの操作は、読むだけの写しには実行前のエラー
    let rejected = [
        "struct P { i64 x; }\nstruct S { (own P)[] ps; }\nfn main() {\n    let S[] ss = [S { ps: [own P { x: 1 }] }];\n    for s in ss { let o = s.ps.pop(); print(o.is_some()); }\n}",
        "struct P { i64 x; }\nstruct S { (own P)[] ps; }\nfn main() {\n    let S[] ss = [S { ps: [own P { x: 1 }] }];\n    for s in ss { let q = s.ps.remove(0); print(q.x); }\n}",
        "struct P { i64 x; }\nstruct S { (own P)[] ps; }\nfn main() {\n    let S[] ss = [S { ps: [own P { x: 1 }] }];\n    for s in ss { for q in s.ps.drain() { print(q.x); } }\n}",
        "struct P { i64 x; }\nstruct S { (own P)[] ps; }\nfn main() {\n    let S[] ss = [S { ps: [own P { x: 1 }] }];\n    for s in ss { let k = s.ps.filter((p) -> true); print(k.len()); }\n}",
        "struct P { i64 x; }\nstruct S { (own P)[] ps; }\nfn main() {\n    let S[] ss = [S { ps: [own P { x: 2 }, own P { x: 1 }] }];\n    for s in ss { s.ps.sort_by((a, b) -> a.x < b.x); }\n}",
        "struct P { i64 x; }\nstruct S { (own P)[] ps; }\nfn main() {\n    let (own P)[] mut ps = [own P { x: 1 }];\n    let f = () -> { let o = ps.pop(); o.is_some() };\n    print(f());\n}",
        "struct P { i64 x; }\nstruct S { (own P)[] ps; }\nfn main() {\n    let (own P)[] ps = [own P { x: 1 }];\n    let f = () -> ps.filter((p) -> true);\n    let k = f();\n    print(k.len());\n}",
        "struct P { i64 x; }\nstruct S { (own P)[] ps; }\nfn main() {\n    let (own P)[str] m = [\"a\": own P { x: 1 }];\n    let o = m.get(\"a\");\n    print(o.is_some());\n}",
        "struct P { i64 x; }\nstruct S { (own P)[] ps; }\nstruct M { (own P)[str] m; }\nfn main() {\n    let M[] ms = [M { m: [\"a\": own P { x: 1 }] }];\n    for x in ms { let o = x.m.remove(\"a\"); print(o.is_some()); }\n}",
    ];
    for src in rejected.iter() {
        let o = jabberwock::run_source(src, false);
        assert!(o.error.is_some() && o.output.is_empty(), "{}", src);
    }
}

#[test]
fn view_mark_is_kept_or_dropped_where_it_should_be() {
    // 写しの印（`RefVal.view`）: ref として受け取る所（match ref・for・map・sort_by・借り直し・ref の interface）では消え、own の型の場所（own の interface も）では残る
    let runs = [
        ("u_matchref", "struct P { i64 x; }\nstruct Box<T> { T v; }\nfn T get<T>(ref Box<T> b) { b.v }\nstruct O { Option<own P> o; }\nfn main() {\n    let O[] os = [O { o: Some(own P { x: 1 }) }];\n    for o in os {\n        match ref o.o {\n            case Some(p) -> { let bx = Box { v: p }; print(get(ref bx).x); }\n            case None -> print(0);\n        }\n    }\n}"),
        ("u_nested_for", "struct P { i64 x; }\nstruct Box<T> { T v; }\nfn T get<T>(ref Box<T> b) { b.v }\nstruct S { (own P)[] ps; }\nfn main() {\n    let S[] ss = [S { ps: [own P { x: 2 }] }];\n    for s in ss { for p in s.ps { let bx = Box { v: p }; print(get(ref bx).x); } }\n}"),
        ("u_map", "struct P { i64 x; }\nstruct Box<T> { T v; }\nfn T get<T>(ref Box<T> b) { b.v }\nstruct S { (own P)[] ps; }\nfn main() {\n    let S[] ss = [S { ps: [own P { x: 3 }] }];\n    for s in ss { let rs = s.ps.map((p) -> p); let bx = Box { v: rs[0] }; print(get(ref bx).x); }\n}"),
        ("u_sortby", "struct P { i64 x; }\nstruct Box<T> { T v; }\nfn T get<T>(ref Box<T> b) { b.v }\nfn main() {\n    let (own P)[] mut ps = [own P { x: 2 }, own P { x: 1 }];\n    ps.sort_by((a, b) -> { let bx = Box { v: a }; get(ref bx).x < b.x });\n    print(ps[0].x);\n}"),
        ("u_mapfor", "struct P { i64 x; }\nstruct Box<T> { T v; }\nfn T get<T>(ref Box<T> b) { b.v }\nstruct M { (own P)[str] m; }\nfn main() {\n    let M[] ms = [M { m: [\"a\": own P { x: 8 }] }];\n    for x in ms { for (k, v) in x.m { let bx = Box { v: v }; print(k, get(ref bx).x); } }\n}"),
        ("u_arrfor", "struct P { i64 x; }\nstruct Box<T> { T v; }\nfn T get<T>(ref Box<T> b) { b.v }\nstruct A { (own P)[2] a; }\nfn main() {\n    let A[] as2 = [A { a: [own P { x: 9 }, own P { x: 10 }] }];\n    for x in as2 { for p in x.a { let bx = Box { v: p }; print(get(ref bx).x); } }\n}"),
        ("u_closure", "struct P { i64 x; }\nstruct Box<T> { T v; }\nfn T get<T>(ref Box<T> b) { b.v }\nfn main() {\n    let a = own P { x: 4 };\n    let f = () -> { let r = ref a; let bx = Box { v: r }; get(ref bx).x };\n    print(f());\n}"),
        ("u_closure_view", "struct P { i64 x; }\nstruct Box<T> { T v; }\nfn T get<T>(ref Box<T> b) { b.v }\nstruct S { own P b; }\nfn main() {\n    let s = S { b: own P { x: 5 } };\n    let f = () -> { let bx = Box { v: ref s.b }; get(ref bx).x };\n    print(f());\n}"),
        ("u_field_ref", "struct P { i64 x; }\nstruct Box<T> { T v; }\nfn T get<T>(ref Box<T> b) { b.v }\nstruct S { own P b; }\nstruct R { ref P r; }\nfn main() {\n    let S[] ss = [S { b: own P { x: 6 } }];\n    for s in ss { let rr = R { r: ref s.b }; let bx = Box { v: rr.r }; print(get(ref bx).x); }\n}"),
        ("u_this", "struct P { i64 x; }\nstruct Box<T> { T v; }\nfn T get<T>(ref Box<T> b) { b.v }\nstruct S { own P b; fn i64 g() { let bx = Box { v: ref b }; get(ref bx).x } }\nfn main() {\n    let S[] ss = [S { b: own P { x: 7 } }];\n    for s in ss { print(s.g()); }\n}"),
        ("u_iface_ref", "interface Shape { fn i64 area(); }\nstruct Sq { i64 w; fn i64 area() { w * w } }\nstruct Box<T> { T v; }\nfn T get<T>(ref Box<T> b) { b.v }\nstruct H { own Sq s; }\nfn i64 area_of(ref Shape s) { s.area() }\nfn main() {\n    let H[] hs = [H { s: own Sq { w: 3 } }];\n    for h in hs { print(area_of(h.s)); let ref Shape r = ref h.s; let bx = Box { v: r }; print(get(ref bx).area()); }\n}"),
        ("k_iface_own", "interface Shape { fn i64 area(); }\nstruct Sq { i64 w; fn i64 area() { w * w } }\nstruct Box<T> { T v; }\nfn T get<T>(ref Box<T> b) { b.v }\nstruct H { own Sq s; }\nfn main() {\n    let H[] hs = [H { s: own Sq { w: 3 } }];\n    for h in hs { let own Shape x = h.s; print(x.area()); }\n}"),
        ("k_iface_generic", "interface Shape { fn i64 area(); }\nstruct Sq { i64 w; fn i64 area() { w * w } }\nstruct Box<T> { T v; }\nfn T get<T>(ref Box<T> b) { b.v }\nstruct H { own Sq s; }\nstruct G { Box<own Shape> b; }\nfn main() {\n    let G[] gs = [G { b: Box { v: own Sq { w: 2 } } }];\n    for g in gs { let x = get(g.b); print(x.area()); }\n}"),
        ("u_autoborrow", "struct P { i64 x; }\nstruct Box<T> { T v; }\nfn T get<T>(ref Box<T> b) { b.v }\nstruct S { own P b; }\nfn i64 f(ref P p) { let bx = Box { v: p }; get(ref bx).x }\nfn main() {\n    let S[] ss = [S { b: own P { x: 11 } }];\n    for s in ss { print(f(s.b)); }\n}"),
        ("u_capture_ref", "struct P { i64 x; }\nstruct Box<T> { T v; }\nfn T get<T>(ref Box<T> b) { b.v }\nstruct S { own P b; }\nfn main() {\n    let S[] ss = [S { b: own P { x: 12 } }];\n    for s in ss { let r = ref s.b; let f = () -> { let bx = Box { v: r }; get(ref bx).x }; print(f()); }\n}"),
        ("k_arrfor_take", "struct P { i64 x; }\nstruct Box<T> { T v; }\nfn T get<T>(ref Box<T> b) { b.v }\nstruct A { (own P)[2] a; }\nfn main() {\n    let A[] as2 = [A { a: [own P { x: 9 }, own P { x: 10 }] }];\n    for x in as2 { for p in x.a { let bx = Box { v: p }; print(get(ref bx).x); } let q = x.a[0]; print(q.x); }\n}"),
    ];
    for (name, src) in runs.iter() {
        let o = jabberwock::run_source(src, false);
        assert_eq!(o.error.is_none(), name.starts_with("u_"), "{}: {:?}", name, o.error.as_ref().map(|d| d.title.clone()));
        same_as_interp(name, src);
    }
}

#[test]
fn spawn_fails_with_an_error_when_no_thread_can_be_made() {
    // スレッドを作れないとき（環境変数 JW_MAX_THREADS で上限を下げて起こす）は、spawn の位置でエラー。固まらない。
    // 環境変数はほかのテストに効かないよう、コマンドの jabberwock と実行ファイルにだけ渡す
    let sent = "struct P { i64 x; }\nfn main() {\n    let p = own P { x: 1 };\n    let t = spawn(() -> p.x);\n    print(t.join());\n}";
    // 1 つめのスレッドが待っているあいだに 2 つめを作る（上限 1 なら必ず作れない）
    let blocked = "fn main() {\n    let chan<i64> c = chan();\n    let w = spawn(() -> c.recv());\n    let t = spawn(() -> 2);\n    \
                   c.send(1);\n    print(w.join(), t.join());\n}";
    for (name, src, limit) in [("spawn_fail_sent", sent, "0"), ("spawn_fail_blocked", blocked, "1")] {
        let dir = std::env::temp_dir().join(format!("jw-native-test-{}-{}", std::process::id(), name));
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(dir.join("main.jw"), src).expect("writes source");
        let o = Command::new(env!("CARGO_BIN_EXE_jabberwock"))
            .arg("main.jw")
            .current_dir(&dir)
            .env("JW_MAX_THREADS", limit)
            .output()
            .expect("runs jabberwock");
        let want = String::from_utf8_lossy(&o.stderr).to_string();
        assert_eq!(o.status.code(), Some(1), "{}: {}", name, want);
        assert!(want.starts_with("error: スレッドを作れませんでした\n"), "{}: {}", name, want);
        assert!(want.contains("ここで spawn した"), "{}: {}", name, want);
        if have_clang() {
            let ll = llvm_build(src, &no_modules, "main.jw", false).expect("lowers to LLVM");
            let exe = native::compile(&ll, &dir, name, "-O1").expect("clang compiles");
            let out = Command::new(&exe).env("JW_MAX_THREADS", limit).output().expect("runs");
            assert_eq!(out.status.code(), Some(1), "{}", name);
            assert_eq!(String::from_utf8_lossy(&out.stderr), want, "{}: エラー表示", name);
            // 上限がなければふつうに動く
            let out = Command::new(&exe).output().expect("runs");
            assert_eq!(out.status.code(), Some(0), "{}: {}", name, String::from_utf8_lossy(&out.stderr));
            // --release の実行ファイルは JW_MAX_THREADS を見ない（テスト用の仕組みを本番に残さない）
            let ll = llvm_build(src, &no_modules, "main.jw", true).expect("lowers to LLVM");
            let exe = native::compile(&ll, &dir, &format!("{}-release", name), "-O1").expect("clang compiles");
            let out = Command::new(&exe).env("JW_MAX_THREADS", limit).output().expect("runs");
            assert_eq!(out.status.code(), Some(0), "{} --release: {}", name, String::from_utf8_lossy(&out.stderr));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn temporary_receiver_frees_its_owns_with_the_same_reason() {
    // 一時的な構造体の this を捨てたとき、中の own は「使い終わった一時的な値」として解放される（interp も変数を捨てるのと同じ付け方。差分テストで見つかったずれ）
    let head = "struct P { i64 x; }\nstruct S { (own P)[] ps; own P b;\n    fn ref P first() { ref ps[0] }\n    fn ref P bb() { ref b }\n}\n\
                fn S mk(i64 n) { S { ps: [own P { x: n + 1 }], b: own P { x: n } } }\n";
    same_as_interp("temp_this_field", &format!("{}fn main() {{\n    let u = mk(4).bb();\n    print(u.x);\n}}", head));
    same_as_interp("temp_this_list", &format!("{}fn main() {{\n    let u = mk(4).first();\n    print(u.x);\n}}", head));
}

#[test]
fn measurement_builds_keep_the_output_and_count_checks() {
    // ベンチマーク用の切り替え（bench/run.sh）: 検査を全部残す・全部消す・数える。正しいプログラムなら出力は同じ
    if !have_clang() {
        return;
    }
    use jabberwock::ir::llvm::Opts;
    let src = "struct P { i64 x; }\nfn i64 sum(ref P[] ps) {\n    let mut s = 0;\n    for i in range(ps.len()) { s += ps[i].x; }\n    s\n}\n\
               fn main() {\n    let P[] mut ps = [];\n    for i in range(10) { ps.push(P { x: i }); }\n    let r = ref ps[3];\n    print(sum(ps), r.x);\n}";
    let dir = std::env::temp_dir().join(format!("jw-native-test-{}-measure", std::process::id()));
    let variants = [
        ("normal", true, Opts::default()),
        ("all", false, Opts::default()),
        ("none", true, Opts { gen: false, bounds: false, count: false }),
        ("count", true, Opts { count: true, ..Opts::default() }),
    ];
    for (name, elide, opts) in variants {
        let (ll, st) = jabberwock::llvm_build_with(src, &no_modules, "main.jw", false, elide, opts).expect("lowers to LLVM");
        assert!(st.bounds > 0, "{}: {:?}", name, st);
        if !elide {
            assert_eq!(st.elided, 0, "{}: {:?}", name, st);
        }
        let exe = native::compile(&ll, &dir, name, "-O1").expect("clang compiles");
        let out = Command::new(&exe).output().expect("runs");
        assert_eq!(String::from_utf8_lossy(&out.stdout), "45 3\n", "{}", name);
        let err = String::from_utf8_lossy(&out.stderr);
        if opts.count {
            // 添字は 10 回（sum の中）と ps[3]、世代は r.x の 1 回
            assert!(err.starts_with("[checks] gen "), "{}", err);
            let n: Vec<u64> = err.split_whitespace().filter_map(|w| w.parse().ok()).collect();
            assert_eq!(n.len(), 3, "{}", err);
            assert!(n[2] >= 11, "{}", err);
        } else {
            assert!(err.is_empty(), "{}: {}", name, err);
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sorting_matches_interp() {
    // 比べた順まで同じ（比べる関数の中で数えた値を出す）
    same_as_interp("sort_patterns", include_str!("sort_patterns.jw"));
}

#[test]
fn number_methods_match_interp() {
    same_as_interp(
        "nummeth",
        "fn main() {
    let x = 2.0;
    let own f64 p = own 9.0;
    print(x.sqrt(), (x - 5.0).abs(), x.max(3), x.min(1.5), p.sqrt(), (16.0).sqrt());
    print((-1.0).sqrt(), (0.0 / 0.0).max(1.0), x.max(0.0 / 0.0));
    let i32 a = -7;
    let u8 b = 200;
    let i64 m = -9223372036854775807 - 1;
    let i8 n = -128;
    let ref i32 r = ref a;
    print(a.abs(), a.max(3), a.min(-10), b.max(100), b.min(201), b.abs(), m.abs(), n.abs(), r.abs());
    let u64 big = 18446744073709551615;
    print(big.max(1), big.min(1), (3 - 10).abs());
}",
    );
}
