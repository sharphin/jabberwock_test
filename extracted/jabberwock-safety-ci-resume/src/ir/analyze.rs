//! IR の上の静的解析: 借りた先が必ず生きていると証明できた `*p` の世代チェックを消す（`Check::Elided`）。
//!
//! 考え方は構文木の上の解析（`crate::analyze`）と同じ。
//!   * own の変数 `a` が持つ先を解放できるのは、`a` 自身への操作（drop・再代入・ムーブ）だけ。呼ばれた関数は `a` に触れない。
//!   * 変数そのもの（own でない値）の場所を解放できるのも、その変数の drop とムーブだけ。再代入は同じ場所に入れ直すので、
//!     変数を借りた ref は生きたまま（構文木の解析にはない、IR で増えた分）。
//!   * 引数の ref は、すべての呼び出し元が生きている ref を渡すなら、関数の中ずっと安全（Pinned。関数をまたいで繰り返し決める）。
//!   * リストの要素を借りた ref（`ref mut ps[i]`）は、要素を動かしうる操作（リストの操作・呼び出し・リストを持ちうる値の上書きや解放）が
//!     どこにもないあいだ安全（RefElem）。ほかの名前から同じリストに届きうるので、名前ではなく操作の種類で消す。
//!     ただし、取り込みがなく数・真偽・文字列だけを値渡しする直接呼び出しは、呼び出し元のリストに届かないので事実を残せる。
//!
//! 基本ブロックの上の前向きのデータフロー解析で、合流では両方で成り立つ事実だけを残す。
//! 証明できない `*p` は `Check::Required` のまま（実行時に検査する）。デバッグビルドでは、消した検査も実行時に確かめる。

use std::collections::HashMap;

use super::*;
use crate::analyze::Stats;

/// 変数について必ず成り立つこと
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fact {
    /// ムーブされていない own を持っている
    Own,
    /// own の変数の先（とその中）を借りた ref。その変数が `Own` のままなら安全
    RefTo(Local),
    /// 変数そのもの（とその中）を借りた ref。その変数が drop・ムーブされるまで安全（されたら事実ごと消える）
    RefSlot(Local),
    /// 関数の実行中ずっと借りた先が生きている ref（安全な引数から来たもの）
    Pinned,
    /// 変数（Pinned の引数・own・リストの変数）から届くリストの要素を借りた ref。要素を動かしうる操作で消える（`kill_elems`）。
    /// 呼ばれた関数の中では守れないので、引数としては安全と見なさない（`is_live` は偽）
    RefElem(Local),
}

type Facts = HashMap<Local, Fact>;

/// 両方の道で成り立つ事実だけを残す（合流点）
fn meet(a: &Facts, b: &Facts) -> Facts {
    a.iter().filter(|(k, v)| b.get(k) == Some(v)).map(|(k, v)| (*k, *v)).collect()
}

/// 変数 `x` がなくなった（drop・ムーブ）。それを借りた ref の事実も消す
fn kill_all(facts: &mut Facts, x: Local) {
    facts.remove(&x);
    facts.retain(|_, f| *f != Fact::RefTo(x) && *f != Fact::RefSlot(x) && *f != Fact::RefElem(x));
}

/// リストの要素が動いたかもしれない。要素を借りた ref の事実をすべて消す
fn kill_elems(facts: &mut Facts) {
    facts.retain(|_, f| !matches!(f, Fact::RefElem(_)));
}

/// この型の値を上書き・解放すると、リストの中身が解放されうるか（数・文字列・ref は持たない）
fn may_hold_list(t: &Ty) -> bool {
    !matches!(t, Ty::Int(_) | Ty::Float | Ty::Bool | Ty::Str | Ty::Unit | Ty::Never | Ty::Ref(..))
}

/// 呼び出し元のリストへ届く値を受け取らない関数。
/// ref はリストを持たなくても別名から要素を動かせるので、may_hold_list とは分ける。
/// 文字列は不変の値。暗黙の this や取り込みも含め、引数をすべて調べる。
fn scalar_call(f: &Func) -> bool {
    let scalar = |t: &Ty| matches!(t, Ty::Int(_) | Ty::Float | Ty::Bool | Ty::Str | Ty::Unit | Ty::Never);
    f.captures == 0 && scalar(&f.ret) && f.locals[..f.params].iter().all(|l| scalar(&l.ty))
}

/// 変数 `x` に入れ直した。古い own は解放されるが、変数の場所はそのまま
fn kill_value(facts: &mut Facts, x: Local) {
    facts.remove(&x);
    facts.retain(|_, f| *f != Fact::RefTo(x) && *f != Fact::RefElem(x));
}

