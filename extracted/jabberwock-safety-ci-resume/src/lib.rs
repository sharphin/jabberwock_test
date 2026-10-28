//! jabberwock: `own` と世代付き `ref` だけで安全を目指す小さな言語の核。

pub mod analyze;
pub mod astdump;
pub mod ast;
pub mod diag;
pub mod interp;
pub mod ir;
pub mod lexer;
pub mod modules;
pub mod native;
pub mod parser;
pub mod project;
mod sort;
pub mod threads;
pub mod types;

use diag::Diagnostic;

/// 実行結果
pub struct Outcome {
    /// `print` が出した内容
    pub output: String,
    /// `eprint` が出した内容（標準エラー）
    pub errput: String,
    /// `exit(n)` で終わったときの終了コード（このとき error は None）
    pub exit_code: Option<i32>,
    /// エラーがあれば診断
    pub error: Option<Diagnostic>,
    /// 警告（実行は止めない）
    pub warnings: Vec<Diagnostic>,
    /// 実行後に解放されずに残ったヒープオブジェクト数（0 ならリークなし）
    pub live_objects: usize,
    /// 静的解析の結果（`*p` のうち検査を消せた箇所・残った箇所）
    pub analysis: analyze::Stats,
    /// 実行時に行った ref の世代チェックの回数
    pub ref_checks: usize,
    /// 静的解析のおかげで飛ばせた ref の世代チェックの回数
    pub ref_checks_elided: usize,
    /// 読んだファイル（モジュール名、ソース）。診断の位置の `Span.file` がこの番号。0 番は最初のファイル
    pub files: Vec<(String, String)>,
}

/// モジュール名からソースを返す（見つからなければ None）
pub type Reader<'a> = &'a (dyn Fn(&str) -> Option<String> + Sync);

/// モジュールを読まない（`use` はエラーになる）
pub fn no_modules(_: &str) -> Option<String> {
    None
}

/// 診断を、読んだファイルの中の位置で出す。`main` は最初のファイルの名前、モジュールは `名前.jw`
pub fn render(d: &Diagnostic, main: &str, files: &[(String, String)]) -> String {
    render_named(d, main, files, &|m| format!("{}.jw", m.trim_start_matches('*')))
}

/// `render` の、モジュールのファイルの表示名を `name_of(モジュール名)` で決めるもの（パッケージでは src/geom.jw など）
pub fn render_named(d: &Diagnostic, main: &str, files: &[(String, String)], name_of: &dyn Fn(&str) -> String) -> String {
    let names: Vec<String> =
        files.iter().enumerate().map(|(i, (m, _))| if i == 0 { main.to_string() } else { name_of(m) }).collect();
    let fs: Vec<(&str, &str)> = names.iter().zip(files).map(|(n, (_, s))| (n.as_str(), s.as_str())).collect();
    if fs.is_empty() {
        return d.render(main, "");
    }
    d.render_in(&fs)
}

/// `jabberwock --tokens`: トークンを 1 行に 1 つ出す（jwc の字句解析との突き合わせ用。形は jwc/main.jw の `dump` と同じ）。
/// 埋め込みのある文字列は平らに並べる（interp → lit / code と中のトークン → end）。エラーなら診断の表示
pub fn token_dump(src: &str, path: &str) -> String {
    fn escape(s: &str) -> String {
        let mut out = String::new();
        for c in s.chars() {
            match c {
                '\\' => out.push_str("\\\\"),
                '"' => out.push_str("\\\""),
                '\n' => out.push_str("\\n"),
                '\t' => out.push_str("\\t"),
                '\r' => out.push_str("\\r"),
                '\0' => out.push_str("\\0"),
                _ => out.push(c),
            }
        }
        out
    }
    fn dump(toks: &[lexer::Token], lines: &mut Vec<String>) {
        for t in toks {
            let head = |k: &str| format!("{}:{}:{} {}", t.span.line, t.span.col, t.span.len, k);
            let line = match &t.tok {
                lexer::Tok::Ident(s) => format!("{} {}", head("ident"), s),
                lexer::Tok::Float(s) => format!("{} {}", head("float"), s),
                lexer::Tok::Int(n) => format!("{} {}", head("int"), n),
                lexer::Tok::Str(s) => format!("{} \"{}\"", head("str"), escape(s)),
                lexer::Tok::Eof => head("eof"),
                lexer::Tok::Interp(parts) => {
                    lines.push(head("interp"));
                    for p in parts {
                        match p {
                            lexer::StrPart::Lit(s) => lines.push(format!("lit \"{}\"", escape(s))),
                            lexer::StrPart::Code(inner) => {
                                lines.push("code".to_string());
                                dump(inner, lines);
                            }
                        }
                    }
                    "end".to_string()
                }
                other => head(other.describe().trim_matches('`')),
            };
            lines.push(line);
        }
    }
    match lexer::lex(src) {
        Ok(toks) => {
            let mut lines = Vec::new();
            dump(&toks, &mut lines);
            lines.join("\n")
        }
        Err(d) => d.render(path, src),
    }
}

