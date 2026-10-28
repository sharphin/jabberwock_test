//! IR を実行する小さなインタプリタ。今のインタプリタ（`interp.rs`）と出力・エラー・リークの有無を突き合わせて、
//! 書き出し（`lower`）が正しいことを確かめるためのもの。
//!
//! ヒープ（スロット＋世代）・表示・計算は `Interp` のものをそのまま使う。変数も interp と同じくスロットに置く
//! （`ref x` で変数そのものを借りられるように）。

use std::rc::Rc;
use std::sync::atomic::Ordering;

use super::*;
use crate::ast::UnOp;
use crate::diag::Diagnostic;
use crate::interp::{text_rc, 
    Callee,
    as_bool, binary, bounds, float_to_int, locate, map_borrow_error, moved_error, none, some,
    str_method, Interp, MapVal, RefVal,
    Value, MAX_DEPTH,
};

use crate::threads::{aborted_marker, Portable};

type R<T> = Result<T, Diagnostic>;

/// 変数の今の状態
#[derive(Clone)]
enum Loc {
    /// まだ値がない（宣言の前・drop のあと）
    Unset,
    Live(usize),
    /// ムーブ済み（どこでムーブしたか）
    Moved(Span),
}

/// `main` を実行する。出力・ヒープは `it` に残る
pub fn run(prog: &Program, it: &mut Interp) -> R<()> {
    let main = prog.by_name["main"];
    it.set_structs(prog.structs.clone());
    let span = prog.fns[main].span;
    let mut m = Machine { prog, it, depth: 0, views: Vec::new() };
    let r = m.call(main, Vec::new(), Vec::new(), span).map(|_| ());
    it.finish_main(r, span)
}

struct Machine<'a> {
    prog: &'a Program,
    it: &'a mut Interp,
    depth: usize,
    /// 呼び出し中の関数ごとに、取り込んだムーブする値の写しを持つ変数（値として読めない。interp の `Var.view`）
    views: Vec<Vec<Local>>,
}

#[cfg(test)]
pub(crate) fn retirement_probe(prog: &Program, it: &mut Interp, slot: usize, span: Span) -> R<Value> {
    let mut machine = Machine { prog, it, depth: 0, views: Vec::new() };
    machine.seq_op("reverse", slot, &[], Vec::new(), span, span)
}

impl<'a> Machine<'a> {
    fn call(&mut self, fi: usize, args: Vec<Value>, views: Vec<Local>, span: Span) -> R<Value> {
        if self.it.world.aborted.load(Ordering::Relaxed) {
            return Err(aborted_marker());
        }
        if self.depth >= MAX_DEPTH {
            return Err(Diagnostic::new("関数呼び出しが深すぎます")
                .primary(span, format!("呼び出しの深さが {} を超えました", MAX_DEPTH)));
        }
        self.depth += 1;
        self.views.push(views);
        let prog = self.prog;
        let f = &prog.fns[fi];
        let mut locs = vec![Loc::Unset; f.locals.len()];
        for (i, a) in args.into_iter().enumerate() {
            locs[i] = Loc::Live(self.it.heap.alloc(a));
        }
        let mut bb = 0;
        loop {
            let block = &f.blocks[bb];
            for inst in &block.insts {
                self.inst(f, &mut locs, inst)?;
            }
            bb = match &block.term {
                Term::Goto(b) => *b,
                Term::Branch(c, t, e) => {
                    if self.it.world.aborted.load(Ordering::Relaxed) {
                        return Err(aborted_marker());
                    }
                    let v = self.operand(f, &mut locs, c)?;
                    if as_bool(v, f.span)? {
                        *t
                    } else {
                        *e
                    }
                }
                Term::Return(o) => {
                    let v = self.operand(f, &mut locs, o)?;
                    self.depth -= 1;
                    self.views.pop();
                    return Ok(v);
                }
                Term::Switch(place, arms, other) => {
                    let (slot, path) = self.resolve(f, &mut locs, place)?;
                    let Value::Enum { variant, .. } = at(self.it, slot, &path) else { unreachable!("switch on an enum") };
                    arms.iter().find(|(v, _)| *v == *variant).map_or(*other, |(_, b)| *b)
                }
                Term::Step { counter, step, next, done } => {
                    let Loc::Live(slot) = locs[*counter] else { unreachable!("loop counter is live") };
                    let st = match self.operand(f, &mut locs, step)? {
                        Value::Int(n, _) => n,
                        _ => unreachable!("range step is an integer"),
                    };
                    let v = self.it.heap.slots[slot].value.as_mut().expect("live slot");
                    let Value::Int(n, t) = v else { unreachable!("loop counter is an integer") };
                    let n2 = *n + st;
                    if t.contains(n2) {
                        *n = n2;
                        *next
                    } else {
                        *done
                    }
                }
                Term::Fail(title, span, label) => return Err(Diagnostic::new(title.clone()).primary(*span, label.clone())),
                Term::Unreachable => unreachable!("reached an unreachable block in {}", f.name),
            };
        }
    }