/// この事実を持つ値は、今、借りた先が生きている ref（か own）か
fn is_live(facts: &Facts, f: Option<Fact>) -> bool {
    match f {
        Some(Fact::Pinned | Fact::RefSlot(_)) => true,
        Some(Fact::RefTo(x)) => facts.get(&x) == Some(&Fact::Own),
        _ => false,
    }
}

/// 全体を解析して、各 `Proj::Deref` に `Check` を書き込む
pub fn analyze(prog: &mut Program) -> Stats {
    let n = prog.fns.len();
    // 引数ごとに「Pinned か」。楽観的に全部 true から始め、反例が出たら false にする（減る一方なので止まる）
    let mut pinned: Vec<Vec<bool>> = prog.fns.iter().map(|f| vec![true; f.params]).collect();
    let tables: Vec<usize> = prog.fns.iter().map(|f| f.params).collect();
    let scalar_calls: Vec<bool> = prog.fns.iter().map(scalar_call).collect();
    loop {
        let mut calls: Vec<Option<Vec<bool>>> = vec![None; n];
        for (fi, pin) in pinned.iter().enumerate() {
            let entry = fixpoint(&prog.fns[fi], pin, &scalar_calls);
            let f = &mut prog.fns[fi];
            for (b, facts) in entry.into_iter().enumerate() {
                let Some(mut facts) = facts else { continue };
                let mut cx = Cx { set: true, calls: Some(&mut calls), arity: &tables, scalar_calls: &scalar_calls, locals: &f.locals };
                let block = &mut f.blocks[b];
                for inst in block.insts.iter_mut() {
                    cx.inst(inst, &mut facts);
                }
                cx.term(&mut block.term, &mut facts);
            }
        }
        // 一度も呼ばれない関数の引数（main・クロージャ）は何も仮定しない
        let next: Vec<Vec<bool>> =
            calls.into_iter().zip(&tables).map(|(c, &p)| c.unwrap_or_else(|| vec![false; p])).collect();
        if next == pinned {
            break;
        }
        pinned = next;
    }
    let mut stats = Stats::default();
    for f in &prog.fns {
        for b in &f.blocks {
            let mut count = |p: &Place| {
                for pr in &p.proj {
                    if let Proj::Deref(c, _) = pr {
                        if *c == Check::Elided {
                            stats.elided += 1;
                        } else {
                            stats.required += 1;
                        }
                    }
                }
            };
            for inst in &b.insts {
                places_of_inst(inst, &mut count);
            }
            if let Term::Switch(p, ..) = &b.term {
                count(p);
            }
        }
    }
    stats
}

/// 各ブロックの入口で成り立つ事実（どこからも来ないブロックは None）
fn fixpoint(f: &Func, pinned: &[bool], scalar_calls: &[bool]) -> Vec<Option<Facts>> {
    let mut entry: Vec<Option<Facts>> = vec![None; f.blocks.len()];
    let mut start = Facts::new();
    for i in 0..f.params {
        if pinned.get(i) == Some(&true) {
            start.insert(i, Fact::Pinned);
        } else if matches!(f.locals[i].ty, Ty::Own(_)) && i < f.params - f.captures {
            start.insert(i, Fact::Own);
        }
    }
    entry[0] = Some(start);
    let mut blocks = f.blocks.clone();
    let mut work = vec![0];
    while let Some(b) = work.pop() {
        let mut facts = entry[b].clone().expect("queued blocks have facts");
        let mut cx = Cx { set: false, calls: None, arity: &[], scalar_calls, locals: &f.locals };
        for inst in blocks[b].insts.iter_mut() {
            cx.inst(inst, &mut facts);
        }
        cx.term(&mut blocks[b].term, &mut facts);
        for s in successors(&blocks[b].term) {
            let next = match &entry[s] {
                Some(old) => meet(old, &facts),
                None => facts.clone(),
            };
            if entry[s].as_ref() != Some(&next) {
                entry[s] = Some(next);
                work.push(s);
            }
        }
    }
    entry
}

fn successors(t: &Term) -> Vec<Bb> {
    match t {
        Term::Goto(b) => vec![*b],
        Term::Branch(_, a, b) | Term::Step { next: a, done: b, .. } => vec![*a, *b],
        Term::Switch(_, arms, d) => arms.iter().map(|(_, b)| *b).chain(std::iter::once(*d)).collect(),
        Term::Return(_) | Term::Fail(..) | Term::Unreachable => Vec::new(),
    }
}