/// どのプログラムにも最初からある定義。`Some(x)` / `None` / `Ok(x)` / `Err(e)` は
/// `Option.Some(x)` などの短い書き方
const PRELUDE: &str = "enum Option<T> {\n    Some(T value);\n    None;\n\
    fn bool is_some() { match this { case Some(_) -> true; case None -> false; } }\n\
    fn bool is_none() { match this { case Some(_) -> false; case None -> true; } }\n\
    fn take T unwrap_or(T _d) { match this { case Some(_v) -> _v; case None -> _d; } }\n\
    fn take Result<T, E> ok_or<E>(E _e) { match this { case Some(_v) -> Ok(_v); case None -> Err(_e); } }\n\
    fn take Option<U> map<U>(fn U(T) _f) { match this { case Some(_v) -> Some(_f(_v)); case None -> None; } }\n}\n\
enum Result<T, E> {\n    Ok(T value);\n    Err(E error);\n\
    fn bool is_ok() { match this { case Ok(_) -> true; case Err(_) -> false; } }\n\
    fn bool is_err() { match this { case Ok(_) -> false; case Err(_) -> true; } }\n\
    fn take T unwrap_or(T _d) { match this { case Ok(_v) -> _v; case Err(_) -> _d; } }\n\
    fn take Option<T> ok() { match this { case Ok(_v) -> Some(_v); case Err(_) -> None; } }\n\
    fn take Result<U, E> map<U>(fn U(T) _f) { match this { case Ok(_v) -> Ok(_f(_v)); case Err(_e) -> Err(_e); } }\n}\n\
fn Result<T, E> ok_or<T, E>(Option<T> _o, E _e) {\n    match _o {\n        case Some(_v) -> Ok(_v);\n        case None -> Err(_e);\n    }\n}\n";

/// 字句解析と構文解析をして（`use` したモジュールも読む）、最初からある定義（`Option`）を前に足す。
/// 読んだファイルも返す（エラーの表示に使う）
fn parse(src: &str, read: Reader) -> (Result<ast::Program, Diagnostic>, Vec<(String, String)>) {
    let mut loader = modules::Loader::new(read);
    let r = loader.load(src).map(add_prelude);
    (r, loader.files)
}

/// `jabberwock --ast`: 構文解析した結果（最初からある定義も含む）を文字にする（jwc の構文解析との突き合わせ用）。エラーなら診断の表示
pub fn ast_dump(src: &str, read: Reader, path: &str) -> String {
    let (r, files) = parse(src, read);
    match r {
        Ok(p) => astdump::dump(&p),
        Err(d) => render(&d, path, &files),
    }
}

/// `jabberwock --typed`: 型検査の結果つきの構文木（jwc の型検査との突き合わせ用）。警告は前に、エラーなら診断の表示
pub fn typed_dump(src: &str, read: Reader, path: &str) -> String {
    let (r, files) = parse(src, read);
    let prog = match r {
        Ok(p) => p,
        Err(d) => return render(&d, path, &files),
    };
    match types::check_with_info(&prog) {
        Ok((warnings, info)) => {
            let mut out: String = warnings.iter().map(|w| render(w, path, &files)).collect();
            out += &astdump::dump_typed(&prog, &info);
            out
        }
        Err(d) => render(&d, path, &files),
    }
}

fn add_prelude(mut prog: ast::Program) -> ast::Program {
    let prelude = parser::Parser::new(lexer::lex(PRELUDE).expect("prelude lexes"))
        .parse_program()
        .expect("prelude parses");
    let mut enums = prelude.enums;
    enums.append(&mut prog.enums);
    prog.enums = enums;
    // Option を Result に変える `ok_or(o, e)`（`?` で使う）
    let mut fns = prelude.fns;
    fns.append(&mut prog.fns);
    prog.fns = fns;
    prog
}