    fn inst(&mut self, f: &Func, locs: &mut [Loc], inst: &Inst) -> R<()> {
        match inst {
            Inst::Assign { place, value, span, what } => {
                // own（own の配列）を場所から取り出すのは、中身が読むだけの写しでもエラー（interp の `takes_own`）
                if let Rvalue::CopyOut(from, what, sp) = value {
                    let t = &f.locals[place.local].ty;
                    if place.proj.is_empty() && holds_own(t) {
                        self.resolve(f, locs, from)?;
                        let name = if matches!(t, Ty::Own(_)) { "own" } else { "配列" };
                        return Err(Diagnostic::new(format!("{}にある {} は取り出せません", what, name))
                            .primary(*sp, "所有権をここから動かすことはできない"));
                    }
                }
                let v = self.rvalue(f, locs, value)?;
                if place.proj.is_empty() {
                    // 解析は「own 型の変数には own が入っている」を根拠にする
                    debug_assert!(
                        !matches!(f.locals[place.local].ty, Ty::Own(_)) || matches!(v, Value::Own(_)),
                        "own 型の {} に own でない値", f.locals[place.local].name
                    );
                    match locs[place.local] {
                        Loc::Live(slot) => {
                            let old = self.it.heap.slots[slot].value.replace(v).expect("live slot");
                            let why = format!("`{}` に再代入されて、古い値が解放された", f.locals[place.local].name);
                            self.it.release(old, *span, &why);
                        }
                        Loc::Unset | Loc::Moved(_) => locs[place.local] = Loc::Live(self.it.heap.alloc(v)),
                    }
                    return Ok(());
                }
                let (slot, path) = self.resolve_in(f, locs, place, true)?;
                let old = std::mem::replace(at_mut(self.it, slot, &path), v);
                self.it.release(old, *span, &overwritten(what));
            }
            Inst::Drop { local, span, why } => {
                if let Loc::Live(slot) = locs[*local] {
                    let v = self.it.vacate(slot, *span, why);
                    self.it.release(v, *span, why);
                }
                locs[*local] = Loc::Unset;
            }
            Inst::Discard { value, span, why } => {
                let v = self.operand(f, locs, value)?;
                self.it.release(v, *span, why);
            }
        }
        Ok(())
    }

    /// 場所が指すスロットと、その中の道筋。ref の先は世代チェックをする
    fn resolve(&mut self, f: &Func, locs: &mut [Loc], place: &Place) -> R<(usize, Vec<usize>)> {
        self.resolve_in(f, locs, place, false)
    }

