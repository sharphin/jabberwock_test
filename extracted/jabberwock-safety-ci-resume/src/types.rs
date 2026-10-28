//! 型検査。実行する前にプログラム全体を調べる。
//!
//! 型は関数の引数・戻り値・構造体のフィールドに書き、`let` は初期値から決める（前から順に決めるだけ）。
//! 型の食い違いと、`mut` / `ref mut` の違反（書き換えてはいけない場所への書き込み）をここで見つける。
//!
//! ムーブ済みの変数を使う・ダングリングした ref を使う、は型の問題ではないので、ここでは扱わない
//! （ref の安全は世代付き ref と `analyze` が守る。ref の型に寿命は書かない）。

use std::cell::RefCell;
use std::collections::HashMap;

use crate::ast::*;
use crate::diag::{Diagnostic, Span};

type R<T> = Result<T, Diagnostic>;

use crate::interp::BUILTINS;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ty {
    Int(IntTy),
    /// 小数（f64）
    Float,
    Bool,
    Str,
    /// 値なし（戻り値のない関数の呼び出しなど）
    Unit,
    /// 決して終わらない（return / break / continue で必ず抜ける）。どの型にも合う
    Never,
    /// 構造体・enum。2 つ目は型の引数（`Pair<i64>` の `i64`）
    Struct(String, Vec<Ty>),
    Enum(String, Vec<Ty>),
    /// 型の引数（`fn T max<T>` の中の `T`）。中身が何かは分からない
    Param(String),
    /// interface（`own Shape` / `ref Shape` の `Shape`）。当てはまる構造体のどれか。own か ref を通してだけ使う
    Iface(String),
    Own(Box<Ty>),
    /// bool は `mut`
    Ref(Box<Ty>, bool),
    /// `i64[3]`
    Array(Box<Ty>, u64),
    /// `i64[]`（伸び縮みするリスト）
    List(Box<Ty>),
    /// `fn bool(i64, i64)`: 関数（名前のある関数とクロージャ）。戻り値がなければ Unit
    Fn(Vec<Ty>, Box<Ty>),
    /// `i64[str]`: 辞書（キーの型, 値の型）
    Map(Box<Ty>, Box<Ty>),
    /// `task<R>`: `spawn` で作ったスレッド（`join()` で R を受け取る）
    Task(Box<Ty>),
    /// `chan<T>`: スレッドのあいだで値を送るチャンネル
    Chan(Box<Ty>),
}

impl Ty {
    pub fn show(&self) -> String {
        match self {
            Ty::Int(t) => t.name().to_string(),
            Ty::Float => "f64".to_string(),
            Ty::Bool => "bool".to_string(),
            Ty::Str => "str".to_string(),
            Ty::Unit => "値なし".to_string(),
            Ty::Never => "（終わらない）".to_string(),
            Ty::Struct(n, args) | Ty::Enum(n, args) if args.is_empty() => n.clone(),
            Ty::Struct(n, args) | Ty::Enum(n, args) => {
                let a: Vec<String> = args.iter().map(Ty::show).collect();
                format!("{}<{}>", n, a.join(", "))
            }
            Ty::Param(n) | Ty::Iface(n) => n.clone(),
            Ty::Own(t) => format!("own {}", t.show()),
            Ty::Ref(t, false) => format!("ref {}", t.show()),
            Ty::Ref(t, true) => format!("ref mut {}", t.show()),
            Ty::Array(t, n) => format!("{}[{}]", t.show(), n),
            Ty::List(t) => format!("{}[]", t.show()),
            Ty::Map(k, v) => format!("{}[{}]", v.show(), k.show()),
            Ty::Task(t) => format!("task<{}>", t.show()),
            Ty::Chan(t) => format!("chan<{}>", t.show()),
            Ty::Fn(ps, r) => {
                let ps: Vec<String> = ps.iter().map(Ty::show).collect();
                match &**r {
                    Ty::Unit => format!("fn({})", ps.join(", ")),
                    r => format!("fn {}({})", r.show(), ps.join(", ")),
                }
            }
        }
    }

    /// own / ref の先の型
    fn pointee(&self) -> Option<&Ty> {
        match self {
            Ty::Own(t) | Ty::Ref(t, _) => Some(t),
            _ => None,
        }
    }
}

const I64: Ty = Ty::Int(IntTy::I64);

/// `from` の値を `to` の場所に入れられるか。`ref mut T` は `ref T` としても使える。
/// 整数は値が欠けない変換（u8 → i64 など）なら自動で変換する
fn assignable(from: &Ty, to: &Ty) -> bool {
    match (from, to) {
        (Ty::Never, _) => true,
        (Ty::Int(a), Ty::Int(b)) => a.widens_to(*b),
        (Ty::Ref(a, true), Ty::Ref(b, false)) => a == b,
        _ => from == to,
    }
}

fn mismatch(want: &Ty, got: &Ty, span: Span) -> Diagnostic {
    let d = Diagnostic::new("型が合いません")
        .primary(span, format!("ここは `{}` ですが、`{}` が必要です", got.show(), want.show()));
    if matches!(want, Ty::Ref(_, true)) && !matches!(got, Ty::Ref(..)) {
        return d.note("書き換えられる引数（`ref mut`）には、呼ぶ側でも `ref mut x` と書いてください");
    }
    if let (Ty::Int(_), Ty::Int(w)) = (got, want) {
        // 値が欠けうる変換は自動ではしない
        return d.note(format!("値が欠けるかもしれない変換は `as` で明示してください（例: `x as {}`）", w.name()));
    }
    d
}

/// 型の引数を具体的な型に置き換える（決まっていないものはそのまま）
pub fn subst(t: &Ty, map: &HashMap<String, Ty>) -> Ty {
    match t {
        Ty::Param(p) => map.get(p).cloned().unwrap_or_else(|| t.clone()),
        Ty::Struct(n, args) => Ty::Struct(n.clone(), args.iter().map(|a| subst(a, map)).collect()),
        Ty::Enum(n, args) => Ty::Enum(n.clone(), args.iter().map(|a| subst(a, map)).collect()),
        Ty::Own(x) => Ty::Own(Box::new(subst(x, map))),
        Ty::Ref(x, m) => Ty::Ref(Box::new(subst(x, map)), *m),
        Ty::Array(x, n) => Ty::Array(Box::new(subst(x, map)), *n),
        Ty::List(x) => Ty::List(Box::new(subst(x, map))),
        Ty::Map(k, v) => Ty::Map(Box::new(subst(k, map)), Box::new(subst(v, map))),
        Ty::Task(x) => Ty::Task(Box::new(subst(x, map))),
        Ty::Chan(x) => Ty::Chan(Box::new(subst(x, map))),
        Ty::Fn(ps, r) => Ty::Fn(ps.iter().map(|p| subst(p, map)).collect(), Box::new(subst(r, map))),
        _ => t.clone(),
    }
}

fn contains_iface(t: &Ty) -> bool {
    match t {
        Ty::Iface(_) => true,
        Ty::Struct(_, args) | Ty::Enum(_, args) => args.iter().any(contains_iface),
        Ty::Own(x) | Ty::Ref(x, _) | Ty::Array(x, _) | Ty::List(x) | Ty::Task(x) | Ty::Chan(x) => contains_iface(x),
        Ty::Fn(ps, r) => ps.iter().any(contains_iface) || contains_iface(r),
        Ty::Map(k, v) => contains_iface(k) || contains_iface(v),
        _ => false,
    }
}

fn contains_param(t: &Ty) -> bool {
    match t {
        Ty::Param(_) => true,
        Ty::Struct(_, args) | Ty::Enum(_, args) => args.iter().any(contains_param),
        Ty::Own(x) | Ty::Ref(x, _) | Ty::Array(x, _) | Ty::List(x) | Ty::Task(x) | Ty::Chan(x) => contains_param(x),
        Ty::Fn(ps, r) => ps.iter().any(contains_param) || contains_param(r),
        Ty::Map(k, v) => contains_param(k) || contains_param(v),
        _ => false,
    }
}

/// `pattern` の中の型の引数（`tparams`）が、まだ決まっていないものを含むか
fn has_unbound(pattern: &Ty, tparams: &[String], map: &HashMap<String, Ty>) -> bool {
    match pattern {
        Ty::Param(p) => tparams.contains(p) && !map.contains_key(p),
        Ty::Struct(_, args) | Ty::Enum(_, args) => args.iter().any(|a| has_unbound(a, tparams, map)),
        Ty::Own(x) | Ty::Ref(x, _) | Ty::Array(x, _) | Ty::List(x) | Ty::Task(x) | Ty::Chan(x) => {
            has_unbound(x, tparams, map)
        }
        Ty::Fn(ps, r) => ps.iter().any(|p| has_unbound(p, tparams, map)) || has_unbound(r, tparams, map),
        Ty::Map(k, v) => has_unbound(k, tparams, map) || has_unbound(v, tparams, map),
        _ => false,
    }
}

/// 型の形を見比べて、型の引数が何かを決める（`T` と `i64` なら T = i64）。
/// 合わないところは放っておく（あとで型の食い違いとして見つかる）。
/// 整数どうしは、欠けずにそろえられる広い方にする（`max(a_u8, b_i64)` の T は i64）
fn unify(pattern: &Ty, actual: &Ty, tparams: &[String], map: &mut HashMap<String, Ty>) {
    match (pattern, actual) {
        (_, Ty::Never) => {}
        (Ty::Param(p), _) if tparams.contains(p) => match map.get(p) {
            None => {
                map.insert(p.clone(), actual.clone());
            }
            Some(Ty::Int(a)) => {
                if let Ty::Int(b) = actual {
                    if let Some(c) = IntTy::common(*a, *b) {
                        map.insert(p.clone(), Ty::Int(c));
                    }
                }
            }
            Some(_) => {}
        },
        (Ty::Struct(n, xs), Ty::Struct(m, ys)) | (Ty::Enum(n, xs), Ty::Enum(m, ys)) if n == m => {
            for (x, y) in xs.iter().zip(ys) {
                unify(x, y, tparams, map);
            }
        }
        (Ty::Own(x), Ty::Own(y)) | (Ty::Ref(x, _), Ty::Ref(y, _)) => unify(x, y, tparams, map),
        (Ty::Array(x, n), Ty::Array(y, m)) if n == m => unify(x, y, tparams, map),
        (Ty::List(x), Ty::List(y)) | (Ty::Task(x), Ty::Task(y)) | (Ty::Chan(x), Ty::Chan(y)) => unify(x, y, tparams, map),
        (Ty::Map(k, x), Ty::Map(j, y)) => {
            unify(k, j, tparams, map);
            unify(x, y, tparams, map);
        }
        (Ty::Fn(xs, a), Ty::Fn(ys, b)) if xs.len() == ys.len() => {
            for (x, y) in xs.iter().zip(ys) {
                unify(x, y, tparams, map);
            }
            unify(a, b, tparams, map);
        }
        _ => {}
    }
}

/// 整数の値を、型 `want` の場所で使えるようにする（実行時に変換する印をつける）
fn convert(e: &Expr, got: &Ty, want: &Ty) {
    if let (Ty::Int(a), Ty::Int(b)) = (got, want) {
        if a != b {
            e.int.set(Some(*b));
        }
    }
}

#[derive(Clone)]
struct VarInfo {
    ty: Ty,
    mutable: bool,
    decl: Span,
}

/// 場所に書き込めるか（interp の `Access` と同じ考え方を、型から静的に決める）
enum Access {
    Write,
    ImmutVar { name: String, decl: Span },
    ReadOnlyRef { what: String, origin: Span },
}

impl Access {
    /// ポインタ（型 `t` の式 `e`）の先の場所に書き込めるか。own の先は持ち主と同じ、ref の先は ref 次第
    fn through(self, t: &Ty, origin: Span, what: String) -> Access {
        match t {
            Ty::Ref(_, true) => Access::Write,
            Ty::Ref(_, false) => Access::ReadOnlyRef { what, origin },
            _ => self,
        }
    }

    fn require(&self, span: Span, doing: &str) -> R<()> {
        match self {
            Access::Write => Ok(()),
            // `this` は ref なので、変数そのものではなく先を書き換える（enum のメソッドで `this = ...` と書いたとき）
            Access::ImmutVar { name, .. } if name == "this" => Err(Diagnostic::new("`this` そのものは書き換えられません")
                .primary(span, format!("ここで{}", doing))
                .note("this の先を丸ごと書き換えるなら `*this = 値;` と書いてください（`fn mut` のメソッドで）")),
            Access::ImmutVar { name, decl } => Err(Diagnostic::new(format!(
                "`{}` は mut ではないので書き換えられません",
                name
            ))
            .label(*decl, format!("ここで `{}` を宣言した（mut がない）", name))
            .primary(span, format!("ここで{}", doing))
            .note(format!("書き換えるなら `let mut {}` や `i64 mut {}` のように mut をつけてください", name, name))),
            Access::ReadOnlyRef { what, .. } if what == "this" => Err(Diagnostic::new("書き換えないメソッドの中で書き換えようとしました")
                .primary(span, format!("ここで{}", doing))
                .note("フィールドを書き換えるメソッドは `fn mut 名前()` と書いてください")),
            Access::ReadOnlyRef { what, origin } => Err(Diagnostic::new("読むだけの ref を通して書き換えようとしました")
                .label(*origin, format!("`{}` は読むだけの ref（mut がない）", what))
                .primary(span, format!("ここで{}", doing))
                .note("書き換えるなら `ref mut` で借りてください")),
        }
    }
}

struct FnSig {
    /// 最初の引数が `this` のメソッドか（`p.m()` でだけ呼べる）
    method: bool,
    tparams: Vec<String>,
    /// 型の引数ごとの条件（`tparams` と同じ順）
    bounds: Vec<Vec<String>>,
    params: Vec<Ty>,
    ret: Ty,
}

/// 調べているクロージャ: 本体が始まるスコープの位置と、取り込んだ変数の名前（と借りるか）
struct LambdaScope {
    base: usize,
    caps: Vec<String>,
    borrowed: Vec<bool>,
    borrows: bool,
}

/// 固定長の配列の大きさの上限（8 バイトの数で 2^17 = 1 MiB）と、`[x; n]` のリストの上限（2^59 = 2^62 バイト）
const ARRAY_CELLS: u64 = 1 << 17;
const LIST_CELLS: u64 = 1 << 59;
/// 大きさの数え方の頭打ち（jwc は i64 で数えるので、あふれない値にそろえる）
const CELLS_CAP: u64 = 1 << 60;

fn cap_mul(a: u64, b: u64) -> u64 {
    let (a, b) = (a.min(CELLS_CAP), b.min(CELLS_CAP));
    if a != 0 && b > CELLS_CAP / a {
        CELLS_CAP
    } else {
        a * b
    }
}

fn cap_add(a: u64, b: u64) -> u64 {
    (a.min(CELLS_CAP) + b.min(CELLS_CAP)).min(CELLS_CAP)
}

struct Checker<'a> {
    structs: HashMap<&'a str, (&'a StructDef, Vec<Ty>)>,
    /// enum ごとの定義と、種類ごとの中身の型
    enums: HashMap<&'a str, (&'a EnumDef, Vec<Vec<Ty>>)>,
    fns: HashMap<&'a str, FnSig>,
    scopes: Vec<HashMap<String, VarInfo>>,
    /// 今調べている関数の戻り値の型
    ret: Ty,
    /// 今いる定義（関数・構造体・enum）の型の引数
    tparams: Vec<String>,
    /// interface ごとのメソッドの形
    interfaces: HashMap<&'a str, Vec<IMethod>>,
    /// 今調べている関数の、型の引数ごとの条件
    bounds: HashMap<String, Vec<String>>,
    /// 今調べているメソッドの構造体のフィールド名（シャドーイングの禁止に使う）
    fields: Vec<String>,
    /// 今調べているクロージャ（外から順）
    lambdas: Vec<LambdaScope>,
    warnings: Vec<Diagnostic>,
    /// 読み込んだモジュールの名前（変数の名前にできない）
    modules: &'a [String],
    /// 定数の名前（変数の名前にできない。モジュールの中のものは `geom.N`）
    consts: &'a [String],
    /// 今調べている関数のモジュール（最初のファイルなら None）
    module: Option<&'a str>,
    /// 式ごとの型などの記録
    info: TypeInfo,
    /// いちばん最近 `instantiate` で決めた型の引数（呼び出しの式が受け取って記録する）
    pending_targs: Option<HashMap<String, Ty>>,
}

/// interface のメソッドの形（`this` を除いた引数）
#[derive(Clone)]
struct IMethod {
    name: String,
    mutable: bool,
    params: Vec<Ty>,
    ret: Ty,
}

/// 組み込みの条件: 大小を比べられる型（整数と str）
const ORDERED: &str = "ordered";

const STR_METHODS: &str = "len, byte_len, chars, bytes, code, lines, split, trim, to_upper, to_lower, contains, starts_with, ends_with, find, slice, replace, repeat, to_int, to_float";

/// 型検査で分かったこと（IR への書き出しに使う）。式は構文木の中の位置（アドレス）で引く
#[derive(Default)]
pub struct TypeInfo {
    /// 式の型（ジェネリックな関数の中では `T` を含む）。整数の自動変換の印（`Expr.int`）はこれとは別
    pub expr_ty: HashMap<*const Expr, Ty>,
    /// 呼び出し・構造体や enum の値を作る式で決めた型の引数
    pub targs: HashMap<*const Expr, HashMap<String, Ty>>,
    /// 変数（引数・ローカル変数・ループ変数・case で受け取る名前）の型。宣言した位置で引く
    pub var_ty: HashMap<Span, Ty>,
    /// 関数の戻り値の型（なければ Unit）
    pub fn_ret: HashMap<String, Ty>,
    /// enum の型の引数と種類（名前, 中身の型。型の引数は `Ty::Param` のまま）
    pub enums: HashMap<String, (Vec<String>, Variants)>,
    /// 構造体の型の引数とフィールドの型（定義の順。型の引数は `Ty::Param` のまま）
    pub structs: HashMap<String, (Vec<String>, Vec<Ty>)>,
}

