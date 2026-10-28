//! IR → LLVM のテキスト（.ll）。実行時ライブラリ `runtime/jw_rt.c` と一緒に clang でコンパイルする。
//! 値の形は docs/ir-design.md の「実行時の値の形」。
//!
//! - 変数はスタック（`alloca`）に置く。`ref x` で変数そのものを借りられる変数だけ、区画（ヒープ）に置く
//!   （変数がなくなったときに世代を進めて、借りた ref を無効にするため）。
//! - 変数ごとに「状態」を持つ: null なら値がある、`@jw.unset` ならまだない、それ以外はムーブした位置（`WSite`）。
//!   ムーブ済みの変数を読むと、その位置を使ってエラーを出す（interp と同じ）。
//! - 型ごとの手伝いの関数（解放・複製・写し・表示・比べる）を、使われた型の分だけ作る。
//! - own の「読むだけの写し」（for の要素など）は、下の 1 ビットを立てたポインタ（実行時ライブラリの `WCell`）。
//!
//! まだ書き出せないもの（クロージャなど）があれば `Err(理由)`。

use std::collections::HashMap;
use std::fmt::Write as _;

use super::*;
use crate::ast::{BinOp, IntTy, UnOp};

type G<T> = Result<T, String>;

const CHAIN: &str = "持ち主が解放されたので、中身も連鎖して解放された";
const MAX_DEPTH: usize = 5000;

/// 型ごとの手伝いの関数の種類
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum H {
    /// `void (ptr 値, ptr 解放の位置)`: 中の own をすべて解放する
    Drop,
    /// `void (ptr 元, ptr 先)`: 複製（文字列は数を増やす、own の写しは札を複製）
    Clone,
    /// `void (ptr 値, ptr 文字の入れ物, i64 深さ, ptr 使った位置)`: print の文字
    Show,
    /// `void (ptr 元, ptr 先, ptr 借りた位置)`: 読むだけの写し（own → 写しの札）
    View,
    /// `i1 (ptr 値)`: 値として取り出せないか（interp の `has_own() || moves()`）
    Hazard,
    /// `i32 (ptr a, ptr b, ptr env)`: a < b（sort）
    Less,
    /// `i1 (ptr 値)`: 値として読むとムーブになるか（interp の `moves()`。own の写しはムーブしない）
    Moves,
    /// `i1 (ptr 値)`: 中に own を持つか（interp の `has_own()`）
    HasOwn,
    /// `void (ptr 値, ptr 位置)`: ほかのスレッドへ送る準備（own の区画の世代を進め、文字列は複製する）
    Send,
}

/// プログラム全体を LLVM のテキストにする。`files` はファイルの番号から名前。
/// `sources` はファイルの中身（エラーでソースの行を出すため）。空なら位置だけ出す（`--release`）
pub fn emit(prog: &Program, files: &[String], sources: &[String]) -> G<String> {
    emit_with(prog, files, sources, Opts::default()).map(|(t, _)| t)
}

/// 書き出しの切り替え（ベンチマークで検査の費用を分けて測るため。ふだんは `Opts::default()`）
#[derive(Clone, Copy, Debug)]
pub struct Opts {
    /// ref の世代を確かめる（false なら安全でない。上限値を測るだけ）
    pub gen: bool,
    /// 添字の範囲を確かめる（false なら安全でない）
    pub bounds: bool,
    /// 実行時に通った検査の数を数え、終わるときに標準エラーへ `[checks] ...` と出す
    pub count: bool,
}

impl Default for Opts {
    fn default() -> Self {
        Opts { gen: true, bounds: true, count: false }
    }
}

/// 書き出した検査の数（場所の数。実行時の回数ではない）
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    /// 世代を確かめる場所
    pub gen: usize,
    /// 解析が世代の検査を消した場所
    pub elided: usize,
    /// 添字の範囲を確かめる場所
    pub bounds: usize,
}

/// `emit` に切り替えを渡したもの。書き出した検査の数も返す
pub fn emit_with(prog: &Program, files: &[String], sources: &[String], opts: Opts) -> G<(String, Stats)> {
    let mut m = Module::new(prog, files);
    m.opts = opts;
    for (i, f) in prog.fns.iter().enumerate() {
        FnGen::new(&mut m, f, i).run()?;
    }
    let main = *prog.by_name.get("main").ok_or("main がない")?;
    let mf = &prog.fns[main];
    let ret = m.lt(&mf.ret)?;
    let site = m.site(mf.span, "");
    let _ = write!(
        m.code,
        "define void @jw_main() {{\n  %r = call {} @f{}(i64 1)\n  call void @jw_wait_all(ptr {})\n{}  ret void\n}}\n\n",
        ret, main, site, if opts.count { "  call void @jw.count.report()\n" } else { "" }
    );
    m.finish_helpers()?;
    let mut entries = Vec::new();
    for (i, text) in sources.iter().enumerate() {
        let name = m.cstr(files.get(i).map_or("?", |s| s.as_str()));
        let text = m.cstr(text);
        entries.push(format!("%wsrc {{ ptr {}, ptr {} }}", name, text));
    }
    let _ = writeln!(
        m.globals,
        "@jw_files = constant [{} x %wsrc] [{}]\n@jw_nfiles = constant i32 {}",
        entries.len(),
        entries.join(", "),
        entries.len()
    );
    if opts.count {
        // 正常終了時に数を出す（exit と実行時エラーは _Exit で終わるため対象外）
        let _ = writeln!(
            m.globals,
            "@jw.count.gen = global i64 0\n@jw.count.elided = global i64 0\n@jw.count.bounds = global i64 0\n\
             declare void @jw_report_checks(i64, i64, i64)\n\
             define internal void @jw.count.report() {{\n  %g = load i64, ptr @jw.count.gen\n  %e = load i64, ptr @jw.count.elided\n  \
             %b = load i64, ptr @jw.count.bounds\n  call void @jw_report_checks(i64 %g, i64 %e, i64 %b)\n  ret void\n}}"
        );
    }
    let stats = m.stats;
    Ok((m.output(), stats))
}

/// 数える版なら、数を 1 つ足す（`what` は gen / elided / bounds）
fn count(b: &mut B, o: &Opts, what: &str) {
    if o.count {
        b.op(format!("atomicrmw add ptr @jw.count.{}, i64 1 monotonic", what));
    }
}

/// 型にもとづく別名の情報（TBAA）。リストの頭（要素の領域・長さ）と、リスト・辞書の要素の中の数やポインタは
/// 別の場所にあると LLVM に教える。要素へ書き込んでも長さを読み直さずに済み、添字の範囲の確認をループの外へ出せる。
/// 印のない読み書き（構造体ごとの写し・実行時ライブラリ）は、どれとも重なりうるものとして扱われる
const TBAA: &str = "!9000 = !{!\"jabberwock\"}\n!9001 = !{!\"jabberwock list header\", !9000, i64 0}\n!9002 = !{!\"jabberwock element\", !9000, i64 0}\n\
!9003 = !{!9001, !9001, i64 0}\n!9004 = !{!9002, !9002, i64 0}\n";
const TBAA_HEADER: &str = ", !tbaa !9003";
const TBAA_ELEMENT: &str = ", !tbaa !9004";

/// 1 回の load / store で読み書きする型（要素の印をつけてよいもの）
fn scalar_lt(lt: &str) -> bool {
    matches!(lt, "i1" | "i8" | "i16" | "i32" | "i64" | "double" | "ptr")
}

const PRELUDE: &str = r#"; jabberwock が書き出した LLVM IR
%ref = type { ptr, ptr, i64, ptr }
%iref = type { ptr, ptr, i64, ptr, i64 }
%iown = type { ptr, i64 }
%list = type { ptr, i64, i64 }
%site = type { ptr, i32, i32, i32, i32, ptr, ptr }
%fnv = type { ptr, ptr }
%wsrc = type { ptr, ptr }

@jw_depth = external thread_local(initialexec) global i64
@jw.unset = private constant %site { ptr null, i32 0, i32 0, i32 0, i32 0, ptr null, ptr null }

declare double @llvm.sqrt.f64(double)
declare double @llvm.fabs.f64(double)
declare double @llvm.maxnum.f64(double, double)
declare double @llvm.minnum.f64(double, double)
declare void @jw_print_begin()
declare ptr @jw_chan_new(i64, ptr)
declare void @jw_chan_retain(ptr)
declare void @jw_chan_release(ptr, ptr)
declare void @jw_chan_send(ptr, ptr, ptr)
declare i32 @jw_chan_recv(ptr, ptr, ptr)
declare void @jw_chan_close(ptr, ptr)
declare ptr @jw_spawn(ptr, ptr, i64, ptr, ptr)
declare ptr @jw_spawn_args(i64)
declare void @jw_task_join(ptr, ptr, ptr)
declare void @jw_task_drop(ptr, ptr)
declare ptr @jw_task_view(ptr)
declare void @jw_wait_all(ptr)
declare void @jw_send_block(ptr, ptr)
declare ptr @jw_str_unshare(ptr)
declare void @jw_map_unshare_keys(ptr)
declare void @jw_print_part(ptr, i32)
declare void @jw_print_end()
declare void @jw_fail(ptr, ptr, ptr, ptr) noreturn
declare void @jw_bug(ptr) noreturn
declare void @jw_fail_moved(ptr, ptr, ptr) noreturn
declare void @jw_fail_dangling(ptr, ptr, ptr, i64) noreturn
declare void @jw_fail_bounds(i64, i32, i64, ptr) noreturn
declare void @jw_fail_depth(ptr) noreturn
declare void @jw_fail_nokey(i32, i64, i32, ptr) noreturn
declare void @jw_fail_insert(i64, i64, ptr) noreturn
declare ptr @jw_alloc(i64)
declare void @jw_free(ptr, i64, ptr)
declare ptr @jw_view(ptr, ptr)
declare void @jw_view_free(ptr)
declare ptr @jw_view_target(ptr, ptr)
declare ptr @jw_view_target_nocheck(ptr)
declare i64 @jw_view_gen(ptr)
declare ptr @jw_view_site(ptr)
declare ptr @jw_str_new(ptr, i64)
declare void @jw_str_retain(ptr)
declare void @jw_str_release(ptr)
declare ptr @jw_str_concat(ptr, ptr)
declare i32 @jw_str_cmp(ptr, ptr)
declare i32 @jw_str_eq(ptr, ptr)
declare ptr @jw_sb_new()
declare void @jw_sb_cstr(ptr, ptr)
declare void @jw_sb_str(ptr, ptr)
declare void @jw_sb_int(ptr, i64, i32)
declare void @jw_sb_float(ptr, double)
declare ptr @jw_sb_finish(ptr)
declare ptr @jw_str_int(i64, i32)
declare i64 @jw_f2i(double, i32, i32)
declare void @jw_list_reserve(ptr, i64, i64)
declare ptr @jw_list_push_slot(ptr, i64, ptr)
declare ptr @jw_list_insert_slot(ptr, i64, i64, ptr)
declare void @jw_list_remove(ptr, i64, i64, ptr, ptr)
declare void @jw_list_reverse(ptr, i64, ptr)
declare void @jw_list_moved(ptr, ptr)
declare void @jw_list_free(ptr, i64, ptr)
declare void @jw_sort(ptr, i64, i64, ptr, ptr)
declare void @jw_sort_unstable(ptr, i64, i64, ptr, ptr)
declare void @jw_sort_int(ptr, i64, i32, i32)
declare void @jw_permute(ptr, i64, i64, ptr)
declare ptr @jw_env_new(i64, ptr)
declare void @jw_env_retain(ptr)
declare void @jw_env_release(ptr, ptr)
declare ptr @malloc(i64)
declare void @free(ptr)
declare ptr @jw_map_new(i64, i32)
declare i64 @jw_map_find(ptr, i64)
declare i64 @jw_map_add(ptr, i64)
declare ptr @jw_map_val(ptr, i64)
declare i64 @jw_map_key(ptr, i64)
declare i64 @jw_map_len(ptr)
declare void @jw_map_remove_at(ptr, i64)
declare void @jw_map_free(ptr)
declare i64 @jw_str_chars(ptr)
declare void @jw_str_chars_list(ptr, ptr)
declare void @jw_str_lines(ptr, ptr)
declare void @jw_str_split(ptr, ptr, ptr, ptr)
declare ptr @jw_str_trim(ptr)
declare ptr @jw_str_upper(ptr)
declare ptr @jw_str_lower(ptr)
declare i32 @jw_str_contains(ptr, ptr)
declare i32 @jw_str_starts(ptr, ptr)
declare i32 @jw_str_ends(ptr, ptr)
declare i64 @jw_str_find(ptr, ptr)
declare ptr @jw_str_slice(ptr, i64, i64, ptr)
declare ptr @jw_str_replace(ptr, ptr, ptr)
declare ptr @jw_str_repeat(ptr, i64, ptr)
declare i32 @jw_str_to_int(ptr, ptr)
declare i32 @jw_str_to_float(ptr, ptr)
declare i32 @jw_read_line(ptr)
declare void @jw_eprint_begin()
declare void @jw_eprint_part(ptr, i32)
declare void @jw_eprint_end()
declare i32 @jw_read_file(ptr, ptr)
declare i32 @jw_write_file(ptr, ptr, ptr)
declare i32 @jw_run_command(ptr, ptr, ptr, ptr)
declare void @jw_exit(i64) noreturn
declare ptr @jw_chr(i64, ptr)
declare ptr @jw_str_append(ptr, ptr)
declare ptr @jw_list_join(ptr, ptr)
declare void @jw_str_bytes(ptr, ptr)
declare i64 @jw_str_code(ptr, ptr)
declare void @jw_args(ptr)

"#;

/// 命令を書きためる入れ物（関数 1 つ分）
#[derive(Default)]
struct B {
    text: String,
    /// 入口のブロックに置く alloca
    allocas: String,
    n: usize,
}

impl B {
    fn t(&mut self) -> String {
        self.n += 1;
        format!("%v{}", self.n)
    }

    fn l(&mut self) -> String {
        self.n += 1;
        format!("L{}", self.n)
    }

    fn op(&mut self, s: impl AsRef<str>) {
        self.text.push_str("  ");
        self.text.push_str(s.as_ref());
        self.text.push('\n');
    }

    /// `%v = rhs`
    fn val(&mut self, rhs: impl AsRef<str>) -> String {
        let t = self.t();
        self.op(format!("{} = {}", t, rhs.as_ref()));
        t
    }

    fn label(&mut self, l: &str) {
        let _ = writeln!(self.text, "{}:", l);
    }

    fn alloca(&mut self, ty: &str) -> String {
        let t = self.t();
        let _ = writeln!(self.allocas, "  {} = alloca {}", t, ty);
        t
    }

    fn br(&mut self, l: &str) {
        self.op(format!("br label %{}", l));
    }

    /// `c` が真なら新しいブロックへ進んで `then` の中身を書き、偽なら続きへ。続きのブロックで終わる
    fn if_true(&mut self, c: &str) -> (String, String) {
        let yes = self.l();
        let no = self.l();
        self.op(format!("br i1 {}, label %{}, label %{}", c, yes, no));
        self.label(&yes);
        (yes, no)
    }

    /// i を 0..n で回すループの始め。(i, 終わりに使うもの)
    fn loop_begin(&mut self, n: &str) -> (String, LoopEnd) {
        let ip = self.alloca("i64");
        self.op(format!("store i64 0, ptr {}", ip));
        let head = self.l();
        let body = self.l();
        let exit = self.l();
        self.br(&head);
        self.label(&head);
        let i = self.val(format!("load i64, ptr {}", ip));
        let c = self.val(format!("icmp slt i64 {}, {}", i, n));
        self.op(format!("br i1 {}, label %{}, label %{}", c, body, exit));
        self.label(&body);
        (i.clone(), LoopEnd { ip, i, head, exit })
    }

    fn loop_end(&mut self, e: LoopEnd) {
        let i2 = self.val(format!("add i64 {}, 1", e.i));
        self.op(format!("store i64 {}, ptr {}", i2, e.ip));
        self.br(&e.head);
        self.label(&e.exit);
    }
}

struct LoopEnd {
    ip: String,
    i: String,
    head: String,
    exit: String,
}

struct Module<'p> {
    prog: &'p Program,
    files: &'p [String],
    types: String,
    globals: String,
    code: String,
    /// 構造体・enum の型（`{:?}` の形）→ LLVM の名前
    named: HashMap<String, String>,
    strs: HashMap<String, String>,
    cstrs: HashMap<String, String>,
    sites: HashMap<(Span, String), String>,
    helpers: HashMap<(H, String), String>,
    todo: Vec<(H, Ty, String)>,
    /// interface として使った構造体の型の番号（type id）
    tids: Vec<Ty>,
    n: usize,
    opts: Opts,
    stats: Stats,
}

impl<'p> Module<'p> {
    fn new(prog: &'p Program, files: &'p [String]) -> Self {
        Module {
            prog,
            files,
            types: String::new(),
            globals: String::new(),
            code: String::new(),
            named: HashMap::new(),
            strs: HashMap::new(),
            cstrs: HashMap::new(),
            sites: HashMap::new(),
            helpers: HashMap::new(),
            todo: Vec::new(),
            tids: Vec::new(),
            n: 0,
            opts: Opts::default(),
            stats: Stats::default(),
        }
    }

    fn output(&self) -> String {
        format!("{}{}\n{}\n{}{}", PRELUDE, self.types, self.globals, self.code, TBAA)
    }

    fn fresh(&mut self) -> usize {
        self.n += 1;
        self.n
    }

    // ------------------------------------------------------------ 型

    fn lt(&mut self, t: &Ty) -> G<String> {
        Ok(match t {
            Ty::Int(i) => format!("i{}", i.bits()),
            Ty::Float => "double".to_string(),
            Ty::Bool => "i1".to_string(),
            Ty::Str | Ty::Map(..) | Ty::Task(_) | Ty::Chan(_) => "ptr".to_string(),
            Ty::Unit | Ty::Never => "i8".to_string(),
            Ty::Own(x) if matches!(**x, Ty::Iface(_)) => "%iown".to_string(),
            Ty::Own(_) => "ptr".to_string(),
            Ty::Ref(x, _) if matches!(**x, Ty::Iface(_)) => "%iref".to_string(),
            Ty::Ref(..) => "%ref".to_string(),
            Ty::Array(x, n) => format!("[{} x {}]", n, self.lt(x)?),
            Ty::List(_) => "%list".to_string(),
            Ty::Struct(..) | Ty::Enum(..) => self.named_type(t)?,
            Ty::Fn(..) => "%fnv".to_string(),
            Ty::Param(_) | Ty::Iface(_) => return Err(format!("型 {}", t.show())),
        })
    }

    fn named_type(&mut self, t: &Ty) -> G<String> {
        let key = format!("{:?}", t);
        if let Some(n) = self.named.get(&key) {
            return Ok(n.clone());
        }
        let name = format!("%T{}", self.fresh());
        self.named.insert(key, name.clone());
        let body = match t {
            Ty::Struct(..) => {
                let fs = self.struct_fields(t)?;
                let mut parts = Vec::new();
                for f in &fs {
                    parts.push(self.lt(f)?);
                }
                format!("{{ {} }}", parts.join(", "))
            }
            _ if self.niche(t)?.is_some() => "{ ptr }".to_string(),
            _ => {
                let mut k = 0;
                for (_, fs) in self.variants(t)? {
                    let (s, _) = self.size_align(&Ty::Struct(String::new(), fs))?;
                    k = k.max(ceil_div(s, 8));
                }
                format!("{{ i32, [{} x i64] }}", k)
            }
        };
        let _ = writeln!(self.types, "{} = type {} ; {}", name, body, t.show());
        Ok(name)
    }

    /// 構造体のフィールドの型（型の引数を入れたもの）
    fn struct_fields(&self, t: &Ty) -> G<Vec<Ty>> {
        let Ty::Struct(n, args) = t else { return Err(format!("構造体でない {}", t.show())) };
        let (tps, fs) = self.prog.struct_tys.get(n).ok_or_else(|| format!("構造体 {} の記録がない", n))?;
        let map: HashMap<String, Ty> = tps.iter().cloned().zip(args.iter().cloned()).collect();
        Ok(fs.iter().map(|f| crate::types::subst(f, &map)).collect())
    }

    fn variants(&self, t: &Ty) -> G<Vec<(String, Vec<Ty>)>> {
        let Ty::Enum(n, args) = t else { return Err(format!("enum でない {}", t.show())) };
        let (tps, vs) = self.prog.enums.get(n).ok_or_else(|| format!("enum {} の記録がない", n))?;
        let map: HashMap<String, Ty> = tps.iter().cloned().zip(args.iter().cloned()).collect();
        Ok(vs.iter().map(|(v, fs)| (v.clone(), fs.iter().map(|f| crate::types::subst(f, &map)).collect())).collect())
    }

    fn variant_index(&self, t: &Ty, v: &str) -> G<usize> {
        self.variants(t)?.iter().position(|(n, _)| n == v).ok_or_else(|| format!("enum の種類 {} がない", v))
    }

    /// 「中身なし」と「own 1 つ」の 2 種類だけの enum（`Option<own T>` など）は、ポインタ 1 つで表す（null が中身なし）。
    /// そのときは（中身なしの種類, own の種類）の番号
    fn niche(&self, t: &Ty) -> G<Option<(usize, usize)>> {
        let Ty::Enum(..) = t else { return Ok(None) };
        let vs = self.variants(t)?;
        if vs.len() != 2 {
            return Ok(None);
        }
        let own = |fs: &[Ty]| matches!(fs, [Ty::Own(x)] if !matches!(**x, Ty::Iface(_)));
        Ok(match (&vs[0].1[..], &vs[1].1[..]) {
            ([], fs) if own(fs) => Some((0, 1)),
            (fs, []) if own(fs) => Some((1, 0)),
            _ => None,
        })
    }

    /// enum の種類の番号（i32）を読む
    fn enum_tag(&mut self, b: &mut B, t: &Ty, p: &str) -> G<String> {
        let et = self.lt(t)?;
        if let Some((none, some)) = self.niche(t)? {
            let v = b.val(format!("load ptr, ptr {}", p));
            let is_none = b.val(format!("icmp eq ptr {}, null", v));
            return Ok(b.val(format!("select i1 {}, i32 {}, i32 {}", is_none, none, some)));
        }
        let tp = b.val(format!("getelementptr {}, ptr {}, i32 0, i32 0", et, p));
        Ok(b.val(format!("load i32, ptr {}", tp)))
    }

    /// enum の中身の場所（種類ごとの構造体 `variant_lt` として読む）
    fn enum_payload(&mut self, b: &mut B, t: &Ty, p: &str) -> G<String> {
        if self.niche(t)?.is_some() {
            return Ok(p.to_string());
        }
        let et = self.lt(t)?;
        Ok(b.val(format!("getelementptr {}, ptr {}, i32 0, i32 1", et, p)))
    }

    /// 大きさと並びの単位（LLVM の既定の並べ方と同じ）。構造体の名前が空なら、中身を並べただけのもの（enum の中身）
    fn size_align(&mut self, t: &Ty) -> G<(u64, u64)> {
        Ok(match t {
            Ty::Int(i) => (i.bits() as u64 / 8, i.bits() as u64 / 8),
            Ty::Float | Ty::Str | Ty::Map(..) | Ty::Task(_) | Ty::Chan(_) => (8, 8),
            Ty::Bool | Ty::Unit | Ty::Never => (1, 1),
            Ty::Own(x) if matches!(**x, Ty::Iface(_)) => (16, 8),
            Ty::Own(_) => (8, 8),
            Ty::Ref(x, _) if matches!(**x, Ty::Iface(_)) => (40, 8),
            Ty::Ref(..) => (32, 8),
            Ty::List(_) => (24, 8),
            Ty::Fn(..) => (16, 8),
            Ty::Array(x, n) => {
                let (s, a) = self.size_align(x)?;
                (ceil_div(s, a).checked_mul(a).and_then(|v| v.checked_mul(*n))
                    .ok_or_else(|| format!("配列型 `{}` のサイズがオーバーフローしました", t.show()))?, a)
            }
            Ty::Struct(name, fs) if name.is_empty() => self.layout(fs)?,
            Ty::Struct(..) => {
                let fs = self.struct_fields(t)?;
                self.layout(&fs)?
            }
            Ty::Enum(..) if self.niche(t)?.is_some() => (8, 8),
            Ty::Enum(..) => {
                let mut k = 0;
                for (_, fs) in self.variants(t)? {
                    let (s, _) = self.layout(&fs)?;
                    k = k.max(ceil_div(s, 8));
                }
                (k.checked_mul(8).and_then(|v| v.checked_add(8))
                    .ok_or_else(|| format!("enum `{}` のサイズがオーバーフローしました", t.show()))?, 8)
            }
            _ => return Err(format!("型 {} の大きさ", t.show())),
        })
    }

    fn layout(&mut self, fs: &[Ty]) -> G<(u64, u64)> {
        let (mut off, mut al) = (0u64, 1u64);
        for f in fs {
            let (s, a) = self.size_align(f)?;
            off = ceil_div(off, a).checked_mul(a)
                .and_then(|v| v.checked_add(s))
                .ok_or_else(|| "構造体のサイズがオーバーフローしました".to_string())?;
            al = al.max(a);
        }
        Ok((ceil_div(off, al).checked_mul(al)
            .ok_or_else(|| "構造体のアラインメント計算がオーバーフローしました".to_string())?, al))
    }

