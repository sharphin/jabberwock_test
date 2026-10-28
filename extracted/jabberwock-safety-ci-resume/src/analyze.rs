//! 静的解析: 「この `*r` / `r.f` の借りた先は必ず生きている」と証明できた箇所の世代チェックを消す。
//!
//! 実行時の世代チェックは安全の最後の砦。ただし毎回払うと遅い。そこで実行前にプログラムを
//! 読み、証明できた `*r` には `Check::Elided` を書き込む。インタプリタはそこだけ検査を飛ばす。
//! 証明できない箇所は `Check::Required` のまま残る（安全側に倒す）。
//!
//! 証明の考え方:
//!   * `let a = own ...` の変数 `a` が持つスロットを解放できるのは、`a` 自身への操作だけ。
//!     （スコープを抜ける・再代入・ムーブ・`drop`。どれも `a` という名前が現れる）
//!     呼び出された関数は `a` に触れないので、`ref a` を渡しても `a` のスロットは解放できない。
//!   * なので `let r = ref a` のあと、`a` が死なずに残っている間の `*r` は安全。
//!   * 関数の引数 `r` は、すべての呼び出し元が「その時点で生きている ref」を渡しているなら、
//!     関数の実行中ずっと安全（Pinned）。これは関数をまたいで繰り返し計算して決める。
//!
//! 解析はフロー依存（文の順に事実を更新）で、`if` の合流やループでは両方で成り立つ事実だけ残す。
//! `break` / `continue` で抜ける道も合流に含める（抜けるときに消える変数の事実は消してから）。

use std::cell::Cell;
use std::collections::HashMap;

use crate::ast::*;

/// 解析中に変数を区別する番号。同じ名前でも宣言ごとに別の番号になる（シャドーイング対策）。
type VarId = usize;

/// 変数について「必ず成り立つ」と分かっていること。分からない変数は表に載せない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fact {
    /// ムーブされていない own を必ず持っている
    Own,
    /// このフレームの own 変数を直接借りた ref。その変数が `Own` のままなら安全
    RefTo(VarId),
    /// 関数の実行中ずっと借りた先が生きている ref（安全な引数から来たもの）
    Pinned,
}

type Facts = HashMap<VarId, Fact>;

/// 変数 `id` の中身が解放された・移動したかもしれない。それを借りた ref の事実も消す。
fn kill_in(facts: &mut Facts, id: VarId) {
    facts.remove(&id);
    facts.retain(|_, f| *f != Fact::RefTo(id));
}

/// 片方がまだない合流（break や continue が 1 つも出てこない場合）
fn meet_opt(acc: Option<Facts>, f: &Facts) -> Option<Facts> {
    Some(match acc {
        Some(a) => meet(&a, f),
        None => f.clone(),
    })
}

/// 解析中のループ。`break` / `continue` の地点の事実を集める。
struct LoopCtx {
    /// ループ本体に入る前のスコープの深さ。抜けるときはこれより深い変数が消える
    depth: usize,
    breaks: Option<Facts>,
    continues: Option<Facts>,
}

/// 両方の道で成り立つ事実だけを残す（合流点）
fn meet(a: &Facts, b: &Facts) -> Facts {
    a.iter().filter(|(k, v)| b.get(k) == Some(v)).map(|(k, v)| (*k, *v)).collect()
}

/// 解析の結果（`*p` の箇所数）
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    /// 検査不要と証明できた箇所
    pub elided: usize,
    /// 実行時の検査が残る箇所
    pub required: usize,
}

/// プログラム全体を解析し、各 `*p` に `Check` を書き込む。
pub fn analyze(prog: &Program) -> Stats {
    // 関数ごと・引数ごとに「Pinned か」。楽観的に全部 true から始め、反例が出たら false にする。
    // 値は減る一方なので必ず止まる。main から辿れる実際の呼び出しは呼び出しの深さに関する帰納法で安全。
    let mut pinned: HashMap<&str, Vec<bool>> =
        prog.fns.iter().map(|f| (f.name.as_str(), vec![true; f.params.len()])).collect();

    loop {
        for f in &prog.fns {
            for_each_check(&f.body, &mut |c| c.set(Check::Unvisited));
        }
        let mut calls: HashMap<String, Vec<bool>> = HashMap::new();
        for f in &prog.fns {
            FnAnalyzer::new(&pinned, &mut calls).run(f);
        }
        // 一度も呼ばれない関数の引数は何も仮定しない
        let next: HashMap<&str, Vec<bool>> = prog
            .fns
            .iter()
            .map(|f| {
                let v = calls.get(&f.name).cloned().unwrap_or_else(|| vec![false; f.params.len()]);
                (f.name.as_str(), v)
            })
            .collect();
        if next == pinned {
            break;
        }
        pinned = next;
    }

    let mut stats = Stats::default();
    for f in &prog.fns {
        for_each_check(&f.body, &mut |c| match c.get() {
            Check::Elided => stats.elided += 1,
            _ => stats.required += 1,
        });
    }
    stats
}