/// 構文解析と型検査をして、プログラムと型の記録を返す（IR への書き出しや、そのテスト用）。
/// `TypeInfo` は構文木の中の式の位置で引くので、返したプログラムは書き換えないこと
pub fn typed(src: &str, read: Reader) -> Result<(ast::Program, types::TypeInfo), Diagnostic> {
    let prog = parse(src, read).0?;
    let (_, info) = types::check_with_info(&prog)?;
    Ok((prog, info))
}

/// IR（中間表現）を読みやすい文字にする（`jabberwock --ir`）。エラーや、まだ書き出せない書き方があれば Err
pub fn ir_text(src: &str, read: Reader) -> Result<String, String> {
    let (prog, info) = typed(src, read).map_err(|d| d.title)?;
    ir::lower::lower(&prog, &info)
        .map(|mut p| {
            ir::analyze::analyze(&mut p);
            p.to_string()
        })
        .map_err(|r| format!("まだ IR に書き出せない書き方があります: {}", r))
}

/// jwc の `--ir` / `--ir-raw` と突き合わせる IR の表示。`analyze` なら検査を消す解析のあと。
/// 構文・型のエラーは診断の表示、書き出せなければその理由
pub fn ir_dump(src: &str, read: Reader, path: &str, analyze: bool) -> String {
    let (r, files) = parse(src, read);
    let prog = match r {
        Ok(p) => p,
        Err(d) => return render(&d, path, &files),
    };
    let info = match types::check_with_info(&prog) {
        Ok((_, info)) => info,
        Err(d) => return render(&d, path, &files),
    };
    match ir::lower::lower(&prog, &info) {
        Ok(mut p) => {
            if analyze {
                ir::analyze::analyze(&mut p);
            }
            p.to_string()
        }
        Err(r) => format!("まだ IR に書き出せない書き方があります: {}\n", r),
    }
}

/// LLVM のテキスト（`jabberwock --ll`）。`main` は最初のファイルの名前（エラーの位置に出す）
pub fn llvm_text(src: &str, read: Reader, main: &str) -> Result<String, String> {
    llvm_build(src, read, main, false)
}

/// `jabberwock build` 用の LLVM IR。`release` なら実行時エラーは使った位置だけ出す（ソースを入れない）。
/// 書けないときは、表示する文（ソースの行つきのエラー）を返す
pub fn llvm_build(src: &str, read: Reader, main: &str, release: bool) -> Result<String, String> {
    llvm_build_with(src, read, main, release, true, ir::llvm::Opts::default()).map(|(t, _)| t)
}

/// `llvm_build` に測定用の切り替えを足したもの（`jabberwock build --no-elide` など）。`elide` が false なら検査を消す解析をしない。
/// 書き出した検査の数も返す（`--stats`）
pub fn llvm_build_with(
    src: &str,
    read: Reader,
    main: &str,
    release: bool,
    elide: bool,
    opts: ir::llvm::Opts,
) -> Result<(String, ir::llvm::Stats), String> {
    let (prog, files) = parse(src, read);
    let names: Vec<String> =
        files.iter().enumerate().map(|(i, (m, _))| if i == 0 { main.to_string() } else { format!("{}.jw", m.trim_start_matches('*')) }).collect();
    let prog = prog.map_err(|d| render(&d, main, &files))?;
    let (_, info) = types::check_with_info(&prog).map_err(|d| render(&d, main, &files))?;
    let mut irp = ir::lower::lower(&prog, &info).map_err(|r| format!("まだ IR に書き出せない書き方があります: {}\n", r))?;
    if elide {
        ir::analyze::analyze(&mut irp);
    }
    let sources: Vec<String> = if release { Vec::new() } else { files.iter().map(|(_, t)| t.clone()).collect() };
    ir::llvm::emit_with(&irp, &names, &sources, opts).map_err(|r| format!("まだ LLVM に書き出せない書き方があります: {}\n", r))
}

/// 実行せずに型検査だけして、警告と読んだファイルを返す（エラーがあれば警告は空。エラーは実行時に返る）
pub fn warnings(src: &str, read: Reader) -> (Vec<Diagnostic>, Vec<(String, String)>) {
    let (prog, files) = parse(src, read);
    let Ok(prog) = prog else { return (Vec::new(), files) };
    (types::check(&prog).unwrap_or_default(), files)
}

/// ソースを実行する。静的解析で証明できた箇所の検査は消す。
pub fn run_source(src: &str, echo: bool) -> Outcome {
    run_source_with(src, echo, true)
}