    /// 型の大きさ（LLVM の定数式）
    fn sizeof(&mut self, t: &Ty) -> G<String> {
        let l = self.lt(t)?;
        Ok(format!("ptrtoint (ptr getelementptr ({}, ptr null, i32 1) to i64)", l))
    }

    /// 中に解放するもの（own・文字列・リスト・辞書）があるか
    fn needs_drop(&mut self, t: &Ty) -> G<bool> {
        Ok(match t {
            Ty::Str | Ty::Own(_) | Ty::List(_) | Ty::Map(..) | Ty::Fn(..) | Ty::Task(_) | Ty::Chan(_) => true,
            Ty::Array(x, _) => self.needs_drop(x)?,
            Ty::Struct(..) => {
                for f in self.struct_fields(t)? {
                    if self.needs_drop(&f)? {
                        return Ok(true);
                    }
                }
                false
            }
            Ty::Enum(..) => {
                for (_, fs) in self.variants(t)? {
                    for f in fs {
                        if self.needs_drop(&f)? {
                            return Ok(true);
                        }
                    }
                }
                false
            }
            _ => false,
        })
    }

    /// 写すだけで複製になるか（文字列・own の写し・リスト・辞書を含まない）
    fn trivial(&mut self, t: &Ty) -> G<bool> {
        self.needs_drop(t).map(|d| !d)
    }

    // ------------------------------------------------------------ 定数

    fn cstr(&mut self, s: &str) -> String {
        if let Some(n) = self.cstrs.get(s) {
            return n.clone();
        }
        let name = format!("@cs{}", self.fresh());
        let _ = writeln!(self.globals, "{} = private unnamed_addr constant [{} x i8] c\"{}\\00\"", name, s.len() + 1, esc(s));
        self.cstrs.insert(s.to_string(), name.clone());
        name
    }

    /// 文字列の定数（数を数えない。rc = -1）
    fn strconst(&mut self, s: &str) -> String {
        if let Some(n) = self.strs.get(s) {
            return n.clone();
        }
        let name = format!("@str{}", self.fresh());
        let _ = writeln!(
            self.globals,
            "{} = private unnamed_addr constant {{ i64, i64, [{} x i8] }} {{ i64 -1, i64 {}, [{} x i8] c\"{}\\00\" }}",
            name,
            s.len() + 1,
            s.len(),
            s.len() + 1,
            esc(s)
        );
        self.strs.insert(s.to_string(), name.clone());
        name
    }

    /// 位置と説明の定数（`WSite`）。解放の位置なら、連鎖して解放されたときの位置も一緒に作る
    fn site(&mut self, span: Span, text: &str) -> String {
        if let Some(n) = self.sites.get(&(span, text.to_string())) {
            return n.clone();
        }
        let name = format!("@site{}", self.fresh());
        self.sites.insert((span, text.to_string()), name.clone());
        let chain = if text == CHAIN { name.clone() } else { self.site(span, CHAIN) };
        let file = self.files.get(span.file).cloned().unwrap_or_else(|| "?".to_string());
        let file = self.cstr(&file);
        let t = self.cstr(text);
        let _ = writeln!(
            self.globals,
            "{} = private unnamed_addr constant %site {{ ptr {}, i32 {}, i32 {}, i32 {}, i32 {}, ptr {}, ptr {} }}",
            name, file, span.line, span.col, span.len, span.file, t, chain
        );
        name
    }

    /// 型の手伝いの関数の名前（まだなければ、作る予定に入れる）
    fn helper(&mut self, h: H, t: &Ty) -> String {
        let key = (h, format!("{:?}", t));
        if let Some(n) = self.helpers.get(&key) {
            return n.clone();
        }
        let name = format!("@{}{}", format!("{:?}", h).to_lowercase(), self.fresh());
        self.helpers.insert(key, name.clone());
        self.todo.push((h, t.clone(), name.clone()));
        name
    }

    /// interface として使う構造体の型の番号
    fn tid(&mut self, t: &Ty) -> usize {
        match self.tids.iter().position(|x| x == t) {
            Some(i) => i,
            None => {
                self.tids.push(t.clone());
                self.tids.len() - 1
            }
        }
    }

    // ------------------------------------------------------------ 型の手伝いの関数

    fn finish_helpers(&mut self) -> G<()> {
        // interface の解放・表示などは、使った構造体がすべて分かってから作る
        let mut deferred = Vec::new();
        loop {
            while let Some((h, t, name)) = self.todo.pop() {
                if is_iface_ptr(&t) {
                    deferred.push((h, t, name));
                    continue;
                }
                self.gen_helper(h, &t, &name)?;
            }
            if deferred.is_empty() {
                break;
            }
            for (h, t, name) in std::mem::take(&mut deferred) {
                self.gen_helper(h, &t, &name)?;
            }
            if self.todo.is_empty() {
                break;
            }
        }
        Ok(())
    }

    fn gen_helper(&mut self, h: H, t: &Ty, name: &str) -> G<()> {
        let mut b = B::default();
        let (sig, ret) = match h {
            H::Drop => ("ptr %p, ptr %site", "void"),
            H::Clone => ("ptr %p, ptr %dst", "void"),
            H::Show => ("ptr %p, ptr %sb, i64 %d, ptr %use", "void"),
            H::View => ("ptr %p, ptr %dst, ptr %site", "void"),
            H::Hazard => ("ptr %p", "i1"),
            H::Less => ("ptr %p, ptr %q, ptr %env", "i32"),
            H::Moves | H::HasOwn => ("ptr %p", "i1"),
            H::Send => ("ptr %p, ptr %site", "void"),
        };
        let result = match h {
            H::Drop => {
                self.gen_drop(&mut b, t)?;
                "ret void".to_string()
            }
            H::Clone => {
                self.gen_copy(&mut b, t, None)?;
                "ret void".to_string()
            }
            H::View => {
                self.gen_copy(&mut b, t, Some("%site"))?;
                "ret void".to_string()
            }
            H::Show => {
                self.gen_show(&mut b, t)?;
                "ret void".to_string()
            }
            H::Hazard => {
                let r = self.gen_hazard(&mut b, t)?;
                format!("ret i1 {}", r)
            }
            H::Moves | H::HasOwn => {
                let r = self.gen_moves(&mut b, t, h == H::Moves)?;
                format!("ret i1 {}", r)
            }
            H::Send => {
                self.gen_send(&mut b, t)?;
                "ret void".to_string()
            }
            H::Less => {
                let r = self.gen_less(&mut b, t)?;
                let r = b.val(format!("zext i1 {} to i32", r));
                format!("ret i32 {}", r)
            }
        };
        let _ = write!(
            self.code,
            "; {:?} {}\ndefine internal {} {}({}) {{\nentry:\n{}{}  {}\n}}\n\n",
            h,
            t.show(),
            ret,
            name,
            sig,
            b.allocas,
            b.text,
            result
        );
        Ok(())
    }

    /// `%p` の値の中の own をすべて解放する
    fn gen_drop(&mut self, b: &mut B, t: &Ty) -> G<()> {
        match t {
            Ty::Chan(_) => {
                let v = b.val("load ptr, ptr %p");
                b.op(format!("call void @jw_chan_release(ptr {}, ptr %site)", v));
            }
            Ty::Task(_) => {
                let v = b.val("load ptr, ptr %p");
                b.op(format!("call void @jw_task_drop(ptr {}, ptr %site)", v));
            }
            Ty::Str => {
                let v = b.val("load ptr, ptr %p");
                b.op(format!("call void @jw_str_release(ptr {})", v));
            }
            Ty::Own(x) => {
                let (p, tid) = if is_iface_ptr(t) {
                    let v = b.val("load %iown, ptr %p");
                    (b.val(format!("extractvalue %iown {}, 0", v)), Some(b.val(format!("extractvalue %iown {}, 1", v))))
                } else {
                    (b.val("load ptr, ptr %p"), None)
                };
                let bits = b.val(format!("ptrtoint ptr {} to i64", p));
                let tag = b.val(format!("and i64 {}, 1", bits));
                let isv = b.val(format!("icmp ne i64 {}, 0", tag));
                let (_, no) = b.if_true(&isv);
                let done = b.l();
                b.op(format!("call void @jw_view_free(ptr {})", p));
                b.br(&done);
                b.label(&no);
                let chain = b.val("getelementptr %site, ptr %site, i32 0, i32 6");
                let chain = b.val(format!("load ptr, ptr {}", chain));
                let plain = tid.is_none();
                match tid {
                    None => {
                        if self.needs_drop(x)? {
                            let d = self.helper(H::Drop, x);
                            b.op(format!("call void {}(ptr {}, ptr {})", d, p, chain));
                        }
                        let size = self.sizeof(x)?;
                        b.op(format!("call void @jw_free(ptr {}, i64 {}, ptr %site)", p, size));
                    }
                    Some(tid) => {
                        // 構造体ごとに分かれる
                        let structs = self.tids.clone();
                        let mut cases = Vec::new();
                        let mut bodies = String::new();
                        for (i, s) in structs.iter().enumerate() {
                            let l = b.l();
                            cases.push(format!("i64 {}, label %{}", i, l));
                            let mut sub = B { n: b.n, ..B::default() };
                            sub.label(&l);
                            if self.needs_drop(s)? {
                                let d = self.helper(H::Drop, s);
                                sub.op(format!("call void {}(ptr {}, ptr {})", d, p, chain));
                            }
                            let size = self.sizeof(s)?;
                            sub.op(format!("call void @jw_free(ptr {}, i64 {}, ptr %site)", p, size));
                            sub.br(&done);
                            b.n = sub.n;
                            bodies.push_str(&sub.text);
                        }
                        let bad = b.l();
                        b.op(format!("switch i64 {}, label %{} [ {} ]", tid, bad, cases.join(" ")));
                        b.text.push_str(&bodies);
                        b.label(&bad);
                        let msg = self.cstr("interface の値の構造体が分からない");
                        b.op(format!("call void @jw_bug(ptr {})", msg));
                        b.op("unreachable");
                    }
                }
                if plain {
                    b.br(&done);
                }
                b.label(&done);
            }
            Ty::Struct(..) => {
                let st = self.lt(t)?;
                for (i, f) in self.struct_fields(t)?.iter().enumerate() {
                    if self.needs_drop(f)? {
                        let fp = b.val(format!("getelementptr {}, ptr %p, i32 0, i32 {}", st, i));
                        let d = self.helper(H::Drop, f);
                        b.op(format!("call void {}(ptr {}, ptr %site)", d, fp));
                    }
                }
            }
            Ty::Enum(..) => {
                let vs = self.variants(t)?;
                let tag = self.enum_tag(b, t, "%p")?;
                let pl = self.enum_payload(b, t, "%p")?;
                let done = b.l();
                let mut cases = Vec::new();
                let mut bodies = String::new();
                for (vi, (_, fs)) in vs.iter().enumerate() {
                    let mut any = false;
                    for f in fs {
                        any |= self.needs_drop(f)?;
                    }
                    if !any {
                        continue;
                    }
                    let mut sub = B { n: b.n, ..B::default() };
                    let l = sub.l();
                    cases.push(format!("i32 {}, label %{}", vi, l));
                    sub.label(&l);
                    let vt = self.variant_lt(fs)?;
                    for (i, f) in fs.iter().enumerate() {
                        if self.needs_drop(f)? {
                            let fp = sub.val(format!("getelementptr {}, ptr {}, i32 0, i32 {}", vt, pl, i));
                            let d = self.helper(H::Drop, f);
                            sub.op(format!("call void {}(ptr {}, ptr %site)", d, fp));
                        }
                    }
                    sub.br(&done);
                    b.n = sub.n;
                    bodies.push_str(&sub.text);
                }
                b.op(format!("switch i32 {}, label %{} [ {} ]", tag, done, cases.join(" ")));
                b.text.push_str(&bodies);
                b.label(&done);
            }
            Ty::Array(x, n) => {
                if self.needs_drop(x)? {
                    let at = self.lt(t)?;
                    let d = self.helper(H::Drop, x);
                    let (i, e) = b.loop_begin(&n.to_string());
                    let ep = b.val(format!("getelementptr {}, ptr %p, i64 0, i64 {}", at, i));
                    b.op(format!("call void {}(ptr {}, ptr %site)", d, ep));
                    b.loop_end(e);
                }
            }
            Ty::List(x) => {
                if self.needs_drop(x)? {
                    let xt = self.lt(x)?;
                    let d = self.helper(H::Drop, x);
                    let data = b.val("load ptr, ptr %p");
                    let lp = b.val("getelementptr %list, ptr %p, i32 0, i32 1");
                    let len = b.val(format!("load i64, ptr {}", lp));
                    let (i, e) = b.loop_begin(&len);
                    let ep = b.val(format!("getelementptr {}, ptr {}, i64 {}", xt, data, i));
                    b.op(format!("call void {}(ptr {}, ptr %site)", d, ep));
                    b.loop_end(e);
                }
                let size = self.sizeof(x)?;
                b.op(format!("call void @jw_list_free(ptr %p, i64 {}, ptr %site)", size));
            }
            Ty::Fn(..) => {
                let v = b.val("load %fnv, ptr %p");
                let env = b.val(format!("extractvalue %fnv {}, 1", v));
                let has = b.val(format!("icmp ne ptr {}, null", env));
                let (_, no) = b.if_true(&has);
                b.op(format!("call void @jw_env_release(ptr {}, ptr %site)", env));
                b.br(&no);
                b.label(&no);
            }
            Ty::Map(k, v) => {
                let m = b.val("load ptr, ptr %p");
                let len = b.val(format!("call i64 @jw_map_len(ptr {})", m));
                let (i, e) = b.loop_begin(&len);
                if matches!(**k, Ty::Str) {
                    let kv = b.val(format!("call i64 @jw_map_key(ptr {}, i64 {})", m, i));
                    let kp = b.val(format!("inttoptr i64 {} to ptr", kv));
                    b.op(format!("call void @jw_str_release(ptr {})", kp));
                }
                if self.needs_drop(v)? {
                    let d = self.helper(H::Drop, v);
                    let vp = b.val(format!("call ptr @jw_map_val(ptr {}, i64 {})", m, i));
                    b.op(format!("call void {}(ptr {}, ptr %site)", d, vp));
                }
                b.loop_end(e);
                b.op(format!("call void @jw_map_free(ptr {})", m));
            }
            _ => {}
        }
        Ok(())
    }