    /// `create` なら、最後の辞書のキーがなければ入れる（`m[k] = v` の代入。値は呼び出し元が置く）
    fn resolve_in(&mut self, f: &Func, locs: &mut [Loc], place: &Place, create: bool) -> R<(usize, Vec<usize>)> {
        let mut slot = match locs[place.local] {
            Loc::Live(s) => s,
            Loc::Moved(at) => return Err(moved_error(&f.locals[place.local].name, at, place.span)),
            Loc::Unset => unreachable!("read of an unset local _{} in {}", place.local, f.name),
        };
        let mut path = Vec::new();
        for p in &place.proj {
            match p {
                Proj::Deref(check, span) => {
                    let ptr = at(self.it, slot, &path).clone();
                    let (s, pth) = self.it.check_ptr(&ptr, *span, *check)?;
                    slot = s;
                    path = pth;
                }
                Proj::Field(i, _) | Proj::Variant(_, i) => path.push(*i),
                Proj::Index(k, key_span, base_span) => {
                    let key = self.operand(f, locs, k)?;
                    let last = std::ptr::eq(p, place.proj.last().expect("non-empty"));
                    // リストの要素は、リストの中身のスロットから数える
                    if let Value::List(s) = at(self.it, slot, &path) {
                        slot = *s;
                        path = Vec::new();
                    }
                    let c = at_mut(self.it, slot, &path);
                    let i = match c {
                        Value::Map(m) if create && last && m.find(&key).is_none() => {
                            m.insert(key, Value::Unit);
                            m.entries.len() - 1
                        }
                        c => locate(c, &key, *key_span, *base_span)?,
                    };
                    path.push(i);
                }
            }
        }
        Ok((slot, path))
    }

    fn operand(&mut self, f: &Func, locs: &mut [Loc], o: &Operand) -> R<Value> {
        match o {
            Operand::Const(c) => Ok(match c {
                Const::Unit => Value::Unit,
                Const::Int(n, t) => Value::Int(*n, *t),
                Const::Float(x) => Value::Float(*x),
                Const::Bool(b) => Value::Bool(*b),
                Const::Str(s) => Value::Str(text_rc(s)),
            }),
            Operand::Copy(place) => {
                let (slot, path) = self.resolve(f, locs, place)?;
                // 写しは別の値（中のリストも写す。実行ファイルの複製と同じ）
                let v = at(self.it, slot, &path).clone();
                Ok(self.it.copy_lists(v).0)
            }
            Operand::Read(l, span) => match locs[*l] {
                Loc::Live(_) if matches!(self.views.last(), Some(v) if v.contains(l)) => {
                    Err(Diagnostic::new(format!("クロージャに取り込んだ `{}` は取り出せません", f.locals[*l].name))
                        .primary(*span, "クロージャは何度も呼べるので、取り込んだ値をムーブできない"))
                }
                Loc::Live(slot) if self.it.heap.slots[slot].value.as_ref().expect("live slot").moves() => {
                    self.operand(f, locs, &Operand::Move(*l, *span))
                }
                Loc::Live(slot) => Ok(self.it.heap.slots[slot].value.clone().expect("live slot")),
                Loc::Moved(at) => Err(moved_error(&f.locals[*l].name, at, *span)),
                Loc::Unset => unreachable!("read of an unset local _{} in {}", l, f.name),
            },
            Operand::Move(l, span) => match locs[*l] {
                Loc::Live(slot) => {
                    let name = &f.locals[*l].name;
                    let v = self.it.vacate(slot, *span, &format!("`{}` がムーブされた", name));
                    locs[*l] = Loc::Moved(*span);
                    Ok(v)
                }
                Loc::Moved(at) => Err(moved_error(&f.locals[*l].name, at, *span)),
                Loc::Unset => unreachable!("move of an unset local _{} in {}", l, f.name),
            },
        }
    }

    fn operands(&mut self, f: &Func, locs: &mut [Loc], os: &[Operand]) -> R<Vec<Value>> {
        let mut out = Vec::new();
        for o in os {
            out.push(self.operand(f, locs, o)?);
        }
        Ok(out)
    }