struct FnAnalyzer<'a> {
    pinned: &'a HashMap<&'a str, Vec<bool>>,
    /// 呼び出し先ごとの「その引数が安全な ref か」。全呼び出し箇所の AND を取る。
    calls: &'a mut HashMap<String, Vec<bool>>,
    scopes: Vec<Vec<(String, VarId)>>,
    next_id: VarId,
    facts: Facts,
    loops: Vec<LoopCtx>,
}

impl<'a> FnAnalyzer<'a> {
    fn new(pinned: &'a HashMap<&'a str, Vec<bool>>, calls: &'a mut HashMap<String, Vec<bool>>) -> Self {
        FnAnalyzer { pinned, calls, scopes: Vec::new(), next_id: 0, facts: Facts::new(), loops: Vec::new() }
    }

    fn run(&mut self, f: &FnDef) {
        self.scopes.push(Vec::new());
        let safe = self.pinned.get(f.name.as_str()).cloned().unwrap_or_default();
        for (i, p) in f.params.iter().enumerate() {
            let fact = if safe.get(i) == Some(&true) { Some(Fact::Pinned) } else { None };
            self.declare(&p.name, fact);
        }
        self.block(&f.body);
    }

    // ------------------------------------------------------------ 変数

    fn lookup(&self, name: &str) -> Option<VarId> {
        self.scopes.iter().rev().flat_map(|s| s.iter().rev()).find(|(n, _)| n == name).map(|(_, id)| *id)
    }

    fn declare(&mut self, name: &str, fact: Option<Fact>) {
        let id = self.next_id;
        self.next_id += 1;
        self.scopes.last_mut().expect("scope").push((name.to_string(), id));
        if let Some(f) = fact {
            self.facts.insert(id, f);
        }
    }

    fn kill(&mut self, id: VarId) {
        kill_in(&mut self.facts, id);
    }

    /// `depth` より深いスコープを抜けたあとの事実（break / continue で飛ぶ先で成り立つこと）
    fn facts_leaving(&self, depth: usize) -> Facts {
        let mut f = self.facts.clone();
        for (_, id) in self.scopes[depth..].iter().flatten() {
            kill_in(&mut f, *id);
        }
        f
    }

    /// ループを不動点まで解析する。
    /// `cond` は while の条件、`vars` は for のループ変数、`exits_at_head` は先頭で抜けうるか。
    fn loop_body(&mut self, cond: Option<&Expr>, vars: &[&str], body: &Block, exits_at_head: bool) {
        loop {
            let entry = self.facts.clone();
            if let Some(c) = cond {
                self.expr(c, false);
            }
            let head_exit = self.facts.clone();
            self.loops.push(LoopCtx { depth: self.scopes.len(), breaks: None, continues: None });
            if !vars.is_empty() {
                self.scopes.push(Vec::new());
                for v in vars {
                    self.declare(v, None);
                }
            }
            self.block(body);
            if !vars.is_empty() {
                for (_, id) in self.scopes.pop().expect("scope") {
                    self.kill(id);
                }
            }
            let ctx = self.loops.pop().expect("loop");
            let end = match &ctx.continues {
                Some(c) => meet(&self.facts, c),
                None => self.facts.clone(),
            };
            let next = meet(&entry, &end);
            if next == entry {
                self.facts = match (exits_at_head, ctx.breaks) {
                    (true, Some(b)) => meet(&head_exit, &b),
                    (_, Some(b)) => b,
                    // break のない loop は抜けない。後ろは到達しないので何でもよい
                    (_, None) => head_exit,
                };
                return;
            }
            self.facts = next;
        }
    }

    fn fact_of(&self, name: &str) -> Option<Fact> {
        self.lookup(name).and_then(|id| self.facts.get(&id).copied())
    }

    /// この ref は今、借りた先が生きていると言えるか
    fn is_live_ref(&self, f: Option<Fact>) -> bool {
        match f {
            Some(Fact::Pinned) => true,
            Some(Fact::RefTo(x)) => self.facts.get(&x) == Some(&Fact::Own),
            _ => false,
        }
    }