/// `elide` が false なら静的解析をせず、すべての ref を実行時に検査する（比較用）。
pub fn run_source_with(src: &str, echo: bool, elide: bool) -> Outcome {
    run_program(src, &no_modules, echo, elide, Box::new(std::io::empty()), Vec::new())
}

/// モジュール（名前とソース）を渡して実行する（テスト用）
pub fn run_with_modules(src: &str, mods: &[(&str, &str)]) -> Outcome {
    let mods: Vec<(String, String)> = mods.iter().map(|(n, s)| (n.to_string(), s.to_string())).collect();
    let read = move |name: &str| mods.iter().find(|(n, _)| n == name).map(|(_, s)| s.clone());
    run_program(src, &read, false, true, Box::new(std::io::empty()), Vec::new())
}

/// 入力（`read_line()` が読む）を文字列で渡して実行する（テスト用）
pub fn run_source_with_input(src: &str, input: &str, args: &[&str]) -> Outcome {
    let bytes = input.as_bytes().to_vec();
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let input = Box::new(std::io::Cursor::new(bytes.clone()));
    in_big_stack(|| run_inner(src, &no_modules, false, true, input, args, Some(bytes)))
}

/// `input` は `read_line()` が読む入力、`args` は `args()` が返す引数。
/// 深い再帰でも落ちないよう、大きなスタックのスレッドで動かす。
pub fn run_program(
    src: &str,
    read: Reader,
    echo: bool,
    elide: bool,
    input: Box<dyn std::io::BufRead + Send>,
    args: Vec<String>,
) -> Outcome {
    in_big_stack(|| run_inner(src, read, echo, elide, input, args, None))
}

/// 新しいスレッドのために、読んだファイルからプログラムを作り直す（型検査と解析も同じにする）
fn reloader(files: &[(String, String)], elide: bool) -> Box<dyn Fn() -> ast::Program + Send + Sync> {
    let files = files.to_vec();
    Box::new(move || {
        // 依存先の lib.jw の問い合わせ `空間*名前` は、ファイルの名前 `*名前` で探す（使えるかは最初に読んだときに確かめた）
        let read = |name: &str| {
            let name = match name.split_once('*') {
                Some((_, u)) => format!("*{}", u),
                None => name.to_string(),
            };
            files.iter().skip(1).find(|(n, _)| *n == name).map(|(_, s)| s.clone())
        };
        let (prog, _) = parse(&files[0].1, &read);
        let prog = prog.expect("parsed before");
        types::check_with_info(&prog).expect("checked before");
        if elide {
            analyze::analyze(&prog);
        }
        prog
    })
}

/// IR で動かすときの、新しいスレッドのための IR の作り直し
fn ir_reloader(files: &[(String, String)], elide: bool) -> Box<dyn Fn() -> ir::Program + Send + Sync> {
    let load = reloader(files, false);
    Box::new(move || {
        let prog = load();
        let (_, info) = types::check_with_info(&prog).expect("checked before");
        let mut irp = ir::lower::lower(&prog, &info).expect("lowered before");
        if elide { ir::analyze::analyze(&mut irp); }
        irp
    })
}

/// コマンドの入口。構文解析や型検査も入れ子をたどる再帰なので、実行と同じ大きなスタックで動かす
pub fn main_in_big_stack(f: fn()) {
    let t = std::thread::Builder::new().stack_size(1 << 30).spawn(f).expect("failed to spawn main thread");
    if t.join().is_err() {
        std::process::exit(101);
    }
}

fn in_big_stack(f: impl FnOnce() -> Outcome + Send) -> Outcome {
    std::thread::scope(|s| {
        std::thread::Builder::new()
            .stack_size(1 << 30)
            .spawn_scoped(s, f)
            .expect("failed to spawn interpreter thread")
            .join()
            .expect("interpreter thread panicked")
    })
}