/// enum の種類の一覧（名前, 中身の型）
pub type Variants = Vec<(String, Vec<Ty>)>;

impl TypeInfo {
    pub fn ty(&self, e: &Expr) -> Option<&Ty> {
        self.expr_ty.get(&(e as *const Expr))
    }
}

/// プログラム全体の型を調べる。よければ警告（実行は止めない）を返す
pub fn check(prog: &Program) -> R<Vec<Diagnostic>> {
    Ok(check_with_info(prog)?.0)
}

/// 型を調べて、警告と、式ごとの型などの記録を返す
pub fn check_with_info(prog: &Program) -> R<(Vec<Diagnostic>, TypeInfo)> {
    let mut c = Checker {
        structs: HashMap::new(),
        enums: HashMap::new(),
        fns: HashMap::new(),
        scopes: Vec::new(),
        fields: Vec::new(),
        lambdas: Vec::new(),
        ret: Ty::Unit,
        tparams: Vec::new(),
        interfaces: HashMap::new(),
        bounds: HashMap::new(),
        warnings: Vec::new(),
        modules: &prog.modules,
        consts: &prog.consts,
        module: None,
        info: TypeInfo::default(),
        pending_targs: None,
    };
    for s in &prog.structs {
        if c.structs.contains_key(s.name.as_str()) {
            return Err(Diagnostic::new(format!("構造体 `{}` が二重に定義されています", s.name))
                .primary(s.span, "ここで再定義されている"));
        }
        c.structs.insert(&s.name, (s, Vec::new()));
    }
    for en in &prog.enums {
        if c.structs.contains_key(en.name.as_str()) || c.enums.contains_key(en.name.as_str()) {
            return Err(Diagnostic::new(format!("型 `{}` が二重に定義されています", en.name))
                .primary(en.span, "同じ名前の型がすでにある"));
        }
        c.enums.insert(&en.name, (en, Vec::new()));
    }
    for i in &prog.interfaces {
        let n = i.name.as_str();
        if c.structs.contains_key(n) || c.enums.contains_key(n) || c.interfaces.contains_key(n) || n == ORDERED {
            return Err(Diagnostic::new(format!("型 `{}` が二重に定義されています", n))
                .primary(i.span, "同じ名前の型がすでにある"));
        }
        c.interfaces.insert(n, Vec::new());
    }
    for i in &prog.interfaces {
        c.tparams = Vec::new();
        let mut ms = Vec::new();
        for (m, mutable) in &i.methods {
            let params = m.params.iter().map(|p| c.resolve(&p.ty)).collect::<R<Vec<_>>>()?;
            let ret = match &m.ret {
                Some(t) => c.resolve(t)?,
                None => Ty::Unit,
            };
            ms.push(IMethod { name: m.name.clone(), mutable: *mutable, params, ret });
        }
        c.interfaces.insert(&i.name, ms);
    }
    for s in &prog.structs {
        c.tparams = s.tparams.clone();
        let tys = s.field_types.iter().map(|t| c.resolve(t)).collect::<R<Vec<_>>>()?;
        c.info.structs.insert(s.name.clone(), (s.tparams.clone(), tys.clone()));
        c.structs.get_mut(s.name.as_str()).expect("inserted").1 = tys;
    }
    for en in &prog.enums {
        c.tparams = en.tparams.clone();
        let mut all = Vec::new();
        for v in &en.variants {
            all.push(v.fields.iter().map(|(t, _)| c.resolve(t)).collect::<R<Vec<_>>>()?);
        }
        let variants = en.variants.iter().map(|v| v.name.clone()).zip(all.iter().cloned()).collect();
        c.info.enums.insert(en.name.clone(), (en.tparams.clone(), variants));
        c.enums.get_mut(en.name.as_str()).expect("inserted").1 = all;
    }
    for s in &prog.structs {
        c.check_finite(&s.name, s.span)?;
    }
    for en in &prog.enums {
        c.check_finite(&en.name, en.span)?;
    }
    for f in &prog.fns {
        if BUILTINS.contains(&f.name.as_str()) {
            return Err(Diagnostic::new(format!("`{}` は組み込み関数の名前です", f.name))
                .primary(f.span, "別の名前にしてください"));
        }
        if c.fns.contains_key(f.name.as_str()) {
            return Err(Diagnostic::new(format!("関数 `{}` が二重に定義されています", f.name))
                .primary(f.span, "ここで再定義されている"));
        }
        if f.name == "main" && (f.ret.is_some() || !f.params.is_empty()) {
            return Err(Diagnostic::new("`main` は引数も戻り値もない関数です")
                .primary(f.span, "ここ")
                .note("`fn main() { ... }` と書いてください"));
        }
        for b in f.bounds.iter().flatten() {
            if b != ORDERED && !c.interfaces.contains_key(b.as_str()) {
                return Err(Diagnostic::new(format!("条件 `{}` は定義されていません", b))
                    .primary(f.span, "型の引数の条件には interface の名前か `ordered` を書きます"));
            }
        }
        c.tparams = f.tparams.clone();
        let params = f.params.iter().map(|p| c.resolve(&p.ty)).collect::<R<Vec<_>>>()?;
        let ret = match &f.ret {
            Some(t) => c.resolve(t)?,
            None => Ty::Unit,
        };
        c.fns.insert(&f.name, FnSig {
            method: f.name.contains('.') && matches!(f.params.first(), Some(p) if p.name == "this"),
            tparams: f.tparams.clone(),
            bounds: f.bounds.clone(),
            params,
            ret,
        });
    }
    for f in &prog.fns {
        c.check_fn(f)?;
    }
    Ok((c.warnings, c.info))
}

impl<'a> Checker<'a> {
    /// own / ref の先の型。interface の名前はここでだけ書ける
    fn resolve_pointee(&self, t: &TypeExpr) -> R<Ty> {
        match &t.kind {
            TypeKind::Name(n) if self.interfaces.contains_key(n.as_str()) && !self.tparams.contains(n) => {
                Ok(Ty::Iface(n.clone()))
            }
            _ => self.resolve(t),
        }
    }