/// 命令を順にたどる。`set` なら場所の検査を決めて書き込み、呼び出しの引数が安全かを記録する
struct Cx<'a> {
    set: bool,
    calls: Option<&'a mut Vec<Option<Vec<bool>>>>,
    /// 関数ごとの引数の数
    arity: &'a [usize],
    /// true の直接呼び出しだけは、呼び出し元のリストに届かない
    scalar_calls: &'a [bool],
    locals: &'a [LocalDecl],
}

impl<'a> Cx<'a> {
    fn inst(&mut self, inst: &mut Inst, facts: &mut Facts) {
        match inst {
            Inst::Assign { place, value, .. } => {
                // 値を先に計算し、そのあとで場所を求める（exec と同じ順）
                let f = self.rvalue(value, facts);
                self.place(place, facts);
                // リストを持ちうる値を上書きすると、古い値の中のリストが解放される
                let holds = if place.proj.is_empty() { may_hold_list(&self.locals[place.local].ty) } else { self.value_may_hold(value) };
                if holds {
                    kill_elems(facts);
                }
                if place.proj.is_empty() {
                    let x = place.local;
                    kill_value(facts, x);
                    // own の型の変数に入れた値は、その変数が持つ own。ただし場所から取り出した値（copy_out）は読むだけの写しでありうるので
                    // 何も仮定しない（取り出しは実行時にエラーになるが、念のため）
                    let copied = matches!(value, Rvalue::CopyOut(..));
                    let f = if matches!(self.locals[x].ty, Ty::Own(_)) && !copied { Some(Fact::Own) } else { f };
                    match f {
                        Some(Fact::RefTo(y) | Fact::RefSlot(y)) if y == x => {}
                        Some(f) => {
                            facts.insert(x, f);
                        }
                        None => {}
                    }
                }
            }
            Inst::Drop { local, .. } => {
                if may_hold_list(&self.locals[*local].ty) {
                    kill_elems(facts);
                }
                kill_all(facts, *local)
            }
            Inst::Discard { value, .. } => {
                let holds = match value {
                    Operand::Move(l, _) | Operand::Read(l, _) => may_hold_list(&self.locals[*l].ty),
                    Operand::Copy(p) if p.proj.is_empty() => may_hold_list(&self.locals[p.local].ty),
                    Operand::Const(_) => false,
                    Operand::Copy(_) => true,
                };
                self.operand(value, facts);
                if holds {
                    kill_elems(facts);
                }
            }
        }
    }

    fn term(&mut self, t: &mut Term, facts: &mut Facts) {
        match t {
            Term::Branch(o, ..) | Term::Return(o) | Term::Step { step: o, .. } => {
                self.operand(o, facts);
            }
            Term::Switch(p, ..) => self.place(p, facts),
            Term::Goto(_) | Term::Fail(..) | Term::Unreachable => {}
        }
    }

    /// 上書きに使う値がリストを持ちうるか（数の計算なら持たない）
    fn value_may_hold(&self, v: &Rvalue) -> bool {
        match v {
            Rvalue::Bin(..) | Rvalue::Un(..) | Rvalue::Cast(..) | Rvalue::Show(..) | Rvalue::Ref { .. } => false,
            Rvalue::Use(Operand::Const(_)) => false,
            Rvalue::Use(Operand::Move(l, _) | Operand::Read(l, _)) => may_hold_list(&self.locals[*l].ty),
            Rvalue::Use(Operand::Copy(p)) if p.proj.is_empty() => may_hold_list(&self.locals[p.local].ty),
            _ => true,
        }
    }

    /// 場所をたどる。先頭の `*` の検査を決める（ほかの `*` は実行時に検査する）
    fn place(&mut self, p: &mut Place, facts: &mut Facts) {
        let safe = {
            let f = facts.get(&p.local).copied();
            // RefElem is valid for direct dereferences while its fact survives.
            // Any operation that can move list elements removes RefElem facts.
            // Do not use is_live here: call arguments intentionally treat RefElem as unsafe,
            // because the callee may mutate the backing list.
            f == Some(Fact::Own) || matches!(f, Some(Fact::RefElem(_))) || is_live(facts, f)
        };
        if self.set {
            for (i, pr) in p.proj.iter_mut().enumerate() {
                if let Proj::Deref(c, _) = pr {
                    *c = if i == 0 && safe { Check::Elided } else { Check::Required };
                }
            }
        }
        for pr in p.proj.iter_mut() {
            if let Proj::Index(k, ..) = pr {
                self.operand(k, facts);
            }
        }
    }

