//! IR（中間表現）。設計は docs/ir-design.md。
//!
//! 関数ごとに「基本ブロック＋命令」で書く。木をたどるインタプリタでは暗黙だった
//! ムーブ（`Operand::Move`）・解放（`Inst::Drop`）・変数の寿命の終わり（`Inst::Dead`）・
//! 世代チェック（`Proj::Deref` の `Check`）・ムーブ済みかの確かめ（`Inst::Live`）が、すべて命令として見える。
//!
//! 型はすべて具体的（ジェネリクスは型ごとに作り分けた関数になる）。
//! 正しさは `exec`（IR を実行する小さなインタプリタ）と今のインタプリタの出力を突き合わせて確かめる。

pub mod analyze;
pub mod exec;
pub mod llvm;
pub mod lower;

use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use crate::ast::{BinOp, Check, IntTy, StructDef, UnOp};
use crate::diag::Span;
use crate::types::Ty;

/// 関数の中の変数の番号（引数・ローカル変数・一時的な値・ムーブの印）
pub type Local = usize;
/// 基本ブロックの番号
pub type Bb = usize;

/// プログラム全体（単相化したあと）
pub struct Program {
    pub fns: Vec<Func>,
    /// 関数の名前から番号
    pub by_name: HashMap<String, usize>,
    /// 構造体の定義（値の表示とフィールドの名前に使う）
    pub structs: HashMap<String, Rc<StructDef>>,
    /// 構造体のフィールドの型（型の引数, 定義の順の型）。LLVM の書き出しに使う
    pub struct_tys: HashMap<String, (Vec<String>, Vec<Ty>)>,
    /// enum の型の引数と種類（種類の番号は並びの順）
    pub enums: HashMap<String, (Vec<String>, crate::types::Variants)>,
}

pub struct Func {
    /// 単相化した名前（`first<i64>`）。クロージャは `作った関数::λ行:列`
    pub name: String,
    /// `locals[0..params]` が引数（クロージャなら、そのうち最後の `captures` 個が取り込んだ値）
    pub params: usize,
    pub captures: usize,
    /// 取り込んだ値の元の型（lower は own を ref にして受け取る）
    pub cap_tys: Vec<Ty>,
    pub ret: Ty,
    pub locals: Vec<LocalDecl>,
    pub blocks: Vec<Block>,
    pub span: Span,
}

pub struct LocalDecl {
    /// ソースの名前。一時的な値は空
    pub name: String,
    pub ty: Ty,
    pub mutable: bool,
    pub span: Span,
}

#[derive(Clone)]
pub struct Block {
    pub insts: Vec<Inst>,
    pub term: Term,
}

#[derive(Debug, Clone)]
pub enum Inst {
    /// `場所 = 値`。値を先に計算してから場所を求める。場所に古い値があれば解放する（代入）。
    /// 場所が変数そのもので、まだ値がない（宣言・ムーブ済み）なら、新しく置く
    /// `what` は場所の説明（上書きで古い値を解放したときのエラー表示。interp の `describe`）
    Assign { place: Place, value: Rvalue, span: Span, what: String },
    /// 変数の寿命の終わり: 中の own をすべて解放し、変数の場所も解放する（世代が進み、変数を借りた ref は無効になる）。
    /// ムーブ済みなら何もしない（LLVM ではムーブの印を見る）
    Drop { local: Local, span: Span, why: String },
    /// 値を捨てる（中の own をすべて解放する）。式文の値・`drop(x)`
    Discard { value: Operand, span: Span, why: String },
}

#[derive(Debug, Clone)]
pub enum Term {
    Goto(Bb),
    Branch(Operand, Bb, Bb),
    Return(Operand),
    /// enum の種類で分かれる（場所の値の種類, (種類, 飛ぶ先), どれでもなければ）
    Switch(Place, Vec<(Rc<str>, Bb)>, Bb),
    /// `for i in range(..)` の次の周回: `counter += step`。型に入れば `next`、あふれたら `done`
    Step { counter: Local, step: Operand, next: Bb, done: Bb },
    /// 実行時エラー（題・位置・位置につける説明）
    Fail(String, Span, String),
    /// ここには来ない（return や break のあとの、どこからも飛んでこないブロック）
    Unreachable,
}