    fn rvalue(&mut self, f: &Func, locs: &mut [Loc], r: &Rvalue) -> R<Value> {
        match r {
            Rvalue::Use(o) => self.operand(f, locs, o),
            Rvalue::Bin(op, a, b, span) => {
                let a = self.operand(f, locs, a)?;
                let b = self.operand(f, locs, b)?;
                binary(*op, a, b, *span)
            }
            Rvalue::Un(op, a, span) => match (op, self.operand(f, locs, a)?) {
                (UnOp::Neg, Value::Int(n, t)) => Ok(Value::Int(t.wrap(-n), t)),
                (UnOp::Neg, Value::Float(x)) => Ok(Value::Float(-x)),
                (UnOp::Not, Value::Bool(b)) => Ok(Value::Bool(!b)),
                (UnOp::BitNot, Value::Int(n, t)) => Ok(Value::Int(t.wrap(!n), t)),
                (_, other) => Err(Diagnostic::new("型が合わない演算です")
                    .primary(*span, format!("{} にはこの演算を使えません", other.type_name()))),
            },
            Rvalue::Cast(a, ty) => Ok(match (self.operand(f, locs, a)?, ty) {
                (Value::Int(n, _), Ty::Int(t)) => Value::Int(t.wrap(n), *t),
                (Value::Int(n, _), Ty::Float) => Value::Float(n as f64),
                (Value::Float(x), Ty::Int(t)) => Value::Int(t.wrap(float_to_int(x)), *t),
                (v, _) => v,
            }),
            Rvalue::Ref { place, mutable, what, span, .. } => {
                let (slot, path) = self.resolve(f, locs, place)?;
                Ok(match at(self.it, slot, &path) {
                    Value::Own(s) => {
                        let s = *s;
                        Value::Ref(RefVal {
                            slot: s,
                            generation: self.it.heap.slots[s].generation,
                            path: Vec::new(),
                            borrowed_at: *span,
                            what: what.clone(),
                            mutable: *mutable,
                            view: false,
                        })
                    }
                    Value::Ref(r) => Value::Ref(RefVal { mutable: *mutable, view: false, ..r.clone() }),
                    _ => {
                        if self.it.through_map(slot, &path) {
                            return Err(map_borrow_error(*span));
                        }
                        Value::Ref(RefVal {
                            slot,
                            generation: self.it.heap.slots[slot].generation,
                            path,
                            borrowed_at: *span,
                            what: what.clone(),
                            mutable: *mutable,
                            view: false,
                        })
                    }
                })
            }
            Rvalue::Enum(variant, ops, copy) => {
                Ok(Value::Enum { variant: variant.clone(), fields: self.operands(f, locs, ops)?, copy: *copy })
            }
            Rvalue::Take(place) => {
                let (slot, path) = self.resolve(f, locs, place)?;
                Ok(std::mem::replace(at_mut(self.it, slot, &path), Value::Unit))
            }
            Rvalue::Array(items, list) => {
                let items = self.operands(f, locs, items)?;
                Ok(if *list { self.it.new_list(items) } else { Value::Array(items, false) })
            }
            Rvalue::Repeat(x, n, list) => {
                let v = self.operand(f, locs, x)?;
                let items = vec![v; *n as usize];
                Ok(if *list { self.it.new_list(items) } else { Value::Array(items, false) })
            }
            Rvalue::Map(pairs, span) => {
                let mut m = Box::<MapVal>::default();
                for (k, v) in pairs {
                    let k = self.operand(f, locs, k)?;
                    let v = self.operand(f, locs, v)?;
                    if let Some(old) = m.insert(k, v) {
                        self.it.release(old, *span, "辞書の同じキーに後から入れた値で置き換えられた");
                    }
                }
                Ok(Value::Map(m))
            }
            Rvalue::SeqOp { name, place, args, span, arg_span } => {
                let vals = self.operands(f, locs, args)?;
                let (slot, path) = self.resolve(f, locs, place)?;
                self.seq_op(name, slot, &path, vals, *span, *arg_span)
            }
            Rvalue::Items { place, what, span } => {
                let (slot, path) = self.resolve(f, locs, place)?;
                let items: Vec<Value> = match self.it.seq_at(at(self.it, slot, &path)) {
                    Value::Array(items, _) => items.clone(),
                    Value::Map(m) => m.entries.iter().map(|(_, v)| v.clone()).collect(),
                    _ => unreachable!("for over an array, a list or a map"),
                };
                let views = items.into_iter().map(|v| self.it.view_of(v, *span, what)).collect();
                Ok(self.it.new_list(views))
            }
            Rvalue::Keys(place) => {
                let (slot, path) = self.resolve(f, locs, place)?;
                let keys = match self.it.seq_at(at(self.it, slot, &path)) {
                    Value::Array(items, _) => (0..items.len()).map(|i| Value::Int(i as i128, IntTy::I64)).collect(),
                    Value::Map(m) => m.entries.iter().map(|(k, _)| k.clone()).collect(),
                    _ => unreachable!("for over an array, a list or a map"),
                };
                Ok(self.it.new_list(keys))
            }
            Rvalue::Own(a) => {
                let v = self.operand(f, locs, a)?;
                Ok(Value::Own(self.it.heap.alloc(v)))
            }
            Rvalue::Struct(name, ops) => {
                let def = self.prog.structs[name].clone();
                Ok(Value::Struct(def, self.operands(f, locs, ops)?))
            }
            Rvalue::Call(fi, ops, span) => {
                let args = self.operands(f, locs, ops)?;
                self.call(*fi, args, Vec::new(), *span)
            }
            Rvalue::CallDyn { table, this, mutable, what, args, span } => {
                let mut vals = self.operands(f, locs, args)?;
                let (slot, path) = self.resolve(f, locs, this)?;
                let Value::Struct(def, _) = at(self.it, slot, &path) else { unreachable!("interface values are structs") };
                let fi = table.iter().find(|(n, _)| *n == def.name).map(|(_, i)| *i).ok_or_else(|| {
                    Diagnostic::new(format!("`{}` にメソッドがありません", def.name)).primary(*span, "ここ")
                })?;
                let r = RefVal {
                    slot,
                    generation: self.it.heap.slots[slot].generation,
                    path,
                    borrowed_at: this.span,
                    what: what.clone(),
                    mutable: *mutable,
                    view: false,
                };
                vals.insert(0, Value::Ref(r));
                self.call(fi, vals, Vec::new(), *span)
            }
            Rvalue::Closure(fi, ops) => Ok(Value::Func(Callee::Ir(*fi), self.operands(f, locs, ops)?)),
            Rvalue::CallValue(fv, ops, span) => {
                let args = self.operands(f, locs, ops)?;
                let fv = self.operand(f, locs, fv)?;
                self.call_value(&fv, args, *span)
            }
            Rvalue::CopyOut(place, what, span) => {
                let (slot, path) = self.resolve(f, locs, place)?;
                let v = at(self.it, slot, &path).clone();
                if self.it.has_own(&v) || v.moves() {
                    return Err(Diagnostic::new(format!("{}にある {} は取り出せません", what, v.type_name()))
                        .primary(*span, "所有権をここから動かすことはできない"));
                }
                Ok(v)
            }
            Rvalue::Show(Operand::Copy(p), false, span) => {
                // 場所の値は写さずに見る（実行ファイルと同じ）
                let (slot, path) = self.resolve(f, locs, p)?;
                let v = at(self.it, slot, &path).clone();
                let shown = self.it.show(&v, *span, 0)?;
                Ok(Value::Str(text_rc(shown.as_str())))
            }
            Rvalue::Show(o, temp, span) => {
                let v = self.operand(f, locs, o)?;
                let shown = self.it.show(&v, *span, 0);
                if *temp {
                    self.it.release(v, *span, "使い終わった一時的な値");
                }
                Ok(Value::Str(text_rc(shown?.as_str())))
            }
            Rvalue::Builtin(name, ops, _) if name.starts_with("f64.") || name.starts_with("int.") => {
                let mut vals = self.operands(f, locs, ops)?;
                let x = vals.remove(0);
                Ok(crate::interp::num_method(x, &name[4..], vals))
            }
            Rvalue::Builtin(name, ops, span) if name.starts_with("str.") => {
                let mut vals = self.operands(f, locs, ops)?;
                let Value::Str(text) = vals.remove(0) else { unreachable!("string method on a string") };
                let v = str_method(&text, &name[4..], vals, *span)?;
                Ok(self.it.adopt(v))
            }
            Rvalue::Builtin(name, ops, span) if name.starts_with("task.") || name.starts_with("chan.") => {
                let mut vals = self.operands(f, locs, ops)?;
                let h = vals.remove(0);
                self.it.thread_method(&h, &name[5..], vals, *span)
            }
            Rvalue::Builtin(name, ops, span) if name == "spawn" => {
                let mut caps = self.operands(f, locs, ops)?;
                let Value::Int(fi, _) = caps.remove(0) else { unreachable!("spawn takes a function number") };
                let fi = fi as usize;
                let span = *span;
                let caps: Vec<Portable> = caps.into_iter().map(|c| self.it.export(c, span)).collect();
                self.it.start_thread(span, Box::new(move |it: &mut Interp| {
                    let prog = (it.world.load_ir.as_ref().expect("IR world"))();
                    it.set_structs(prog.structs.clone());
                    let args: Vec<Value> = caps.into_iter().map(|c| it.import(c)).collect();
                    // 1 回だけ呼ぶので、取り込んだ値は写しにせずそのまま渡す
                    let mut m = Machine { prog: &prog, it, depth: 0, views: Vec::new() };
                    m.call(fi, args, Vec::new(), span)
                }))
            }
            Rvalue::Builtin(name, ops, span) => {
                let vals = self.operands(f, locs, ops)?;
                let strs = || -> Vec<String> {
                    vals.iter()
                        .map(|v| match v {
                            Value::Str(s) => s.to_string(),
                            _ => unreachable!("shown values are strings"),
                        })
                        .collect()
                };
                match name.as_str() {
                    "print" => {
                        let line = strs().join(" ");
                        self.it.world.print_line(&line);
                        Ok(Value::Unit)
                    }
                    "eprint" => {
                        let line = strs().join(" ");
                        self.it.world.eprint_line(&line);
                        Ok(Value::Unit)
                    }
                    "exit" | "read_file" | "write_file" | "run_command" => self.it.system(name, vals, *span),
                    "chr" => {
                        let Value::Int(n, _) = vals[0] else { unreachable!("type checked: i64") };
                        crate::interp::chr(n, *span)
                    }
                    "concat" | "append" => Ok(Value::Str(text_rc(strs().concat().as_str()))),
                    "read_line" => self.it.read_line(*span),
                    "chan" => Ok(Value::Chan(std::sync::Arc::new(crate::threads::Chan::default()))),
                    "args" => Ok(self.it.args_value()),
                    "assert" => {
                        if !as_bool(vals[0].clone(), *span)? {
                            return Err(Diagnostic::new("assert に失敗しました").primary(*span, "この条件が false でした"));
                        }
                        Ok(Value::Unit)
                    }
                    _ => unreachable!("unknown builtin {}", name),
                }
            }
        }
    }

