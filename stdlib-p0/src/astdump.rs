//! 構文木を文字にする（`jabberwock --ast`）。jwc の構文解析と突き合わせるための形で、jwc/astdump.jw と同じものを出す。
//!
//! 1 行に 1 つの節。子は 2 つ下げて書く。位置は `@ファイル:行:列:長さ`。型は 1 行にまとめる（`list(own(i64@..)@..)@..`）。
//! `--typed`（`dump_typed`）は、型検査が決めたこと（式の型・整数の変換の印・型の引数など）を行の後ろに足し、
//! 変数の型・関数の戻り値・構造体と enum の型を最後に並べる（jwc の型検査との突き合わせ用）。

use crate::ast::*;
use crate::diag::Span;
use crate::types::{Ty, TypeInfo};

fn sp(s: Span) -> String {
    format!("@{}:{}:{}:{}", s.file, s.line, s.col, s.len)
}

pub fn escape(s: &str) -> String {
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

pub fn ty(t: &TypeExpr) -> String {
    let inner = match &t.kind {
        TypeKind::Name(n) => n.clone(),
        TypeKind::App(n, args) => format!("{}<{}>", n, args.iter().map(ty).collect::<Vec<_>>().join(", ")),
        TypeKind::Own(x) => format!("own({})", ty(x)),
        TypeKind::Ref(x, m) => format!("{}({})", if *m { "refmut" } else { "ref" }, ty(x)),
        TypeKind::Array(x, n) => format!("arr({}, {})", ty(x), n),
        TypeKind::List(x) => format!("list({})", ty(x)),
        TypeKind::Map(v, k) => format!("map({}, {})", ty(v), ty(k)),
        TypeKind::Fn(ps, r) => format!(
            "fn({}; {})",
            r.as_ref().map(|r| ty(r)).unwrap_or_else(|| "-".to_string()),
            ps.iter().map(ty).collect::<Vec<_>>().join(", ")
        ),
    };
    format!("{}{}", inner, sp(t.span))
}

struct Out<'a> {
    lines: Vec<String>,
    info: Option<&'a TypeInfo>,
}

fn tys(ts: &[Ty]) -> String {
    ts.iter().map(Ty::show).collect::<Vec<_>>().join(", ")
}

impl<'a> Out<'a> {
    fn line(&mut self, depth: usize, s: String) {
        self.lines.push(format!("{}{}", "  ".repeat(depth), s));
    }

    fn block(&mut self, d: usize, b: &Block) {
        self.line(d, format!("block tail={} end{}", b.tail, sp(b.end)));
        for s in &b.stmts {
            self.stmt(d + 1, s);
        }
    }

    fn stmt(&mut self, d: usize, s: &Stmt) {
        match s {
            Stmt::Let { name, ty: t, mutable, span, init } => {
                let t = t.as_ref().map(|t| format!(" {}", ty(t))).unwrap_or_default();
                self.line(d, format!("let{} {}{}{}", if *mutable { " mut" } else { "" }, name, t, sp(*span)));
                self.expr(d + 1, init);
            }
            Stmt::Assign { target, value, span } => {
                self.line(d, format!("assign{}", sp(*span)));
                self.expr(d + 1, target);
                self.expr(d + 1, value);
            }
            Stmt::Expr(e) => {
                self.line(d, "expr".to_string());
                self.expr(d + 1, e);
            }
            Stmt::While { cond, body } => {
                self.line(d, "while".to_string());
                self.expr(d + 1, cond);
                self.block(d + 1, body);
            }
            Stmt::Loop { body } => {
                self.line(d, "loop".to_string());
                self.block(d + 1, body);
            }
            Stmt::For { var, range, range_span, body, els } => {
                self.line(d, format!("for {} range{}", var, sp(*range_span)));
                for a in range {
                    self.expr(d + 1, a);
                }
                self.block(d + 1, body);
                if let Some(e) = els {
                    self.line(d, "else".to_string());
                    self.block(d + 1, e);
                }
            }
            Stmt::ForEach { key, var, var_span, iter, body, els, consume } => {
                let k = key.as_ref().map(|(k, s)| format!(" key {}{}", k, sp(*s))).unwrap_or_default();
                let c = if self.info.is_some() && consume.get() { " consume" } else { "" };
                self.line(d, format!("foreach{} {}{}{}", k, var, sp(*var_span), c));
                self.expr(d + 1, iter);
                self.block(d + 1, body);
                if let Some(e) = els {
                    self.line(d, "else".to_string());
                    self.block(d + 1, e);
                }
            }
            Stmt::Break(s) => self.line(d, format!("break{}", sp(*s))),
            Stmt::Continue(s) => self.line(d, format!("continue{}", sp(*s))),
            Stmt::Return { value, span } => {
                self.line(d, format!("return{}", sp(*span)));
                if let Some(v) = value {
                    self.expr(d + 1, v);
                }
            }
        }
    }

