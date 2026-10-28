//! モジュール: 1 ファイルが 1 モジュール。`use geom;` で同じフォルダの geom.jw を読む。
//!
//! モジュールの中のトップレベルの名前（関数・構造体・enum・interface）は、`geom.area` のように
//! モジュール名をつけた名前に書き換える（`qualify`）。使う側の `geom.area` はパーサが 1 つの名前にまとめる
//! （`parser::merge_paths`。`pub` でない名前はそこでエラー）。なので、型検査・解析・実行は
//! 全部をまとめた 1 つのプログラムとして扱える。
//! `use` が輪になるのはエラー（読む順が決まらないため。Go と同じ）。
//!
//! パッケージ（alice）のために、モジュールを探すときの問い合わせは 3 種類（`read` に渡す名前）:
//! - `名前`: 最初のパッケージのモジュール（パッケージでないときは同じフォルダの 名前.jw）。読んだモジュールの名前もそのまま
//! - `*名前` / `空間*名前`: 依存先のパッケージ `名前` の src/lib.jw（`空間` は use を書いたパッケージ。読む関数は、
//!   そのパッケージが直接の依存先として書いたものだけ返す）。モジュールの名前は `名前`（そのパッケージの中は `名前` の空間）
//! - `空間/名前`: 依存先のパッケージ `空間` の中のモジュール。モジュールの名前は `空間/名前` で、`use 名前;` と書いたファイルの中の
//!   `名前.x` を `空間/名前.x` に書き換える（`Qualify.aliases`）。なので別のパッケージに同じ名前のモジュールがあってよい
//!
//! 依存先の中の `use` は、まず自分の空間（`空間/名前`）、なければ依存先のパッケージ（`空間*名前`）を探す。最初のパッケージの中の `use` は
//! `名前` → `*名前` の順。`files` の名前は問い合わせた名前のまま（スレッドが同じ問い合わせで読み直せるように）。

use std::collections::{HashMap, HashSet};

use crate::ast::*;
use crate::diag::{Diagnostic, Span};
use crate::lexer::{lex_file, Tok};
use crate::parser::{ModuleInfo, NameKind, Parser};

type R<T> = Result<T, Diagnostic>;

/// ファイルを読みながら、モジュールを順に解析する
pub struct Loader<'a> {
    read: &'a dyn Fn(&str) -> Option<String>,
    /// 読んだファイル（モジュール名、ソース）。番号が `Span.file`。0 番が最初のファイル（名前は空）
    pub files: Vec<(String, String)>,
    infos: HashMap<String, ModuleInfo>,
    /// 今読んでいる途中のモジュール（輪を見つけるため）
    loading: Vec<String>,
    /// 依存先のパッケージの lib.jw として読んだモジュール（その中の `use` は自分の空間から探す）
    libs: HashSet<String>,
    parts: Vec<Program>,
}

impl<'a> Loader<'a> {
    /// `read` はモジュール名からソースを返す（見つからなければ None）
    pub fn new(read: &'a dyn Fn(&str) -> Option<String>) -> Self {
        Loader { read, files: Vec::new(), infos: HashMap::new(), loading: Vec::new(), libs: HashSet::new(), parts: Vec::new() }
    }

    /// 最初のファイルと、そこから `use` でたどれるモジュールをすべて読み、1 つのプログラムにする
    pub fn load(&mut self, src: &str) -> R<Program> {
        self.files.push((String::new(), src.to_string()));
        self.module("", 0)?;
        let mut all = Program {
            structs: Vec::new(),
            enums: Vec::new(),
            interfaces: Vec::new(),
            fns: Vec::new(),
            uses: Vec::new(),
            public: Vec::new(),
            // 変数につけられない名前: モジュールの名前と、`空間/名前` の書いた名前
            modules: {
                let mut ms: Vec<String> = Vec::new();
                for (n, _) in self.files.iter().skip(1) {
                    let n = n.trim_start_matches('*');
                    ms.push(n.to_string());
                    if let Some((_, short)) = n.split_once('/') {
                        if !ms.iter().any(|m| m == short) {
                            ms.push(short.to_string());
                        }
                    }
                }
                ms
            },
            consts: Vec::new(),
        };
        for p in self.parts.drain(..) {
            all.structs.extend(p.structs);
            all.enums.extend(p.enums);
            all.interfaces.extend(p.interfaces);
            all.fns.extend(p.fns);
            all.consts.extend(p.consts);
        }
        Ok(all)
    }