/// 変数から始めて、たどっていく場所。`span` は変数を書いた位置（ムーブ済みのエラー用）
#[derive(Debug, Clone)]
pub struct Place {
    pub local: Local,
    pub proj: Vec<Proj>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum Proj {
    /// own / ref の先。ref なら世代チェックをする（解析で消せたら `Check::Elided`）
    Deref(Check, Span),
    /// 構造体のフィールド（定義の順の番号）
    Field(usize, Span),
    /// enum の中身（`switch` で種類を確かめたあと）。種類の名前は表示用
    Variant(Rc<str>, usize),
    /// 配列・リストの添字、辞書のキー（範囲・キーがあるかを確かめる）。添字の値, 添字の位置, 配列の位置。
    /// 添字は場所をたどる前に計算しておく（interp と同じ順）
    Index(Operand, Span, Span),
}

#[derive(Debug, Clone)]
pub enum Operand {
    /// 読む（変数は空にならない）。コピーできる値か、借りて読むだけのとき（print など）
    Copy(Place),
    /// 読んで、変数は空になる（own・構造体・一時的な値）。空の変数を読めば「ムーブ済み」の実行時エラー
    Move(Local, Span),
    /// 値によって決める読み方（interp の `read_var` と同じ）: ムーブする値ならムーブ、そうでなければコピー。
    /// クロージャが取り込んだムーブする値なら実行時エラー。関数の値（取り込んだものしだい）と、取り込んだ変数に使う
    Read(Local, Span),
    Const(Const),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Const {
    Unit,
    Int(i128, IntTy),
    Float(f64),
    Bool(bool),
    Str(Rc<str>),
}

#[derive(Debug, Clone)]
pub enum Rvalue {
    Use(Operand),
    Bin(BinOp, Operand, Operand, Span),
    Un(UnOp, Operand, Span),
    /// 数の変換（`as` と、型検査が決めた自動の変換）。数でない値はそのまま
    Cast(Operand, Ty),
    /// `ref 場所`。own なら先を、ref なら同じ先を、それ以外はその場所を借りる。`what` はエラー表示用の名前。
    /// `follows` は場所の値が own / ref か（型から決めたもの。検査を消す解析が使う）
    Ref { place: Place, mutable: bool, follows: bool, what: String, span: Span },
    /// `own 値`
    Own(Operand),
    /// 構造体の値（フィールドは定義の順）
    Struct(String, Vec<Operand>),
    /// enum の値（種類, 中身, 値として読むとコピーか）
    Enum(Rc<str>, Vec<Operand>, bool),
    /// 場所の値を取り出し、そこを空（`()`）にする（match で enum の中身を受け取る名前へ移す）
    Take(Place),
    /// 配列（false）・リスト（true）
    Array(Vec<Operand>, bool),
    /// `[x; n]`
    Repeat(Operand, u64, bool),
    /// 辞書（キー, 値）。同じキーがあれば後のものが残り、前の値は捨てる
    Map(Vec<(Operand, Operand)>, Span),
    /// 場所にある配列・リスト・辞書の操作（`len`・`push`・`pop`・`remove`・`insert`・`sort`・`contains`・`get`・`keys`）。
    /// 場所の中をじかに書き換える。最後の Span は最初の引数の位置（範囲のエラー用）
    SeqOp { name: String, place: Place, args: Vec<Operand>, span: Span, arg_span: Span },
    /// `for x in a` が回す要素の写し（始めた時点の要素。中の own は読むだけの ref にする）。名前・位置は ref の表示用
    Items { place: Place, what: String, span: Span },
    /// `for (k, v) in m` の前の名前の一覧（リストなら添字、辞書ならキー）
    Keys(Place),
    /// 名前のある関数の呼び出し（関数の番号）
    Call(usize, Vec<Operand>, Span),
    /// interface を型として使った値のメソッド呼び出し。`this` の先の構造体の名前で、表から関数を選ぶ（vtable）。
    /// `this` は場所（先をたどったもの）で、引数を計算したあとで借りる
    CallDyn { table: Vec<(String, usize)>, this: Place, mutable: bool, what: String, args: Vec<Operand>, span: Span },
    /// 関数の値を作る（関数の番号, 取り込む値）
    Closure(usize, Vec<Operand>),
    /// 関数の値の呼び出し（関数の値, 引数）。取り込んだ値は、読むだけの写しにして最後の引数として渡す
    CallValue(Operand, Vec<Operand>, Span),
    /// 場所の値を読む。ムーブする値なら実行時エラー（「〜にある X は取り出せません」の〜）
    CopyOut(Place, String, Span),
    /// print と埋め込みで出す文字にする（own / ref は先をたどる）。bool が true なら、値は一時的なので見たあと捨てる
    Show(Operand, bool, Span),
    /// 組み込みの操作: `print`（文字を空白でつないで 1 行出す）・`concat`（文字をつなぐ）・`assert`
    Builtin(String, Vec<Operand>, Span),
}

impl Program {
    /// 外部状態にアクセスする組み込みを使うか。
    ///
    /// Debug の AST/IR 差分実行はプログラムを二度実行するため、ファイルの
    /// 読み書きや外部コマンドを含む場合は通常の実行経路から再実行しない。
    /// 新しい外部効果を持つ組み込みを追加する際はここも更新すること。
    pub fn uses_external_system(&self) -> bool {
        self.fns.iter().any(|f| {
            f.blocks.iter().any(|b| {
                b.insts.iter().any(|i| {
                    matches!(i, Inst::Assign {
                        value: Rvalue::Builtin(n, ..), ..
                    } if matches!(n.as_str(), "read_file" | "write_file" | "run_command"))
                })
            })
        })
    }

