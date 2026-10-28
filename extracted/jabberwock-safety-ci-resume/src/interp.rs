//! 核になるインタプリタ。
//!
//! 安全性の仕組み（この言語の看板）:
//!   * `own`  : 唯一の所有者。代入・引数渡しで「ムーブ」され、元の変数は使えなくなる。
//!   * `ref`  : 世代付き参照。(スロット番号, 世代) を持つ。所有者が解放されるとスロットの
//!     世代が進むので、古い参照を使った瞬間に「確実に」検出できる。
//!     検出時は「借りた → 解放された → 使った」を時系列で報告する。
//!
//! 静的解析（`analyze`）が「絶対安全」と証明できた `*p` は世代チェックを飛ばす（速さ）。
//! 証明できない箇所だけ実行時検査が残る（安全）。

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::BufRead;
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use crate::ast::*;
use crate::diag::{Diagnostic, Span};
use crate::threads::{aborted_marker, Chan, Entry, Portable, Task, World};

type R<T> = Result<T, Diagnostic>;

/// 新しいスレッドで動かすもの（新しい `Interp` を受け取る）
pub(crate) type ThreadBody = Box<dyn FnOnce(&mut Interp) -> R<Value> + Send>;

pub(crate) const MAX_DEPTH: usize = 5000;
pub(crate) const BUILTINS: [&str; 13] = [
    "print", "eprint", "drop", "assert", "read_line", "args", "spawn", "chan", "exit", "read_file", "write_file", "run_command", "chr",
];

#[derive(Debug, Clone)]
pub enum Value {
    Unit,
    /// 整数。値は i128 で持ち、型 `IntTy` の範囲に入れておく
    Int(i128, IntTy),
    /// 小数（f64）
    Float(f64),
    Bool(bool),
    Str(Rc<String>),
    /// 唯一の所有者（ヒープのスロット番号）
    Own(usize),
    /// 世代付き参照
    Ref(RefVal),
    /// 構造体。フィールドは定義の順。中に own を持ちうるので、値として読むとムーブになる。
    Struct(Rc<StructDef>, Vec<Value>),
    /// enum の値（Option も）。`copy` は型検査が決めた「値として読むとコピーか」（false ならムーブ）
    Enum { variant: Rc<str>, fields: Vec<Value>, copy: bool },
    /// 配列（bool が false）。要素にムーブする値があればムーブ（なければコピー）。
    /// bool が true のものはリストの中身で、リストの区画（`List` のスロット）にだけ置く
    Array(Vec<Value>, bool),
    /// リスト。中身（`Array(_, true)`）は own の先と同じく自分のスロットに置く（いつもムーブ）。
    /// 要素を借りた ref はそのスロットを指すので、リストをムーブしても有効で、長さが変わると世代が進んで無効になる
    List(usize),
    /// 関数の値。クロージャは作ったときに取り込んだ外の変数の値を持つ（`Lambda.captures` の順）
    Func(Callee, Vec<Value>),
    /// 辞書。入れた順に並ぶ（表示と for の順）。いつもムーブ
    Map(Box<MapVal>),
    /// `spawn` したスレッド（ムーブ）
    Task(Arc<Task>),
    /// チャンネル（コピーしても同じものを指す）
    Chan(Arc<Chan>),
}

/// 辞書の中身。`entries` が入れた順の (キー, 値)、`index` がキーから位置を引く表
#[derive(Debug, Clone, Default)]
pub struct MapVal {
    pub(crate) entries: Vec<(Value, Value)>,
    index: HashMap<Key, usize>,
}

/// 文字列の値を作る（`Rc<String>` なのは、持ち主が 1 人のときにその場で後ろに足すため。`s += x`）
pub(crate) fn text_rc(s: &str) -> Rc<String> {
    Rc::new(s.to_string())
}

/// 辞書のキー（整数・文字列・真偽値）
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Key {
    Int(i128),
    Str(Rc<String>),
    Bool(bool),
}

fn key_of(v: &Value) -> Key {
    match v {
        Value::Int(n, _) => Key::Int(*n),
        Value::Str(s) => Key::Str(s.clone()),
        Value::Bool(b) => Key::Bool(*b),
        _ => unreachable!("type checked: map keys are ints, strings or bools"),
    }
}

impl MapVal {
    pub(crate) fn find(&self, k: &Value) -> Option<usize> {
        self.index.get(&key_of(k)).copied()
    }

    /// 入れる。同じキーがあれば値を置き換えて、古い値を返す
    pub(crate) fn insert(&mut self, k: Value, v: Value) -> Option<Value> {
        match self.find(&k) {
            Some(i) => Some(std::mem::replace(&mut self.entries[i].1, v)),
            None => {
                self.index.insert(key_of(&k), self.entries.len());
                self.entries.push((k, v));
                None
            }
        }
    }

    /// 取り出す。後ろの組は前に詰める（入れた順を保つ）
    pub(crate) fn remove(&mut self, k: &Value) -> Option<Value> {
        let i = self.index.remove(&key_of(k))?;
        let (_, v) = self.entries.remove(i);
        for (j, (k, _)) in self.entries.iter().enumerate().skip(i) {
            self.index.insert(key_of(k), j);
        }
        Some(v)
    }
}

/// 関数の値の中身
#[derive(Debug, Clone)]
pub enum Callee {
    /// 名前のある関数（`v.sort_by(less)`）
    Named(Rc<FnDef>),
    /// その場で作った関数 `(a, b) -> ...`
    Lambda(Rc<Lambda>),
    /// IR の関数（番号）。IR インタプリタ（`ir::exec`）だけが作る
    Ir(usize),
}

#[derive(Debug, Clone)]
pub struct RefVal {
    pub(crate) slot: usize,
    pub(crate) generation: u64,
    /// スロットの中のどこを指すか（構造体のフィールドと配列の添字の道筋）。own の先を指すなら空
    pub(crate) path: Vec<usize>,
    pub(crate) borrowed_at: Span,
    pub(crate) what: String,
    /// `ref mut` で借りたか（書き換えられるか）
    pub(crate) mutable: bool,
    /// own の読むだけの写し（`view_of`）か。型が own の場所にあるので、値として取り出せない（型の引数 `T` を通っても）
    pub(crate) view: bool,
}

/// 取り出せない値の名前。own の型の場所にある読むだけの写し（for の要素の中など）は ref の値だが、own として言う
fn taken_name(v: &Value, e: &Expr) -> &'static str {
    if (e.takes_own.get() || is_view(v)) && matches!(v, Value::Ref(_)) {
        "own"
    } else {
        v.type_name()
    }
}

/// own の読むだけの写しを含むか。型の引数 `T` の場所では型検査が `takes_own` を立てられないので、値で見る
fn is_view(v: &Value) -> bool {
    match v {
        Value::Ref(r) => r.view,
        Value::Struct(_, fields) | Value::Enum { fields, .. } | Value::Array(fields, _) | Value::Func(_, fields) => {
            fields.iter().any(is_view)
        }
        _ => false,
    }
}

/// ref の型の名前で受け取る値から、写しの印を消す（中身の場所は同じ。取り出せないのは own の型の場所だけ）
fn unview(v: Value) -> Value {
    match v {
        Value::Ref(r) => Value::Ref(RefVal { view: false, ..r }),
        other => other,
    }
}

impl Value {
    pub(crate) fn type_name(&self) -> &'static str {
        match self {
            Value::Unit => "()",
            Value::Int(..) => "整数",
            Value::Float(_) => "小数",
            Value::Bool(_) => "真偽値",
            Value::Str(_) => "文字列",
            Value::Own(_) => "own",
            Value::Ref(_) => "ref",
            Value::Struct(..) => "構造体",
            Value::Enum { .. } => "enum",
            Value::Array(_, false) => "配列",
            Value::Array(_, true) | Value::List(_) => "リスト",
            Value::Func(..) => "関数",
            Value::Map(_) => "辞書",
            Value::Task(_) => "task",
            Value::Chan(_) => "chan",
        }
    }

    /// 値として読むとムーブになるか（own・構造体・コピーできない enum）
    pub(crate) fn moves(&self) -> bool {
        match self {
            Value::Array(items, list) => *list || items.iter().any(Value::moves),
            // 取り込んだ値にムーブするものがあればムーブ（なければコピー）
            Value::Func(_, caps) => caps.iter().any(Value::moves),
            Value::Map(_) | Value::Task(_) | Value::List(_) => true,
            _ => matches!(self, Value::Own(_) | Value::Struct(..) | Value::Enum { copy: false, .. }),
        }
    }

    /// 中にある task をすべて集める（捨てるときに終わるのを待つ）
    fn collect_tasks(&self, out: &mut Vec<Arc<Task>>) {
        match self {
            Value::Task(t) => out.push(t.clone()),
            Value::Struct(_, fields) | Value::Enum { fields, .. } | Value::Array(fields, _) | Value::Func(_, fields) => {
                fields.iter().for_each(|f| f.collect_tasks(out))
            }
            Value::Map(m) => m.entries.iter().for_each(|(_, v)| v.collect_tasks(out)),
            _ => {}
        }
    }

    /// 中にある own のスロット（とリストの中身のスロット）をすべて集める
    fn collect_owns(&self, out: &mut Vec<usize>) {
        match self {
            Value::Own(s) | Value::List(s) => out.push(*s),
            Value::Struct(_, fields) | Value::Enum { fields, .. } | Value::Array(fields, _) | Value::Func(_, fields) => {
                fields.iter().for_each(|f| f.collect_owns(out))
            }
            Value::Map(m) => m.entries.iter().for_each(|(_, v)| v.collect_owns(out)),
            _ => {}
        }
    }
}

// ---------------------------------------------------------------- ヒープ

pub(crate) struct Slot {
    pub(crate) generation: u64,
    pub(crate) value: Option<Value>,
    /// 世代の上限で退役した識別子。再利用せず、世代が一致するrefも拒否する。
    retired: bool,
}

struct Freed {
    span: Span,
    why: String,
}

pub(crate) struct Heap {
    pub(crate) slots: Vec<Slot>,
    free_list: Vec<usize>,
    /// (スロット, その世代) がいつ・なぜ解放されたか。古い参照のエラー報告に使う。
    history: HashMap<(usize, u64), Freed>,
    #[cfg(test)]
    generation_limit: u64,
}

impl Heap {
    fn new() -> Self {
        Heap {
            slots: Vec::new(), free_list: Vec::new(), history: HashMap::new(),
            #[cfg(test)]
            generation_limit: u64::MAX,
        }
    }

    pub(crate) fn alloc(&mut self, v: Value) -> usize {
        if let Some(i) = self.free_list.pop() {
            debug_assert!(!self.slots[i].retired && self.slots[i].value.is_none());
            self.slots[i].value = Some(v);
            i
        } else {
            self.slots.push(Slot { generation: 0, value: Some(v), retired: false });
            self.slots.len() - 1
        }
    }

    fn live(&self) -> usize {
        self.slots.iter().filter(|s| s.value.is_some()).count()
    }

    /// 周回する前に退役する。退役したスロットのmetadataはHeapの終了まで保持する。
    fn advance_generation(&mut self, slot: usize) -> bool {
        #[cfg(test)]
        let limit = self.generation_limit;
        #[cfg(not(test))]
        let limit = u64::MAX;
        let cell = &mut self.slots[slot];
        if !cell.retired && cell.generation < limit {
            cell.generation += 1;
            true
        } else {
            cell.retired = true;
            false
        }
    }
}

// ---------------------------------------------------------------- 環境

#[derive(Clone)]
enum VarState {
    /// 値はヒープのスロットに置く（`ref x` で変数そのものを借りられるように）
    Live(usize),
    /// 所有権が移動済み。どこで移動したかを覚えておく。
    Moved(Span),
}

struct Var {
    name: String,
    state: VarState,
    /// `mut` がついているか（書き換えられるか）
    mutable: bool,
    /// 宣言した位置（書き換えられないときのエラー用）
    decl: Span,
    /// クロージャが取り込んだムーブする値を、呼び出しのあいだ読むだけの写しで持っている（値として読めない）
    view: bool,
}

struct Frame {
    scopes: Vec<Vec<Var>>,
}

/// 書き込み・読み出しができる場所（ヒープのスロット。変数もスロットに置く）と、その中のフィールドの道筋
struct Place {
    slot: usize,
    path: Vec<usize>,
    access: Access,
}

/// 場所に書き込めるか。書き込めないときはその理由（エラー表示用）
#[derive(Clone)]
enum Access {
    Write,
    /// `mut` のない変数（とその中身）
    ImmutVar { name: String, decl: Span },
    /// `mut` のない ref の先
    ReadOnlyRef { what: String, borrowed_at: Span },
}

impl Access {
    /// ポインタの先の場所に書き込めるか。own の先は持ち主と同じ、ref の先は ref 次第。
    fn through(self, p: &Value) -> Access {
        match p {
            Value::Ref(r) if r.mutable => Access::Write,
            Value::Ref(r) => Access::ReadOnlyRef { what: r.what.clone(), borrowed_at: r.borrowed_at },
            _ => self,
        }
    }

    /// 書き込めなければ、`doing`（「書き換えようとした」など）を添えたエラーにする
    fn require(&self, span: Span, doing: &str) -> R<()> {
        match self {
            Access::Write => Ok(()),
            Access::ImmutVar { name, decl } => Err(Diagnostic::new(format!(
                "`{}` は mut ではないので書き換えられません",
                name
            ))
            .label(*decl, format!("ここで `{}` を宣言した（mut がない）", name))
            .primary(span, format!("ここで{}", doing))
            .note(format!("書き換えるなら `let mut {}` や `fn f(mut {})` のように mut をつけてください", name, name))),
            Access::ReadOnlyRef { what, borrowed_at } => Err(Diagnostic::new("読むだけの ref を通して書き換えようとしました")
                .label(*borrowed_at, format!("ここで `{}` を ref で借りた（mut がない）", what))
                .primary(span, format!("ここで{}", doing))
                .note("書き換えるなら `ref mut` で借りてください")),
        }
    }
}

/// メソッド呼び出しのために `this` として貸したスロット。呼び出しのあと解放する。
/// `back` は値を戻す場所と、その場所を持つスロットの（貸したときの）世代
struct Lent {
    slot: usize,
    back: Option<(Place, Option<(usize, u64)>)>,
    /// 一時的な own の先（スロットごと解放する）。そうでなければ、貸すために置いたスロット（中の値を捨てる。変数を捨てるのと同じ理由の付け方）
    own: bool,
}

enum Flow {
    Normal,
    Return(Value, Span),
    Break(Span),
    Continue(Span),
}

pub struct Interp {
    pub(crate) heap: Heap,
    frames: Vec<Frame>,
    fns: HashMap<String, Rc<FnDef>>,
    structs: HashMap<String, Rc<StructDef>>,
    /// スレッドのあいだで共有するもの（出力・待ち合わせ・エラー）
    pub(crate) world: Arc<World>,
    depth: usize,
    /// `e?` で関数を抜けている途中なら、返す値（`Err(x)` / `None`）と `?` の場所。
    /// そのあいだ R は Err を返して、呼び出し中の関数（`call_fn`）まで戻る
    unwinding: Option<(Value, Span)>,
    /// unwinding が、値として使う if / match の中の `return` から始まったか（`?` でなく）
    returning: bool,
    /// 実行時に行った ref の世代チェックの回数
    pub ref_checks: usize,
    /// 静的解析のおかげで飛ばせた ref の世代チェックの回数
    pub ref_checks_elided: usize,
    /// `read_line()` が読む入力（スレッドで共有）
    input: Arc<Mutex<Box<dyn BufRead + Send>>>,
    /// `args()` が返すコマンドライン引数
    args: Vec<String>,
    /// `ref T` の引数に渡した一時的な値のスロット。呼び出しが終わったら解放する（`with_lent`）
    lent: Vec<usize>,
}

impl Interp {
    pub fn new(echo: bool) -> Self {
        Interp {
            heap: Heap::new(),
            frames: Vec::new(),
            fns: HashMap::new(),
            structs: HashMap::new(),
            world: World::new(echo, Box::new(|| unreachable!("spawn needs World set by run_inner"))),
            depth: 0,
            unwinding: None,
            returning: false,
            ref_checks: 0,
            ref_checks_elided: 0,
            input: Arc::new(Mutex::new(Box::new(std::io::empty()))),
            args: Vec::new(),
            lent: Vec::new(),
        }
    }

    /// print が出した内容（すべてのスレッドの分）
    pub fn output(&self) -> String {
        self.world.lock().output.clone()
    }

    pub fn set_world(&mut self, w: Arc<World>) {
        self.world = w;
    }

    /// `read_line()` が読む入力と、`args()` が返す引数を決める
    /// `read_line()`: 1 行読む（終わりの改行は取る）。入力が終わっていれば None
    pub(crate) fn read_line(&mut self, span: Span) -> R<Value> {
        let mut line = String::new();
        let n = self
            .input
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .read_line(&mut line)
            .map_err(|e| Diagnostic::new("入力を読めませんでした").primary(span, e.to_string()))?;
        if n == 0 {
            return Ok(none());
        }
        if line.ends_with('\n') {
            line.pop();
            if line.ends_with('\r') {
                line.pop();
            }
        }
        Ok(some(Value::Str(text_rc(line.as_str()))))
    }