    /// `%p` の値を、ほかのスレッドへ送れるようにする（その場で書き換える）。
    /// own の区画は世代を進め（送った側に残った ref を無効にする。interp の `export` の解放と同じ）、文字列は複製する
    fn gen_send(&mut self, b: &mut B, t: &Ty) -> G<()> {
        let sub = |me: &mut Self, b: &mut B, x: &Ty, p: &str| -> G<()> {
            if me.needs_drop(x)? {
                let f = me.helper(H::Send, x);
                b.op(format!("call void {}(ptr {}, ptr %site)", f, p));
            }
            Ok(())
        };
        match t {
            Ty::Str => {
                let v = b.val("load ptr, ptr %p");
                let n = b.val(format!("call ptr @jw_str_unshare(ptr {})", v));
                b.op(format!("store ptr {}, ptr %p", n));
            }
            Ty::Own(x) => {
                let p = b.val("load ptr, ptr %p");
                b.op(format!("call void @jw_send_block(ptr {}, ptr %site)", p));
                sub(self, b, x, &p)?;
            }
            Ty::Struct(..) => {
                let st = self.lt(t)?;
                for (i, f) in self.struct_fields(t)?.iter().enumerate() {
                    let fp = b.val(format!("getelementptr {}, ptr %p, i32 0, i32 {}", st, i));
                    sub(self, b, f, &fp)?;
                }
            }
            Ty::Enum(..) => {
                let vs = self.variants(t)?;
                let tag = self.enum_tag(b, t, "%p")?;
                let pl = self.enum_payload(b, t, "%p")?;
                let done = b.l();
                let mut cases = Vec::new();
                let mut bodies = String::new();
                for (vi, (_, fs)) in vs.iter().enumerate() {
                    let mut s2 = B { n: b.n, ..B::default() };
                    let l = s2.l();
                    cases.push(format!("i32 {}, label %{}", vi, l));
                    s2.label(&l);
                    let vt = self.variant_lt(fs)?;
                    for (i, f) in fs.iter().enumerate() {
                        let fp = s2.val(format!("getelementptr {}, ptr {}, i32 0, i32 {}", vt, pl, i));
                        sub(self, &mut s2, f, &fp)?;
                    }
                    s2.br(&done);
                    b.n = s2.n;
                    bodies.push_str(&s2.text);
                }
                b.op(format!("switch i32 {}, label %{} [ {} ]", tag, done, cases.join(" ")));
                b.text.push_str(&bodies);
                b.label(&done);
            }
            Ty::Array(x, n) => {
                if self.needs_drop(x)? {
                    let at = self.lt(t)?;
                    let (i, e) = b.loop_begin(&n.to_string());
                    let ep = b.val(format!("getelementptr {}, ptr %p, i64 0, i64 {}", at, i));
                    sub(self, b, x, &ep)?;
                    b.loop_end(e);
                }
            }
            Ty::List(x) => {
                // 中身の区画の世代を進める（送った側に残った、要素を借りた ref を無効にする。interp の `export`）
                b.op("call void @jw_list_moved(ptr %p, ptr %site)");
                if self.needs_drop(x)? {
                    let xt = self.lt(x)?;
                    let data = b.val("load ptr, ptr %p");
                    let lp = b.val("getelementptr %list, ptr %p, i32 0, i32 1");
                    let len = b.val(format!("load i64, ptr {}", lp));
                    let (i, e) = b.loop_begin(&len);
                    let ep = b.val(format!("getelementptr {}, ptr {}, i64 {}", xt, data, i));
                    sub(self, b, x, &ep)?;
                    b.loop_end(e);
                }
            }
            Ty::Map(k, v) => {
                let m = b.val("load ptr, ptr %p");
                if matches!(**k, Ty::Str) {
                    b.op(format!("call void @jw_map_unshare_keys(ptr {})", m));
                }
                if self.needs_drop(v)? {
                    let len = b.val(format!("call i64 @jw_map_len(ptr {})", m));
                    let (i, e) = b.loop_begin(&len);
                    let vp = b.val(format!("call ptr @jw_map_val(ptr {}, i64 {})", m, i));
                    sub(self, b, v, &vp)?;
                    b.loop_end(e);
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// spawn の入口: `void (ptr 取り込んだ値, ptr 戻り値の置き場)`。新しいスレッドで関数 fi を 1 回呼ぶ
    fn spawn_thunk(&mut self, fi: usize, ctys: &[Ty]) -> G<String> {
        let name = format!("@spawn{}", fi);
        let key = (H::Send, format!("spawn {}", fi));
        if self.helpers.contains_key(&key) {
            return Ok(name);
        }
        self.helpers.insert(key, name.clone());
        let args_t = self.variant_lt(ctys)?;
        let mut b = B::default();
        let mut args = vec!["i64 1".to_string()];
        for (i, ct) in ctys.iter().enumerate() {
            let lt = self.lt(ct)?;
            let p = b.val(format!("getelementptr {}, ptr %args, i32 0, i32 {}", args_t, i));
            let v = b.val(format!("load {}, ptr {}", lt, p));
            args.push(format!("{} {}", lt, v));
        }
        let ret_ty = self.prog.fns[fi].ret.clone();
        let ret = self.lt(&ret_ty)?;
        let r = b.val(format!("call {} @f{}({})", ret, fi, args.join(", ")));
        b.op(format!("store {} {}, ptr %out", ret, r));
        let _ = write!(
            self.code,
            "define internal void {}(ptr %args, ptr %out) {{\nentry:\n{}{}  ret void\n}}\n\n",
            name, b.allocas, b.text
        );
        Ok(name)
    }

    fn variant_lt(&mut self, fs: &[Ty]) -> G<String> {
        let mut parts = Vec::new();
        for f in fs {
            parts.push(self.lt(f)?);
        }
        Ok(format!("{{ {} }}", parts.join(", ")))
    }

    /// `%p` を `%dst` へ複製する。`view` があれば読むだけの写し（own → 写しの札。借りた位置が view）
    fn gen_copy(&mut self, b: &mut B, t: &Ty, view: Option<&str>) -> G<()> {
        let h = if view.is_some() { H::View } else { H::Clone };
        let call = |me: &mut Self, b: &mut B, x: &Ty, src: &str, dst: &str| {
            let f = me.helper(h, x);
            match view {
                Some(s) => b.op(format!("call void {}(ptr {}, ptr {}, ptr {})", f, src, dst, s)),
                None => b.op(format!("call void {}(ptr {}, ptr {})", f, src, dst)),
            }
        };
        if self.trivial(t)? {
            let l = self.lt(t)?;
            let v = b.val(format!("load {}, ptr %p", l));
            b.op(format!("store {} {}, ptr %dst", l, v));
            return Ok(());
        }
        match t {
            Ty::Str => {
                let v = b.val("load ptr, ptr %p");
                b.op(format!("call void @jw_str_retain(ptr {})", v));
                b.op(format!("store ptr {}, ptr %dst", v));
            }
            Ty::Chan(_) => {
                let v = b.val("load ptr, ptr %p");
                b.op(format!("call void @jw_chan_retain(ptr {})", v));
                b.op(format!("store ptr {}, ptr %dst", v));
            }
            // task の写しは、解放しない印（下の 1 ビット）をつけたポインタ
            Ty::Task(_) => {
                let v = b.val("load ptr, ptr %p");
                let w = b.val(format!("call ptr @jw_task_view(ptr {})", v));
                b.op(format!("store ptr {}, ptr %dst", w));
            }
            Ty::Own(_) => {
                let iface = is_iface_ptr(t);
                let p = if iface {
                    let v = b.val("load %iown, ptr %p");
                    b.val(format!("extractvalue %iown {}, 0", v))
                } else {
                    b.val("load ptr, ptr %p")
                };
                if view.is_none() {
                    // own の複製は、写しのときだけ（本物の own は唯一の持ち主なので複製しない）
                    let bits = b.val(format!("ptrtoint ptr {} to i64", p));
                    let tag = b.val(format!("and i64 {}, 1", bits));
                    let real = b.val(format!("icmp eq i64 {}, 0", tag));
                    let (_, no) = b.if_true(&real);
                    let msg = self.cstr("own を複製しようとした");
                    b.op(format!("call void @jw_bug(ptr {})", msg));
                    b.op("unreachable");
                    b.label(&no);
                }
                let site = view.unwrap_or("null");
                let c = b.val(format!("call ptr @jw_view(ptr {}, ptr {})", p, site));
                if iface {
                    let v = b.val("load %iown, ptr %p");
                    let v2 = b.val(format!("insertvalue %iown {}, ptr {}, 0", v, c));
                    b.op(format!("store %iown {}, ptr %dst", v2));
                } else {
                    b.op(format!("store ptr {}, ptr %dst", c));
                }
            }
            Ty::Struct(..) => {
                let st = self.lt(t)?;
                for (i, f) in self.struct_fields(t)?.iter().enumerate() {
                    let s = b.val(format!("getelementptr {}, ptr %p, i32 0, i32 {}", st, i));
                    let d = b.val(format!("getelementptr {}, ptr %dst, i32 0, i32 {}", st, i));
                    call(self, b, f, &s, &d);
                }
            }
            Ty::Array(x, n) => {
                let at = self.lt(t)?;
                let (i, e) = b.loop_begin(&n.to_string());
                let s = b.val(format!("getelementptr {}, ptr %p, i64 0, i64 {}", at, i));
                let d = b.val(format!("getelementptr {}, ptr %dst, i64 0, i64 {}", at, i));
                call(self, b, x, &s, &d);
                b.loop_end(e);
            }
            Ty::Enum(..) => {
                let vs = self.variants(t)?;
                let et = self.lt(t)?;
                // 種類の番号と中身をそのまま写してから、複製が要る中身だけ上書きする
                let v = b.val(format!("load {}, ptr %p", et));
                b.op(format!("store {} {}, ptr %dst", et, v));
                let tag = self.enum_tag(b, t, "%p")?;
                let spl = self.enum_payload(b, t, "%p")?;
                let dpl = self.enum_payload(b, t, "%dst")?;
                let done = b.l();
                let mut cases = Vec::new();
                let mut bodies = String::new();
                for (vi, (_, fs)) in vs.iter().enumerate() {
                    let mut any = false;
                    for f in fs {
                        any |= !self.trivial(f)?;
                    }
                    if !any {
                        continue;
                    }
                    let mut sub = B { n: b.n, ..B::default() };
                    let l = sub.l();
                    cases.push(format!("i32 {}, label %{}", vi, l));
                    sub.label(&l);
                    let vt = self.variant_lt(fs)?;
                    for (i, f) in fs.iter().enumerate() {
                        if !self.trivial(f)? {
                            let s = sub.val(format!("getelementptr {}, ptr {}, i32 0, i32 {}", vt, spl, i));
                            let d = sub.val(format!("getelementptr {}, ptr {}, i32 0, i32 {}", vt, dpl, i));
                            call(self, &mut sub, f, &s, &d);
                        }
                    }
                    sub.br(&done);
                    b.n = sub.n;
                    bodies.push_str(&sub.text);
                }
                b.op(format!("switch i32 {}, label %{} [ {} ]", tag, done, cases.join(" ")));
                b.text.push_str(&bodies);
                b.label(&done);
            }
            Ty::List(x) => {
                let xt = self.lt(x)?;
                let size = self.sizeof(x)?;
                let data = b.val("load ptr, ptr %p");
                let lp = b.val("getelementptr %list, ptr %p, i32 0, i32 1");
                let len = b.val(format!("load i64, ptr {}", lp));
                b.op("store %list zeroinitializer, ptr %dst");
                b.op(format!("call void @jw_list_reserve(ptr %dst, i64 {}, i64 {})", size, len));
                let nd = b.val("load ptr, ptr %dst");
                let (i, e) = b.loop_begin(&len);
                let s = b.val(format!("getelementptr {}, ptr {}, i64 {}", xt, data, i));
                let d = b.val(format!("getelementptr {}, ptr {}, i64 {}", xt, nd, i));
                call(self, b, x, &s, &d);
                b.loop_end(e);
                let dl = b.val("getelementptr %list, ptr %dst, i32 0, i32 1");
                b.op(format!("store i64 {}, ptr {}", len, dl));
            }
            Ty::Map(k, v) => {
                let m = b.val("load ptr, ptr %p");
                let vsize = self.sizeof(v)?;
                let nm = b.val(format!("call ptr @jw_map_new(i64 {}, i32 {})", vsize, key_kind(k)));
                b.op(format!("store ptr {}, ptr %dst", nm));
                let len = b.val(format!("call i64 @jw_map_len(ptr {})", m));
                let (i, e) = b.loop_begin(&len);
                let kv = b.val(format!("call i64 @jw_map_key(ptr {}, i64 {})", m, i));
                if matches!(**k, Ty::Str) {
                    let kp = b.val(format!("inttoptr i64 {} to ptr", kv));
                    b.op(format!("call void @jw_str_retain(ptr {})", kp));
                }
                let j = b.val(format!("call i64 @jw_map_add(ptr {}, i64 {})", nm, kv));
                let s = b.val(format!("call ptr @jw_map_val(ptr {}, i64 {})", m, i));
                let d = b.val(format!("call ptr @jw_map_val(ptr {}, i64 {})", nm, j));
                call(self, b, v, &s, &d);
                b.loop_end(e);
            }
            Ty::Fn(..) => {
                // 複製は取り込んだ値の入れ物を共有する（数を増やす）。写しは中身を写した新しい入れ物
                let v = b.val("load %fnv, ptr %p");
                let env = b.val(format!("extractvalue %fnv {}, 1", v));
                let has = b.val(format!("icmp ne ptr {}, null", env));
                let out = b.alloca("%fnv");
                b.op(format!("store %fnv {}, ptr {}", v, out));
                let (_, no) = b.if_true(&has);
                match view {
                    None => b.op(format!("call void @jw_env_retain(ptr {})", env)),
                    Some(site) => {
                        let vt = b.val(format!("getelementptr i8, ptr {}, i64 8", env));
                        let vt = b.val(format!("load ptr, ptr {}", vt));
                        let vf = b.val(format!("getelementptr {{ ptr, ptr }}, ptr {}, i32 0, i32 1", vt));
                        let vf = b.val(format!("load ptr, ptr {}", vf));
                        let ne = b.val(format!("call ptr {}(ptr {}, ptr {})", vf, env, site));
                        let v2 = b.val(format!("insertvalue %fnv {}, ptr {}, 1", v, ne));
                        b.op(format!("store %fnv {}, ptr {}", v2, out));
                    }
                }
                b.br(&no);
                b.label(&no);
                let r = b.val(format!("load %fnv, ptr {}", out));
                b.op(format!("store %fnv {}, ptr %dst", r));
            }
            _ => return Err(format!("{} の複製", t.show())),
        }
        Ok(())
    }

    fn gen_hazard(&mut self, b: &mut B, t: &Ty) -> G<String> {
        Ok(match t {
            // 読むだけの写し（下の 1 ビット）でも取り出せない（写しが own の型で外へ出ないように。interp の `takes_own`）
            Ty::Own(_) => "true".to_string(),
            Ty::Struct(..) | Ty::List(_) | Ty::Map(..) | Ty::Task(_) => "true".to_string(),
            Ty::Fn(..) => env_flag(b, "%p", 3),
            Ty::Enum(..) => {
                if self.trivial(t)? || !self.enum_moves(t)? {
                    "false".to_string()
                } else {
                    "true".to_string()
                }
            }
            Ty::Array(x, n) => {
                if *n == 0 || !self.can_hazard(x)? {
                    "false".to_string()
                } else {
                    let at = self.lt(t)?;
                    let f = self.helper(H::Hazard, x);
                    let acc = b.alloca("i1");
                    b.op(format!("store i1 false, ptr {}", acc));
                    let (i, e) = b.loop_begin(&n.to_string());
                    let ep = b.val(format!("getelementptr {}, ptr %p, i64 0, i64 {}", at, i));
                    let h = b.val(format!("call i1 {}(ptr {})", f, ep));
                    let a = b.val(format!("load i1, ptr {}", acc));
                    let o = b.val(format!("or i1 {}, {}", a, h));
                    b.op(format!("store i1 {}, ptr {}", o, acc));
                    b.loop_end(e);
                    b.val(format!("load i1, ptr {}", acc))
                }
            }
            _ => "false".to_string(),
        })
    }

    /// 値として取り出すとエラーになりうる型か
    fn can_hazard(&mut self, t: &Ty) -> G<bool> {
        Ok(match t {
            Ty::Own(_) | Ty::Struct(..) | Ty::List(_) | Ty::Map(..) | Ty::Fn(..) | Ty::Task(_) => true,
            Ty::Enum(..) => self.enum_moves(t)?,
            Ty::Array(x, n) => *n > 0 && self.can_hazard(x)?,
            _ => false,
        })
    }

    /// 値として読むとムーブになる enum か（lower の `moves` と同じ）
    fn enum_moves(&mut self, t: &Ty) -> G<bool> {
        for (_, fs) in self.variants(t)? {
            for f in fs {
                if moves(self, &f)? {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn gen_show(&mut self, b: &mut B, t: &Ty) -> G<()> {
        // ref は輪になりうるので、深くなりすぎたら打ち切る（interp の `show`）
        let deep = b.val("icmp sgt i64 %d, 32");
        let (_, no) = b.if_true(&deep);
        let dots = self.cstr("...");
        b.op(format!("call void @jw_sb_cstr(ptr %sb, ptr {})", dots));
        b.op("ret void");
        b.label(&no);
        let d1 = b.val("add i64 %d, 1");
        let show = |me: &mut Self, b: &mut B, x: &Ty, p: &str| {
            let f = me.helper(H::Show, x);
            b.op(format!("call void {}(ptr {}, ptr %sb, i64 {}, ptr %use)", f, p, d1));
        };
        let lit = |me: &mut Self, b: &mut B, s: &str| {
            let c = me.cstr(s);
            b.op(format!("call void @jw_sb_cstr(ptr %sb, ptr {})", c));
        };
        match t {
            Ty::Unit | Ty::Never => lit(self, b, "()"),
            Ty::Fn(..) => lit(self, b, "<関数>"),
            Ty::Task(_) => lit(self, b, "<task>"),
            Ty::Chan(_) => lit(self, b, "<chan>"),
            Ty::Int(i) => {
                let v = b.val(format!("load i{}, ptr %p", i.bits()));
                let w = widen(b, &v, *i);
                b.op(format!("call void @jw_sb_int(ptr %sb, i64 {}, i32 {})", w, if i.signed() { 0 } else { 1 }));
            }
            Ty::Float => {
                let v = b.val("load double, ptr %p");
                b.op(format!("call void @jw_sb_float(ptr %sb, double {})", v));
            }
            Ty::Bool => {
                let v = b.val("load i1, ptr %p");
                let tr = self.cstr("true");
                let fa = self.cstr("false");
                let s = b.val(format!("select i1 {}, ptr {}, ptr {}", v, tr, fa));
                b.op(format!("call void @jw_sb_cstr(ptr %sb, ptr {})", s));
            }
            Ty::Str => {
                let v = b.val("load ptr, ptr %p");
                b.op(format!("call void @jw_sb_str(ptr %sb, ptr {})", v));
            }
            Ty::Own(x) => {
                let iface = is_iface_ptr(t);
                let (p, tid) = if iface {
                    let v = b.val("load %iown, ptr %p");
                    (b.val(format!("extractvalue %iown {}, 0", v)), Some(b.val(format!("extractvalue %iown {}, 1", v))))
                } else {
                    (b.val("load ptr, ptr %p"), None)
                };
                let o = self.opts;
                self.stats.gen += 1;
                let target = own_target(b, &p, o.gen, "%use", &o);
                match tid {
                    None => {
                        let f = self.helper(H::Show, x);
                        b.op(format!("call void {}(ptr {}, ptr %sb, i64 {}, ptr %use)", f, target, d1));
                    }
                    Some(tid) => self.dispatch_show(b, &tid, &target, &d1)?,
                }
            }
            Ty::Ref(x, _) => {
                let iface = is_iface_ptr(t);
                let rt = if iface { "%iref" } else { "%ref" };
                let r = b.val(format!("load {}, ptr %p", rt));
                let o = self.opts;
                self.stats.gen += 1;
                let addr = check_ref(b, rt, &r, "%use", &o);
                if iface {
                    let tid = b.val(format!("extractvalue %iref {}, 4", r));
                    self.dispatch_show(b, &tid, &addr, &d1)?;
                } else {
                    let f = self.helper(H::Show, x);
                    b.op(format!("call void {}(ptr {}, ptr %sb, i64 {}, ptr %use)", f, addr, d1));
                }
            }
            Ty::Struct(name, _) => {
                let def = self.prog.structs.get(name).ok_or("構造体の定義がない")?.clone();
                let st = self.lt(t)?;
                lit(self, b, &format!("{} {{ ", def.name));
                for (i, f) in self.struct_fields(t)?.iter().enumerate() {
                    let sep = if i > 0 { ", " } else { "" };
                    lit(self, b, &format!("{}{}: ", sep, def.fields[i]));
                    let fp = b.val(format!("getelementptr {}, ptr %p, i32 0, i32 {}", st, i));
                    show(self, b, f, &fp);
                }
                lit(self, b, " }");
            }
            Ty::Enum(..) => {
                let vs = self.variants(t)?;
                let tag = self.enum_tag(b, t, "%p")?;
                let pl = self.enum_payload(b, t, "%p")?;
                let done = b.l();
                let mut cases = Vec::new();
                let mut bodies = String::new();
                for (vi, (vn, fs)) in vs.iter().enumerate() {
                    let mut sub = B { n: b.n, ..B::default() };
                    let l = sub.l();
                    cases.push(format!("i32 {}, label %{}", vi, l));
                    sub.label(&l);
                    if fs.is_empty() {
                        lit(self, &mut sub, vn);
                    } else {
                        lit(self, &mut sub, &format!("{}(", vn));
                        let vt = self.variant_lt(fs)?;
                        for (i, f) in fs.iter().enumerate() {
                            if i > 0 {
                                lit(self, &mut sub, ", ");
                            }
                            let fp = sub.val(format!("getelementptr {}, ptr {}, i32 0, i32 {}", vt, pl, i));
                            show(self, &mut sub, f, &fp);
                        }
                        lit(self, &mut sub, ")");
                    }
                    sub.br(&done);
                    b.n = sub.n;
                    bodies.push_str(&sub.text);
                }
                b.op(format!("switch i32 {}, label %{} [ {} ]", tag, done, cases.join(" ")));
                b.text.push_str(&bodies);
                b.label(&done);
            }
            Ty::Array(x, n) => {
                let at = self.lt(t)?;
                lit(self, b, "[");
                let (i, e) = b.loop_begin(&n.to_string());
                self.comma(b, &i)?;
                let ep = b.val(format!("getelementptr {}, ptr %p, i64 0, i64 {}", at, i));
                show(self, b, x, &ep);
                b.loop_end(e);
                lit(self, b, "]");
            }
            Ty::List(x) => {
                let xt = self.lt(x)?;
                let data = b.val("load ptr, ptr %p");
                let lp = b.val("getelementptr %list, ptr %p, i32 0, i32 1");
                let len = b.val(format!("load i64, ptr {}", lp));
                lit(self, b, "[");
                let (i, e) = b.loop_begin(&len);
                self.comma(b, &i)?;
                let ep = b.val(format!("getelementptr {}, ptr {}, i64 {}", xt, data, i));
                show(self, b, x, &ep);
                b.loop_end(e);
                lit(self, b, "]");
            }
            Ty::Map(k, v) => {
                let m = b.val("load ptr, ptr %p");
                let len = b.val(format!("call i64 @jw_map_len(ptr {})", m));
                lit(self, b, "[");
                let (i, e) = b.loop_begin(&len);
                self.comma(b, &i)?;
                let kv = b.val(format!("call i64 @jw_map_key(ptr {}, i64 {})", m, i));
                let kp = b.alloca(&self.lt(k)?);
                store_key(self, b, k, &kv, &kp)?;
                show(self, b, k, &kp);
                lit(self, b, ": ");
                let vp = b.val(format!("call ptr @jw_map_val(ptr {}, i64 {})", m, i));
                show(self, b, v, &vp);
                b.loop_end(e);
                lit(self, b, "]");
            }
            _ => return Err(format!("{} の表示", t.show())),
        }
        Ok(())
    }

    fn comma(&mut self, b: &mut B, i: &str) -> G<()> {
        let c = b.val(format!("icmp sgt i64 {}, 0", i));
        let (_, no) = b.if_true(&c);
        let s = self.cstr(", ");
        b.op(format!("call void @jw_sb_cstr(ptr %sb, ptr {})", s));
        b.br(&no);
        b.label(&no);
        Ok(())
    }

    /// interface の値の表示（構造体の番号で分かれる）
    fn dispatch_show(&mut self, b: &mut B, tid: &str, p: &str, d1: &str) -> G<()> {
        let structs = self.tids.clone();
        let done = b.l();
        let mut cases = Vec::new();
        let mut bodies = String::new();
        for (i, s) in structs.iter().enumerate() {
            let mut sub = B { n: b.n, ..B::default() };
            let l = sub.l();
            cases.push(format!("i64 {}, label %{}", i, l));
            sub.label(&l);
            let f = self.helper(H::Show, s);
            sub.op(format!("call void {}(ptr {}, ptr %sb, i64 {}, ptr %use)", f, p, d1));
            sub.br(&done);
            b.n = sub.n;
            bodies.push_str(&sub.text);
        }
        b.op(format!("switch i64 {}, label %{} [ {} ]", tid, done, cases.join(" ")));
        b.text.push_str(&bodies);
        b.label(&done);
        Ok(())
    }

    /// `moves` なら interp の `moves()`、でなければ `has_own()`（値を見て決める）
    fn gen_moves(&mut self, b: &mut B, t: &Ty, moves: bool) -> G<String> {
        let h = if moves { H::Moves } else { H::HasOwn };
        // 中身の一つでも当てはまれば true
        let any = |me: &mut Self, b: &mut B, parts: Vec<(String, Ty)>| -> String {
            let acc = b.alloca("i1");
            b.op(format!("store i1 false, ptr {}", acc));
            for (p, x) in parts {
                let f = me.helper(h, &x);
                let r = b.val(format!("call i1 {}(ptr {})", f, p));
                let a = b.val(format!("load i1, ptr {}", acc));
                let o = b.val(format!("or i1 {}, {}", a, r));
                b.op(format!("store i1 {}, ptr {}", o, acc));
            }
            b.val(format!("load i1, ptr {}", acc))
        };
        Ok(match t {
            Ty::Own(_) => {
                let p = if is_iface_ptr(t) {
                    let v = b.val("load %iown, ptr %p");
                    b.val(format!("extractvalue %iown {}, 0", v))
                } else {
                    b.val("load ptr, ptr %p")
                };
                let bits = b.val(format!("ptrtoint ptr {} to i64", p));
                let tag = b.val(format!("and i64 {}, 1", bits));
                b.val(format!("icmp eq i64 {}, 0", tag))
            }
            Ty::Fn(..) => env_flag(b, "%p", if moves { 1 } else { 2 }),
            Ty::Struct(..) | Ty::List(_) | Ty::Map(..) | Ty::Task(_) if moves => "true".to_string(),
            Ty::Task(_) | Ty::Chan(_) => "false".to_string(),
            Ty::Enum(..) if moves => {
                if self.enum_moves(t)? {
                    "true".to_string()
                } else {
                    "false".to_string()
                }
            }
            Ty::Struct(..) => {
                let st = self.lt(t)?;
                let mut parts = Vec::new();
                for (i, f) in self.struct_fields(t)?.iter().enumerate() {
                    if self.needs_drop(f)? {
                        parts.push((b.val(format!("getelementptr {}, ptr %p, i32 0, i32 {}", st, i)), f.clone()));
                    }
                }
                any(self, b, parts)
            }
            Ty::Enum(..) => {
                // 中身に own を持ちうる enum は、種類ごとに見る
                let vs = self.variants(t)?;
                let acc = b.alloca("i1");
                b.op(format!("store i1 false, ptr {}", acc));
                let tag = self.enum_tag(b, t, "%p")?;
                let pl = self.enum_payload(b, t, "%p")?;
                let done = b.l();
                let mut cases = Vec::new();
                let mut bodies = String::new();
                for (vi, (_, fs)) in vs.iter().enumerate() {
                    let mut sub = B { n: b.n, ..B::default() };
                    let l = sub.l();
                    cases.push(format!("i32 {}, label %{}", vi, l));
                    sub.label(&l);
                    let vt = self.variant_lt(fs)?;
                    let mut parts = Vec::new();
                    for (i, f) in fs.iter().enumerate() {
                        if self.needs_drop(f)? {
                            parts.push((sub.val(format!("getelementptr {}, ptr {}, i32 0, i32 {}", vt, pl, i)), f.clone()));
                        }
                    }
                    let r = any(self, &mut sub, parts);
                    sub.op(format!("store i1 {}, ptr {}", r, acc));
                    sub.br(&done);
                    b.n = sub.n;
                    b.allocas.push_str(&sub.allocas);
                    bodies.push_str(&sub.text);
                }
                b.op(format!("switch i32 {}, label %{} [ {} ]", tag, done, cases.join(" ")));
                b.text.push_str(&bodies);
                b.label(&done);
                b.val(format!("load i1, ptr {}", acc))
            }
            Ty::Array(x, n) => {
                let at = self.lt(t)?;
                let f = self.helper(h, x);
                let acc = b.alloca("i1");
                b.op(format!("store i1 false, ptr {}", acc));
                let (i, e) = b.loop_begin(&n.to_string());
                let ep = b.val(format!("getelementptr {}, ptr %p, i64 0, i64 {}", at, i));
                let r = b.val(format!("call i1 {}(ptr {})", f, ep));
                let a = b.val(format!("load i1, ptr {}", acc));
                let o = b.val(format!("or i1 {}, {}", a, r));
                b.op(format!("store i1 {}, ptr {}", o, acc));
                b.loop_end(e);
                b.val(format!("load i1, ptr {}", acc))
            }
            Ty::List(x) | Ty::Map(_, x) => {
                let f = self.helper(h, x);
                let acc = b.alloca("i1");
                b.op(format!("store i1 false, ptr {}", acc));
                let (len, get): (String, ElemPtr) = if let Ty::List(_) = t {
                    let xt = self.lt(x)?;
                    let data = b.val("load ptr, ptr %p");
                    let lp = b.val("getelementptr %list, ptr %p, i32 0, i32 1");
                    let len = b.val(format!("load i64, ptr {}", lp));
                    (len, Box::new(move |b: &mut B, i: &str| b.val(format!("getelementptr {}, ptr {}, i64 {}", xt, data, i))))
                } else {
                    let m = b.val("load ptr, ptr %p");
                    let len = b.val(format!("call i64 @jw_map_len(ptr {})", m));
                    (len, Box::new(move |b: &mut B, i: &str| b.val(format!("call ptr @jw_map_val(ptr {}, i64 {})", m, i))))
                };
                let (i, e) = b.loop_begin(&len);
                let ep = get(b, &i);
                let r = b.val(format!("call i1 {}(ptr {})", f, ep));
                let a = b.val(format!("load i1, ptr {}", acc));
                let o = b.val(format!("or i1 {}, {}", a, r));
                b.op(format!("store i1 {}, ptr {}", o, acc));
                b.loop_end(e);
                b.val(format!("load i1, ptr {}", acc))
            }
            _ => "false".to_string(),
        })
    }

    /// 関数の値を作るための関数（関数の番号 fi、取り込む値の型）。(呼び出しの入口, 入れ物の表)
    /// 入口 `@tramp{fi}(i64 深さ, ptr env, ptr 呼んだ位置, 引数...)` は、取り込んだ値の読むだけの写しを作って本体を呼ぶ
    fn closure_fns(&mut self, fi: usize, ctys: &[Ty]) -> G<(String, String)> {
        let tramp = format!("@tramp{}", fi);
        let vt = format!("@vt{}", fi);
        let key = (H::Clone, format!("closure {}", fi));
        if self.helpers.contains_key(&key) {
            return Ok((tramp, vt));
        }
        self.helpers.insert(key, tramp.clone());
        let f = &self.prog.fns[fi];
        let nparams = f.params - f.captures;
        let caps_t = self.variant_lt(ctys)?;
        let mut b = B::default();
        let mut params = vec!["i64 %depth".to_string(), "ptr %env".to_string(), "ptr %site".to_string()];
        let mut args = vec!["i64 %depth".to_string()];
        for i in 0..nparams {
            let lt = self.lt(&f.locals[i].ty)?;
            params.push(format!("{} %a{}", lt, i));
            args.push(format!("{} %a{}", lt, i));
        }
        let cp = b.val("getelementptr i8, ptr %env, i64 24");
        for (i, ct) in ctys.iter().enumerate() {
            let pt = f.locals[nparams + i].ty.clone();
            let src = b.val(format!("getelementptr {}, ptr {}, i32 0, i32 {}", caps_t, cp, i));
            let lt = self.lt(ct)?;
            let tmp = b.alloca(&lt);
            let view = self.helper(H::View, ct);
            b.op(format!("call void {}(ptr {}, ptr {}, ptr %site)", view, src, tmp));
            let v = b.val(format!("load {}, ptr {}", lt, tmp));
            // own は読むだけの ref として受け取る（lower の `lower_lambda`）
            let v = match (ct, &pt) {
                (Ty::Own(_), Ty::Ref(..)) => {
                    let iface = is_iface_ptr(ct);
                    let raw = if iface { b.val(format!("extractvalue %iown {}, 0", v)) } else { v.clone() };
                    let t = b.val(format!("call ptr @jw_view_target_nocheck(ptr {})", raw));
                    let g = b.val(format!("call i64 @jw_view_gen(ptr {})", raw));
                    let s = b.val(format!("call ptr @jw_view_site(ptr {})", raw));
                    b.op(format!("call void @jw_view_free(ptr {})", raw));
                    let rt = if iface { "%iref" } else { "%ref" };
                    let r = b.val(format!("insertvalue {} undef, ptr {}, 0", rt, t));
                    let r = b.val(format!("insertvalue {} {}, ptr {}, 1", rt, r, t));
                    let r = b.val(format!("insertvalue {} {}, i64 {}, 2", rt, r, g));
                    let r = b.val(format!("insertvalue {} {}, ptr {}, 3", rt, r, s));
                    if iface {
                        let tid = b.val(format!("extractvalue %iown {}, 1", v));
                        b.val(format!("insertvalue %iref {}, i64 {}, 4", r, tid))
                    } else {
                        r
                    }
                }
                _ => v,
            };
            args.push(format!("{} {}", self.lt(&pt)?, v));
        }
        let ret = self.lt(&f.ret)?;
        let r = b.val(format!("call {} @f{}({})", ret, fi, args.join(", ")));
        let _ = write!(
            self.code,
            "define internal {} {}({}) {{\nentry:\n{}{}  ret {} {}\n}}\n\n",
            ret,
            tramp,
            params.join(", "),
            b.allocas,
            b.text,
            ret,
            r
        );
        if ctys.is_empty() {
            return Ok((tramp, vt));
        }
        // 入れ物の解放と写し
        let mut b = B::default();
        let cp = b.val("getelementptr i8, ptr %env, i64 24");
        for (i, ct) in ctys.iter().enumerate() {
            if self.needs_drop(ct)? {
                let src = b.val(format!("getelementptr {}, ptr {}, i32 0, i32 {}", caps_t, cp, i));
                let d = self.helper(H::Drop, ct);
                b.op(format!("call void {}(ptr {}, ptr %site)", d, src));
            }
        }
        let _ = write!(self.code, "define internal void @envdrop{}(ptr %env, ptr %site) {{\nentry:\n{}{}  ret void\n}}\n\n", fi, b.allocas, b.text);
        let mut b = B::default();
        let size = format!("ptrtoint (ptr getelementptr ({}, ptr null, i32 1) to i64)", caps_t);
        let total = b.val(format!("add i64 24, {}", size));
        let ne = b.val(format!("call ptr @jw_env_new(i64 {}, ptr {})", total, vt));
        let cp = b.val("getelementptr i8, ptr %env, i64 24");
        let np = b.val(format!("getelementptr i8, ptr {}, i64 24", ne));
        for (i, ct) in ctys.iter().enumerate() {
            let src = b.val(format!("getelementptr {}, ptr {}, i32 0, i32 {}", caps_t, cp, i));
            let dst = b.val(format!("getelementptr {}, ptr {}, i32 0, i32 {}", caps_t, np, i));
            let view = self.helper(H::View, ct);
            b.op(format!("call void {}(ptr {}, ptr {}, ptr %site)", view, src, dst));
        }
        let flags = self.env_flags(&mut b, &np, ctys, &caps_t)?;
        // 元の値がムーブするものだったか（取り込んだ関数の値を、呼んだ先で取り出せるか）
        let old = b.val("getelementptr i8, ptr %env, i64 16");
        let old = b.val(format!("load i64, ptr {}", old));
        let om = b.val(format!("and i64 {}, 1", old));
        let om = b.val(format!("shl i64 {}, 2", om));
        let flags = b.val(format!("or i64 {}, {}", flags, om));
        let fp = b.val(format!("getelementptr i8, ptr {}, i64 16", ne));
        b.op(format!("store i64 {}, ptr {}", flags, fp));
        let _ = write!(self.code, "define internal ptr @envview{}(ptr %env, ptr %site) {{\nentry:\n{}{}  ret ptr {}\n}}\n\n", fi, b.allocas, b.text, ne);
        let _ = writeln!(self.globals, "{} = private unnamed_addr constant {{ ptr, ptr }} {{ ptr @envdrop{}, ptr @envview{} }}", vt, fi, fi);
        Ok((tramp, vt))
    }

    /// 取り込んだ値（`caps` の場所）から、入れ物の印（1: ムーブする・2: own を持つ）を決める
    fn env_flags(&mut self, b: &mut B, caps: &str, ctys: &[Ty], caps_t: &str) -> G<String> {
        let mut flags = "0".to_string();
        for (i, ct) in ctys.iter().enumerate() {
            let src = b.val(format!("getelementptr {}, ptr {}, i32 0, i32 {}", caps_t, caps, i));
            for (h, bit) in [(H::Moves, 1), (H::HasOwn, 2)] {
                let f = self.helper(h, ct);
                let r = b.val(format!("call i1 {}(ptr {})", f, src));
                let z = b.val(format!("zext i1 {} to i64", r));
                let sh = b.val(format!("mul i64 {}, {}", z, bit));
                flags = b.val(format!("or i64 {}, {}", flags, sh));
            }
        }
        Ok(flags)
    }

    fn gen_less(&mut self, b: &mut B, t: &Ty) -> G<String> {
        Ok(match t {
            Ty::Int(i) => {
                let a = b.val(format!("load i{}, ptr %p", i.bits()));
                let c = b.val(format!("load i{}, ptr %q", i.bits()));
                b.val(format!("icmp {} i{} {}, {}", if i.signed() { "slt" } else { "ult" }, i.bits(), a, c))
            }
            Ty::Float => {
                let a = b.val("load double, ptr %p");
                let c = b.val("load double, ptr %q");
                b.val(format!("fcmp olt double {}, {}", a, c))
            }
            Ty::Str => {
                let a = b.val("load ptr, ptr %p");
                let c = b.val("load ptr, ptr %q");
                let r = b.val(format!("call i32 @jw_str_cmp(ptr {}, ptr {})", a, c));
                b.val(format!("icmp slt i32 {}, 0", r))
            }
            _ => return Err(format!("{} の並べ替え", t.show())),
        })
    }
}

/// 値として読むとムーブになる型か（lower の `moves` と同じ）
fn moves(m: &mut Module, t: &Ty) -> G<bool> {
    Ok(match t {
        Ty::Own(_) | Ty::Struct(..) | Ty::List(_) | Ty::Map(..) | Ty::Fn(..) | Ty::Task(_) => true,
        Ty::Array(x, _) => moves(m, x)?,
        Ty::Enum(..) => m.enum_moves(t)?,
        _ => false,
    })
}

/// 関数の値（`p` の場所）の入れ物の印に `mask` が立っているか（入れ物がなければ false）
fn env_flag(b: &mut B, p: &str, mask: i64) -> String {
    let v = b.val(format!("load %fnv, ptr {}", p));
    let env = b.val(format!("extractvalue %fnv {}, 1", v));
    let has = b.val(format!("icmp ne ptr {}, null", env));
    let out = b.alloca("i1");
    b.op(format!("store i1 false, ptr {}", out));
    let (_, no) = b.if_true(&has);
    let fp = b.val(format!("getelementptr i8, ptr {}, i64 16", env));
    let f = b.val(format!("load i64, ptr {}", fp));
    let m = b.val(format!("and i64 {}, {}", f, mask));
    let r = b.val(format!("icmp ne i64 {}, 0", m));
    b.op(format!("store i1 {}, ptr {}", r, out));
    b.br(&no);
    b.label(&no);
    b.val(format!("load i1, ptr {}", out))
}

fn is_iface_ptr(t: &Ty) -> bool {
    matches!(t, Ty::Own(x) | Ty::Ref(x, _) if matches!(**x, Ty::Iface(_)))
}

/// 辞書のキーの種類（実行時ライブラリの `kind`）
fn key_kind(k: &Ty) -> i32 {
    match k {
        Ty::Str => 1,
        Ty::Bool => 2,
        _ => 0,
    }
}

/// 整数を i64 に広げる
fn widen(b: &mut B, v: &str, t: IntTy) -> String {
    if t.bits() == 64 {
        return v.to_string();
    }
    b.val(format!("{} i{} {} to i64", if t.signed() { "sext" } else { "zext" }, t.bits(), v))
}

/// 辞書のキー（i64 にしたもの）を、キーの型の値として `dst` に置く
fn store_key(m: &mut Module, b: &mut B, k: &Ty, kv: &str, dst: &str) -> G<()> {
    match k {
        Ty::Str => {
            let p = b.val(format!("inttoptr i64 {} to ptr", kv));
            b.op(format!("store ptr {}, ptr {}", p, dst));
        }
        Ty::Bool => {
            let v = b.val(format!("trunc i64 {} to i1", kv));
            b.op(format!("store i1 {}, ptr {}", v, dst));
        }
        Ty::Int(i) if i.bits() < 64 => {
            let v = b.val(format!("trunc i64 {} to i{}", kv, i.bits()));
            b.op(format!("store i{} {}, ptr {}", i.bits(), v, dst));
        }
        Ty::Int(_) => b.op(format!("store i64 {}, ptr {}", kv, dst)),
        _ => return Err(format!("辞書のキー {}", m.lt(k).unwrap_or_default())),
    }
    Ok(())
}

/// own（または写し）の先。写しなら世代を確かめる（`check` が false なら確かめない）
fn own_target(b: &mut B, p: &str, check: bool, use_site: &str, o: &Opts) -> String {
    let bits = b.val(format!("ptrtoint ptr {} to i64", p));
    let tag = b.val(format!("and i64 {}, 1", bits));
    let isv = b.val(format!("icmp ne i64 {}, 0", tag));
    let from = b.l();
    let yes = b.l();
    let join = b.l();
    b.br(&from);
    b.label(&from);
    b.op(format!("br i1 {}, label %{}, label %{}", isv, yes, join));
    b.label(&yes);
    let t = if check {
        count(b, o, "gen");
        b.val(format!("call ptr @jw_view_target(ptr {}, ptr {})", p, use_site))
    } else {
        count(b, o, "elided");
        b.val(format!("call ptr @jw_view_target_nocheck(ptr {})", p))
    };
    b.br(&join);
    b.label(&join);
    b.val(format!("phi ptr [ {}, %{} ], [ {}, %{} ]", p, from, t, yes))
}

/// ref の世代を確かめて、指す先を返す（`o.gen` が false なら確かめない）
fn check_ref(b: &mut B, rt: &str, r: &str, use_site: &str, o: &Opts) -> String {
    let addr = b.val(format!("extractvalue {} {}, 0", rt, r));
    if !o.gen {
        return addr;
    }
    count(b, o, "gen");
    let blk = b.val(format!("extractvalue {} {}, 1", rt, r));
    let gen = b.val(format!("extractvalue {} {}, 2", rt, r));
    let site = b.val(format!("extractvalue {} {}, 3", rt, r));
    let hp = b.val(format!("getelementptr i8, ptr {}, i64 -8", blk));
    let g = b.val(format!("load atomic i64, ptr {} monotonic, align 8", hp));
    let bad = b.val(format!("icmp ne i64 {}, {}", g, gen));
    let retired = b.val(format!("icmp eq i64 {}, -1", g));
    let bad = b.val(format!("or i1 {}, {}", bad, retired));
    let (_, ok) = b.if_true(&bad);
    b.op(format!("call void @jw_fail_dangling(ptr {}, ptr {}, ptr {}, i64 {})", use_site, site, blk, gen));
    b.op("unreachable");
    b.label(&ok);
    addr
}

/// 中の own を取り出そうとしたときの案内（interp と同じ）
fn copy_out_note(what: &str) -> &'static str {
    match what {
        "ポインタの先" => "`ref` で借りるか、`*p = ...` で置き換えてください",
        "フィールド" => "`ref` で借りるか、`p.f = ...` で置き換えてください",
        _ => "`ref a[i]` で借りるか、`a[i] = ...` で置き換えてください",
    }
}

fn esc(s: &str) -> String {
    let mut out = String::new();
    for &c in s.as_bytes() {
        if (0x20..0x7f).contains(&c) && c != b'"' && c != b'\\' {
            out.push(c as char);
        } else {
            let _ = write!(out, "\\{:02X}", c);
        }
    }
    out
}

fn float_lit(x: f64) -> String {
    format!("0x{:016X}", x.to_bits())
}

// ---------------------------------------------------------------- 関数

/// i 番目の要素の場所を求める命令を出す
type ElemPtr = Box<dyn Fn(&mut B, &str) -> String>;

/// 場所をたどった結果
struct PlaceRes {
    ptr: String,
    ty: Ty,
    /// 場所を含む区画（ref で借りるときの世代の元）。スタックの変数なら None
    block: Option<String>,
    /// 最後の区画のあとでリスト・辞書の要素の中を通ったか（要素の印 TBAA をつけてよい）
    list: bool,
    /// 最後の区画のあとで辞書の中を通ったか（借りられない）
    map: bool,
    /// `block` がリストの中身の区画か（要素を借りるときに世代へ「借りられた」の印を立てる。jw_rt.c の `jw_list_moved`）
    list_block: bool,
    /// interface の先なら、構造体の番号
    tid: Option<String>,
    /// 辞書に新しく入れたキーの場所か（代入で古い値を解放しない）
    fresh: Option<String>,
}

struct FnGen<'m, 'p> {
    m: &'m mut Module<'p>,
    f: &'p Func,
    idx: usize,
    b: B,
    /// 区画に置く変数（`ref x` で変数そのものを借りる）
    boxed: Vec<bool>,
    /// ムーブされうる変数（読むときにムーブ済みか確かめる）
    may_move: Vec<bool>,
    /// 呼び出しの深さ（関数の最初の引数。グローバル変数に書かないので、呼び出しが安い）
    depth: String,
}

impl<'m, 'p> FnGen<'m, 'p> {
    fn new(m: &'m mut Module<'p>, f: &'p Func, idx: usize) -> Self {
        let n = f.locals.len();
        let mut boxed = vec![false; n];
        let mut may_move = vec![false; n];
        let mut mark = |o: &Operand| {
            if let Operand::Move(l, _) | Operand::Read(l, _) = o {
                may_move[*l] = true;
            }
        };
        for blk in &f.blocks {
            for inst in &blk.insts {
                visit_operands(inst, &mut mark);
                if let Inst::Assign { value, .. } = inst {
                    match value {
                        Rvalue::Ref { place, follows: false, .. } | Rvalue::CallDyn { this: place, .. }
                            if !place.proj.iter().any(|p| matches!(p, Proj::Deref(..))) =>
                        {
                            boxed[place.local] = true
                        }
                        _ => {}
                    }
                }
            }
            match &blk.term {
                Term::Branch(o, ..) | Term::Return(o) => mark(o),
                Term::Step { step, .. } => mark(step),
                _ => {}
            }
        }
        FnGen { m, f, idx, b: B::default(), boxed, may_move, depth: "%depth".to_string() }
    }

    fn run(mut self) -> G<()> {
        let f = self.f;
        let mut params = vec!["i64 %depth".to_string()];
        for i in 0..f.params {
            params.push(format!("{} %a{}", self.m.lt(&f.locals[i].ty)?, i));
        }
        let ret = self.m.lt(&f.ret)?;
        // 変数と状態
        let mut init = String::new();
        for (i, l) in f.locals.iter().enumerate() {
            let lt = self.m.lt(&l.ty)?;
            if self.boxed[i] {
                let _ = writeln!(self.b.allocas, "  %l{} = alloca ptr", i);
            } else {
                let _ = writeln!(self.b.allocas, "  %l{} = alloca {}", i, lt);
            }
            let _ = writeln!(self.b.allocas, "  %s{} = alloca ptr", i);
            if i < f.params {
                let _ = writeln!(init, "  store ptr null, ptr %s{}", i);
                if self.boxed[i] {
                    let size = self.m.sizeof(&l.ty)?;
                    let _ = writeln!(init, "  %box{} = call ptr @jw_alloc(i64 {})", i, size);
                    let _ = writeln!(init, "  store {} %a{}, ptr %box{}", lt, i, i);
                    let _ = writeln!(init, "  store ptr %box{}, ptr %l{}", i, i);
                } else {
                    let _ = writeln!(init, "  store {} %a{}, ptr %l{}", lt, i, i);
                }
            } else {
                let _ = writeln!(init, "  store ptr @jw.unset, ptr %s{}", i);
            }
        }
        for (bi, blk) in f.blocks.iter().enumerate() {
            self.b.label(&format!("bb{}", bi));
            // match で中身を取り出した入れ物（このブロックの drop は中身を解放しない）
            let mut taken: Vec<Local> = Vec::new();
            for inst in &blk.insts {
                if let Inst::Assign { value: Rvalue::Take(p), .. } = inst {
                    taken.push(p.local);
                }
                self.inst(inst, &taken)?;
            }
            self.term(&blk.term)?;
        }
        let _ = write!(
            self.m.code,
            "; {}\ndefine internal {} @f{}({}) {{\nentry:\n{}{}  br label %bb0\n{}}}\n\n",
            f.name,
            ret,
            self.idx,
            params.join(", "),
            self.b.allocas,
            init,
            self.b.text
        );
        Ok(())
    }

    fn name_of(&self, l: Local) -> &str {
        &self.f.locals[l].name
    }

    // ------------------------------------------------------------ 変数

    /// 変数に値があるか確かめる（ムーブ済みならエラー）
    fn check_live(&mut self, l: Local, span: Span) {
        if !self.may_move[l] {
            return;
        }
        let st = self.b.val(format!("load ptr, ptr %s{}", l));
        let live = self.b.val(format!("icmp eq ptr {}, null", st));
        let bad = self.b.l();
        let ok = self.b.l();
        self.b.op(format!("br i1 {}, label %{}, label %{}", live, ok, bad));
        self.b.label(&bad);
        let name = self.m.cstr(&self.f.locals[l].name);
        let use_site = self.m.site(span, "");
        self.b.op(format!("call void @jw_fail_moved(ptr {}, ptr {}, ptr {})", name, st, use_site));
        self.b.op("unreachable");
        self.b.label(&ok);
    }

    /// 変数の値の場所（区画に置く変数なら区画の中）
    fn local_ptr(&mut self, l: Local) -> String {
        if self.boxed[l] {
            self.b.val(format!("load ptr, ptr %l{}", l))
        } else {
            format!("%l{}", l)
        }
    }

    /// 変数の値を解放して、空にする（`drop`）。`shallow` なら中身は解放しない（match で取り出した入れ物）
    fn drop_local(&mut self, l: Local, span: Span, why: &str, shallow: bool) -> G<()> {
        let ty = self.f.locals[l].ty.clone();
        let st = self.b.val(format!("load ptr, ptr %s{}", l));
        let live = self.b.val(format!("icmp eq ptr {}, null", st));
        let (_, done) = self.b.if_true(&live);
        let site = self.m.site(span, why);
        let p = self.local_ptr(l);
        if shallow {
            // own の入れ物だけを解放する（中の enum の中身は取り出し済み）
            if let Ty::Own(x) = &ty {
                let v = self.b.val(format!("load ptr, ptr {}", p));
                let size = self.m.sizeof(x)?;
                self.b.op(format!("call void @jw_free(ptr {}, i64 {}, ptr {})", v, size, site));
            }
        } else if self.m.needs_drop(&ty)? {
            let d = self.m.helper(H::Drop, &ty);
            self.b.op(format!("call void {}(ptr {}, ptr {})", d, p, site));
        }
        if self.boxed[l] {
            let size = self.m.sizeof(&ty)?;
            self.b.op(format!("call void @jw_free(ptr {}, i64 {}, ptr {})", p, size, site));
        }
        self.b.op(format!("store ptr @jw.unset, ptr %s{}", l));
        self.b.br(&done);
        self.b.label(&done);
        Ok(())
    }

    /// 変数に値を置く（古い値があれば解放する）
    fn assign_local(&mut self, l: Local, v: &str, span: Span) -> G<()> {
        let ty = self.f.locals[l].ty.clone();
        let lt = self.m.lt(&ty)?;
        let st = self.b.val(format!("load ptr, ptr %s{}", l));
        let live = self.b.val(format!("icmp eq ptr {}, null", st));
        let yes = self.b.l();
        let no = self.b.l();
        let done = self.b.l();
        self.b.op(format!("br i1 {}, label %{}, label %{}", live, yes, no));
        self.b.label(&yes);
        let p = self.local_ptr(l);
        if self.m.needs_drop(&ty)? {
            // 新しい値を置いてから古い値を解放する（interp と同じ順）
            let old = self.b.val(format!("load {}, ptr {}", lt, p));
            self.b.op(format!("store {} {}, ptr {}", lt, v, p));
            let tmp = self.b.alloca(&lt);
            self.b.op(format!("store {} {}, ptr {}", lt, old, tmp));
            let why = format!("`{}` に再代入されて、古い値が解放された", self.name_of(l));
            let site = self.m.site(span, &why);
            let d = self.m.helper(H::Drop, &ty);
            self.b.op(format!("call void {}(ptr {}, ptr {})", d, tmp, site));
        } else {
            self.b.op(format!("store {} {}, ptr {}", lt, v, p));
        }
        self.b.br(&done);
        self.b.label(&no);
        if self.boxed[l] {
            let size = self.m.sizeof(&ty)?;
            let blk = self.b.val(format!("call ptr @jw_alloc(i64 {})", size));
            self.b.op(format!("store {} {}, ptr {}", lt, v, blk));
            self.b.op(format!("store ptr {}, ptr %l{}", blk, l));
        } else {
            self.b.op(format!("store {} {}, ptr %l{}", lt, v, l));
        }
        self.b.op(format!("store ptr null, ptr %s{}", l));
        self.b.br(&done);
        self.b.label(&done);
        Ok(())
    }

    // ------------------------------------------------------------ 命令

    fn inst(&mut self, inst: &Inst, taken: &[Local]) -> G<()> {
        match inst {
            Inst::Assign { place, value, span, what } => {
                let want = self.place_ty(place)?;
                let v = self.rvalue(value, &want)?;
                if place.proj.is_empty() {
                    return self.assign_local(place.local, &v, *span);
                }
                let pr = self.place(place, true)?;
                let lt = self.m.lt(&pr.ty)?;
                let v = self.coerce(&v, &want, &pr.ty)?;
                if self.m.needs_drop(&pr.ty)? {
                    let old = self.b.alloca(&lt);
                    let ov = self.b.val(format!("load {}, ptr {}", lt, pr.ptr));
                    self.b.op(format!("store {} {}, ptr {}", lt, v, pr.ptr));
                    let skip = match &pr.fresh {
                        Some(fr) => {
                            let (_, no) = {
                                let c = self.b.val(format!("xor i1 {}, true", fr));
                                self.b.if_true(&c)
                            };
                            Some(no)
                        }
                        None => None,
                    };
                    self.b.op(format!("store {} {}, ptr {}", lt, ov, old));
                    let site = self.m.site(*span, &super::overwritten(what));
                    let d = self.m.helper(H::Drop, &pr.ty);
                    self.b.op(format!("call void {}(ptr {}, ptr {})", d, old, site));
                    if let Some(no) = skip {
                        self.b.br(&no);
                        self.b.label(&no);
                    }
                } else {
                    let tag = if pr.list && scalar_lt(&lt) { TBAA_ELEMENT } else { "" };
                    self.b.op(format!("store {} {}, ptr {}{}", lt, v, pr.ptr, tag));
                }
            }
            Inst::Drop { local, span, why } => {
                self.drop_local(*local, *span, why, taken.contains(local))?;
            }
            Inst::Discard { value, span, why } => {
                let (v, t) = self.operand(value)?;
                self.discard(&v, &t, *span, why)?;
            }
        }
        Ok(())
    }

    /// 値を捨てる（中の own を解放する）
    fn discard(&mut self, v: &str, t: &Ty, span: Span, why: &str) -> G<()> {
        if self.m.needs_drop(t)? {
            let lt = self.m.lt(t)?;
            let tmp = self.b.alloca(&lt);
            self.b.op(format!("store {} {}, ptr {}", lt, v, tmp));
            let site = self.m.site(span, why);
            let d = self.m.helper(H::Drop, t);
            self.b.op(format!("call void {}(ptr {}, ptr {})", d, tmp, site));
        }
        Ok(())
    }

    fn term(&mut self, t: &Term) -> G<()> {
        match t {
            Term::Goto(b) => self.b.op(format!("br label %bb{}", b)),
            Term::Branch(c, t, e) => {
                let (v, _) = self.operand(c)?;
                self.b.op(format!("br i1 {}, label %bb{}, label %bb{}", v, t, e));
            }
            Term::Return(o) => {
                let (v, t) = self.operand(o)?;
                let ret = self.f.ret.clone();
                let v = self.coerce(&v, &t, &ret)?;
                let lt = self.m.lt(&ret)?;
                self.b.op(format!("ret {} {}", lt, v));
            }
            Term::Switch(place, arms, other) => {
                let pr = self.place(place, false)?;
                let tag = self.m.enum_tag(&mut self.b, &pr.ty, &pr.ptr)?;
                let mut cases = Vec::new();
                for (v, b) in arms {
                    let i = self.m.variant_index(&pr.ty, v)?;
                    cases.push(format!("i32 {}, label %bb{}", i, b));
                }
                self.b.op(format!("switch i32 {}, label %bb{} [ {} ]", tag, other, cases.join(" ")));
            }
            Term::Step { counter, step, next, done } => {
                let Ty::Int(it) = self.f.locals[*counter].ty.clone() else { return Err("整数でないループの数".to_string()) };
                let (sv, st) = self.operand(step)?;
                let Ty::Int(sty) = st else { return Err("整数でない step".to_string()) };
                let p = self.local_ptr(*counter);
                let bits = it.bits();
                let n = self.b.val(format!("load i{}, ptr {}", bits, p));
                // i128 で足して、型の範囲に入るか（interp と同じ）
                let n2 = self.b.val(format!("{} i{} {} to i128", ext(it), bits, n));
                let s2 = self.b.val(format!("{} i{} {} to i128", ext(sty), sty.bits(), sv));
                let sum = self.b.val(format!("add i128 {}, {}", n2, s2));
                let lo = self.b.val(format!("icmp sge i128 {}, {}", sum, it.min()));
                let hi = self.b.val(format!("icmp sle i128 {}, {}", sum, it.max()));
                let ok = self.b.val(format!("and i1 {}, {}", lo, hi));
                let tr = self.b.val(format!("trunc i128 {} to i{}", sum, bits));
                let (_, no) = self.b.if_true(&ok);
                self.b.op(format!("store i{} {}, ptr {}", bits, tr, p));
                self.b.op(format!("br label %bb{}", next));
                self.b.label(&no);
                self.b.op(format!("br label %bb{}", done));
            }
            Term::Fail(title, span, label) => {
                let t = self.m.cstr(title);
                let l = self.m.cstr(label);
                let s = self.m.site(*span, "");
                self.b.op(format!("call void @jw_fail(ptr {}, ptr {}, ptr {}, ptr null)", t, s, l));
                self.b.op("unreachable");
            }
            Term::Unreachable => self.b.op("unreachable"),
        }
        Ok(())
    }

    // ------------------------------------------------------------ 場所

    /// 場所の型（命令を出さずに決める）
    fn place_ty(&mut self, p: &Place) -> G<Ty> {
        let mut ty = self.f.locals[p.local].ty.clone();
        for pr in &p.proj {
            ty = match (pr, &ty) {
                (Proj::Deref(..), Ty::Own(x) | Ty::Ref(x, _)) => (**x).clone(),
                (Proj::Field(i, _), Ty::Struct(..)) => self.m.struct_fields(&ty)?[*i].clone(),
                (Proj::Variant(v, i), Ty::Enum(..)) => {
                    let vs = self.m.variants(&ty)?;
                    let (_, fs) = vs.iter().find(|(n, _)| **n == **v).ok_or("enum の種類がない")?;
                    fs[*i].clone()
                }
                (Proj::Index(..), Ty::Array(x, _) | Ty::List(x)) => (**x).clone(),
                (Proj::Index(..), Ty::Map(_, v)) => (**v).clone(),
                _ => return Err(format!("場所の型 {}", ty.show())),
            };
        }
        Ok(ty)
    }

    /// 場所をたどる（ref の世代チェック・添字の範囲・辞書のキー）。`create` なら最後の辞書のキーがなければ入れる
    fn place(&mut self, p: &Place, create: bool) -> G<PlaceRes> {
        let l = p.local;
        self.check_live(l, p.span);
        let ptr = self.local_ptr(l);
        let block = if self.boxed[l] { Some(ptr.clone()) } else { None };
        let mut r = PlaceRes { ptr, ty: self.f.locals[l].ty.clone(), block, list: false, map: false, list_block: false, tid: None, fresh: None };
        let last = p.proj.len();
        for (k, pr) in p.proj.iter().enumerate() {
            match pr {
                Proj::Deref(check, span) => {
                    let o = self.m.opts;
                    let elided = *check == Check::Elided;
                    if elided {
                        self.m.stats.elided += 1;
                    } else {
                        self.m.stats.gen += 1;
                    }
                    let check = !elided && o.gen;
                    let use_site = self.m.site(*span, "");
                    match r.ty.clone() {
                        Ty::Own(x) => {
                            let raw = if is_iface_ptr(&r.ty) {
                                let v = self.b.val(format!("load %iown, ptr {}", r.ptr));
                                r.tid = Some(self.b.val(format!("extractvalue %iown {}, 1", v)));
                                self.b.val(format!("extractvalue %iown {}, 0", v))
                            } else {
                                self.b.val(format!("load ptr, ptr {}", r.ptr))
                            };
                            let t = own_target(&mut self.b, &raw, check, &use_site, &o);
                            r.ptr = t.clone();
                            r.block = Some(t);
                            r.ty = *x;
                        }
                        Ty::Ref(x, _) => {
                            let iface = is_iface_ptr(&r.ty);
                            let rt = if iface { "%iref" } else { "%ref" };
                            let v = self.b.val(format!("load {}, ptr {}", rt, r.ptr));
                            let addr = if check {
                                check_ref(&mut self.b, rt, &v, &use_site, &o)
                            } else {
                                if elided {
                                    count(&mut self.b, &o, "elided");
                                }
                                self.b.val(format!("extractvalue {} {}, 0", rt, v))
                            };
                            if iface {
                                r.tid = Some(self.b.val(format!("extractvalue %iref {}, 4", v)));
                            }
                            r.block = Some(self.b.val(format!("extractvalue {} {}, 1", rt, v)));
                            r.ptr = addr;
                            r.ty = *x;
                        }
                        t => return Err(format!("{} の先", t.show())),
                    }
                    r.list = false;
                    r.map = false;
                    r.list_block = false;
                }
                Proj::Field(i, _) => {
                    let st = self.m.lt(&r.ty)?;
                    let fs = self.m.struct_fields(&r.ty)?;
                    r.ptr = self.b.val(format!("getelementptr {}, ptr {}, i32 0, i32 {}", st, r.ptr, i));
                    r.ty = fs[*i].clone();
                }
                Proj::Variant(v, i) => {
                    let vs = self.m.variants(&r.ty)?;
                    let (_, fs) = vs.iter().find(|(n, _)| **n == **v).ok_or("enum の種類がない")?;
                    let vt = self.m.variant_lt(fs)?;
                    let pl = self.m.enum_payload(&mut self.b, &r.ty, &r.ptr)?;
                    r.ptr = self.b.val(format!("getelementptr {}, ptr {}, i32 0, i32 {}", vt, pl, i));
                    r.ty = fs[*i].clone();
                }
                Proj::Index(key, key_span, _) => {
                    let (kv, kt) = self.operand(key)?;
                    match r.ty.clone() {
                        Ty::Array(x, n) => {
                            let i = self.index(&kv, &kt, &n.to_string(), *key_span)?;
                            let at = self.m.lt(&r.ty)?;
                            r.ptr = self.b.val(format!("getelementptr {}, ptr {}, i64 0, i64 {}", at, r.ptr, i));
                            r.ty = *x;
                        }
                        Ty::List(x) => {
                            let lp = self.b.val(format!("getelementptr %list, ptr {}, i32 0, i32 1", r.ptr));
                            let len = self.b.val(format!("load i64, ptr {}{}", lp, TBAA_HEADER));
                            let i = self.index(&kv, &kt, &len, *key_span)?;
                            let data = self.b.val(format!("load ptr, ptr {}{}", r.ptr, TBAA_HEADER));
                            let xt = self.m.lt(&x)?;
                            r.ptr = self.b.val(format!("getelementptr {}, ptr {}, i64 {}", xt, data, i));
                            r.ty = *x;
                            r.list = true;
                            // 要素はリストの中身の区画にある（要素を借りた ref はこの区画の世代を持つ）
                            r.block = Some(data);
                            r.map = false;
                            r.list_block = true;
                        }
                        Ty::Map(kty, v) => {
                            let k64 = self.key64(&kv, &kty)?;
                            let m = self.b.val(format!("load ptr, ptr {}", r.ptr));
                            let i = self.b.val(format!("call i64 @jw_map_find(ptr {}, i64 {})", m, k64));
                            let missing = self.b.val(format!("icmp slt i64 {}, 0", i));
                            if create && k + 1 == last {
                                // `m[k] = v`: なければ入れる（キーは辞書のものになる）
                                let ip = self.b.alloca("i64");
                                self.b.op(format!("store i64 {}, ptr {}", i, ip));
                                let (_, no) = self.b.if_true(&missing);
                                let j = self.b.val(format!("call i64 @jw_map_add(ptr {}, i64 {})", m, k64));
                                self.b.op(format!("store i64 {}, ptr {}", j, ip));
                                let skip = self.b.l();
                                self.b.br(&skip);
                                self.b.label(&no);
                                self.release_key(&kv, &kty);
                                self.b.br(&skip);
                                self.b.label(&skip);
                                let i = self.b.val(format!("load i64, ptr {}", ip));
                                r.ptr = self.b.val(format!("call ptr @jw_map_val(ptr {}, i64 {})", m, i));
                                r.fresh = Some(missing);
                            } else {
                                let (_, ok) = self.b.if_true(&missing);
                                let unsigned = matches!(&*kty, Ty::Int(t) if !t.signed());
                                let s = self.m.site(*key_span, "");
                                self.b.op(format!(
                                    "call void @jw_fail_nokey(i32 {}, i64 {}, i32 {}, ptr {})",
                                    key_kind(&kty),
                                    k64,
                                    unsigned as i32,
                                    s
                                ));
                                self.b.op("unreachable");
                                self.b.label(&ok);
                                self.release_key(&kv, &kty);
                                r.ptr = self.b.val(format!("call ptr @jw_map_val(ptr {}, i64 {})", m, i));
                            }
                            r.ty = *v;
                            r.list = true;
                            r.map = true;
                        }
                        t => return Err(format!("{} の添字", t.show())),
                    }
                }
            }
        }
        Ok(r)
    }

    /// 添字の範囲を確かめて i64 にする
    fn index(&mut self, kv: &str, kt: &Ty, len: &str, span: Span) -> G<String> {
        let Ty::Int(t) = kt else { return Err("整数でない添字".to_string()) };
        let i = widen(&mut self.b, kv, *t);
        let o = self.m.opts;
        self.m.stats.bounds += 1;
        if !o.bounds {
            return Ok(i);
        }
        count(&mut self.b, &o, "bounds");
        let bad = if t.signed() {
            let neg = self.b.val(format!("icmp slt i64 {}, 0", i));
            let big = self.b.val(format!("icmp sge i64 {}, {}", i, len));
            self.b.val(format!("or i1 {}, {}", neg, big))
        } else {
            self.b.val(format!("icmp uge i64 {}, {}", i, len))
        };
        let (_, ok) = self.b.if_true(&bad);
        let s = self.m.site(span, "");
        self.b.op(format!("call void @jw_fail_bounds(i64 {}, i32 {}, i64 {}, ptr {})", i, !t.signed() as i32, len, s));
        self.b.op("unreachable");
        self.b.label(&ok);
        Ok(i)
    }

    /// 辞書のキーを i64 にする
    fn key64(&mut self, kv: &str, kt: &Ty) -> G<String> {
        Ok(match kt {
            Ty::Str => self.b.val(format!("ptrtoint ptr {} to i64", kv)),
            Ty::Bool => self.b.val(format!("zext i1 {} to i64", kv)),
            Ty::Int(t) => widen(&mut self.b, kv, *t),
            _ => return Err(format!("辞書のキー {}", kt.show())),
        })
    }

    fn release_key(&mut self, kv: &str, kt: &Ty) {
        if matches!(kt, Ty::Str) {
            self.b.op(format!("call void @jw_str_release(ptr {})", kv));
        }
    }

    // ------------------------------------------------------------ 値

    /// 値（自分のものになった値。文字列なら数を 1 つ持つ）と型
    fn operand(&mut self, o: &Operand) -> G<(String, Ty)> {
        match o {
            Operand::Const(c) => Ok(match c {
                Const::Unit => ("0".to_string(), Ty::Unit),
                Const::Int(n, t) => {
                    // ビットの形で書く（u64 の大きな値は負の数になる）
                    let v = if t.bits() == 64 { (*n as u64 as i64) as i128 } else { *n };
                    (v.to_string(), Ty::Int(*t))
                }
                Const::Float(x) => (float_lit(*x), Ty::Float),
                Const::Bool(b) => (b.to_string(), Ty::Bool),
                Const::Str(s) => (self.m.strconst(s), Ty::Str),
            }),
            Operand::Copy(p) => {
                let pr = self.place(p, false)?;
                let v = self.read_place(&pr)?;
                Ok((v, pr.ty))
            }
            Operand::Move(l, span) => {
                let l = *l;
                self.check_live(l, *span);
                let ty = self.f.locals[l].ty.clone();
                let lt = self.m.lt(&ty)?;
                let p = self.local_ptr(l);
                let v = self.b.val(format!("load {}, ptr {}", lt, p));
                let why = format!("`{}` がムーブされた", self.name_of(l));
                let site = self.m.site(*span, &why);
                if self.boxed[l] {
                    let size = self.m.sizeof(&ty)?;
                    self.b.op(format!("call void @jw_free(ptr {}, i64 {}, ptr {})", p, size, site));
                }
                self.b.op(format!("store ptr {}, ptr %s{}", site, l));
                Ok((v, ty))
            }
            Operand::Read(l, span) => self.read(*l, *span),
        }
    }

    /// 値によって決める読み方（interp の `read_var`）。取り込んだ変数・関数の値の変数
    fn read(&mut self, l: Local, span: Span) -> G<(String, Ty)> {
        let f = self.f;
        let ty = f.locals[l].ty.clone();
        let first_cap = f.params - f.captures;
        let is_cap = l >= first_cap && l < f.params;
        self.check_live(l, span);
        let p = self.local_ptr(l);
        if is_cap {
            // 取り込んだ元の値がムーブするものなら取り出せない（呼ぶたびに使うので）
            let cant = match &ty {
                Ty::Fn(..) => Some(env_flag(&mut self.b, &p, 4)),
                _ => {
                    // own は ref として受け取っている（元は own なのでムーブする）
                    let orig_moves = matches!(ty, Ty::Ref(..)) && self.cap_was_own(l) || moves(self.m, &ty)?;
                    if orig_moves {
                        Some("true".to_string())
                    } else {
                        None
                    }
                }
            };
            if let Some(c) = cant {
                let always = c == "true";
                let (_, ok) = self.b.if_true(&c);
                let title = self.m.cstr(&format!("クロージャに取り込んだ `{}` は取り出せません", f.locals[l].name));
                let lab = self.m.cstr("クロージャは何度も呼べるので、取り込んだ値をムーブできない");
                let s = self.m.site(span, "");
                let note = self.m.cstr("`ref` で借りるか、中の値をコピーして使ってください");
                self.b.op(format!("call void @jw_fail(ptr {}, ptr {}, ptr {}, ptr {})", title, s, lab, note));
                self.b.op("unreachable");
                self.b.label(&ok);
                if always {
                    // ここには来ない。元の型の値として続ける
                    let orig = f.cap_tys.get(l - first_cap).cloned().unwrap_or(ty);
                    return Ok(("poison".to_string(), orig));
                }
            }
            let v = self.read_copy(&p, &ty)?;
            return Ok((v, ty));
        }
        if !matches!(ty, Ty::Fn(..)) {
            return Err(format!("値で決める読み方 {}", ty.show()));
        }
        // 関数の値: 取り込んだものがムーブするならムーブ、でなければ複製
        let mv = env_flag(&mut self.b, &p, 1);
        let out = self.b.alloca("%fnv");
        let yes = self.b.l();
        let no = self.b.l();
        let done = self.b.l();
        self.b.op(format!("br i1 {}, label %{}, label %{}", mv, yes, no));
        self.b.label(&yes);
        let v = self.b.val(format!("load %fnv, ptr {}", p));
        self.b.op(format!("store %fnv {}, ptr {}", v, out));
        let why = format!("`{}` がムーブされた", f.locals[l].name);
        let site = self.m.site(span, &why);
        if self.boxed[l] {
            let size = self.m.sizeof(&ty)?;
            self.b.op(format!("call void @jw_free(ptr {}, i64 {}, ptr {})", p, size, site));
        }
        self.b.op(format!("store ptr {}, ptr %s{}", site, l));
        self.b.br(&done);
        self.b.label(&no);
        let c = self.read_copy(&p, &ty)?;
        self.b.op(format!("store %fnv {}, ptr {}", c, out));
        self.b.br(&done);
        self.b.label(&done);
        Ok((self.b.val(format!("load %fnv, ptr {}", out)), ty))
    }

    /// 取り込んだ変数 `l` の元の型が own だったか（lower が ref にしている）
    fn cap_was_own(&self, l: Local) -> bool {
        let first = self.f.params - self.f.captures;
        matches!(self.f.cap_tys.get(l.wrapping_sub(first)), Some(Ty::Own(_)))
    }

    /// 場所の値を複製して読む
    /// たどった場所の写しを読む（リスト・辞書の要素の中の数なら、要素の印をつける）
    fn read_place(&mut self, pr: &PlaceRes) -> G<String> {
        let lt = self.m.lt(&pr.ty)?;
        if pr.list && self.m.trivial(&pr.ty)? && scalar_lt(&lt) {
            return Ok(self.b.val(format!("load {}, ptr {}{}", lt, pr.ptr, TBAA_ELEMENT)));
        }
        self.read_copy(&pr.ptr, &pr.ty)
    }

    fn read_copy(&mut self, ptr: &str, ty: &Ty) -> G<String> {
        let lt = self.m.lt(ty)?;
        if self.m.trivial(ty)? {
            return Ok(self.b.val(format!("load {}, ptr {}", lt, ptr)));
        }
        if matches!(ty, Ty::Str) {
            let v = self.b.val(format!("load ptr, ptr {}", ptr));
            self.b.op(format!("call void @jw_str_retain(ptr {})", v));
            return Ok(v);
        }
        let tmp = self.b.alloca(&lt);
        let c = self.m.helper(H::Clone, ty);
        self.b.op(format!("call void {}(ptr {}, ptr {})", c, ptr, tmp));
        Ok(self.b.val(format!("load {}, ptr {}", lt, tmp)))
    }

    /// 型 `from` の値を `to` の形にする（interface への変換・整数の幅・own の写し → ref）
    fn coerce(&mut self, v: &str, from: &Ty, to: &Ty) -> G<String> {
        if from == to || matches!(to, Ty::Never) {
            return Ok(v.to_string());
        }
        if matches!(from, Ty::Never) {
            let lt = self.m.lt(to)?;
            return Ok(if lt == "i8" { "0" } else { "poison" }.to_string());
        }
        let (lf, lto) = (self.m.lt(from)?, self.m.lt(to)?);
        match (from, to) {
            (Ty::Own(s), Ty::Own(i)) if matches!(**i, Ty::Iface(_)) && !matches!(**s, Ty::Iface(_)) => {
                let tid = self.m.tid(s);
                let a = self.b.val(format!("insertvalue %iown undef, ptr {}, 0", v));
                return Ok(self.b.val(format!("insertvalue %iown {}, i64 {}, 1", a, tid)));
            }
            (Ty::Ref(s, _), Ty::Ref(i, _)) if matches!(**i, Ty::Iface(_)) && !matches!(**s, Ty::Iface(_)) => {
                let tid = self.m.tid(s);
                let mut acc = "undef".to_string();
                for k in 0..4 {
                    let f = self.b.val(format!("extractvalue %ref {}, {}", v, k));
                    let ft = ["ptr", "ptr", "i64", "ptr"][k];
                    acc = self.b.val(format!("insertvalue %iref {}, {} {}, {}", acc, ft, f, k));
                }
                return Ok(self.b.val(format!("insertvalue %iref {}, i64 {}, 4", acc, tid)));
            }
            // own の写し（for の要素）を ref にする
            (Ty::Own(x), Ty::Ref(y, _)) if x == y && !is_iface_ptr(from) => {
                let bits = self.b.val(format!("ptrtoint ptr {} to i64", v));
                let tag = self.b.val(format!("and i64 {}, 1", bits));
                let real = self.b.val(format!("icmp eq i64 {}, 0", tag));
                let (_, no) = self.b.if_true(&real);
                let msg = self.m.cstr("own を ref にしようとした");
                self.b.op(format!("call void @jw_bug(ptr {})", msg));
                self.b.op("unreachable");
                self.b.label(&no);
                let t = self.b.val(format!("call ptr @jw_view_target_nocheck(ptr {})", v));
                let g = self.b.val(format!("call i64 @jw_view_gen(ptr {})", v));
                let s = self.b.val(format!("call ptr @jw_view_site(ptr {})", v));
                self.b.op(format!("call void @jw_view_free(ptr {})", v));
                return Ok(self.make_ref(&t, &t, &g, &s));
            }
            // interface の own の写し（for の要素）を ref にする: 中の写しを ref にして、構造体の番号をつける
            (Ty::Own(x), Ty::Ref(y, m)) if x == y && is_iface_ptr(from) => {
                let p = self.b.val(format!("extractvalue %iown {}, 0", v));
                let tid = self.b.val(format!("extractvalue %iown {}, 1", v));
                let unit = Ty::Struct(String::new(), Vec::new());
                let r = self.coerce(&p, &Ty::Own(Box::new(unit.clone())), &Ty::Ref(Box::new(unit), *m))?;
                let mut acc = "undef".to_string();
                for k in 0..4 {
                    let f = self.b.val(format!("extractvalue %ref {}, {}", r, k));
                    let ft = ["ptr", "ptr", "i64", "ptr"][k];
                    acc = self.b.val(format!("insertvalue %iref {}, {} {}, {}", acc, ft, f, k));
                }
                return Ok(self.b.val(format!("insertvalue %iref {}, i64 {}, 4", acc, tid)));
            }
            (Ty::Int(a), Ty::Int(b)) => {
                return Ok(if a.bits() == b.bits() {
                    v.to_string()
                } else if a.bits() > b.bits() {
                    self.b.val(format!("trunc {} {} to {}", lf, v, lto))
                } else {
                    self.b.val(format!("{} {} {} to {}", ext(*a), lf, v, lto))
                });
            }
            _ => {}
        }
        if lf == lto {
            return Ok(v.to_string());
        }
        // ポインタ 1 つで表す enum と、そうでない形（中身の型が決まらなかった空の値など）のあいだは、種類の番号で作り直す
        if let (Ty::Enum(a, _), Ty::Enum(b2, _)) = (from, to) {
            let (nf, nt) = (self.m.niche(from)?, self.m.niche(to)?);
            if a == b2 && nf != nt {
                let tmp = self.b.alloca(&lf);
                self.b.op(format!("store {} {}, ptr {}", lf, v, tmp));
                let tag = self.m.enum_tag(&mut self.b, from, &tmp)?;
                let pl = self.m.enum_payload(&mut self.b, from, &tmp)?;
                let out = self.b.alloca(&lto);
                self.b.op(format!("store {} zeroinitializer, ptr {}", lto, out));
                let vs = self.m.variants(to)?;
                let fvs = self.m.variants(from)?;
                for (vi, (_, fs)) in vs.iter().enumerate() {
                    let is = self.b.val(format!("icmp eq i32 {}, {}", tag, vi));
                    let (_, skip) = self.b.if_true(&is);
                    if nt.is_none() {
                        let tp = self.b.val(format!("getelementptr {}, ptr {}, i32 0, i32 0", lto, out));
                        self.b.op(format!("store i32 {}, ptr {}", vi, tp));
                    }
                    // 中身は形が同じときだけ写す（違えばその種類の値は作られない: 中身の型が決まらなかった空の値）
                    if !fs.is_empty() && fvs[vi].1 == *fs {
                        let vt = self.m.variant_lt(fs)?;
                        let dpl = self.m.enum_payload(&mut self.b, to, &out)?;
                        let x = self.b.val(format!("load {}, ptr {}", vt, pl));
                        self.b.op(format!("store {} {}, ptr {}", vt, x, dpl));
                    }
                    self.b.br(&skip);
                    self.b.label(&skip);
                }
                return Ok(self.b.val(format!("load {}, ptr {}", lto, out)));
            }
        }
        // 型の引数だけ違う（中身の型が決まらなかった空の値など）: 大きさが入るなら、置き直して読む
        let (sf, _) = self.m.size_align(from)?;
        let (st, _) = self.m.size_align(to)?;
        if sf <= st && !self.m.needs_drop(from)? || matches!((from, to), (Ty::Enum(a, _), Ty::Enum(b, _)) if a == b) && sf <= st
            || matches!((from, to), (Ty::List(_), Ty::List(_)) | (Ty::Map(..), Ty::Map(..)))
        {
            let tmp = self.b.alloca(&lto);
            self.b.op(format!("store {} zeroinitializer, ptr {}", lto, tmp));
            self.b.op(format!("store {} {}, ptr {}", lf, v, tmp));
            return Ok(self.b.val(format!("load {}, ptr {}", lto, tmp)));
        }
        Err(format!("型の変換 {} → {}", from.show(), to.show()))
    }

    /// 区画を借りるときの世代。リストの中身の区画なら「借りられた」「借りられたことがある」の印を立てる（jw_rt.c の `LIST_SHARED`）
    fn borrow_gen(&mut self, blk: &str, list_block: bool) -> String {
        let hp = self.b.val(format!("getelementptr i8, ptr {}, i64 -8", blk));
        let g = self.b.val(format!("load atomic i64, ptr {} monotonic, align 8", hp));
        if !list_block {
            return g;
        }
        let g2 = self.b.val(format!("or i64 {}, {}", g, (1u64 << 62) | 2));
        self.b.op(format!("store atomic i64 {}, ptr {} monotonic, align 8", g2, hp));
        g2
    }

    fn make_ref(&mut self, addr: &str, blk: &str, gen: &str, site: &str) -> String {
        let a = self.b.val(format!("insertvalue %ref undef, ptr {}, 0", addr));
        let a = self.b.val(format!("insertvalue %ref {}, ptr {}, 1", a, blk));
        let a = self.b.val(format!("insertvalue %ref {}, i64 {}, 2", a, gen));
        self.b.val(format!("insertvalue %ref {}, ptr {}, 3", a, site))
    }

    /// 値を作って、`want` の型にする
    fn rvalue(&mut self, r: &Rvalue, want: &Ty) -> G<String> {
        let (v, t) = self.rvalue_raw(r, want)?;
        self.coerce(&v, &t, want)
    }

    fn rvalue_raw(&mut self, r: &Rvalue, want: &Ty) -> G<(String, Ty)> {
        match r {
            Rvalue::Use(o) => self.operand(o),
            Rvalue::Bin(op, a, b, span) => self.binary(*op, a, b, *span),
            Rvalue::Un(op, a, _) => {
                let (v, t) = self.operand(a)?;
                Ok(match (op, &t) {
                    (UnOp::Neg, Ty::Int(i)) => (self.b.val(format!("sub i{} 0, {}", i.bits(), v)), t),
                    (UnOp::Neg, Ty::Float) => (self.b.val(format!("fneg double {}", v)), t),
                    (UnOp::Not, Ty::Bool) => (self.b.val(format!("xor i1 {}, true", v)), t),
                    (UnOp::BitNot, Ty::Int(i)) => (self.b.val(format!("xor i{} {}, -1", i.bits(), v)), t),
                    _ => return Err(format!("{:?} {}", op, t.show())),
                })
            }
            Rvalue::Cast(a, to) => {
                let (v, t) = self.operand(a)?;
                Ok(match (&t, to) {
                    (Ty::Int(x), Ty::Int(_)) => (self.coerce(&v, &Ty::Int(*x), to)?, to.clone()),
                    (Ty::Int(x), Ty::Float) => {
                        let op = if x.signed() { "sitofp" } else { "uitofp" };
                        (self.b.val(format!("{} i{} {} to double", op, x.bits(), v)), Ty::Float)
                    }
                    (Ty::Float, Ty::Int(y)) => {
                        let w = self.b.val(format!("call i64 @jw_f2i(double {}, i32 {}, i32 {})", v, y.bits(), y.signed() as i32));
                        let r = if y.bits() < 64 { self.b.val(format!("trunc i64 {} to i{}", w, y.bits())) } else { w };
                        (r, to.clone())
                    }
                    _ => (v, t),
                })
            }
            Rvalue::Ref { place, what, span, .. } => {
                let pr = self.place(place, false)?;
                let site = self.m.site(*span, what);
                match pr.ty.clone() {
                    Ty::Own(x) => {
                        let iface = is_iface_ptr(&pr.ty);
                        let (raw, tid) = if iface {
                            let v = self.b.val(format!("load %iown, ptr {}", pr.ptr));
                            (self.b.val(format!("extractvalue %iown {}, 0", v)), Some(self.b.val(format!("extractvalue %iown {}, 1", v))))
                        } else {
                            (self.b.val(format!("load ptr, ptr {}", pr.ptr)), None)
                        };
                        // 写しなら、写しの借りた先・世代・位置をそのまま使う
                        let bits = self.b.val(format!("ptrtoint ptr {} to i64", raw));
                        let tag = self.b.val(format!("and i64 {}, 1", bits));
                        let isv = self.b.val(format!("icmp ne i64 {}, 0", tag));
                        let tgt = self.b.val(format!("call ptr @jw_view_target_nocheck(ptr {})", raw));
                        let target = self.b.val(format!("select i1 {}, ptr {}, ptr {}", isv, tgt, raw));
                        let vg = self.b.alloca("i64");
                        let vs = self.b.alloca("ptr");
                        let (_, no) = self.b.if_true(&isv);
                        let g1 = self.b.val(format!("call i64 @jw_view_gen(ptr {})", raw));
                        let s1 = self.b.val(format!("call ptr @jw_view_site(ptr {})", raw));
                        self.b.op(format!("store i64 {}, ptr {}", g1, vg));
                        self.b.op(format!("store ptr {}, ptr {}", s1, vs));
                        let join = self.b.l();
                        self.b.br(&join);
                        self.b.label(&no);
                        let hp = self.b.val(format!("getelementptr i8, ptr {}, i64 -8", raw));
                        let g2 = self.b.val(format!("load atomic i64, ptr {} monotonic, align 8", hp));
                        self.b.op(format!("store i64 {}, ptr {}", g2, vg));
                        self.b.op(format!("store ptr {}, ptr {}", site, vs));
                        self.b.br(&join);
                        self.b.label(&join);
                        let g = self.b.val(format!("load i64, ptr {}", vg));
                        let s = self.b.val(format!("load ptr, ptr {}", vs));
                        let rv = self.make_ref(&target, &target, &g, &s);
                        match tid {
                            Some(tid) => {
                                let mut acc = "undef".to_string();
                                for k in 0..4 {
                                    let f = self.b.val(format!("extractvalue %ref {}, {}", rv, k));
                                    let ft = ["ptr", "ptr", "i64", "ptr"][k];
                                    acc = self.b.val(format!("insertvalue %iref {}, {} {}, {}", acc, ft, f, k));
                                }
                                let v = self.b.val(format!("insertvalue %iref {}, i64 {}, 4", acc, tid));
                                Ok((v, Ty::Ref(x, false)))
                            }
                            None => Ok((rv, Ty::Ref(x, false))),
                        }
                    }
                    Ty::Ref(x, m) => {
                        let rt = if is_iface_ptr(&pr.ty) { "%iref" } else { "%ref" };
                        Ok((self.b.val(format!("load {}, ptr {}", rt, pr.ptr)), Ty::Ref(x, m)))
                    }
                    t => {
                        if pr.map {
                            let title = self.m.cstr("辞書の中の値は借りられません");
                            let l = self.m.cstr("辞書はキーを取り除くと値の場所が詰まるので、中の場所が変わる");
                            let s = self.m.site(*span, "");
                            let note = self.m.cstr("値を借りたいときは、値を own にしてください（`(own T)[K]`）");
                            self.b.op(format!("call void @jw_fail(ptr {}, ptr {}, ptr {}, ptr {})", title, s, l, note));
                            self.b.op("unreachable");
                            let after = self.b.l();
                            self.b.label(&after);
                        }
                        let blk = pr.block.clone().ok_or("区画にない変数を借りた")?;
                        let g = self.borrow_gen(&blk, pr.list_block);
                        let rv = self.make_ref(&pr.ptr, &blk, &g, &site);
                        if let Some(tid) = pr.tid {
                            let mut acc = "undef".to_string();
                            for k in 0..4 {
                                let f = self.b.val(format!("extractvalue %ref {}, {}", rv, k));
                                let ft = ["ptr", "ptr", "i64", "ptr"][k];
                                acc = self.b.val(format!("insertvalue %iref {}, {} {}, {}", acc, ft, f, k));
                            }
                            let v = self.b.val(format!("insertvalue %iref {}, i64 {}, 4", acc, tid));
                            return Ok((v, Ty::Ref(Box::new(t), false)));
                        }
                        Ok((rv, Ty::Ref(Box::new(t), false)))
                    }
                }
            }
            Rvalue::Own(a) => {
                let (v, t) = self.operand(a)?;
                let inner = match want {
                    Ty::Own(x) if !matches!(**x, Ty::Iface(_)) => (**x).clone(),
                    _ => t.clone(),
                };
                let v = self.coerce(&v, &t, &inner)?;
                let size = self.m.sizeof(&inner)?;
                let lt = self.m.lt(&inner)?;
                let p = self.b.val(format!("call ptr @jw_alloc(i64 {})", size));
                self.b.op(format!("store {} {}, ptr {}", lt, v, p));
                Ok((p, Ty::Own(Box::new(inner))))
            }
            Rvalue::Struct(name, ops) => {
                let st = match want {
                    Ty::Struct(n, _) if n == name => want.clone(),
                    _ => return Err(format!("構造体 {} の型が分からない", name)),
                };
                let fs = self.m.struct_fields(&st)?;
                let lt = self.m.lt(&st)?;
                let mut acc = "undef".to_string();
                for (i, o) in ops.iter().enumerate() {
                    let (v, t) = self.operand(o)?;
                    let v = self.coerce(&v, &t, &fs[i])?;
                    let ft = self.m.lt(&fs[i])?;
                    acc = self.b.val(format!("insertvalue {} {}, {} {}, {}", lt, acc, ft, v, i));
                }
                if ops.is_empty() {
                    acc = "zeroinitializer".to_string();
                }
                Ok((acc, st))
            }
            Rvalue::Enum(variant, ops, _) => {
                let et = match want {
                    Ty::Enum(..) => want.clone(),
                    _ => return Err(format!("enum の値 {} の型が分からない", variant)),
                };
                let mut vals = Vec::new();
                for o in ops {
                    vals.push(self.operand(o)?);
                }
                let v = self.make_enum(&et, variant, vals)?;
                Ok((v, et))
            }
            Rvalue::Take(place) => {
                let pr = self.place(place, false)?;
                let lt = self.m.lt(&pr.ty)?;
                Ok((self.b.val(format!("load {}, ptr {}", lt, pr.ptr)), pr.ty))
            }
            Rvalue::Array(items, list) => {
                let mut vals = Vec::new();
                for o in items {
                    vals.push(self.operand(o)?);
                }
                self.make_seq(want, vals, *list)
            }
            Rvalue::Repeat(x, n, list) => {
                let (v, t) = self.operand(x)?;
                let elem = match want {
                    Ty::Array(e, _) | Ty::List(e) => (**e).clone(),
                    _ => t.clone(),
                };
                let v = self.coerce(&v, &t, &elem)?;
                let lt = self.m.lt(&elem)?;
                let mut vals = Vec::new();
                if *n > 0 {
                    let tmp = self.b.alloca(&lt);
                    self.b.op(format!("store {} {}, ptr {}", lt, v, tmp));
                    for _ in 1..*n {
                        vals.push((self.read_copy(&tmp, &elem)?, elem.clone()));
                    }
                    vals.push((v, elem.clone()));
                }
                if *n > 64 {
                    return Err("大きな [x; n]".to_string());
                }
                self.make_seq(want, vals, *list)
            }
            Rvalue::Map(pairs, span) => {
                let Ty::Map(kt, vt) = want else { return Err("辞書の型が分からない".to_string()) };
                let vsize = self.m.sizeof(vt)?;
                let m = self.b.val(format!("call ptr @jw_map_new(i64 {}, i32 {})", vsize, key_kind(kt)));
                let vl = self.m.lt(vt)?;
                for (k, v) in pairs {
                    let (kv, kty) = self.operand(k)?;
                    let kv = self.coerce(&kv, &kty, kt)?;
                    let (vv, vty) = self.operand(v)?;
                    let vv = self.coerce(&vv, &vty, vt)?;
                    let k64 = self.key64(&kv, kt)?;
                    let i = self.b.val(format!("call i64 @jw_map_find(ptr {}, i64 {})", m, k64));
                    let missing = self.b.val(format!("icmp slt i64 {}, 0", i));
                    let yes = self.b.l();
                    let no = self.b.l();
                    let done = self.b.l();
                    self.b.op(format!("br i1 {}, label %{}, label %{}", missing, yes, no));
                    self.b.label(&yes);
                    let j = self.b.val(format!("call i64 @jw_map_add(ptr {}, i64 {})", m, k64));
                    let p = self.b.val(format!("call ptr @jw_map_val(ptr {}, i64 {})", m, j));
                    self.b.op(format!("store {} {}, ptr {}", vl, vv, p));
                    self.b.br(&done);
                    self.b.label(&no);
                    // 同じキー: 後の値が残り、前の値は捨てる
                    self.release_key(&kv, kt);
                    let p = self.b.val(format!("call ptr @jw_map_val(ptr {}, i64 {})", m, i));
                    let old = self.b.val(format!("load {}, ptr {}", vl, p));
                    self.b.op(format!("store {} {}, ptr {}", vl, vv, p));
                    self.discard(&old, vt, *span, "辞書の同じキーに後から入れた値で置き換えられた")?;
                    self.b.br(&done);
                    self.b.label(&done);
                }
                Ok((m, want.clone()))
            }
            Rvalue::SeqOp { name, place, args, span, arg_span } => self.seq_op(name, place, args, *span, *arg_span, want),
            Rvalue::Items { place, what, span } => {
                let pr = self.place(place, false)?;
                let site = self.m.site(*span, what);
                let (elem, n, get): (Ty, String, ElemPtr) = match pr.ty.clone() {
                    Ty::Array(x, n) => {
                        let at = self.m.lt(&pr.ty)?;
                        let base = pr.ptr.clone();
                        (*x, n.to_string(), Box::new(move |b: &mut B, i: &str| b.val(format!("getelementptr {}, ptr {}, i64 0, i64 {}", at, base, i))))
                    }
                    Ty::List(x) => {
                        let lp = self.b.val(format!("getelementptr %list, ptr {}, i32 0, i32 1", pr.ptr));
                        let len = self.b.val(format!("load i64, ptr {}", lp));
                        let data = self.b.val(format!("load ptr, ptr {}", pr.ptr));
                        let xt = self.m.lt(&x)?;
                        (*x, len, Box::new(move |b: &mut B, i: &str| b.val(format!("getelementptr {}, ptr {}, i64 {}", xt, data, i))))
                    }
                    Ty::Map(_, v) => {
                        let m = self.b.val(format!("load ptr, ptr {}", pr.ptr));
                        let len = self.b.val(format!("call i64 @jw_map_len(ptr {})", m));
                        (*v, len, Box::new(move |b: &mut B, i: &str| b.val(format!("call ptr @jw_map_val(ptr {}, i64 {})", m, i))))
                    }
                    t => return Err(format!("{} の for", t.show())),
                };
                let out = self.b.alloca("%list");
                self.b.op(format!("store %list zeroinitializer, ptr {}", out));
                let size = self.m.sizeof(&elem)?;
                self.b.op(format!("call void @jw_list_reserve(ptr {}, i64 {}, i64 {})", out, size, n));
                let view = self.m.helper(H::View, &elem);
                let (i, e) = self.b.loop_begin(&n);
                let src = get(&mut self.b, &i);
                let dst = self.b.val(format!("call ptr @jw_list_push_slot(ptr {}, i64 {}, ptr null)", out, size));
                self.b.op(format!("call void {}(ptr {}, ptr {}, ptr {})", view, src, dst, site));
                self.b.loop_end(e);
                Ok((self.b.val(format!("load %list, ptr {}", out)), Ty::List(Box::new(elem))))
            }
            Rvalue::Keys(place) => {
                let pr = self.place(place, false)?;
                let out = self.b.alloca("%list");
                self.b.op(format!("store %list zeroinitializer, ptr {}", out));
                match pr.ty.clone() {
                    Ty::Map(k, _) => {
                        let m = self.b.val(format!("load ptr, ptr {}", pr.ptr));
                        let len = self.b.val(format!("call i64 @jw_map_len(ptr {})", m));
                        let size = self.m.sizeof(&k)?;
                        self.b.op(format!("call void @jw_list_reserve(ptr {}, i64 {}, i64 {})", out, size, len));
                        let (i, e) = self.b.loop_begin(&len);
                        let kv = self.b.val(format!("call i64 @jw_map_key(ptr {}, i64 {})", m, i));
                        if matches!(*k, Ty::Str) {
                            let kp = self.b.val(format!("inttoptr i64 {} to ptr", kv));
                            self.b.op(format!("call void @jw_str_retain(ptr {})", kp));
                        }
                        let dst = self.b.val(format!("call ptr @jw_list_push_slot(ptr {}, i64 {}, ptr null)", out, size));
                        store_key(self.m, &mut self.b, &k, &kv, &dst)?;
                        self.b.loop_end(e);
                        Ok((self.b.val(format!("load %list, ptr {}", out)), Ty::List(k)))
                    }
                    t => {
                        let len = match &t {
                            Ty::Array(_, n) => n.to_string(),
                            Ty::List(_) => {
                                let lp = self.b.val(format!("getelementptr %list, ptr {}, i32 0, i32 1", pr.ptr));
                                self.b.val(format!("load i64, ptr {}", lp))
                            }
                            _ => return Err(format!("{} の for (i, x)", t.show())),
                        };
                        self.b.op(format!("call void @jw_list_reserve(ptr {}, i64 8, i64 {})", out, len));
                        let (i, e) = self.b.loop_begin(&len);
                        let dst = self.b.val(format!("call ptr @jw_list_push_slot(ptr {}, i64 8, ptr null)", out));
                        self.b.op(format!("store i64 {}, ptr {}", i, dst));
                        self.b.loop_end(e);
                        Ok((self.b.val(format!("load %list, ptr {}", out)), Ty::List(Box::new(Ty::Int(IntTy::I64)))))
                    }
                }
            }
            Rvalue::Call(fi, ops, span) => {
                let callee = &self.m.prog.fns[*fi];
                let mut args = Vec::new();
                for (i, o) in ops.iter().enumerate() {
                    let (v, t) = self.operand(o)?;
                    let pt = callee.locals[i].ty.clone();
                    let v = self.coerce(&v, &t, &pt)?;
                    args.push(format!("{} {}", self.m.lt(&pt)?, v));
                }
                let ret = callee.ret.clone();
                let v = self.call(&format!("@f{}", fi), &ret, &args, *span)?;
                Ok((v, ret))
            }
            Rvalue::CallDyn { table, this, what, args, span, .. } => {
                let mut vals = Vec::new();
                for o in args {
                    vals.push(self.operand(o)?);
                }
                let pr = self.place(this, false)?;
                let tid = pr.tid.clone().ok_or("interface でない this")?;
                let blk = pr.block.clone().ok_or("区画にない this")?;
                let g = self.borrow_gen(&blk, pr.list_block);
                let site = self.m.site(this.span, what);
                let rv = self.make_ref(&pr.ptr, &blk, &g, &site);
                let ret = want.clone();
                let rl = self.m.lt(&ret)?;
                let out = self.b.alloca(&rl);
                let done = self.b.l();
                let mut cases = Vec::new();
                let mut bodies = Vec::new();
                for (tidx, sty) in self.m.tids.clone().iter().enumerate() {
                    let Ty::Struct(sn, _) = sty else { continue };
                    let Some((_, fi)) = table.iter().find(|(n, _)| n == sn) else { continue };
                    let l = self.b.l();
                    cases.push(format!("i64 {}, label %{}", tidx, l));
                    bodies.push((l, *fi));
                }
                let bad = self.b.l();
                self.b.op(format!("switch i64 {}, label %{} [ {} ]", tid, bad, cases.join(" ")));
                for (l, fi) in bodies {
                    self.b.label(&l);
                    let callee = &self.m.prog.fns[fi];
                    let mut a = vec![format!("%ref {}", rv)];
                    for (i, (v, t)) in vals.iter().enumerate() {
                        let pt = callee.locals[i + 1].ty.clone();
                        let v = self.coerce(v, t, &pt)?;
                        a.push(format!("{} {}", self.m.lt(&pt)?, v));
                    }
                    let cr = callee.ret.clone();
                    let v = self.call(&format!("@f{}", fi), &cr, &a, *span)?;
                    let v = self.coerce(&v, &cr, &ret)?;
                    self.b.op(format!("store {} {}, ptr {}", rl, v, out));
                    self.b.br(&done);
                }
                self.b.label(&bad);
                let msg = self.m.cstr("interface のメソッドが見つからない");
                self.b.op(format!("call void @jw_bug(ptr {})", msg));
                self.b.op("unreachable");
                self.b.label(&done);
                Ok((self.b.val(format!("load {}, ptr {}", rl, out)), ret))
            }
            Rvalue::Closure(fi, ops) => {
                let mut vals = Vec::new();
                for o in ops {
                    vals.push(self.operand(o)?);
                }
                let ctys: Vec<Ty> = vals.iter().map(|(_, t)| t.clone()).collect();
                let (tramp, vt) = self.m.closure_fns(*fi, &ctys)?;
                let fv = self.b.val(format!("insertvalue %fnv undef, ptr {}, 0", tramp));
                if vals.is_empty() {
                    let v = self.b.val(format!("insertvalue %fnv {}, ptr null, 1", fv));
                    return Ok((v, want.clone()));
                }
                // 取り込んだ値の入れ物（頭 24 バイト: 数・表・印）
                let caps_t = self.m.variant_lt(&ctys)?;
                let size = format!("ptrtoint (ptr getelementptr ({}, ptr null, i32 1) to i64)", caps_t);
                let total = self.b.val(format!("add i64 24, {}", size));
                let env = self.b.val(format!("call ptr @jw_env_new(i64 {}, ptr {})", total, vt));
                let cp = self.b.val(format!("getelementptr i8, ptr {}, i64 24", env));
                for (i, (v, t)) in vals.iter().enumerate() {
                    let lt = self.m.lt(t)?;
                    let dst = self.b.val(format!("getelementptr {}, ptr {}, i32 0, i32 {}", caps_t, cp, i));
                    self.b.op(format!("store {} {}, ptr {}", lt, v, dst));
                }
                let flags = self.m.env_flags(&mut self.b, &cp, &ctys, &caps_t)?;
                let fp = self.b.val(format!("getelementptr i8, ptr {}, i64 16", env));
                self.b.op(format!("store i64 {}, ptr {}", flags, fp));
                let v = self.b.val(format!("insertvalue %fnv {}, ptr {}, 1", fv, env));
                Ok((v, want.clone()))
            }
            Rvalue::CallValue(fo, ops, span) => {
                let mut vals = Vec::new();
                for o in ops {
                    vals.push(self.operand(o)?);
                }
                let (fv, ft) = self.operand(fo)?;
                let v = self.call_value(&fv, &ft, vals, *span)?;
                // 呼ぶために複製した関数の値を捨てる
                self.discard(&fv, &ft, *span, "呼び終わった関数の値")?;
                let Ty::Fn(_, ret) = ft else { return Err("関数でない値の呼び出し".to_string()) };
                Ok((v, *ret))
            }
            Rvalue::CopyOut(place, what, span) => {
                let pr = self.place(place, false)?;
                if self.m.can_hazard(&pr.ty)? {
                    let h = self.m.helper(H::Hazard, &pr.ty);
                    let bad = self.b.val(format!("call i1 {}(ptr {})", h, pr.ptr));
                    let (_, ok) = self.b.if_true(&bad);
                    let title = self.m.cstr(&format!("{}にある {} は取り出せません", what, type_name(&pr.ty)));
                    let l = self.m.cstr("所有権をここから動かすことはできない");
                    let s = self.m.site(*span, "");
                    let note = self.m.cstr(copy_out_note(what));
                    self.b.op(format!("call void @jw_fail(ptr {}, ptr {}, ptr {}, ptr {})", title, s, l, note));
                    self.b.op("unreachable");
                    self.b.label(&ok);
                }
                let v = self.read_place(&pr)?;
                Ok((v, pr.ty))
            }
            Rvalue::Show(o, temp, span) => {
                let use_site = self.m.site(*span, "");
                match (o, temp) {
                    (Operand::Copy(p), false) => {
                        let sb = self.b.val("call ptr @jw_sb_new()");
                        let pr = self.place(p, false)?;
                        let f = self.m.helper(H::Show, &pr.ty);
                        self.b.op(format!("call void {}(ptr {}, ptr {}, i64 0, ptr {})", f, pr.ptr, sb, use_site));
                        Ok((self.b.val(format!("call ptr @jw_sb_finish(ptr {})", sb)), Ty::Str))
                    }
                    _ => {
                        let (v, t) = self.operand(o)?;
                        // 整数は組み立てずにそのまま文字列にする
                        if let Ty::Int(i) = t {
                            let w = widen(&mut self.b, &v, i);
                            let r = self.b.val(format!("call ptr @jw_str_int(i64 {}, i32 {})", w, !i.signed() as i32));
                            return Ok((r, Ty::Str));
                        }
                        let sb = self.b.val("call ptr @jw_sb_new()");
                        let lt = self.m.lt(&t)?;
                        let tmp = self.b.alloca(&lt);
                        self.b.op(format!("store {} {}, ptr {}", lt, v, tmp));
                        let f = self.m.helper(H::Show, &t);
                        self.b.op(format!("call void {}(ptr {}, ptr {}, i64 0, ptr {})", f, tmp, sb, use_site));
                        if *temp || matches!(o, Operand::Copy(_)) {
                            let why = if *temp { "使い終わった一時的な値" } else { "表示のための写し" };
                            let v2 = self.b.val(format!("load {}, ptr {}", lt, tmp));
                            self.discard(&v2, &t, *span, why)?;
                        }
                        Ok((self.b.val(format!("call ptr @jw_sb_finish(ptr {})", sb)), Ty::Str))
                    }
                }
            }
            Rvalue::Builtin(name, ops, span) => self.builtin(name, ops, *span, want),
        }
    }

    /// 呼び出し（深さを確かめる）
    fn call(&mut self, f: &str, ret: &Ty, args: &[String], span: Span) -> G<String> {
        let d = self.depth.clone();
        let deep = self.b.val(format!("icmp sge i64 {}, {}", d, MAX_DEPTH));
        let (_, ok) = self.b.if_true(&deep);
        let s = self.m.site(span, "");
        self.b.op(format!("call void @jw_fail_depth(ptr {})", s));
        self.b.op("unreachable");
        self.b.label(&ok);
        let d1 = self.b.val(format!("add i64 {}, 1", d));
        let rl = self.m.lt(ret)?;
        let mut all = vec![format!("i64 {}", d1)];
        all.extend(args.iter().cloned());
        Ok(self.b.val(format!("call {} {}({})", rl, f, all.join(", "))))
    }

    /// 関数の値を呼ぶ（深さを確かめる）。引数は関数の型の引数の型にそろえる
    fn call_value(&mut self, fv: &str, ft: &Ty, vals: Vec<(String, Ty)>, span: Span) -> G<String> {
        let Ty::Fn(ps, ret) = ft else { return Err(format!("関数でない値 {}", ft.show())) };
        let code = self.b.val(format!("extractvalue %fnv {}, 0", fv));
        let env = self.b.val(format!("extractvalue %fnv {}, 1", fv));
        let site = self.m.site(span, "クロージャに取り込んだ値");
        let mut args = vec![format!("ptr {}", env), format!("ptr {}", site)];
        for (i, (v, t)) in vals.into_iter().enumerate() {
            let pt = ps.get(i).cloned().ok_or("引数の数が違う")?;
            let v = self.coerce(&v, &t, &pt)?;
            args.push(format!("{} {}", self.m.lt(&pt)?, v));
        }
        self.call(&code, ret, &args, span)
    }

    /// 整数の `sort()` / `sort_unstable()` は比べる関数なしの `jw_sort_int`（比べ方が見えないので速い手順でよい）。書き出したら true
    fn sort_int(&mut self, data: &str, n: &str, x: &Ty) -> bool {
        match x {
            Ty::Int(i) => {
                self.b.op(format!("call void @jw_sort_int(ptr {}, i64 {}, i32 {}, i32 {})", data, n, i.bits() / 8, i.signed() as i32));
                true
            }
            _ => false,
        }
    }

    /// `sort_by(f)`（interp の `sort_list`）。要素の読むだけの写しを作って比べる関数に渡し、並べ終わった順に要素を動かす
    fn sort_by(&mut self, data: &str, n: &str, x: &Ty, f: (String, Ty), stable: bool, span: Span) -> G<()> {
        let (fv, ft) = f;
        let Ty::Fn(ps, _) = &ft else { return Err("sort_by の関数の型".to_string()) };
        let pa = ps.first().cloned().ok_or("sort_by の関数の引数")?;
        let pl = self.m.lt(&pa)?;
        let xl = self.m.lt(x)?;
        let xsize = self.m.sizeof(x)?;
        let psize = self.m.sizeof(&pa)?;
        let site = self.m.site(span, "要素");
        // 比べる関数に渡す値（要素の写し。ムーブする写しは区画に置いて ref で渡す）
        let bytes = self.b.val(format!("mul i64 {}, {}", n, psize));
        let bytes = self.b.val(format!("add i64 {}, 8", bytes));
        let args = self.b.val(format!("call ptr @malloc(i64 {})", bytes));
        let boxed = moves(self.m, x)? && !matches!(x, Ty::Own(_));
        let view = self.m.helper(H::View, x);
        let (i, e) = self.b.loop_begin(n);
        let src = self.b.val(format!("getelementptr {}, ptr {}, i64 {}", xl, data, i));
        let dst = self.b.val(format!("getelementptr {}, ptr {}, i64 {}", pl, args, i));
        if boxed {
            let blk = self.b.val(format!("call ptr @jw_alloc(i64 {})", xsize));
            self.b.op(format!("call void {}(ptr {}, ptr {}, ptr {})", view, src, blk, site));
            let hp = self.b.val(format!("getelementptr i8, ptr {}, i64 -8", blk));
            let g = self.b.val(format!("load atomic i64, ptr {} monotonic, align 8", hp));
            let r = self.make_ref(&blk, &blk, &g, &site);
            self.b.op(format!("store %ref {}, ptr {}", r, dst));
        } else {
            let tmp = self.b.alloca(&xl);
            self.b.op(format!("call void {}(ptr {}, ptr {}, ptr {})", view, src, tmp, site));
            let v = self.b.val(format!("load {}, ptr {}", xl, tmp));
            let v = self.coerce(&v, x, &pa)?;
            self.b.op(format!("store {} {}, ptr {}", pl, v, dst));
        }
        self.b.loop_end(e);
        // 添字の並びを比べる関数で並べる
        let ob = self.b.val(format!("mul i64 {}, 8", n));
        let ob = self.b.val(format!("add i64 {}, 8", ob));
        let order = self.b.val(format!("call ptr @malloc(i64 {})", ob));
        let (i, e) = self.b.loop_begin(n);
        let op = self.b.val(format!("getelementptr i64, ptr {}, i64 {}", order, i));
        self.b.op(format!("store i64 {}, ptr {}", i, op));
        self.b.loop_end(e);
        let cmp = self.sort_cmp(&ft, &pa, span)?;
        let ctx = self.b.alloca("{ %fnv, ptr }");
        let c1 = self.b.val(format!("insertvalue {{ %fnv, ptr }} undef, %fnv {}, 0", fv));
        let c2 = self.b.val(format!("insertvalue {{ %fnv, ptr }} {}, ptr {}, 1", c1, args));
        self.b.op(format!("store {{ %fnv, ptr }} {}, ptr {}", c2, ctx));
        // 比べる関数は実行時ライブラリから呼ばれるので、深さはグローバル変数で渡す
        self.b.op(format!("store i64 {}, ptr @jw_depth", self.depth));
        let f = if stable { "jw_sort" } else { "jw_sort_unstable" };
        self.b.op(format!("call void @{}(ptr {}, i64 {}, i64 8, ptr {}, ptr {})", f, order, n, cmp, ctx));
        // 写しを片づける
        let fin = self.m.site(span, "sort_by の比べ終わった写し");
        let (i, e) = self.b.loop_begin(n);
        let ap = self.b.val(format!("getelementptr {}, ptr {}, i64 {}", pl, args, i));
        if boxed {
            let r = self.b.val(format!("load %ref, ptr {}", ap));
            let blk = self.b.val(format!("extractvalue %ref {}, 1", r));
            if self.m.needs_drop(x)? {
                let d = self.m.helper(H::Drop, x);
                self.b.op(format!("call void {}(ptr {}, ptr {})", d, blk, fin));
            }
            self.b.op(format!("call void @jw_free(ptr {}, i64 {}, ptr {})", blk, xsize, fin));
        } else if self.m.needs_drop(&pa)? {
            let d = self.m.helper(H::Drop, &pa);
            self.b.op(format!("call void {}(ptr {}, ptr {})", d, ap, fin));
        }
        self.b.loop_end(e);
        self.b.op(format!("call void @free(ptr {})", args));
        self.discard(&fv, &ft, span, "sort_by が使い終わった関数")?;
        self.b.op(format!("call void @jw_permute(ptr {}, i64 {}, i64 {}, ptr {})", data, n, xsize, order));
        self.b.op(format!("call void @free(ptr {})", order));
        Ok(())
    }

    /// sort_by の比べる関数（`i32 (ptr 添字a, ptr 添字b, ptr {関数の値, 写しの並び})`）
    fn sort_cmp(&mut self, ft: &Ty, pa: &Ty, span: Span) -> G<String> {
        let name = format!("@sortcmp{}", self.m.fresh());
        let saved = std::mem::take(&mut self.b);
        let depth = self.b.val("load i64, ptr @jw_depth");
        let saved_depth = std::mem::replace(&mut self.depth, depth);
        let pl = self.m.lt(pa)?;
        let ctx = "%env";
        let fv = self.b.val(format!("load %fnv, ptr {}", ctx));
        let ap = self.b.val(format!("getelementptr {{ %fnv, ptr }}, ptr {}, i32 0, i32 1", ctx));
        let args = self.b.val(format!("load ptr, ptr {}", ap));
        let mut vals = Vec::new();
        for q in ["%p", "%q"] {
            let k = self.b.val(format!("load i64, ptr {}", q));
            let p = self.b.val(format!("getelementptr {}, ptr {}, i64 {}", pl, args, k));
            vals.push((self.read_copy(&p, pa)?, pa.clone()));
        }
        let r = self.call_value(&fv, ft, vals, span)?;
        let r = self.b.val(format!("zext i1 {} to i32", r));
        let body = std::mem::replace(&mut self.b, saved);
        self.depth = saved_depth;
        let _ = write!(
            self.m.code,
            "define internal i32 {}(ptr %p, ptr %q, ptr %env) {{\nentry:\n{}{}  ret i32 {}\n}}\n\n",
            name, body.allocas, body.text, r
        );
        Ok(name)
    }

    /// `Result<T, str>` を作る。`ok` が 0 でなければ `Ok(ok_val)`、0 なら `Err(*err)`（err は文字列を指す場所）
    fn result_of(&mut self, want: &Ty, ok: &str, err: &str, ok_val: impl FnOnce(&mut Self) -> G<(String, Ty)>) -> G<(String, Ty)> {
        let lt = self.m.lt(want)?;
        let res = self.b.alloca(&lt);
        let failed = self.b.val(format!("icmp eq i32 {}, 0", ok));
        let (_, fine) = self.b.if_true(&failed);
        let msg = self.b.val(format!("load ptr, ptr {}", err));
        let ev = self.make_enum(want, "Err", vec![(msg, Ty::Str)])?;
        self.b.op(format!("store {} {}, ptr {}", lt, ev, res));
        let done = self.b.l();
        self.b.br(&done);
        self.b.label(&fine);
        let v = ok_val(self)?;
        let ov = self.make_enum(want, "Ok", vec![v])?;
        self.b.op(format!("store {} {}, ptr {}", lt, ov, res));
        self.b.br(&done);
        self.b.label(&done);
        Ok((self.b.val(format!("load {}, ptr {}", lt, res)), want.clone()))
    }

    fn make_enum(&mut self, et: &Ty, variant: &str, vals: Vec<(String, Ty)>) -> G<String> {
        let vs = self.m.variants(et)?;
        let vi = vs.iter().position(|(n, _)| n == variant).ok_or_else(|| format!("enum の種類 {} がない", variant))?;
        let fs = vs[vi].1.clone();
        let lt = self.m.lt(et)?;
        let tmp = self.b.alloca(&lt);
        self.b.op(format!("store {} zeroinitializer, ptr {}", lt, tmp));
        if self.m.niche(et)?.is_none() {
            let tp = self.b.val(format!("getelementptr {}, ptr {}, i32 0, i32 0", lt, tmp));
            self.b.op(format!("store i32 {}, ptr {}", vi, tp));
        }
        if !fs.is_empty() {
            let vt = self.m.variant_lt(&fs)?;
            let pl = self.m.enum_payload(&mut self.b, et, &tmp)?;
            for (i, (v, t)) in vals.into_iter().enumerate() {
                let v = self.coerce(&v, &t, &fs[i])?;
                let ft = self.m.lt(&fs[i])?;
                let fp = self.b.val(format!("getelementptr {}, ptr {}, i32 0, i32 {}", vt, pl, i));
                self.b.op(format!("store {} {}, ptr {}", ft, v, fp));
            }
        }
        Ok(self.b.val(format!("load {}, ptr {}", lt, tmp)))
    }

    /// 配列（list が false）・リストを作る
    fn make_seq(&mut self, want: &Ty, vals: Vec<(String, Ty)>, list: bool) -> G<(String, Ty)> {
        let elem = match want {
            Ty::Array(e, _) | Ty::List(e) => (**e).clone(),
            _ => match vals.first() {
                Some((_, t)) => t.clone(),
                None => return Err("要素の型が分からない".to_string()),
            },
        };
        let el = self.m.lt(&elem)?;
        if !list {
            let at = format!("[{} x {}]", vals.len(), el);
            let mut acc = if vals.is_empty() { "zeroinitializer".to_string() } else { "undef".to_string() };
            for (i, (v, t)) in vals.iter().enumerate() {
                let v = self.coerce(v, t, &elem)?;
                acc = self.b.val(format!("insertvalue {} {}, {} {}, {}", at, acc, el, v, i));
            }
            return Ok((acc, Ty::Array(Box::new(elem), vals.len() as u64)));
        }
        let out = self.b.alloca("%list");
        self.b.op(format!("store %list zeroinitializer, ptr {}", out));
        let size = self.m.sizeof(&elem)?;
        self.b.op(format!("call void @jw_list_reserve(ptr {}, i64 {}, i64 {})", out, size, vals.len()));
        for (v, t) in vals {
            let v = self.coerce(&v, &t, &elem)?;
            let dst = self.b.val(format!("call ptr @jw_list_push_slot(ptr {}, i64 {}, ptr null)", out, size));
            self.b.op(format!("store {} {}, ptr {}", el, v, dst));
        }
        Ok((self.b.val(format!("load %list, ptr {}", out)), Ty::List(Box::new(elem))))
    }

    /// `x << y` / `x >> y`（interp の `binary` と同じ: 幅以上なら 0 か -1、負ならエラー）
    fn shift(&mut self, op: BinOp, x: &str, i: IntTy, y: &str, n: IntTy, span: Span) -> G<(String, Ty)> {
        let lt = format!("i{}", i.bits());
        let y64 = widen(&mut self.b, y, n);
        if n.signed() {
            let neg = self.b.val(format!("icmp slt i64 {}, 0", y64));
            let (_, ok) = self.b.if_true(&neg);
            let title = self.m.cstr("負の数だけはずらせません");
            let l = self.m.cstr("右の数が負です");
            let s = self.m.site(span, "");
            self.b.op(format!("call void @jw_fail(ptr {}, ptr {}, ptr {}, ptr null)", title, s, l));
            self.b.op("unreachable");
            self.b.label(&ok);
        }
        let big = self.b.val(format!("icmp uge i64 {}, {}", y64, i.bits()));
        let safe = self.b.val(format!("select i1 {}, i64 {}, i64 {}", big, i.bits() - 1, y64));
        let amt = if i.bits() == 64 { safe } else { self.b.val(format!("trunc i64 {} to {}", safe, lt)) };
        let v = match op {
            BinOp::Shl => {
                let s = self.b.val(format!("shl {} {}, {}", lt, x, amt));
                self.b.val(format!("select i1 {}, {} 0, {} {}", big, lt, lt, s))
            }
            // 幅以上のときは幅 - 1 だけずらしたもの（符号つきなら 0 か -1、符号なしなら 0）
            _ if i.signed() => self.b.val(format!("ashr {} {}, {}", lt, x, amt)),
            _ => {
                let s = self.b.val(format!("lshr {} {}, {}", lt, x, amt));
                self.b.val(format!("select i1 {}, {} 0, {} {}", big, lt, lt, s))
            }
        };
        Ok((v, Ty::Int(i)))
    }

    fn binary(&mut self, op: BinOp, a: &Operand, b: &Operand, span: Span) -> G<(String, Ty)> {
        let (x, t) = self.operand(a)?;
        let (y, tb) = self.operand(b)?;
        if let (BinOp::Shl | BinOp::Shr, Ty::Int(i), Ty::Int(n)) = (op, &t, &tb) {
            return self.shift(op, &x, *i, &y, *n, span);
        }
        let y = self.coerce(&y, &tb, &t)?;
        let cmp = |op: BinOp| match op {
            BinOp::Eq => "eq",
            BinOp::Ne => "ne",
            BinOp::Lt => "lt",
            BinOp::Le => "le",
            BinOp::Gt => "gt",
            _ => "ge",
        };
        Ok(match &t {
            Ty::Int(i) => {
                let lt = format!("i{}", i.bits());
                match op {
                    BinOp::Add | BinOp::Sub | BinOp::Mul => {
                        let o = match op {
                            BinOp::Add => "add",
                            BinOp::Sub => "sub",
                            _ => "mul",
                        };
                        (self.b.val(format!("{} {} {}, {}", o, lt, x, y)), t.clone())
                    }
                    BinOp::Div | BinOp::Rem => {
                        let z = self.b.val(format!("icmp eq {} {}, 0", lt, y));
                        let (_, ok) = self.b.if_true(&z);
                        let title = self.m.cstr("0 で割りました");
                        let l = self.m.cstr("右辺が 0 です");
                        let s = self.m.site(span, "");
                        self.b.op(format!("call void @jw_fail(ptr {}, ptr {}, ptr {}, ptr null)", title, s, l));
                        self.b.op("unreachable");
                        self.b.label(&ok);
                        let div = op == BinOp::Div;
                        if i.signed() {
                            // -1 で割るときは、あふれ（LLVM では未定義）を避けて計算する
                            let m1 = self.b.val(format!("icmp eq {} {}, -1", lt, y));
                            let safe = self.b.val(format!("select i1 {}, {} 1, {} {}", m1, lt, lt, y));
                            let q = self.b.val(format!("{} {} {}, {}", if div { "sdiv" } else { "srem" }, lt, x, safe));
                            let alt = if div { self.b.val(format!("sub {} 0, {}", lt, x)) } else { "0".to_string() };
                            (self.b.val(format!("select i1 {}, {} {}, {} {}", m1, lt, alt, lt, q)), t.clone())
                        } else {
                            (self.b.val(format!("{} {} {}, {}", if div { "udiv" } else { "urem" }, lt, x, y)), t.clone())
                        }
                    }
                    BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor => {
                        let o = match op {
                            BinOp::BitAnd => "and",
                            BinOp::BitOr => "or",
                            _ => "xor",
                        };
                        (self.b.val(format!("{} {} {}, {}", o, lt, x, y)), t.clone())
                    }
                    BinOp::Eq | BinOp::Ne => (self.b.val(format!("icmp {} {} {}, {}", cmp(op), lt, x, y)), Ty::Bool),
                    BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                        let p = format!("{}{}", if i.signed() { "s" } else { "u" }, cmp(op));
                        (self.b.val(format!("icmp {} {} {}, {}", p, lt, x, y)), Ty::Bool)
                    }
                    _ => return Err(format!("{:?} の整数", op)),
                }
            }
            Ty::Float => match op {
                BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem => {
                    let o = match op {
                        BinOp::Add => "fadd",
                        BinOp::Sub => "fsub",
                        BinOp::Mul => "fmul",
                        BinOp::Div => "fdiv",
                        _ => "frem",
                    };
                    (self.b.val(format!("{} double {}, {}", o, x, y)), Ty::Float)
                }
                BinOp::Ne => (self.b.val(format!("fcmp une double {}, {}", x, y)), Ty::Bool),
                _ => (self.b.val(format!("fcmp o{} double {}, {}", cmp(op), x, y)), Ty::Bool),
            },
            Ty::Bool => match op {
                BinOp::Eq | BinOp::Ne => (self.b.val(format!("icmp {} i1 {}, {}", cmp(op), x, y)), Ty::Bool),
                _ => return Err(format!("{:?} の真偽値", op)),
            },
            Ty::Unit => (if op == BinOp::Eq { "true" } else { "false" }.to_string(), Ty::Bool),
            Ty::Str => {
                let r = match op {
                    BinOp::Add => (self.b.val(format!("call ptr @jw_str_concat(ptr {}, ptr {})", x, y)), Ty::Str),
                    BinOp::Eq | BinOp::Ne => {
                        let e = self.b.val(format!("call i32 @jw_str_eq(ptr {}, ptr {})", x, y));
                        (self.b.val(format!("icmp {} i32 {}, 0", if op == BinOp::Eq { "ne" } else { "eq" }, e)), Ty::Bool)
                    }
                    _ => {
                        let c = self.b.val(format!("call i32 @jw_str_cmp(ptr {}, ptr {})", x, y));
                        (self.b.val(format!("icmp s{} i32 {}, 0", cmp(op), c)), Ty::Bool)
                    }
                };
                self.b.op(format!("call void @jw_str_release(ptr {})", x));
                self.b.op(format!("call void @jw_str_release(ptr {})", y));
                r
            }
            _ => return Err(format!("{:?} の {}", op, t.show())),
        })
    }

    fn builtin(&mut self, name: &str, ops: &[Operand], span: Span, want: &Ty) -> G<(String, Ty)> {
        let mut vals = Vec::new();
        // チャンネルのメソッドは、場所のチャンネルを複製せずに読む（数を増やすと、スレッドのあいだで取り合いになる）
        let borrowed = name.starts_with("chan.") && matches!(ops.first(), Some(Operand::Copy(_)));
        // `append` は足す先の変数の場所を渡す（数が 1 ならその場で足して、場所の値を置き換える）
        if let ("append", [Operand::Copy(p), b]) = (name, ops) {
            let pr = self.place(p, false)?;
            let (bv, _) = self.operand(b)?;
            let r = self.b.val(format!("call ptr @jw_str_append(ptr {}, ptr {})", pr.ptr, bv));
            self.b.op(format!("call void @jw_str_release(ptr {})", bv));
            return Ok((r, Ty::Str));
        }
        for (i, o) in ops.iter().enumerate() {
            match o {
                Operand::Copy(p) if i == 0 && borrowed => {
                    let pr = self.place(p, false)?;
                    vals.push((self.b.val(format!("load ptr, ptr {}", pr.ptr)), pr.ty));
                }
                _ => vals.push(self.operand(o)?),
            }
        }
        let release = |me: &mut Self, vals: &[(String, Ty)]| {
            for (v, t) in vals {
                if matches!(t, Ty::Str) {
                    me.b.op(format!("call void @jw_str_release(ptr {})", v));
                }
            }
        };
        match name {
            "spawn" => {
                let fi: usize = vals[0].0.parse().map_err(|_| "spawn の関数の番号")?;
                let caps: Vec<(String, Ty)> = vals[1..].to_vec();
                let ctys: Vec<Ty> = caps.iter().map(|(_, t)| t.clone()).collect();
                let args_t = self.m.variant_lt(&ctys)?;
                let size = format!("ptrtoint (ptr getelementptr ({}, ptr null, i32 1) to i64)", args_t);
                let args = self.b.val(format!("call ptr @jw_spawn_args(i64 {})", size));
                let sent = self.m.site(span, "ほかのスレッドへ送られた");
                let at = self.m.site(span, "");
                for (i, (v, t)) in caps.iter().enumerate() {
                    let lt = self.m.lt(t)?;
                    let p = self.b.val(format!("getelementptr {}, ptr {}, i32 0, i32 {}", args_t, args, i));
                    self.b.op(format!("store {} {}, ptr {}", lt, v, p));
                    if self.m.needs_drop(t)? {
                        let f = self.m.helper(H::Send, t);
                        self.b.op(format!("call void {}(ptr {}, ptr {})", f, p, sent));
                    }
                }
                let thunk = self.m.spawn_thunk(fi, &ctys)?;
                let Ty::Task(r) = want else { return Err("spawn の型".to_string()) };
                let rsize = self.m.sizeof(r)?;
                let drop = if self.m.needs_drop(r)? { self.m.helper(H::Drop, r) } else { "null".to_string() };
                let t = self.b.val(format!("call ptr @jw_spawn(ptr {}, ptr {}, i64 {}, ptr {}, ptr {})", thunk, args, rsize, drop, at));
                Ok((t, want.clone()))
            }
            "chan" => {
                let Ty::Chan(x) = want else { return Err("chan の型".to_string()) };
                let size = self.m.sizeof(x)?;
                let drop = if self.m.needs_drop(x)? { self.m.helper(H::Drop, x) } else { "null".to_string() };
                Ok((self.b.val(format!("call ptr @jw_chan_new(i64 {}, ptr {})", size, drop)), want.clone()))
            }
            "task.join" => {
                let lt = self.m.lt(want)?;
                let out = self.b.alloca(&lt);
                let site = self.m.site(span, "");
                self.b.op(format!("call void @jw_task_join(ptr {}, ptr {}, ptr {})", vals[0].0, out, site));
                Ok((self.b.val(format!("load {}, ptr {}", lt, out)), want.clone()))
            }
            "chan.send" | "chan.recv" | "chan.close" => {
                let c = vals[0].0.clone();
                let site = self.m.site(span, "");
                let r = match name {
                    "chan.send" => {
                        let (v, t) = vals[1].clone();
                        let lt = self.m.lt(&t)?;
                        let tmp = self.b.alloca(&lt);
                        self.b.op(format!("store {} {}, ptr {}", lt, v, tmp));
                        if self.m.needs_drop(&t)? {
                            let sent = self.m.site(span, "ほかのスレッドへ送られた");
                            let f = self.m.helper(H::Send, &t);
                            self.b.op(format!("call void {}(ptr {}, ptr {})", f, tmp, sent));
                        }
                        self.b.op(format!("call void @jw_chan_send(ptr {}, ptr {}, ptr {})", c, tmp, site));
                        ("0".to_string(), Ty::Unit)
                    }
                    "chan.recv" => {
                        let Ty::Enum(_, targs) = want else { return Err("recv の型".to_string()) };
                        let x = targs[0].clone();
                        let xl = self.m.lt(&x)?;
                        let out = self.b.alloca(&xl);
                        let got = self.b.val(format!("call i32 @jw_chan_recv(ptr {}, ptr {}, ptr {})", c, out, site));
                        let lt = self.m.lt(want)?;
                        let res = self.b.alloca(&lt);
                        let none = self.b.val(format!("icmp eq i32 {}, 0", got));
                        let (_, no) = self.b.if_true(&none);
                        let nv = self.make_enum(want, "None", Vec::new())?;
                        self.b.op(format!("store {} {}, ptr {}", lt, nv, res));
                        let done = self.b.l();
                        self.b.br(&done);
                        self.b.label(&no);
                        let v = self.b.val(format!("load {}, ptr {}", xl, out));
                        let sv = self.make_enum(want, "Some", vec![(v, x)])?;
                        self.b.op(format!("store {} {}, ptr {}", lt, sv, res));
                        self.b.br(&done);
                        self.b.label(&done);
                        (self.b.val(format!("load {}, ptr {}", lt, res)), want.clone())
                    }
                    _ => {
                        self.b.op(format!("call void @jw_chan_close(ptr {}, ptr {})", c, site));
                        ("0".to_string(), Ty::Unit)
                    }
                };
                // 一時的なチャンネルの値なら捨てる
                if !borrowed {
                    self.b.op(format!("call void @jw_chan_release(ptr {}, ptr {})", c, site));
                }
                Ok(r)
            }
            "print" | "eprint" => {
                self.b.op(format!("call void @jw_{}_begin()", name));
                for (i, (v, _)) in vals.iter().enumerate() {
                    self.b.op(format!("call void @jw_{}_part(ptr {}, i32 {})", name, v, (i == 0) as i32));
                }
                self.b.op(format!("call void @jw_{}_end()", name));
                release(self, &vals);
                Ok(("0".to_string(), Ty::Unit))
            }
            // 戻らない（IR のブロックは続けて unreachable で終わる）
            "chr" => {
                let site = self.m.site(span, "");
                Ok((self.b.val(format!("call ptr @jw_chr(i64 {}, ptr {})", vals[0].0, site)), Ty::Str))
            }
            "exit" => {
                self.b.op(format!("call void @jw_exit(i64 {})", vals[0].0));
                Ok(("0".to_string(), Ty::Unit))
            }
            "read_file" => {
                let out = self.b.alloca("ptr");
                let ok = self.b.val(format!("call i32 @jw_read_file(ptr {}, ptr {})", vals[0].0, out));
                release(self, &vals);
                self.result_of(want, &ok, &out, |me| Ok((me.b.val(format!("load ptr, ptr {}", out)), Ty::Str)))
            }
            "write_file" => {
                let out = self.b.alloca("ptr");
                let ok = self.b.val(format!("call i32 @jw_write_file(ptr {}, ptr {}, ptr {})", vals[0].0, vals[1].0, out));
                release(self, &vals);
                self.result_of(want, &ok, &out, |_| Ok(("0".to_string(), Ty::Unit)))
            }
            // 引数のリストは jw_run_command が解放する
            "run_command" => {
                let list = self.b.alloca("%list");
                self.b.op(format!("store %list {}, ptr {}", vals[1].0, list));
                let code = self.b.alloca("i64");
                let out = self.b.alloca("ptr");
                let ok = self.b.val(format!("call i32 @jw_run_command(ptr {}, ptr {}, ptr {}, ptr {})", vals[0].0, list, code, out));
                release(self, &vals[..1]);
                self.result_of(want, &ok, &out, |me| Ok((me.b.val(format!("load i64, ptr {}", code)), Ty::Int(IntTy::I64))))
            }
            "concat" if vals.len() == 2 => {
                let r = self.b.val(format!("call ptr @jw_str_concat(ptr {}, ptr {})", vals[0].0, vals[1].0));
                release(self, &vals);
                Ok((r, Ty::Str))
            }
            "concat" => {
                let sb = self.b.val("call ptr @jw_sb_new()");
                for (v, _) in &vals {
                    self.b.op(format!("call void @jw_sb_str(ptr {}, ptr {})", sb, v));
                }
                release(self, &vals);
                Ok((self.b.val(format!("call ptr @jw_sb_finish(ptr {})", sb)), Ty::Str))
            }
            "assert" => {
                let bad = self.b.val(format!("xor i1 {}, true", vals[0].0));
                let (_, ok) = self.b.if_true(&bad);
                let title = self.m.cstr("assert に失敗しました");
                let l = self.m.cstr("この条件が false でした");
                let s = self.m.site(span, "");
                self.b.op(format!("call void @jw_fail(ptr {}, ptr {}, ptr {}, ptr null)", title, s, l));
                self.b.op("unreachable");
                self.b.label(&ok);
                Ok(("0".to_string(), Ty::Unit))
            }
            "read_line" => {
                let out = self.b.alloca("ptr");
                let ok = self.b.val(format!("call i32 @jw_read_line(ptr {})", out));
                let none = self.b.val(format!("icmp eq i32 {}, 0", ok));
                let lt = self.m.lt(want)?;
                let res = self.b.alloca(&lt);
                let (_, no) = self.b.if_true(&none);
                let nv = self.make_enum(want, "None", Vec::new())?;
                self.b.op(format!("store {} {}, ptr {}", lt, nv, res));
                let done = self.b.l();
                self.b.br(&done);
                self.b.label(&no);
                let line = self.b.val(format!("load ptr, ptr {}", out));
                let sv = self.make_enum(want, "Some", vec![(line, Ty::Str)])?;
                self.b.op(format!("store {} {}, ptr {}", lt, sv, res));
                self.b.br(&done);
                self.b.label(&done);
                Ok((self.b.val(format!("load {}, ptr {}", lt, res)), want.clone()))
            }
            "args" => {
                let out = self.b.alloca("%list");
                self.b.op(format!("store %list zeroinitializer, ptr {}", out));
                self.b.op(format!("call void @jw_args(ptr {})", out));
                Ok((self.b.val(format!("load %list, ptr {}", out)), Ty::List(Box::new(Ty::Str))))
            }
            "f64.to_bits" => Ok((self.b.val(format!("bitcast double {} to i64", vals[0].0)), Ty::Int(IntTy::U64))),
            "f64.sqrt" | "f64.abs" => {
                let f = if name == "f64.sqrt" { "sqrt" } else { "fabs" };
                Ok((self.b.val(format!("call double @llvm.{}.f64(double {})", f, vals[0].0)), Ty::Float))
            }
            "f64.max" | "f64.min" => {
                let f = if name == "f64.max" { "maxnum" } else { "minnum" };
                let v = self.b.val(format!("call double @llvm.{}.f64(double {}, double {})", f, vals[0].0, vals[1].0));
                Ok((v, Ty::Float))
            }
            // 整数の abs / max / min は比べて選ぶ（abs は一周する: 最小の値はそのまま）
            _ if name.starts_with("int.") => {
                let Ty::Int(i) = vals[0].1 else { unreachable!("an integer method on an integer") };
                let lt = format!("i{}", i.bits());
                let x = vals[0].0.clone();
                let v = match &name[4..] {
                    "abs" if !i.signed() => x,
                    "abs" => {
                        let n = self.b.val(format!("sub {} 0, {}", lt, x));
                        let c = self.b.val(format!("icmp slt {} {}, 0", lt, x));
                        self.b.val(format!("select i1 {}, {} {}, {} {}", c, lt, n, lt, x))
                    }
                    m => {
                        let y = vals[1].0.clone();
                        let cmp = match (m == "max", i.signed()) {
                            (true, true) => "sgt",
                            (true, false) => "ugt",
                            (false, true) => "slt",
                            (false, false) => "ult",
                        };
                        let c = self.b.val(format!("icmp {} {} {}, {}", cmp, lt, x, y));
                        self.b.val(format!("select i1 {}, {} {}, {} {}", c, lt, x, lt, y))
                    }
                };
                Ok((v, Ty::Int(i)))
            }
            _ if name.starts_with("str.") => {
                let r = self.str_method(&name[4..], &vals, span, want)?;
                release(self, &vals);
                Ok(r)
            }
            _ => Err(format!("組み込み {}", name)),
        }
    }

    fn str_method(&mut self, name: &str, vals: &[(String, Ty)], span: Span, want: &Ty) -> G<(String, Ty)> {
        let s = vals[0].0.clone();
        let int = |me: &mut Self, i: usize| -> G<String> {
            match &vals[i].1 {
                Ty::Int(t) => Ok(widen(&mut me.b, &vals[i].0, *t)),
                t => Err(format!("整数でない引数 {}", t.show())),
            }
        };
        let i64t = Ty::Int(IntTy::I64);
        let site = self.m.site(span, "");
        let list_of = |me: &mut Self, f: &str, extra: &str| -> (String, Ty) {
            let out = me.b.alloca("%list");
            me.b.op(format!("store %list zeroinitializer, ptr {}", out));
            me.b.op(format!("call void @{}(ptr {}{}, ptr {})", f, s, extra, out));
            (me.b.val(format!("load %list, ptr {}", out)), Ty::List(Box::new(Ty::Str)))
        };
        Ok(match name {
            "len" => (self.b.val(format!("call i64 @jw_str_chars(ptr {})", s)), i64t),
            "byte_len" => {
                let lp = self.b.val(format!("getelementptr i8, ptr {}, i64 8", s));
                (self.b.val(format!("load i64, ptr {}", lp)), i64t)
            }
            "chars" => list_of(self, "jw_str_chars_list", ""),
            "bytes" => {
                let (v, _) = list_of(self, "jw_str_bytes", "");
                (v, Ty::List(Box::new(Ty::Int(IntTy::U8))))
            }
            "code" => (self.b.val(format!("call i64 @jw_str_code(ptr {}, ptr {})", s, site)), i64t),
            "lines" => list_of(self, "jw_str_lines", ""),
            "split" => {
                let sep = vals[1].0.clone();
                let lp = self.b.val(format!("getelementptr i8, ptr {}, i64 8", sep));
                let n = self.b.val(format!("load i64, ptr {}", lp));
                let empty = self.b.val(format!("icmp eq i64 {}, 0", n));
                let (_, ok) = self.b.if_true(&empty);
                let title = self.m.cstr("空の文字列では分けられません");
                let l = self.m.cstr("1 文字ずつにするなら `chars()` を使ってください");
                self.b.op(format!("call void @jw_fail(ptr {}, ptr {}, ptr {}, ptr null)", title, site, l));
                self.b.op("unreachable");
                self.b.label(&ok);
                list_of(self, "jw_str_split", &format!(", ptr {}", sep))
            }
            "trim" => (self.b.val(format!("call ptr @jw_str_trim(ptr {})", s)), Ty::Str),
            "to_upper" => (self.b.val(format!("call ptr @jw_str_upper(ptr {})", s)), Ty::Str),
            "to_lower" => (self.b.val(format!("call ptr @jw_str_lower(ptr {})", s)), Ty::Str),
            "contains" | "starts_with" | "ends_with" => {
                let f = match name {
                    "contains" => "jw_str_contains",
                    "starts_with" => "jw_str_starts",
                    _ => "jw_str_ends",
                };
                let r = self.b.val(format!("call i32 @{}(ptr {}, ptr {})", f, s, vals[1].0));
                (self.b.val(format!("icmp ne i32 {}, 0", r)), Ty::Bool)
            }
            "find" => {
                let r = self.b.val(format!("call i64 @jw_str_find(ptr {}, ptr {})", s, vals[1].0));
                self.option_of(want, &r, &i64t)?
            }
            "slice" => {
                let a = int(self, 1)?;
                let c = int(self, 2)?;
                (self.b.val(format!("call ptr @jw_str_slice(ptr {}, i64 {}, i64 {}, ptr {})", s, a, c, site)), Ty::Str)
            }
            "replace" => (self.b.val(format!("call ptr @jw_str_replace(ptr {}, ptr {}, ptr {})", s, vals[1].0, vals[2].0)), Ty::Str),
            "repeat" => {
                let k = int(self, 1)?;
                (self.b.val(format!("call ptr @jw_str_repeat(ptr {}, i64 {}, ptr {})", s, k, site)), Ty::Str)
            }
            "to_float" => {
                let out = self.b.alloca("double");
                let ok = self.b.val(format!("call i32 @jw_str_to_float(ptr {}, ptr {})", s, out));
                let opt = want.clone();
                let lt = self.m.lt(&opt)?;
                let res = self.b.alloca(&lt);
                let none = self.b.val(format!("icmp eq i32 {}, 0", ok));
                let (_, no) = self.b.if_true(&none);
                let nv = self.make_enum(&opt, "None", Vec::new())?;
                self.b.op(format!("store {} {}, ptr {}", lt, nv, res));
                let done = self.b.l();
                self.b.br(&done);
                self.b.label(&no);
                let v = self.b.val(format!("load double, ptr {}", out));
                let sv = self.make_enum(&opt, "Some", vec![(v, Ty::Float)])?;
                self.b.op(format!("store {} {}, ptr {}", lt, sv, res));
                self.b.br(&done);
                self.b.label(&done);
                (self.b.val(format!("load {}, ptr {}", lt, res)), opt)
            }
            "to_int" => {
                let out = self.b.alloca("i64");
                let ok = self.b.val(format!("call i32 @jw_str_to_int(ptr {}, ptr {})", s, out));
                let v = self.b.val(format!("load i64, ptr {}", out));
                let none = self.b.val(format!("icmp eq i32 {}, 0", ok));
                let r = self.b.val(format!("select i1 {}, i64 -1, i64 {}", none, v));
                // -1 の文字列と区別するため、ないときだけ None にする
                let opt = want.clone();
                let lt = self.m.lt(&opt)?;
                let res = self.b.alloca(&lt);
                let (_, no) = self.b.if_true(&none);
                let nv = self.make_enum(&opt, "None", Vec::new())?;
                self.b.op(format!("store {} {}, ptr {}", lt, nv, res));
                let done = self.b.l();
                self.b.br(&done);
                self.b.label(&no);
                let sv = self.make_enum(&opt, "Some", vec![(r, i64t)])?;
                self.b.op(format!("store {} {}, ptr {}", lt, sv, res));
                self.b.br(&done);
                self.b.label(&done);
                (self.b.val(format!("load {}, ptr {}", lt, res)), opt)
            }
            _ => return Err(format!("文字列のメソッド {}", name)),
        })
    }

    /// 負なら None、そうでなければ Some(v)
    fn option_of(&mut self, want: &Ty, v: &str, t: &Ty) -> G<(String, Ty)> {
        let lt = self.m.lt(want)?;
        let res = self.b.alloca(&lt);
        let neg = self.b.val(format!("icmp slt i64 {}, 0", v));
        let (_, no) = self.b.if_true(&neg);
        let nv = self.make_enum(want, "None", Vec::new())?;
        self.b.op(format!("store {} {}, ptr {}", lt, nv, res));
        let done = self.b.l();
        self.b.br(&done);
        self.b.label(&no);
        let sv = self.make_enum(want, "Some", vec![(v.to_string(), t.clone())])?;
        self.b.op(format!("store {} {}, ptr {}", lt, sv, res));
        self.b.br(&done);
        self.b.label(&done);
        Ok((self.b.val(format!("load {}, ptr {}", lt, res)), want.clone()))
    }

    fn seq_op(&mut self, name: &str, place: &Place, args: &[Operand], span: Span, arg_span: Span, want: &Ty) -> G<(String, Ty)> {
        let mut vals = Vec::new();
        for o in args {
            vals.push(self.operand(o)?);
        }
        let pr = self.place(place, false)?;
        let i64t = Ty::Int(IntTy::I64);
        match pr.ty.clone() {
            Ty::Map(kt, vt) => {
                let m = self.b.val(format!("load ptr, ptr {}", pr.ptr));
                if name == "len" {
                    return Ok((self.b.val(format!("call i64 @jw_map_len(ptr {})", m)), i64t));
                }
                if name == "keys" {
                    let out = self.b.alloca("%list");
                    self.b.op(format!("store %list zeroinitializer, ptr {}", out));
                    let len = self.b.val(format!("call i64 @jw_map_len(ptr {})", m));
                    let size = self.m.sizeof(&kt)?;
                    self.b.op(format!("call void @jw_list_reserve(ptr {}, i64 {}, i64 {})", out, size, len));
                    let (i, e) = self.b.loop_begin(&len);
                    let kv = self.b.val(format!("call i64 @jw_map_key(ptr {}, i64 {})", m, i));
                    if matches!(*kt, Ty::Str) {
                        let kp = self.b.val(format!("inttoptr i64 {} to ptr", kv));
                        self.b.op(format!("call void @jw_str_retain(ptr {})", kp));
                    }
                    let dst = self.b.val(format!("call ptr @jw_list_push_slot(ptr {}, i64 {}, ptr null)", out, size));
                    store_key(self.m, &mut self.b, &kt, &kv, &dst)?;
                    self.b.loop_end(e);
                    return Ok((self.b.val(format!("load %list, ptr {}", out)), Ty::List(kt)));
                }
                let (kv, kty) = vals[0].clone();
                let kv = self.coerce(&kv, &kty, &kt)?;
                let k64 = self.key64(&kv, &kt)?;
                let i = self.b.val(format!("call i64 @jw_map_find(ptr {}, i64 {})", m, k64));
                self.release_key(&kv, &kt);
                let found = self.b.val(format!("icmp sge i64 {}, 0", i));
                match name {
                    "contains" => Ok((found, Ty::Bool)),
                    "get" | "remove" => {
                        let lt = self.m.lt(want)?;
                        let res = self.b.alloca(&lt);
                        let yes = self.b.l();
                        let no = self.b.l();
                        let done = self.b.l();
                        self.b.op(format!("br i1 {}, label %{}, label %{}", found, yes, no));
                        self.b.label(&yes);
                        let vp = self.b.val(format!("call ptr @jw_map_val(ptr {}, i64 {})", m, i));
                        let v = if name == "get" {
                            self.read_copy(&vp, &vt)?
                        } else {
                            let vl = self.m.lt(&vt)?;
                            let v = self.b.val(format!("load {}, ptr {}", vl, vp));
                            if matches!(*kt, Ty::Str) {
                                let key = self.b.val(format!("call i64 @jw_map_key(ptr {}, i64 {})", m, i));
                                let kp = self.b.val(format!("inttoptr i64 {} to ptr", key));
                                self.b.op(format!("call void @jw_str_release(ptr {})", kp));
                            }
                            self.b.op(format!("call void @jw_map_remove_at(ptr {}, i64 {})", m, i));
                            v
                        };
                        let sv = self.make_enum(want, "Some", vec![(v, (*vt).clone())])?;
                        self.b.op(format!("store {} {}, ptr {}", lt, sv, res));
                        self.b.br(&done);
                        self.b.label(&no);
                        let nv = self.make_enum(want, "None", Vec::new())?;
                        self.b.op(format!("store {} {}, ptr {}", lt, nv, res));
                        self.b.br(&done);
                        self.b.label(&done);
                        Ok((self.b.val(format!("load {}, ptr {}", lt, res)), want.clone()))
                    }
                    _ => Err(format!("辞書のメソッド {}", name)),
                }
            }
            Ty::Array(x, n) => match name {
                "len" => Ok((n.to_string(), i64t)),
                "sort" | "sort_unstable" => {
                    let ptr = pr.ptr.clone();
                    if !self.sort_int(&ptr, &n.to_string(), &x) {
                        let less = self.m.helper(H::Less, &x);
                        let size = self.m.sizeof(&x)?;
                        let f = if name == "sort" { "jw_sort" } else { "jw_sort_unstable" };
                        self.b.op(format!("call void @{}(ptr {}, i64 {}, i64 {}, ptr {}, ptr null)", f, ptr, n, size, less));
                    }
                    Ok(("0".to_string(), Ty::Unit))
                }
                "sort_by" | "sort_unstable_by" => {
                    let ptr = pr.ptr.clone();
                    self.sort_by(&ptr, &n.to_string(), &x, vals[0].clone(), name == "sort_by", span)?;
                    Ok(("0".to_string(), Ty::Unit))
                }
                _ => Err(format!("配列のメソッド {}", name)),
            },
            Ty::List(x) => {
                let xl = self.m.lt(&x)?;
                let size = self.m.sizeof(&x)?;
                let lp = self.b.val(format!("getelementptr %list, ptr {}, i32 0, i32 1", pr.ptr));
                let len = self.b.val(format!("load i64, ptr {}", lp));
                // 要素が動いたときの位置（要素を借りた ref を無効にする。interp の `list_moved`）
                let moved = if matches!(name, "len") { String::new() } else { self.m.site(span, &crate::interp::list_moved_why(name)) };
                match name {
                    "len" => Ok((len, i64t)),
                    "join" => {
                        let r = self.b.val(format!("call ptr @jw_list_join(ptr {}, ptr {})", pr.ptr, vals[0].0));
                        self.b.op(format!("call void @jw_str_release(ptr {})", vals[0].0));
                        Ok((r, Ty::Str))
                    }
                    "push" => {
                        let (v, t) = vals[0].clone();
                        let v = self.coerce(&v, &t, &x)?;
                        let dst = self.b.val(format!("call ptr @jw_list_push_slot(ptr {}, i64 {}, ptr {})", pr.ptr, size, moved));
                        self.b.op(format!("store {} {}, ptr {}", xl, v, dst));
                        Ok(("0".to_string(), Ty::Unit))
                    }
                    "pop" => {
                        let lt = self.m.lt(want)?;
                        let res = self.b.alloca(&lt);
                        let empty = self.b.val(format!("icmp eq i64 {}, 0", len));
                        let yes = self.b.l();
                        let no = self.b.l();
                        let done = self.b.l();
                        self.b.op(format!("br i1 {}, label %{}, label %{}", empty, yes, no));
                        self.b.label(&yes);
                        let nv = self.make_enum(want, "None", Vec::new())?;
                        self.b.op(format!("store {} {}, ptr {}", lt, nv, res));
                        self.b.br(&done);
                        self.b.label(&no);
                        let n1 = self.b.val(format!("sub i64 {}, 1", len));
                        self.b.op(format!("store i64 {}, ptr {}", n1, lp));
                        self.b.op(format!("call void @jw_list_moved(ptr {}, ptr {})", pr.ptr, moved));
                        let data = self.b.val(format!("load ptr, ptr {}", pr.ptr));
                        let ep = self.b.val(format!("getelementptr {}, ptr {}, i64 {}", xl, data, n1));
                        let v = self.b.val(format!("load {}, ptr {}", xl, ep));
                        let sv = self.make_enum(want, "Some", vec![(v, (*x).clone())])?;
                        self.b.op(format!("store {} {}, ptr {}", lt, sv, res));
                        self.b.br(&done);
                        self.b.label(&done);
                        Ok((self.b.val(format!("load {}, ptr {}", lt, res)), want.clone()))
                    }
                    "remove" => {
                        let (iv, it) = vals[0].clone();
                        let i = self.index(&iv, &it, &len, arg_span)?;
                        let tmp = self.b.alloca(&xl);
                        self.b.op(format!("call void @jw_list_remove(ptr {}, i64 {}, i64 {}, ptr {}, ptr {})", pr.ptr, size, i, tmp, moved));
                        Ok((self.b.val(format!("load {}, ptr {}", xl, tmp)), (*x).clone()))
                    }
                    "insert" => {
                        let (iv, it) = vals[0].clone();
                        let Ty::Int(ity) = it else { return Err("整数でない位置".to_string()) };
                        let i = widen(&mut self.b, &iv, ity);
                        let (v, t) = vals[1].clone();
                        let v = self.coerce(&v, &t, &x)?;
                        let neg = self.b.val(format!("icmp slt i64 {}, 0", i));
                        let big = self.b.val(format!("icmp sgt i64 {}, {}", i, len));
                        let bad = self.b.val(format!("or i1 {}, {}", neg, big));
                        let (_, ok) = self.b.if_true(&bad);
                        self.discard(&v, &x, span, "入れられずに捨てられた")?;
                        let s = self.m.site(arg_span, "");
                        self.b.op(format!("call void @jw_fail_insert(i64 {}, i64 {}, ptr {})", i, len, s));
                        self.b.op("unreachable");
                        self.b.label(&ok);
                        let dst = self.b.val(format!("call ptr @jw_list_insert_slot(ptr {}, i64 {}, i64 {}, ptr {})", pr.ptr, size, i, moved));
                        self.b.op(format!("store {} {}, ptr {}", xl, v, dst));
                        Ok(("0".to_string(), Ty::Unit))
                    }
                    "reverse" => {
                        self.b.op(format!("call void @jw_list_reverse(ptr {}, i64 {}, ptr {})", pr.ptr, size, moved));
                        Ok(("0".to_string(), Ty::Unit))
                    }
                    // 中身ごと新しいリストへ移し、元は空にする
                    "drain" => {
                        self.b.op(format!("call void @jw_list_moved(ptr {}, ptr {})", pr.ptr, moved));
                        let v = self.b.val(format!("load %list, ptr {}", pr.ptr));
                        self.b.op(format!("store %list zeroinitializer, ptr {}", pr.ptr));
                        Ok((v, Ty::List(x.clone())))
                    }
                    "sort_by" | "sort_unstable_by" => {
                        let data = self.b.val(format!("load ptr, ptr {}", pr.ptr));
                        self.sort_by(&data, &len, &x, vals[0].clone(), name == "sort_by", span)?;
                        self.b.op(format!("call void @jw_list_moved(ptr {}, ptr {})", pr.ptr, moved));
                        Ok(("0".to_string(), Ty::Unit))
                    }
                    "sort" | "sort_unstable" => {
                        let data = self.b.val(format!("load ptr, ptr {}", pr.ptr));
                        if !self.sort_int(&data, &len, &x) {
                            let less = self.m.helper(H::Less, &x);
                            let f = if name == "sort" { "jw_sort" } else { "jw_sort_unstable" };
                            self.b.op(format!("call void @{}(ptr {}, i64 {}, i64 {}, ptr {}, ptr null)", f, data, len, size, less));
                        }
                        self.b.op(format!("call void @jw_list_moved(ptr {}, ptr {})", pr.ptr, moved));
                        Ok(("0".to_string(), Ty::Unit))
                    }
                    _ => Err(format!("リストのメソッド {}", name)),
                }
            }
            t => Err(format!("{} のメソッド {}", t.show(), name)),
        }
    }
}

// u64::div_ceil は Rust 1.73 から（1.65 で動かすため自前）
#[allow(clippy::manual_div_ceil)]
fn ceil_div(a: u64, b: u64) -> u64 {
    a / b + u64::from(a % b != 0)
}

fn ext(t: IntTy) -> &'static str {
    if t.signed() {
        "sext"
    } else {
        "zext"
    }
}

/// interp の `type_name`（型から決めたもの）
fn type_name(t: &Ty) -> &'static str {
    match t {
        Ty::Unit | Ty::Never => "()",
        Ty::Int(_) => "整数",
        Ty::Float => "小数",
        Ty::Bool => "真偽値",
        Ty::Str => "文字列",
        Ty::Own(_) => "own",
        Ty::Ref(..) => "ref",
        Ty::Struct(..) => "構造体",
        Ty::Enum(..) => "enum",
        Ty::Array(..) => "配列",
        Ty::List(_) => "リスト",
        Ty::Fn(..) => "関数",
        Ty::Map(..) => "辞書",
        Ty::Task(_) => "task",
        Ty::Chan(_) => "chan",
        _ => "?",
    }
}

/// 命令の中の値（Operand）をすべて見る
fn visit_operands(inst: &Inst, f: &mut impl FnMut(&Operand)) {
    let place = |p: &Place, f: &mut dyn FnMut(&Operand)| {
        for pr in &p.proj {
            if let Proj::Index(k, ..) = pr {
                f(k);
            }
        }
    };
    match inst {
        Inst::Assign { place: p, value, .. } => {
            place(p, f);
            match value {
                Rvalue::Use(o) | Rvalue::Un(_, o, _) | Rvalue::Cast(o, _) | Rvalue::Own(o) | Rvalue::Show(o, ..) | Rvalue::Repeat(o, ..) => f(o),
                Rvalue::Bin(_, a, b, _) => {
                    f(a);
                    f(b);
                }
                Rvalue::Ref { place: p, .. } | Rvalue::Take(p) | Rvalue::Items { place: p, .. } | Rvalue::Keys(p) | Rvalue::CopyOut(p, ..) => {
                    place(p, f)
                }
                Rvalue::Struct(_, os) | Rvalue::Enum(_, os, _) | Rvalue::Array(os, _) | Rvalue::Call(_, os, _) | Rvalue::Closure(_, os) | Rvalue::Builtin(_, os, _) => {
                    for o in os {
                        f(o)
                    }
                }
                Rvalue::Map(ps, _) => ps.iter().for_each(|(k, v)| {
                    f(k);
                    f(v);
                }),
                Rvalue::SeqOp { place: p, args, .. } => {
                    place(p, f);
                    for o in args {
                        f(o)
                    }
                }
                Rvalue::CallDyn { this, args, .. } => {
                    place(this, f);
                    for o in args {
                        f(o)
                    }
                }
                Rvalue::CallValue(o, os, _) => {
                    f(o);
                    for o in os {
                        f(o)
                    }
                }
            }
        }
        Inst::Discard { value, .. } => f(value),
        Inst::Drop { .. } => {}
    }
}