    // ------------------------------------------------------------ 文

    fn block(&mut self, b: &Block) {
        self.scopes.push(Vec::new());
        for s in &b.stmts {
            self.stmt(s);
        }
        for (_, id) in self.scopes.pop().expect("scope") {
            self.kill(id);
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Let { name, init, .. } => {
                let f = self.expr(init, false);
                self.declare(name, f);
            }
            Stmt::Assign { target, value, .. } => {
                // インタプリタと同じく、右辺を先に評価する
                let f = self.expr(value, false);
                match &target.kind {
                    ExprKind::Var(name) => {
                        if let Some(id) = self.lookup(name) {
                            self.kill(id); // 古い own は解放される
                            if let Some(f) = f.filter(|f| *f != Fact::RefTo(id)) {
                                self.facts.insert(id, f);
                            }
                        }
                    }
                    ExprKind::Deref(inner, check) | ExprKind::Field(inner, _, check) => self.deref(inner, check),
                    ExprKind::Index(inner, idx, check) => {
                        self.expr(idx, false);
                        self.deref(inner, check);
                    }
                    _ => {}
                }
            }
            Stmt::Expr(e) => {
                self.expr(e, false);
            }
            Stmt::While { cond, body } => self.loop_body(Some(cond), &[], body, true),
            Stmt::Loop { body } => self.loop_body(None, &[], body, false),
            Stmt::ForEach { key, var, iter, body, els, consume, .. } => {
                // インタプリタは eval_borrow で読む（変数ならムーブしない）。その場で作ったリストを回すときは eval
                self.expr(iter, !consume.get());
                let before = self.facts.clone();
                let mut vars: Vec<&str> = key.iter().map(|(k, _)| k.as_str()).collect();
                vars.push(var);
                self.loop_body(None, &vars, body, true);
                if let Some(els) = els {
                    let after_loop = std::mem::replace(&mut self.facts, before);
                    self.block(els);
                    self.facts = meet(&after_loop, &self.facts);
                }
            }
            Stmt::For { var, range, body, els, .. } => {
                for a in range {
                    self.expr(a, false);
                }
                let before = self.facts.clone();
                self.loop_body(None, &[var.as_str()], body, true);
                if let Some(els) = els {
                    // else は 1 回も回らなかったとき、つまりループに入る前の状態から実行される
                    let after_loop = std::mem::replace(&mut self.facts, before);
                    self.block(els);
                    self.facts = meet(&after_loop, &self.facts);
                }
            }
            Stmt::Break(_) => {
                let depth = self.loops.last().expect("parser checks break is in a loop").depth;
                let f = self.facts_leaving(depth);
                let ctx = self.loops.last_mut().expect("loop");
                ctx.breaks = meet_opt(ctx.breaks.take(), &f);
            }
            Stmt::Continue(_) => {
                let depth = self.loops.last().expect("parser checks continue is in a loop").depth;
                let f = self.facts_leaving(depth);
                let ctx = self.loops.last_mut().expect("loop");
                ctx.continues = meet_opt(ctx.continues.take(), &f);
            }
            Stmt::Return { value, .. } => {
                if let Some(e) = value {
                    self.expr(e, false);
                }
            }
        }
    }

    // ------------------------------------------------------------ 式

    /// `ref inner`。own 変数を直接借りたなら RefTo、ref 変数の借り直しなら同じ事実
    fn ref_of(&mut self, inner: &Expr) -> Option<Fact> {
        self.expr_kind(inner, true);
        match &inner.kind {
            ExprKind::Var(name) => {
                let id = self.lookup(name)?;
                match self.facts.get(&id).copied()? {
                    Fact::Own => Some(Fact::RefTo(id)),
                    f => Some(f), // ref の ref は同じ先を指す
                }
            }
            _ => None,
        }
    }

    /// 式を評価順にたどり、結果の値について分かることを返す。
    /// `borrow` はインタプリタの `eval_borrow` に当たる（変数を読んでもムーブしない）。
    fn expr(&mut self, e: &Expr, borrow: bool) -> Option<Fact> {
        if e.borrow.get() {
            // 型検査が自動で借りると決めた引数（`ref e` と同じ）。一時的な値は値として評価して借りる
            if !is_place(e) {
                self.expr_kind(e, false);
                return None;
            }
            return self.ref_of(e);
        }
        self.expr_kind(e, borrow)
    }