    /// 入力（`read_line`）を使うか
    pub fn uses_input(&self) -> bool {
        self.fns.iter().any(|f| {
            f.blocks.iter().any(|b| {
                b.insts.iter().any(|i| matches!(i, Inst::Assign { value: Rvalue::Builtin(n, ..), .. } if n == "read_line"))
            })
        })
    }
}

/// 上書きで古い値を解放したときの説明（interp と同じ）
pub fn overwritten(what: &str) -> String {
    if what.is_empty() {
        "上書きされて、古い値が解放された".to_string()
    } else {
        format!("`{}` が上書きされて、古い値が解放された", what)
    }
}

impl Place {
    pub fn local(l: Local, span: Span) -> Place {
        Place { local: l, proj: Vec::new(), span }
    }
}

// ---------------------------------------------------------------- 表示（`jabberwock --ir`）

impl fmt::Display for Program {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for (i, func) in self.fns.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{}", Show(func, self))?;
        }
        Ok(())
    }
}

struct Show<'a>(&'a Func, &'a Program);

impl<'a> fmt::Display for Show<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let Show(func, prog) = self;
        let params: Vec<String> = (0..func.params).map(|i| format!("_{}: {}", i, ty_of(&func.locals[i]))).collect();
        writeln!(f, "fn {}({}) -> {} {{", func.name, params.join(", "), func.ret.show())?;
        for (i, l) in func.locals.iter().enumerate().skip(func.params) {
            let name = if l.name.is_empty() { String::new() } else { format!("  // {}", l.name) };
            writeln!(f, "    let _{}: {};{}", i, ty_of(l), name)?;
        }
        for (i, b) in func.blocks.iter().enumerate() {
            writeln!(f, "  bb{}:", i)?;
            for inst in &b.insts {
                writeln!(f, "    {}", ShowInst(inst, prog))?;
            }
            writeln!(f, "    {}", show_term(&b.term))?;
        }
        writeln!(f, "}}")
    }
}

fn ty_of(l: &LocalDecl) -> String {
    l.ty.show()
}

struct ShowInst<'a>(&'a Inst, &'a Program);

impl<'a> fmt::Display for ShowInst<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.0 {
            Inst::Assign { place, value, .. } => write!(f, "{} = {}", show_place(place), show_rvalue(value, self.1)),
            Inst::Drop { local, .. } => write!(f, "drop _{}", local),
            Inst::Discard { value, .. } => write!(f, "discard {}", show_operand(value)),
        }
    }
}

fn show_term(t: &Term) -> String {
    match t {
        Term::Goto(b) => format!("goto bb{}", b),
        Term::Branch(c, t, e) => format!("branch {} bb{} bb{}", show_operand(c), t, e),
        Term::Return(o) => format!("return {}", show_operand(o)),
        Term::Switch(p, arms, d) => {
            let arms: Vec<String> = arms.iter().map(|(v, b)| format!("{} => bb{}", v, b)).collect();
            format!("switch {} [{}] それ以外 bb{}", show_place(p), arms.join(", "), d)
        }
        Term::Step { counter, step, next, done } => {
            format!("step _{} += {} bb{} (あふれたら bb{})", counter, show_operand(step), next, done)
        }
        Term::Fail(title, ..) => format!("fail {:?}", title),
        Term::Unreachable => "unreachable".to_string(),
    }
}