    fn resolve(&self, t: &TypeExpr) -> R<Ty> {
        Ok(match &t.kind {
            TypeKind::Own(inner) => Ty::Own(Box::new(self.resolve_pointee(inner)?)),
            TypeKind::App(n, args) if n == "task" || n == "chan" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new(format!("`{}` の型の引数の数が違います", n))
                        .primary(t.span, format!("1 個必要ですが {} 個書かれています", args.len())));
                }
                let x = self.resolve(&args[0])?;
                if n == "task" {
                    Ty::Task(Box::new(x))
                } else {
                    self.require_sendable(&x, args[0].span, "チャンネルで送る値")?;
                    Ty::Chan(Box::new(x))
                }
            }
            TypeKind::App(n, args) => {
                let want = match (self.structs.get(n.as_str()), self.enums.get(n.as_str())) {
                    (Some((d, _)), _) => d.tparams.len(),
                    (_, Some((d, _))) => d.tparams.len(),
                    _ => {
                        return Err(Diagnostic::new(format!("未定義の型 `{}`", n))
                            .primary(t.span, "この型は定義されていません"))
                    }
                };
                if args.len() != want {
                    return Err(Diagnostic::new(format!("`{}` の型の引数の数が違います", n))
                        .primary(t.span, format!("{} 個必要ですが {} 個書かれています", want, args.len())));
                }
                let args = args.iter().map(|a| self.resolve(a)).collect::<R<Vec<_>>>()?;
                // Recheck fixed arrays after substituting concrete generic arguments.
                // Checking T[N] at its definition cannot determine the size of T.
                if let Some((def, fields)) = self.structs.get(n.as_str()) {
                    let map = Self::targs(&def.tparams, &args);
                    for field in fields { self.check_size(&subst(field, &map), t.span)?; }
                    Ty::Struct(n.clone(), args)
                } else {
                    let result = Ty::Enum(n.clone(), args);
                    if let Some(variants) = self.variants_of(&result) {
                        for (_, fields) in variants {
                            for field in fields { self.check_size(&field, t.span)?; }
                        }
                    }
                    result
                }
            }
            TypeKind::Ref(inner, m) => Ty::Ref(Box::new(self.resolve_pointee(inner)?), *m),
            TypeKind::Array(inner, n) => {
                let a = Ty::Array(Box::new(self.resolve(inner)?), *n);
                self.check_size(&a, t.span)?;
                a
            }
            TypeKind::List(inner) => Ty::List(Box::new(self.resolve(inner)?)),
            TypeKind::Map(v, k) => {
                let kt = self.resolve(k)?;
                if !matches!(kt, Ty::Int(_) | Ty::Str | Ty::Bool) {
                    return Err(Diagnostic::new(format!("`{}` は辞書のキーにできません", kt.show()))
                        .primary(k.span, "キーにできるのは整数・str・bool だけ"));
                }
                Ty::Map(Box::new(kt), Box::new(self.resolve(v)?))
            }
            TypeKind::Fn(ps, r) => {
                let ps = ps.iter().map(|p| self.resolve(p)).collect::<R<Vec<_>>>()?;
                let r = match r {
                    Some(r) => self.resolve(r)?,
                    None => Ty::Unit,
                };
                Ty::Fn(ps, Box::new(r))
            }
            TypeKind::Name(n) => match n.as_str() {
                "bool" => Ty::Bool,
                "()" => Ty::Unit,
                "str" => Ty::Str,
                "f64" => Ty::Float,
                _ if IntTy::from_name(n).is_some() => Ty::Int(IntTy::from_name(n).expect("checked")),
                "isize" | "f32" => {
                    return Err(Diagnostic::new(format!("型 `{}` はまだ使えません", n))
                        .primary(t.span, "今使える数の型は整数（i8〜i64、u8〜u64、usize）と f64 だけです"))
                }
                _ if self.tparams.contains(n) => Ty::Param(n.clone()),
                _ if self.interfaces.contains_key(n.as_str()) => {
                    return Err(Diagnostic::new(format!("interface `{}` は own か ref を通して使います", n))
                        .primary(t.span, format!("`own {}` か `ref {}` と書いてください", n, n))
                        .note("中身の構造体によって大きさが違うので、そのままの値としては持てません"))
                }
                _ if self.structs.contains_key(n.as_str()) || self.enums.contains_key(n.as_str()) => {
                    let tps = match self.structs.get(n.as_str()) {
                        Some((d, _)) => &d.tparams,
                        None => &self.enums[n.as_str()].0.tparams,
                    };
                    if !tps.is_empty() {
                        return Err(Diagnostic::new(format!("`{}` には型の引数が必要です", n))
                            .primary(t.span, format!("`{}<{}>` のように書いてください", n, vec!["i64"; tps.len()].join(", "))));
                    }
                    if self.structs.contains_key(n.as_str()) {
                        Ty::Struct(n.clone(), Vec::new())
                    } else {
                        Ty::Enum(n.clone(), Vec::new())
                    }
                }
                _ => {
                    return Err(Diagnostic::new(format!("未定義の型 `{}`", n))
                        .primary(t.span, "この型は定義されていません")
                        .note("使える型: i8〜i64、u8〜u64、usize、bool、str、構造体の名前、own 型、ref 型、ref mut 型"))
                }
            },
        })
    }

    /// 構造体や enum が自分自身を（own / ref を通さず）じかに含んでいないか。含むと大きさが無限になる
    fn check_finite(&self, name: &str, span: Span) -> R<()> {
        let mut stack = vec![name.to_string()];
        let mut seen: Vec<String> = Vec::new();
        while let Some(n) = stack.pop() {
            let mut direct = Vec::new();
            for t in self.contents(&n) {
                by_value_names(t, &mut direct);
            }
            for m in direct {
                if m == name {
                    return Err(Diagnostic::new(format!("`{}` が自分自身をじかに含んでいます", name))
                        .primary(span, "これだと大きさが無限になる")
                        .note("`own` を通して持ってください（例: `own Node next;`）"));
                }
                if !seen.contains(&m) {
                    seen.push(m.clone());
                    stack.push(m);
                }
            }
        }
        Ok(())
    }

    /// 構造体のフィールド、または enum のすべての種類の中身の型
    fn contents(&self, name: &str) -> Vec<&Ty> {
        if let Some((_, tys)) = self.structs.get(name) {
            return tys.iter().collect();
        }
        match self.enums.get(name) {
            Some((_, vs)) => vs.iter().flatten().collect(),
            None => Vec::new(),
        }
    }

    /// ほかのスレッドへ渡せる型か（中に ref・関数の値・型の引数・interface を含まない）。だめなら理由
    fn sendable(&self, t: &Ty, seen: &mut Vec<String>) -> Result<(), String> {
        match t {
            Ty::Int(_) | Ty::Float | Ty::Bool | Ty::Str | Ty::Unit | Ty::Never | Ty::Chan(_) => Ok(()),
            Ty::Ref(..) => Err(format!("`{}` は ref なので、ほかのスレッドへは渡せません", t.show())),
            Ty::Fn(..) => Err("関数の値は、ほかのスレッドへは渡せません".to_string()),
            Ty::Param(p) => Err(format!("型の引数 `{}` は ref かもしれないので、ほかのスレッドへは渡せません", p)),
            Ty::Iface(i) => Err(format!("interface `{}` の中身は分からないので、ほかのスレッドへは渡せません", i)),
            Ty::Own(x) | Ty::Array(x, _) | Ty::List(x) | Ty::Task(x) => self.sendable(x, seen),
            Ty::Map(k, v) => self.sendable(k, seen).and_then(|_| self.sendable(v, seen)),
            Ty::Struct(n, args) | Ty::Enum(n, args) => {
                let key = t.show();
                if seen.contains(&key) {
                    return Ok(());
                }
                seen.push(key);
                let inner: Vec<Ty> = if matches!(t, Ty::Struct(..)) {
                    let (d, tys) = &self.structs[n.as_str()];
                    let map = Self::targs(&d.tparams, args);
                    tys.iter().map(|x| subst(x, &map)).collect()
                } else {
                    self.variants_of(t).expect("enum").into_iter().flat_map(|(_, f)| f).collect()
                };
                for x in &inner {
                    self.sendable(x, seen).map_err(|why| format!("`{}` の中: {}", t.show(), why))?;
                }
                Ok(())
            }
        }
    }

    fn require_sendable(&self, t: &Ty, span: Span, what: &str) -> R<()> {
        self.sendable(t, &mut Vec::new()).map_err(|why| {
            Diagnostic::new(format!("{}に `{}` は使えません", what, t.show()))
                .primary(span, why)
                .note("スレッドに渡せるのは own とコピーできる値（数・文字列・構造体など、中に ref のないもの）だけです")
        })
    }

    /// 値として読んだときにコピーされる型か（own や構造体を中に持ちうるならムーブ）。
    /// 型の引数（`T`）は分からないのでコピーとは言えない
    fn copyable(&self, t: &Ty) -> bool {
        match t {
            Ty::Int(_) | Ty::Float | Ty::Bool | Ty::Str | Ty::Ref(..) | Ty::Unit | Ty::Never => true,
            Ty::Own(_) | Ty::Struct(..) | Ty::Param(_) | Ty::Iface(_) => false,
            // 要素がコピーできる配列はコピー（Rust と同じ）
            Ty::Array(x, _) => self.copyable(x),
            // リストはいつもムーブ（中身はヒープにあり、複製は重いので）
            Ty::List(_) | Ty::Map(..) => false,
            // 関数の値は取り込んだ own を持ちうるのでムーブ
            Ty::Fn(..) => false,
            // スレッドを待てるのは 1 か所だけ。チャンネルはコピーしても同じものを指す
            Ty::Task(_) => false,
            Ty::Chan(_) => true,
            // check_finite が自分をじかに含む enum を先に弾いているので、再帰は止まる
            Ty::Enum(..) => self.variants_of(t).expect("enum").iter().flat_map(|(_, f)| f).all(|t| self.copyable(t)),
        }
    }

    /// 定義の型の引数と、実際の型の引数の対応
    fn targs(tparams: &[String], args: &[Ty]) -> HashMap<String, Ty> {
        tparams.iter().cloned().zip(args.iter().cloned()).collect()
    }

    /// enum の種類の一覧: (名前, 中身の型)。型の引数は置き換え済み
    /// 値の大きさ（8 バイトの数で数えたおよその数。CELLS_CAP で頭打ち）。固定長の配列とその中の構造体・enum だけを中まで数える。
    /// jwc/typeck.jw の `cells` と同じ数え方（途中で読んでいる enum は中身なし、深さ 32 で打ち切り）
    fn cells(&self, t: &Ty, depth: usize) -> u64 {
        if depth > 32 {
            return 1;
        }
        match t {
            Ty::Array(x, n) => cap_mul(*n, self.cells(x, depth + 1)),
            Ty::Struct(n, args) => match self.structs.get(n.as_str()) {
                Some((def, fields)) => {
                    let map = Self::targs(&def.tparams, args);
                    fields.iter().fold(0, |a, f| cap_add(a, self.cells(&subst(f, &map), depth + 1))).max(1)
                }
                None => 1,
            },
            Ty::Enum(..) => match self.variants_of(t) {
                Some(vs) => cap_add(
                    vs.iter().map(|(_, fs)| fs.iter().fold(0, |a, f| cap_add(a, self.cells(f, depth + 1)))).max().unwrap_or(0),
                    1,
                ),
                None => 1,
            },
            _ => 1,
        }
    }

    /// 固定長の配列は 1 MiB まで（値なので、実行ファイルではスタックに置く）
    fn check_size(&self, t: &Ty, span: Span) -> R<()> {
        match t {
            Ty::Array(..) if self.cells(t, 0) > ARRAY_CELLS => Err(Diagnostic::new("配列が大きすぎます")
                .primary(span, format!("`{}` は 1 MiB を超える", t.show()))
                .note("固定長の配列は 1 MiB まで（値なので、実行ファイルではスタックに置きます）。大きいものはリスト（`i64[]` など）にしてください")),
            _ => Ok(()),
        }
    }

    fn variants_of(&self, t: &Ty) -> Option<Vec<(String, Vec<Ty>)>> {
        match t {
            Ty::Enum(n, args) => {
                let (def, tys) = &self.enums[n.as_str()];
                let map = Self::targs(&def.tparams, args);
                Some(
                    def.variants
                        .iter()
                        .zip(tys)
                        .map(|(v, t)| (v.name.clone(), t.iter().map(|x| subst(x, &map)).collect()))
                        .collect(),
                )
            }
            _ => None,
        }
    }

    /// 型の引数を決めながら、引数（や中身）を調べる。`params` は定義の側の型（`T` を含みうる）。
    /// `pre` は (戻り値などの型, 入れる先の型) の組。先に見比べて型の引数を決める手がかりにする。
    /// 数や `None` のように相手しだいで型が決まる式は、ほかの引数で型の引数が決まってから調べる
    fn instantiate(
        &mut self,
        tparams: &[String],
        params: &[Ty],
        args: &[&Expr],
        pre: &[(&Ty, &Ty)],
        auto_ref: bool,
    ) -> R<HashMap<String, Ty>> {
        let mut map = HashMap::new();
        for (pat, hint) in pre {
            unify(pat, hint, tparams, &mut map);
        }
        let mut got: Vec<Option<Ty>> = vec![None; args.len()];
        // 0: ふつうの式、1: 数の式、2: `None` など enum の値（中身の型を相手に合わせたいもの）
        let order = |a: &Expr| match a.kind {
            ExprKind::Variant { .. } | ExprKind::Lambda(_) => 2,
            _ if is_const_tree(a) => 1,
            _ => 0,
        };
        for pass in 0..3 {
            for (i, (a, p)) in args.iter().zip(params).enumerate() {
                if got[i].is_some() || order(a) != pass {
                    continue;
                }
                let hint = match p {
                    _ if !has_unbound(p, tparams, &map) => Some(subst(p, &map)),
                    // クロージャの引数の型だけ決まっていれば、それを渡す（戻り値の型は本体から決まる。`Never` は「分からない」）
                    Ty::Fn(ps, _) if matches!(a.kind, ExprKind::Lambda(_)) && !ps.iter().any(|x| has_unbound(x, tparams, &map)) => {
                        Some(Ty::Fn(ps.iter().map(|x| subst(x, &map)).collect(), Box::new(Ty::Never)))
                    }
                    _ => None,
                };
                let hint = if auto_ref { hint.map(|h| arg_hint(a, h)) } else { hint };
                let t = self.expr_h(a, hint.as_ref())?;
                if t == Ty::Unit && !matches!(a.kind, ExprKind::Unit) {
                    return Err(Diagnostic::new("値のない式です").primary(a.span, "これは値を返さない"));
                }
                let t = if auto_ref { self.auto_borrow(a, t, p)? } else { t };
                unify(p, &t, tparams, &mut map);
                got[i] = Some(t);
            }
        }
        for ((a, p), t) in args.iter().zip(params).zip(&got) {
            let t = t.as_ref().expect("all arguments are checked");
            if has_unbound(p, tparams, &map) {
                continue;
            }
            let want = subst(p, &map);
            if !self.fits(t, &want) {
                return Err(mismatch(&want, t, a.span));
            }
            convert(a, t, &want);
        }
        self.pending_targs = Some(map.clone());
        Ok(map)
    }

    /// 型の引数がすべて決まったか。決まらなければエラー
    fn all_bound(&self, tparams: &[String], map: &HashMap<String, Ty>, span: Span, what: &str) -> R<Vec<Ty>> {
        tparams
            .iter()
            .map(|p| {
                map.get(p).cloned().ok_or_else(|| {
                    Diagnostic::new(format!("{} の型の引数 `{}` が決まりません", what, p))
                        .primary(span, "何の型か分からない")
                        .note("`let Option<i64> x = None;` のように、入れる先の型を書いてください")
                })
            })
            .collect()
    }

    /// `Shape.Circle(5)` / `Some(x)` / `None` の型。`hint` は値を入れる先の型
    fn variant_expr(&mut self, e: &Expr, hint: Option<&Ty>) -> R<Ty> {
        let ExprKind::Variant { enum_name, variant, args, copy } = &e.kind else { unreachable!() };
        let (def, tys) = &self.enums[enum_name.as_str()];
        let def: &EnumDef = def;
        let Some(i) = def.variants.iter().position(|v| v.name == *variant) else {
            let names: Vec<&str> = def.variants.iter().map(|v| v.name.as_str()).collect();
            return Err(Diagnostic::new(format!("`{}` に種類 `{}` はありません", enum_name, variant))
                .primary(e.span, "この種類は定義されていません")
                .note(format!("`{}` の種類: {}", enum_name, names.join(", "))));
        };
        let params = tys[i].clone();
        if args.len() != params.len() {
            return Err(arity_error(&format!("{}.{}", enum_name, variant), params.len(), args.len(), e.span));
        }
        let tparams = def.tparams.clone();
        let generic = Ty::Enum(enum_name.clone(), tparams.iter().map(|p| Ty::Param(p.clone())).collect());
        let pre = match hint {
            Some(h @ Ty::Enum(n, _)) if n == enum_name => Some((&generic, h)),
            _ => None,
        };
        let args: Vec<&Expr> = args.iter().collect();
        let map = self.instantiate(&tparams, &params, &args, &pre.into_iter().collect::<Vec<_>>(), false)?;
        let targs = match self.all_bound(&tparams, &map, e.span, enum_name) {
            Ok(t) => t,
            Err(_) if enum_name == "Option" => {
                return Err(Diagnostic::new(format!("{} の型が決まりません", variant))
                    .primary(e.span, "何の Option か分からない")
                    .note("`let Option<i64> x = None;` のように型を書いてください"))
            }
            Err(d) => return Err(d),
        };
        let ty = Ty::Enum(enum_name.clone(), targs);
        // 型の引数を含む（ジェネリックな関数の中）なら、実行時に中身を見て決める
        copy.set(if contains_param(&ty) { None } else { Some(self.copyable(&ty)) });
        Ok(ty)
    }

    /// enum（または Option）の match。全部の種類を書いたかも調べる
    fn match_enum(
        &mut self,
        e: &Expr,
        st: &Ty,
        arms: &[MatchArm],
        default: Option<&Block>,
        want_value: bool,
        hint: Option<&Ty>,
    ) -> R<Ty> {
        // `match x`（値）か `match ref x` / ref の変数（借りる）か
        let (ety, by_ref) = match st {
            Ty::Ref(t, m) => ((**t).clone(), Some(*m)),
            Ty::Own(t) => ((**t).clone(), None),
            t => (t.clone(), None),
        };
        let variants = self.variants_of(&ety).expect("caller checked");
        let mut tys = Vec::new();
        let mut hint = hint.cloned();
        for arm in arms {
            self.scopes.push(HashMap::new());
            let r = self.match_arm(&ety, by_ref, arm);
            let r = r.and_then(|_| self.block(&arm.body, want_value, hint.as_ref()));
            self.scopes.pop();
            let t = r?;
            // `case Some(x) -> Some(x); case None -> None;` の None の型を前の case から決める
            if hint.is_none() && matches!(&t, Ty::Enum(_, a) if !a.is_empty()) {
                hint = Some(t.clone());
            }
            tys.push((t, &arm.body));
        }
        if let Some(d) = default {
            tys.push((self.block(d, want_value, hint.as_ref())?, d));
        } else {
            let pats: Vec<&Pat> = arms.iter().flat_map(|a| a.pats.iter().map(|(p, _)| p)).collect();
            let missing: Vec<&str> =
                variants.iter().map(|(n, _)| n.as_str()).filter(|n| !self.variant_covered(&ety, n, &pats)).collect();
            if !missing.is_empty() {
                let partly = missing.iter().any(|m| pats.iter().any(|p| matches!(p, Pat::Variant(n, ..) if n == m)));
                let note = if partly {
                    "重ねた形で一部だけ書いた種類があります。`case Some(x) -> ...` のように残りを受ける case か、`default -> ...` を足してください"
                } else {
                    "case を足すか、`default -> ...` でまとめてください"
                };
                return Err(Diagnostic::new(format!("match で {} の場合が書かれていません", missing.join("・")))
                    .primary(e.span, format!("`{}` の種類をすべて書く必要がある", ety.show()))
                    .note(note));
            }
        }
        self.join(tys, None, want_value, "match", hint.as_ref())
    }

    /// enum の case 1 つ分。受け取る名前を今のスコープに置く
    fn match_arm(&mut self, ety: &Ty, by_ref: Option<bool>, arm: &MatchArm) -> R<()> {
        let several = arm.pats.len() > 1;
        for (pat, span) in &arm.pats {
            if several {
                if let Some((_, bspan)) = pat.names().first() {
                    return Err(Diagnostic::new("種類をまとめた case では中身を受け取れません")
                        .primary(*bspan, "ここ")
                        .note("case を分けるか、`_` にしてください"));
                }
            }
            self.variant_pat(ety, by_ref, pat, *span)?;
        }
        Ok(())
    }

    /// enum の種類の形 1 つ（重ねた形の中も）。受け取る名前を今のスコープに置く
    fn variant_pat(&mut self, ety: &Ty, by_ref: Option<bool>, pat: &Pat, span: Span) -> R<()> {
        let Pat::Variant(name, binds, subs) = pat else {
            return Err(Diagnostic::new("enum の match の case には種類の名前を書きます")
                .primary(span, format!("`{}` の種類ではない", ety.show())));
        };
        let variants = self.variants_of(ety).expect("caller checked");
        let Some((_, fields)) = variants.iter().find(|(n, _)| n == name) else {
            let names: Vec<&str> = variants.iter().map(|(n, _)| n.as_str()).collect();
            return Err(Diagnostic::new(format!("`{}` に種類 `{}` はありません", ety.show(), name))
                .primary(span, "この種類はない")
                .note(format!("種類: {}", names.join(", "))));
        };
        if binds.len() != fields.len() {
            return Err(Diagnostic::new(format!("`{}` の中身の数が違います", name))
                .primary(span, format!("中身は {} 個ですが、{} 個の名前が書かれています", fields.len(), binds.len()))
                .note("受け取らない中身には `_` と書いてください"));
        }
        for (((b, bspan), sub), ft) in binds.iter().zip(subs).zip(fields) {
            if let Some(sub) = sub {
                self.sub_pat(ft, by_ref, sub, *bspan)?;
                continue;
            }
            if b == "_" {
                continue;
            }
            let t = match by_ref {
                None => ft.clone(),
                Some(_) if self.copyable(ft) => ft.clone(),
                Some(m) => match ft {
                    Ty::Own(inner) => Ty::Ref(inner.clone(), m),
                    _ => {
                        return Err(Diagnostic::new("借りて match するとき、この中身は受け取れません")
                            .primary(*bspan, format!("中身は `{}`（own を通さずに持つ値）", ft.show()))
                            .note("中身を `own` で持たせるか、`_` で受け取らないでください"))
                    }
                },
            };
            self.declare(b, VarInfo { ty: t, mutable: false, decl: *bspan })?;
        }
        Ok(())
    }

    /// 中身の位置に重ねて書いた形（`Some(Op(op))` の `Op(op)`、`Some(0)` の `0`）
    fn sub_pat(&mut self, ft: &Ty, by_ref: Option<bool>, sub: &Pat, span: Span) -> R<()> {
        let lit = match sub {
            Pat::Int(v) => match ft {
                Ty::Int(t) if t.contains(*v) => Some(ft.clone()),
                Ty::Int(t) => return Err(out_of_range(*v, *t, span)),
                _ => Some(I64),
            },
            Pat::Bool(_) => Some(Ty::Bool),
            Pat::Str(_) => Some(Ty::Str),
            Pat::Variant(..) => None,
        };
        if let Some(lt) = lit {
            if lt != *ft {
                return Err(Diagnostic::new("case の値と中身の型が合いません")
                    .primary(span, format!("中身は `{}`、この形は `{}`", ft.show(), lt.show())));
            }
            return Ok(());
        }
        let inner = match ft {
            Ty::Enum(..) => ft.clone(),
            // 借りた match なら own の先も重ねて書ける（中身はやはり借りて受け取る）
            Ty::Own(x) if matches!(**x, Ty::Enum(..)) && by_ref.is_some() => (**x).clone(),
            Ty::Own(x) if matches!(**x, Ty::Enum(..)) => {
                return Err(Diagnostic::new("値の match では、own の中に形を重ねられません")
                    .primary(span, format!("中身は `{}`", ft.show()))
                    .note("`match ref x` で借りて match すれば重ねて書けます"))
            }
            _ => {
                return Err(Diagnostic::new("この中身には形を重ねられません")
                    .primary(span, format!("中身は `{}`（enum でも数・真偽値・文字列でもない）", ft.show())))
            }
        };
        self.variant_pat(&inner, by_ref, sub, span)
    }

    /// 種類 `variant` が、case の形 `pats` のどれかに必ず当たるか（重ねた形は、1 か所だけ重ねたものを中まで調べる）
    fn variant_covered(&self, ety: &Ty, variant: &str, pats: &[&Pat]) -> bool {
        let mine: Vec<&Pat> = pats.iter().copied().filter(|p| matches!(p, Pat::Variant(n, ..) if n == variant)).collect();
        if mine.iter().any(|p| p.is_flat()) {
            return true;
        }
        let Some((_, fields)) = self.variants_of(ety).and_then(|vs| vs.into_iter().find(|(n, _)| n == variant)) else {
            return false;
        };
        // どの形も同じ 1 か所だけ重ねているなら、その位置の形をまとめて調べる
        for (k, ft) in fields.iter().enumerate() {
            let only_k = mine.iter().all(|p| match p {
                Pat::Variant(_, _, subs) => subs.iter().enumerate().all(|(i, s)| s.is_none() || i == k),
                _ => false,
            });
            if !only_k {
                continue;
            }
            let at_k: Vec<&Pat> = mine
                .iter()
                .filter_map(|p| match p {
                    Pat::Variant(_, _, subs) => subs.get(k).and_then(Option::as_ref),
                    _ => None,
                })
                .collect();
            return self.pats_cover(ft, &at_k);
        }
        false
    }

    /// 型 `t` の値が、形 `pats` のどれかに必ず当たるか
    fn pats_cover(&self, t: &Ty, pats: &[&Pat]) -> bool {
        let t = match t {
            Ty::Own(x) => &**x,
            t => t,
        };
        match t {
            Ty::Bool => pats.contains(&&Pat::Bool(true)) && pats.contains(&&Pat::Bool(false)),
            Ty::Enum(..) => match self.variants_of(t) {
                Some(vs) => vs.iter().all(|(n, _)| self.variant_covered(t, n, pats)),
                None => false,
            },
            _ => false,
        }
    }

    fn check_fn(&mut self, f: &FnDef) -> R<()> {
        let sig = &self.fns[f.name.as_str()];
        // `geom.area` / `geom.Point.len` は geom の中の関数
        self.module = self.modules.iter().map(String::as_str).find(|m| {
            f.name.len() > m.len() && f.name.starts_with(m) && f.name.as_bytes()[m.len()] == b'.'
        });
        self.ret = sig.ret.clone();
        self.info.fn_ret.insert(f.name.clone(), sig.ret.clone());
        self.tparams = f.tparams.clone();
        self.bounds = f.tparams.iter().cloned().zip(f.bounds.iter().cloned()).collect();
        self.fields = match (sig.method, f.name.rsplit_once('.')) {
            (true, Some((owner, _))) => self.structs.get(owner).map(|(d, _)| d.fields.clone()).unwrap_or_default(),
            _ => Vec::new(),
        };
        let mut params = HashMap::new();
        for (p, t) in f.params.iter().zip(sig.params.clone()) {
            if params.contains_key(&p.name) {
                return Err(Diagnostic::new(format!("引数 `{}` が二回あります", p.name))
                    .primary(p.span, "同じ名前の引数がすでにある"));
            }
            self.not_a_field(&p.name, p.span)?;
            self.not_a_fn_name(&p.name, p.span)?;
            self.info.var_ty.insert(p.span, t.clone());
            params.insert(p.name.clone(), VarInfo { ty: t, mutable: p.mutable, decl: p.span });
        }
        self.scopes = vec![params];
        let ret = self.ret.clone();
        let wants = ret != Ty::Unit;
        let got = self.block(&f.body, wants, Some(&ret))?;
        if wants && !self.fits(&got, &ret) {
            if got == Ty::Unit {
                return Err(Diagnostic::new("値を返さないまま関数が終わる道があります")
                    .primary(f.body.end, format!("ここで終わるが、`{}` の値を返していない", ret.show()))
                    .note("最後に `;` のない式を書くか、`return 値;` で返してください"));
            }
            let span = match f.body.stmts.last() {
                Some(Stmt::Expr(e)) => e.span,
                _ => f.body.end,
            };
            return Err(mismatch(&ret, &got, span));
        }
        if let Some(t) = tail_expr(&f.body) {
            convert(t, &got, &ret);
        }
        Ok(())
    }

    fn lookup(&self, name: &str) -> Option<&VarInfo> {
        self.scopes.iter().rev().find_map(|s| s.get(name))
    }

    /// 変数を宣言する。シャドーイングは禁止なので、見えている名前と同じならエラー
    fn declare(&mut self, name: &str, info: VarInfo) -> R<()> {
        if let Some(old) = self.lookup(name) {
            return Err(Diagnostic::new(format!("`{}` はすでにある名前です", name))
                .label(old.decl, "ここで宣言した")
                .primary(info.decl, "同じ名前で宣言し直している")
                .note("シャドーイング（同じ名前で宣言し直すこと）はできません。別の名前をつけてください"));
        }
        self.not_a_field(name, info.decl)?;
        self.not_a_fn_name(name, info.decl)?;
        self.info.var_ty.insert(info.decl, info.ty.clone());
        self.scopes.last_mut().expect("scope").insert(name.to_string(), info);
        Ok(())
    }

    /// 変数に関数と同じ名前はつけられない（`f(x)` がどちらの呼び出しか分からなくなるため）
    fn not_a_fn_name(&self, name: &str, span: Span) -> R<()> {
        let full = match self.module {
            Some(m) => format!("{}.{}", m, name),
            None => name.to_string(),
        };
        if self.consts.contains(&full) {
            return Err(Diagnostic::new(format!("`{}` は定数の名前です", name))
                .primary(span, "変数や引数に、定数と同じ名前はつけられない")
                .note("別の名前をつけてください"));
        }
        if self.modules.iter().any(|m| m == name) {
            return Err(Diagnostic::new(format!("`{}` はモジュールの名前です", name))
                .primary(span, "変数や引数に、モジュールと同じ名前はつけられない")
                .note("別の名前をつけてください"));
        }
        // モジュールの中では、モジュール名をつける前の関数の名前も同じ（`area(x)` は `geom.area(x)` のこと）
        let in_module = matches!(self.module, Some(m) if self.fns.contains_key(format!("{}.{}", m, name).as_str()));
        if self.fns.contains_key(name) || in_module || BUILTINS.contains(&name) {
            return Err(Diagnostic::new(format!("`{}` は関数の名前です", name))
                .primary(span, "変数や引数に、関数と同じ名前はつけられない")
                .note("別の名前をつけてください"));
        }
        Ok(())
    }

    /// 変数がどのスコープにあるか（外から数えた位置）
    fn var_index(&self, name: &str) -> Option<usize> {
        self.scopes.iter().rposition(|s| s.contains_key(name))
    }

    /// 式の中で変数を使う。クロージャの外の変数なら、そのクロージャが取り込むものとして覚える。
    /// 引数に書いたクロージャ（`borrows`）は、ムーブする値を借りて取り込むので、中では `ref T` に見える
    fn use_var(&mut self, name: &str) -> Option<VarInfo> {
        let i = self.var_index(name)?;
        let mut info = self.scopes[i].get(name).cloned()?;
        for k in 0..self.lambdas.len() {
            if i >= self.lambdas[k].base {
                continue;
            }
            let borrow = self.lambdas[k].borrows && self.borrowable(&info.ty);
            if borrow {
                info.ty = match info.ty {
                    Ty::Own(x) => Ty::Ref(x, false),
                    t => Ty::Ref(Box::new(t), false),
                };
            }
            let l = &mut self.lambdas[k];
            if !l.caps.iter().any(|c| c == name) {
                l.caps.push(name.to_string());
                l.borrowed.push(borrow);
            }
        }
        Some(info)
    }

    /// 引数に書いたクロージャが、取り込むときに借りる型か（ムーブする値。関数の値と task は今までどおり）
    fn borrowable(&self, t: &Ty) -> bool {
        !self.copyable(t) && !matches!(t, Ty::Fn(..) | Ty::Task(_))
    }

    /// 今いるクロージャが取り込んだ（外の）変数か
    fn is_captured(&self, name: &str) -> bool {
        match (self.var_index(name), self.lambdas.last()) {
            (Some(i), Some(l)) => i < l.base,
            _ => false,
        }
    }

    /// `(a, b) -> 本体`。引数の型は書いてあればそれ、なければ入れる先の関数の型（`hint`）から決める
    fn lambda(&mut self, l: &Lambda, hint: Option<&Ty>) -> R<Ty> {
        let (hps, hret) = match hint {
            // 戻り値が Never の手がかりは「引数の型だけ分かっている」（`a.map(f)`）
            Some(Ty::Fn(ps, r)) if ps.len() == l.params.len() && **r == Ty::Never => (Some(ps.clone()), None),
            Some(Ty::Fn(ps, r)) if ps.len() == l.params.len() => (Some(ps.clone()), Some((**r).clone())),
            _ => (None, None),
        };
        let mut ptys = Vec::new();
        for (i, p) in l.params.iter().enumerate() {
            ptys.push(match (&p.ty, &hps) {
                (Some(te), _) => self.resolve(te)?,
                (None, Some(ps)) => ps[i].clone(),
                (None, None) => {
                    return Err(Diagnostic::new(format!("クロージャの引数 `{}` の型が分かりません", p.name))
                        .primary(p.span, "型を決める手がかりがない")
                        .note("`(i64 a) -> ...` のように型を書くか、関数の型が決まっている場所（引数など）で使ってください"))
                }
            });
        }
        let base = self.scopes.len();
        self.lambdas.push(LambdaScope { base, caps: Vec::new(), borrowed: Vec::new(), borrows: l.borrows.get() });
        self.scopes.push(HashMap::new());
        let saved_ret = std::mem::replace(&mut self.ret, hret.clone().unwrap_or(Ty::Never));
        let r = self.lambda_body(l, &ptys, hret.as_ref());
        self.ret = saved_ret;
        self.scopes.truncate(base);
        let scope = self.lambdas.pop().expect("pushed above");
        *l.captures.borrow_mut() = scope.caps;
        *l.borrowed.borrow_mut() = scope.borrowed;
        let r = r?;
        l.returns.set(r != Ty::Unit);
        Ok(Ty::Fn(ptys, Box::new(r)))
    }

    /// クロージャの本体を調べて、戻り値の型を返す
    fn lambda_body(&mut self, l: &Lambda, ptys: &[Ty], ret: Option<&Ty>) -> R<Ty> {
        for (p, t) in l.params.iter().zip(ptys) {
            self.declare(&p.name, VarInfo { ty: t.clone(), mutable: false, decl: p.span })?;
        }
        let Some(ret) = ret else {
            // 戻り値の型は本体の最後の式から決める（この場合 return は使えない）
            let got = self.block(&l.body, true, None)?;
            return Ok(if got == Ty::Never { Ty::Unit } else { got });
        };
        if *ret == Ty::Unit {
            self.block(&l.body, false, None)?;
            return Ok(Ty::Unit);
        }
        let got = self.block(&l.body, true, Some(ret))?;
        if !self.fits(&got, ret) {
            let span = tail_expr(&l.body).map_or(l.body.end, |t| t.span);
            return Err(mismatch(ret, &got, span));
        }
        if let Some(t) = tail_expr(&l.body) {
            convert(t, &got, ret);
        }
        Ok(ret.clone())
    }

    /// 関数の値（変数 `name` の型 `ty`）を呼ぶ
    fn call_value(&mut self, name: &str, ty: &Ty, args: &[Expr], span: Span) -> R<Ty> {
        let (ps, ret) = match ty {
            Ty::Fn(ps, r) => (ps.clone(), (**r).clone()),
            Ty::Own(x) | Ty::Ref(x, _) if matches!(**x, Ty::Fn(..)) => {
                let Ty::Fn(ps, r) = &**x else { unreachable!() };
                (ps.clone(), (**r).clone())
            }
            _ => {
                return Err(Diagnostic::new(format!("`{}` は関数ではありません", name))
                    .primary(span, format!("`{}` は `{}` です", name, ty.show())))
            }
        };
        if args.len() != ps.len() {
            return Err(arity_error(name, ps.len(), args.len(), span));
        }
        for (a, p) in args.iter().zip(&ps) {
            let got = self.expr_h(a, Some(&arg_hint(a, p.clone())))?;
            let got = self.auto_borrow(a, got, p)?;
            if !self.fits(&got, p) {
                return Err(mismatch(p, &got, a.span));
            }
            convert(a, &got, p);
        }
        Ok(ret)
    }

    /// メソッドの中では、フィールドと同じ名前の引数や変数を作れない
    fn not_a_field(&self, name: &str, span: Span) -> R<()> {
        if self.fields.iter().any(|f| f == name) {
            return Err(Diagnostic::new(format!("`{}` はフィールドの名前です", name))
                .primary(span, "メソッドの中では、フィールドと同じ名前の引数や変数を作れない")
                .note(format!("フィールドは `{}` のまま使えます。引数や変数には別の名前をつけてください", name)));
        }
        Ok(())
    }

    /// ブロックの型。`want_value` なら最後の `;` のない式の型（なければ値なし）。
    /// 途中で必ず抜けるなら Never
    /// `hint` は値を入れる先の型（数の書き方をそれに合わせる）
    fn block(&mut self, b: &Block, want_value: bool, hint: Option<&Ty>) -> R<Ty> {
        self.scopes.push(HashMap::new());
        let r = self.block_inner(b, want_value, hint);
        self.scopes.pop();
        r
    }

    fn block_inner(&mut self, b: &Block, want_value: bool, hint: Option<&Ty>) -> R<Ty> {
        let mut diverges = false;
        for (i, s) in b.stmts.iter().enumerate() {
            match s {
                Stmt::Expr(e) if want_value && b.tail && i + 1 == b.stmts.len() => {
                    let t = self.expr_ctx(e, true, hint)?;
                    return Ok(if diverges { Ty::Never } else { t });
                }
                _ => diverges |= self.stmt(s)?,
            }
        }
        Ok(if diverges { Ty::Never } else { Ty::Unit })
    }

    /// 文を調べる。必ずそこで抜ける（return など）なら true
    fn stmt(&mut self, s: &Stmt) -> R<bool> {
        match s {
            Stmt::Let { name, ty, mutable, span, init } => {
                let want = match ty {
                    Some(te) => Some(self.resolve(te)?),
                    None => None,
                };
                let got = self.expr_h(init, want.as_ref())?;
                if got == Ty::Unit {
                    return Err(Diagnostic::new("値のない式は変数に入れられません")
                        .primary(init.span, "これは値を返さない"));
                }
                let t = match (ty, want) {
                    (Some(te), Some(want)) => {
                        if !self.fits(&got, &want) {
                            return Err(mismatch(&want, &got, init.span).label(te.span, "ここで型を決めた"));
                        }
                        convert(init, &got, &want);
                        want
                    }
                    _ => got,
                };
                self.declare(name, VarInfo { ty: t, mutable: *mutable, decl: *span })?;
                Ok(false)
            }
            Stmt::Assign { target, value, .. } => {
                let want = self.expr(target)?;
                self.access(target)?.require(target.span, "書き換えようとした")?;
                self.expect(value, &want)?;
                Ok(false)
            }
            Stmt::Expr(e) => Ok(self.expr_ctx(e, false, None)? == Ty::Never),
            Stmt::While { cond, body } => {
                self.expect(cond, &Ty::Bool)?;
                self.block(body, false, None)?;
                Ok(false)
            }
            Stmt::Loop { body } => {
                self.block(body, false, None)?;
                Ok(!has_break(body))
            }
            Stmt::For { var, range, range_span, body, els } => {
                // 始めと終わりは同じ整数型にそろえる（それがループ変数の型）。刻みはいつも i64（負にもなるため）
                let ends = &range[..range.len().min(2)];
                let mut t: Option<IntTy> = None;
                for a in ends.iter().filter(|a| !is_const_tree(a)) {
                    let at = match self.expr(a)? {
                        Ty::Int(at) => at,
                        other => return Err(mismatch(&I64, &other, a.span)),
                    };
                    t = match t {
                        None => Some(at),
                        Some(prev) => match IntTy::common(prev, at) {
                            Some(c) => Some(c),
                            None => {
                                return Err(Diagnostic::new("range の始めと終わりの型が違います")
                                    .primary(*range_span, format!("`{}` と `{}`", prev.name(), at.name()))
                                    .note("`as` で型をそろえてください"))
                            }
                        },
                    };
                }
                let t = Ty::Int(t.unwrap_or(IntTy::I64));
                for a in ends {
                    self.expect(a, &t)?;
                }
                if let Some(step) = range.get(2) {
                    self.expect(step, &I64)?;
                }
                self.scopes.push(HashMap::new());
                self.declare(var, VarInfo { ty: t, mutable: false, decl: *range_span })?;
                let r = self.block(body, false, None);
                self.scopes.pop();
                r?;
                if let Some(els) = els {
                    self.block(els, false, None)?;
                    self.warn_for_else(body);
                }
                Ok(false)
            }
            Stmt::ForEach { key, var, var_span, iter, body, els, consume } => {
                let t = self.expr(iter)?;
                // その場で作ったリスト（`a.drain()`・関数の戻り値）は、要素を値として受け取る
                consume.set(matches!(t, Ty::List(_)) && !is_place(iter));
                // 前の名前の型（リストなら添字、辞書ならキー）と、要素の型
                let (kt, elem) = match (seq_of(&t), map_of(&t)) {
                    (Some((elem, _)), _) => (I64, elem),
                    (_, Some((k, v))) => {
                        if key.is_none() {
                            return Err(Diagnostic::new("辞書は `for (k, v) in m` で回します")
                                .primary(*var_span, "キーと値の 2 つの名前を書く")
                                .note("キーだけなら `for k in m.keys()`"));
                        }
                        (k, v)
                    }
                    _ => {
                        return Err(Diagnostic::new("for で回せるのは配列・リスト・辞書です")
                            .primary(iter.span, format!("ここは `{}` です", t.show()))
                            .note("数を回すなら `for i in range(10)` と書きます"))
                    }
                };
                let vt = if consume.get() { elem } else { self.view_ty(elem) };
                self.scopes.push(HashMap::new());
                if let Some((k, ks)) = key {
                    self.declare(k, VarInfo { ty: kt, mutable: false, decl: *ks })?;
                }
                self.declare(var, VarInfo { ty: vt, mutable: false, decl: *var_span })?;
                let r = self.block(body, false, None);
                self.scopes.pop();
                r?;
                if let Some(els) = els {
                    self.block(els, false, None)?;
                    self.warn_for_else(body);
                }
                Ok(false)
            }
            Stmt::Break(_) | Stmt::Continue(_) => Ok(true),
            Stmt::Return { value, span } => {
                let ret = self.ret.clone();
                if ret == Ty::Never {
                    // 戻り値の型の手がかりがないクロージャ（戻り値は最後の式で決める）
                    return Err(Diagnostic::new("このクロージャでは return を使えません")
                        .primary(*span, "戻り値の型が決まっていない")
                        .note("最後の式（`;` なし）で値を返すか、関数の型が決まっている場所で使ってください"));
                }
                match (value, &ret) {
                    (None, Ty::Unit) => {}
                    (None, _) => {
                        return Err(Diagnostic::new("return に値がありません")
                            .primary(*span, format!("この関数は `{}` を返します", ret.show())))
                    }
                    (Some(e), Ty::Unit) => {
                        return Err(Diagnostic::new("値を返さない関数で値を return しています")
                            .primary(e.span, "この値の行き先がない")
                            .note("値を返すなら `fn i64 f()` のように戻り値の型を書いてください"))
                    }
                    (Some(e), _) => self.expect(e, &ret)?,
                }
                Ok(true)
            }
        }
    }

    /// 型 `want` の場所で使う式。整数は欠けない変換なら自動で変換する
    fn expect(&mut self, e: &Expr, want: &Ty) -> R<()> {
        let got = self.expr_h(e, Some(want))?;
        if self.fits(&got, want) {
            convert(e, &got, want);
            Ok(())
        } else {
            Err(mismatch(want, &got, e.span))
        }
    }

    fn expr(&mut self, e: &Expr) -> R<Ty> {
        self.expr_ctx(e, true, None)
    }

    /// `hint` は値を入れる先の型（数の書き方をそれに合わせる）
    fn expr_h(&mut self, e: &Expr, hint: Option<&Ty>) -> R<Ty> {
        self.expr_ctx(e, true, hint)
    }

    /// 式の型。`want_value` が false なら、if / match は文として使われている（値を捨てる）。
    /// `hint` は値を入れる先の型。整数の定数（`300` や `1 + 2`）はその型になる（なければ i64）
    fn expr_ctx(&mut self, e: &Expr, want_value: bool, hint: Option<&Ty>) -> R<Ty> {
        self.pending_targs = None;
        let t = self.expr_kind(e, want_value, hint)?;
        if matches!(e.kind, ExprKind::Field(..) | ExprKind::Index(..) | ExprKind::Deref(..)) {
            e.takes_own.set(holds_own(&t));
        }
        // 同じ式を何度か調べることがある（警告のための調べ直しなど）。最後に調べた結果を残す
        self.info.expr_ty.insert(e as *const Expr, t.clone());
        let instantiates = matches!(
            e.kind,
            ExprKind::Call(..) | ExprKind::MethodCall(..) | ExprKind::StructLit(..) | ExprKind::Variant { .. }
        );
        match self.pending_targs.take() {
            Some(m) if instantiates => {
                self.info.targs.insert(e as *const Expr, m);
            }
            _ => {}
        }
        Ok(t)
    }

    fn expr_kind(&mut self, e: &Expr, want_value: bool, hint: Option<&Ty>) -> R<Ty> {
        if is_const_tree(e) {
            if let Some(Ty::Float) = hint {
                // 整数の書き方でも、小数の場所では小数として計算する（`let f64 x = 7 / 2;` は 3.5）
                set_float_tree(e);
                return Ok(Ty::Float);
            }
            let t = match hint {
                Some(Ty::Int(t)) => *t,
                _ => IntTy::I64,
            };
            return self.const_expr(e, t);
        }
        match &e.kind {
            ExprKind::Int(_) => unreachable!("integer literals are constant trees"),
            ExprKind::Float(_) => Ok(Ty::Float),
            ExprKind::Try(inner) => self.try_expr(inner, e.span, hint),
            ExprKind::Index(base, idx, _) => {
                let t = self.expr(base)?;
                if let Some((k, v)) = map_of(&t) {
                    self.expect(idx, &k)?;
                    return Ok(v);
                }
                self.index_value(idx)?;
                let (elem, len) = match seq_of(&t) {
                    Some(x) => x,
                    None => return Err(not_an_array(&t, base.span)),
                };
                // リストの長さは実行するまで分からないので、定数の添字を調べるのは配列だけ
                let Some(len) = len else { return Ok(elem) };
                if is_const_tree(idx) {
                    if let Ok(v) = const_val(idx) {
                        if v < 0 || v >= len as i128 {
                            return Err(Diagnostic::new(format!("添字 {} は配列の範囲の外です", v))
                                .primary(idx.span, format!("`{}` の添字は 0〜{} です", t.show(), len as i128 - 1)));
                        }
                    }
                }
                Ok(elem)
            }
            ExprKind::ArrayLit(items, seq) => {
                if let (Some(Ty::Map(..)), true) = (hint, items.is_empty()) {
                    // 入れる先が辞書なら、空の `[]` は空の辞書
                    seq.set(Seq::Map);
                    return Ok(hint.expect("matched").clone());
                }
                if let Some(Ty::List(w)) = hint {
                    // 入れる先がリストなら、リストとして作る
                    seq.set(Seq::List);
                    for x in items {
                        self.expect(x, w)?;
                    }
                    return Ok(Ty::List(w.clone()));
                }
                let want = match hint {
                    Some(Ty::Array(x, n)) if *n == items.len() as u64 => Some((**x).clone()),
                    _ => None,
                };
                if let Some(w) = want {
                    for x in items {
                        self.expect(x, &w)?;
                    }
                    return Ok(Ty::Array(Box::new(w), items.len() as u64));
                }
                if items.is_empty() {
                    return Err(Diagnostic::new("空の配列の型が決まりません")
                        .primary(e.span, "要素の型が分からない")
                        .note("`let i64[0] a = [];` のように型を書いてください"));
                }
                // 要素の型をそろえる（数の書き方は、ほかの要素に合わせる）
                let mut tys: Vec<Option<Ty>> = vec![None; items.len()];
                let mut acc: Option<Ty> = None;
                for (i, x) in items.iter().enumerate() {
                    if is_const_tree(x) {
                        continue;
                    }
                    let t = self.value(x)?;
                    acc = Some(match acc {
                        None => t.clone(),
                        Some(Ty::Int(a)) => match (&t, IntTy::common(a, if let Ty::Int(b) = &t { *b } else { a })) {
                            (Ty::Int(_), Some(c)) => Ty::Int(c),
                            _ => return Err(mismatch(&Ty::Int(a), &t, x.span)),
                        },
                        Some(a) if self.fits(&t, &a) => a,
                        Some(a) => return Err(mismatch(&a, &t, x.span)),
                    });
                    tys[i] = Some(t);
                }
                let elem = acc.unwrap_or(I64);
                for (x, t) in items.iter().zip(tys) {
                    match t {
                        None => {
                            self.expect(x, &elem)?;
                        }
                        Some(t) => convert(x, &t, &elem),
                    }
                }
                let a = Ty::Array(Box::new(elem), items.len() as u64);
                self.check_size(&a, e.span)?;
                Ok(a)
            }
            ExprKind::MapLit(pairs) => {
                let (kt, vt) = match hint {
                    Some(Ty::Map(k, v)) => ((**k).clone(), (**v).clone()),
                    // 入れる先の型がなければ、最初の組から決める
                    _ => {
                        let (k, v) = &pairs[0];
                        let kt = self.value(k)?;
                        let vt = self.value(v)?;
                        if !matches!(kt, Ty::Int(_) | Ty::Str | Ty::Bool) {
                            return Err(Diagnostic::new(format!("`{}` は辞書のキーにできません", kt.show()))
                                .primary(k.span, "キーにできるのは整数・str・bool だけ"));
                        }
                        (kt, vt)
                    }
                };
                for (k, v) in pairs {
                    self.expect(k, &kt)?;
                    self.expect(v, &vt)?;
                }
                Ok(Ty::Map(Box::new(kt), Box::new(vt)))
            }
            ExprKind::ArrayRepeat(x, n, list) => {
                let h = match hint {
                    Some(Ty::Array(x, _)) => Some((**x).clone()),
                    Some(Ty::List(x)) => {
                        list.set(true);
                        Some((**x).clone())
                    }
                    _ => None,
                };
                let t = match &h {
                    Some(w) => {
                        self.expect(x, w)?;
                        w.clone()
                    }
                    None => self.value(x)?,
                };
                if !self.copyable(&t) {
                    return Err(Diagnostic::new(format!("`{}` は複製できないので並べられません", t.show()))
                        .primary(x.span, "own や構造体は 1 つしかない")
                        .note("`[a, b, c]` のように 1 つずつ書いてください"));
                }
                if list.get() {
                    // リストの中身は 2^62 バイトまで（数の掛け算があふれないように。それより小さくても、確保できなければ実行時に止まる）
                    if cap_mul(*n, self.cells(&t, 0)) > LIST_CELLS {
                        return Err(Diagnostic::new("リストが大きすぎます")
                            .primary(e.span, format!("{} 個は作れない", n))
                            .note("リストの中身は 2^62 バイトまで"));
                    }
                    return Ok(Ty::List(Box::new(t)));
                }
                let a = Ty::Array(Box::new(t), *n);
                self.check_size(&a, e.span)?;
                Ok(a)
            }
            ExprKind::Interp(parts) => {
                // print と同じく、どんな値でも文字にできる
                for p in parts {
                    if let InterpPart::Expr(x) = p {
                        self.value(x)?;
                    }
                }
                Ok(Ty::Str)
            }
            ExprKind::Cast(inner, te) => {
                let target = self.resolve(te)?;
                if !matches!(target, Ty::Int(_) | Ty::Float) {
                    return Err(Diagnostic::new("`as` で変換できるのは数の型（整数と f64）だけです")
                        .primary(te.span, format!("`{}` は数の型ではありません", target.show())));
                }
                let got = self.expr(inner)?;
                if !matches!(got, Ty::Int(_) | Ty::Float) {
                    return Err(Diagnostic::new("`as` で変換できるのは数（整数と f64）だけです")
                        .primary(inner.span, format!("ここは `{}` です", got.show())));
                }
                Ok(target)
            }
            ExprKind::Bool(_) => Ok(Ty::Bool),
            ExprKind::Unit => Ok(Ty::Unit),
            ExprKind::Str(_) => Ok(Ty::Str),
            ExprKind::Lambda(l) => self.lambda(l, hint),
            ExprKind::Var(name) => match self.use_var(name) {
                Some(v) => Ok(v.ty),
                // 名前のある関数を値として使う（`v.sort_by(less)`）
                None if self.fns.contains_key(name.as_str()) => {
                    let sig = &self.fns[name.as_str()];
                    if sig.method || !sig.tparams.is_empty() {
                        return Err(Diagnostic::new(format!("`{}` は値として使えません", name))
                            .primary(e.span, "メソッドや型の引数のある関数は、まだ値にできない")
                            .note("`(a) -> f(a)` のようにクロージャで包んでください"));
                    }
                    Ok(Ty::Fn(sig.params.clone(), Box::new(sig.ret.clone())))
                }
                None => Err(Diagnostic::new(format!("未定義の変数 `{}`", name))
                    .primary(e.span, "この名前は見つかりません")),
            },
            ExprKind::Own(inner) => {
                let inner_hint = match hint {
                    Some(Ty::Own(t)) => Some(&**t),
                    _ => None,
                };
                let t = self.expr_h(inner, inner_hint)?;
                if t == Ty::Unit {
                    return Err(Diagnostic::new("値のない式です").primary(inner.span, "これは値を返さない"));
                }
                Ok(Ty::Own(Box::new(t)))
            }
            ExprKind::Ref(inner, mutable) => {
                if !is_place(inner) {
                    return Err(Diagnostic::new("借用できない式です")
                        .primary(inner.span, "`ref` は変数か `*p` か `p.f` にだけ使えます")
                        .note("`let x = own ...;` で変数に入れてから `ref x` としてください"));
                }
                let t = self.expr(inner)?;
                // own なら先を、ref なら同じ先を借りる。それ以外の値は、その場所そのものを借りる
                let target = match t.pointee() {
                    Some(x) => x.clone(),
                    None => {
                        if let Some(at) = self.inside_map(inner)? {
                            return Err(crate::interp::map_borrow_error(at));
                        }
                        t.clone()
                    }
                };
                if *mutable {
                    // own を ref mut で借りるには持ち主が mut、ref から借り直すにはその ref が ref mut
                    let origin = self.origin(inner);
                    self.access(inner)?.through(&t, origin, describe(inner)).require(e.span, "ref mut で借りようとした")?;
                }
                Ok(Ty::Ref(Box::new(target), *mutable))
            }
            ExprKind::Deref(inner, _) => {
                let t = self.expr(inner)?;
                match t.pointee() {
                    Some(x) => Ok(x.clone()),
                    None => Err(Diagnostic::new("`*` でたどれるのは own と ref だけです")
                        .primary(inner.span, format!("ここは `{}` です", t.show()))),
                }
            }
            ExprKind::Field(base, name, _) => {
                let t = self.expr(base)?;
                let (sname, targs) = match &t {
                    Ty::Struct(n, a) => (n, a),
                    Ty::Own(x) | Ty::Ref(x, _) => match &**x {
                        Ty::Struct(n, a) => (n, a),
                        _ => return Err(not_a_struct(&t, base.span)),
                    },
                    _ => return Err(not_a_struct(&t, base.span)),
                };
                let (def, tys) = &self.structs[sname.as_str()];
                match def.field_index(name) {
                    Some(i) => Ok(subst(&tys[i], &Self::targs(&def.tparams, targs))),
                    None => Err(no_field(def, name, e.span)),
                }
            }
            ExprKind::StructLit(name, inits) => {
                let Some((def, tys)) = self.structs.get(name.as_str()) else {
                    return Err(Diagnostic::new(format!("未定義の構造体 `{}`", name))
                        .primary(e.span, "この構造体は定義されていません"));
                };
                let (def, tys) = (*def, tys.clone());
                let mut seen = vec![false; def.fields.len()];
                let mut params = Vec::new();
                for init in inits {
                    let Some(i) = def.field_index(&init.name) else {
                        return Err(no_field(def, &init.name, init.span));
                    };
                    if seen[i] {
                        return Err(Diagnostic::new(format!("フィールド `{}` が二回書かれています", init.name))
                            .primary(init.span, "ここが二回目"));
                    }
                    seen[i] = true;
                    params.push(tys[i].clone());
                }
                let missing: Vec<&str> =
                    def.fields.iter().zip(&seen).filter(|(_, s)| !**s).map(|(n, _)| n.as_str()).collect();
                if !missing.is_empty() {
                    return Err(Diagnostic::new(format!("`{}` のフィールドが足りません", name))
                        .primary(e.span, format!("{} がありません", missing.join(", "))));
                }
                let generic = Ty::Struct(name.clone(), def.tparams.iter().map(|p| Ty::Param(p.clone())).collect());
                let pre = match hint {
                    Some(h @ Ty::Struct(n, _)) if n == name => Some((&generic, h)),
                    _ => None,
                };
                let values: Vec<&Expr> = inits.iter().map(|i| &i.value).collect();
                let map = self.instantiate(&def.tparams, &params, &values, &pre.into_iter().collect::<Vec<_>>(), false)?;
                let targs = self.all_bound(&def.tparams, &map, e.span, name)?;
                Ok(Ty::Struct(name.clone(), targs))
            }
            ExprKind::If { cond, then, els } => {
                self.expect(cond, &Ty::Bool)?;
                let a = self.block(then, want_value, hint)?;
                // `if c { Some(1) } else { None }` の None の型を then から決める
                let else_hint = match (hint, &a) {
                    (None, Ty::Enum(_, args)) if !args.is_empty() => Some(&a),
                    _ => hint,
                };
                let mut tys = vec![(a.clone(), then)];
                if let Some(els) = els {
                    tys.push((self.block(els, want_value, else_hint)?, els));
                }
                let missing = if els.is_none() { Some(e.span) } else { None };
                self.join(tys, missing, want_value, "if", hint)
            }
            ExprKind::Variant { .. } => self.variant_expr(e, hint),
            ExprKind::Match { scrut, arms, default } => {
                let st = self.expr(scrut)?;
                let enum_like = |t: &Ty| matches!(t, Ty::Enum(..));
                let subject = match &st {
                    Ty::Ref(t, _) | Ty::Own(t) => &**t,
                    t => t,
                };
                if enum_like(subject) {
                    return self.match_enum(e, &st, arms, default.as_ref(), want_value, hint);
                }
                if !matches!(st, Ty::Int(_) | Ty::Bool | Ty::Str) {
                    return Err(Diagnostic::new("match できるのは、今のところ整数・真偽値・文字列・enum だけです")
                        .primary(scrut.span, format!("ここは `{}` です", st.show())));
                }
                let mut tys = Vec::new();
                for arm in arms {
                    for (pat, span) in &arm.pats {
                        let pt = match (pat, &st) {
                            (Pat::Int(v), Ty::Int(t)) => {
                                if !t.contains(*v) {
                                    return Err(out_of_range(*v, *t, *span));
                                }
                                st.clone()
                            }
                            (Pat::Int(_), _) => I64,
                            (Pat::Bool(_), _) => Ty::Bool,
                            (Pat::Str(_), _) => Ty::Str,
                            (Pat::Variant(n, ..), _) => {
                                return Err(Diagnostic::new("enum ではない値に、種類の case は使えません")
                                    .label(scrut.span, format!("これは `{}`", st.show()))
                                    .primary(*span, format!("`{}` は種類の名前として読まれた", n)))
                            }
                        };
                        if pt != st {
                            return Err(Diagnostic::new("case の値と match する値の型が合いません")
                                .label(scrut.span, format!("これは `{}`", st.show()))
                                .primary(*span, format!("この case は `{}`", pt.show())));
                        }
                    }
                    tys.push((self.block(&arm.body, want_value, hint)?, &arm.body));
                }
                if let Some(d) = default {
                    tys.push((self.block(d, want_value, hint)?, d));
                }
                // default がなく、どの case にも当たらなければ何もせず抜ける
                let missing = if default.is_none() && !covers_both_bools(arms) { Some(e.span) } else { None };
                self.join(tys, missing, want_value, "match", hint)
            }
            ExprKind::Unary(UnOp::Not, inner) => {
                self.expect(inner, &Ty::Bool)?;
                Ok(Ty::Bool)
            }
            ExprKind::Unary(UnOp::BitNot, inner) => match self.expr_h(inner, hint)? {
                Ty::Int(t) => Ok(Ty::Int(t)),
                other => Err(Diagnostic::new("`~` は整数に使います").primary(inner.span, format!("ここは `{}` です", other.show()))),
            },
            ExprKind::Unary(UnOp::Neg, inner) => match self.expr_h(inner, hint)? {
                Ty::Float => Ok(Ty::Float),
                Ty::Int(t) if t.signed() => Ok(Ty::Int(t)),
                Ty::Int(t) => Err(Diagnostic::new(format!("符号なしの型 `{}` に `-` は使えません", t.name()))
                    .primary(e.span, "ここ")
                    .note("符号つきの型（i64 など）に `as` で変換してください")),
                other => Err(mismatch(&I64, &other, inner.span)),
            },
            ExprKind::Binary(op, l, r) => self.binary(*op, l, r, e.span, hint),
            ExprKind::Call(name, args) => self.call(name, args, e.span, hint),
            ExprKind::MethodCall(recv, name, args, owner) => self.method_call(recv, name, args, owner, e.span, hint),
        }
    }

    /// 値として使う式。値なしならエラー
    fn value(&mut self, e: &Expr) -> R<Ty> {
        let t = self.expr(e)?;
        if t == Ty::Unit && !matches!(e.kind, ExprKind::Unit) {
            return Err(Diagnostic::new("値のない式です").primary(e.span, "これは値を返さない"));
        }
        Ok(t)
    }

    /// if / match の枝の型をまとめる。`missing` は枝がない場合（else / default なし）の位置
    fn join(&self, branches: Vec<(Ty, &Block)>, missing: Option<Span>, want_value: bool, what: &str, hint: Option<&Ty>) -> R<Ty> {
        if !want_value {
            // 文として使うときは値を捨てる。どの枝も必ず抜けるなら、ここで必ず抜ける
            let all_never = missing.is_none() && branches.iter().all(|(t, _)| *t == Ty::Never);
            return Ok(if all_never { Ty::Never } else { Ty::Unit });
        }
        let mut list: Vec<(Ty, Span)> = branches.iter().map(|(t, b)| (t.clone(), tail_span(b))).collect();
        if let Some(sp) = missing {
            list.push((Ty::Unit, sp));
        }
        // 枝ごとに違う構造体でも、入れる先が interface でみな当てはまるなら、その interface にまとめる
        if let Some(h) = hint {
            if contains_iface(h) && list.iter().all(|(t, _)| self.fits(t, h)) {
                return Ok(h.clone());
            }
        }
        let mut acc = Ty::Never;
        let mut first: Option<(Ty, Span)> = None;
        for (t, span) in list {
            if self.fits(&t, &acc) {
                continue;
            }
            if self.fits(&acc, &t) {
                acc = t.clone();
            } else {
                let (ft, fs) = first.clone().expect("acc came from an earlier branch");
                return Err(Diagnostic::new(format!("{} の枝で値の型が違います", what))
                    .label(fs, format!("こちらは `{}`", ft.show()))
                    .primary(span, format!("こちらは `{}`", t.show())));
            }
            if first.is_none() || t != Ty::Never {
                first = Some((t, span));
            }
        }
        // 整数の枝は広い方の型にそろえる
        for (t, b) in &branches {
            if let Some(e) = tail_expr(b) {
                convert(e, t, &acc);
            }
        }
        Ok(acc)
    }

    fn binary(&mut self, op: BinOp, l: &Expr, r: &Expr, span: Span, hint: Option<&Ty>) -> R<Ty> {
        // `a << n` / `a >> n`: 結果は a の型。n はどの整数の型でもよい（数の書き方なら i64）
        if matches!(op, BinOp::Shl | BinOp::Shr) {
            let h = hint.filter(|t| matches!(t, Ty::Int(_)));
            let lt = self.expr_h(l, h)?;
            let rt = self.expr_h(r, None)?;
            return match (&lt, &rt) {
                (Ty::Int(_), Ty::Int(_)) => Ok(lt),
                _ => Err(Diagnostic::new("`<<` と `>>` は整数に使います")
                    .primary(span, format!("`{}` と `{}` です", lt.show(), rt.show()))),
            };
        }
        // 片方が定数（`x + 1` の `1`）なら、もう片方の型に合わせる。算術なら外の型も手がかりにする
        let arith =
            matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem | BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor);
        let outer = if arith { hint } else { None };
        let (lt, rt) = if is_const_tree(l) {
            let rt = self.expr_h(r, outer)?;
            let h = if matches!(rt, Ty::Int(_) | Ty::Float) { Some(&rt) } else { outer };
            (self.expr_h(l, h)?, rt)
        } else {
            let lt = self.expr_h(l, outer)?;
            let h = if matches!(lt, Ty::Int(_) | Ty::Float) { Some(&lt) } else { outer };
            let rt = self.expr_h(r, h)?;
            (lt, rt)
        };
        let bad = || {
            Diagnostic::new("型が合わない演算です")
                .primary(span, format!("`{}` と `{}` にはこの演算を使えません", lt.show(), rt.show()))
        };
        match op {
            BinOp::And | BinOp::Or => {
                if lt != Ty::Bool {
                    return Err(mismatch(&Ty::Bool, &lt, l.span));
                }
                if rt != Ty::Bool {
                    return Err(mismatch(&Ty::Bool, &rt, r.span));
                }
                Ok(Ty::Bool)
            }
            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor => match (&lt, &rt) {
                (Ty::Int(_), Ty::Int(_)) => Ok(Ty::Int(self.common(l, &lt, r, &rt, span)?)),
                _ => Err(bad().note("`&` `|` `^` は整数のビット演算です（真偽値には `&&` `||` を使ってください）")),
            },
            BinOp::Shl | BinOp::Shr => unreachable!("shifts are handled above"),
            BinOp::Add if lt == Ty::Str && rt == Ty::Str => Ok(Ty::Str),
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem => match (&lt, &rt) {
                (Ty::Int(_), Ty::Int(_)) => Ok(Ty::Int(self.common(l, &lt, r, &rt, span)?)),
                (Ty::Float, Ty::Float) => Ok(Ty::Float),
                (Ty::Float, Ty::Int(_)) | (Ty::Int(_), Ty::Float) => {
                    Err(bad().note("整数と小数をまぜて計算するときは、`x as f64` のように型をそろえてください"))
                }
                _ => Err(bad()),
            },
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => match (&lt, &rt) {
                (Ty::Int(_), Ty::Int(_)) => {
                    self.common(l, &lt, r, &rt, span)?;
                    Ok(Ty::Bool)
                }
                (Ty::Str, Ty::Str) | (Ty::Float, Ty::Float) => Ok(Ty::Bool),
                _ if lt == rt && self.is_ordered_param(&lt) => Ok(Ty::Bool),
                (Ty::Param(_), _) | (_, Ty::Param(_)) => Err(bad().note("型の引数どうしを比べるには `<T: ordered>` と条件をつけてください")),
                _ => Err(bad()),
            },
            BinOp::Eq | BinOp::Ne => {
                if let (Ty::Int(_), Ty::Int(_)) = (&lt, &rt) {
                    self.common(l, &lt, r, &rt, span)?;
                    return Ok(Ty::Bool);
                }
                if lt == rt && self.is_ordered_param(&lt) {
                    return Ok(Ty::Bool);
                }
                if lt == rt && matches!(lt, Ty::Bool | Ty::Str | Ty::Float) {
                    Ok(Ty::Bool)
                } else {
                    Err(Diagnostic::new("型が合わない比較です")
                        .primary(span, format!("`{}` と `{}` は比較できません", lt.show(), rt.show())))
                }
            }
        }
    }

    /// `from` の値を `to` の場所に入れられるか（`assignable` に、構造体 → interface を足したもの）
    fn fits(&self, from: &Ty, to: &Ty) -> bool {
        if assignable(from, to) {
            return true;
        }
        match (from, to) {
            (Ty::Ref(a, m), Ty::Ref(b, n)) if *m || !*n => self.fits_iface(a, b),
            (Ty::Own(a), Ty::Own(b)) => self.fits_iface(a, b),
            _ => false,
        }
    }

    /// 構造体（や条件つきの T）を interface として扱えるか
    fn fits_iface(&self, from: &Ty, to: &Ty) -> bool {
        match to {
            Ty::Iface(i) => !matches!(from, Ty::Iface(_)) && self.satisfies(from, i).is_ok(),
            _ => false,
        }
    }

    /// リストのメソッド: `len()`、`push(x)`、`pop()`
    /// 要素を読むだけで受け取るときの型（for の変数・sort_by の比べる関数の引数）。
    /// コピーできればコピー、own なら own の先を借りた ref、それ以外（構造体など）は要素を読むだけの ref
    fn view_ty(&self, elem: Ty) -> Ty {
        if self.copyable(&elem) {
            elem
        } else if let Ty::Own(u) = elem {
            Ty::Ref(u, false)
        } else {
            Ty::Ref(Box::new(elem), false)
        }
    }

    fn list_method(&mut self, recv: &Expr, rt: &Ty, elem: &Ty, name: &str, args: &[Expr], span: Span) -> R<Ty> {
        let (want, ret, mutable) = match name {
            "len" => (0, I64, false),
            "push" => (1, Ty::Unit, true),
            "pop" => (0, Ty::Enum("Option".to_string(), vec![elem.clone()]), true),
            "remove" => (1, elem.clone(), true),
            "insert" => (2, Ty::Unit, true),
            "reverse" => (0, Ty::Unit, true),
            "join" => {
                if *elem != Ty::Str || args.len() != 1 {
                    return Err(Diagnostic::new("join は文字列のリストに使います")
                        .primary(span, format!("`{}` のリストです（`sep` を 1 つ渡します）", elem.show())));
                }
                self.expect(&args[0], &Ty::Str)?;
                return Ok(Ty::Str);
            }
            // 読むだけ。戻り値の型は関数を調べてから決める
            "map" | "filter" => (1, Ty::Unit, false),
            "drain" => (0, Ty::List(Box::new(elem.clone())), true),
            "sort" | "sort_by" | "sort_unstable" | "sort_unstable_by" => (if name.ends_with("_by") { 1 } else { 0 }, Ty::Unit, true),
            _ => {
                return Err(Diagnostic::new(format!("リストにメソッド `{}` はありません", name)).primary(
                    span,
                    "リストのメソッドは `len()`・`push(x)`・`pop()`・`insert(i, x)`・`remove(i)`・`reverse()`・`drain()`・`map(f)`・`filter(f)`・`sort()`・`sort_by(f)`・`sort_unstable()`・`sort_unstable_by(f)`・`join(sep)` です",
                ))
            }
        };
        if args.len() != want {
            return Err(arity_error(name, want, args.len(), span));
        }
        if mutable {
            let a = if is_place(recv) { self.access(recv)? } else { Access::Write };
            a.through(rt, self.origin(recv), describe(recv))
                .require(span, format!("リストを `{}` で書き換えようとした", name).as_str())?;
        }
        // 添字（insert・remove の最初の引数）は整数、要素（push・insert の最後の引数）は要素の型
        let (index, value) = match name {
            "remove" => (args.first(), None),
            "insert" => (args.first(), args.get(1)),
            _ => (None, args.first()),
        };
        if name == "sort" || name == "sort_unstable" {
            if let Err(why) = self.satisfies(elem, ORDERED) {
                return Err(Diagnostic::new("この要素は sort で並べられません")
                    .primary(span, why)
                    .note("並べ方を決める関数を渡す `sort_by((a, b) -> a.x < b.x)` を使ってください"));
            }
        }
        if name == "sort_by" || name == "sort_unstable_by" {
            // 比べる関数: 「a を b より前に置くか」。要素は読むだけで受け取る
            let v = self.view_ty(elem.clone());
            let f = Ty::Fn(vec![v.clone(), v], Box::new(Ty::Bool));
            self.expect(&args[0], &f)?;
            return Ok(ret);
        }
        if name == "filter" {
            // 残す要素はコピーする（構造体や own は drain してから選ぶ）
            if !self.copyable(elem) {
                return Err(Diagnostic::new(format!("`{}` の要素は filter で選べません", elem.show()))
                    .primary(span, "filter は要素をコピーして新しいリストを作る")
                    .note("`for x in a.drain() { if ... { b.push(x); } }` と書いてください"));
            }
            let f = Ty::Fn(vec![elem.clone()], Box::new(Ty::Bool));
            self.expect(&args[0], &f)?;
            return Ok(Ty::List(Box::new(elem.clone())));
        }
        if name == "map" {
            // 関数は要素を読むだけで受け取る（for x in a の x と同じ）。戻り値の型は関数から決める
            let v = self.view_ty(elem.clone());
            let hint = Ty::Fn(vec![v.clone()], Box::new(Ty::Never));
            let ft = self.expr_h(&args[0], Some(&hint))?;
            let r = match &ft {
                Ty::Fn(ps, r) if ps.len() == 1 && self.fits(&v, &ps[0]) => (**r).clone(),
                _ => return Err(mismatch(&Ty::Fn(vec![v], Box::new(Ty::Param("U".to_string()))), &ft, args[0].span)),
            };
            if r == Ty::Unit {
                return Err(Diagnostic::new("map の関数が値を返しません")
                    .primary(args[0].span, "要素ごとの新しい値を返す関数を渡してください"));
            }
            return Ok(Ty::List(Box::new(r)));
        }
        if let Some(i) = index {
            self.index_value(i)?;
        }
        if let Some(a) = value {
            self.expect(a, elem)?;
        }
        Ok(ret)
    }

    /// 辞書のメソッド
    #[allow(clippy::too_many_arguments)]
    fn map_method(&mut self, recv: &Expr, rt: &Ty, k: &Ty, v: &Ty, name: &str, args: &[Expr], span: Span) -> R<Ty> {
        let opt = Ty::Enum("Option".to_string(), vec![v.clone()]);
        let (params, ret, mutable) = match name {
            "len" => (vec![], I64, false),
            "contains" => (vec![k.clone()], Ty::Bool, false),
            "get" => (vec![k.clone()], opt, false),
            "remove" => (vec![k.clone()], opt, true),
            "keys" => (vec![], Ty::List(Box::new(k.clone())), false),
            _ => {
                return Err(Diagnostic::new(format!("辞書にメソッド `{}` はありません", name)).primary(
                    span,
                    "辞書のメソッドは `len()`・`contains(k)`・`get(k)`・`remove(k)`・`keys()` です",
                ))
            }
        };
        if args.len() != params.len() {
            return Err(arity_error(name, params.len(), args.len(), span));
        }
        if name == "get" && !self.copyable(v) {
            return Err(Diagnostic::new(format!("`{}` はコピーできないので get で取り出せません", v.show()))
                .primary(span, "辞書の中の値はそのまま残るので、写しを作れない値は取り出せない")
                .note("`m[k].x` のように場所として読むか、`remove(k)` で取り出してください"));
        }
        if mutable {
            let a = if is_place(recv) { self.access(recv)? } else { Access::Write };
            a.through(rt, self.origin(recv), describe(recv))
                .require(span, format!("辞書を `{}` で書き換えようとした", name).as_str())?;
        }
        for (a, t) in args.iter().zip(&params) {
            self.expect(a, t)?;
        }
        Ok(ret)
    }

    /// `spawn(f)`: 引数のないクロージャか関数の名前を、新しいスレッドで動かす
    fn spawn(&mut self, args: &[Expr], span: Span, hint: Option<&Ty>) -> R<Ty> {
        if args.len() != 1 {
            return Err(arity_error("spawn", 1, args.len(), span));
        }
        let a = &args[0];
        let fn_hint = Ty::Fn(Vec::new(), Box::new(match hint {
            Some(Ty::Task(r)) => (**r).clone(),
            _ => Ty::Unit,
        }));
        let ok_shape = match &a.kind {
            ExprKind::Lambda(_) => true,
            ExprKind::Var(n) => self.lookup(n).is_none() && self.fns.contains_key(n.as_str()),
            _ => false,
        };
        if !ok_shape {
            return Err(Diagnostic::new("spawn にはその場で書いたクロージャか、関数の名前を渡してください")
                .primary(a.span, "ここで取り込んだ値が、ほかのスレッドへ渡せるか分からない")
                .note("`spawn(() -> work(data))` のように書いてください"));
        }
        let hint_ok = matches!(hint, Some(Ty::Task(_)));
        let t = self.expr_h(a, if hint_ok { Some(&fn_hint) } else { None })?;
        let Ty::Fn(ps, ret) = t else { unreachable!("lambda or function") };
        if !ps.is_empty() {
            return Err(Diagnostic::new("spawn に渡す関数は引数を取れません")
                .primary(a.span, format!("引数が {} 個ある", ps.len()))
                .note("値はクロージャに取り込んで渡してください（`spawn(() -> work(x))`）"));
        }
        if let ExprKind::Lambda(l) = &a.kind {
            for c in l.captures.borrow().iter() {
                let Some(v) = self.lookup(c) else { continue };
                let (ty, decl) = (v.ty.clone(), v.decl);
                self.sendable(&ty, &mut Vec::new()).map_err(|why| {
                    Diagnostic::new(format!("`{}` はほかのスレッドへ渡せません", c))
                        .label(decl, format!("`{}` は `{}`", c, ty.show()))
                        .primary(a.span, why)
                        .note("スレッドに渡せるのは own とコピーできる値（中に ref のないもの）だけです")
                })?;
            }
        }
        self.require_sendable(&ret, a.span, "スレッドの戻り値")?;
        Ok(Ty::Task(ret))
    }

    /// `t.join()`: スレッドが終わるのを待って、戻り値を受け取る（1 回だけ）
    fn task_method(&mut self, r: &Ty, name: &str, args: &[Expr], span: Span) -> R<Ty> {
        if name != "join" {
            return Err(Diagnostic::new(format!("task にメソッド `{}` はありません", name))
                .primary(span, "task のメソッドは `join()` だけです"));
        }
        if !args.is_empty() {
            return Err(arity_error(name, 0, args.len(), span));
        }
        Ok(r.clone())
    }

    /// チャンネルのメソッド
    fn chan_method(&mut self, x: &Ty, name: &str, args: &[Expr], span: Span) -> R<Ty> {
        let (params, ret) = match name {
            "send" => (vec![x.clone()], Ty::Unit),
            "recv" => (vec![], Ty::Enum("Option".to_string(), vec![x.clone()])),
            "close" => (vec![], Ty::Unit),
            _ => {
                return Err(Diagnostic::new(format!("chan にメソッド `{}` はありません", name))
                    .primary(span, "chan のメソッドは `send(x)`・`recv()`・`close()` です"))
            }
        };
        if args.len() != params.len() {
            return Err(arity_error(name, params.len(), args.len(), span));
        }
        for (a, t) in args.iter().zip(&params) {
            self.expect(a, t)?;
        }
        Ok(ret)
    }

    /// 文字列のメソッド。位置と長さは文字単位
    fn str_method(&mut self, name: &str, args: &[Expr], span: Span) -> R<Ty> {
        let list = Ty::List(Box::new(Ty::Str));
        let (params, ret): (Vec<Ty>, Ty) = match name {
            "len" | "byte_len" => (vec![], I64),
            "chars" | "lines" => (vec![], list),
            "bytes" => (vec![], Ty::List(Box::new(Ty::Int(IntTy::U8)))),
            "code" => (vec![], I64),
            "split" => (vec![Ty::Str], list),
            "trim" | "to_upper" | "to_lower" => (vec![], Ty::Str),
            "contains" | "starts_with" | "ends_with" => (vec![Ty::Str], Ty::Bool),
            "find" => (vec![Ty::Str], Ty::Enum("Option".to_string(), vec![I64])),
            "slice" => (vec![I64, I64], Ty::Str),
            "replace" => (vec![Ty::Str, Ty::Str], Ty::Str),
            "repeat" => (vec![I64], Ty::Str),
            "to_int" => (vec![], Ty::Enum("Option".to_string(), vec![I64])),
            "to_float" => (vec![], Ty::Enum("Option".to_string(), vec![Ty::Float])),
            _ => {
                return Err(Diagnostic::new(format!("文字列にメソッド `{}` はありません", name))
                    .primary(span, "このメソッドは定義されていません")
                    .note(format!("文字列のメソッド: {}", STR_METHODS)))
            }
        };
        if args.len() != params.len() {
            return Err(arity_error(name, params.len(), args.len(), span));
        }
        for (a, t) in args.iter().zip(&params) {
            self.expect(a, t)?;
        }
        Ok(ret)
    }

    /// 添字の式。整数ならどの型でもよい（数の書き方は i64）
    fn index_value(&mut self, idx: &Expr) -> R<()> {
        match self.expr_h(idx, Some(&I64))? {
            Ty::Int(_) => Ok(()),
            t => Err(Diagnostic::new("添字は整数です").primary(idx.span, format!("ここは `{}` です", t.show()))),
        }
    }

    /// `<T: ordered>` の T か
    fn is_ordered_param(&self, t: &Ty) -> bool {
        matches!(t, Ty::Param(p) if matches!(self.bounds.get(p), Some(b) if b.iter().any(|x| x == ORDERED)))
    }

    /// 型 `t` が条件 `bound` を満たすか。満たさなければその理由
    fn satisfies(&self, t: &Ty, bound: &str) -> Result<(), String> {
        if let Ty::Iface(i) = t {
            return if i == bound { Ok(()) } else { Err(format!("interface `{}` は `{}` ではありません", i, bound)) };
        }
        if let Ty::Param(p) = t {
            // 型の引数のまま渡すなら、こちらの関数にも同じ条件が要る
            let ok = matches!(self.bounds.get(p), Some(b) if b.iter().any(|x| x == bound));
            return if ok { Ok(()) } else { Err(format!("`{}` に条件 `{}` がついていません", p, bound)) };
        }
        if bound == ORDERED {
            return if matches!(t, Ty::Int(_) | Ty::Float | Ty::Str) {
                Ok(())
            } else {
                Err(format!("`{}` は大小を比べられません（ordered は整数・f64・str だけ）", t.show()))
            };
        }
        let Ty::Struct(sname, targs) = t else {
            if matches!(t, Ty::Enum(..)) {
                // enum の値は実行時に型の名前を持たないので、interface のメソッドを探せない
                return Err(format!("interface に当てはめられるのは構造体だけです（`{}` は enum）", t.show()));
            }
            return Err(format!("`{}` は構造体ではないのでメソッドを持ちません", t.show()));
        };
        let map = Self::targs(&self.structs[sname.as_str()].0.tparams, targs);
        for im in &self.interfaces[bound] {
            let full = format!("{}.{}", sname, im.name);
            let Some(sig) = self.fns.get(full.as_str()).filter(|s| s.method) else {
                return Err(format!("`{}` にメソッド `{}` がありません", sname, im.name));
            };
            let Ty::Ref(_, smut) = &sig.params[0] else {
                return Err(format!("`{}.{}` は this を値で受け取る（fn take）ので、interface のメソッドになれません", sname, im.name));
            };
            let params: Vec<Ty> = sig.params[1..].iter().map(|p| subst(p, &map)).collect();
            let shape_ok = sig.tparams.len() == self.structs[sname.as_str()].0.tparams.len()
                && params == im.params
                && subst(&sig.ret, &map) == im.ret;
            if !shape_ok {
                return Err(format!("`{}.{}` の引数か戻り値の型が `{}` の形と違います", sname, im.name, bound));
            }
            if *smut && !im.mutable {
                return Err(format!("`{}.{}` は fn mut ですが、`{}` では書き換えないメソッドです", sname, im.name, bound));
            }
        }
        Ok(())
    }

    /// 呼び出しで決まった型の引数が、それぞれの条件を満たすか
    fn check_bounds(&self, tparams: &[String], bounds: &[Vec<String>], map: &HashMap<String, Ty>, span: Span) -> R<()> {
        for (p, bs) in tparams.iter().zip(bounds) {
            let Some(t) = map.get(p) else { continue };
            for b in bs {
                if let Err(why) = self.satisfies(t, b) {
                    return Err(Diagnostic::new(format!("`{}` は条件 `{}` を満たしません", t.show(), b))
                        .primary(span, format!("ここで `{}` は `{}` になる", p, t.show()))
                        .note(why));
                }
            }
        }
        Ok(())
    }

    /// 型の引数（`T`）や interface の値に対するメソッド呼び出し。`ifaces` の interface から探す
    #[allow(clippy::too_many_arguments)]
    fn iface_method_call(
        &mut self,
        recv: &Expr,
        rt: &Ty,
        ifaces: &[String],
        label: &str,
        help: &str,
        name: &str,
        args: &[Expr],
        span: Span,
    ) -> R<Ty> {
        let found = ifaces.iter().find_map(|b| self.interfaces[b.as_str()].iter().find(|m| m.name == name).cloned());
        let Some(m) = found else {
            return Err(Diagnostic::new(format!("`{}` にメソッド `{}` があるか分かりません", label, name))
                .primary(span, "呼べるのは interface に書いたメソッドだけです")
                .note(help));
        };
        if args.len() != m.params.len() {
            return Err(arity_error(name, m.params.len(), args.len(), span));
        }
        if m.mutable {
            let a = if is_place(recv) { self.access(recv)? } else { Access::Write };
            a.through(rt, self.origin(recv), describe(recv))
                .require(span, format!("書き換えるメソッド `{}` を呼ぼうとした", name).as_str())?;
        }
        for (a, t) in args.iter().zip(&m.params) {
            let got = self.expr_h(a, Some(&arg_hint(a, t.clone())))?;
            let got = self.auto_borrow(a, got, t)?;
            if !self.fits(&got, t) {
                return Err(mismatch(t, &got, a.span));
            }
            convert(a, &got, t);
        }
        Ok(m.ret)
    }

    /// `e?`。e が `Result<T, E>` なら関数の戻り値も `Result<_, E>`、`Option<T>` なら `Option<_>` でなければならない。
    /// 式の型は中身の `T`
    fn try_expr(&mut self, inner: &Expr, span: Span, hint: Option<&Ty>) -> R<Ty> {
        // 中身の型の手がかりを、戻り値の型と合わせて渡す（`let i64 x = Ok(5)?;` の 5 を i64 に）
        let inner_hint = match (&self.ret, hint) {
            (Ty::Enum(n, a), Some(h)) if n == "Result" => Some(Ty::Enum(n.clone(), vec![h.clone(), a[1].clone()])),
            (Ty::Enum(n, _), Some(h)) if n == "Option" => Some(Ty::Enum(n.clone(), vec![h.clone()])),
            _ => None,
        };
        let t = self.expr_h(inner, inner_hint.as_ref())?;
        let kind = match &t {
            Ty::Enum(n, _) if n == "Result" || n == "Option" => n.as_str(),
            _ => {
                return Err(Diagnostic::new("`?` は Result か Option に使います")
                    .primary(inner.span, format!("これは `{}` です", t.show())))
            }
        };
        let Ty::Enum(_, args) = &t else { unreachable!() };
        let usage = match kind {
            "Result" => "`?` は、Result を返す関数の中で使えます（Err ならそのまま返す）",
            _ => "`?` は、Option を返す関数の中で使えます（None ならそのまま返す）",
        };
        match &self.ret {
            Ty::Enum(m, rargs) if m == kind => {
                if kind == "Result" && args[1] != rargs[1] {
                    return Err(Diagnostic::new("エラーの型が合いません")
                        .primary(span, format!("ここの Err は `{}` ですが、関数が返す Err は `{}` です", args[1].show(), rargs[1].show()))
                        .note("`match` で取り出して、関数のエラーの型に作り直してください"));
                }
            }
            r => {
                return Err(Diagnostic::new("ここでは `?` を使えません")
                    .primary(span, format!("この関数の戻り値は `{}` です", r.show()))
                    .note(usage))
            }
        }
        Ok(args[0].clone())
    }

    fn call(&mut self, name: &str, args: &[Expr], span: Span, hint: Option<&Ty>) -> R<Ty> {
        match name {
            "print" | "eprint" => {
                for a in args {
                    self.value(a)?;
                }
                return Ok(Ty::Unit);
            }
            "chr" => {
                if args.len() != 1 {
                    return Err(arity_error(name, 1, args.len(), span));
                }
                self.expect(&args[0], &I64)?;
                return Ok(Ty::Str);
            }
            "exit" | "read_file" | "write_file" | "run_command" => {
                let str_list = Ty::List(Box::new(Ty::Str));
                let (params, ret) = match name {
                    "exit" => (vec![Ty::Int(IntTy::I64)], Ty::Never),
                    "read_file" => (vec![Ty::Str], Ty::Str),
                    "write_file" => (vec![Ty::Str, Ty::Str], Ty::Unit),
                    _ => (vec![Ty::Str, str_list], Ty::Int(IntTy::I64)),
                };
                if args.len() != params.len() {
                    return Err(arity_error(name, params.len(), args.len(), span));
                }
                for (a, p) in args.iter().zip(&params) {
                    self.expect(a, p)?;
                }
                return Ok(if ret == Ty::Never { ret } else { Ty::Enum("Result".to_string(), vec![ret, Ty::Str]) });
            }
            "read_line" | "args" => {
                if !args.is_empty() {
                    return Err(arity_error(name, 0, args.len(), span));
                }
                return Ok(if name == "args" {
                    Ty::List(Box::new(Ty::Str))
                } else {
                    Ty::Enum("Option".to_string(), vec![Ty::Str])
                });
            }
            "spawn" => return self.spawn(args, span, hint),
            "chan" => {
                if !args.is_empty() {
                    return Err(arity_error(name, 0, args.len(), span));
                }
                return match hint {
                    Some(t @ Ty::Chan(_)) => Ok(t.clone()),
                    _ => Err(Diagnostic::new("chan() の型が分かりません")
                        .primary(span, "何を送るチャンネルか決める手がかりがない")
                        .note("`let chan<i64> c = chan();` のように型を書いてください")),
                };
            }
            "drop" | "assert" => {
                if args.len() != 1 {
                    return Err(arity_error(name, 1, args.len(), span));
                }
                if name == "drop" {
                    self.value(&args[0])?;
                } else {
                    self.expect(&args[0], &Ty::Bool)?;
                }
                return Ok(Ty::Unit);
            }
            _ => {}
        }
        if let Some(v) = self.use_var(name) {
            // 関数の値を入れた変数（クロージャ）の呼び出し
            return self.call_value(name, &v.ty, args, span);
        }
        let Some(sig) = self.fns.get(name) else {
            // `Shape.Square(1)`: 種類でもメソッドでもない（パーサは種類でない名前を静的メソッドとして読む）
            if let Some((en, v)) = name.rsplit_once('.') {
                if let Some((def, _)) = self.enums.get(en) {
                    let names: Vec<&str> = def.variants.iter().map(|v| v.name.as_str()).collect();
                    return Err(Diagnostic::new(format!("`{}` に種類 `{}` はありません", en, v))
                        .primary(span, "この種類は定義されていません")
                        .note(format!("`{}` の種類: {}", en, names.join(", "))));
                }
            }
            let d = Diagnostic::new(format!("未定義の関数 `{}`", name)).primary(span, "この関数は定義されていません");
            return Err(if name == "range" { d.note("range(...) は `for i in range(...)` の中でだけ使えます") } else { d });
        };
        if sig.method {
            let (s, m) = name.rsplit_once('.').expect("method name");
            return Err(Diagnostic::new(format!("`{}` は this のあるメソッドです", name))
                .primary(span, "型の名前からは呼べない")
                .note(format!("`{}` の値 p に対して `p.{}()` と呼んでください", s, m)));
        }
        if args.len() != sig.params.len() {
            return Err(arity_error(name, sig.params.len(), args.len(), span));
        }
        let (tparams, params, ret) = (sig.tparams.clone(), sig.params.clone(), sig.ret.clone());
        let bounds = sig.bounds.clone();
        let pre = hint.map(|h| (&ret, h));
        let args: Vec<&Expr> = args.iter().collect();
        let map = self.instantiate(&tparams, &params, &args, &pre.into_iter().collect::<Vec<_>>(), true)?;
        if contains_param(&ret) || params.iter().any(|p| has_unbound(p, &tparams, &map)) {
            self.all_bound(&tparams, &map, span, &format!("`{}`", name))?;
        }
        self.check_bounds(&tparams, &bounds, &map, span)?;
        Ok(subst(&ret, &map))
    }

    /// `p.name(args)`。メソッドは `構造体名.name` という関数で、最初の引数が `this`（`ref 構造体`）
    /// 数のメソッド。f64 は `to_bits` / `sqrt` / `abs` / `max` / `min`、整数は `abs` / `max` / `min`。
    /// `max` / `min` の引数は receiver と同じ型（数の書き方はそれに合わせる）
    fn num_method(&mut self, t: &Ty, name: &str, args: &[Expr], span: Span) -> R<Ty> {
        let float = matches!(t, Ty::Float);
        let (arity, ret) = match name {
            "to_bits" if float => (0, Ty::Int(IntTy::U64)),
            "sqrt" if float => (0, Ty::Float),
            "abs" => (0, t.clone()),
            "max" | "min" => (1, t.clone()),
            _ if float => {
                return Err(Diagnostic::new(format!("f64 にメソッド `{}` はありません", name)).primary(
                    span,
                    "f64 のメソッドは `to_bits()`（ビットの並びを u64 で）・`sqrt()`・`abs()`・`max(y)`・`min(y)` だけです",
                ))
            }
            _ => {
                return Err(Diagnostic::new(format!("`{}` にメソッド `{}` はありません", t.show(), name))
                    .primary(span, "整数のメソッドは `abs()`・`max(y)`・`min(y)` だけです"))
            }
        };
        if args.len() != arity {
            return Err(arity_error(name, arity, args.len(), span));
        }
        for a in args {
            self.expect(a, t)?;
        }
        Ok(ret)
    }

    fn method_call(
        &mut self,
        recv: &Expr,
        name: &str,
        args: &[Expr],
        owner: &RefCell<MethodTarget>,
        span: Span,
        hint: Option<&Ty>,
    ) -> R<Ty> {
        let rt = self.expr(recv)?;
        let inner = match &rt {
            Ty::Own(x) | Ty::Ref(x, _) => (**x).clone(),
            t => t.clone(),
        };
        match &inner {
            // 並べ替えは長さを変えないので、配列にも使える。map / filter は新しいリストを作る
            Ty::Array(elem, _) if matches!(name, "sort" | "sort_by" | "sort_unstable" | "sort_unstable_by" | "map" | "filter") => {
                return self.list_method(recv, &rt, elem, name, args, span)
            }
            Ty::Array(..) => {
                if name != "len" || !args.is_empty() {
                    return Err(Diagnostic::new(format!("配列にメソッド `{}` はありません", name))
                        .primary(span, "配列のメソッドは `len()`（長さ）と `sort()`・`sort_by(f)`・`sort_unstable()`・`sort_unstable_by(f)`・`map(f)`・`filter(f)` だけです")
                        .note("伸び縮みさせたいなら、リスト `i64[]` を使ってください"));
                }
                return Ok(I64);
            }
            Ty::List(elem) => return self.list_method(recv, &rt, elem, name, args, span),
            Ty::Map(k, v) => return self.map_method(recv, &rt, k, v, name, args, span),
            Ty::Str => return self.str_method(name, args, span),
            Ty::Float | Ty::Int(_) => return self.num_method(&inner, name, args, span),
            Ty::Task(r) => return self.task_method(r, name, args, span),
            Ty::Chan(x) => return self.chan_method(x, name, args, span),
            Ty::Param(p) => {
                let ifaces: Vec<String> =
                    self.bounds.get(p).into_iter().flatten().filter(|b| *b != ORDERED).cloned().collect();
                let help = format!("`<{}: 名前>` のように、`{}` を持つ interface を条件にしてください", p, name);
                return self.iface_method_call(recv, &rt, &ifaces, p, &help, name, args, span);
            }
            Ty::Iface(i) => {
                let help = format!("interface `{}` に書いたメソッドだけ呼べます", i);
                return self.iface_method_call(recv, &rt, std::slice::from_ref(i), i, &help, name, args, span);
            }
            _ => {}
        }
        let st = match &rt {
            Ty::Struct(..) | Ty::Enum(..) => rt.clone(),
            Ty::Own(x) | Ty::Ref(x, _) if matches!(**x, Ty::Struct(..) | Ty::Enum(..)) => (**x).clone(),
            _ => {
                return Err(Diagnostic::new("メソッドを呼べるのは構造体と enum だけです")
                    .primary(recv.span, format!("これは `{}` です", rt.show())))
            }
        };
        let (Ty::Struct(sname, _) | Ty::Enum(sname, _)) = &st else { unreachable!() };
        if matches!(st, Ty::Enum(..)) {
            // interp は enum の値から型の名前が分からないので、ここで書いておく
            owner.borrow_mut().enum_name = Some(sname.clone());
        }
        let full = format!("{}.{}", sname, name);
        let Some(sig) = self.fns.get(full.as_str()) else {
            let prefix = format!("{}.", sname);
            let mut names: Vec<&str> = self.fns.keys().filter_map(|k| k.strip_prefix(prefix.as_str())).collect();
            names.sort();
            let d = Diagnostic::new(format!("`{}` にメソッド `{}` はありません", sname, name))
                .primary(span, "このメソッドは定義されていません");
            return Err(if names.is_empty() {
                d.note(format!("`{}` にはメソッドがありません", sname))
            } else {
                d.note(format!("`{}` のメソッド: {}", sname, names.join(", ")))
            });
        };
        if !sig.method {
            return Err(Diagnostic::new(format!("`{}` は静的メソッドです", full))
                .primary(span, "値からは呼べない")
                .note(format!("`{}.{}(...)` と呼んでください", sname, name)));
        }
        if args.len() + 1 != sig.params.len() {
            return Err(arity_error(&format!("{}.{}", sname, name), sig.params.len() - 1, args.len(), span));
        }
        let (tparams, params, ret) = (sig.tparams.clone(), sig.params.clone(), sig.ret.clone());
        let (this_ty, mutable) = match &params[0] {
            Ty::Ref(t, m) => (&**t, *m),
            // `fn take`: receiver を値として渡す（ムーブ）。own / ref の先からは取り出せない
            t => {
                if !matches!(rt, Ty::Struct(..) | Ty::Enum(..)) {
                    return Err(Diagnostic::new(format!("`{}` は this を値で受け取るメソッドです", full))
                        .primary(recv.span, format!("これは `{}` なので、中身を取り出せない", rt.show()))
                        .note("own や ref の先の値には呼べません（構造体のフィールドから値を取り出せないのと同じ）"));
                }
                owner.borrow_mut().take = true;
                (t, false)
            }
        };
        if mutable {
            // 書き換えるメソッド（`fn mut`）は、`ref mut` で借りられる相手にだけ呼べる
            let a = if is_place(recv) { self.access(recv)? } else { Access::Write };
            a.through(&rt, self.origin(recv), describe(recv))
                .require(span, format!("書き換えるメソッド `{}` を呼ぼうとした", name).as_str())?;
        }
        let mut pre: Vec<(&Ty, &Ty)> = vec![(this_ty, &st)];
        if let Some(h) = hint {
            pre.push((&ret, h));
        }
        let args: Vec<&Expr> = args.iter().collect();
        let map = self.instantiate(&tparams, &params[1..], &args, &pre, true)?;
        if contains_param(&ret) || params.iter().any(|p| has_unbound(p, &tparams, &map)) {
            self.all_bound(&tparams, &map, span, &format!("`{}`", name))?;
        }
        let bounds = self.fns[full.as_str()].bounds.clone();
        self.check_bounds(&tparams, &bounds, &map, span)?;
        Ok(subst(&ret, &map))
    }

    /// 二つの整数を同じ型（広い方）にそろえる。欠けない変換でそろわなければエラー
    fn common(&self, l: &Expr, lt: &Ty, r: &Expr, rt: &Ty, span: Span) -> R<IntTy> {
        let (Ty::Int(a), Ty::Int(b)) = (lt, rt) else { unreachable!("both sides are integers") };
        let Some(c) = IntTy::common(*a, *b) else {
            return Err(Diagnostic::new("型の違う整数どうしの演算です")
                .primary(span, format!("`{}` と `{}`", a.name(), b.name()))
                .note("どちらかに値が欠けずにそろえられないので、`as` で型をそろえてください"));
        };
        convert(l, lt, &Ty::Int(c));
        convert(r, rt, &Ty::Int(c));
        Ok(c)
    }

    /// 整数の定数の式（`300`、`-1`、`60 * 60`）を型 `t` として調べる。
    /// 数をそのまま書いて範囲を超えるならエラー、計算した結果が超えるなら警告（実行すると一周する）
    fn const_expr(&mut self, e: &Expr, t: IntTy) -> R<Ty> {
        set_tree(e, t);
        match const_val(e) {
            Err(ConstErr::DivZero(span)) => {
                self.warnings
                    .push(Diagnostic::warning("0 で割っています").primary(span, "実行するとここでエラーになる"));
            }
            Err(ConstErr::Overflow) => self.warn_overflow(e.span, t),
            Ok(v) if !t.contains(v) => {
                let bare = matches!(&e.kind, ExprKind::Int(_))
                    || matches!(&e.kind, ExprKind::Unary(UnOp::Neg, i) if matches!(i.kind, ExprKind::Int(_)));
                if bare {
                    return Err(out_of_range(v, t, e.span));
                }
                self.warn_overflow(e.span, t);
            }
            Ok(_) => {}
        }
        Ok(Ty::Int(t))
    }

    /// for-else の本体に break があるときの警告。Python の for-else（break しなかったら else）と読み違えやすいため
    fn warn_for_else(&mut self, body: &Block) {
        if let Some(at) = find_break(body) {
            self.warnings.push(
                Diagnostic::warning("for の else は、1 回も回らなかったときだけ実行されます")
                    .primary(at, "この break で抜けても抜けなくても、else とは関係ない")
                    .note("Python の for-else（break しなかったら実行）とは違います。break したかを知りたいときは、変数に記録してください"),
            );
        }
    }

    fn warn_overflow(&mut self, span: Span, t: IntTy) {
        self.warnings.push(
            Diagnostic::warning("整数があふれます")
                .primary(span, format!("この計算は {} の範囲を超える", t.name()))
                .note(format!(
                    "実行すると値は一周します（{} の範囲は {}〜{}。最大値 + 1 は最小値になる）",
                    t.name(),
                    t.min(),
                    t.max()
                )),
        );
    }

    /// 場所 `e` に書き込めるか
    /// 引数 `a`（型 `t`）を、引数の型 `want` が `ref T` なら自動で借りる（`ref a` と書いたのと同じ）。
    /// 場所でない式（一時的な値）も借りる。値は呼び出しが終わるまで置いておき、終わったら捨てる。
    /// `ref mut` は呼ぶ側に書いてもらう
    fn auto_borrow(&mut self, a: &Expr, t: Ty, want: &Ty) -> R<Ty> {
        if !matches!(want, Ty::Ref(_, false)) || matches!(t, Ty::Ref(..)) {
            return Ok(t);
        }
        if !is_place(a) {
            if matches!(t, Ty::Unit | Ty::Never | Ty::Fn(..)) {
                return Ok(t);
            }
            a.borrow.set(true);
            let target = match t {
                Ty::Own(x) => *x,
                t => t,
            };
            return Ok(Ty::Ref(Box::new(target), false));
        }
        let target = match t.pointee() {
            Some(x) => x.clone(),
            None => {
                if let Some(at) = self.inside_map(a)? {
                    return Err(crate::interp::map_borrow_error(at));
                }
                t
            }
        };
        a.borrow.set(true);
        Ok(Ty::Ref(Box::new(target), false))
    }

    /// 場所 `e`（own / ref でない値）が、辞書の値の中にあるか。あればその添字の位置。
    /// リストの要素はリストの中身の区画から数えるので、リストの添字まで来たらそこで切れる
    fn inside_map(&mut self, e: &Expr) -> R<Option<Span>> {
        let (base, is_index) = match &e.kind {
            ExprKind::Field(base, ..) => (base, false),
            ExprKind::Index(base, ..) => (base, true),
            _ => return Ok(None),
        };
        // 型をもう一度求める（警告が二重に出ないよう、増えた分は捨てる）
        let n = self.warnings.len();
        let bt = self.expr(base);
        self.warnings.truncate(n);
        match bt? {
            // own / ref の先は別のスロットなので、ここで道筋が切れる
            Ty::Own(x) | Ty::Ref(x, _) => Ok(match &*x {
                Ty::Map(..) if is_index => Some(e.span),
                _ => None,
            }),
            Ty::Map(..) if is_index => Ok(Some(e.span)),
            Ty::List(_) if is_index => Ok(None),
            _ => self.inside_map(base),
        }
    }

    fn access(&mut self, e: &Expr) -> R<Access> {
        Ok(match &e.kind {
            ExprKind::Var(name) => {
                if self.is_captured(name) {
                    return Err(Diagnostic::new(format!("クロージャに取り込んだ `{}` は書き換えられません", name))
                        .primary(e.span, "クロージャは外の変数の値を写して持つので、書き換えても外には届かない")
                        .note("書き換えたいときは、外で `ref mut` を取り、その ref を使ってください"));
                }
                let v = self.lookup(name).expect("typed before");
                if v.mutable {
                    Access::Write
                } else {
                    Access::ImmutVar { name: name.clone(), decl: v.decl }
                }
            }
            ExprKind::Deref(inner, _) => {
                let t = self.expr(inner)?;
                // 取り込んだ `ref mut` の先は書き換えられる（写しても同じ先を指す）
                let captured_ref = matches!(&inner.kind, ExprKind::Var(n) if self.is_captured(n)) && matches!(t, Ty::Ref(..));
                let outer = if is_place(inner) && !captured_ref { self.access(inner)? } else { Access::Write };
                outer.through(&t, self.origin(inner), describe(inner))
            }
            ExprKind::Field(base, ..) | ExprKind::Index(base, ..) => {
                let a = self.access(base)?;
                let t = self.expr(base)?;
                a.through(&t, self.origin(base), describe(base))
            }
            _ => Access::Write,
        })
    }

    /// エラーで「この ref はどこから来たか」として指す位置。変数なら宣言
    fn origin(&self, e: &Expr) -> Span {
        match &e.kind {
            ExprKind::Var(n) => self.lookup(n).map_or(e.span, |v| v.decl),
            _ => e.span,
        }
    }
}