    /// 値を読む。ムーブなら変数の事実を消す。読んだ値について分かることを返す
    fn operand(&mut self, o: &mut Operand, facts: &mut Facts) -> Option<Fact> {
        match o {
            Operand::Const(_) => None,
            Operand::Copy(p) => {
                let f = if p.proj.is_empty() { facts.get(&p.local).copied() } else { None };
                self.place(p, facts);
                f
            }
            Operand::Move(l, _) | Operand::Read(l, _) => {
                let f = facts.get(l).copied();
                kill_all(facts, *l);
                f
            }
        }
    }

    /// 値を順に読む。呼び出しの引数として安全か（読む前の事実で決める）も返す
    fn operands(&mut self, os: &mut [Operand], facts: &mut Facts) -> Vec<bool> {
        let mut fs = Vec::new();
        for o in os.iter() {
            fs.push(match o {
                Operand::Copy(p) if p.proj.is_empty() => facts.get(&p.local).copied(),
                Operand::Move(l, _) | Operand::Read(l, _) => facts.get(l).copied(),
                _ => None,
            });
        }
        // `f(ref a, a)` のように後の引数でムーブされるものは、前の命令で事実が消えている
        for o in os.iter_mut() {
            self.operand(o, facts);
        }
        // Evaluate argument liveness only after every argument has been consumed.
        // A later Move may invalidate the owner borrowed by an earlier argument.
        fs.iter().map(|f| is_live(facts, *f)).collect()
    }

    fn record(&mut self, fi: usize, safe: Vec<bool>) {
        let Some(calls) = self.calls.as_deref_mut() else { return };
        if self.arity.get(fi) != Some(&safe.len()) {
            return;
        }
        let e = calls[fi].get_or_insert_with(|| vec![true; safe.len()]);
        for (a, s) in e.iter_mut().zip(safe) {
            *a = *a && s;
        }
    }