    /// `file` 番目のファイルを、モジュール `name` として読む（使うモジュールを先に読む）
    fn module(&mut self, name: &str, file: usize) -> R<ModuleInfo> {
        let src = self.files[file].1.clone();
        let toks = lex_file(&src, file)?;
        let uses: Vec<(String, Span)> = toks
            .windows(2)
            .filter_map(|w| match (&w[0].tok, &w[1].tok) {
                (Tok::Use, Tok::Ident(n)) => Some((n.clone(), w[1].span)),
                _ => None,
            })
            .collect();
        self.loading.push(name.to_string());
        // 依存先のパッケージの中なら、その空間
        let space: Option<String> = match name.split_once('/') {
            Some((sp, _)) => Some(sp.to_string()),
            None if self.libs.contains(name) => Some(name.to_string()),
            None => None,
        };
        // 書いた名前 → モジュールの名前
        let mut keys: HashMap<String, String> = HashMap::new();
        for (u, span) in &uses {
            if keys.contains_key(u) {
                continue;
            }
            // (問い合わせ, モジュールの名前)。依存先の lib.jw の問い合わせは `空間*名前`（最初のパッケージなら `*名前`）で、
            // 読む関数は、その空間のパッケージが直接の依存先として書いたものだけ返す
            let candidates: Vec<(String, String)> = match &space {
                Some(sp) => vec![(format!("{}/{}", sp, u), format!("{}/{}", sp, u)), (format!("{}*{}", sp, u), u.clone())],
                None => vec![(u.clone(), u.clone()), (format!("*{}", u), u.clone())],
            };
            let mut found = None;
            for (query, key) in candidates {
                if self.loading.contains(&key) {
                    return Err(Diagnostic::new(format!("モジュール `{}` の use が輪になっています", u))
                        .primary(*span, "ここで使うと、読む順が決まらない")
                        .note("共通の部分を別のモジュールに分けてください"));
                }
                let lib = query.contains('*');
                if !lib && self.infos.contains_key(&key) {
                    found = Some(key);
                    break;
                }
                // 依存先のパッケージは、読んだことがあっても毎回この空間から使えるかを問い合わせる
                if let Some(text) = (self.read)(&query) {
                    // `*名前` は依存先のパッケージだけ（最初のパッケージの同じ名前のモジュールは使わない）
                    if lib && self.infos.contains_key(&key) && self.libs.contains(&key) {
                        found = Some(key);
                        break;
                    }
                    let query = if lib {
                        self.libs.insert(key.clone());
                        // ファイルの名前はどの空間から読んでも `*名前`
                        format!("*{}", u)
                    } else {
                        query
                    };
                    self.files.push((query, text));
                    let info = self.module(&key, self.files.len() - 1)?;
                    self.infos.insert(key.clone(), info);
                    found = Some(key);
                    break;
                }
            }
            let Some(key) = found else {
                return Err(Diagnostic::new(format!("モジュール `{}` が見つかりません", u))
                    .primary(*span, format!("同じフォルダに {}.jw がない", u)));
            };
            keys.insert(u.clone(), key);
        }
        self.loading.pop();
        let deps: HashMap<String, ModuleInfo> =
            uses.iter().map(|(u, _)| (u.clone(), self.infos[&keys[u]].clone())).collect();
        let aliases: HashMap<String, String> = keys.into_iter().filter(|(u, k)| u != k).collect();
        let mut prog = Parser::with_modules(toks, &deps)?.parse_program()?;
        let info = info_of(&prog);
        if !name.is_empty() {
            qualify(&mut prog, name, &info, &aliases);
            prog.consts = prog.consts.iter().map(|c| format!("{}.{}", name, c)).collect();
        }
        self.parts.push(prog);
        Ok(info)
    }
}

/// モジュールのトップレベルの名前（書き換える前）
fn info_of(p: &Program) -> ModuleInfo {
    let mut names = HashMap::new();
    let public = |n: &str| p.public.iter().any(|x| x == n);
    // メソッド（`Point.len`）はトップレベルの名前ではない
    for f in p.fns.iter().filter(|f| !f.name.contains('.')) {
        names.insert(f.name.clone(), (NameKind::Fn, public(&f.name)));
    }
    for s in &p.structs {
        names.insert(s.name.clone(), (NameKind::Struct, public(&s.name)));
    }
    let mut variants = HashMap::new();
    for e in &p.enums {
        names.insert(e.name.clone(), (NameKind::Enum, public(&e.name)));
        variants.insert(e.name.clone(), e.variants.iter().map(|v| v.name.clone()).collect());
    }
    for i in &p.interfaces {
        names.insert(i.name.clone(), (NameKind::Interface, public(&i.name)));
    }
    ModuleInfo { names, variants }
}