/// 値として取り出すと own を動かすことになる型（own か、own の配列）。構造体・リスト・enum などはムーブする値なので、別に実行時に弾く
fn holds_own(t: &Ty) -> bool {
    match t {
        Ty::Own(_) => true,
        Ty::Array(x, _) => holds_own(x),
        _ => false,
    }
}

/// 整数の数だけでできた式か（`300`、`-1`、`60 * 60`）。値はコンパイルの時点で決まる
pub fn is_const_tree(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Int(_) => true,
        ExprKind::Unary(UnOp::Neg, inner) => is_const_tree(inner),
        ExprKind::Binary(BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem, l, r) => {
            is_const_tree(l) && is_const_tree(r)
        }
        _ => false,
    }
}

/// 定数の式の中の数をすべて小数にする（実行時に小数として計算する）
fn set_float_tree(e: &Expr) {
    e.float.set(true);
    match &e.kind {
        ExprKind::Unary(_, inner) => set_float_tree(inner),
        ExprKind::Binary(_, l, r) => {
            set_float_tree(l);
            set_float_tree(r);
        }
        _ => {}
    }
}

/// 定数の式の中の数をすべて型 `t` にする（実行時もその型で計算して一周する）
fn set_tree(e: &Expr, t: IntTy) {
    e.int.set(Some(t));
    match &e.kind {
        ExprKind::Unary(_, inner) => set_tree(inner, t),
        ExprKind::Binary(_, l, r) => {
            set_tree(l, t);
            set_tree(r, t);
        }
        _ => {}
    }
}

