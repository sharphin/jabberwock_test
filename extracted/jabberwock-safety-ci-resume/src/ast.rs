use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::diag::Span;

/// ソースに書いた型。`i64`、`Player`、`own T`、`ref T`、`ref mut T`
#[derive(Debug, Clone)]
pub struct TypeExpr {
    pub kind: TypeKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum TypeKind {
    Name(String),
    /// `Pair<i64>`、`Option<T>`。型の引数つきの名前
    App(String, Vec<TypeExpr>),
    Own(Box<TypeExpr>),
    /// bool は `mut`
    Ref(Box<TypeExpr>, bool),
    /// `i64[3]`: 長さが決まった配列
    Array(Box<TypeExpr>, u64),
    /// `i64[]`: 伸び縮みするリスト
    List(Box<TypeExpr>),
    /// `i64[str]`: 辞書（値の型, キーの型）
    Map(Box<TypeExpr>, Box<TypeExpr>),
    /// `fn bool(i64, i64)`: 関数の型。戻り値がなければ None
    Fn(Vec<TypeExpr>, Option<Box<TypeExpr>>),
}

/// 整数の型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntTy {
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    /// 配列の添字などに使う符号なし。幅は 64 ビットだが、u64 とは別の型
    Usize,
}

impl IntTy {
    pub fn from_name(n: &str) -> Option<IntTy> {
        Some(match n {
            "i8" => IntTy::I8,
            "i16" => IntTy::I16,
            "i32" => IntTy::I32,
            "i64" => IntTy::I64,
            "u8" => IntTy::U8,
            "u16" => IntTy::U16,
            "u32" => IntTy::U32,
            "u64" => IntTy::U64,
            "usize" => IntTy::Usize,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            IntTy::I8 => "i8",
            IntTy::I16 => "i16",
            IntTy::I32 => "i32",
            IntTy::I64 => "i64",
            IntTy::U8 => "u8",
            IntTy::U16 => "u16",
            IntTy::U32 => "u32",
            IntTy::U64 => "u64",
            IntTy::Usize => "usize",
        }
    }

    pub fn bits(self) -> u32 {
        match self {
            IntTy::I8 | IntTy::U8 => 8,
            IntTy::I16 | IntTy::U16 => 16,
            IntTy::I32 | IntTy::U32 => 32,
            IntTy::I64 | IntTy::U64 | IntTy::Usize => 64,
        }
    }

    pub fn signed(self) -> bool {
        matches!(self, IntTy::I8 | IntTy::I16 | IntTy::I32 | IntTy::I64)
    }

    pub fn min(self) -> i128 {
        if self.signed() {
            -(1i128 << (self.bits() - 1))
        } else {
            0
        }
    }

    pub fn max(self) -> i128 {
        if self.signed() {
            (1i128 << (self.bits() - 1)) - 1
        } else {
            (1i128 << self.bits()) - 1
        }
    }

    pub fn contains(self, v: i128) -> bool {
        self.min() <= v && v <= self.max()
    }

    /// 範囲の外の値を一周させて範囲に入れる（あふれ・`as` での変換）
    pub fn wrap(self, v: i128) -> i128 {
        let m = 1i128 << self.bits();
        let u = v.rem_euclid(m);
        if self.signed() && u > self.max() {
            u - m
        } else {
            u
        }
    }

    /// 値が欠けずに `to` へ変換できるか（自動で変換してよいか）。
    /// usize は環境で幅が変わりうる型として扱い、u8 / u16 / u32 からだけ受け取る
    pub fn widens_to(self, to: IntTy) -> bool {
        if self == to {
            return true;
        }
        if self == IntTy::Usize || to == IntTy::Usize {
            return to == IntTy::Usize && matches!(self, IntTy::U8 | IntTy::U16 | IntTy::U32);
        }
        match (self.signed(), to.signed()) {
            (false, false) | (true, true) => self.bits() <= to.bits(),
            (false, true) => self.bits() < to.bits(),
            (true, false) => false,
        }
    }

    /// 二つの整数の演算をどの型でするか（片方へ欠けずに変換できるなら広い方）
    pub fn common(a: IntTy, b: IntTy) -> Option<IntTy> {
        if a.widens_to(b) {
            Some(b)
        } else if b.widens_to(a) {
            Some(a)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
    /// 整数の式なら、型検査が決めた「この式の値を使う型」。実行時はこの型に変換してから使う
    /// （数の書き方を相手の型に合わせる・欠けない変換を自動でする、のため）
    pub int: Cell<Option<IntTy>>,
    /// 型検査が「自動で借りる」と決めた引数（`ref T` の引数に `ref` なしで渡した場所）。
    /// interp と analyze は `ref e` と同じに扱う
    pub borrow: Cell<bool>,
    /// 型検査が「この整数の書き方は小数（f64）として使う」と決めた（`let f64 x = 1;` の `1`）
    pub float: Cell<bool>,
    /// 型検査が「値として読むと own を取り出すことになる場所」と決めた `p.f` / `a[i]` / `*p`（型が own か、own の配列）。
    /// interp はこれを値として読むとエラーにする（中身が for の要素などの読むだけの写しでも。写しは own の型で外へ出せない）
    pub takes_own: Cell<bool>,
}

impl Expr {
    pub fn new(kind: ExprKind, span: Span) -> Expr {
        Expr { kind, span, int: Cell::new(None), borrow: Cell::new(false), float: Cell::new(false), takes_own: Cell::new(false) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
    /// `~x`: ビットを反転する（整数だけ）
    BitNot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    /// ビット演算（整数だけ）。`<<` `>>` の右は ずらす数
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

/// `*p` で ref の世代チェックが要るかどうか。静的解析（`analyze`）が書き込む。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    /// 解析がまだ見ていない（実行時に検査する）
    Unvisited,
    /// 「借りた先は必ず生きている」と証明できた。実行時の検査を消す
    Elided,
    /// 証明できなかった。実行時に検査する
    Required,
}

/// 型検査が決めた、メソッド呼び出しの相手
#[derive(Debug, Clone, Default)]
pub struct MethodTarget {
    /// receiver が enum なら、その enum の名前（enum の値は型の名前を持たないので、interp がメソッドを探すのに使う）
    pub enum_name: Option<String>,
    /// `fn take` のメソッド: receiver を値として読む（ムーブする）
    pub take: bool,
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    Int(i128),
    Float(f64),
    Bool(bool),
    Str(String),
    /// `()`: 値なしの値（`Ok(())` のように型の引数が `()` の場所に書く）
    Unit,
    Var(String),
    /// `e as T`。整数の型を明示的に変える（範囲の外は一周する）
    Cast(Box<Expr>, TypeExpr),
    /// `e?`: e が `Err(x)` / `None` なら、関数からそれをそのまま return する。そうでなければ中身
    Try(Box<Expr>),
    /// `Shape.Circle(5)` / `Shape.Empty` / `Some(x)` / `None`。enum の値を作る。
    /// Option の enum 名は "Option"。`copy` は型検査が決める「値として読むとコピーか」
    /// （中に own や構造体を持ちうる enum は、構造体と同じく読むとムーブ）。
    /// None は型の引数を含んで決められないとき（実行時に中身を見て決める）
    Variant { enum_name: String, variant: String, args: Vec<Expr>, copy: Cell<Option<bool>> },
    /// `own e` : ヒープに値を置き、唯一の所有者を返す
    Own(Box<Expr>),
    /// `ref p` / `ref mut p` : 所有者（変数か `*p` か `p.f`）を世代付き参照として借りる。
    /// bool は `mut`（書き換えられる ref か）
    Ref(Box<Expr>, bool),
    /// `*p` : own / ref の先を読む。2つ目は静的解析の結論（世代チェックを消せるか）。
    Deref(Box<Expr>, Cell<Check>),
    /// `e.f` : フィールド。`e` が own / ref なら自動で先をたどる（そのときの検査が `Check`）。
    Field(Box<Expr>, String, Cell<Check>),
    /// `Name { f: e, ... }` : 構造体リテラル。フィールドはソースに書いた順
    StructLit(String, Vec<FieldInit>),
    /// `if c { ... } else { ... }`。文としても値としても使える。値はブロックの最後の行。
    /// 値として使う位置では else が必須（パーサが確かめる）。
    If { cond: Box<Expr>, then: Block, els: Option<Block> },
    /// `match x { case 1, 2 -> ... default -> ... }`（Java の switch の矢印形式）。
    /// 上から順に調べ、当てはまる case がなければ default。値として使う位置では default が必須。
    Match { scrut: Box<Expr>, arms: Vec<MatchArm>, default: Option<Block> },
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
    /// `a[i]`: 配列の要素。`a` が own / ref なら自動で先をたどる（そのときの検査が `Check`）
    Index(Box<Expr>, Box<Expr>, Cell<Check>),
    /// `[1, 2, 3]`
    /// 何として作るか（入れる先の型が `T[]` ならリスト、空の `[]` で入れる先が辞書なら辞書。型検査が書く）
    ArrayLit(Vec<Expr>, Cell<Seq>),
    /// `["a": 1, "b": 2]`: 辞書（キー, 値）
    MapLit(Vec<(Expr, Expr)>),
    /// `[0; 5]`: 同じ値を並べた配列（値はコピーできる型だけ）
    ArrayRepeat(Box<Expr>, u64, Cell<bool>),
    /// `"x = {x}"`: 埋め込みのある文字列。式は print と同じく借りて読む（ムーブしない）
    Interp(Vec<InterpPart>),
    /// `(a, b) -> a < b`: その場で作る関数（クロージャ）
    Lambda(Rc<Lambda>),
    /// `p.name(args)` : メソッド呼び出し。メソッドは `構造体名.name` という名前の関数で、
    /// 最初の引数 `this` に p を借りて渡す（p が own / ref ならその先、構造体の値なら一時的にヒープへ移して借りる）
    /// 4 つめ: 型検査が書く、呼ぶメソッドのこと（interp / analyze が使う）
    MethodCall(Box<Expr>, String, Vec<Expr>, RefCell<MethodTarget>),
}

#[derive(Debug, Clone)]
pub enum InterpPart {
    Lit(String),
    Expr(Expr),
}

#[derive(Debug, Clone)]
pub struct FieldInit {
    pub name: String,
    pub span: Span,
    pub value: Expr,
}

/// `case` に書ける値（今のところリテラルだけ）
#[derive(Debug, Clone, PartialEq)]
pub enum Pat {
    Int(i128),
    Bool(bool),
    Str(String),
    /// `Circle(r, _)` / `None`。enum の種類と、中身を受け取る名前（`_` は受け取らない）。
    /// 3 つめは中身ごとの重ねた形（`Some(Op(op))` の `Op(op)`・`Some(0)` の `0`）。重ねた中身の名前は `_`
    Variant(String, Vec<(String, Span)>, Vec<Option<Pat>>),
}

impl Pat {
    /// 先に書いた case `self` があると、`later` が決して選ばれないか（受け取る名前は問わない）
    pub fn same_case(&self, later: &Pat) -> bool {
        match (self, later) {
            (Pat::Variant(a, _, subs), Pat::Variant(b, ..)) => a == b && subs.iter().all(Option::is_none),
            _ => self == later,
        }
    }

    /// 重ねた形がない（種類が合えば必ず当たる）
    pub fn is_flat(&self) -> bool {
        match self {
            Pat::Variant(_, _, subs) => subs.iter().all(Option::is_none),
            _ => true,
        }
    }

    /// 受け取る名前すべて（重ねた形の中も。`_` は除く）
    pub fn names(&self) -> Vec<(String, Span)> {
        let mut out = Vec::new();
        if let Pat::Variant(_, binds, subs) = self {
            for (b, s) in binds.iter().zip(subs) {
                match s {
                    Some(p) => out.extend(p.names()),
                    None if b.0 != "_" => out.push(b.clone()),
                    None => {}
                }
            }
        }
        out
    }
}

#[derive(Debug, Clone)]
pub struct MatchArm {
    pub pats: Vec<(Pat, Span)>,
    /// `-> 式` も `-> { ... }` もブロックにそろえる。値はブロックの最後の行
    pub body: Block,
}

#[derive(Debug, Clone)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    /// 最後の文が `;` のない式で、ブロックの値になるか（Rust と同じ規則）
    pub tail: bool,
    /// 閉じ括弧 `}` の位置。スコープ終了での解放を指し示すのに使う。
    pub end: Span,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    /// `let 型 mut 名前 = 式`。型は省略できる（初期値から決める）
    Let { name: String, ty: Option<TypeExpr>, mutable: bool, span: Span, init: Expr },
    Assign { target: Expr, value: Expr, span: Span },
    /// 式文。`if` もここに入る（文として使う if）
    Expr(Expr),
    While { cond: Expr, body: Block },
    Loop { body: Block },
    /// `for i in range(a, b, step) { ... } else { ... }`。else は 1 回も回らなかったときに実行
    For { var: String, range: Vec<Expr>, range_span: Span, body: Block, els: Option<Block> },
    /// `for x in a { ... } else { ... }`: 配列・リストの要素を回す（a は借りるだけ。ループを始めた時点の要素を回す）
    /// `for x in a` / `for (k, v) in m`（`key` は組の前の名前。リストなら添字、辞書ならキー）
    /// `consume`: その場で作ったリストを回す（要素を値として受け取る。型検査が書く）
    ForEach {
        key: Option<(String, Span)>,
        var: String,
        var_span: Span,
        iter: Expr,
        body: Block,
        els: Option<Block>,
        consume: Cell<bool>,
    },
    Break(Span),
    Continue(Span),
    Return { value: Option<Expr>, span: Span },
}

#[derive(Debug, Clone)]
pub struct Param {
    pub ty: TypeExpr,
    pub name: String,
    pub mutable: bool,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct FnDef {
    pub name: String,
    /// 型の引数（`fn T max<T>(...)` の `T`）
    pub tparams: Vec<String>,
    /// 型の引数ごとの条件（`<T: Shape + ordered>` の `Shape`, `ordered`）。`tparams` と同じ順
    pub bounds: Vec<Vec<String>>,
    /// 戻り値の型。なければ値を返さない関数
    pub ret: Option<TypeExpr>,
    pub params: Vec<Param>,
    pub body: Block,
    pub span: Span,
}

/// `struct Name { i64 a; own Point b; }`
#[derive(Debug, Clone)]
pub struct StructDef {
    pub name: String,
    pub tparams: Vec<String>,
    pub fields: Vec<String>,
    /// `fields` と同じ順のフィールドの型
    pub field_types: Vec<TypeExpr>,
    pub span: Span,
}

impl StructDef {
    pub fn field_index(&self, name: &str) -> Option<usize> {
        self.fields.iter().position(|f| f == name)
    }
}

/// `enum Shape { Circle(i64 r); Rect(i64 w, i64 h); Empty; }`
#[derive(Debug, Clone)]
pub struct EnumDef {
    pub name: String,
    pub tparams: Vec<String>,
    pub variants: Vec<VariantDef>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct VariantDef {
    pub name: String,
    /// 中身（型 名前）
    pub fields: Vec<(TypeExpr, String)>,
    pub span: Span,
}

impl EnumDef {
    pub fn variant(&self, name: &str) -> Option<&VariantDef> {
        self.variants.iter().find(|v| v.name == name)
    }
}

/// `interface Shape { fn i64 area(); fn mut grow(i64 n); }`（Go 風: 構造体は宣言なしで当てはまる）
#[derive(Debug, Clone)]
pub struct InterfaceDef {
    pub name: String,
    /// メソッドの形（本体は空）と、`fn mut` か
    pub methods: Vec<(FnDef, bool)>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Program {
    pub structs: Vec<StructDef>,
    pub enums: Vec<EnumDef>,
    pub interfaces: Vec<InterfaceDef>,
    pub fns: Vec<FnDef>,
    /// `use geom;` で使うモジュールの名前と位置（このファイルのもの）
    pub uses: Vec<(String, Span)>,
    /// `pub` をつけた名前（このファイルのトップレベルの、モジュール名をつける前の名前）
    pub public: Vec<String>,
    /// 読み込んだすべてのモジュールの名前（全体をまとめたプログラムだけが持つ）
    pub modules: Vec<String>,
    /// `const` の名前（使う場所はパーサが値に置き換え済み。モジュールの中のものは `geom.N`）。変数に同じ名前をつけるのを型検査が弾く
    pub consts: Vec<String>,
}

/// `[...]` を何として作るか
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seq {
    Array,
    List,
    Map,
}

/// その場で作る関数 `(a, b) -> 式` / `(i64 a) -> { ... }`
#[derive(Debug)]
pub struct Lambda {
    pub params: Vec<LambdaParam>,
    /// 本体。式で書いたときは、その式だけの（値を返す）ブロック
    pub body: Block,
    /// 取り込む外の変数の名前（型検査が本体を調べて決める）。作るときにこの順で値として読む
    pub captures: RefCell<Vec<String>>,
    /// 取り込みごとに、借りるか（true なら作るときに `ref 名前` を取る。型検査が書く）
    pub borrowed: RefCell<Vec<bool>>,
    /// 関数・メソッドの引数にその場で書いたクロージャか（パーサが書く）。ムーブする外の変数を借りて取り込む
    pub borrows: Cell<bool>,
    /// 値を返すか（戻り値の型が () でない。型検査が書く）
    pub returns: Cell<bool>,
}

#[derive(Debug)]
pub struct LambdaParam {
    /// 書かなければ、渡す先の関数の型から決める
    pub ty: Option<TypeExpr>,
    pub name: String,
    pub span: Span,
}

// ---------------------------------------------------------------- 走査

/// ブロックの中の式を、外から順にすべてたどる。`f` が false を返したら、その式の中には入らない
pub fn walk_block(b: &Block, f: &mut dyn FnMut(&Expr) -> bool) {
    for s in &b.stmts {
        walk_stmt(s, f);
    }
}

pub fn walk_stmt(s: &Stmt, f: &mut dyn FnMut(&Expr) -> bool) {
    match s {
        Stmt::Let { init, .. } => walk_expr(init, f),
        Stmt::Assign { target, value, .. } => {
            walk_expr(target, f);
            walk_expr(value, f);
        }
        Stmt::Expr(e) => walk_expr(e, f),
        Stmt::While { cond, body } => {
            walk_expr(cond, f);
            walk_block(body, f);
        }
        Stmt::Loop { body } => walk_block(body, f),
        Stmt::For { range, body, els, .. } => {
            range.iter().for_each(|a| walk_expr(a, f));
            walk_block(body, f);
            if let Some(els) = els {
                walk_block(els, f);
            }
        }
        Stmt::ForEach { iter, body, els, .. } => {
            walk_expr(iter, f);
            walk_block(body, f);
            if let Some(els) = els {
                walk_block(els, f);
            }
        }
        Stmt::Break(_) | Stmt::Continue(_) => {}
        Stmt::Return { value, .. } => {
            if let Some(e) = value {
                walk_expr(e, f);
            }
        }
    }
}

pub fn walk_expr(e: &Expr, f: &mut dyn FnMut(&Expr) -> bool) {
    if !f(e) {
        return;
    }
    match &e.kind {
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::Str(_) | ExprKind::Unit | ExprKind::Var(_) => {}
        ExprKind::Cast(x, _) | ExprKind::Try(x) | ExprKind::Own(x) | ExprKind::Ref(x, _) | ExprKind::Deref(x, _) => walk_expr(x, f),
        ExprKind::Field(x, ..) | ExprKind::Unary(_, x) | ExprKind::ArrayRepeat(x, ..) => walk_expr(x, f),
        ExprKind::Variant { args, .. } | ExprKind::Call(_, args) | ExprKind::ArrayLit(args, _) => {
            args.iter().for_each(|a| walk_expr(a, f))
        }
        ExprKind::StructLit(_, inits) => inits.iter().for_each(|i| walk_expr(&i.value, f)),
        ExprKind::If { cond, then, els } => {
            walk_expr(cond, f);
            walk_block(then, f);
            if let Some(els) = els {
                walk_block(els, f);
            }
        }
        ExprKind::Match { scrut, arms, default } => {
            walk_expr(scrut, f);
            arms.iter().for_each(|a| walk_block(&a.body, f));
            if let Some(d) = default {
                walk_block(d, f);
            }
        }
        ExprKind::Binary(_, l, r) | ExprKind::Index(l, r, _) => {
            walk_expr(l, f);
            walk_expr(r, f);
        }
        ExprKind::MapLit(pairs) => {
            for (k, v) in pairs {
                walk_expr(k, f);
                walk_expr(v, f);
            }
        }
        ExprKind::Interp(parts) => {
            for p in parts {
                if let InterpPart::Expr(x) = p {
                    walk_expr(x, f);
                }
            }
        }
        ExprKind::Lambda(l) => walk_block(&l.body, f),
        ExprKind::MethodCall(recv, _, args, _) => {
            walk_expr(recv, f);
            args.iter().for_each(|a| walk_expr(a, f));
        }
    }
}

/// `walk_block` の書き換えられる版（パーサが `const` の名前を値に置き換えるのに使う）
pub fn walk_block_mut(b: &mut Block, f: &mut dyn FnMut(&mut Expr) -> bool) {
    for s in &mut b.stmts {
        walk_stmt_mut(s, f);
    }
}

pub fn walk_stmt_mut(s: &mut Stmt, f: &mut dyn FnMut(&mut Expr) -> bool) {
    match s {
        Stmt::Let { init, .. } => walk_expr_mut(init, f),
        Stmt::Assign { target, value, .. } => {
            walk_expr_mut(target, f);
            walk_expr_mut(value, f);
        }
        Stmt::Expr(e) => walk_expr_mut(e, f),
        Stmt::While { cond, body } => {
            walk_expr_mut(cond, f);
            walk_block_mut(body, f);
        }
        Stmt::Loop { body } => walk_block_mut(body, f),
        Stmt::For { range, body, els, .. } => {
            range.iter_mut().for_each(|a| walk_expr_mut(a, f));
            walk_block_mut(body, f);
            if let Some(els) = els {
                walk_block_mut(els, f);
            }
        }
        Stmt::ForEach { iter, body, els, .. } => {
            walk_expr_mut(iter, f);
            walk_block_mut(body, f);
            if let Some(els) = els {
                walk_block_mut(els, f);
            }
        }
        Stmt::Break(_) | Stmt::Continue(_) => {}
        Stmt::Return { value, .. } => {
            if let Some(e) = value {
                walk_expr_mut(e, f);
            }
        }
    }
}

pub fn walk_expr_mut(e: &mut Expr, f: &mut dyn FnMut(&mut Expr) -> bool) {
    if !f(e) {
        return;
    }
    match &mut e.kind {
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::Str(_) | ExprKind::Unit | ExprKind::Var(_) => {}
        ExprKind::Cast(x, _) | ExprKind::Try(x) | ExprKind::Own(x) | ExprKind::Ref(x, _) | ExprKind::Deref(x, _) => walk_expr_mut(x, f),
        ExprKind::Field(x, ..) | ExprKind::Unary(_, x) | ExprKind::ArrayRepeat(x, ..) => walk_expr_mut(x, f),
        ExprKind::Variant { args, .. } | ExprKind::Call(_, args) | ExprKind::ArrayLit(args, _) => {
            args.iter_mut().for_each(|a| walk_expr_mut(a, f))
        }
        ExprKind::StructLit(_, inits) => inits.iter_mut().for_each(|i| walk_expr_mut(&mut i.value, f)),
        ExprKind::If { cond, then, els } => {
            walk_expr_mut(cond, f);
            walk_block_mut(then, f);
            if let Some(els) = els {
                walk_block_mut(els, f);
            }
        }
        ExprKind::Match { scrut, arms, default } => {
            walk_expr_mut(scrut, f);
            arms.iter_mut().for_each(|a| walk_block_mut(&mut a.body, f));
            if let Some(d) = default {
                walk_block_mut(d, f);
            }
        }
        ExprKind::Binary(_, l, r) | ExprKind::Index(l, r, _) => {
            walk_expr_mut(l, f);
            walk_expr_mut(r, f);
        }
        ExprKind::MapLit(pairs) => {
            for (k, v) in pairs {
                walk_expr_mut(k, f);
                walk_expr_mut(v, f);
            }
        }
        ExprKind::Interp(parts) => {
            for p in parts {
                if let InterpPart::Expr(x) = p {
                    walk_expr_mut(x, f);
                }
            }
        }
        ExprKind::Lambda(l) => walk_block_mut(&mut Rc::get_mut(l).expect("lambda is not shared while parsing").body, f),
        ExprKind::MethodCall(recv, _, args, _) => {
            walk_expr_mut(recv, f);
            args.iter_mut().for_each(|a| walk_expr_mut(a, f));
        }
    }
}