/// モジュールの中の名前を、モジュール名をつけた名前に書き換える
struct Qualify<'a> {
    module: &'a str,
    fns: HashSet<String>,
    structs: HashSet<String>,
    /// 型として書ける名前（構造体・enum・interface）
    types: HashSet<String>,
    enums: HashSet<String>,
    /// `use 名前;` で読んだ依存先の中のモジュールの、書いた名前 → モジュールの名前（`sq` → `mathx/sq`）
    aliases: &'a HashMap<String, String>,
}

fn qualify(p: &mut Program, module: &str, info: &ModuleInfo, aliases: &HashMap<String, String>) {
    let of = |k: NameKind| -> HashSet<String> {
        info.names.iter().filter(|(_, (kind, _))| *kind == k).map(|(n, _)| n.clone()).collect()
    };
    let q = Qualify {
        module,
        fns: of(NameKind::Fn),
        structs: of(NameKind::Struct),
        enums: of(NameKind::Enum),
        types: info
            .names
            .iter()
            .filter(|(_, (k, _))| *k != NameKind::Fn)
            .map(|(n, _)| n.clone())
            .collect(),
        aliases,
    };
    for s in &mut p.structs {
        s.name = q.name(&s.name);
        for t in &mut s.field_types {
            q.ty(t, &s.tparams);
        }
    }
    for e in &mut p.enums {
        e.name = q.name(&e.name);
        for v in &mut e.variants {
            for (t, _) in &mut v.fields {
                q.ty(t, &e.tparams);
            }
        }
    }
    for i in &mut p.interfaces {
        i.name = q.name(&i.name);
        for (m, _) in &mut i.methods {
            q.sig(m);
        }
    }
    for f in &mut p.fns {
        f.name = match f.name.split_once('.') {
            // メソッド・静的メソッド（`Point.len`）は構造体の名前を書き換える
            Some((s, m)) => format!("{}.{}", q.name(s), m),
            None => q.name(&f.name),
        };
        q.sig(f);
        let tps = f.tparams.clone();
        q.block(&mut f.body, &tps);
    }
}

impl<'a> Qualify<'a> {
    fn name(&self, n: &str) -> String {
        format!("{}.{}", self.module, n)
    }

    /// ほかのモジュールの名前（`sq.f`）を、モジュールの本当の名前に（`mathx/sq.f`）
    fn alias(&self, n: &mut String) {
        if let Some((m, rest)) = n.split_once('.') {
            if let Some(k) = self.aliases.get(m) {
                *n = format!("{}.{}", k, rest);
            }
        }
    }

    /// 関数の形（引数・戻り値の型と、型の引数の条件）
    fn sig(&self, f: &mut FnDef) {
        for p in &mut f.params {
            self.ty(&mut p.ty, &f.tparams);
        }
        if let Some(r) = &mut f.ret {
            self.ty(r, &f.tparams);
        }
        for bs in &mut f.bounds {
            for b in bs {
                if self.types.contains(b.as_str()) {
                    *b = self.name(b);
                } else {
                    self.alias(b);
                }
            }
        }
    }

    /// 型の中の名前。型の引数（`tps`）は書き換えない
    fn ty(&self, t: &mut TypeExpr, tps: &[String]) {
        match &mut t.kind {
            TypeKind::Name(n) | TypeKind::App(n, _) if self.types.contains(n.as_str()) && !tps.contains(n) => {
                *n = self.name(n);
            }
            TypeKind::Name(n) | TypeKind::App(n, _) => self.alias(n),
            _ => {}
        }
        match &mut t.kind {
            TypeKind::Name(_) => {}
            TypeKind::App(_, args) => args.iter_mut().for_each(|a| self.ty(a, tps)),
            TypeKind::Own(x) | TypeKind::Ref(x, _) | TypeKind::Array(x, _) | TypeKind::List(x) => self.ty(x, tps),
            TypeKind::Map(v, k) => {
                self.ty(v, tps);
                self.ty(k, tps);
            }
            TypeKind::Fn(ps, r) => {
                ps.iter_mut().for_each(|p| self.ty(p, tps));
                if let Some(r) = r {
                    self.ty(r, tps);
                }
            }
        }
    }

    fn block(&self, b: &mut Block, tps: &[String]) {
        for s in &mut b.stmts {
            self.stmt(s, tps);
        }
    }