    fn pat(&mut self, d: usize, p: &Pat, s: Span) {
        match p {
            Pat::Int(n) => self.line(d, format!("pat int {}{}", n, sp(s))),
            Pat::Bool(b) => self.line(d, format!("pat bool {}{}", b, sp(s))),
            Pat::Str(x) => self.line(d, format!("pat str \"{}\"{}", escape(x), sp(s))),
            Pat::Variant(name, binds, subs) => {
                self.line(d, format!("pat variant {}{}", name, sp(s)));
                for ((b, bs), sub) in binds.iter().zip(subs) {
                    match sub {
                        Some(q) => self.pat(d + 1, q, *bs),
                        None => self.line(d + 1, format!("bind {}{}", b, sp(*bs))),
                    }
                }
            }
        }
    }

    /// 型検査が決めたこと（`--typed` のときだけ）
    fn typed(&self, e: &Expr) -> String {
        let Some(info) = self.info else { return String::new() };
        let mut out = String::new();
        if let Some(t) = info.ty(e) {
            out += &format!(" : {}", t.show());
        }
        if let Some(t) = e.int.get() {
            out += &format!(" int={}", t.name());
        }
        if e.float.get() {
            out += " float";
        }
        if e.borrow.get() {
            out += " borrow";
        }
        match &e.kind {
            ExprKind::Variant { copy, .. } => {
                if let Some(c) = copy.get() {
                    out += &format!(" copy={}", c);
                }
            }
            ExprKind::ArrayLit(_, seq) => match seq.get() {
                Seq::Array => {}
                Seq::List => out += " seq=list",
                Seq::Map => out += " seq=map",
            },
            ExprKind::ArrayRepeat(_, _, list) if list.get() => out += " list",
            ExprKind::MethodCall(_, _, _, t) => {
                let t = t.borrow();
                if let Some(n) = &t.enum_name {
                    out += &format!(" enum={}", n);
                }
                if t.take {
                    out += " take";
                }
            }
            ExprKind::Lambda(l) => {
                let b: Vec<String> = l.borrowed.borrow().iter().map(|b| b.to_string()).collect();
                out += &format!(" caps=[{}] borrowed=[{}] returns={}", l.captures.borrow().join(", "), b.join(", "), l.returns.get());
            }
            _ => {}
        }
        if let Some(m) = info.targs.get(&(e as *const Expr)) {
            let mut ks: Vec<&String> = m.keys().collect();
            ks.sort();
            let ps: Vec<String> = ks.iter().map(|k| format!("{}={}", k, m[*k].show())).collect();
            out += &format!(" targs={{{}}}", ps.join(", "));
        }
        out
    }