pub fn show_place(p: &Place) -> String {
    let mut s = format!("_{}", p.local);
    for pr in &p.proj {
        s = match pr {
            Proj::Deref(Check::Elided, _) => format!("(*{})", s),
            Proj::Deref(..) => format!("(*!{})", s),
            Proj::Field(i, _) => format!("{}.{}", s, i),
            Proj::Variant(v, i) => format!("({} as {}).{}", s, v, i),
            Proj::Index(k, ..) => format!("{}[{}]", s, show_operand(k)),
        };
    }
    s
}

fn show_operand(o: &Operand) -> String {
    match o {
        Operand::Copy(p) => show_place(p),
        Operand::Move(l, _) => format!("move _{}", l),
        Operand::Read(l, _) => format!("read _{}", l),
        Operand::Const(c) => match c {
            Const::Unit => "()".to_string(),
            Const::Int(n, t) => format!("{}{}", n, t.name()),
            Const::Float(x) => format!("{:?}", x),
            Const::Bool(b) => b.to_string(),
            Const::Str(s) => format!("{:?}", s),
        },
    }
}

fn show_rvalue(r: &Rvalue, prog: &Program) -> String {
    let ops = |v: &[Operand]| v.iter().map(show_operand).collect::<Vec<_>>().join(", ");
    match r {
        Rvalue::Use(o) => show_operand(o),
        Rvalue::Bin(op, a, b, _) => format!("{:?}({}, {})", op, show_operand(a), show_operand(b)),
        Rvalue::Un(op, a, _) => format!("{:?}({})", op, show_operand(a)),
        Rvalue::Cast(a, t) => format!("{} as {}", show_operand(a), t.show()),
        Rvalue::Ref { place, mutable: true, .. } => format!("ref mut {}", show_place(place)),
        Rvalue::Ref { place, .. } => format!("ref {}", show_place(place)),
        Rvalue::Own(a) => format!("own {}", show_operand(a)),
        Rvalue::Struct(n, fs) => format!("{} {{ {} }}", n, ops(fs)),
        Rvalue::Enum(v, fs, _) if fs.is_empty() => v.to_string(),
        Rvalue::Enum(v, fs, _) => format!("{}({})", v, ops(fs)),
        Rvalue::Take(p) => format!("take {}", show_place(p)),
        Rvalue::Array(items, false) => format!("[{}]", ops(items)),
        Rvalue::Array(items, true) => format!("list[{}]", ops(items)),
        Rvalue::Repeat(x, n, list) => format!("{}[{}; {}]", if *list { "list" } else { "" }, show_operand(x), n),
        Rvalue::Map(pairs, _) => {
            let ps: Vec<String> = pairs.iter().map(|(k, v)| format!("{}: {}", show_operand(k), show_operand(v))).collect();
            format!("map[{}]", ps.join(", "))
        }
        Rvalue::SeqOp { name, place, args, .. } => format!("{}.{}({})", show_place(place), name, ops(args)),
        Rvalue::Items { place, .. } => format!("items {}", show_place(place)),
        Rvalue::Keys(p) => format!("keys {}", show_place(p)),
        Rvalue::Call(i, args, _) => format!("{}({})", prog.fns[*i].name, ops(args)),
        Rvalue::CallDyn { this, args, table, .. } => {
            let names: Vec<&str> = table.iter().map(|(_, i)| prog.fns[*i].name.as_str()).collect();
            format!("dyn [{}](ref {}, {})", names.join(" | "), show_place(this), ops(args))
        }
        Rvalue::Closure(i, caps) => format!("closure {} [{}]", prog.fns[*i].name, ops(caps)),
        Rvalue::CallValue(f, args, _) => format!("call {}({})", show_operand(f), ops(args)),
        Rvalue::CopyOut(p, ..) => format!("copy_out {}", show_place(p)),
        Rvalue::Show(o, true, _) => format!("show!({}, 捨てる)", show_operand(o)),
        Rvalue::Show(o, false, _) => format!("show!({})", show_operand(o)),
        Rvalue::Builtin(n, args, _) => format!("{}!({})", n, ops(args)),
    }
}