fn run_inner(
    src: &str,
    read: Reader,
    echo: bool,
    elide: bool,
    input: Box<dyn std::io::BufRead + Send>,
    args: Vec<String>,
    replay: Option<Vec<u8>>,
) -> Outcome {
    let (prog, files) = parse(src, read);
    let fail = |d: Diagnostic| Outcome {
        output: String::new(),
        errput: String::new(),
        exit_code: None,
        error: Some(d),
        warnings: Vec::new(),
        live_objects: 0,
        analysis: analyze::Stats::default(),
        ref_checks: 0,
        ref_checks_elided: 0,
        files: files.clone(),
    };

    let prog = match prog {
        Ok(p) => p,
        Err(d) => return fail(d),
    };
    let (warnings, info) = match types::check_with_info(&prog) {
        Ok(w) => w,
        Err(d) => return fail(d),
    };
    let analysis = if elide { analyze::analyze(&prog) } else { analyze::Stats::default() };
    // デバッグビルドでは、書き出せるプログラムは IR でも実行して結果を突き合わせる（IR の書き出しの検査）
    let lowered = if cfg!(debug_assertions) { ir::lower::lower(&prog, &info).ok() } else { None };
    let lowered = lowered.map(|mut p| {
        if elide { ir::analyze::analyze(&mut p); }
        p
    });
    drop(info);
    let mut it = interp::Interp::new(echo);
    it.set_world(threads::World::new(echo, reloader(&files, elide)));
    let io = (replay, args.clone());
    it.set_io(input, args);
    let res = it.run(prog);
    // exit で終わったのはエラーではない
    let res = match res {
        Err(d) if d.title == threads::exit_marker().title => Ok(()),
        r => r,
    };
    let want = Expected {
        output: it.output(),
        errput: it.errput(),
        error: res.as_ref().err().cloned(),
        live: it.live_objects(),
        exit: it.exit_code(),
    };
    // 入力を使うプログラムは、同じ入力をもう一度渡せるときだけ突き合わせる
    // 通常実行後の IR/native 比較は同じプログラムをもう一度実行する。
    // ファイルや外部コマンドを扱うプログラムでは、二重書込み・コマンドの
    // 二重起動だけでなく、一度目の実行で変わった状態を読んでしまう。
    // 外部効果を持つプログラムの比較は、独立した環境を使う専用テストで行う。
    let lowered = lowered.filter(|p| (io.0.is_some() || !p.uses_input()) && !p.uses_external_system());
    if let Some(irp) = lowered {
        let input = io.0.clone().unwrap_or_default();
        if let Some(diff) = ir_mismatch(&irp, &files, &want, &input, &io.1, elide) {
            panic!("IR の実行結果がインタプリタと違います: {}\n--- IR ---\n{}", diff, irp);
        }
        // JW_NATIVE があれば、clang で作った実行ファイルとも突き合わせる（clang が要る）
        if std::env::var_os("JW_NATIVE").is_some() {
            if let Some(diff) = native_mismatch(&irp, &files, &want, &input, &io.1) {
                panic!("実行ファイルの結果がインタプリタと違います: {}\n--- IR ---\n{}", diff, irp);
            }
        }
    }
    Outcome {
        output: want.output,
        errput: want.errput,
        exit_code: want.exit,
        error: want.error,
        warnings,
        live_objects: want.live,
        analysis,
        ref_checks: it.ref_checks,
        ref_checks_elided: it.ref_checks_elided,
        files,
    }
}

/// インタプリタの実行結果（IR・実行ファイルと突き合わせる）
struct Expected {
    output: String,
    errput: String,
    error: Option<Diagnostic>,
    live: usize,
    exit: Option<i32>,
}

/// IR を実行して、インタプリタの結果（出力・エラーの題と位置・残ったヒープの数・終了コード）と違えばその説明を返す
fn ir_mismatch(irp: &ir::Program, files: &[(String, String)], want: &Expected, input: &[u8], args: &[String], elide: bool) -> Option<String> {
    let mut it = interp::Interp::new(false);
    it.set_world(threads::World::new_ir(ir_reloader(files, elide)));
    it.set_io(Box::new(std::io::Cursor::new(input.to_vec())), args.to_vec());
    let res = ir::exec::run(irp, &mut it);
    let res = match res {
        Err(d) if d.title == threads::exit_marker().title => Ok(()),
        r => r,
    };
    if it.output() != want.output {
        return Some(format!("出力が違う\ninterp:\n{}\nIR:\n{}", want.output, it.output()));
    }
    if it.errput() != want.errput {
        return Some(format!("標準エラーへの出力が違う\ninterp:\n{}\nIR:\n{}", want.errput, it.errput()));
    }
    if it.exit_code() != want.exit {
        return Some(format!("終了コードが違う（interp {:?}・IR {:?}）", want.exit, it.exit_code()));
    }
    let live = want.live;
    let key = |d: &Diagnostic| (d.title.clone(), d.labels.iter().find(|l| l.primary).map(|l| l.span));
    match (want.error.as_ref(), res.as_ref().err()) {
        // exit で終わったときは、片づけずに終わるので残りの数は比べない
        (None, None) if want.exit.is_some() => None,
        (None, None) if live != it.live_objects() => {
            Some(format!("残ったヒープの数が違う（interp {}・IR {}）", live, it.live_objects()))
        }
        (None, None) => None,
        (Some(a), Some(b)) if key(a) == key(b) => None,
        (a, b) => Some(format!("エラーが違う\ninterp: {:?}\nIR: {:?}", a.map(key), b.map(key))),
    }
}