    fn expr(&mut self, d: usize, e: &Expr) {
        let s = sp(e.span) + &self.typed(e);
        match &e.kind {
            ExprKind::Int(n) => self.line(d, format!("int {}{}", n, s)),
            ExprKind::Float(x) => self.line(d, format!("float {:?}{}", x, s)),
            ExprKind::Bool(b) => self.line(d, format!("bool {}{}", b, s)),
            ExprKind::Str(x) => self.line(d, format!("str \"{}\"{}", escape(x), s)),
            ExprKind::Unit => self.line(d, format!("unit{}", s)),
            ExprKind::Var(n) => self.line(d, format!("var {}{}", n, s)),
            ExprKind::Cast(x, t) => {
                self.line(d, format!("cast {}{}", ty(t), s));
                self.expr(d + 1, x);
            }
            ExprKind::Try(x) => {
                self.line(d, format!("try{}", s));
                self.expr(d + 1, x);
            }
            ExprKind::Variant { enum_name, variant, args, .. } => {
                self.line(d, format!("variant {}.{}{}", enum_name, variant, s));
                for a in args {
                    self.expr(d + 1, a);
                }
            }
            ExprKind::Own(x) => {
                self.line(d, format!("own{}", s));
                self.expr(d + 1, x);
            }
            ExprKind::Ref(x, m) => {
                self.line(d, format!("{}{}", if *m { "ref mut" } else { "ref" }, s));
                self.expr(d + 1, x);
            }
            ExprKind::Deref(x, _) => {
                self.line(d, format!("deref{}", s));
                self.expr(d + 1, x);
            }
            ExprKind::Field(x, f, _) => {
                self.line(d, format!("field {}{}", f, s));
                self.expr(d + 1, x);
            }
            ExprKind::StructLit(n, inits) => {
                self.line(d, format!("struct {}{}", n, s));
                for i in inits {
                    self.line(d + 1, format!("init {}{}", i.name, sp(i.span)));
                    self.expr(d + 2, &i.value);
                }
            }
            ExprKind::If { cond, then, els } => {
                self.line(d, format!("if{}", s));
                self.expr(d + 1, cond);
                self.block(d + 1, then);
                if let Some(b) = els {
                    self.line(d, "else".to_string());
                    self.block(d + 1, b);
                }
            }
            ExprKind::Match { scrut, arms, default } => {
                self.line(d, format!("match{}", s));
                self.expr(d + 1, scrut);
                for a in arms {
                    self.line(d + 1, "case".to_string());
                    for (p, ps) in &a.pats {
                        self.pat(d + 2, p, *ps);
                    }
                    self.block(d + 2, &a.body);
                }
                if let Some(b) = default {
                    self.line(d + 1, "default".to_string());
                    self.block(d + 2, b);
                }
            }
            ExprKind::Unary(op, x) => {
                let o = match op {
                    UnOp::Neg => "-",
                    UnOp::Not => "!",
                    UnOp::BitNot => "~",
                };
                self.line(d, format!("unary {}{}", o, s));
                self.expr(d + 1, x);
            }
            ExprKind::Binary(op, l, r) => {
                let o = match op {
                    BinOp::Add => "+",
                    BinOp::Sub => "-",
                    BinOp::Mul => "*",
                    BinOp::Div => "/",
                    BinOp::Rem => "%",
                    BinOp::Eq => "==",
                    BinOp::Ne => "!=",
                    BinOp::Lt => "<",
                    BinOp::Le => "<=",
                    BinOp::Gt => ">",
                    BinOp::Ge => ">=",
                    BinOp::And => "&&",
                    BinOp::Or => "||",
                    BinOp::BitAnd => "&",
                    BinOp::BitOr => "|",
                    BinOp::BitXor => "^",
                    BinOp::Shl => "<<",
                    BinOp::Shr => ">>",
                };
                self.line(d, format!("binary {}{}", o, s));
                self.expr(d + 1, l);
                self.expr(d + 1, r);
            }
            ExprKind::Call(n, args) => {
                self.line(d, format!("call {}{}", n, s));
                for a in args {
                    self.expr(d + 1, a);
                }
            }
            ExprKind::Index(b, i, _) => {
                self.line(d, format!("index{}", s));
                self.expr(d + 1, b);
                self.expr(d + 1, i);
            }
            ExprKind::ArrayLit(items, _) => {
                self.line(d, format!("array{}", s));
                for x in items {
                    self.expr(d + 1, x);
                }
            }
            ExprKind::MapLit(pairs) => {
                self.line(d, format!("map{}", s));
                for (k, v) in pairs {
                    self.expr(d + 1, k);
                    self.expr(d + 1, v);
                }
            }
            ExprKind::ArrayRepeat(x, n, _) => {
                self.line(d, format!("repeat {}{}", n, s));
                self.expr(d + 1, x);
            }
            ExprKind::Interp(parts) => {
                self.line(d, format!("interp{}", s));
                for p in parts {
                    match p {
                        InterpPart::Lit(x) => self.line(d + 1, format!("lit \"{}\"", escape(x))),
                        InterpPart::Expr(x) => self.expr(d + 1, x),
                    }
                }
            }
            ExprKind::Lambda(l) => {
                self.line(d, format!("lambda borrows={}{}", l.borrows.get(), s));
                for p in &l.params {
                    let t = p.ty.as_ref().map(|t| format!(" {}", ty(t))).unwrap_or_default();
                    self.line(d + 1, format!("param {}{}{}", p.name, t, sp(p.span)));
                }
                self.block(d + 1, &l.body);
            }
            ExprKind::MethodCall(recv, n, args, _) => {
                self.line(d, format!("mcall {}{}", n, s));
                self.expr(d + 1, recv);
                for a in args {
                    self.expr(d + 1, a);
                }
            }
        }
    }