    /// `ref 場所` で作った ref について分かること
    fn ref_fact(&self, place: &Place, follows: bool, facts: &Facts) -> Option<Fact> {
        let root = facts.get(&place.local).copied();
        // 添字は通さない（リストの要素はリストの中身の区画にあり、長さが変わると無効になる。
        // 配列の添字も、型を見ずに見分けられないので安全側に倒す）
        let rest_ok = |from: usize| place.proj[from..].iter().all(|p| matches!(p, Proj::Field(..)));
        match place.proj.first() {
            // 変数そのもの
            None if follows => match root? {
                Fact::Own => Some(Fact::RefTo(place.local)),
                f => Some(f), // ref の借り直しは同じ先
            },
            None => Some(Fact::RefSlot(place.local)),
            // `*a` の中（own の先の場所。own の値を借りるならその先なので分からない）
            Some(Proj::Deref(..)) if !follows && rest_ok(1) => match root? {
                Fact::Own => Some(Fact::RefTo(place.local)),
                f if is_live(facts, Some(f)) => Some(f),
                _ => None,
            },
            // 変数の中（own でない値のフィールド・配列の要素）
            Some(Proj::Field(..) | Proj::Index(..)) if !follows && rest_ok(0) => {
                if matches!(self.locals[place.local].ty, Ty::Own(_) | Ty::Ref(..)) {
                    None
                } else {
                    Some(Fact::RefSlot(place.local))
                }
            }
            // リストの要素（とその中）: `(*ps)[i]`（ps は Pinned の引数か own）・`ps[i]`（ps はリストの変数）
            _ if !follows => {
                let elems = |from: usize| place.proj[from..].iter().all(|p| matches!(p, Proj::Field(..) | Proj::Index(..)));
                match place.proj.first() {
                    Some(Proj::Deref(..)) if elems(1) && matches!(root, Some(Fact::Pinned | Fact::Own)) => {
                        Some(Fact::RefElem(place.local))
                    }
                    Some(Proj::Index(..)) if elems(0) && matches!(self.locals[place.local].ty, Ty::List(_)) => {
                        Some(Fact::RefElem(place.local))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn rvalue(&mut self, r: &mut Rvalue, facts: &mut Facts) -> Option<Fact> {
        match r {
            Rvalue::Use(o) => self.operand(o, facts),
            Rvalue::Bin(_, a, b, _) => {
                self.operand(a, facts);
                self.operand(b, facts);
                None
            }
            Rvalue::Un(_, a, _) | Rvalue::Cast(a, _) | Rvalue::Repeat(a, ..) | Rvalue::Show(a, ..) => {
                self.operand(a, facts);
                None
            }
            Rvalue::Own(a) => {
                self.operand(a, facts);
                Some(Fact::Own)
            }
            Rvalue::Ref { place, follows, .. } => {
                let f = self.ref_fact(place, *follows, facts);
                self.place(place, facts);
                f
            }
            Rvalue::Struct(_, ops) | Rvalue::Enum(_, ops, _) | Rvalue::Array(ops, _) => {
                self.operands(ops, facts);
                None
            }
            Rvalue::Builtin(name, ops, _) => {
                self.operands(ops, facts);
                // 数と文字列の計算・表示のほかは、何が起きるか個別に見ない（スレッド・入出力など）
                let pure = matches!(name.as_str(), "print" | "eprint" | "concat" | "append" | "assert");
                if !(pure || name.starts_with("f64.") || name.starts_with("int.") || name.starts_with("str.")) {
                    kill_elems(facts);
                }
                None
            }
            Rvalue::Map(pairs, _) => {
                for (k, v) in pairs.iter_mut() {
                    self.operand(k, facts);
                    self.operand(v, facts);
                }
                None
            }
            Rvalue::Call(fi, ops, _) => {
                let safe = self.operands(ops, facts);
                self.record(*fi, safe);
                // 引数の評価による無効化は、上の operands が既に反映している。
                // 不明な番号も安全側。関数値と interface の呼び出しは従来どおり消す。
                if self.scalar_calls.get(*fi) != Some(&true) {
                    kill_elems(facts);
                }
                None
            }
            Rvalue::Closure(fi, ops) => {
                if ops.is_empty() {
                    // 名前のある関数を値にした。どこから何が渡されるか分からない
                    let n = self.arity.get(*fi).copied().unwrap_or(0);
                    self.record(*fi, vec![false; n]);
                }
                self.operands(ops, facts);
                None
            }
            Rvalue::CallValue(fv, ops, _) => {
                self.operands(ops, facts);
                self.operand(fv, facts);
                kill_elems(facts);
                None
            }
            Rvalue::CallDyn { table, this, args, .. } => {
                let mut safe = self.operands(args, facts);
                let this_safe = is_live(facts, self.ref_fact(this, false, facts));
                self.place(this, facts);
                safe.insert(0, this_safe);
                for (_, fi) in table.iter() {
                    self.record(*fi, safe.clone());
                }
                kill_elems(facts);
                None
            }
            Rvalue::SeqOp { name, place, args, .. } => {
                self.operands(args, facts);
                self.place(place, facts);
                // 読むだけの操作のほかは、要素が動きうる（push・pop・sort など。辞書の値の解放も）
                if !matches!(name.as_str(), "len" | "contains" | "get" | "keys") {
                    kill_elems(facts);
                }
                None
            }
            Rvalue::CopyOut(p, ..) | Rvalue::Take(p) | Rvalue::Items { place: p, .. } | Rvalue::Keys(p) => {
                self.place(p, facts);
                None
            }
        }
    }
}

/// 命令の中の場所（数えるため）
fn places_of_inst(inst: &Inst, f: &mut impl FnMut(&Place)) {
    let op = |o: &Operand, f: &mut dyn FnMut(&Place)| {
        if let Operand::Copy(p) = o {
            f(p);
        }
    };
    match inst {
        Inst::Assign { place, value, .. } => {
            f(place);
            match value {
                Rvalue::Use(o) | Rvalue::Un(_, o, _) | Rvalue::Cast(o, _) | Rvalue::Own(o) | Rvalue::Repeat(o, ..) => {
                    op(o, f)
                }
                Rvalue::Show(o, ..) => op(o, f),
                Rvalue::Bin(_, a, b, _) => {
                    op(a, f);
                    op(b, f);
                }
                Rvalue::Ref { place, .. }
                | Rvalue::CopyOut(place, ..)
                | Rvalue::Take(place)
                | Rvalue::Items { place, .. }
                | Rvalue::Keys(place) => f(place),
                Rvalue::Struct(_, ops)
                | Rvalue::Enum(_, ops, _)
                | Rvalue::Array(ops, _)
                | Rvalue::Builtin(_, ops, _)
                | Rvalue::Call(_, ops, _)
                | Rvalue::Closure(_, ops) => ops.iter().for_each(|o| op(o, f)),
                Rvalue::Map(pairs, _) => pairs.iter().for_each(|(k, v)| {
                    op(k, f);
                    op(v, f)
                }),
                Rvalue::CallValue(fv, ops, _) => {
                    ops.iter().for_each(|o| op(o, f));
                    op(fv, f);
                }
                Rvalue::CallDyn { this, args, .. } => {
                    args.iter().for_each(|o| op(o, f));
                    f(this);
                }
                Rvalue::SeqOp { place, args, .. } => {
                    args.iter().for_each(|o| op(o, f));
                    f(place);
                }
            }
        }
        Inst::Discard { value, .. } => op(value, f),
        Inst::Drop { .. } => {}
    }
}