enum ConstErr {
    DivZero(Span),
    Overflow,
}

/// 定数の式の本当の値（一周させない）
fn const_val(e: &Expr) -> Result<i128, ConstErr> {
    match &e.kind {
        ExprKind::Int(n) => Ok(*n),
        ExprKind::Unary(_, inner) => Ok(-const_val(inner)?),
        ExprKind::Binary(op, l, r) => {
            let (a, b) = (const_val(l)?, const_val(r)?);
            match op {
                BinOp::Add => a.checked_add(b),
                BinOp::Sub => a.checked_sub(b),
                BinOp::Mul => a.checked_mul(b),
                BinOp::Div | BinOp::Rem if b == 0 => return Err(ConstErr::DivZero(e.span)),
                BinOp::Div => a.checked_div(b),
                _ => a.checked_rem(b),
            }
            .ok_or(ConstErr::Overflow)
        }
        _ => unreachable!("only constant trees"),
    }
}

fn out_of_range(v: i128, t: IntTy, span: Span) -> Diagnostic {
    Diagnostic::new(format!("`{}` は `{}` に入りません", v, t.name()))
        .primary(span, format!("{} の範囲は {}〜{}", t.name(), t.min(), t.max()))
}

/// ループ本体に、このループを抜ける break があるか（内側のループの break は数えない）
fn has_break(b: &Block) -> bool {
    find_break(b).is_some()
}