    fn func(&mut self, d: usize, f: &FnDef) {
        let tps: Vec<String> = f
            .tparams
            .iter()
            .zip(&f.bounds)
            .map(|(t, b)| if b.is_empty() { t.clone() } else { format!("{}: {}", t, b.join(" + ")) })
            .collect();
        self.line(d, format!("fn {}<{}>{}", f.name, tps.join(", "), sp(f.span)));
        if let Some(r) = &f.ret {
            self.line(d + 1, format!("ret {}", ty(r)));
        }
        for p in &f.params {
            self.line(d + 1, format!("param{} {} {}{}", if p.mutable { " mut" } else { "" }, ty(&p.ty), p.name, sp(p.span)));
        }
        self.block(d + 1, &f.body);
    }
}

pub fn dump(p: &Program) -> String {
    dump_with(p, None)
}

/// 型検査の結果つき（`--typed`）
pub fn dump_typed(p: &Program, info: &TypeInfo) -> String {
    let mut out = dump_with(p, Some(info));
    let mut vars: Vec<(&Span, &Ty)> = info.var_ty.iter().collect();
    vars.sort_by_key(|(s, _)| (s.file, s.line, s.col, s.len));
    for (s, t) in vars {
        out += &format!("\nvar{} {}", sp(*s), t.show());
    }
    let mut rets: Vec<(&String, &Ty)> = info.fn_ret.iter().collect();
    rets.sort_by_key(|(n, _)| n.as_str());
    for (n, t) in rets {
        out += &format!("\nret {} {}", n, t.show());
    }
    let mut ss: Vec<_> = info.structs.iter().collect();
    ss.sort_by_key(|(n, _)| n.as_str());
    for (n, (tps, fs)) in ss {
        out += &format!("\nstructty {}<{}> {}", n, tps.join(", "), tys(fs));
    }
    let mut es: Vec<_> = info.enums.iter().collect();
    es.sort_by_key(|(n, _)| n.as_str());
    for (n, (tps, vs)) in es {
        let vs: Vec<String> = vs.iter().map(|(v, fs)| format!("{}({})", v, tys(fs))).collect();
        out += &format!("\nenumty {}<{}> {}", n, tps.join(", "), vs.join(" "));
    }
    out
}

fn dump_with(p: &Program, info: Option<&TypeInfo>) -> String {
    let mut o = Out { lines: Vec::new(), info };
    for s in &p.structs {
        o.line(0, format!("struct {}<{}>{}", s.name, s.tparams.join(", "), sp(s.span)));
        for (f, t) in s.fields.iter().zip(&s.field_types) {
            o.line(1, format!("field {} {}", ty(t), f));
        }
    }
    for e in &p.enums {
        o.line(0, format!("enum {}<{}>{}", e.name, e.tparams.join(", "), sp(e.span)));
        for v in &e.variants {
            o.line(1, format!("variant {}{}", v.name, sp(v.span)));
            for (t, n) in &v.fields {
                o.line(2, format!("{} {}", ty(t), n));
            }
        }
    }
    for i in &p.interfaces {
        o.line(0, format!("interface {}{}", i.name, sp(i.span)));
        for (m, mutable) in &i.methods {
            o.line(1, format!("method mut={}", mutable));
            o.func(2, m);
        }
    }
    for f in &p.fns {
        o.func(0, f);
    }
    let uses: Vec<String> = p.uses.iter().map(|(n, s)| format!("{}{}", n, sp(*s))).collect();
    o.line(0, format!("uses [{}]", uses.join(", ")));
    o.line(0, format!("public [{}]", p.public.join(", ")));
    o.line(0, format!("modules [{}]", p.modules.join(", ")));
    o.line(0, format!("consts [{}]", p.consts.join(", ")));
    o.lines.join("\n")
}