    /// 関数の値を呼ぶ（interp の `call_func`）。取り込んだ値は読むだけの写しにして、最後の引数として渡す
    fn call_value(&mut self, fv: &Value, mut args: Vec<Value>, span: Span) -> R<Value> {
        let Value::Func(Callee::Ir(fi), caps) = fv else {
            return Err(Diagnostic::new("関数ではない値を呼び出しました")
                .primary(span, format!("これは {} です", fv.type_name())));
        };
        let callee = &self.prog.fns[*fi];
        let first = callee.params - callee.captures;
        let mut views = Vec::new();
        for (i, c) in caps.iter().enumerate() {
            let l = first + i;
            if c.moves() {
                views.push(l);
            }
            args.push(self.it.view_of(c.clone(), span, &callee.locals[l].name));
        }
        self.call(*fi, args, views, span)
    }

    /// 配列・リスト・辞書の操作（interp の `eval_method` の配列・辞書の部分と同じ）
    fn seq_op(&mut self, name: &str, slot: usize, path: &[usize], vals: Vec<Value>, span: Span, arg_span: Span) -> R<Value> {
        // リストなら中身のスロットで操作し、要素が動いたら世代を進める（interp の `list_moved`）
        let (slot, path, list) = match at(self.it, slot, path) {
            Value::List(s) => (*s, Vec::new(), true),
            _ => (slot, path.to_vec(), false),
        };
        let len = match at(self.it, slot, &path) {
            Value::Array(items, _) => items.len(),
            _ => 0,
        };
        let r = self.seq_op_in(name, slot, &path, vals, span, arg_span)?;
        let moved = match name {
            "push" | "insert" | "remove" | "reverse" | "drain" | "sort" | "sort_by" | "sort_unstable" | "sort_unstable_by" => true,
            "pop" => len > 0,
            _ => false,
        };
        if list && moved {
            self.it.list_moved(slot, span, name)?;
        }
        Ok(self.it.adopt(r))
    }

