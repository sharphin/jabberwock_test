use std::{env, fs, process};

fn main() {
    jabberwock::main_in_big_stack(real_main);
}

fn real_main() {
    if env::args().nth(1).as_deref() == Some("build") {
        build(env::args().skip(2).collect());
        return;
    }
    let mut stats = false;
    let mut elide = true;
    let mut show_ir = false;
    let mut show_ll = false;
    let mut show_tokens = false;
    let mut show_ast = false;
    let mut show_typed = false;
    let mut show_ir_raw = false;
    let mut path = None;
    let mut prog_args = Vec::new();
    for a in env::args().skip(1) {
        if path.is_some() {
            // ファイル名のあとの引数は、プログラムの `args()` に渡す
            prog_args.push(a);
            continue;
        }
        match a.as_str() {
            "-h" | "--help" => {
                eprintln!("使い方: jabberwock [--stats] [--no-elide] [--ir] [--ll] <file.jw> [プログラムへの引数...]");
                eprintln!("        jabberwock build [--release] [-o 出力] <file.jw>   実行ファイルを作る（clang が要る）");
                eprintln!("パッケージ（alice.toml のあるフォルダ）は `alice` コマンドで扱う（docs/packages.md）");
                eprintln!("  --stats     ref の検査を静的解析でどれだけ消せたか表示する");
                eprintln!("  --no-elide  静的解析をせず、すべての ref を実行時に検査する");
                eprintln!("  --ir        実行せず、IR（中間表現）を表示する");
                eprintln!("  --ll        実行せず、LLVM IR を表示する");
                process::exit(0);
            }
            "--stats" => stats = true,
            "--no-elide" => elide = false,
            "--ir" => show_ir = true,
            "--ll" => show_ll = true,
            "--tokens" => show_tokens = true,
            "--ast" => show_ast = true,
            "--typed" => show_typed = true,
            "--ir-raw" => show_ir_raw = true,
            _ => path = Some(a),
        }
    }
    let Some(path) = path else {
        eprintln!("使い方: jabberwock [--stats] [--no-elide] [--ir] [--ll] <file.jw> [プログラムへの引数...]");
        process::exit(2);
    };

    let src = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("ファイルを読めません: {}: {}", path, e);
            process::exit(1);
        }
    };

    // `use geom;` は、このファイルと同じフォルダの geom.jw を読む
    let dir = std::path::Path::new(&path).parent().map(|d| d.to_path_buf()).unwrap_or_default();
    let read = move |name: &str| fs::read_to_string(dir.join(format!("{}.jw", name))).ok();

    if show_tokens {
        println!("{}", jabberwock::token_dump(&src, &path));
        return;
    }

    if show_ast {
        println!("{}", jabberwock::ast_dump(&src, &read, &path));
        return;
    }

    if show_ir_raw {
        print!("{}", jabberwock::ir_dump(&src, &read, &path, false));
        return;
    }

    if show_typed {
        println!("{}", jabberwock::typed_dump(&src, &read, &path));
        return;
    }

    if show_ll {
        match jabberwock::llvm_build_with(&src, &read, &path, false, elide, jabberwock::ir::llvm::Opts::default()) {
            Ok((t, _)) => print!("{}", t),
            Err(e) => {
                eprintln!("{}", e);
                process::exit(1);
            }
        }
        return;
    }

    if show_ir {
        match jabberwock::ir_text(&src, &read) {
            Ok(t) => print!("{}", t),
            Err(e) => {
                eprintln!("{}", e);
                process::exit(1);
            }
        }
        return;
    }

    // 警告は実行の前に出す（Rust と同じ）
    let (warnings, files) = jabberwock::warnings(&src, &read);
    for w in warnings {
        eprint!("{}", jabberwock::render(&w, &path, &files));
    }
    let input = Box::new(std::io::BufReader::new(std::io::stdin()));
    let outcome = jabberwock::run_program(&src, &read, true, elide, input, prog_args);
    if stats {
        let a = outcome.analysis;
        eprintln!("[stats] `*p` と `p.f` の箇所: 検査を消せた {} / 実行時に残る {}", a.elided, a.required);
        eprintln!(
            "[stats] ref の世代チェック: 実行 {} 回 / 省略 {} 回",
            outcome.ref_checks, outcome.ref_checks_elided
        );
    }
    if let Some(d) = &outcome.error {
        eprint!("{}", jabberwock::render(d, &path, &outcome.files));
        process::exit(1);
    }
    if let Some(code) = outcome.exit_code {
        process::exit(code);
    }
}