/// ループ本体の中で、このループを抜ける最初の break の位置
fn find_break(b: &Block) -> Option<Span> {
    b.stmts.iter().find_map(|s| match s {
        Stmt::Break(at) => Some(*at),
        Stmt::Expr(e) => match &e.kind {
            ExprKind::If { then, els, .. } => find_break(then).or_else(|| els.as_ref().and_then(find_break)),
            ExprKind::Match { arms, default, .. } => {
                arms.iter().find_map(|a| find_break(&a.body)).or_else(|| default.as_ref().and_then(find_break))
            }
            _ => None,
        },
        Stmt::For { els: Some(els), .. } | Stmt::ForEach { els: Some(els), .. } => find_break(els),
        _ => None,
    })
}

/// ブロックの値になる最後の式
fn tail_expr(b: &Block) -> Option<&Expr> {
    match b.stmts.last() {
        Some(Stmt::Expr(e)) if b.tail => Some(e),
        _ => None,
    }
}

/// 型の中に（own / ref を通さず）じかに入っている構造体・enum の名前
fn by_value_names(t: &Ty, out: &mut Vec<String>) {
    match t {
        // 配列は要素をじかに含む
        Ty::Array(x, _) => by_value_names(x, out),
        Ty::Struct(n, args) | Ty::Enum(n, args) => {
            out.push(n.clone());
            // 型の引数も、じかに含まれるかもしれないとみなす（安全側）
            for a in args {
                by_value_names(a, out);
            }
        }
        _ => {}
    }
}