    /// ファイル・コマンド・終了（`exit` / `read_file` / `write_file` / `run_command`）。IR の exec も使う。
    /// エラーの文は jw_rt.c と同じにする（OS のエラーの文は使わない）
    pub(crate) fn system(&mut self, name: &str, vals: Vec<Value>, span: Span) -> R<Value> {
        let text = |v: &Value| match v {
            Value::Str(s) => s.to_string(),
            _ => unreachable!("type checked: str"),
        };
        let result = |ok: bool, v: Value| Value::Enum { variant: Rc::from(if ok { "Ok" } else { "Err" }), fields: vec![v], copy: true };
        let msg = |m: String| Value::Str(text_rc(m.as_str()));
        match name {
            "exit" => {
                let Value::Int(n, _) = vals[0] else { unreachable!("type checked: i64") };
                Err(self.world.exit(n as i32))
            }
            "read_file" => {
                let path = text(&vals[0]);
                Ok(match std::fs::read(&path) {
                    Ok(b) => match String::from_utf8(b) {
                        Ok(t) => result(true, Value::Str(text_rc(t.as_str()))),
                        Err(_) => result(false, msg(format!("ファイルが UTF-8 ではありません: {}", path))),
                    },
                    Err(_) => result(false, msg(format!("ファイルを読めません: {}", path))),
                })
            }
            "write_file" => {
                let path = text(&vals[0]);
                Ok(match std::fs::write(&path, text(&vals[1])) {
                    Ok(()) => result(true, Value::Unit),
                    Err(_) => result(false, msg(format!("ファイルに書けません: {}", path))),
                })
            }
            "run_command" => {
                let cmd = text(&vals[0]);
                let Value::Array(items, _) = self.seq_at(&vals[1]).clone() else { unreachable!("type checked: str[]") };
                let mut vals = vals;
                let list = vals.remove(1);
                self.release(list, span, "使い終わった一時的な値");
                use std::io::Write;
                let _ = std::io::stdout().flush();
                let status = std::process::Command::new(&cmd).args(items.iter().map(text)).status();
                Ok(match status {
                    Ok(st) => result(true, Value::Int(st.code().unwrap_or(-1) as i128, IntTy::I64)),
                    Err(_) => result(false, msg(format!("コマンドを起動できません: {}", cmd))),
                })
            }
            _ => unreachable!("not a system builtin: {} at {:?}", name, span),
        }
    }

    /// `exit(n)` で終わったときの終了コード
    pub fn exit_code(&self) -> Option<i32> {
        self.world.lock().exit
    }

    /// eprint が出した内容
    pub fn errput(&self) -> String {
        self.world.lock().errput.clone()
    }

    /// `args()`: コマンドライン引数のリスト
    pub(crate) fn args_value(&mut self) -> Value {
        let items = self.args.iter().map(|a| Value::Str(text_rc(a.as_str()))).collect();
        self.new_list(items)
    }

    pub fn set_io(&mut self, input: Box<dyn BufRead + Send>, args: Vec<String>) {
        self.input = Arc::new(Mutex::new(input));
        self.args = args;
    }

    /// 解放されていないヒープオブジェクトの数（リーク検査用。終わったスレッドの分も足す）
    pub fn live_objects(&self) -> usize {
        self.heap.live() + self.world.lock().leaked
    }

    pub fn run(&mut self, prog: Program) -> R<()> {
        self.load(prog)?;
        let main = self.fns.get("main").cloned().ok_or_else(|| {
            Diagnostic::new("`main` 関数がありません").note("`fn main() { ... }` を定義してください")
        })?;
        let r = self.call_fn(&main, Vec::new(), main.span);
        if r.is_err() {
            // 実行時エラーで止まったときも、残っている変数を片づける（リーク検査を正しくするため）
            while let Some(frame) = self.frames.pop() {
                for scope in frame.scopes.into_iter().rev() {
                    self.drop_scope(scope, main.span, "エラーで止まった");
                }
            }
        }
        self.finish_main(r.map(|_| ()), main.span)
    }

    /// 構造体と関数を登録する
    fn load(&mut self, prog: Program) -> R<()> {
        for s in prog.structs {
            if self.structs.contains_key(&s.name) {
                return Err(Diagnostic::new(format!("構造体 `{}` が二重に定義されています", s.name))
                    .primary(s.span, "ここで再定義されている"));
            }
            self.structs.insert(s.name.clone(), Rc::new(s));
        }
        for f in prog.fns {
            if BUILTINS.contains(&f.name.as_str()) {
                return Err(Diagnostic::new(format!("`{}` は組み込み関数の名前です", f.name))
                    .primary(f.span, "別の名前にしてください"));
            }
            if self.fns.contains_key(&f.name) {
                return Err(Diagnostic::new(format!("関数 `{}` が二重に定義されています", f.name))
                    .primary(f.span, "ここで再定義されている"));
            }
            self.fns.insert(f.name.clone(), Rc::new(f));
        }
        Ok(())
    }

    // ------------------------------------------------------------ スレッド

    /// 値をヒープから切り離す（ほかのスレッドへ送る）。中の own のスロットは解放する
    /// （送った側に残った ref は、使うとダングリングとして見つかる）
    pub(crate) fn export(&mut self, v: Value, span: Span) -> Portable {
        match v {
            Value::Unit => Portable::Unit,
            Value::Int(n, t) => Portable::Int(n, t),
            Value::Float(x) => Portable::Float(x),
            Value::Bool(b) => Portable::Bool(b),
            Value::Str(s) => Portable::Str(s.to_string()),
            Value::Own(slot) => {
                let inner = self.vacate(slot, span, "ほかのスレッドへ送られた");
                Portable::Own(Box::new(self.export(inner, span)))
            }
            Value::Struct(def, fields) => {
                Portable::Struct(def.name.clone(), fields.into_iter().map(|f| self.export(f, span)).collect())
            }
            Value::Enum { variant, fields, copy } => Portable::Enum {
                variant: variant.to_string(),
                fields: fields.into_iter().map(|f| self.export(f, span)).collect(),
                copy,
            },
            Value::Array(items, list) => Portable::Array(items.into_iter().map(|x| self.export(x, span)).collect(), list),
            Value::List(slot) => {
                let Value::Array(items, _) = self.vacate(slot, span, "ほかのスレッドへ送られた") else { unreachable!("list storage") };
                Portable::Array(items.into_iter().map(|x| self.export(x, span)).collect(), true)
            }
            Value::Map(m) => {
                Portable::Map(m.entries.into_iter().map(|(k, v)| (self.export(k, span), self.export(v, span))).collect())
            }
            Value::Task(t) => Portable::Task(t),
            Value::Chan(c) => Portable::Chan(c),
            Value::Ref(_) | Value::Func(..) => unreachable!("type checked: not sendable"),
        }
    }

    /// ほかのスレッドから来た値を、このヒープに置く
    pub(crate) fn import(&mut self, p: Portable) -> Value {
        match p {
            Portable::Unit => Value::Unit,
            Portable::Int(n, t) => Value::Int(n, t),
            Portable::Float(x) => Value::Float(x),
            Portable::Bool(b) => Value::Bool(b),
            Portable::Str(s) => Value::Str(text_rc(s.as_str())),
            Portable::Own(x) => {
                let v = self.import(*x);
                Value::Own(self.heap.alloc(v))
            }
            Portable::Struct(name, fields) => {
                let def = self.structs[&name].clone();
                Value::Struct(def, fields.into_iter().map(|f| self.import(f)).collect())
            }
            Portable::Enum { variant, fields, copy } => Value::Enum {
                variant: Rc::from(variant.as_str()),
                fields: fields.into_iter().map(|f| self.import(f)).collect(),
                copy,
            },
            Portable::Array(items, list) => {
                let items = items.into_iter().map(|x| self.import(x)).collect();
                if list {
                    self.new_list(items)
                } else {
                    Value::Array(items, false)
                }
            }
            Portable::Map(entries) => {
                let mut m = MapVal::default();
                for (k, v) in entries {
                    let (k, v) = (self.import(k), self.import(v));
                    m.insert(k, v);
                }
                Value::Map(Box::new(m))
            }
            Portable::Task(t) => Value::Task(t),
            Portable::Chan(c) => Value::Chan(c),
        }
    }

    /// `spawn(f)`: f を新しいスレッドで動かす。取り込んだ値はそのスレッドへ移す
    fn spawn(&mut self, arg: &Expr, span: Span) -> R<Value> {
        let f = self.eval(arg)?;
        let Value::Func(callee, caps) = f else { unreachable!("type checked: spawn takes a function") };
        let entry = match &callee {
            Callee::Named(d) => Entry::Named(d.name.clone()),
            Callee::Lambda(_) => Entry::Lambda(arg.span),
            Callee::Ir(_) => unreachable!("IR functions run only in ir::exec"),
        };
        let caps: Vec<Portable> = caps.into_iter().map(|c| self.export(c, span)).collect();
        self.start_thread(span, Box::new(move |it: &mut Interp| {
            let prog = (it.world.load)();
            it.run_entry(prog, entry, caps, span)
        }))
    }

    /// 新しいスレッドを始めて、その task を返す。`run` は新しい `Interp`（同じ world・入力）で動く。
    /// 作れなければエラー（生きているスレッドの数には入れない。錠を持ったまま作るので、新しいスレッドが先に終わって数を減らすことはない）
    pub(crate) fn start_thread(&mut self, span: Span, run: ThreadBody) -> R<Value> {
        let task = Arc::new(Task::default());
        let world = self.world.clone();
        let (w2, t2, input, args) = (world.clone(), task.clone(), self.input.clone(), self.args.clone());
        let mut st = world.lock();
        let made = if !world.may_start(&st) {
            None
        } else {
            std::thread::Builder::new()
                .stack_size(1 << 30)
                .spawn(move || {
                    crate::threads::mark_worker();
                    let world = w2;
                    let mut it = Interp::new(false);
                    it.world = world.clone();
                    it.input = input;
                    it.args = args;
                    match run(&mut it) {
                        Ok(v) => {
                            let p = it.export(v, span);
                            let leaked = it.heap.live();
                            world.finished(&t2, Ok(p), leaked);
                        }
                        Err(d) => {
                            world.fail(d);
                            let leaked = it.heap.live();
                            world.finished(&t2, Err(()), leaked);
                        }
                    }
                })
                .ok()
        };
        if made.is_none() {
            // 送るはずだった値は run と一緒に捨てる（own のスロットは送ったときにもう空けてある）
            return Err(Diagnostic::new("スレッドを作れませんでした")
                .primary(span, "ここで spawn した")
                .note("OS のスレッドの数か、メモリが足りません"));
        }
        world.started_locked(&mut st);
        Ok(Value::Task(task))
    }

    /// main が終わったあと: ほかのスレッドを待ち、どれかのエラーがあればそれを返す
    pub(crate) fn finish_main(&mut self, r: R<()>, span: Span) -> R<()> {
        let r = match r {
            Ok(_) => self.world.wait_all(span),
            Err(d) => {
                self.world.fail(d.clone());
                Err(d)
            }
        };
        if r.is_err() {
            self.world.wait_stopped();
        }
        let abort = self.world.lock().abort.clone();
        match abort {
            Some(d) => Err(d),
            None => r,
        }
    }

    pub(crate) fn set_structs(&mut self, structs: HashMap<String, Rc<StructDef>>) {
        self.structs = structs;
    }

    /// 新しいスレッドの中: プログラムを登録して、spawn に渡された関数を 1 回だけ呼ぶ
    /// （1 回だけなので、取り込んだ own は写しにせず、そのまま変数に置く。中から取り出してよい）
    fn run_entry(&mut self, prog: Program, entry: Entry, caps: Vec<Portable>, span: Span) -> R<Value> {
        self.load(prog)?;
        let r = match entry {
            Entry::Named(n) => {
                let f = self.fns[&n].clone();
                self.call_fn(&f, Vec::new(), span)
            }
            Entry::Lambda(at) => {
                let mut found = None;
                for f in self.fns.values() {
                    walk_block(&f.body, &mut |e| {
                        if let ExprKind::Lambda(l) = &e.kind {
                            if e.span == at {
                                found = Some(l.clone());
                            }
                        }
                        found.is_none()
                    });
                }
                let l = found.expect("the same program has the same lambda");
                let names = l.captures.borrow().clone();
                let mut vars = Vec::new();
                for (n, c) in names.iter().zip(caps) {
                    let v = self.import(c);
                    let slot = self.heap.alloc(v);
                    vars.push(Var { name: n.clone(), state: VarState::Live(slot), mutable: false, decl: span, view: false });
                }
                self.run_frame(vars, &l.body, l.returns.get())
            }
        };
        if r.is_err() {
            while let Some(frame) = self.frames.pop() {
                for scope in frame.scopes.into_iter().rev() {
                    self.drop_scope(scope, span, "エラーで止まった");
                }
            }
        }
        r
    }

    /// task と chan のメソッド
    pub(crate) fn thread_method(&mut self, h: &Value, name: &str, mut vals: Vec<Value>, span: Span) -> R<Value> {
        match (h, name) {
            (Value::Task(t), "join") => {
                let p = self.world.join(t, span)?;
                Ok(self.import(p))
            }
            (Value::Chan(c), "send") => {
                let v = vals.remove(0);
                let p = self.export(v, span);
                self.world.send(c, p, span)?;
                Ok(Value::Unit)
            }
            (Value::Chan(c), "recv") => Ok(match self.world.recv(c, span)? {
                Some(p) => {
                    let v = self.import(p);
                    let copy = !v.moves();
                    Value::Enum { variant: Rc::from("Some"), fields: vec![v], copy }
                }
                None => none(),
            }),
            (Value::Chan(c), "close") => {
                self.world.close(c, span)?;
                Ok(Value::Unit)
            }
            _ => unreachable!("type checked: task / chan method"),
        }
    }

    /// 捨てる値の中の task が終わるのを待つ（戻り値は捨てる）
    fn settle_tasks(&mut self, v: &Value, span: Span) {
        let mut tasks = Vec::new();
        v.collect_tasks(&mut tasks);
        for t in tasks {
            // エラーで止まったなら、world に残っている（あとで main が返す）
            if let Ok(Some(p)) = self.world.forget(&t, span) {
                let v = self.import(p);
                self.release(v, span, "join されずに捨てられた task の戻り値");
            }
        }
    }

    // ------------------------------------------------------------ 解放

    /// スロットを解放し、世代を進める。中身が own なら連鎖して解放する。
    pub(crate) fn free(&mut self, slot: usize, span: Span, why: &str) {
        let mut work: Vec<(usize, String)> = vec![(slot, why.to_string())];
        while let Some((s, why)) = work.pop() {
            let Some(inner) = self.heap.slots[s].value.take() else {
                continue;
            };
            let g = self.heap.slots[s].generation;
            let reusable = self.heap.advance_generation(s);
            self.heap.history.insert((s, g), Freed { span, why: why.clone() });
            if reusable { self.heap.free_list.push(s); }
            self.settle_tasks(&inner, span);
            let mut children = Vec::new();
            inner.collect_owns(&mut children);
            // リストの中身は、リストと同じ理由で解放される（リストの要素は持ち主の中にあるのと同じ）
            let list = matches!(inner, Value::Array(_, true));
            for child in children {
                let why = if list { why.clone() } else { "持ち主が解放されたので、中身も連鎖して解放された".to_string() };
                work.push((child, why));
            }
        }
    }

    /// エラーで止まるときに、渡しそこねた値を解放する
    fn discard_all(&mut self, vals: Vec<Value>, span: Span) {
        for v in vals {
            self.release(v, span, "エラーで止まったので捨てられた");
        }
    }

    /// 評価の途中の値（引数など）を、`?` で抜けるときに解放する（ふつうのエラーなら何もしない）
    fn discard(&mut self, vals: Vec<Value>, span: Span) {
        if self.unwinding.is_some() {
            let why = if self.returning { "return で関数を抜けたので、使われずに捨てられた" } else { "`?` で関数を抜けたので、使われずに捨てられた" };
            for v in vals {
                self.release(v, span, why);
            }
        }
    }