    fn expr_kind(&mut self, e: &Expr, borrow: bool) -> Option<Fact> {
        match &e.kind {
            ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::Str(_) | ExprKind::Unit => None,
            ExprKind::Var(name) => {
                let Some(id) = self.lookup(name) else {
                    // 名前のある関数を値として使う。どこから何が渡されるか分からないので、引数は何も仮定しない
                    if let Some(n) = self.pinned.get(name.as_str()).map(Vec::len) {
                        self.record_call(name, vec![false; n]);
                    }
                    return None;
                };
                let f = self.facts.get(&id).copied();
                let is_ref = matches!(f, Some(Fact::RefTo(_) | Fact::Pinned));
                if !borrow && !is_ref {
                    // own かもしれない値を読む = ムーブかもしれない
                    self.kill(id);
                }
                f
            }
            ExprKind::Own(inner) => {
                self.expr(inner, false);
                Some(Fact::Own)
            }
            ExprKind::Lambda(l) => {
                // 作るときに外の変数を値として読む（インタプリタと同じ順。own ならムーブ）
                // 借りて取り込む変数（`ref 名前`）は、事実を変えない
                let borrowed = l.borrowed.borrow().clone();
                for (k, n) in l.captures.borrow().iter().enumerate() {
                    if borrowed.get(k).copied().unwrap_or(false) {
                        continue;
                    }
                    if let Some(id) = self.lookup(n) {
                        if !matches!(self.facts.get(&id), Some(Fact::RefTo(_) | Fact::Pinned)) {
                            self.kill(id);
                        }
                    }
                }
                // 本体はいつ呼ばれるか分からないので、何も分からないところから調べる
                let scopes = std::mem::take(&mut self.scopes);
                let facts = std::mem::take(&mut self.facts);
                let loops = std::mem::take(&mut self.loops);
                self.scopes.push(Vec::new());
                for p in &l.params {
                    self.declare(&p.name, None);
                }
                for n in l.captures.borrow().iter() {
                    self.declare(n, None);
                }
                self.block(&l.body);
                self.scopes = scopes;
                self.facts = facts;
                self.loops = loops;
                None
            }
            ExprKind::Ref(inner, _) => self.ref_of(inner),
            // `r.f` も r が own / ref なら先をたどるので、`*r` と同じ判定
            ExprKind::Deref(inner, check) | ExprKind::Field(inner, _, check) => {
                self.deref(inner, check);
                None
            }
            ExprKind::StructLit(_, inits) => {
                for init in inits {
                    self.expr(&init.value, false);
                }
                None
            }
            ExprKind::If { cond, then, els } => {
                self.expr(cond, false);
                self.branches(std::iter::once(then), els.as_ref());
                None
            }
            ExprKind::Match { scrut, arms, default } => {
                self.expr(scrut, false);
                // case で受け取る名前は、その case の中だけの新しい変数（何も分からない）
                let before = self.facts.clone();
                let mut acc: Option<Facts> = None;
                for a in arms {
                    self.facts = before.clone();
                    self.scopes.push(Vec::new());
                    for (p, _) in &a.pats {
                        for (b, _) in p.names() {
                            self.declare(&b, None);
                        }
                    }
                    self.block(&a.body);
                    for (_, id) in self.scopes.pop().expect("scope") {
                        self.kill(id);
                    }
                    acc = meet_opt(acc, &self.facts);
                }
                if let Some(d) = default {
                    self.facts = before.clone();
                    self.block(d);
                    acc = meet_opt(acc, &self.facts);
                }
                // default がなければ、どの case にも当たらない道も合流に入れる（安全側）
                if default.is_none() {
                    acc = meet_opt(acc, &before);
                }
                self.facts = acc.unwrap_or(before);
                None
            }
            ExprKind::Variant { args, .. } => {
                for a in args {
                    self.expr(a, false);
                }
                None
            }
            // `e?` で関数を抜ける道は、関数の中のどこにも合流しないので、続く道の事実だけ考えればよい
            ExprKind::Unary(_, inner) | ExprKind::Cast(inner, _) | ExprKind::Try(inner) => {
                self.expr(inner, false);
                None
            }
            ExprKind::Binary(op, l, r) => {
                self.expr(l, false);
                if matches!(op, BinOp::And | BinOp::Or) {
                    // 右辺は評価されないこともある
                    let before = self.facts.clone();
                    self.expr(r, false);
                    self.facts = meet(&before, &self.facts);
                } else {
                    self.expr(r, false);
                }
                None
            }
            ExprKind::Call(name, args) => {
                match name.as_str() {
                    "print" | "eprint" => {
                        for a in args {
                            self.expr(a, true);
                        }
                    }
                    "drop" | "assert" | "spawn" | "chan" | "exit" | "read_file" | "write_file" | "run_command" | "chr" => {
                        for a in args {
                            self.expr(a, false);
                        }
                    }
                    "read_line" | "args" => {}
                    _ => {
                        let facts: Vec<Option<Fact>> = args.iter().map(|a| self.expr(a, false)).collect();
                        // 全引数を評価し終えた時点で判定する（`f(ref a, a)` のように後でムーブされる場合を除く）
                        let safe: Vec<bool> = facts.iter().map(|f| self.is_live_ref(*f)).collect();
                        self.record_call(name, safe);
                    }
                }
                None
            }
            // インタプリタと同じく、添字を先に評価してから配列をたどる
            ExprKind::Index(inner, idx, check) => {
                self.expr(idx, false);
                if is_place(inner) {
                    self.deref(inner, check);
                } else {
                    // 一時的な配列（`make()[0]`）は評価して捨てる
                    self.expr(inner, false);
                    check.set(Check::Required);
                }
                None
            }
            ExprKind::ArrayLit(items, _) => {
                for x in items {
                    self.expr(x, false);
                }
                None
            }
            ExprKind::ArrayRepeat(x, _, _) => {
                self.expr(x, false);
                None
            }
            ExprKind::MapLit(pairs) => {
                for (k, v) in pairs {
                    self.expr(k, false);
                    self.expr(v, false);
                }
                None
            }
            ExprKind::Interp(parts) => {
                // print と同じく借りて読む
                for p in parts {
                    if let InterpPart::Expr(x) = p {
                        self.expr(x, true);
                    }
                }
                None
            }
            ExprKind::MethodCall(recv, name, args, target) => {
                // インタプリタと同じ順: 一時的な値の receiver は先に評価（ムーブ）、
                // 変数などの場所なら引数のあとで借りる。`fn take` のメソッドは receiver をいつも先に読む（ムーブ）
                let take = target.borrow().take;
                let temp = if is_place(recv) && !take { None } else { Some(self.expr(recv, false)) };
                let facts: Vec<Option<Fact>> = args.iter().map(|a| self.expr(a, false)).collect();
                // `this` が呼び出しのあいだ生きているか。一時的な own はこの呼び出しが終わるまで解放されない
                let this_safe = match temp {
                    Some(f) => f == Some(Fact::Own) || self.is_live_ref(f),
                    None => {
                        self.expr(recv, true);
                        match &recv.kind {
                            ExprKind::Var(n) => {
                                let f = self.fact_of(n);
                                f == Some(Fact::Own) || self.is_live_ref(f)
                            }
                            _ => false,
                        }
                    }
                };
                let mut safe = vec![this_safe];
                safe.extend(facts.iter().map(|f| self.is_live_ref(*f)));
                // どの構造体のメソッドかは型しだいなので、同じ名前のメソッドすべての呼び出しとして数える（安全側）
                let suffix = format!(".{}", name);
                let cands: Vec<String> =
                    self.pinned.keys().filter(|k| k.ends_with(suffix.as_str())).map(|k| k.to_string()).collect();
                for c in cands {
                    self.record_call(&c, safe.clone());
                }
                None
            }
        }
    }