    fn seq_op_in(&mut self, name: &str, slot: usize, path: &[usize], mut vals: Vec<Value>, span: Span, arg_span: Span) -> R<Value> {
        if name == "sort_by" || name == "sort_unstable_by" {
            return self.sort_by(slot, path, vals.remove(0), name == "sort_by", span);
        }
        if name == "sort" || name == "sort_unstable" {
            let Value::Array(items, _) = at(self.it, slot, path) else { unreachable!("sort on an array") };
            let items = items.clone();
            let mut less = |_: &mut (), a: usize, b: usize| -> R<bool> {
                Ok(match (&items[a], &items[b]) {
                    (Value::Int(x, _), Value::Int(y, _)) => x < y,
                    (Value::Float(x), Value::Float(y)) => x < y,
                    (Value::Str(x), Value::Str(y)) => x < y,
                    _ => unreachable!("type checked: ordered elements"),
                })
            };
            let order = crate::sort::sort(&mut (), items.len(), name == "sort", &mut less)?;
            let Value::Array(items, _) = at_mut(self.it, slot, path) else { unreachable!() };
            let mut old: Vec<Option<Value>> = std::mem::take(items).into_iter().map(Some).collect();
            *items = order.into_iter().map(|i| old[i].take().expect("each index once")).collect();
            return Ok(Value::Unit);
        }
        let c = at_mut(self.it, slot, path);
        if let Value::Map(m) = c {
            return Ok(match name {
                "len" => Value::Int(m.entries.len() as i128, IntTy::I64),
                "contains" => Value::Bool(m.find(&vals[0]).is_some()),
                "get" => match m.find(&vals[0]) {
                    Some(i) => some(m.entries[i].1.clone()),
                    None => none(),
                },
                "remove" => match m.remove(&vals[0]) {
                    Some(v) => {
                        let copy = !v.moves();
                        Value::Enum { variant: Rc::from("Some"), fields: vec![v], copy }
                    }
                    None => none(),
                },
                "keys" => Value::Array(m.entries.iter().map(|(k, _)| k.clone()).collect(), true),
                _ => unreachable!("map method {}", name),
            });
        }
        let Value::Array(items, _) = c else { unreachable!("array method on an array") };
        Ok(match name {
            "len" => Value::Int(items.len() as i128, IntTy::I64),
            "push" => {
                items.push(vals.remove(0));
                Value::Unit
            }
            "pop" => match items.pop() {
                Some(v) => {
                    let copy = !v.moves();
                    Value::Enum { variant: Rc::from("Some"), fields: vec![v], copy }
                }
                None => none(),
            },
            "remove" => {
                let Value::Int(i, _) = vals[0] else { unreachable!("type checked") };
                let k = bounds(i, items.len(), arg_span)?;
                items.remove(k)
            }
            "reverse" => {
                items.reverse();
                Value::Unit
            }
            "join" => Value::Str(Rc::new(crate::interp::join_strs(items, &vals[0]))),
            "drain" => Value::Array(std::mem::take(items), true),
            "insert" => {
                let Value::Int(i, _) = vals[0] else { unreachable!("type checked") };
                if i < 0 || i > items.len() as i128 {
                    let len = items.len();
                    let v = vals.remove(1);
                    self.it.release(v, span, "入れられずに捨てられた");
                    return Err(Diagnostic::new("insert の位置が範囲の外です")
                        .primary(arg_span, format!("位置は {} ですが、長さは {} です", i, len))
                        .note("入れられる位置は 0〜長さ です"));
                }
                items.insert(i as usize, vals.remove(1));
                Value::Unit
            }
            _ => unreachable!("array method {}", name),
        })
    }