    /// 式を順に評価する。途中で `?` により抜けるなら、それまでの値を解放する
    fn eval_all<'a>(&mut self, es: impl IntoIterator<Item = &'a Expr>) -> R<Vec<Value>> {
        let mut vals = Vec::new();
        for a in es {
            match self.eval(a) {
                Ok(v) => vals.push(v),
                Err(d) => {
                    self.discard(vals, a.span);
                    return Err(d);
                }
            }
        }
        Ok(vals)
    }

    /// 値を捨てる。中にある own は（構造体のフィールドも含めて）すべて解放する。
    pub(crate) fn release(&mut self, v: Value, span: Span, why: &str) {
        self.settle_tasks(&v, span);
        let mut owns = Vec::new();
        v.collect_owns(&mut owns);
        for s in owns {
            self.free(s, span, why);
        }
    }

    fn drop_scope(&mut self, scope: Vec<Var>, span: Span, how: &str) {
        for var in scope.into_iter().rev() {
            if let VarState::Live(slot) = var.state {
                let why = format!("`{}` は{}ので解放された", var.name, how);
                let v = self.vacate(slot, span, &why);
                self.release(v, span, &why);
            }
        }
    }

    /// スロットの中身を取り出して、スロットだけ解放する（中の own は解放しない）。
    /// 世代が進むので、このスロットを指していた ref は無効になる
    pub(crate) fn vacate(&mut self, slot: usize, span: Span, why: &str) -> Value {
        let v = self.heap.slots[slot].value.take().expect("live slot");
        let g = self.heap.slots[slot].generation;
        let reusable = self.heap.advance_generation(slot);
        self.heap.history.insert((slot, g), Freed { span, why: why.to_string() });
        if reusable { self.heap.free_list.push(slot); }
        v
    }

    /// 値の中に（フィールドやリストの中も含めて）own を持っているか
    pub(crate) fn has_own(&self, v: &Value) -> bool {
        match v {
            Value::Own(_) => true,
            Value::List(s) => matches!(&self.heap.slots[*s].value, Some(x) if self.has_own(x)),
            Value::Struct(_, fields) | Value::Enum { fields, .. } | Value::Array(fields, _) | Value::Func(_, fields) => {
                fields.iter().any(|f| self.has_own(f))
            }
            Value::Map(m) => m.entries.iter().any(|(_, x)| self.has_own(x)),
            _ => false,
        }
    }

    /// 新しいリスト（中身を置くスロットを作る）
    pub(crate) fn new_list(&mut self, items: Vec<Value>) -> Value {
        Value::List(self.heap.alloc(Value::Array(items, true)))
    }

    /// リストなら中身（`Array(_, true)`）、ほかはそのまま
    pub(crate) fn seq_at<'v>(&'v self, v: &'v Value) -> &'v Value {
        match v {
            Value::List(s) => self.heap.slots[*s].value.as_ref().expect("live list"),
            v => v,
        }
    }

    /// 値の中のリストを、新しいスロットへ写したものに置き換える（一時的な値の中から読むとき）。写したものがあれば true
    pub(crate) fn copy_lists(&mut self, v: Value) -> (Value, bool) {
        match v {
            Value::List(s) => {
                let Some(Value::Array(items, _)) = self.heap.slots[s].value.clone() else { unreachable!("live list") };
                let items = items.into_iter().map(|x| self.copy_lists(x).0).collect();
                (self.new_list(items), true)
            }
            Value::Struct(def, fields) => {
                let mut any = false;
                let fields = fields.into_iter().map(|f| {
                    let (f, c) = self.copy_lists(f);
                    any |= c;
                    f
                }).collect();
                (Value::Struct(def, fields), any)
            }
            Value::Array(items, list) => {
                let mut any = false;
                let items = items.into_iter().map(|f| {
                    let (f, c) = self.copy_lists(f);
                    any |= c;
                    f
                }).collect();
                (Value::Array(items, list), any)
            }
            Value::Enum { variant, fields, copy } => {
                let mut any = false;
                let fields = fields.into_iter().map(|f| {
                    let (f, c) = self.copy_lists(f);
                    any |= c;
                    f
                }).collect();
                (Value::Enum { variant, fields, copy }, any)
            }
            Value::Map(mut m) => {
                let mut any = false;
                for e in m.entries.iter_mut() {
                    let x = std::mem::replace(&mut e.1, Value::Unit);
                    let (x, c) = self.copy_lists(x);
                    any |= c;
                    e.1 = x;
                }
                (Value::Map(m), any)
            }
            other => (other, false),
        }
    }

    /// ヒープを使わずに作ったリスト（`Array(_, true)`。文字列のメソッドの結果）を、中身のスロットに置いたリストにする
    pub(crate) fn adopt(&mut self, v: Value) -> Value {
        match v {
            Value::Array(items, true) => self.new_list(items),
            v => v,
        }
    }

    /// 道筋の先がリストなら、その中身のスロットへ移る（要素の場所はそこから数える）
    pub(crate) fn list_place(&self, slot: usize, path: Vec<usize>) -> (usize, Vec<usize>) {
        match self.at(slot, &path) {
            Value::List(s) => (*s, Vec::new()),
            _ => (slot, path),
        }
    }

    /// リストの要素が動いた（長さが変わった・並べ替えた）。中身のスロットの世代を進め、要素を借りた ref を無効にする
    pub(crate) fn list_moved(&mut self, slot: usize, span: Span, name: &str) -> R<()> {
        let g = self.heap.slots[slot].generation;
        let advanced = self.heap.advance_generation(slot);
        self.heap.history.insert((slot, g), Freed { span, why: list_moved_why(name) });
        if !advanced {
            return Err(Diagnostic::new("世代番号の上限に達しました")
                .primary(span, "これ以上この区画の参照を安全に無効化できません"));
        }
        Ok(())
    }

    /// print と埋め込みで出す文字。own と ref は先の中身を出す（ref が無効なら実行時エラー）
    pub(crate) fn show(&mut self, v: &Value, span: Span, depth: usize) -> R<String> {
        // ref は輪になりうるので、深くなりすぎたら打ち切る
        if depth > 32 {
            return Ok("...".to_string());
        }
        Ok(match v {
            Value::Unit => "()".to_string(),
            Value::Int(n, _) => n.to_string(),
            // 整数と見分けがつくよう、3.0 は `3.0` と出す
            Value::Float(x) => format!("{:?}", x),
            Value::Bool(b) => b.to_string(),
            Value::Str(s) => s.to_string(),
            Value::Own(_) | Value::Ref(_) => {
                let (slot, path) = self.check_ptr(v, span, Check::Required)?;
                let inner = self.at(slot, &path).clone();
                self.show(&inner, span, depth + 1)?
            }
            Value::Struct(def, fields) => {
                let mut parts = Vec::new();
                for (n, f) in def.fields.iter().zip(fields) {
                    parts.push(format!("{}: {}", n, self.show(f, span, depth + 1)?));
                }
                format!("{} {{ {} }}", def.name, parts.join(", "))
            }
            Value::Array(items, _) => {
                let mut parts = Vec::new();
                for x in items {
                    parts.push(self.show(x, span, depth + 1)?);
                }
                format!("[{}]", parts.join(", "))
            }
            Value::List(slot) => {
                let inner = self.heap.slots[*slot].value.clone().expect("live list");
                self.show(&inner, span, depth)?
            }
            Value::Enum { variant, fields, .. } if fields.is_empty() => variant.to_string(),
            Value::Enum { variant, fields, .. } => {
                let mut parts = Vec::new();
                for x in fields {
                    parts.push(self.show(x, span, depth + 1)?);
                }
                format!("{}({})", variant, parts.join(", "))
            }
            Value::Func(..) => "<関数>".to_string(),
            Value::Task(_) => "<task>".to_string(),
            Value::Chan(_) => "<chan>".to_string(),
            Value::Map(m) => {
                let mut parts = Vec::new();
                for (k, v) in &m.entries {
                    parts.push(format!("{}: {}", self.show(k, span, depth + 1)?, self.show(v, span, depth + 1)?));
                }
                format!("[{}]", parts.join(", "))
            }
        })
    }

    /// 値の写し。中の own はすべて、それを借りた読むだけの ref にする（own は唯一の持ち主なので写せない）
    /// リストは中身の写しを新しいスロットに置く（写しを捨てるときは `release` する）
    pub(crate) fn view_of(&mut self, v: Value, span: Span, what: &str) -> Value {
        match v {
            Value::Own(slot) => Value::Ref(RefVal {
                slot,
                generation: self.heap.slots[slot].generation,
                path: Vec::new(),
                borrowed_at: span,
                what: what.to_string(),
                mutable: false,
                view: true,
            }),
            Value::Struct(def, fields) => {
                Value::Struct(def, fields.into_iter().map(|f| self.view_of(f, span, what)).collect())
            }
            Value::Array(items, list) => {
                Value::Array(items.into_iter().map(|f| self.view_of(f, span, what)).collect(), list)
            }
            Value::List(slot) => {
                let Some(Value::Array(items, _)) = self.heap.slots[slot].value.clone() else { unreachable!("live list") };
                let items = items.into_iter().map(|f| self.view_of(f, span, what)).collect();
                self.new_list(items)
            }
            Value::Enum { variant, fields, copy } => Value::Enum {
                variant,
                fields: fields.into_iter().map(|f| self.view_of(f, span, what)).collect(),
                copy,
            },
            Value::Func(c, caps) => Value::Func(c, caps.into_iter().map(|f| self.view_of(f, span, what)).collect()),
            Value::Map(mut m) => {
                for e in m.entries.iter_mut() {
                    let v = std::mem::replace(&mut e.1, Value::Unit);
                    e.1 = self.view_of(v, span, what);
                }
                Value::Map(m)
            }
            other => other,
        }
    }

    /// スロットの中の道筋の先にある値
    pub(crate) fn at(&self, slot: usize, path: &[usize]) -> &Value {
        let mut v = self.heap.slots[slot].value.as_ref().expect("live slot");
        for &i in path {
            v = match v {
                Value::Struct(_, fields) | Value::Array(fields, _) => &fields[i],
                Value::Map(m) => &m.entries[i].1,
                _ => unreachable!("paths go through structs, arrays and maps"),
            };
        }
        v
    }

    /// 道筋が辞書の中を通るか（辞書の値の場所は借りられない。リストの要素は中身のスロットから数えるので通らない）
    pub(crate) fn through_map(&self, slot: usize, path: &[usize]) -> bool {
        let mut v = self.heap.slots[slot].value.as_ref().expect("live slot");
        for &i in path {
            v = match v {
                Value::Map(_) => return true,
                Value::Struct(_, fields) | Value::Array(fields, _) => &fields[i],
                _ => unreachable!("paths go through structs and arrays"),
            };
        }
        false
    }

    pub(crate) fn at_mut(&mut self, slot: usize, path: &[usize]) -> &mut Value {
        let mut v = self.heap.slots[slot].value.as_mut().expect("live slot");
        for &i in path {
            v = match v {
                Value::Struct(_, fields) | Value::Array(fields, _) => &mut fields[i],
                Value::Map(m) => &mut m.entries[i].1,
                _ => unreachable!("paths go through structs, arrays and maps"),
            };
        }
        v
    }

    // ------------------------------------------------------------ 変数

    fn find_var(&mut self, name: &str) -> Option<&mut Var> {
        let frame = self.frames.last_mut()?;
        for scope in frame.scopes.iter_mut().rev() {
            for v in scope.iter_mut().rev() {
                if v.name == name {
                    return Some(v);
                }
            }
        }
        None
    }

    /// 変数を値として読む。ムーブする値ならムーブ（元の変数は使えなくなる）
    fn read_var(&mut self, name: &str, span: Span) -> R<Value> {
        let Some(var) = self.find_var(name) else {
            return self.fn_value(name, span);
        };
        let view = var.view;
        match var.state.clone() {
            VarState::Live(_) if view => Err(Diagnostic::new(format!("クロージャに取り込んだ `{}` は取り出せません", name))
                .primary(span, "クロージャは何度も呼べるので、取り込んだ値をムーブできない")
                .note("`ref` で借りるか、中の値をコピーして使ってください")),
            // own を読む = ムーブ。元の変数は使えなくなる（変数を借りた ref も無効になる）
            VarState::Live(slot) if self.heap.slots[slot].value.as_ref().expect("live slot").moves() => {
                self.find_var(name).expect("var").state = VarState::Moved(span);
                Ok(self.vacate(slot, span, &format!("`{}` がムーブされた", name)))
            }
            VarState::Live(slot) => Ok(self.heap.slots[slot].value.clone().expect("live slot")),
            VarState::Moved(sp) => Err(moved_error(name, sp, span)),
        }
    }

    /// 名前のある関数を値として使う
    fn fn_value(&self, name: &str, span: Span) -> R<Value> {
        match self.fns.get(name) {
            Some(f) => Ok(Value::Func(Callee::Named(f.clone()), Vec::new())),
            None => Err(undefined_var(name, span)),
        }
    }

    fn declare(&mut self, name: &str, v: Value, mutable: bool, decl: Span) {
        let slot = self.heap.alloc(v);
        self.frames
            .last_mut()
            .expect("frame")
            .scopes
            .last_mut()
            .expect("scope")
            .push(Var { name: name.to_string(), state: VarState::Live(slot), mutable, decl, view: false });
    }

    // ------------------------------------------------------------ ポインタ検査

    /// ポインタの先のスロットを返す。参照が古ければ時系列付きのエラーにする。
    /// `check` が `Elided` なら、静的解析が安全を証明済みなので世代チェックを飛ばす。
    pub(crate) fn check_ptr(&mut self, p: &Value, use_span: Span, check: Check) -> R<(usize, Vec<usize>)> {
        match p {
            Value::Own(s) => Ok((*s, Vec::new())),
            Value::Ref(r) => {
                let slot = &self.heap.slots[r.slot];
                let valid = !slot.retired && slot.generation == r.generation && slot.value.is_some();
                if check == Check::Elided {
                    // 解析の誤りを見逃さないよう、デバッグビルドでだけ念のため確かめる
                    debug_assert!(valid, "static analysis elided the check of a dangling ref");
                    self.ref_checks_elided += 1;
                    return Ok((r.slot, r.path.clone()));
                }
                self.ref_checks += 1;
                if valid {
                    return Ok((r.slot, r.path.clone()));
                }
                let mut d = Diagnostic::new("無効になった参照を使いました (dangling reference)")
                    .label(r.borrowed_at, format!("ここで `{}` を借りた", r.what));
                if let Some(f) = self.heap.history.get(&(r.slot, r.generation)) {
                    d = d.label(f.span, format!("ここで解放された: {}", f.why));
                }
                Err(d
                    .primary(use_span, "ここで使ったが、借りた先はもう存在しない")
                    .note("参照は、借りた元の所有者より長くは生きられません"))
            }
            other => Err(Diagnostic::new("ポインタではない値に `*` を使いました")
                .primary(use_span, format!("{} は own でも ref でもありません", other.type_name()))),
        }
    }

    // ------------------------------------------------------------ 文

    fn exec_block(&mut self, block: &Block) -> R<Flow> {
        Ok(self.exec_block_value(block, false)?.0)
    }

    /// ブロックを実行する。`want_value` なら最後の行の式の値を返す（if 式の値）。
    fn exec_block_value(&mut self, block: &Block, want_value: bool) -> R<(Flow, Value)> {
        self.frames.last_mut().expect("frame").scopes.push(Vec::new());
        let mut result = Flow::Normal;
        let mut value = Value::Unit;
        for (i, s) in block.stmts.iter().enumerate() {
            let flow = match s {
                Stmt::Expr(e) if want_value && block.tail && i + 1 == block.stmts.len() => {
                    let (flow, v) = self.eval_flow(e, true)?;
                    value = v;
                    flow
                }
                _ => self.exec_stmt(s)?,
            };
            if !matches!(flow, Flow::Normal) {
                result = flow;
                break;
            }
        }
        self.leave_scope(&result, block.end);
        Ok((result, value))
    }

    /// 一番内側のスコープを抜け、その変数を解放する。`end` は普通に抜けたときの位置
    fn leave_scope(&mut self, result: &Flow, end: Span) {
        let scope = self.frames.last_mut().expect("frame").scopes.pop().expect("scope");
        let (span, how) = match result {
            Flow::Return(_, sp) => (*sp, "関数から return した"),
            Flow::Break(sp) => (*sp, "break でループを抜けた"),
            Flow::Continue(sp) => (*sp, "continue で次の周回へ進んだ"),
            Flow::Normal => (end, "スコープを抜けた"),
        };
        self.drop_scope(scope, span, how);
    }

    /// 式を評価する。文として使う if は中の return / break / continue を外へ伝える。
    fn eval_flow(&mut self, e: &Expr, want_value: bool) -> R<(Flow, Value)> {
        let (flow, v) = self.eval_flow_kind(e, want_value)?;
        Ok((flow, convert(e, v)))
    }

    fn eval_flow_kind(&mut self, e: &Expr, want_value: bool) -> R<(Flow, Value)> {
        match &e.kind {
            ExprKind::If { cond, then, els } => {
                let c = self.eval(cond)?;
                let blk = if as_bool(c, cond.span)? { Some(then) } else { els.as_ref() };
                match blk {
                    Some(blk) => self.exec_block_value(blk, want_value),
                    None => Ok((Flow::Normal, Value::Unit)),
                }
            }
            ExprKind::Match { scrut, arms, default } => {
                let v = self.eval(scrut)?;
                if let Some(r) = self.match_enum(v.clone(), scrut, arms, default.as_ref(), want_value) {
                    return r;
                }
                match select_arm(&v, scrut.span, arms)?.or(default.as_ref()) {
                    Some(blk) => self.exec_block_value(blk, want_value),
                    None => Ok((Flow::Normal, Value::Unit)),
                }
            }
            _ => Ok((Flow::Normal, self.eval(e)?)),
        }
    }

    /// enum の match。enum でなければ None（ふつうの match として続ける）。
    /// 値の match なら中身は受け取る名前へ移る。借りた match（ref）なら、中身の own は ref として受け取る
    fn match_enum(
        &mut self,
        v: Value,
        scrut: &Expr,
        arms: &[MatchArm],
        default: Option<&Block>,
        want_value: bool,
    ) -> Option<R<(Flow, Value)>> {
        let (subject, by_ref) = match v {
            Value::Ref(r) => {
                let (slot, path) = match self.check_ptr(&Value::Ref(r.clone()), scrut.span, Check::Required) {
                    Ok(s) => s,
                    Err(d) => return Some(Err(d)),
                };
                let inner = self.at(slot, &path).clone();
                if !matches!(inner, Value::Enum { .. }) {
                    return None;
                }
                // 中身の説明は match に書いた名前から（`e の中身 x`。lower / 実行ファイルと同じ）
                let what = match &scrut.kind {
                    ExprKind::Ref(x, _) => describe(x),
                    _ => describe(scrut),
                };
                (inner, Some(RefVal { what, ..r }))
            }
            Value::Own(slot) => {
                let inner = self.heap.slots[slot].value.clone().expect("live slot");
                if !matches!(inner, Value::Enum { .. }) {
                    return None;
                }
                // 中身を取り出して、入れ物だけ解放する
                self.heap.slots[slot].value = Some(Value::Unit);
                self.free(slot, scrut.span, "match で中身を取り出したので、入れ物が解放された");
                (inner, None)
            }
            v @ Value::Enum { .. } => (v, None),
            _ => return None,
        };
        let Value::Enum { variant, fields, .. } = subject else { unreachable!() };
        let hit = arms.iter().find_map(|a| {
            a.pats.iter().find_map(|(p, _)| match p {
                Pat::Variant(n, binds, subs) if **n == *variant && self.subs_match(&fields, subs) => Some((a, binds, subs)),
                _ => None,
            })
        });
        let Some((arm, binds, subs)) = hit else {
            // default（型検査が、全部の種類を書いたか default があることを確かめている）
            if by_ref.is_none() {
                self.release(Value::Enum { variant, fields, copy: false }, scrut.span, "match の default で使われずに捨てられた");
            }
            return Some(match default {
                Some(blk) => self.exec_block_value(blk, want_value),
                None => Ok((Flow::Normal, Value::Unit)),
            });
        };
        self.frames.last_mut().expect("frame").scopes.push(Vec::new());
        self.bind_fields(fields, binds, subs, &by_ref);
        let r = self.exec_block_value(&arm.body, want_value);
        let flow = match &r {
            Ok((f, _)) => f,
            Err(_) => &Flow::Normal,
        };
        let flow = match flow {
            Flow::Return(_, sp) => Flow::Return(Value::Unit, *sp),
            Flow::Break(sp) => Flow::Break(*sp),
            Flow::Continue(sp) => Flow::Continue(*sp),
            Flow::Normal => Flow::Normal,
        };
        self.leave_scope(&flow, arm.body.end);
        Some(r)
    }

    /// 重ねた形（`Some(Op(op))` の `Op(op)`）がすべて当たるか。値は動かさずに見るだけ
    fn subs_match(&self, fields: &[Value], subs: &[Option<Pat>]) -> bool {
        fields.iter().zip(subs).all(|(f, s)| match s {
            Some(p) => self.pat_matches(f, p),
            None => true,
        })
    }

    fn pat_matches(&self, v: &Value, p: &Pat) -> bool {
        match (v, p) {
            (Value::Own(slot), _) => match &self.heap.slots[*slot].value {
                Some(inner) => self.pat_matches(inner, p),
                None => false,
            },
            (Value::Enum { variant, fields, .. }, Pat::Variant(n, _, subs)) => **n == **variant && self.subs_match(fields, subs),
            (Value::Int(a, _), Pat::Int(b)) => a == b,
            (Value::Bool(a), Pat::Bool(b)) => a == b,
            (Value::Str(a), Pat::Str(b)) => **a == **b,
            _ => false,
        }
    }

    /// match で当たった種類の中身を、受け取る名前へ置く（重ねた形の中も）。
    /// 値の match は中身を移し、借りた match（`by_ref`）は中身の own を ref にして渡す
    fn bind_fields(&mut self, fields: Vec<Value>, binds: &[(String, Span)], subs: &[Option<Pat>], by_ref: &Option<RefVal>) {
        for ((f, (b, bspan)), sub) in fields.into_iter().zip(binds).zip(subs) {
            if let Some(Pat::Variant(_, ibinds, isubs)) = sub {
                let inner = match f {
                    // 借りた match では own の先も見る（型検査が、値の match では own の先に重ねないことを確かめている）
                    Value::Own(s) => self.heap.slots[s].value.clone().expect("live slot"),
                    other => other,
                };
                let Value::Enum { fields, .. } = inner else { unreachable!("type checked") };
                self.bind_fields(fields, ibinds, isubs, by_ref);
                continue;
            }
            let val = match by_ref {
                None => f,
                Some(r) => match f {
                    // 中身の own は、借りた ref として受け取る
                    Value::Own(s) => Value::Ref(RefVal {
                        slot: s,
                        generation: self.heap.slots[s].generation,
                        path: Vec::new(),
                        borrowed_at: *bspan,
                        what: format!("{} の中身 {}", r.what, b),
                        mutable: r.mutable,
                        view: false,
                    }),
                    // 写しの中の own（印のある ref）も、ref として受け取るので印を消す
                    other => unview(other),
                },
            };
            if b == "_" {
                if by_ref.is_none() {
                    self.release(val, *bspan, "match で受け取られずに捨てられた");
                }
                continue;
            }
            self.declare(b, val, false, *bspan);
        }
    }

    /// ループ本体を1周実行し、ループを続けるかどうかを返す
    fn loop_iteration(&mut self, body: &Block) -> R<Option<Flow>> {
        if self.world.aborted.load(Ordering::Relaxed) {
            return Err(aborted_marker());
        }
        match self.exec_block(body)? {
            Flow::Normal | Flow::Continue(_) => Ok(None),
            Flow::Break(_) => Ok(Some(Flow::Normal)),
            r @ Flow::Return(..) => Ok(Some(r)),
        }
    }

    /// `for x in a` / `for (k, v) in m`。深い再帰で exec_stmt の枠が大きくならないよう、別の関数にしている
    #[inline(never)]
    fn exec_for_each(
        &mut self,
        key: Option<&(String, Span)>,
        var: &str,
        var_span: Span,
        iter: &Expr,
        body: &Block,
        els: Option<&Block>,
    ) -> R<Flow> {
        let (src, temp) = self.eval_borrow(iter)?;
        let target = match &src {
            Value::Own(_) | Value::Ref(_) => {
                let (slot, path) = self.check_ptr(&src, iter.span, Check::Required)?;
                self.seq_at(self.at(slot, &path))
            }
            v => self.seq_at(v),
        };
        // (前の名前の値, 要素)。リストなら添字、辞書ならキー
        let (keys, items): (Vec<Value>, Vec<Value>) = match target {
            Value::Array(items, _) => {
                (0..items.len()).map(|i| Value::Int(i as i128, IntTy::I64)).zip(items.iter().cloned()).unzip()
            }
            Value::Map(m) => m.entries.iter().cloned().unzip(),
            other => return Err(not_an_array(other, iter.span)),
        };
        let mut keys = keys.into_iter();
        // ループを始めた時点の要素を回す。own の要素は借りた ref にする
        // （ループの中で置き換えられたり取り出されたりすれば、その ref は無効になる）
        let what = format!("{} の要素", describe(iter));
        // 構造体などムーブする要素は、始めた時点の写しにする（IR と同じく、ループの前にまとめて作る）。
        // 写しの中の own は、元の要素の own を借りた ref にする（元が解放されれば、たどったときに検出される）
        let views: Vec<Value> = items
            .into_iter()
            .map(|v| match v {
                Value::Own(slot) => Value::Ref(RefVal {
                    slot,
                    generation: self.heap.slots[slot].generation,
                    path: Vec::new(),
                    borrowed_at: var_span,
                    what: what.clone(),
                    mutable: false,
                    view: false,
                }),
                other if other.moves() => self.view_of(other, var_span, &what),
                // 写しのリストの own の要素（印のある ref）は、ref の x として受け取るので印を消す
                other => unview(other),
            })
            .collect();
        let ran = !views.is_empty();
        let mut out = Ok(Flow::Normal);
        let mut views = views.into_iter();
        for v in views.by_ref() {
            // ムーブする要素の写しは一時的なスロットに置き、読むだけの ref で渡す
            let (v, copy) = if v.moves() {
                let slot = self.heap.alloc(v);
                let r = RefVal {
                    slot,
                    generation: self.heap.slots[slot].generation,
                    path: Vec::new(),
                    borrowed_at: var_span,
                    what: what.clone(),
                    mutable: false,
                    view: false,
                };
                (Value::Ref(r), Some(slot))
            } else {
                (v, None)
            };
            self.frames.last_mut().expect("frame").scopes.push(Vec::new());
            let k = keys.next().expect("one key per item");
            if let Some((kn, ks)) = key {
                self.declare(kn, k, false, *ks);
            }
            self.declare(var, v, false, var_span);
            let r = self.loop_iteration(body);
            let scope = self.frames.last_mut().expect("frame").scopes.pop().expect("scope");
            self.drop_scope(scope, var_span, "次の周回へ進んだ");
            if let Some(slot) = copy {
                // 写しは捨てる（中の own は ref なので解放しない。写したリストの中身だけ解放される）
                let v = self.vacate(slot, var_span, "for の次の周回へ進んだ");
                self.release(v, var_span, "for の次の周回へ進んだ");
            }
            match r {
                Ok(None) => {}
                Ok(Some(f)) => {
                    out = Ok(f);
                    break;
                }
                Err(d) => {
                    out = Err(d);
                    break;
                }
            }
        }
        // 途中で抜けたら、残りの写しを捨てる
        for v in views {
            self.release(v, var_span, "for の次の周回へ進んだ");
        }
        if temp {
            match &out {
                Err(_) => self.discard(vec![src], iter.span),
                Ok(_) => self.release(src, iter.span, "for で回し終わった一時的な値"),
            }
        }
        match (out?, els) {
            (Flow::Normal, Some(els)) if !ran => self.exec_block(els),
            (f, _) => Ok(f),
        }
    }

    /// その場で作ったリストを回す（`for x in a.drain()`）。要素を前から取り出して x へ移す。
    /// IR と同じく、はじめに逆順にして後ろから pop する（途中で抜けたら、残りは逆順のまま捨てる）
    #[inline(never)]
    fn exec_for_take(
        &mut self,
        key: Option<&(String, Span)>,
        var: &str,
        var_span: Span,
        iter: &Expr,
        body: &Block,
        els: Option<&Block>,
    ) -> R<Flow> {
        let Value::List(list) = self.eval(iter)? else { unreachable!("type checked: a list") };
        let Value::Array(mut items, _) = self.vacate(list, iter.span, "for で回し終わった一時的な値") else {
            unreachable!("list storage")
        };
        items.reverse();
        let mut out = Ok(Flow::Normal);
        let mut i = 0;
        while let Some(v) = items.pop() {
            self.frames.last_mut().expect("frame").scopes.push(Vec::new());
            if let Some((kn, ks)) = key {
                self.declare(kn, Value::Int(i, IntTy::I64), false, *ks);
            }
            self.declare(var, v, false, var_span);
            let r = self.loop_iteration(body);
            let scope = self.frames.last_mut().expect("frame").scopes.pop().expect("scope");
            self.drop_scope(scope, var_span, "次の周回へ進んだ");
            i += 1;
            match r {
                Ok(None) => {}
                Ok(Some(f)) => {
                    out = Ok(f);
                    break;
                }
                Err(d) => {
                    out = Err(d);
                    break;
                }
            }
        }
        let rest = Value::Array(items, true);
        match &out {
            Err(_) => self.discard(vec![rest], iter.span),
            Ok(_) => self.release(rest, iter.span, "for で回し終わった一時的な値"),
        }
        match (out?, els) {
            (Flow::Normal, Some(els)) if i == 0 => self.exec_block(els),
            (f, _) => Ok(f),
        }
    }

    fn exec_stmt(&mut self, s: &Stmt) -> R<Flow> {
        match s {
            Stmt::Let { name, mutable, span, init, .. } => {
                let v = self.eval(init)?;
                self.declare(name, v, *mutable, *span);
                Ok(Flow::Normal)
            }
            Stmt::Assign { target, value, span } => {
                self.assign(target, value, *span)?;
                Ok(Flow::Normal)
            }
            Stmt::Expr(e) => {
                let (flow, v) = self.eval_flow(e, false)?;
                self.release(v, e.span, "どこにも束縛されず、そのまま捨てられた");
                Ok(flow)
            }
            Stmt::While { cond, body } => loop {
                let c = self.eval(cond)?;
                if !as_bool(c, cond.span)? {
                    return Ok(Flow::Normal);
                }
                if let Some(f) = self.loop_iteration(body)? {
                    return Ok(f);
                }
            },
            Stmt::Loop { body } => loop {
                if let Some(f) = self.loop_iteration(body)? {
                    return Ok(f);
                }
            },
            Stmt::For { var, range, range_span, body, els } => {
                let mut nums = Vec::new();
                // ループ変数の型は始めと終わりの型（刻みはいつも i64。型検査がそろえている）
                let mut ty = IntTy::I64;
                for (k, a) in range.iter().enumerate() {
                    match self.eval(a)? {
                        Value::Int(n, t) => {
                            if k < 2 {
                                ty = t;
                            }
                            nums.push(n)
                        }
                        other => {
                            return Err(Diagnostic::new("range の引数は整数です")
                                .primary(a.span, format!("ここは {} です", other.type_name())))
                        }
                    }
                }
                let (start, end, step) = match nums[..] {
                    [e] => (0, e, 1),
                    [s, e] => (s, e, 1),
                    [s, e, st] => (s, e, st),
                    _ => unreachable!("parser checks the range arity"),
                };
                if step == 0 {
                    return Err(Diagnostic::new("range の刻みが 0 です")
                        .primary(*range_span, "これだと永遠に終わりません"));
                }
                let mut i = start;
                let mut ran = false;
                while (step > 0 && i < end) || (step < 0 && i > end) {
                    ran = true;
                    // ループ変数は周回ごとの新しいスコープに置く
                    self.frames.last_mut().expect("frame").scopes.push(Vec::new());
                    self.declare(var, Value::Int(i, ty), false, *range_span);
                    let r = self.loop_iteration(body);
                    let scope = self.frames.last_mut().expect("frame").scopes.pop().expect("scope");
                    self.drop_scope(scope, *range_span, "次の周回へ進んだ");
                    if let Some(f) = r? {
                        return Ok(f);
                    }
                    i += step;
                    if !ty.contains(i) {
                        break;
                    }
                }
                match els {
                    Some(els) if !ran => self.exec_block(els),
                    _ => Ok(Flow::Normal),
                }
            }
            Stmt::ForEach { key, var, var_span, iter, body, els, consume } if consume.get() => {
                self.exec_for_take(key.as_ref(), var, *var_span, iter, body, els.as_ref())
            }
            Stmt::ForEach { key, var, var_span, iter, body, els, .. } => {
                self.exec_for_each(key.as_ref(), var, *var_span, iter, body, els.as_ref())
            }
            Stmt::Break(sp) => Ok(Flow::Break(*sp)),
            Stmt::Continue(sp) => Ok(Flow::Continue(*sp)),
            Stmt::Return { value, span } => {
                let v = match value {
                    Some(e) => self.eval(e)?,
                    None => Value::Unit,
                };
                Ok(Flow::Return(v, *span))
            }
        }
    }

    fn assign(&mut self, target: &Expr, value: &Expr, span: Span) -> R<()> {
        // `s = s + e`（`s += e`）で s が文字列の変数なら、持ち主が s だけのときその場で後ろに足す（IR の `append` と同じ）。
        // e は s を書き換えられないので、e を先に計算しても結果は同じ
        if let (ExprKind::Var(name), ExprKind::Binary(BinOp::Add, l, r)) = (&target.kind, &value.kind) {
            let slot = match self.find_var(name) {
                Some(Var { state: VarState::Live(slot), mutable: true, .. }) if matches!(&l.kind, ExprKind::Var(y) if y == name) => {
                    Some(*slot)
                }
                _ => None,
            };
            if let Some(slot) = slot.filter(|s| matches!(self.heap.slots[*s].value, Some(Value::Str(_)))) {
                let Value::Str(tail) = self.eval(r)? else { unreachable!("type checked: str + str") };
                if let Some(Value::Str(text)) = &mut self.heap.slots[slot].value {
                    Rc::make_mut(text).push_str(&tail);
                }
                return Ok(());
            }
        }
        let newv = self.eval(value)?;
        match &target.kind {
            ExprKind::Var(name) => {
                let state = {
                    let var = self
                        .find_var(name)
                        .ok_or_else(|| undefined_var(name, target.span))?;
                    if !var.mutable {
                        let access = Access::ImmutVar { name: name.clone(), decl: var.decl };
                        access.require(target.span, "書き換えようとした")?;
                    }
                    var.state.clone()
                };
                match state {
                    // 同じスロットに入れ直す（変数を借りた ref はそのまま使え、新しい値が見える）
                    VarState::Live(slot) => {
                        let old = self.heap.slots[slot].value.replace(newv).expect("live slot");
                        self.release(old, span, &format!("`{}` に再代入されて、古い値が解放された", name));
                    }
                    VarState::Moved(_) => {
                        let slot = self.heap.alloc(newv);
                        self.find_var(name).expect("var").state = VarState::Live(slot);
                    }
                }
                Ok(())
            }
            ExprKind::Deref(..) | ExprKind::Field(..) | ExprKind::Index(..) => {
                let place = match &target.kind {
                    // `m[k] = v` は、辞書になければ入れる
                    ExprKind::Index(base, idx, check) => {
                        let key = self.eval(idx)?;
                        self.index_place(target, base, &key, idx.span, check.get(), true)?
                    }
                    _ => self.place(target)?,
                };
                place.access.require(target.span, "書き換えようとした")?;
                let old = std::mem::replace(self.place_mut(&place), newv);
                self.release(old, span, &format!("`{}` が上書きされて、古い値が解放された", describe(target)));
                Ok(())
            }
            _ => unreachable!("parser only produces Var/Deref/Field assignment targets"),
        }
    }

    // ------------------------------------------------------------ 式

    /// 値を「借りて読む」評価。変数はムーブしない。
    /// 戻り値の bool は「新しく作られた一時値か」（own なら使い終わりで解放が必要）。
    fn eval_borrow(&mut self, e: &Expr) -> R<(Value, bool)> {
        if e.borrow.get() {
            return Ok((self.auto_ref(e)?, true));
        }
        match &e.kind {
            ExprKind::Var(name) => {
                let Some(var) = self.find_var(name) else {
                    return Ok((self.fn_value(name, e.span)?, true));
                };
                match var.state {
                    VarState::Live(slot) => Ok((self.heap.slots[slot].value.clone().expect("live slot"), false)),
                    VarState::Moved(sp) => Err(moved_error(name, sp, e.span)),
                }
            }
            ExprKind::Deref(inner, check) => {
                let (p, temp) = self.eval_borrow(inner)?;
                let (slot, path) = self.check_ptr(&p, e.span, check.get())?;
                let v = self.at(slot, &path).clone();
                if temp {
                    if let Value::Own(tmp) = p {
                        if self.has_own(&v) {
                            return Err(Diagnostic::new("一時的な own の中から own を取り出せません")
                                .primary(e.span, "先に変数に入れてください"));
                        }
                        // 中のリストは一時的な値と一緒に解放されるので、写してから読む
                        let (v, copied) = self.copy_lists(v);
                        self.free(tmp, e.span, "使い終わった一時的な値");
                        return Ok((v, copied));
                    }
                }
                Ok((v, false))
            }
            ExprKind::Field(base, name, check) => {
                if is_place(base) {
                    let place = self.place(e)?;
                    return Ok((self.place_mut(&place).clone(), false));
                }
                // 一時的な値のフィールド（`make().x` など）
                let tmp = self.eval(base)?;
                let (v, holder) = match &tmp {
                    Value::Struct(def, fields) => (fields[self.field_index(def, name, e.span)?].clone(), None),
                    Value::Own(_) | Value::Ref(_) => {
                        let (slot, path) = self.check_ptr(&tmp, e.span, check.get())?;
                        let v = self.field_of(slot, &path, name, e.span)?.clone();
                        (v, matches!(tmp, Value::Own(_)).then_some(slot))
                    }
                    other => return Err(not_a_struct(other, base.span)),
                };
                if self.has_own(&v) && !matches!(tmp, Value::Ref(_)) {
                    return Err(Diagnostic::new("一時的な値の中から own を取り出せません")
                        .primary(e.span, "先に変数に入れてください"));
                }
                // 中のリストは一時的な値と一緒に解放されるので、写してから読む
                let (v, copied) = if matches!(tmp, Value::Ref(_)) { (v, false) } else { self.copy_lists(v) };
                match holder {
                    Some(slot) => self.free(slot, e.span, "使い終わった一時的な値"),
                    None => self.release(tmp, e.span, "使い終わった一時的な値"),
                }
                Ok((v, copied))
            }
            ExprKind::Index(base, idx, check) => {
                if is_place(base) {
                    let place = self.place(e)?;
                    return Ok((self.place_mut(&place).clone(), false));
                }
                // 一時的な配列・辞書の要素（`make()[0]` など）。添字を先に評価する
                let key = self.eval(idx)?;
                let tmp = self.eval(base)?;
                let (v, holder) = match &tmp {
                    Value::Own(_) | Value::Ref(_) => {
                        let (slot, path) = self.check_ptr(&tmp, e.span, check.get())?;
                        let c = self.seq_at(self.at(slot, &path));
                        let v = entry(c, locate(c, &key, idx.span, base.span)?).clone();
                        (v, matches!(tmp, Value::Own(_)).then_some(slot))
                    }
                    c => {
                        let c = self.seq_at(c);
                        (entry(c, locate(c, &key, idx.span, base.span)?).clone(), None)
                    }
                };
                if self.has_own(&v) && !matches!(tmp, Value::Ref(_)) {
                    return Err(Diagnostic::new("一時的な配列の中から own を取り出せません")
                        .primary(e.span, "先に変数に入れてください"));
                }
                // 中のリストは一時的な値と一緒に解放されるので、写してから読む
                let (v, copied) = if matches!(tmp, Value::Ref(_)) { (v, false) } else { self.copy_lists(v) };
                match holder {
                    Some(slot) => self.free(slot, e.span, "使い終わった一時的な値"),
                    None => self.release(tmp, e.span, "使い終わった一時的な値"),
                }
                Ok((v, copied))
            }
            _ => Ok((self.eval(e)?, true)),
        }
    }

    // ------------------------------------------------------------ 場所

    /// 代入やフィールド読み出しの対象になる場所を求める。own / ref は自動でたどる。
    fn place(&mut self, e: &Expr) -> R<Place> {
        match &e.kind {
            ExprKind::Var(name) => {
                let var = self.find_var(name).ok_or_else(|| undefined_var(name, e.span))?;
                if let VarState::Moved(sp) = var.state {
                    return Err(moved_error(name, sp, e.span));
                }
                let access = if var.mutable {
                    Access::Write
                } else {
                    Access::ImmutVar { name: name.clone(), decl: var.decl }
                };
                let VarState::Live(slot) = var.state else { unreachable!("checked above") };
                Ok(Place { slot, path: Vec::new(), access })
            }
            ExprKind::Deref(inner, check) => {
                let (p, outer) = if is_place(inner) {
                    let ip = self.place(inner)?;
                    (self.place_mut(&ip).clone(), ip.access)
                } else {
                    let (p, temp) = self.eval_borrow(inner)?;
                    if temp && matches!(p, Value::Own(_)) {
                        // 一時的な own の中に書き込んでも、すぐ捨てられてしまう
                        return Err(Diagnostic::new("一時的な own の中は場所として使えません")
                            .primary(inner.span, "先に変数に入れてください"));
                    }
                    (p, Access::Write)
                };
                let (slot, path) = self.check_ptr(&p, e.span, check.get())?;
                Ok(Place { slot, path, access: outer.through(&p) })
            }
            ExprKind::Field(base, name, check) => {
                let mut place = self.place(base)?;
                // 構造体を丸ごと複製しないよう、ここで必要なものだけ取り出す
                let here = match self.place_mut(&place) {
                    Value::Struct(def, _) => Ok(def.clone()),
                    v => Err(v.clone()),
                };
                match here {
                    Ok(def) => {
                        place.path.push(self.field_index(&def, name, e.span)?);
                        Ok(place)
                    }
                    Err(p @ (Value::Own(_) | Value::Ref(_))) => {
                        let (slot, mut path) = self.check_ptr(&p, e.span, check.get())?;
                        let idx = match self.at(slot, &path) {
                            Value::Struct(def, _) => self.field_index(def, name, e.span)?,
                            other => return Err(not_a_struct(other, base.span)),
                        };
                        path.push(idx);
                        let access = place.access.through(&p);
                        Ok(Place { slot, path, access })
                    }
                    Err(other) => Err(not_a_struct(&other, base.span)),
                }
            }
            ExprKind::Index(base, idx, check) => {
                // 添字を先に評価する（そのあとで配列の場所を求める。analyze も同じ順）
                let key = self.eval(idx)?;
                self.index_place(e, base, &key, idx.span, check.get(), false)
            }
            _ => Err(Diagnostic::new("一時的な値の中は場所として使えません").primary(e.span, "先に変数に入れてください")),
        }
    }

    /// `base[key]` の場所。`create` なら、辞書にないキーは入れてから場所を返す（`m[k] = v` の代入）
    fn index_place(&mut self, e: &Expr, base: &Expr, key: &Value, key_span: Span, check: Check, create: bool) -> R<Place> {
        if !is_place(base) {
            return Err(Diagnostic::new("一時的な配列の中は場所として使えません")
                .primary(base.span, "先に変数に入れてください"));
        }
        let place = self.place(base)?;
        let here = match self.place_mut(&place) {
            Value::Array(..) | Value::Map(_) | Value::List(_) => None,
            v => Some(v.clone()),
        };
        let (slot, path, access) = match here {
            None => (place.slot, place.path, place.access),
            Some(p @ (Value::Own(_) | Value::Ref(_))) => {
                let (slot, path) = self.check_ptr(&p, e.span, check)?;
                (slot, path, place.access.through(&p))
            }
            Some(other) => return Err(not_an_array(&other, base.span)),
        };
        // リストの要素は、リストの中身のスロットから数える
        let (slot, mut path) = self.list_place(slot, path);
        let missing = matches!(self.at(slot, &path), Value::Map(m) if m.find(key).is_none());
        let i = if create && missing {
            access.require(e.span, "書き換えようとした")?;
            let Value::Map(m) = self.at_mut(slot, &path) else { unreachable!("checked above") };
            // 入れる値は呼び出し元が置く（それまでの仮の値）
            m.insert(key.clone(), Value::Unit);
            m.entries.len() - 1
        } else {
            locate(self.at(slot, &path), key, key_span, base.span)?
        };
        path.push(i);
        Ok(Place { slot, path, access })
    }

    fn place_mut(&mut self, place: &Place) -> &mut Value {
        self.at_mut(place.slot, &place.path)
    }

    fn field_index(&self, def: &StructDef, name: &str, span: Span) -> R<usize> {
        def.field_index(name).ok_or_else(|| {
            Diagnostic::new(format!("`{}` にフィールド `{}` はありません", def.name, name))
                .primary(span, "このフィールドは定義されていません")
                .note(format!("`{}` のフィールド: {}", def.name, def.fields.join(", ")))
        })
    }

    fn field_of(&self, slot: usize, path: &[usize], name: &str, span: Span) -> R<&Value> {
        match self.at(slot, path) {
            Value::Struct(def, fields) => Ok(&fields[self.field_index(def, name, span)?]),
            other => Err(not_a_struct(other, span)),
        }
    }

    /// `ref inner` / `ref mut inner`。own なら先を、ref なら同じ先を借りる。それ以外の値は、その場所そのものを借りる
    /// 型検査が自動で借りると決めた引数。場所なら `ref e` と同じ。一時的な値はスロットに置いて借り、
    /// 呼び出しが終わったら解放する（`with_lent`）
    #[inline(never)]
    fn auto_ref(&mut self, e: &Expr) -> R<Value> {
        if is_place(e) {
            return self.make_ref(e, false, e.span);
        }
        let v = self.eval_kind(e)?;
        let slot = match convert(e, v) {
            Value::Own(slot) => slot,
            v => self.heap.alloc(v),
        };
        self.lent.push(slot);
        Ok(Value::Ref(RefVal {
            slot,
            generation: self.heap.slots[slot].generation,
            path: Vec::new(),
            borrowed_at: e.span,
            what: "一時的な値".to_string(),
            mutable: false,
            view: false,
        }))
    }

    /// 呼び出し `f` のあいだに引数として借りた一時的な値を、終わったら（途中で抜けても）解放する
    #[inline(never)]
    fn with_lent(&mut self, span: Span, f: impl FnOnce(&mut Self) -> R<Value>) -> R<Value> {
        let mark = self.lent.len();
        let r = f(self);
        while self.lent.len() > mark {
            let slot = self.lent.pop().expect("checked above");
            self.free(slot, span, "呼び出しに使い終わった一時的な値");
        }
        r
    }

    fn make_ref(&mut self, inner: &Expr, mutable: bool, span: Span) -> R<Value> {
        if !is_place(inner) {
            return Err(Diagnostic::new("借用できない式です")
                .primary(inner.span, "`ref` は変数か `*p` か `p.f` にだけ使えます")
                .note("`let x = own ...` で変数に入れてから `ref x` としてください"));
        }
        let pl = self.place(inner)?;
        // own なら先を、ref なら同じ先を借りる。それ以外の値は、その場所そのものを借りる
        let p = match self.place_mut(&pl) {
            v @ (Value::Own(_) | Value::Ref(_)) => Some(v.clone()),
            _ => None,
        };
        if mutable {
            // own を ref mut で借りるには持ち主が mut、ref から借り直すにはその ref が ref mut
            let access = match &p {
                Some(p) => pl.access.clone().through(p),
                None => pl.access.clone(),
            };
            access.require(span, "ref mut で借りようとした")?;
        }
        match p {
            Some(Value::Own(slot)) => Ok(Value::Ref(RefVal {
                slot,
                generation: self.heap.slots[slot].generation,
                path: Vec::new(),
                borrowed_at: span,
                what: describe(inner),
                mutable,
                view: false,
            })),
            Some(Value::Ref(r)) => Ok(Value::Ref(RefVal { mutable, view: false, ..r })),
            _ => {
                if self.through_map(pl.slot, &pl.path) {
                    return Err(map_borrow_error(inner.span));
                }
                Ok(Value::Ref(RefVal {
                    slot: pl.slot,
                    generation: self.heap.slots[pl.slot].generation,
                    path: pl.path,
                    borrowed_at: span,
                    what: describe(inner),
                    mutable,
                    view: false,
                }))
            }
        }
    }

    /// 式を評価する。整数なら、型検査が決めた型（`Expr.int`）に変換して返す
    fn eval(&mut self, e: &Expr) -> R<Value> {
        if e.borrow.get() {
            // `ref` の引数に `ref` なしで渡した式は、自動で借りる（型検査が決めた）
            return self.auto_ref(e);
        }
        let v = self.eval_kind(e)?;
        Ok(convert(e, v))
    }

    fn eval_kind(&mut self, e: &Expr) -> R<Value> {
        match &e.kind {
            ExprKind::Int(n) => Ok(Value::Int(*n, IntTy::I64)),
            ExprKind::Variant { variant, args, copy, .. } => {
                let fields = self.eval_all(args)?;
                // 型の引数を含む enum（ジェネリックな関数の中）は、中身を見て決める
                let copy = copy.get().unwrap_or_else(|| !fields.iter().any(Value::moves));
                Ok(Value::Enum { variant: Rc::from(variant.as_str()), fields, copy })
            }
            ExprKind::Try(inner) => match self.eval(inner)? {
                Value::Enum { variant, mut fields, .. } if &*variant == "Ok" || &*variant == "Some" => {
                    Ok(fields.pop().expect("Ok / Some has one field"))
                }
                Value::Enum { variant, fields, .. } => {
                    // Err(x) / None をそのまま関数から返す（型検査が、戻り値の型と合うことを確かめている）。
                    // 型は関数の戻り値の型に変わる（`Result<i64[], str>` → `Result<i64, str>`）ので、コピーかは中身で決め直す
                    let copy = !fields.iter().any(Value::moves);
                    self.unwinding = Some((Value::Enum { variant, fields, copy }, e.span));
                    Err(Diagnostic::new("`?` で関数を抜けています").primary(e.span, "ここ"))
                }
                other => Err(Diagnostic::new("`?` は Result か Option に使います")
                    .primary(e.span, format!("ここは {} です", other.type_name()))),
            },
            ExprKind::Cast(inner, ty) => {
                let target = match &ty.kind {
                    TypeKind::Name(name) => IntTy::from_name(name),
                    _ => None,
                };
                match (self.eval(inner)?, target) {
                    (Value::Int(n, _), Some(t)) => Ok(Value::Int(t.wrap(n), t)),
                    (Value::Int(n, _), None) => Ok(Value::Float(n as f64)),
                    // 小数 → 整数は 0 の方へ切り捨て、型の幅で一周させる（NaN は 0）
                    (Value::Float(x), Some(t)) => Ok(Value::Int(t.wrap(float_to_int(x)), t)),
                    (Value::Float(x), None) => Ok(Value::Float(x)),
                    (other, _) => Err(Diagnostic::new("as で変換できるのは数だけです")
                        .primary(inner.span, format!("ここは {} です", other.type_name()))),
                }
            }
            ExprKind::Float(x) => Ok(Value::Float(*x)),
            ExprKind::Bool(b) => Ok(Value::Bool(*b)),
            ExprKind::Unit => Ok(Value::Unit),
            ExprKind::Str(s) => Ok(Value::Str(text_rc(s.as_str()))),
            ExprKind::Var(name) => self.read_var(name, e.span),
            ExprKind::Lambda(l) => {
                // 外の変数を、名前で読むのと同じように取り込む（own はムーブ、コピーできる値はコピー）。
                // 引数に書いたクロージャは、ムーブする値を `ref 名前` で借りて取り込む
                let names = l.captures.borrow().clone();
                let borrowed = l.borrowed.borrow().clone();
                let mut caps = Vec::new();
                for (k, n) in names.iter().enumerate() {
                    let got = if borrowed.get(k).copied().unwrap_or(false) {
                        self.make_ref(&Expr::new(ExprKind::Var(n.clone()), e.span), false, e.span)
                    } else {
                        self.read_var(n, e.span)
                    };
                    match got {
                        Ok(v) => caps.push(v),
                        Err(d) => {
                            self.discard(caps, e.span);
                            return Err(d);
                        }
                    }
                }
                Ok(Value::Func(Callee::Lambda(l.clone()), caps))
            }
            ExprKind::Own(inner) => {
                let v = self.eval(inner)?;
                Ok(Value::Own(self.heap.alloc(v)))
            }
            ExprKind::Ref(inner, mutable) => self.make_ref(inner, *mutable, e.span),
            ExprKind::Deref(..) => {
                let (v, _) = self.eval_borrow(e)?;
                if self.has_own(&v) || v.moves() || e.takes_own.get() || is_view(&v) {
                    return Err(Diagnostic::new(format!("ポインタの先にある {} は取り出せません", taken_name(&v, e)))
                        .primary(e.span, "所有権をここから動かすことはできない")
                        .note("`ref` で借りるか、`*p = ...` で置き換えてください"));
                }
                Ok(v)
            }
            ExprKind::Field(..) => {
                let (v, _) = self.eval_borrow(e)?;
                if self.has_own(&v) || v.moves() || e.takes_own.get() || is_view(&v) {
                    return Err(Diagnostic::new(format!("フィールドにある {} は取り出せません", taken_name(&v, e)))
                        .primary(e.span, "所有権をここから動かすことはできない")
                        .note("`ref` で借りるか、`p.f = ...` で置き換えてください"));
                }
                Ok(v)
            }
            ExprKind::StructLit(name, inits) => {
                let def = self.structs.get(name).cloned().ok_or_else(|| {
                    Diagnostic::new(format!("未定義の構造体 `{}`", name))
                        .primary(e.span, "この構造体は定義されていません")
                })?;
                // 書いた順に評価し、定義の順に並べる
                let mut slots: Vec<Option<Value>> = vec![None; def.fields.len()];
                for init in inits {
                    let idx = self.field_index(&def, &init.name, init.span)?;
                    if slots[idx].is_some() {
                        return Err(Diagnostic::new(format!("フィールド `{}` が二回書かれています", init.name))
                            .primary(init.span, "ここが二回目"));
                    }
                    match self.eval(&init.value) {
                        Ok(v) => slots[idx] = Some(v),
                        Err(d) => {
                            self.discard(slots.into_iter().flatten().collect(), init.span);
                            return Err(d);
                        }
                    }
                }
                let missing: Vec<&str> = def
                    .fields
                    .iter()
                    .zip(&slots)
                    .filter(|(_, v)| v.is_none())
                    .map(|(n, _)| n.as_str())
                    .collect();
                if !missing.is_empty() {
                    return Err(Diagnostic::new(format!("`{}` のフィールドが足りません", name))
                        .primary(e.span, format!("{} がありません", missing.join(", "))));
                }
                Ok(Value::Struct(def, slots.into_iter().map(|v| v.expect("checked")).collect()))
            }
            ExprKind::Unary(op, inner) => {
                let v = self.eval(inner)?;
                match (op, v) {
                    // あふれたら一周する（決まった結果になる。定数なら型検査が警告を出す）
                    (UnOp::Neg, Value::Int(n, t)) => Ok(Value::Int(t.wrap(-n), t)),
                    (UnOp::Neg, Value::Float(x)) => Ok(Value::Float(-x)),
                    (UnOp::Not, Value::Bool(b)) => Ok(Value::Bool(!b)),
                    (UnOp::BitNot, Value::Int(n, t)) => Ok(Value::Int(t.wrap(!n), t)),
                    (_, other) => Err(Diagnostic::new("型が合わない演算です")
                        .primary(e.span, format!("{} にはこの演算を使えません", other.type_name()))),
                }
            }
            ExprKind::Binary(op, l, r) => {
                if matches!(op, BinOp::And | BinOp::Or) {
                    let lv = self.eval(l)?;
                    let lb = as_bool(lv, l.span)?;
                    if (*op == BinOp::And && !lb) || (*op == BinOp::Or && lb) {
                        return Ok(Value::Bool(lb));
                    }
                    let rv = self.eval(r)?;
                    return Ok(Value::Bool(as_bool(rv, r.span)?));
                }
                let lv = self.eval(l)?;
                let rv = self.eval(r)?;
                binary(*op, lv, rv, e.span)
            }
            ExprKind::Call(name, args) => self.with_lent(e.span, |me| me.eval_call(name, args, e.span)),
            ExprKind::MethodCall(recv, name, args, owner) => {
                self.with_lent(e.span, |me| me.eval_method(recv, name, args, owner, e.span))
            }
            ExprKind::Index(..) => {
                let (v, _) = self.eval_borrow(e)?;
                if self.has_own(&v) || v.moves() || e.takes_own.get() || is_view(&v) {
                    return Err(Diagnostic::new(format!("配列の中にある {} は取り出せません", taken_name(&v, e)))
                        .primary(e.span, "所有権をここから動かすことはできない")
                        .note("`ref a[i]` で借りるか、`a[i] = ...` で置き換えてください"));
                }
                Ok(v)
            }
            ExprKind::ArrayLit(_, seq) if seq.get() == Seq::Map => Ok(Value::Map(Box::default())),
            ExprKind::ArrayLit(items, seq) => {
                let items = self.eval_all(items)?;
                Ok(if seq.get() == Seq::List { self.new_list(items) } else { Value::Array(items, false) })
            }
            ExprKind::MapLit(pairs) => {
                // キー・値の順に評価する。同じキーが二度あれば後のものが残る
                let mut m = Box::<MapVal>::default();
                for (k, v) in pairs {
                    let kv = match self.eval(k) {
                        Ok(x) => x,
                        Err(d) => {
                            self.discard(vec![Value::Map(m)], k.span);
                            return Err(d);
                        }
                    };
                    let vv = match self.eval(v) {
                        Ok(x) => x,
                        Err(d) => {
                            self.discard(vec![Value::Map(m)], v.span);
                            return Err(d);
                        }
                    };
                    if let Some(old) = m.insert(kv, vv) {
                        self.release(old, v.span, "辞書の同じキーに後から入れた値で置き換えられた");
                    }
                }
                Ok(Value::Map(m))
            }
            ExprKind::ArrayRepeat(x, n, list) => {
                let v = self.eval(x)?;
                if v.moves() {
                    return Err(Diagnostic::new("複製できない値は並べられません").primary(x.span, "ここ"));
                }
                // 型検査の上限（2^59）の中でも確保できないことがある（repeat と同じく、確保する前に確かめる）
                let mut items = Vec::new();
                if items.try_reserve_exact(*n as usize).is_err() {
                    return Err(Diagnostic::new("メモリが足りません").primary(e.span, format!("[x; {}]", n)));
                }
                items.resize(*n as usize, v);
                Ok(if list.get() { self.new_list(items) } else { Value::Array(items, false) })
            }
            ExprKind::Interp(parts) => {
                // print と同じく借りて読み、作った一時的な値は使い終わったら解放する
                let mut out = String::new();
                let mut temps = Vec::new();
                for p in parts {
                    match p {
                        InterpPart::Lit(s) => out.push_str(s),
                        InterpPart::Expr(x) => {
                            let (v, temp) = match self.eval_borrow(x) {
                                Ok(r) => r,
                                Err(d) => {
                                    self.discard(temps, x.span);
                                    return Err(d);
                                }
                            };
                            let shown = self.show(&v, x.span, 0);
                            if temp {
                                temps.push(v);
                            }
                            match shown {
                                Ok(t) => out.push_str(&t),
                                Err(d) => {
                                    for v in temps {
                                        self.release(v, x.span, "埋め込みに使えなかった一時的な値");
                                    }
                                    return Err(d);
                                }
                            }
                        }
                    }
                }
                for v in temps {
                    self.release(v, e.span, "文字列に埋め込んで使い終わった一時的な値");
                }
                Ok(Value::Str(text_rc(out.as_str())))
            }
            ExprKind::If { .. } | ExprKind::Match { .. } => match self.eval_flow(e, true)? {
                (Flow::Normal, v) => Ok(v),
                // 中の return: `?` と同じく、Err で呼び出し中の関数（`run_frame`）まで戻る
                (Flow::Return(v, sp), _) => {
                    self.unwinding = Some((v, sp));
                    self.returning = true;
                    Err(Diagnostic::new("return で関数を抜けています").primary(sp, "ここ"))
                }
                // 外への break / continue はパーサが禁止している
                _ => unreachable!("break / continue out of a value if / match"),
            },
        }
    }

    // ------------------------------------------------------------ 呼び出し

    fn eval_call(&mut self, name: &str, args: &[Expr], span: Span) -> R<Value> {
        match name {
            "print" | "eprint" => {
                let mut parts = Vec::new();
                let mut temps = Vec::new();
                for a in args {
                    let (v, temp) = match self.eval_borrow(a) {
                        Ok(x) => x,
                        Err(d) => {
                            self.discard(temps.into_iter().map(|(v, _)| v).collect(), a.span);
                            return Err(d);
                        }
                    };
                    let shown = self.show(&v, a.span, 0);
                    if temp {
                        temps.push((v, a.span));
                    }
                    match shown {
                        Ok(t) => parts.push(t),
                        Err(d) => {
                            for (v, sp) in temps {
                                self.release(v, sp, "print に使えなかった一時的な値");
                            }
                            return Err(d);
                        }
                    }
                }
                let line = parts.join(" ");
                if name == "print" {
                    self.world.print_line(&line);
                } else {
                    self.world.eprint_line(&line);
                }
                for (v, sp) in temps {
                    self.release(v, sp, "print の引数として使い終わった一時的な値");
                }
                Ok(Value::Unit)
            }
            "drop" => {
                if args.len() != 1 {
                    return Err(arity_error("drop", 1, args.len(), span));
                }
                let v = self.eval(&args[0])?;
                let why = match &args[0].kind {
                    ExprKind::Var(n) => format!("`{}` が drop で明示的に解放された", n),
                    _ => "drop で明示的に解放された".to_string(),
                };
                self.release(v, span, &why);
                Ok(Value::Unit)
            }
            "read_line" => self.read_line(span),
            "exit" | "read_file" | "write_file" | "run_command" => {
                let vals = self.eval_all(args)?;
                self.system(name, vals, span)
            }
            "chr" => {
                let v = self.eval(&args[0])?;
                let Value::Int(n, _) = v else { unreachable!("type checked: i64") };
                chr(n, span)
            }
            "spawn" => self.spawn(&args[0], span),
            "chan" => Ok(Value::Chan(Arc::new(Chan::default()))),
            "args" => Ok(self.args_value()),
            "assert" => {
                if args.len() != 1 {
                    return Err(arity_error("assert", 1, args.len(), span));
                }
                let v = self.eval(&args[0])?;
                if !as_bool(v, args[0].span)? {
                    return Err(Diagnostic::new("assert に失敗しました")
                        .primary(args[0].span, "この条件が false でした"));
                }
                Ok(Value::Unit)
            }
            _ if self.find_var(name).is_some() => self.call_var(name, args, span),
            _ => {
                let f = self.fns.get(name).cloned().ok_or_else(|| {
                    Diagnostic::new(format!("未定義の関数 `{}`", name))
                        .primary(span, "この関数は定義されていません")
                })?;
                if args.len() != f.params.len() {
                    return Err(arity_error(name, f.params.len(), args.len(), span));
                }
                let vals = self.eval_all(args)?;
                self.call_fn(&f, vals, span)
            }
        }
    }

    /// `fn take` のメソッドの呼び出し。receiver を先に値として読み（ムーブ）、ふつうの関数と同じく呼ぶ
    #[inline(never)]
    fn eval_take_method(
        &mut self,
        recv: &Expr,
        name: &str,
        args: &[Expr],
        owner: &RefCell<MethodTarget>,
        span: Span,
    ) -> R<Value> {
        let this = self.eval(recv)?;
        let sname = match (&owner.borrow().enum_name, &this) {
            (Some(n), _) => n.clone(),
            (None, Value::Struct(def, _)) => def.name.clone(),
            (None, other) => return Err(not_a_struct(other, recv.span)),
        };
        let vals = match self.eval_all(args) {
            Ok(v) => v,
            Err(d) => {
                self.discard(vec![this], recv.span);
                return Err(d);
            }
        };
        let f = self.fns.get(&format!("{}.{}", sname, name)).cloned().expect("type checked");
        let mut all = vec![this];
        all.extend(vals);
        self.call_fn(&f, all, span)
    }

    /// `recv.name(args)`。`this` には recv を借りた ref を渡す。
    /// recv が構造体の値なら、呼び出しのあいだだけヒープへ移して借り、終わったら元の場所へ戻す
    /// （戻すときにそのスロットは解放されるので、`this` を借りた ref が残っていてもダングリングとして見つかる）
    fn eval_method(
        &mut self,
        recv: &Expr,
        name: &str,
        args: &[Expr],
        owner: &RefCell<MethodTarget>,
        span: Span,
    ) -> R<Value> {
        if owner.borrow().take {
            return self.eval_take_method(recv, name, args, owner, span);
        }
        // 一時的な値は先に評価する（ムーブ）。変数などの場所は、引数を評価してから借りる
        let temp = if is_place(recv) { None } else { Some(self.eval(recv)?) };
        let vals = match self.eval_all(args) {
            Ok(v) => v,
            Err(d) => {
                self.discard(temp.into_iter().collect(), recv.span);
                return Err(d);
            }
        };
        let (mut rv, place) = match temp {
            Some(v) => (v, None),
            None => {
                let pl = self.place(recv)?;
                // 配列・リストは丸ごと複製しない（種類が分かれば十分。中身はあとで場所から触る）
                let v = match self.place_mut(&pl) {
                    Value::Array(..) => Value::Array(Vec::new(), false),
                    v => v.clone(),
                };
                (v, Some(pl))
            }
        };
        // 文字列のメソッド
        let text = match &rv {
            Value::Str(t) => Some(t.clone()),
            Value::Own(_) | Value::Ref(_) => {
                let (slot, path) = self.check_ptr(&rv, recv.span, Check::Required)?;
                match self.at(slot, &path) {
                    Value::Str(t) => Some(t.clone()),
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some(text) = text {
            if place.is_none() {
                self.release(rv, span, "使い終わった一時的な値");
            }
            let v = str_method(&text, name, vals, span)?;
            return Ok(self.adopt(v));
        }
        // 数のメソッド（f64 の `to_bits` / `sqrt` / `abs` / `max` / `min`、整数の `abs` / `max` / `min`）
        let num = match &rv {
            Value::Float(_) | Value::Int(..) => Some(rv.clone()),
            Value::Own(_) | Value::Ref(_) => {
                let (slot, path) = self.check_ptr(&rv, recv.span, Check::Required)?;
                match self.at(slot, &path) {
                    v @ (Value::Float(_) | Value::Int(..)) => Some(v.clone()),
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some(x) = num {
            return Ok(num_method(x, name, vals));
        }
        // task と chan のメソッド
        let handle = match &rv {
            Value::Task(_) | Value::Chan(_) => Some(rv.clone()),
            Value::Own(_) | Value::Ref(_) => {
                let (slot, path) = self.check_ptr(&rv, recv.span, Check::Required)?;
                match self.at(slot, &path) {
                    v @ (Value::Task(_) | Value::Chan(_)) => Some(v.clone()),
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some(h) = handle {
            let r = self.thread_method(&h, name, vals, span);
            if place.is_none() {
                self.release(rv, span, "使い終わった一時的な値");
            }
            return r;
        }
        // 配列・リストのメソッド（`len()`・`push(x)`・`pop()`）
        let seq_slot = match &rv {
            Value::Array(..) => Some(None),
            Value::List(s) => Some(Some((*s, Vec::new()))),
            Value::Own(_) | Value::Ref(_) => {
                let (slot, path) = self.check_ptr(&rv, recv.span, Check::Required)?;
                match self.at(slot, &path) {
                    Value::Array(..) => Some(Some((slot, path))),
                    Value::List(s) => Some(Some((*s, Vec::new()))),
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some(slot) = seq_slot {
            if !matches!(name, "len" | "map" | "filter" | "join") {
                let access = place.as_ref().map_or(Access::Write, |p| p.access.clone());
                access.through(&rv).require(span, &format!("リストを `{}` で書き換えようとした", name))?;
            }
            if matches!(name, "sort" | "sort_by" | "sort_unstable" | "sort_unstable_by") {
                let loc = match (slot, &place) {
                    (Some(sp), _) => Some(sp),
                    (None, Some(pl)) => Some((pl.slot, pl.path.clone())),
                    (None, None) => None,
                };
                let r = self.sort_list(loc.as_ref(), &mut rv, vals.into_iter().next(), !name.starts_with("sort_unstable"), recv, span);
                if let (Ok(()), Some((s, p))) = (&r, &loc) {
                    if p.is_empty() && matches!(self.heap.slots[*s].value, Some(Value::Array(_, true))) {
                        self.list_moved(*s, span, name)?;
                    }
                }
                if place.is_none() {
                    self.release(rv, span, "使い終わった一時的な値");
                }
                return r.map(|_| Value::Unit);
            }
            if name == "map" || name == "filter" {
                let items = match (&slot, &place) {
                    (Some((s, path)), _) => self.at(*s, path).clone(),
                    (None, Some(pl)) => self.place_mut(pl).clone(),
                    (None, None) => rv.clone(),
                };
                let Value::Array(items, _) = self.seq_at(&items).clone() else { unreachable!("checked above") };
                let f = vals.into_iter().next().expect("one function");
                let r = self.map_list(items, &f, name == "filter", recv, span);
                self.release(f, span, "使い終わった関数の値");
                if place.is_none() {
                    self.release(rv, span, "使い終わった一時的な値");
                }
                return r;
            }
            let mut vals = vals;
            // リストの中身のスロット（要素が動いたら世代を進める）
            let list = match &slot {
                Some((s, p)) if p.is_empty() && matches!(self.heap.slots[*s].value, Some(Value::Array(_, true))) => Some(*s),
                _ => None,
            };
            let items = match (slot, &place) {
                (Some((s, path)), _) => self.at_mut(s, &path),
                (None, Some(pl)) => self.place_mut(pl),
                (None, None) => &mut rv,
            };
            let Value::Array(items, _) = items else { unreachable!("checked above") };
            let len = items.len();
            let mut drained = None;
            let result = match name {
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
                    None => Value::Enum { variant: Rc::from("None"), fields: Vec::new(), copy: true },
                },
                "remove" => {
                    let i = match vals[0] {
                        Value::Int(i, _) => i,
                        _ => unreachable!("type checked"),
                    };
                    let k = bounds(i, items.len(), args[0].span)?;
                    items.remove(k)
                }
                "reverse" => {
                    items.reverse();
                    Value::Unit
                }
                "join" => Value::Str(Rc::new(join_strs(items, &vals[0]))),
                "drain" => {
                    drained = Some(std::mem::take(items));
                    Value::Unit
                }
                "insert" => {
                    let i = match vals[0] {
                        Value::Int(i, _) => i,
                        _ => unreachable!("type checked"),
                    };
                    // 末尾（長さと同じ位置）にも入れられる
                    if i < 0 || i > items.len() as i128 {
                        let len = items.len();
                        let v = vals.remove(1);
                        self.release(v, span, "入れられずに捨てられた");
                        return Err(Diagnostic::new("insert の位置が範囲の外です")
                            .primary(args[0].span, format!("位置は {} ですが、長さは {} です", i, len))
                            .note("入れられる位置は 0〜長さ です"));
                    }
                    items.insert(i as usize, vals.remove(1));
                    Value::Unit
                }
                _ => {
                    return Err(Diagnostic::new(format!("メソッド `{}` はありません", name)).primary(span, "ここ"))
                }
            };
            let result = match drained {
                Some(items) => self.new_list(items),
                None => result,
            };
            // 要素が動いた（長さが変わった・並びが変わった）ら、要素を借りた ref を無効にする
            let moved = match name {
                "push" | "insert" | "remove" | "reverse" | "drain" => true,
                "pop" => len > 0,
                _ => false,
            };
            if let (true, Some(s)) = (moved, list) {
                self.list_moved(s, span, name)?;
            }
            if place.is_none() {
                self.release(rv, span, "使い終わった一時的な値");
            }
            return Ok(result);
        }
        // 辞書のメソッド
        let map_slot = match &rv {
            Value::Map(_) => Some(None),
            Value::Own(_) | Value::Ref(_) => {
                let (slot, path) = self.check_ptr(&rv, recv.span, Check::Required)?;
                match self.at(slot, &path) {
                    Value::Map(_) => Some(Some((slot, path))),
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some(slot) = map_slot {
            if name == "remove" {
                let access = place.as_ref().map_or(Access::Write, |p| p.access.clone());
                access.through(&rv).require(span, "辞書を `remove` で書き換えようとした")?;
            }
            let m = match (slot, &place) {
                (Some((s, path)), _) => self.at_mut(s, &path),
                (None, Some(pl)) => self.place_mut(pl),
                (None, None) => &mut rv,
            };
            let Value::Map(m) = m else { unreachable!("checked above") };
            let result = match name {
                "len" => Value::Int(m.entries.len() as i128, IntTy::I64),
                "contains" => Value::Bool(m.find(&vals[0]).is_some()),
                // get は値がコピーできるときだけ（型検査が確かめる）
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
                _ => {
                    return Err(Diagnostic::new(format!("メソッド `{}` はありません", name)).primary(span, "ここ"))
                }
            };
            let result = self.adopt(result);
            if place.is_none() {
                self.release(rv, span, "使い終わった一時的な値");
            }
            return Ok(result);
        }
        // enum の値は型の名前を持たないので、型検査が書いた名前を使う
        let enum_name = owner.borrow().enum_name.clone();
        let sname = match &rv {
            _ if enum_name.is_some() => {
                if matches!(rv, Value::Own(_) | Value::Ref(_)) {
                    self.check_ptr(&rv, recv.span, Check::Required)?;
                }
                enum_name.expect("checked above")
            }
            Value::Struct(def, _) => def.name.clone(),
            Value::Own(_) | Value::Ref(_) => {
                let (slot, path) = self.check_ptr(&rv, recv.span, Check::Required)?;
                match self.at(slot, &path) {
                    Value::Struct(def, _) => def.name.clone(),
                    other => return Err(not_a_struct(other, recv.span)),
                }
            }
            other => return Err(not_a_struct(other, recv.span)),
        };
        let f = self.fns.get(&format!("{}.{}", sname, name)).cloned().ok_or_else(|| {
            Diagnostic::new(format!("`{}` にメソッド `{}` はありません", sname, name))
                .primary(span, "このメソッドは定義されていません")
        })?;
        let mutable = matches!(f.params[0].ty.kind, TypeKind::Ref(_, true));
        if mutable {
            let access = place.as_ref().map_or(Access::Write, |p| p.access.clone());
            access.through(&rv).require(span, &format!("書き換えるメソッド `{}` を呼ぼうとした", name))?;
        }
        let this = |slot: usize, generation: u64, what: String| {
            Value::Ref(RefVal { slot, generation, path: Vec::new(), borrowed_at: recv.span, what, mutable, view: false })
        };
        let mut lent: Option<Lent> = None;
        let this_val = match rv {
            Value::Own(slot) => {
                if place.is_none() {
                    // 一時的な own は、呼び出しが終わったら解放する
                    lent = Some(Lent { slot, back: None, own: true });
                }
                this(slot, self.heap.slots[slot].generation, describe(recv))
            }
            Value::Ref(r) => Value::Ref(RefVal { mutable, view: false, ..r }),
            // 場所にある構造体は、その場所を借りて this にする（リストの中は場所が変わりうるので貸す）
            _ if matches!(&place, Some(pl) if !self.through_map(pl.slot, &pl.path)) => {
                let pl = place.expect("checked above");
                Value::Ref(RefVal {
                    slot: pl.slot,
                    generation: self.heap.slots[pl.slot].generation,
                    path: pl.path,
                    borrowed_at: recv.span,
                    what: describe(recv),
                    mutable,
                    view: false,
                })
            }
            v => {
                let back = place.map(|pl| {
                    // 元の場所は空にしておく（呼び出しのあいだに別の道から触れても、古い値は見えない）
                    *self.place_mut(&pl) = Value::Unit;
                    let owner = Some((pl.slot, self.heap.slots[pl.slot].generation));
                    (pl, owner)
                });
                let slot = self.heap.alloc(v);
                lent = Some(Lent { slot, back, own: false });
                this(slot, self.heap.slots[slot].generation, describe(recv))
            }
        };
        let mut all = vec![this_val];
        all.extend(vals);
        let result = self.call_fn(&f, all, span)?;
        match lent {
            None => {}
            Some(Lent { slot, back: None, own: true }) => self.free(slot, span, "メソッドの呼び出しに使い終わった一時的な値"),
            Some(Lent { slot, back: None, own: false }) => {
                let why = "メソッドの呼び出しに使い終わった一時的な値";
                let v = self.vacate(slot, span, why);
                self.release(v, span, why);
            }
            Some(Lent { slot, back: Some((pl, owner)), .. }) => {
                let v = self.heap.slots[slot].value.replace(Value::Unit).expect("lent slot is live");
                self.free(slot, span, &format!("メソッドの呼び出しが終わって `{}` に戻した", describe(recv)));
                if let Some((s, g)) = owner {
                    if self.heap.slots[s].retired || self.heap.slots[s].generation != g || self.heap.slots[s].value.is_none() {
                        self.release(v, span, "戻す先が解放されていた");
                        return Err(Diagnostic::new("メソッドの呼び出し中に、呼び出し先の持ち主が解放されました")
                            .primary(recv.span, "この値を戻す場所がもうない"));
                    }
                }
                *self.place_mut(&pl) = v;
            }
        }
        Ok(result)
    }

    fn call_fn(&mut self, f: &Rc<FnDef>, args: Vec<Value>, call_span: Span) -> R<Value> {
        if self.world.aborted.load(Ordering::Relaxed) {
            self.discard_all(args, call_span);
            return Err(aborted_marker());
        }
        if self.depth >= MAX_DEPTH {
            return Err(Diagnostic::new("関数呼び出しが深すぎます")
                .primary(call_span, format!("呼び出しの深さが {} を超えました", MAX_DEPTH)));
        }
        let vars: Vec<Var> = f
            .params
            .iter()
            .zip(args)
            .map(|(p, v)| Var {
                name: p.name.clone(),
                state: VarState::Live(self.heap.alloc(v)),
                mutable: p.mutable,
                decl: p.span,
                view: false,
            })
            .collect();
        // 戻り値の型がある関数では、本体の最後の `;` のない式が戻り値になる
        self.run_frame(vars, &f.body, f.ret.is_some())
    }

    /// リストを並べ替える（安定）。`f` があれば「a を b より前に置くか」を返す関数で比べる。
    /// 比べる関数には要素を読むだけの写しを渡す（for と同じ）。並べ終わった順に、元の要素を動かす
    /// `a.map(f)` / `a.filter(f)`。始めた時点の要素を、`for x in a` の x と同じ形（own は ref、構造体は読むだけの写し）で f に渡す
    fn map_list(&mut self, items: Vec<Value>, f: &Value, filter: bool, recv: &Expr, span: Span) -> R<Value> {
        let what = format!("{} の要素", describe(recv));
        let mut out = Vec::new();
        // for と同じく、始めた時点の写しをまとめて作る
        let views: Vec<Value> = items
            .into_iter()
            .map(|v| match v {
                Value::Own(slot) => Value::Ref(RefVal {
                    slot,
                    generation: self.heap.slots[slot].generation,
                    path: Vec::new(),
                    borrowed_at: span,
                    what: what.clone(),
                    mutable: false,
                    view: false,
                }),
                other if other.moves() => self.view_of(other, span, &what),
                other => unview(other),
            })
            .collect();
        let mut views = views.into_iter();
        for v in views.by_ref() {
            let (v, copy) = if v.moves() {
                let slot = self.heap.alloc(v);
                let r = RefVal {
                    slot,
                    generation: self.heap.slots[slot].generation,
                    path: Vec::new(),
                    borrowed_at: span,
                    what: what.clone(),
                    mutable: false,
                    view: false,
                };
                (Value::Ref(r), Some(slot))
            } else {
                (v, None)
            };
            let r = self.call_func(f, vec![v.clone()], span);
            if let Some(slot) = copy {
                let w = self.vacate(slot, span, "map の次の要素へ進んだ");
                self.release(w, span, "map の次の要素へ進んだ");
            }
            let r = match r {
                Ok(r) => r,
                Err(d) => {
                    for w in views {
                        self.release(w, span, "map の次の要素へ進んだ");
                    }
                    self.discard(out, span);
                    return Err(d);
                }
            };
            match (filter, r) {
                (false, r) => out.push(r),
                (true, Value::Bool(true)) => out.push(v),
                (true, _) => {}
            }
        }
        Ok(self.new_list(out))
    }

    fn sort_list(&mut self, loc: Option<&(usize, Vec<usize>)>, tmp: &mut Value, f: Option<Value>, stable: bool, recv: &Expr, span: Span) -> R<()> {
        let items = match loc {
            Some((s, p)) => self.at(*s, p),
            None => &*tmp,
        };
        let Value::Array(items, _) = items else { unreachable!("checked by caller") };
        let items = items.clone();
        let n = items.len();
        let what = format!("{} の要素", describe(recv));
        let mut temps = Vec::new();
        let views: Vec<Value> = items
            .into_iter()
            .map(|v| {
                if f.is_none() {
                    return v; // sort は整数・小数・文字列だけ（コピーできる）
                }
                // own の要素は own の先を借りた ref。構造体などは写しを一時的なスロットに置いて借りる
                // own の要素は ref の引数として渡すので、写しの印は付けない
                let v = unview(self.view_of(v, span, &what));
                if !v.moves() {
                    return v;
                }
                let slot = self.heap.alloc(v);
                temps.push(slot);
                Value::Ref(RefVal {
                    slot,
                    generation: self.heap.slots[slot].generation,
                    path: Vec::new(),
                    borrowed_at: span,
                    what: what.clone(),
                    mutable: false,
                    view: false,
                })
            })
            .collect();
        let mut less = |me: &mut Self, a: usize, b: usize| -> R<bool> {
            match &f {
                None => Ok(match (&views[a], &views[b]) {
                    (Value::Int(x, _), Value::Int(y, _)) => x < y,
                    (Value::Float(x), Value::Float(y)) => x < y,
                    (Value::Str(x), Value::Str(y)) => x < y,
                    _ => unreachable!("type checked: ordered elements"),
                }),
                Some(f) => match me.call_func(f, vec![views[a].clone(), views[b].clone()], span)? {
                    Value::Bool(x) => Ok(x),
                    other => Err(Diagnostic::new("sort_by の関数は true / false を返します")
                        .primary(span, format!("{} が返りました", other.type_name()))),
                },
            }
        };
        let order = crate::sort::sort(self, n, stable, &mut less);
        for slot in temps {
            let w = self.vacate(slot, span, "sort_by の比べ終わった写し");
            self.release(w, span, "sort_by の比べ終わった写し");
        }
        if let Some(f) = f {
            self.release(f, span, "sort_by が使い終わった関数");
        }
        let order = order?;
        let items = match loc {
            Some((s, p)) => self.at_mut(*s, p),
            None => tmp,
        };
        let Value::Array(items, _) = items else {
            return Err(Diagnostic::new("sort_by の途中でリストが置き換えられました").primary(span, "ここ"));
        };
        if items.len() != n {
            return Err(Diagnostic::new("sort_by の途中でリストの長さが変わりました")
                .primary(span, format!("始めは {} 個でしたが、今は {} 個です", n, items.len())));
        }
        let mut old: Vec<Option<Value>> = std::mem::take(items).into_iter().map(Some).collect();
        *items = order.into_iter().map(|i| old[i].take().expect("a permutation")).collect();
        Ok(())
    }

    /// 変数に入った関数の値を呼ぶ。引数を先に評価してから、変数を借りて読む
    fn call_var(&mut self, name: &str, args: &[Expr], span: Span) -> R<Value> {
        let vals = self.eval_all(args)?;
        let var = self.find_var(name).expect("checked by caller");
        let v = match var.state.clone() {
            VarState::Live(slot) => self.heap.slots[slot].value.clone().expect("live slot"),
            VarState::Moved(sp) => {
                self.discard(vals, span);
                return Err(moved_error(name, sp, span));
            }
        };
        let f = match v {
            Value::Own(_) | Value::Ref(_) => match self.check_ptr(&v, span, Check::Required) {
                Ok((slot, path)) => self.at(slot, &path).clone(),
                Err(d) => {
                    self.discard(vals, span);
                    return Err(d);
                }
            },
            v => v,
        };
        self.call_func(&f, vals, span)
    }

    /// 関数の値を呼ぶ。`f` は借りて読んだもの（取り込んだ値は呼び出しのあいだ読むだけの写しで渡す）
    fn call_func(&mut self, f: &Value, args: Vec<Value>, span: Span) -> R<Value> {
        let Value::Func(callee, caps) = f else {
            return Err(Diagnostic::new("関数ではない値を呼び出しました")
                .primary(span, format!("これは {} です", f.type_name())));
        };
        let l = match callee {
            Callee::Named(def) => return self.call_fn(def, args, span),
            Callee::Lambda(l) => l.clone(),
            Callee::Ir(_) => unreachable!("IR functions run only in ir::exec"),
        };
        if self.depth >= MAX_DEPTH {
            return Err(Diagnostic::new("関数呼び出しが深すぎます")
                .primary(span, format!("呼び出しの深さが {} を超えました", MAX_DEPTH)));
        }
        let mut vars: Vec<Var> = Vec::new();
        for (p, v) in l.params.iter().zip(args) {
            let slot = self.heap.alloc(v);
            vars.push(Var { name: p.name.clone(), state: VarState::Live(slot), mutable: false, decl: p.span, view: false });
        }
        let names = l.captures.borrow().clone();
        for (n, c) in names.iter().zip(caps) {
            // 取り込んだ own はクロージャのもの。呼び出しの中では、それを借りた ref の写しで使う
            // （呼び出しの途中で元のクロージャが捨てられても、たどったときに検出される）
            let view = c.moves();
            let v = self.view_of(c.clone(), span, n);
            let slot = self.heap.alloc(v);
            vars.push(Var { name: n.clone(), state: VarState::Live(slot), mutable: false, decl: span, view });
        }
        self.run_frame(vars, &l.body, l.returns.get())
    }

    /// 新しいフレームで本体を実行する。`want_value` なら最後の `;` のない式が戻り値
    fn run_frame(&mut self, vars: Vec<Var>, body: &Block, want_value: bool) -> R<Value> {
        self.depth += 1;
        self.frames.push(Frame { scopes: vec![vars] });
        let mut by_try = false;
        let (flow, tail) = match self.exec_block_value(body, want_value) {
            Ok(r) => r,
            // `e?` で抜けてきた。途中のスコープは frame に残っているので、下でまとめて解放する
            Err(d) => match self.unwinding.take() {
                Some((v, sp)) => {
                    by_try = true;
                    (Flow::Return(v, sp), Value::Unit)
                }
                None => return Err(d),
            },
        };

        let frame = self.frames.pop().expect("frame");
        self.depth -= 1;
        let (value, span, how) = match flow {
            Flow::Return(v, sp) if by_try && !std::mem::take(&mut self.returning) => (v, sp, "`?` で関数を抜けた"),
            Flow::Return(v, sp) => (v, sp, "関数から return した"),
            Flow::Normal => (tail, body.end, "スコープを抜けた"),
            Flow::Break(_) | Flow::Continue(_) => unreachable!("parser rejects break/continue outside loops"),
        };
        for scope in frame.scopes.into_iter().rev() {
            self.drop_scope(scope, span, how);
        }
        Ok(value)
    }
}

// ---------------------------------------------------------------- ヘルパ

/// 整数を、型検査が決めた型（`Expr.int`）に変換する
fn convert(e: &Expr, v: Value) -> Value {
    match (v, e.int.get()) {
        (Value::Int(n, _), _) if e.float.get() => Value::Float(n as f64),
        (Value::Int(n, _), Some(t)) => Value::Int(t.wrap(n), t),
        (v, _) => v,
    }
}

/// 小数を整数にする（0 の方へ切り捨て）。i128 に入らないものは端に寄せ、NaN は 0
pub(crate) fn float_to_int(x: f64) -> i128 {
    if x.is_nan() {
        0
    } else {
        x.trunc().clamp(i128::MIN as f64, i128::MAX as f64) as i128
    }
}

/// リストの要素が動いたときの説明（IR・実行ファイルと同じ文）
pub(crate) fn list_moved_why(name: &str) -> String {
    format!("`{}` でリストの要素が動いた", name)
}

pub(crate) fn describe(e: &Expr) -> String {
    match &e.kind {
        ExprKind::Var(n) => n.clone(),
        ExprKind::Deref(inner, _) => format!("*{}", describe(inner)),
        ExprKind::Field(base, name, _) => format!("{}.{}", describe(base), name),
        ExprKind::Index(base, ..) => format!("{}[..]", describe(base)),
        _ => "式".to_string(),
    }
}

/// 場所として扱える式か（`ref` やフィールド代入の対象になれる）
fn is_place(e: &Expr) -> bool {
    matches!(e.kind, ExprKind::Var(_) | ExprKind::Deref(..) | ExprKind::Field(..) | ExprKind::Index(..))
}

/// 添字が範囲の中か確かめて、usize にする
pub(crate) fn bounds(i: i128, len: usize, span: Span) -> R<usize> {
    if i < 0 || i >= len as i128 {
        return Err(Diagnostic::new(format!("添字 {} は配列の範囲の外です", i))
            .primary(span, format!("長さは {} なので、添字は 0〜{} です", len, len as i128 - 1)));
    }
    Ok(i as usize)
}

/// 配列・辞書 `c` の中で、添字・キー `key` の要素がどこにあるか
pub(crate) fn locate(c: &Value, key: &Value, key_span: Span, base_span: Span) -> R<usize> {
    match (c, key) {
        (Value::Array(items, _), Value::Int(i, _)) => bounds(*i, items.len(), key_span),
        (Value::Array(..), other) => {
            Err(Diagnostic::new("添字は整数です").primary(key_span, format!("ここは {} です", other.type_name())))
        }
        (Value::Map(m), k) => m.find(k).ok_or_else(|| {
            let shown = match k {
                Value::Str(s) => format!("\"{}\"", s),
                Value::Int(n, _) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => "?".to_string(),
            };
            Diagnostic::new(format!("辞書にキー {} がありません", shown))
                .primary(key_span, "このキーの値は入っていない")
                .note("ないかもしれないなら `m.get(k)`（Option）か `m.contains(k)` を使ってください")
        }),
        (other, _) => Err(not_an_array(other, base_span)),
    }
}

/// 配列・辞書の i 番目の要素（辞書なら値）
fn entry(c: &Value, i: usize) -> &Value {
    match c {
        Value::Array(items, _) => &items[i],
        Value::Map(m) => &m.entries[i].1,
        _ => unreachable!("located above"),
    }
}

/// 辞書の中の値を借りようとした（型検査・IR・実行ファイルと同じ文）
pub(crate) fn map_borrow_error(span: Span) -> Diagnostic {
    Diagnostic::new("辞書の中の値は借りられません")
        .primary(span, "辞書はキーを取り除くと値の場所が詰まるので、中の場所が変わる")
        .note("値を借りたいときは、値を own にしてください（`(own T)[K]`）")
}

fn not_an_array(v: &Value, span: Span) -> Diagnostic {
    Diagnostic::new("添字が使えるのは配列だけです").primary(span, format!("ここは {} です", v.type_name()))
}

fn not_a_struct(v: &Value, span: Span) -> Diagnostic {
    Diagnostic::new("フィールドを持たない値です")
        .primary(span, format!("{} は構造体ではありません", v.type_name()))
}

/// match で当てはまる case のブロックを探す（なければ None）
fn select_arm<'a>(v: &Value, scrut_span: Span, arms: &'a [MatchArm]) -> R<Option<&'a Block>> {
    if !matches!(v, Value::Int(..) | Value::Bool(_) | Value::Str(_)) {
        return Err(Diagnostic::new("match できるのは、今のところ整数・真偽値・文字列だけです")
            .primary(scrut_span, format!("ここは {} です", v.type_name())));
    }
    for arm in arms {
        for (pat, span) in &arm.pats {
            let hit = match (pat, v) {
                (Pat::Int(a), Value::Int(b, _)) => a == b,
                (Pat::Bool(a), Value::Bool(b)) => a == b,
                (Pat::Str(a), Value::Str(b)) => *a == **b,
                _ => {
                    return Err(Diagnostic::new("case の値と match する値の型が合いません")
                        .label(scrut_span, format!("これは {}", v.type_name()))
                        .primary(*span, "この case は型が違う"))
                }
            };
            if hit {
                return Ok(Some(&arm.body));
            }
        }
    }
    Ok(None)
}

fn undefined_var(name: &str, span: Span) -> Diagnostic {
    Diagnostic::new(format!("未定義の変数 `{}`", name)).primary(span, "この名前は見つかりません")
}

pub(crate) fn some(v: Value) -> Value {
    Value::Enum { variant: Rc::from("Some"), fields: vec![v], copy: true }
}

pub(crate) fn none() -> Value {
    Value::Enum { variant: Rc::from("None"), fields: Vec::new(), copy: true }
}

fn str_list<'a>(items: impl Iterator<Item = &'a str>) -> Value {
    Value::Array(items.map(|x| Value::Str(text_rc(x))).collect(), true)
}

/// 数のメソッド（types.rs の `num_method` が名前と引数を調べてある）。IR の exec も使う
pub(crate) fn num_method(x: Value, name: &str, vals: Vec<Value>) -> Value {
    match (x, name, vals.as_slice()) {
        (Value::Float(x), "to_bits", []) => Value::Int(x.to_bits() as i128, IntTy::U64),
        (Value::Float(x), "sqrt", []) => Value::Float(x.sqrt()),
        (Value::Float(x), "abs", []) => Value::Float(x.abs()),
        (Value::Float(x), "max", [Value::Float(y)]) => Value::Float(x.max(*y)),
        (Value::Float(x), "min", [Value::Float(y)]) => Value::Float(x.min(*y)),
        (Value::Int(x, t), "abs", []) => Value::Int(t.wrap(x.abs()), t),
        (Value::Int(x, t), "max", [Value::Int(y, _)]) => Value::Int(x.max(*y), t),
        (Value::Int(x, t), "min", [Value::Int(y, _)]) => Value::Int(x.min(*y), t),
        (_, name, _) => unreachable!("type checked: {}", name),
    }
}

/// 文字列のメソッド。位置と長さは文字単位（バイトではない）
/// 文字列の長さの上限（バイト）。jw_rt.c の STR_MAX と同じ
const STR_MAX: i64 = 1 << 62;

pub(crate) fn str_method(text: &str, name: &str, vals: Vec<Value>, span: Span) -> R<Value> {
    let s = |i: usize| match &vals[i] {
        Value::Str(x) => Ok(x.clone()),
        other => Err(Diagnostic::new("文字列が必要です").primary(span, format!("{} が渡された", other.type_name()))),
    };
    let n = |i: usize| match &vals[i] {
        Value::Int(x, _) => Ok(*x),
        other => Err(Diagnostic::new("整数が必要です").primary(span, format!("{} が渡された", other.type_name()))),
    };
    let string = |x: String| Value::Str(text_rc(x.as_str()));
    Ok(match name {
        "len" => Value::Int(text.chars().count() as i128, IntTy::I64),
        "byte_len" => Value::Int(text.len() as i128, IntTy::I64),
        "chars" => Value::Array(text.chars().map(|c| string(c.to_string())).collect(), true),
        "bytes" => Value::Array(text.bytes().map(|b| Value::Int(b as i128, IntTy::U8)).collect(), true),
        "code" => match text.chars().next() {
            Some(c) => Value::Int(c as i128, IntTy::I64),
            None => {
                return Err(Diagnostic::new("空の文字列には文字がありません").primary(span, "code() は 1 文字目の番号を返します"))
            }
        },
        "lines" => str_list(text.lines()),
        "split" => {
            let sep = s(0)?;
            if sep.is_empty() {
                return Err(Diagnostic::new("空の文字列では分けられません")
                    .primary(span, "1 文字ずつにするなら `chars()` を使ってください"));
            }
            str_list(text.split(&*sep))
        }
        "trim" => string(text.trim().to_string()),
        "to_upper" => string(text.to_uppercase()),
        "to_lower" => string(text.to_lowercase()),
        "contains" => Value::Bool(text.contains(&*s(0)?)),
        "starts_with" => Value::Bool(text.starts_with(&*s(0)?)),
        "ends_with" => Value::Bool(text.ends_with(&*s(0)?)),
        "find" => match text.find(&*s(0)?) {
            Some(b) => some(Value::Int(text[..b].chars().count() as i128, IntTy::I64)),
            None => none(),
        },
        "slice" => {
            let (a, b) = (n(0)?, n(1)?);
            let len = text.chars().count() as i128;
            if a < 0 || b < a || b > len {
                return Err(Diagnostic::new("文字列の範囲の外です")
                    .primary(span, format!("slice({}, {}) ですが、長さは {} です", a, b, len))
                    .note("0 <= 始め <= 終わり <= 長さ にしてください"));
            }
            string(text.chars().skip(a as usize).take((b - a) as usize).collect())
        }
        "replace" => string(text.replace(&*s(0)?, &s(1)?)),
        "repeat" => {
            let k = n(0)?;
            if k < 0 {
                return Err(Diagnostic::new("負の回数はくり返せません").primary(span, format!("repeat({})", k)));
            }
            if text.is_empty() || k == 0 {
                return Ok(string(String::new()));
            }
            // 長さ × 回数が上限を超えるなら、確保する前にエラー（jw_rt.c の jw_str_repeat と同じ）
            if k * text.len() as i128 > STR_MAX as i128 {
                return Err(Diagnostic::new("くり返した文字列が長すぎます")
                    .primary(span, format!("repeat({})", k))
                    .note("文字列は 2^62 バイトまで"));
            }
            let mut out = String::new();
            if out.try_reserve_exact(text.len() * k as usize).is_err() {
                return Err(Diagnostic::new("メモリが足りません").primary(span, format!("repeat({})", k)));
            }
            for _ in 0..k {
                out.push_str(text);
            }
            string(out)
        }
        "to_int" => match text.trim().parse::<i64>() {
            Ok(v) => some(Value::Int(v as i128, IntTy::I64)),
            Err(_) => none(),
        },
        "to_float" => match text.trim().parse::<f64>() {
            Ok(v) => some(Value::Float(v)),
            Err(_) => none(),
        },
        _ => return Err(Diagnostic::new(format!("文字列にメソッド `{}` はありません", name)).primary(span, "ここ")),
    })
}

/// `a.join(sep)`: 文字列のリストを sep でつなぐ
pub(crate) fn join_strs(items: &[Value], sep: &Value) -> String {
    let Value::Str(sep) = sep else { unreachable!("type checked: str") };
    let parts: Vec<&str> = items
        .iter()
        .map(|v| match v {
            Value::Str(s) => s.as_str(),
            _ => unreachable!("type checked: str[]"),
        })
        .collect();
    parts.join(sep.as_str())
}

/// `chr(n)`: 番号 n の文字 1 つの文字列
pub(crate) fn chr(n: i128, span: Span) -> R<Value> {
    match u32::try_from(n).ok().and_then(char::from_u32) {
        Some(c) => Ok(Value::Str(text_rc(c.to_string().as_str()))),
        None => Err(Diagnostic::new("文字の番号ではありません")
            .primary(span, format!("chr({}) は文字になりません", n))
            .note("使えるのは 0〜1114111 で、55296〜57343（サロゲート）を除く番号です")),
    }
}

fn arity_error(name: &str, want: usize, got: usize, span: Span) -> Diagnostic {
    Diagnostic::new(format!("`{}` の引数の数が違います", name))
        .primary(span, format!("{} 個必要ですが {} 個渡されました", want, got))
}

pub(crate) fn moved_error(name: &str, moved_at: Span, used_at: Span) -> Diagnostic {
    if moved_at == used_at {
        return Diagnostic::new(format!("ムーブ済みの値 `{}` を使いました (use after move)", name))
            .primary(
                used_at,
                format!("ここで `{}` の所有権を移すが、前の周回ですでに移動済み", name),
            )
            .note("ループの中で own を渡すと、2周目には手元に残っていません");
    }
    Diagnostic::new(format!("ムーブ済みの値 `{}` を使いました (use after move)", name))
        .label(moved_at, format!("ここで `{}` の所有権が移動した", name))
        .primary(used_at, format!("ここで `{}` を使ったが、もう所有していない", name))
        .note("移した後も使いたいなら `ref` で借りてください（例: `f(ref a)`）")
}

pub(crate) fn as_bool(v: Value, span: Span) -> R<bool> {
    match v {
        Value::Bool(b) => Ok(b),
        other => Err(Diagnostic::new("真偽値が必要です")
            .primary(span, format!("ここは {} です", other.type_name()))),
    }
}

pub(crate) fn binary(op: BinOp, l: Value, r: Value, span: Span) -> R<Value> {
    match (op, &l, &r) {
        // 値は型の範囲で持っている（符号つきの負の数は i128 でも負）ので、i128 のまま計算して型の幅で一周させればよい
        (BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor, Value::Int(a, t), Value::Int(b, _)) => Ok(Value::Int(
            t.wrap(match op {
                BinOp::BitAnd => a & b,
                BinOp::BitOr => a | b,
                _ => a ^ b,
            }),
            *t,
        )),
        // 型の幅以上ずらすと、<< は 0、>> は 0 か -1（Go と同じ）。負の数だけはずらせない
        (BinOp::Shl | BinOp::Shr, Value::Int(a, t), Value::Int(n, _)) => {
            if *n < 0 {
                return Err(Diagnostic::new("負の数だけはずらせません").primary(span, "右の数が負です"));
            }
            let big = *n >= t.bits() as i128;
            let v = match op {
                BinOp::Shl if big => 0,
                BinOp::Shl => a << n,
                _ if big => {
                    if *a < 0 {
                        -1
                    } else {
                        0
                    }
                }
                _ => a >> n,
            };
            Ok(Value::Int(t.wrap(v), *t))
        }
        (BinOp::Add, Value::Str(a), Value::Str(b)) => {
            let s = format!("{}{}", a, b);
            Ok(Value::Str(text_rc(s.as_str())))
        }
        (BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem, Value::Int(a, t), Value::Int(b, _)) => {
            // 両辺は型検査が同じ型にそろえている。i128 で計算して、型の幅で一周させる
            // （決まった結果になる。定数なら型検査が警告を出す）
            let res = match op {
                BinOp::Add => a + b,
                BinOp::Sub => a - b,
                BinOp::Mul => a.wrapping_mul(*b),
                BinOp::Div | BinOp::Rem => {
                    if *b == 0 {
                        return Err(Diagnostic::new("0 で割りました")
                            .primary(span, "右辺が 0 です"));
                    }
                    if op == BinOp::Div {
                        a / b
                    } else {
                        a % b
                    }
                }
                _ => unreachable!(),
            };
            Ok(Value::Int(t.wrap(res), *t))
        }
        // 小数は IEEE 754 のまま（0 で割ると inf / NaN。エラーにはしない）
        (BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem, Value::Float(a), Value::Float(b)) => {
            Ok(Value::Float(match op {
                BinOp::Add => a + b,
                BinOp::Sub => a - b,
                BinOp::Mul => a * b,
                BinOp::Div => a / b,
                _ => a % b,
            }))
        }
        (BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(match op {
            BinOp::Lt => a < b,
            BinOp::Le => a <= b,
            BinOp::Gt => a > b,
            _ => a >= b,
        })),
        (BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge, Value::Str(a), Value::Str(b)) => Ok(Value::Bool(match op {
            BinOp::Lt => a < b,
            BinOp::Le => a <= b,
            BinOp::Gt => a > b,
            _ => a >= b,
        })),
        (BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge, Value::Int(a, _), Value::Int(b, _)) => {
            Ok(Value::Bool(match op {
                BinOp::Lt => a < b,
                BinOp::Le => a <= b,
                BinOp::Gt => a > b,
                _ => a >= b,
            }))
        }
        (BinOp::Eq | BinOp::Ne, _, _) => {
            let eq = match (&l, &r) {
                (Value::Int(a, _), Value::Int(b, _)) => a == b,
                (Value::Float(a), Value::Float(b)) => a == b,
                (Value::Bool(a), Value::Bool(b)) => a == b,
                (Value::Str(a), Value::Str(b)) => a == b,
                (Value::Unit, Value::Unit) => true,
                _ => {
                    return Err(Diagnostic::new("型が合わない比較です").primary(
                        span,
                        format!("{} と {} は比較できません", l.type_name(), r.type_name()),
                    ))
                }
            };
            Ok(Value::Bool(if op == BinOp::Eq { eq } else { !eq }))
        }
        _ => Err(Diagnostic::new("型が合わない演算です").primary(
            span,
            format!("{} と {} にはこの演算を使えません", l.type_name(), r.type_name()),
        )),
    }
}

#[cfg(test)]
mod retirement_tests {
    use super::*;

    const AT: Span = Span { file: 0, line: 1, col: 1, len: 1 };

    fn int() -> Value { Value::Int(9, IntTy::I64) }

    fn reference(slot: usize, generation: u64) -> Value {
        Value::Ref(RefVal {
            slot, generation, path: Vec::new(), borrowed_at: AT,
            what: "退役の境界".to_string(), mutable: true, view: false,
        })
    }

    #[test]
    fn released_generations_never_wrap_or_reenter_the_free_list() {
        for limit in [0, 1, 3, u64::MAX] {
            let mut it = Interp::new(false);
            it.heap.generation_limit = limit;
            let slot = it.heap.alloc(int());
            it.heap.slots[slot].generation = limit;
            let old = reference(slot, limit);
            it.free(slot, AT, "解放した");
            assert!(it.heap.slots[slot].retired);
            assert_eq!(it.heap.slots[slot].generation, limit);
            assert!(!it.heap.free_list.contains(&slot));
            assert!(it.check_ptr(&old, AT, Check::Required).is_err());
            for _ in 0..32 {
                let fresh = it.heap.alloc(int());
                assert_ne!(fresh, slot);
                it.free(fresh, AT, "繰返し解放した");
            }
            assert!(it.heap.slots[slot].retired);
            assert_eq!(it.heap.live(), 0);
        }
    }

    #[test]
    fn vacating_a_retired_identity_preserves_the_moved_owner() {
        let mut it = Interp::new(false);
        it.heap.generation_limit = 0;
        let child = it.heap.alloc(int());
        let outer = it.heap.alloc(Value::Own(child));
        let old = reference(outer, 0);
        let moved = it.vacate(outer, AT, "ほかのスレッドへ送られた");
        assert!(matches!(moved, Value::Own(s) if s == child));
        assert!(it.heap.slots[outer].retired && it.heap.slots[child].value.is_some());
        assert!(it.check_ptr(&old, AT, Check::Required).is_err());
        it.release(moved, AT, "移動先で解放した");
        assert_eq!(it.heap.live(), 0);
        assert!(it.heap.slots[child].retired);
    }

    #[test]
    fn recursive_release_retires_each_identity_once() {
        let mut it = Interp::new(false);
        it.heap.generation_limit = 0;
        let child = it.heap.alloc(int());
        let parent = it.heap.alloc(Value::Array(vec![Value::Own(child)], false));
        it.free(parent, AT, "親を解放した");
        it.free(parent, AT, "再度の解放で世代を変えない");
        assert!(it.heap.slots.iter().all(|slot| slot.retired && slot.value.is_none()));
        assert!(it.heap.free_list.is_empty());
        assert_eq!(it.heap.live(), 0);
    }

    #[test]
    fn live_generation_exhaustion_rejects_even_a_matching_reference() {
        for limit in [1, 3, u64::MAX] {
            let mut it = Interp::new(false);
            it.heap.generation_limit = limit;
            let list = it.heap.alloc(Value::Array(vec![int()], true));
            it.heap.slots[list].generation = limit - 1;
            let before = reference(list, limit - 1);
            it.list_moved(list, AT, "reverse").unwrap();
            assert!(it.check_ptr(&before, AT, Check::Required).is_err());
            let at_limit = reference(list, limit);
            assert!(it.check_ptr(&at_limit, AT, Check::Required).is_ok());
            let error = it.list_moved(list, AT, "reverse").unwrap_err();
            assert_eq!(error.title, "世代番号の上限に達しました");
            assert!(it.heap.slots[list].retired && it.heap.slots[list].value.is_some());
            assert!(it.check_ptr(&at_limit, AT, Check::Required).is_err());
            it.free(list, AT, "停止後の回収");
            assert_eq!(it.heap.live(), 0);
            assert!(!it.heap.free_list.contains(&list));
        }
    }

    #[test]
    fn ir_stops_when_a_list_generation_is_exhausted() {
        let mut it = Interp::new(false);
        it.heap.generation_limit = 0;
        let list = it.heap.alloc(Value::Array(vec![int()], true));
        let variable = it.heap.alloc(Value::List(list));
        let prog = crate::ir::Program {
            fns: Vec::new(), by_name: HashMap::new(), structs: HashMap::new(),
            struct_tys: HashMap::new(), enums: HashMap::new(),
        };
        let error = crate::ir::exec::retirement_probe(&prog, &mut it, variable, AT).unwrap_err();
        assert_eq!(error.title, "世代番号の上限に達しました");
        assert!(it.heap.slots[list].retired);
        assert!(it.check_ptr(&reference(list, 0), AT, Check::Required).is_err());
    }
}