    /// 関数 `name` の呼び出しで、各引数が安全な ref かを記録する（全呼び出し箇所の AND）
    fn record_call(&mut self, name: &str, safe: Vec<bool>) {
        let arity_ok = matches!(self.pinned.get(name), Some(p) if p.len() == safe.len());
        if arity_ok {
            let entry = self.calls.entry(name.to_string()).or_insert_with(|| vec![true; safe.len()]);
            for (e, s) in entry.iter_mut().zip(safe) {
                *e = *e && s;
            }
        }
    }

    /// どれか 1 つのブロックが実行される分岐。`otherwise` がなければ、どれも実行されない道もある。
    fn branches<'b>(&mut self, blocks: impl Iterator<Item = &'b Block>, otherwise: Option<&'b Block>) {
        let before = self.facts.clone();
        let mut acc: Option<Facts> = None;
        for b in blocks.chain(otherwise) {
            self.facts = before.clone();
            self.block(b);
            acc = meet_opt(acc, &self.facts);
        }
        if otherwise.is_none() {
            acc = meet_opt(acc, &before);
        }
        self.facts = acc.unwrap_or(before);
    }

    /// `*inner` か `inner.f` の検査を消せるか判定する。`inner` は読んでもムーブしない。
    fn deref(&mut self, inner: &Expr, check: &Cell<Check>) {
        self.expr(inner, true);
        let safe = match &inner.kind {
            ExprKind::Var(name) => {
                let f = self.fact_of(name);
                // own の先は世代チェックが要らない。ref は借りた先が生きていると言えれば安全。
                f == Some(Fact::Own) || self.is_live_ref(f)
            }
            _ => false,
        };
        // ループなどで同じ箇所を何度も見る。一度でも証明できなければ検査を残す。
        let next = match check.get() {
            Check::Unvisited | Check::Elided if safe => Check::Elided,
            _ => Check::Required,
        };
        check.set(next);
    }
}