    /// `sort_by(f)`（interp の `sort_list`）。比べる関数には要素の読むだけの写しを渡し、並べ終わった順に元の要素を動かす
    fn sort_by(&mut self, slot: usize, path: &[usize], f: Value, stable: bool, span: Span) -> R<Value> {
        let Value::Array(items, _) = at(self.it, slot, path) else { unreachable!("sort_by on an array") };
        let items = items.clone();
        let n = items.len();
        let what = "要素";
        let mut temps = Vec::new();
        let views: Vec<Value> = items
            .into_iter()
            .map(|v| {
                let v = self.it.view_of(v, span, what);
                if !v.moves() {
                    return v;
                }
                let s = self.it.heap.alloc(v);
                temps.push(s);
                Value::Ref(RefVal {
                    slot: s,
                    generation: self.it.heap.slots[s].generation,
                    path: Vec::new(),
                    borrowed_at: span,
                    what: what.to_string(),
                    mutable: false,
                    view: false,
                })
            })
            .collect();
        let mut less = |me: &mut Self, a: usize, b: usize| -> R<bool> {
            match me.call_value(&f, vec![views[a].clone(), views[b].clone()], span)? {
                Value::Bool(x) => Ok(x),
                other => Err(Diagnostic::new("sort_by の関数は true / false を返します")
                    .primary(span, format!("{} が返りました", other.type_name()))),
            }
        };
        let order = crate::sort::sort(self, n, stable, &mut less);
        for s in temps {
            let w = self.it.vacate(s, span, "sort_by の比べ終わった写し");
            self.it.release(w, span, "sort_by の比べ終わった写し");
        }
        self.it.release(f, span, "sort_by が使い終わった関数");
        let order = order?;
        let Value::Array(items, _) = at_mut(self.it, slot, path) else {
            return Err(Diagnostic::new("sort_by の途中でリストが置き換えられました").primary(span, "ここ"));
        };
        if items.len() != n {
            return Err(Diagnostic::new("sort_by の途中でリストの長さが変わりました")
                .primary(span, format!("始めは {} 個でしたが、今は {} 個です", n, items.len())));
        }
        let mut old: Vec<Option<Value>> = std::mem::take(items).into_iter().map(Some).collect();
        *items = order.into_iter().map(|i| old[i].take().expect("a permutation")).collect();
        Ok(Value::Unit)
    }
}