fn tail_span(b: &Block) -> Span {
    match b.stmts.last() {
        Some(Stmt::Expr(e)) if b.tail => e.span,
        _ => b.end,
    }
}

fn is_place(e: &Expr) -> bool {
    matches!(e.kind, ExprKind::Var(_) | ExprKind::Deref(..) | ExprKind::Field(..) | ExprKind::Index(..))
}

/// 配列・リスト（own / ref を 1 段たどる）の要素の型と、配列なら長さ
fn seq_of(t: &Ty) -> Option<(Ty, Option<u64>)> {
    match t {
        Ty::Array(x, n) => Some(((**x).clone(), Some(*n))),
        Ty::List(x) => Some(((**x).clone(), None)),
        Ty::Own(p) | Ty::Ref(p, _) => match &**p {
            Ty::Array(x, n) => Some(((**x).clone(), Some(*n))),
            Ty::List(x) => Some(((**x).clone(), None)),
            _ => None,
        },
        _ => None,
    }
}

/// 辞書（own / ref を 1 段たどる）のキーと値の型
fn map_of(t: &Ty) -> Option<(Ty, Ty)> {
    match t {
        Ty::Map(k, v) => Some(((**k).clone(), (**v).clone())),
        Ty::Own(p) | Ty::Ref(p, _) => match &**p {
            Ty::Map(k, v) => Some(((**k).clone(), (**v).clone())),
            _ => None,
        },
        _ => None,
    }
}