/// LLVM に書き出して clang で作った実行ファイルを走らせ、インタプリタの結果と違えばその説明を返す
fn native_mismatch(irp: &ir::Program, files: &[(String, String)], want: &Expected, input: &[u8], args: &[String]) -> Option<String> {
    let (output, live) = (&want.output, want.live);
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let trace = std::env::var_os("JW_NATIVE_TRACE").is_some();
    let names: Vec<String> =
        files.iter().enumerate().map(|(i, (m, _))| if i == 0 { "main.jw".to_string() } else { format!("{}.jw", m.trim_start_matches('*')) }).collect();
    let sources: Vec<String> = files.iter().map(|(_, t)| t.clone()).collect();
    let ll = match ir::llvm::emit(irp, &names, &sources) {
        Ok(t) => t,
        Err(why) => {
            if trace {
                eprintln!("\nNATIVE-SKIP {}", why);
            }
            return None;
        }
    };
    let dir = std::env::temp_dir().join(format!("jw-native-{}", std::process::id()));
    let name = format!("p{}", COUNTER.fetch_add(1, Ordering::SeqCst));
    let exe = match native::compile_with_cached_runtime(&ll, &dir, &name) {
        Ok(e) => e,
        Err(e) => return Some(format!("{}\n--- ll ---\n{}", e, ll)),
    };
    let child = std::process::Command::new(&exe)
        .args(args)
        .env("JW_LEAKS", "1")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();
    let run = child.and_then(|mut c| {
        if let Some(mut si) = c.stdin.take() {
            let _ = si.write_all(input);
        }
        c.wait_with_output()
    });
    let out = match run {
        Ok(o) => o,
        Err(e) => return Some(format!("実行できません: {}", e)),
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    // 標準エラーは eprint の出力のあとに、エラーか `[live]` の行が続く
    let Some(stderr) = stderr.strip_prefix(want.errput.as_str()) else {
        return Some(format!("標準エラーへの出力が違う\ninterp:\n{}\n実行ファイル:\n{}", want.errput, stderr));
    };
    let diff = if stdout != *output {
        Some(format!("出力が違う\ninterp:\n{}\n実行ファイル:\n{}\n{}", output, stdout, stderr))
    } else if let Some(code) = want.exit {
        // Unix の終了コードは下の 8 ビットだけ残る（exit(-1) は 255）
        let low = |c: i32| if cfg!(unix) { c & 0xff } else { c };
        if out.status.code().map(low) != Some(low(code)) || !stderr.is_empty() {
            Some(format!("終了が違う（interp exit {}・実行ファイル {:?}）\n{}", code, out.status.code(), stderr))
        } else {
            None
        }
    } else {
        match want.error.as_ref() {
            None => {
                let got = stderr.lines().find_map(|l| l.strip_prefix("[live] ")).and_then(|n| n.trim().parse::<usize>().ok());
                if !out.status.success() {
                    Some(format!("実行ファイルが失敗した（{:?}）\n{}", out.status.code(), stderr))
                } else if got != Some(live) {
                    Some(format!("残ったヒープの数が違う（interp {}・実行ファイル {:?}）", live, got))
                } else {
                    None
                }
            }
            Some(d) => {
                let shown: Vec<(&str, &str)> = names.iter().zip(files).map(|(n, (_, t))| (n.as_str(), t.as_str())).collect();
                let want = d.render_in(&shown);
                if out.status.code() != Some(1) || stderr != want {
                    Some(format!("エラーが違う\ninterp:\n{}\n実行ファイル（{:?}）:\n{}", want, out.status.code(), stderr))
                } else {
                    None
                }
            }
        }
    };
    if diff.is_none() {
        let _ = std::fs::remove_file(&exe);
        let _ = std::fs::remove_file(dir.join(format!("{}.ll", name)));
        if trace {
            eprintln!("\nNATIVE-OK");
        }
    }
    diff.map(|d| format!("{}\n--- ll: {} ---", d, dir.join(format!("{}.ll", name)).display()))
}