/// スロットの中の道筋の先にある値。interp の `at` と違い、enum の中身もたどる（match で受け取るとき）
fn at<'v>(it: &'v Interp, slot: usize, path: &[usize]) -> &'v Value {
    let mut v = it.heap.slots[slot].value.as_ref().expect("live slot");
    for &i in path {
        v = match v {
            Value::Struct(_, fields) | Value::Array(fields, _) | Value::Enum { fields, .. } => &fields[i],
            Value::Map(m) => &m.entries[i].1,
            _ => unreachable!("paths go through structs, arrays, maps and enums"),
        };
    }
    v
}

fn at_mut<'v>(it: &'v mut Interp, slot: usize, path: &[usize]) -> &'v mut Value {
    let mut v = it.heap.slots[slot].value.as_mut().expect("live slot");
    for &i in path {
        v = match v {
            Value::Struct(_, fields) | Value::Array(fields, _) | Value::Enum { fields, .. } => &mut fields[i],
            Value::Map(m) => &mut m.entries[i].1,
            _ => unreachable!("paths go through structs, arrays, maps and enums"),
        };
    }
    v
}

/// 値として取り出すと own を動かすことになる型（types.rs の `holds_own` と同じ）
fn holds_own(t: &Ty) -> bool {
    match t {
        Ty::Own(_) => true,
        Ty::Array(x, _) => holds_own(x),
        _ => false,
    }
}