    fn stmt(&self, s: &mut Stmt, tps: &[String]) {
        match s {
            Stmt::Let { ty, init, .. } => {
                if let Some(t) = ty {
                    self.ty(t, tps);
                }
                self.expr(init, tps);
            }
            Stmt::Assign { target, value, .. } => {
                self.expr(target, tps);
                self.expr(value, tps);
            }
            Stmt::Expr(e) => self.expr(e, tps),
            Stmt::While { cond, body } => {
                self.expr(cond, tps);
                self.block(body, tps);
            }
            Stmt::Loop { body } => self.block(body, tps),
            Stmt::For { range, body, els, .. } => {
                range.iter_mut().for_each(|a| self.expr(a, tps));
                self.block(body, tps);
                if let Some(els) = els {
                    self.block(els, tps);
                }
            }
            Stmt::ForEach { iter, body, els, .. } => {
                self.expr(iter, tps);
                self.block(body, tps);
                if let Some(els) = els {
                    self.block(els, tps);
                }
            }
            Stmt::Break(_) | Stmt::Continue(_) => {}
            Stmt::Return { value, .. } => {
                if let Some(e) = value {
                    self.expr(e, tps);
                }
            }
        }
    }

    fn expr(&self, e: &mut Expr, tps: &[String]) {
        match &mut e.kind {
            ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::Str(_) | ExprKind::Unit => {}
            // 名前のある関数を値として使う（変数に関数と同じ名前はつけられないので、関数の名前なら関数）
            ExprKind::Var(n) => {
                if self.fns.contains(n.as_str()) {
                    *n = self.name(n);
                } else {
                    self.alias(n);
                }
            }
            ExprKind::Call(n, args) => {
                let static_call =
                    matches!(n.split_once('.'), Some((s, _)) if self.structs.contains(s) || self.enums.contains(s));
                if self.fns.contains(n.as_str()) || static_call {
                    *n = self.name(n);
                } else {
                    self.alias(n);
                }
                args.iter_mut().for_each(|a| self.expr(a, tps));
            }
            ExprKind::StructLit(n, inits) => {
                if self.structs.contains(n.as_str()) {
                    *n = self.name(n);
                } else {
                    self.alias(n);
                }
                inits.iter_mut().for_each(|i| self.expr(&mut i.value, tps));
            }
            ExprKind::Variant { enum_name, args, .. } => {
                if self.enums.contains(enum_name.as_str()) {
                    *enum_name = self.name(enum_name);
                } else {
                    self.alias(enum_name);
                }
                args.iter_mut().for_each(|a| self.expr(a, tps));
            }
            ExprKind::Cast(x, t) => {
                self.expr(x, tps);
                self.ty(t, tps);
            }
            ExprKind::Try(x) | ExprKind::Own(x) | ExprKind::Ref(x, _) | ExprKind::Deref(x, _) => self.expr(x, tps),
            ExprKind::Field(x, ..) | ExprKind::Unary(_, x) | ExprKind::ArrayRepeat(x, ..) => self.expr(x, tps),
            ExprKind::If { cond, then, els } => {
                self.expr(cond, tps);
                self.block(then, tps);
                if let Some(els) = els {
                    self.block(els, tps);
                }
            }
            ExprKind::Match { scrut, arms, default } => {
                self.expr(scrut, tps);
                arms.iter_mut().for_each(|a| self.block(&mut a.body, tps));
                if let Some(d) = default {
                    self.block(d, tps);
                }
            }
            ExprKind::Binary(_, l, r) | ExprKind::Index(l, r, _) => {
                self.expr(l, tps);
                self.expr(r, tps);
            }
            ExprKind::ArrayLit(items, _) => items.iter_mut().for_each(|x| self.expr(x, tps)),
            ExprKind::MapLit(pairs) => {
                for (k, v) in pairs {
                    self.expr(k, tps);
                    self.expr(v, tps);
                }
            }
            ExprKind::Interp(parts) => {
                for p in parts {
                    if let InterpPart::Expr(x) = p {
                        self.expr(x, tps);
                    }
                }
            }
            ExprKind::Lambda(l) => {
                let l = std::rc::Rc::get_mut(l).expect("lambdas are not shared before type checking");
                for p in &mut l.params {
                    if let Some(t) = &mut p.ty {
                        self.ty(t, tps);
                    }
                }
                self.block(&mut l.body, tps);
            }
            ExprKind::MethodCall(recv, _, args, _) => {
                self.expr(recv, tps);
                args.iter_mut().for_each(|a| self.expr(a, tps));
            }
        }
    }
}