// ---------------------------------------------------------------- AST の走査

fn for_each_check(b: &Block, f: &mut impl FnMut(&Cell<Check>)) {
    for s in &b.stmts {
        match s {
            Stmt::Let { init, .. } => expr_checks(init, f),
            Stmt::Assign { target, value, .. } => {
                expr_checks(value, f);
                expr_checks(target, f);
            }
            Stmt::Expr(e) => expr_checks(e, f),
            Stmt::While { cond, body } => {
                expr_checks(cond, f);
                for_each_check(body, f);
            }
            Stmt::Loop { body } => for_each_check(body, f),
            Stmt::ForEach { iter, body, els, .. } => {
                expr_checks(iter, f);
                for_each_check(body, f);
                if let Some(els) = els {
                    for_each_check(els, f);
                }
            }
            Stmt::For { range, body, els, .. } => {
                for a in range {
                    expr_checks(a, f);
                }
                for_each_check(body, f);
                if let Some(els) = els {
                    for_each_check(els, f);
                }
            }
            Stmt::Break(_) | Stmt::Continue(_) => {}
            Stmt::Return { value, .. } => {
                if let Some(e) = value {
                    expr_checks(e, f);
                }
            }
        }
    }
}

fn expr_checks(e: &Expr, f: &mut impl FnMut(&Cell<Check>)) {
    match &e.kind {
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::Str(_) | ExprKind::Unit | ExprKind::Var(_) => {}
        ExprKind::Own(i) | ExprKind::Ref(i, _) | ExprKind::Unary(_, i) | ExprKind::Cast(i, _) | ExprKind::Try(i) => {
            expr_checks(i, f)
        }
        ExprKind::Deref(i, c) | ExprKind::Field(i, _, c) => {
            f(c);
            expr_checks(i, f);
        }
        ExprKind::StructLit(_, inits) => {
            for init in inits {
                expr_checks(&init.value, f);
            }
        }
        ExprKind::If { cond, then, els } => {
            expr_checks(cond, f);
            for_each_check(then, f);
            if let Some(els) = els {
                for_each_check(els, f);
            }
        }
        ExprKind::Match { scrut, arms, default } => {
            expr_checks(scrut, f);
            for a in arms {
                for_each_check(&a.body, f);
            }
            if let Some(d) = default {
                for_each_check(d, f);
            }
        }
        ExprKind::Binary(_, l, r) => {
            expr_checks(l, f);
            expr_checks(r, f);
        }
        ExprKind::Call(_, args) | ExprKind::Variant { args, .. } => {
            for a in args {
                expr_checks(a, f);
            }
        }
        ExprKind::MethodCall(recv, _, args, _) => {
            expr_checks(recv, f);
            for a in args {
                expr_checks(a, f);
            }
        }
        ExprKind::Interp(parts) => {
            for p in parts {
                if let InterpPart::Expr(x) = p {
                    expr_checks(x, f);
                }
            }
        }
        ExprKind::Index(b, i, c) => {
            f(c);
            expr_checks(i, f);
            expr_checks(b, f);
        }
        ExprKind::ArrayLit(items, _) => {
            for x in items {
                expr_checks(x, f);
            }
        }
        ExprKind::ArrayRepeat(x, _, _) => expr_checks(x, f),
        ExprKind::Lambda(l) => for_each_check(&l.body, f),
        ExprKind::MapLit(pairs) => {
            for (k, v) in pairs {
                expr_checks(k, f);
                expr_checks(v, f);
            }
        }
    }
}

/// 場所として扱える式か（interp の `is_place` と同じ）
fn is_place(e: &Expr) -> bool {
    matches!(e.kind, ExprKind::Var(_) | ExprKind::Deref(..) | ExprKind::Field(..) | ExprKind::Index(..))
}