/// `jabberwock build [--release] [-o 出力] <file.jw>`: LLVM に書き出し、clang で実行ファイルを作る
fn build(args: Vec<String>) {
    let usage = "使い方: jabberwock build [--release] [-o 出力] [測定用の切り替え] <file.jw>";
    let mut release = false;
    let mut debug_native = false;
    // 測定用（ベンチマークで検査の費用を分ける）。--no-gen-checks / --no-bounds-checks / --unchecked は安全でない
    let mut elide = true;
    let mut stats = false;
    let mut opts = jabberwock::ir::llvm::Opts::default();
    let mut out = None;
    let mut path = None;
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--release" => release = true,
            "--debug-native" => debug_native = true,
            "--no-elide" => elide = false,
            "--no-gen-checks" => opts.gen = false,
            "--no-bounds-checks" => opts.bounds = false,
            "--unchecked" => {
                opts.gen = false;
                opts.bounds = false;
            }
            "--count-checks" => opts.count = true,
            "--stats" => stats = true,
            "-o" => match it.next() {
                Some(o) => out = Some(o),
                None => {
                    eprintln!("{}", usage);
                    process::exit(2);
                }
            },
            "-h" | "--help" => {
                eprintln!("{}", usage);
                eprintln!("  --release  実行時エラーは使った位置だけ出す（ソースを実行ファイルに入れない）");
                eprintln!("  --debug-native  初回ブートストラップ用に -O1 / LTOなしでネイティブ生成（-O0 だと jwc が自分をコンパイルするときスタックが足りない）");
                eprintln!("  -o 出力     実行ファイルの名前（なければファイル名から .jw を除いたもの）");
                eprintln!("測定用（ベンチマークで検査の費用を分けるため）:");
                eprintln!("  --no-elide          検査を消す解析をしない（世代の検査を全部残す）");
                eprintln!("  --no-gen-checks     ref の世代を確かめない（安全でない。上限値を測るだけ）");
                eprintln!("  --no-bounds-checks  添字の範囲を確かめない（安全でない）");
                eprintln!("  --unchecked         上の 2 つの両方");
                eprintln!("  --count-checks      実行時に通った検査の数を、終わるときに標準エラーへ出す（Linux）");
                eprintln!("  --stats             書き出した検査の場所の数を出す");
                process::exit(0);
            }
            _ if path.is_none() => path = Some(a),
            _ => {
                eprintln!("{}", usage);
                process::exit(2);
            }
        }
    }
    let Some(path) = path else {
        eprintln!("{}", usage);
        process::exit(2);
    };
    let src = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("ファイルを読めません: {}: {}", path, e);
            process::exit(1);
        }
    };
    let dir = std::path::Path::new(&path).parent().map(|d| d.to_path_buf()).unwrap_or_default();
    let read = move |name: &str| fs::read_to_string(dir.join(format!("{}.jw", name))).ok();
    let (warnings, files) = jabberwock::warnings(&src, &read);
    for w in warnings {
        eprint!("{}", jabberwock::render(&w, &path, &files));
    }
    let ll = match jabberwock::llvm_build_with(&src, &read, &path, release, elide, opts) {
        Ok((t, st)) => {
            if stats {
                eprintln!("[stats] 世代の検査 {} か所・解析で消した {} か所・添字の検査 {} か所", st.gen, st.elided, st.bounds);
            }
            t
        }
        Err(e) => {
            eprint!("{}", e);
            process::exit(1);
        }
    };
    let out = out.unwrap_or_else(|| {
        let stem = std::path::Path::new(&path).file_stem().and_then(|s| s.to_str()).unwrap_or("a");
        stem.to_string()
    });
    match jabberwock::native::build(&ll, std::path::Path::new(&out), if debug_native { "-O1" } else { "-O2" }) {
        Ok(exe) => eprintln!("作りました: {}", exe.display()),
        Err(e) => {
            eprintln!("{}", e);
            if e.starts_with("clang を起動できません") {
                eprintln!("clang が要ります。入れ方は docs/native.md を見てください（別の clang を使うなら JW_CLANG）");
            }
            process::exit(1);
        }
    }
}