fn not_an_array(t: &Ty, span: Span) -> Diagnostic {
    Diagnostic::new("添字が使えるのは配列とリストだけです").primary(span, format!("ここは `{}` です", t.show()))
}

fn describe(e: &Expr) -> String {
    match &e.kind {
        ExprKind::Var(n) => n.clone(),
        ExprKind::Deref(inner, _) => format!("*{}", describe(inner)),
        ExprKind::Field(base, name, _) => format!("{}.{}", describe(base), name),
        _ => "式".to_string(),
    }
}

fn covers_both_bools(arms: &[MatchArm]) -> bool {
    let has = |b| arms.iter().any(|a| a.pats.iter().any(|(p, _)| *p == Pat::Bool(b)));
    has(true) && has(false)
}

fn not_a_struct(t: &Ty, span: Span) -> Diagnostic {
    Diagnostic::new("フィールドを持たない値です").primary(span, format!("`{}` は構造体ではありません", t.show()))
}

fn no_field(def: &StructDef, name: &str, span: Span) -> Diagnostic {
    Diagnostic::new(format!("`{}` にフィールド `{}` はありません", def.name, name))
        .primary(span, "このフィールドは定義されていません")
        .note(format!("`{}` のフィールド: {}", def.name, def.fields.join(", ")))
}

fn arity_error(name: &str, want: usize, got: usize, span: Span) -> Diagnostic {
    Diagnostic::new(format!("`{}` の引数の数が違います", name))
        .primary(span, format!("{} 個必要ですが {} 個渡されました", want, got))
}

/// `ref T` の引数に書いた値の式（`[1, 2]`・`None`・`5` など）は、`T` を入れる先として調べる（あとで自動で借りる）
fn arg_hint(a: &Expr, p: Ty) -> Ty {
    let literal = matches!(
        a.kind,
        ExprKind::ArrayLit(..) | ExprKind::MapLit(..) | ExprKind::ArrayRepeat(..) | ExprKind::Variant { .. } | ExprKind::StructLit(..)
    ) || is_const_tree(a);
    match p {
        Ty::Ref(x, false) if literal => *x,
        p => p,
    }
}
