use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::*;
use crate::diag::{Diagnostic, Span};
use crate::lexer::{StrPart, Tok, Token};

type R<T> = Result<T, Diagnostic>;

/// モジュールのトップレベルの名前の種類
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameKind {
    Fn,
    Struct,
    Enum,
    Interface,
}

/// 使う側から見たモジュール: トップレベルの名前ごとに、種類と `pub` か
#[derive(Debug, Clone, Default)]
pub struct ModuleInfo {
    pub names: HashMap<String, (NameKind, bool)>,
    /// enum ごとの種類の名前（`geom.Shape.make()` が種類か静的メソッドかを見分ける）
    pub variants: HashMap<String, Vec<String>>,
}

pub struct Parser {
    toks: Vec<Token>,
    pos: usize,
    /// `if` / `while` の条件の中では `Name {` を構造体リテラルと読まない（ブロックの `{` と区別するため）
    no_struct_lit: bool,
    /// いくつのループの中にいるか（break / continue を許すか）。値として使う if に入ると 0 に戻す
    loop_depth: usize,
    /// 値として使う if / match の中か（return / 外への break を禁止する）
    in_value_if: bool,
    /// ファイル中で定義されている enum の名前。`Name.Variant` を enum の値として読むため
    enum_names: Vec<String>,
    /// ファイル中で定義されている構造体の名前。`Name.f(...)` を静的メソッドの呼び出しとして読むため
    struct_names: Vec<String>,
    /// ファイル中の enum ごとの種類の名前。`Shape.make()` が種類でなければ静的メソッドの呼び出し
    enum_variants: HashMap<String, Vec<String>>,
    /// 今の入れ子の深さ（式・ブロック・型・形）。`MAX_NEST` を超えたらエラー（深い入れ子でスタックがあふれないように）
    depth: usize,
}

/// 入れ子の深さの上限。あとの段（型検査・実行・書き出し）も入れ子をたどる再帰なので、ここで止める
pub const MAX_NEST: usize = 256;

impl Parser {
    pub fn new(toks: Vec<Token>) -> Self {
        let mut enum_names: Vec<String> = toks
            .windows(2)
            .filter_map(|w| match (&w[0].tok, &w[1].tok) {
                (Tok::Enum, Tok::Ident(n)) => Some(n.clone()),
                _ => None,
            })
            .collect();
        let struct_names: Vec<String> = toks
            .windows(2)
            .filter_map(|w| match (&w[0].tok, &w[1].tok) {
                (Tok::Struct, Tok::Ident(n)) => Some(n.clone()),
                _ => None,
            })
            .collect();
        // 組み込みの enum（prelude で定義）
        enum_names.push("Option".to_string());
        enum_names.push("Result".to_string());
        let enum_variants = scan_variants(&toks);
        Parser { toks, pos: 0, no_struct_lit: false, loop_depth: 0, in_value_if: false, enum_names, struct_names, enum_variants, depth: 0 }
    }

    /// `use` したモジュールのあるファイルを読む。`geom.area` は 1 つの名前 `geom.area` にまとめる
    pub fn with_modules(toks: Vec<Token>, deps: &HashMap<String, ModuleInfo>) -> R<Self> {
        let toks = merge_paths(toks, deps)?;
        let mut p = Parser::new(toks);
        for (m, info) in deps {
            for (n, (kind, public)) in &info.names {
                if !public {
                    continue;
                }
                match kind {
                    NameKind::Struct => p.struct_names.push(format!("{}.{}", m, n)),
                    NameKind::Enum => {
                        p.enum_names.push(format!("{}.{}", m, n));
                        if let Some(vs) = info.variants.get(n) {
                            p.enum_variants.insert(format!("{}.{}", m, n), vs.clone());
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(p)
    }

    /// 入れ子を 1 段深くして f を読む
    fn nested<T>(&mut self, f: impl FnOnce(&mut Self) -> R<T>) -> R<T> {
        self.deeper()?;
        let r = f(self);
        self.depth -= 1;
        r
    }

    /// 入れ子を 1 段深くする。演算子・`as`・`.f` / `[i]` / `?` の鎖も 1 つごとに 1 段（構文木が深くなるため）。
    /// 鎖を読み終えたら、呼んだ側が深さを戻す
    fn deeper(&mut self) -> R<()> {
        if self.depth >= MAX_NEST {
            return Err(Diagnostic::new("入れ子が深すぎます")
                .primary(self.span(), format!("ここで {} 段を超えた", MAX_NEST))
                .note("式・ブロック・型・形の入れ子を合わせて数えます（演算子や `.` の鎖も 1 つごとに数えます）。途中を関数や変数に分けてください"));
        }
        self.depth += 1;
        Ok(())
    }

    fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }

    /// 1 つ先のトークン
    fn peek2(&self) -> &Tok {
        &self.toks[(self.pos + 1).min(self.toks.len() - 1)].tok
    }

    fn span(&self) -> Span {
        self.toks[self.pos].span
    }

    fn prev_span(&self) -> Span {
        self.toks[self.pos.saturating_sub(1)].span
    }

    fn bump(&mut self) -> Token {
        let t = self.toks[self.pos].clone();
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }

    /// 空の文（`;` だけ）を読み飛ばす
    fn skip_semis(&mut self) {
        while *self.peek() == Tok::Semi {
            self.bump();
        }
    }

    fn unexpected(&self, expected: &str) -> Diagnostic {
        Diagnostic::new(format!("構文エラー: {}が必要です", expected))
            .primary(self.span(), format!("ここに {} があります", self.peek().describe()))
    }

    fn expect(&mut self, t: Tok, expected: &str) -> R<Span> {
        if *self.peek() == t {
            Ok(self.bump().span)
        } else {
            Err(self.unexpected(expected))
        }
    }

    /// 次が `t` なら読んで true
    fn eat(&mut self, t: Tok) -> bool {
        if *self.peek() == t {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect_ident(&mut self, what: &str) -> R<String> {
        if let Tok::Ident(name) = self.peek().clone() {
            self.bump();
            Ok(name)
        } else {
            Err(self.unexpected(what))
        }
    }

    pub fn parse_program(&mut self) -> R<Program> {
        let mut fns = Vec::new();
        let mut structs = Vec::new();
        let mut enums = Vec::new();
        let mut interfaces = Vec::new();
        let mut uses: Vec<(String, Span)> = Vec::new();
        let mut public = Vec::new();
        let mut consts: Vec<(String, Expr)> = Vec::new();
        loop {
            self.skip_semis();
            if *self.peek() == Tok::Use {
                // `use geom;`: 同じフォルダの geom.jw を使う
                self.bump();
                let span = self.span();
                let name = self.expect_ident("モジュールの名前")?;
                self.expect(Tok::Semi, "`;`")?;
                if uses.iter().any(|(n, _)| *n == name) {
                    return Err(Diagnostic::new(format!("`use {}` が二回あります", name)).primary(span, "ここが二回目"));
                }
                uses.push((name, span));
                continue;
            }
            // `pub` は、ほかのファイルから使えるようにする印
            let is_pub = self.eat(Tok::Pub);
            match self.peek() {
                Tok::Const if is_pub => {
                    return Err(Diagnostic::new("const に pub はつけられません")
                        .primary(self.span(), "定数はそのファイルの中だけで使えます"))
                }
                Tok::Const => {
                    // `const N = 5000;`: 使う場所に値をそのまま書いたのと同じ（数の型は使う場所で決まる）
                    self.bump();
                    let span = self.span();
                    let name = self.expect_ident("定数の名前")?;
                    self.expect(Tok::Assign, "`=`")?;
                    let value = self.parse_expr()?;
                    self.expect(Tok::Semi, "`;`")?;
                    if consts.iter().any(|(n, _)| *n == name) {
                        return Err(Diagnostic::new(format!("定数 `{}` が二重に定義されています", name)).primary(span, "ここが二回目"));
                    }
                    consts.push((name, value));
                }
                Tok::Eof if is_pub => return Err(self.unexpected("`fn`・`struct`・`enum`・`interface`")),
                Tok::Eof => break,
                Tok::Interface => {
                    let i = self.parse_interface()?;
                    if is_pub {
                        public.push(i.name.clone());
                    }
                    interfaces.push(i);
                }
                Tok::Struct => {
                    let (st, methods) = self.parse_struct()?;
                    if is_pub {
                        public.push(st.name.clone());
                    }
                    structs.push(st);
                    fns.extend(methods);
                }
                Tok::Enum => {
                    let (e, methods) = self.parse_enum()?;
                    if is_pub {
                        public.push(e.name.clone());
                    }
                    enums.push(e);
                    fns.extend(methods);
                }
                _ => {
                    let f = self.parse_fn()?;
                    if is_pub {
                        public.push(f.name.clone());
                    }
                    fns.push(f);
                }
            }
        }
        let names = resolve_consts(&mut consts, &mut fns)?;
        Ok(Program { structs, enums, interfaces, fns, uses, public, modules: Vec::new(), consts: names })
    }

    /// `interface Shape { fn i64 area(); fn mut grow(i64 n); }`
    fn parse_interface(&mut self) -> R<InterfaceDef> {
        let start = self.expect(Tok::Interface, "`interface`")?;
        let name = self.expect_ident("interface の名前")?;
        let span = start.join(self.prev_span());
        if *self.peek() == Tok::Lt {
            return Err(Diagnostic::new("interface には型の引数をつけられません").primary(self.span(), "ここ"));
        }
        self.expect(Tok::LBrace, "`{`")?;
        let mut methods: Vec<(FnDef, bool)> = Vec::new();
        loop {
            self.skip_semis();
            if self.eat(Tok::RBrace) {
                break;
            }
            let mspan = self.span();
            let fstart = self.expect(Tok::Fn, "`fn`（interface にはメソッドの形だけ書けます）")?;
            let mutable = self.eat(Tok::Mut);
            let m = self.parse_fn_rest(fstart, false)?;
            if !m.tparams.is_empty() {
                return Err(Diagnostic::new("interface のメソッドには型の引数をつけられません").primary(m.span, "ここ"));
            }
            if methods.iter().any(|(x, _)| x.name == m.name) {
                return Err(Diagnostic::new(format!("メソッド `{}` が二重に書かれています", m.name))
                    .primary(mspan, "同じ名前のメソッドがすでにある"));
            }
            methods.push((m, mutable));
        }
        Ok(InterfaceDef { name, methods, span })
    }

    /// `enum Shape { Circle(i64 r); Rect(i64 w, i64 h); Empty; }`。種類のあとにメソッドを書ける（構造体と同じ）
    fn parse_enum(&mut self) -> R<(EnumDef, Vec<FnDef>)> {
        let start = self.expect(Tok::Enum, "`enum`")?;
        let name = self.expect_ident("enum の名前")?;
        let span = start.join(self.prev_span());
        let tparams = self.parse_tparams()?;
        self.expect(Tok::LBrace, "`{`")?;
        let mut variants: Vec<VariantDef> = Vec::new();
        let mut methods: Vec<(FnDef, Span)> = Vec::new();
        let mut statics: Vec<(FnDef, Span)> = Vec::new();
        loop {
            self.skip_semis();
            if self.eat(Tok::RBrace) {
                break;
            }
            if matches!(self.peek(), Tok::Fn | Tok::Static) {
                self.parse_method(&name, &tparams, &mut methods, &mut statics)?;
                continue;
            }
            if !methods.is_empty() || !statics.is_empty() {
                return Err(Diagnostic::new("種類はメソッドより前に書いてください")
                    .primary(self.span(), "メソッドのあとに種類がある"));
            }
            let vspan = self.span();
            let vname = self.expect_ident("種類の名前")?;
            if variants.iter().any(|v| v.name == vname) {
                return Err(Diagnostic::new(format!("種類 `{}` が二重に定義されています", vname))
                    .primary(vspan, "同じ名前の種類がすでにある"));
            }
            let mut fields: Vec<(TypeExpr, String)> = Vec::new();
            if self.eat(Tok::LParen) {
                while *self.peek() != Tok::RParen {
                    if matches!(self.peek(), Tok::Ident(_)) && matches!(self.peek2(), Tok::Comma | Tok::RParen) {
                        return Err(Diagnostic::new("中身には型と名前が必要です")
                            .primary(self.span(), "型か名前がない")
                            .note("`Circle(i64 r)` のように「型 名前」で書いてください"));
                    }
                    let ty = self.parse_type()?;
                    let fspan = self.span();
                    let f = self.expect_ident("中身の名前")?;
                    if fields.iter().any(|(_, n)| *n == f) {
                        return Err(Diagnostic::new(format!("中身の名前 `{}` が二回あります", f))
                            .primary(fspan, "同じ名前がすでにある"));
                    }
                    fields.push((ty, f));
                    if !self.eat(Tok::Comma) {
                        break;
                    }
                }
                self.expect(Tok::RParen, "`)`")?;
            }
            variants.push(VariantDef { name: vname, fields, span: vspan });
            if *self.peek() == Tok::Comma {
                return Err(Diagnostic::new("構文エラー: 種類の区切りは `;` です")
                    .primary(self.span(), "ここを `;` にしてください"));
            }
            if *self.peek() != Tok::RBrace {
                self.expect(Tok::Semi, "`;`（種類の終わり）")?;
            }
        }
        let vnames: Vec<String> = variants.iter().map(|v| v.name.clone()).collect();
        let fns = finish_methods(&name, &[], &vnames, methods, statics)?;
        Ok((EnumDef { name, tparams, variants, span }, fns))
    }

    /// 構造体・enum の中の `fn` / `fn mut` / `static fn` 1 つ。`this`（`ref 型名<T..>`）を最初の引数に足す
    fn parse_method(
        &mut self,
        name: &str,
        tparams: &[String],
        methods: &mut Vec<(FnDef, Span)>,
        statics: &mut Vec<(FnDef, Span)>,
    ) -> R<()> {
        let mspan = self.span();
        let is_static = self.eat(Tok::Static);
        let start = self.expect(Tok::Fn, "`fn`")?;
        let mutable = !is_static && self.eat(Tok::Mut);
        // `fn take`: this を値で受け取る（呼び出し元からムーブする）。`fn take(...)` は take という名前のメソッド
        let take = !is_static
            && !mutable
            && matches!(self.peek(), Tok::Ident(t) if t == "take")
            && *self.peek2() != Tok::LParen;
        if take {
            self.bump();
        }
        let mut m = self.parse_fn_rest(start, true)?;
        if methods.iter().chain(statics.iter()).any(|(x, _)| x.name == m.name) {
            return Err(Diagnostic::new(format!("メソッド `{}` が二重に定義されています", m.name))
                .primary(mspan, "同じ名前のメソッドがすでにある"));
        }
        if let Some(t) = m.tparams.iter().find(|t| tparams.contains(t)) {
            return Err(Diagnostic::new(format!("型の引数 `{}` は型のものと同じ名前です", t))
                .primary(m.span, "別の名前にしてください"));
        }
        if is_static {
            // 静的メソッドは this のない関数。型の引数は使える
            m.tparams = tparams.iter().cloned().chain(m.tparams).collect();
            m.bounds = vec![Vec::new(); tparams.len()].into_iter().chain(m.bounds).collect();
            statics.push((m, mspan));
            return Ok(());
        }
        // `this` は `ref 型`（`fn mut` なら `ref mut 型`）の引数
        let sty = TypeExpr {
            kind: if tparams.is_empty() {
                TypeKind::Name(name.to_string())
            } else {
                TypeKind::App(
                    name.to_string(),
                    tparams.iter().map(|t| TypeExpr { kind: TypeKind::Name(t.clone()), span: m.span }).collect(),
                )
            },
            span: m.span,
        };
        let this_ty = if take { sty } else { TypeExpr { kind: TypeKind::Ref(Box::new(sty), mutable), span: m.span } };
        m.params.insert(0, Param { ty: this_ty, name: "this".to_string(), mutable: false, span: m.span });
        m.tparams = tparams.iter().cloned().chain(m.tparams).collect();
        m.bounds = vec![Vec::new(); tparams.len()].into_iter().chain(m.bounds).collect();
        methods.push((m, mspan));
        Ok(())
    }

    /// 構造体の定義。中に書いたメソッドは、`this` を最初の引数に足した
    /// `構造体名.メソッド名` という名前の関数として返す
    fn parse_struct(&mut self) -> R<(StructDef, Vec<FnDef>)> {
        let start = self.expect(Tok::Struct, "`struct`")?;
        let name = self.expect_ident("構造体名")?;
        let span = start.join(self.prev_span());
        let tparams = self.parse_tparams()?;
        self.expect(Tok::LBrace, "`{`")?;
        let mut fields: Vec<String> = Vec::new();
        let mut field_types = Vec::new();
        let mut methods: Vec<(FnDef, Span)> = Vec::new();
        let mut statics: Vec<(FnDef, Span)> = Vec::new();
        loop {
            self.skip_semis();
            if self.eat(Tok::RBrace) {
                break;
            }
            if matches!(self.peek(), Tok::Fn | Tok::Static) {
                self.parse_method(&name, &tparams, &mut methods, &mut statics)?;
                continue;
            }
            if matches!(self.peek(), Tok::Ident(_)) && matches!(self.peek2(), Tok::Semi | Tok::Comma | Tok::RBrace) {
                return Err(Diagnostic::new("フィールドには型が必要です")
                    .primary(self.span(), "型がない")
                    .note("`struct Point { i64 x; i64 y; }` のように「型 名前;」で書いてください"));
            }
            let ty = self.parse_type()?;
            let fspan = self.span();
            let f = self.expect_ident("フィールド名")?;
            if fields.contains(&f) {
                return Err(Diagnostic::new(format!("フィールド `{}` が二重に定義されています", f))
                    .primary(fspan, "同じ名前のフィールドがすでにある"));
            }
            if !methods.is_empty() || !statics.is_empty() {
                return Err(Diagnostic::new("フィールドはメソッドより前に書いてください")
                    .primary(fspan, "メソッドのあとにフィールドがある"));
            }
            fields.push(f);
            field_types.push(ty);
            if *self.peek() == Tok::Comma {
                return Err(Diagnostic::new("構文エラー: フィールドの区切りは `;` です")
                    .primary(self.span(), "ここを `;` にしてください"));
            }
            if *self.peek() != Tok::RBrace {
                self.expect(Tok::Semi, "`;`（フィールドの終わり）")?;
            }
        }
        let fns = finish_methods(&name, &fields, &[], methods, statics)?;
        Ok((StructDef { name, tparams, fields, field_types, span }, fns))
    }

    fn parse_fn(&mut self) -> R<FnDef> {
        let start = self.expect(Tok::Fn, "`fn` か `struct`（トップレベルには関数と構造体の定義だけ書けます）")?;
        self.parse_fn_rest(start, true)
    }

    /// `fn` のあと（関数とメソッドで共通）。`body` が false なら本体の代わりに `;`（interface のメソッド）
    fn parse_fn_rest(&mut self, start: Span, body: bool) -> R<FnDef> {
        // `fn 名前(` / `fn 名前<T>(` なら戻り値なし、`fn 型 名前(` なら戻り値あり
        let ret = if self.at_fn_name() { None } else { Some(self.parse_type()?) };
        let name = self.expect_ident("関数名")?;
        let (tparams, bounds) = self.parse_tparams_bounded()?;
        self.expect(Tok::LParen, "`(`")?;
        let mut params = Vec::new();
        while *self.peek() != Tok::RParen {
            if matches!(self.peek(), Tok::Mut)
                || (matches!(self.peek(), Tok::Ident(_)) && matches!(self.peek2(), Tok::Comma | Tok::RParen))
            {
                return Err(Diagnostic::new("引数には型が必要です")
                    .primary(self.span(), "型がない")
                    .note("`fn f(i64 a, ref mut Point p)` のように「型 名前」で書いてください"));
            }
            let ty = self.parse_type()?;
            let mutable = self.eat(Tok::Mut);
            let span = self.span();
            let name = self.expect_ident("引数名")?;
            params.push(Param { ty, name, mutable, span });
            if *self.peek() == Tok::Comma {
                self.bump();
            } else {
                break;
            }
        }
        self.expect(Tok::RParen, "`)`")?;
        let header = start.join(self.prev_span());
        let body = if body {
            self.parse_block()?
        } else {
            let end = self.expect(Tok::Semi, "`;`（interface のメソッドには本体を書きません）")?;
            Block { stmts: Vec::new(), tail: false, end }
        };
        Ok(FnDef { name, tparams, bounds, ret, params, body, span: header })
    }

    /// 今の位置の `(` が、その場で作る関数 `(a, b) -> ...` の始まりか（対応する `)` の次が `->`）
    fn at_lambda(&self) -> bool {
        let mut depth = 0;
        for (i, t) in self.toks[self.pos..].iter().enumerate() {
            match t.tok {
                Tok::LParen => depth += 1,
                Tok::RParen => {
                    depth -= 1;
                    if depth == 0 {
                        return matches!(self.toks.get(self.pos + i + 1), Some(t) if t.tok == Tok::Arrow);
                    }
                }
                Tok::Eof | Tok::LBrace | Tok::Semi => return false,
                _ => {}
            }
        }
        false
    }

    /// `(a, b) -> 式` / `(i64 a) -> { ... }`。本体は関数の本体と同じ扱い（return はこの関数から返る）
    fn parse_lambda(&mut self) -> R<Expr> {
        let start = self.expect(Tok::LParen, "`(`")?;
        let mut params = Vec::new();
        while *self.peek() != Tok::RParen {
            // 名前だけ（`a`）か、型つき（`i64 a`）
            let ty = if matches!(self.peek(), Tok::Ident(_)) && matches!(self.peek2(), Tok::Comma | Tok::RParen) {
                None
            } else {
                Some(self.parse_type()?)
            };
            let span = self.span();
            let name = self.expect_ident("引数名")?;
            params.push(LambdaParam { ty, name, span });
            if !self.eat(Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::RParen, "`)`")?;
        self.expect(Tok::Arrow, "`->`")?;
        let saved = (self.loop_depth, self.in_value_if);
        self.loop_depth = 0;
        self.in_value_if = false;
        let body = if *self.peek() == Tok::LBrace {
            self.parse_block()
        } else {
            self.with_struct_lit(true, |p| p.parse_expr()).map(|e| {
                let end = e.span;
                Block { stmts: vec![Stmt::Expr(e)], tail: true, end }
            })
        };
        self.loop_depth = saved.0;
        self.in_value_if = saved.1;
        let body = body?;
        let span = start.join(body.end);
        let lambda = Lambda {
            params,
            body,
            captures: RefCell::new(Vec::new()),
            borrowed: RefCell::new(Vec::new()),
            borrows: Cell::new(false),
            returns: Cell::new(false),
        };
        Ok(Expr::new(ExprKind::Lambda(Rc::new(lambda)), span))
    }

    /// 今の位置が関数名か（`名前(` か `名前<...>(`）。そうでなければ戻り値の型
    fn at_fn_name(&self) -> bool {
        if !matches!(self.peek(), Tok::Ident(_)) {
            return false;
        }
        match self.peek2() {
            Tok::LParen => true,
            Tok::Lt => {
                // 対応する `>` の次が `(` なら `名前<T>(`、そうでなければ `Option<i64> 名前(`
                let mut depth = 0;
                for t in &self.toks[self.pos + 1..] {
                    match t.tok {
                        Tok::Lt => depth += 1,
                        Tok::Gt => {
                            depth -= 1;
                            if depth == 0 {
                                continue;
                            }
                        }
                        Tok::LParen if depth == 0 => return true,
                        _ if depth == 0 => return false,
                        _ => {}
                    }
                }
                false
            }
            _ => false,
        }
    }

    /// 型の引数の宣言 `<T, U>`（なければ空）
    /// 構造体・enum の型の引数（条件はつけられない）
    fn parse_tparams(&mut self) -> R<Vec<String>> {
        let sp = self.span();
        let (ps, bounds) = self.parse_tparams_bounded()?;
        if bounds.iter().any(|b| !b.is_empty()) {
            return Err(Diagnostic::new("構造体と enum の型の引数には条件をつけられません")
                .primary(sp, "`<T: ...>` の条件は関数とメソッドにだけ書けます"));
        }
        Ok(ps)
    }

    /// `<T, U: Shape + ordered>`。型の引数の名前と、それぞれの条件
    fn parse_tparams_bounded(&mut self) -> R<(Vec<String>, Vec<Vec<String>>)> {
        let mut ps: Vec<String> = Vec::new();
        let mut bounds = Vec::new();
        if !self.eat(Tok::Lt) {
            return Ok((ps, bounds));
        }
        loop {
            let sp = self.span();
            let p = self.expect_ident("型の引数の名前")?;
            if ps.contains(&p) {
                return Err(Diagnostic::new(format!("型の引数 `{}` が二回あります", p)).primary(sp, "ここ"));
            }
            ps.push(p);
            let mut bs = Vec::new();
            if self.eat(Tok::Colon) {
                loop {
                    bs.push(self.expect_ident("条件（interface の名前か ordered）")?);
                    if !self.eat(Tok::Plus) {
                        break;
                    }
                }
            }
            bounds.push(bs);
            if !self.eat(Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::Gt, "`>`")?;
        Ok((ps, bounds))
    }

    /// `{ 文; 文; 式 }`。文の終わりには `;` が要る（`{ }` で終わる if / while などは不要）。
    /// 最後の `;` のない式がブロックの値になる。
    fn parse_block(&mut self) -> R<Block> {
        self.nested(|p| p.parse_block_in())
    }

    fn parse_block_in(&mut self) -> R<Block> {
        self.expect(Tok::LBrace, "`{`")?;
        let mut stmts = Vec::new();
        let mut tail = false;
        loop {
            self.skip_semis();
            match self.peek().clone() {
                Tok::RBrace => {
                    let end = self.bump().span;
                    return Ok(Block { stmts, tail, end });
                }
                Tok::Eof => {
                    return Err(Diagnostic::new("構文エラー: `}` が閉じていません")
                        .primary(self.span(), "ファイルがここで終わっています"));
                }
                _ => {}
            }
            let stmt = self.parse_stmt()?;
            let block_like = is_block_like(&stmt);
            let is_expr = matches!(stmt, Stmt::Expr(_));
            let end_of_stmt = self.prev_span();
            stmts.push(stmt);
            tail = false;
            match self.peek() {
                Tok::Semi => {
                    self.bump();
                }
                Tok::RBrace if is_expr => tail = true,
                _ if block_like => {}
                Tok::RBrace => {
                    return Err(Diagnostic::new("構文エラー: 文の終わりに `;` が必要です")
                        .primary(end_of_stmt, "この後ろに `;` を書いてください")
                        .note("ブロックの最後に `;` なしで書けるのは、値になる式だけです"));
                }
                _ => {
                    return Err(Diagnostic::new("構文エラー: 文の終わりに `;` が必要です")
                        .primary(end_of_stmt, "この後ろに `;` を書いてください"));
                }
            }
        }
    }

    fn parse_stmt(&mut self) -> R<Stmt> {
        match self.peek().clone() {
            Tok::Let => {
                self.bump();
                // `let 型 mut 名前 = 式`。すぐ後ろに `=` が来る名前が変数名で、その手前が型
                let named_now = matches!(self.peek(), Tok::Mut)
                    || (matches!(self.peek(), Tok::Ident(_)) && *self.peek2() == Tok::Assign);
                let ty = if named_now { None } else { Some(self.parse_type()?) };
                let mutable = self.eat(Tok::Mut);
                let span = self.span();
                let name = self.expect_ident("変数名")?;
                self.expect(Tok::Assign, "`=`")?;
                let init = self.parse_expr()?;
                Ok(Stmt::Let { name, ty, mutable, span, init })
            }
            Tok::If => Ok(Stmt::Expr(self.parse_if(false)?)),
            Tok::Match => Ok(Stmt::Expr(self.parse_match(false)?)),
            Tok::While => {
                self.bump();
                let cond = self.parse_cond()?;
                let body = self.parse_loop_body()?;
                Ok(Stmt::While { cond, body })
            }
            Tok::Loop => {
                self.bump();
                let body = self.parse_loop_body()?;
                Ok(Stmt::Loop { body })
            }
            Tok::For => self.parse_for(),
            Tok::Break | Tok::Continue => {
                let t = self.bump();
                let word = if t.tok == Tok::Break { "break" } else { "continue" };
                if self.loop_depth == 0 {
                    let d = if self.in_value_if {
                        Diagnostic::new(format!("値として使う if / match の中から、外のループへ {} できません", word))
                            .primary(t.span, "ここ")
                            .note("文として使うか、ループの中で値を決めてから使ってください")
                    } else {
                        Diagnostic::new(format!("ループの外では {} は使えません", word))
                            .primary(t.span, "囲んでいる loop / while / for がない")
                    };
                    return Err(d);
                }
                Ok(if t.tok == Tok::Break { Stmt::Break(t.span) } else { Stmt::Continue(t.span) })
            }
            Tok::Return => {
                // 値として使う if / match の中でも return できる（`let n = match t { case None -> return 0; ... };`）
                let start = self.bump().span;
                let value = match self.peek().clone() {
                    Tok::Semi | Tok::RBrace | Tok::Eof => None,
                    _ => Some(self.parse_expr()?),
                };
                let span = start.join(self.prev_span());
                Ok(Stmt::Return { value, span })
            }
            _ => {
                let e = self.parse_expr()?;
                let op = match self.peek() {
                    Tok::Assign => None,
                    Tok::PlusEq | Tok::PlusPlus => Some(BinOp::Add),
                    Tok::MinusEq | Tok::MinusMinus => Some(BinOp::Sub),
                    Tok::StarEq => Some(BinOp::Mul),
                    Tok::SlashEq => Some(BinOp::Div),
                    Tok::PercentEq => Some(BinOp::Rem),
                    _ => return Ok(Stmt::Expr(e)),
                };
                if !matches!(e.kind, ExprKind::Var(_) | ExprKind::Deref(..) | ExprKind::Field(..) | ExprKind::Index(..)) {
                    return Err(Diagnostic::new("構文エラー: ここには代入できません")
                        .primary(e.span, "代入先は変数名か `*p` か `p.f` か `a[i]` だけです"));
                }
                let t = self.bump();
                // `x += e` は `x = x + e`、`x++` は `x = x + 1` として扱う（文としてだけ書ける）
                let rhs = match t.tok {
                    Tok::PlusPlus | Tok::MinusMinus => Expr::new(ExprKind::Int(1), t.span),
                    _ => self.parse_expr()?,
                };
                let span = e.span.join(self.prev_span());
                let value = match op {
                    None => rhs,
                    Some(op) => Expr::new(ExprKind::Binary(op, Box::new(e.clone()), Box::new(rhs)), span),
                };
                Ok(Stmt::Assign { target: e, value, span })
            }
        }
    }

    /// `if`。`as_value` なら値として使う位置なので else が必須で、中から外への break / continue はできない（return はできる）。
    fn parse_if(&mut self, as_value: bool) -> R<Expr> {
        let start = self.expect(Tok::If, "`if`")?;
        self.value_context(as_value, |p| p.parse_if_rest(as_value, start))
    }

    /// 値として使う位置なら、中から外への break / continue をできなくする
    fn value_context<T>(&mut self, as_value: bool, f: impl FnOnce(&mut Self) -> R<T>) -> R<T> {
        let saved = (self.loop_depth, self.in_value_if);
        if as_value {
            self.loop_depth = 0;
            self.in_value_if = true;
        }
        let r = f(self);
        self.loop_depth = saved.0;
        self.in_value_if = saved.1;
        r
    }

    /// `match x { case 1, 2 -> ... default -> ... }`
    fn parse_match(&mut self, as_value: bool) -> R<Expr> {
        let start = self.expect(Tok::Match, "`match`")?;
        let scrut = self.parse_cond()?;
        self.value_context(as_value, |p| p.parse_match_rest(as_value, start, scrut))
    }

    fn parse_match_rest(&mut self, as_value: bool, start: Span, scrut: Expr) -> R<Expr> {
        self.expect(Tok::LBrace, "`{`")?;
        let mut arms: Vec<MatchArm> = Vec::new();
        let mut default: Option<Block> = None;
        loop {
            self.skip_semis();
            match self.peek().clone() {
                Tok::RBrace => {
                    self.bump();
                    break;
                }
                Tok::Case => {
                    self.bump();
                    let mut pats = Vec::new();
                    loop {
                        let (pat, span) = self.parse_pat()?;
                        let seen = arms.iter().flat_map(|a| a.pats.iter()).chain(pats.iter());
                        if let Some((_, first)) = seen.into_iter().find(|(p, _)| p.same_case(&pat)) {
                            return Err(Diagnostic::new("同じ値の case が二回あります")
                                .label(*first, "最初はここ")
                                .primary(span, "ここは決して選ばれない"));
                        }
                        pats.push((pat, span));
                        if *self.peek() != Tok::Comma {
                            break;
                        }
                        self.bump();
                    }
                    let body = self.parse_arm_body(as_value)?;
                    arms.push(MatchArm { pats, body });
                }
                Tok::Default => {
                    let span = self.bump().span;
                    if default.is_some() {
                        return Err(Diagnostic::new("default が二回あります").primary(span, "ここが二回目"));
                    }
                    default = Some(self.parse_arm_body(as_value)?);
                }
                _ => return Err(self.unexpected("`case` か `default` か `}`")),
            }
        }
        // enum の場合は、全部の種類を書いたかを型検査が調べる
        let has_variants = arms.iter().any(|a| a.pats.iter().any(|(p, _)| matches!(p, Pat::Variant(..))));
        if as_value && default.is_none() && !covers_both_bools(&arms) && !has_variants {
            return Err(Diagnostic::new("値として使う match には default が必要です")
                .primary(start.join(scrut.span), "どの case にも当てはまらないときの値がない")
                .note("`default -> 値` を足してください"));
        }
        Ok(Expr::new(ExprKind::Match { scrut: Box::new(scrut), arms, default }, start))
    }

    /// `case` に書く値。今のところリテラルだけ。
    fn parse_pat(&mut self) -> R<(Pat, Span)> {
        self.nested(|p| p.parse_pat_in())
    }

    fn parse_pat_in(&mut self) -> R<(Pat, Span)> {
        let start = self.span();
        let pat = match self.peek().clone() {
            Tok::Int(n) => Pat::Int(n),
            Tok::Str(s) => Pat::Str(s),
            Tok::True => Pat::Bool(true),
            Tok::False => Pat::Bool(false),
            Tok::Ident(name) => {
                // `Circle(r, _)` / `Empty` / `Some(x)` / `None`
                self.bump();
                let mut binds = Vec::new();
                let mut subs = Vec::new();
                if self.eat(Tok::LParen) {
                    while *self.peek() != Tok::RParen {
                        let sp = self.span();
                        // 中身の位置: 名前（小文字で始まる）か `_` なら受け取る名前。
                        // 大文字で始まる名前・`名前(...)`・数や文字列なら、重ねた形
                        let nested = match self.peek().clone() {
                            Tok::Ident(n) => {
                                *self.peek2() == Tok::LParen || n.starts_with(|c: char| c.is_ascii_uppercase())
                            }
                            _ => true,
                        };
                        if nested {
                            let (p, psp) = self.parse_pat()?;
                            binds.push(("_".to_string(), psp));
                            subs.push(Some(p));
                        } else {
                            binds.push((self.expect_ident("中身を受け取る名前")?, sp));
                            subs.push(None);
                        }
                        if !self.eat(Tok::Comma) {
                            break;
                        }
                    }
                    self.expect(Tok::RParen, "`)`")?;
                }
                return Ok((Pat::Variant(name, binds, subs), start.join(self.prev_span())));
            }
            Tok::Minus => {
                self.bump();
                match self.peek().clone() {
                    Tok::Int(n) => Pat::Int(-n),
                    _ => return Err(self.unexpected("整数")),
                }
            }
            _ => {
                return Err(Diagnostic::new("case に書けるのは、今のところ整数・真偽値・文字列の値と enum の種類だけです")
                    .primary(start, format!("ここに {} があります", self.peek().describe())))
            }
        };
        self.bump();
        Ok((pat, start.join(self.prev_span())))
    }

    /// `-> 式;`、`-> 文;`、`-> { ... }`（Java と同じ）。どれもブロックにそろえる
    fn parse_arm_body(&mut self, as_value: bool) -> R<Block> {
        self.expect(Tok::Arrow, "`->`")?;
        if *self.peek() == Tok::LBrace {
            return self.parse_value_block(as_value);
        }
        let stmt = self.parse_stmt()?;
        let end = self.prev_span();
        let is_expr = matches!(stmt, Stmt::Expr(_));
        if !is_block_like(&stmt) {
            self.expect(Tok::Semi, "`;`（case の終わり）")?;
        }
        // `case None -> return 0;` は値が要らない（そこで関数を抜ける）
        let returns = matches!(stmt, Stmt::Return { .. });
        if as_value && !is_expr && !returns {
            return Err(Diagnostic::new("値として使う match の case に値がありません")
                .primary(end, "ここは値ではなく文")
                .note("`case 1 -> 値;` のように書いてください"));
        }
        Ok(Block { stmts: vec![stmt], tail: is_expr, end })
    }

    fn parse_if_rest(&mut self, as_value: bool, start: Span) -> R<Expr> {
        let cond = self.parse_cond()?;
        let then = self.parse_value_block(as_value)?;

        // `}` の次の行に else がある書き方も許す
        let save = self.pos;
        let els = if *self.peek() == Tok::Else {
            self.bump();
            if *self.peek() == Tok::If {
                let nested = self.parse_if(as_value)?;
                let end = self.prev_span();
                Some(Block { stmts: vec![Stmt::Expr(nested)], tail: true, end })
            } else {
                Some(self.parse_value_block(as_value)?)
            }
        } else {
            self.pos = save;
            if as_value {
                return Err(Diagnostic::new("値として使う if には else が必要です")
                    .primary(start.join(cond.span), "条件が false のときの値がない")
                    .note("`if 条件 { 値 } else { 値 }` のように書いてください"));
            }
            None
        };
        Ok(Expr::new(ExprKind::If { cond: Box::new(cond), then, els }, start))
    }

    /// 値として使う if / match のブロック。最後に `;` のない式（値）が必要
    fn parse_value_block(&mut self, as_value: bool) -> R<Block> {
        let b = self.parse_block()?;
        // 最後が return なら値は要らない（そこで関数を抜ける）
        let returns = matches!(b.stmts.last(), Some(Stmt::Return { .. }));
        if as_value && !b.tail && !returns {
            return Err(Diagnostic::new("値として使うブロックの最後に値がありません")
                .primary(b.end, "このブロックの値が決まらない")
                .note("最後の式の `;` を外すと、それがブロックの値になります"));
        }
        Ok(b)
    }

    fn parse_loop_body(&mut self) -> R<Block> {
        self.loop_depth += 1;
        let r = self.parse_block();
        self.loop_depth -= 1;
        r
    }

    /// `for i in range(...) { ... } else { ... }`
    fn parse_for(&mut self) -> R<Stmt> {
        self.expect(Tok::For, "`for`")?;
        // `for (k, v) in m`: リストなら (添字, 要素)、辞書なら (キー, 値)
        let key = if self.eat(Tok::LParen) {
            let s = self.span();
            let k = self.expect_ident("ループ変数の名前")?;
            self.expect(Tok::Comma, "`,`")?;
            Some((k, s))
        } else {
            None
        };
        let var_span = self.span();
        let var = self.expect_ident("ループ変数の名前")?;
        if key.is_some() {
            self.expect(Tok::RParen, "`)`")?;
        }
        self.expect(Tok::In, "`in`")?;
        let is_range = matches!(self.peek(), Tok::Ident(n) if n == "range") && *self.peek2() == Tok::LParen;
        if is_range && key.is_some() {
            return Err(Diagnostic::new("range の for では名前を 1 つだけ書きます")
                .primary(var_span, "`for (i, x) in a` は配列・リスト・辞書を回すときの書き方"));
        }
        if !is_range {
            // `for x in a`: 配列・リストの要素
            let iter = self.parse_cond()?;
            let body = self.parse_loop_body()?;
            let els = if self.eat(Tok::Else) { Some(self.parse_block()?) } else { None };
            return Ok(Stmt::ForEach { key, var, var_span, iter, body, els, consume: Cell::new(false) });
        }
        let call = self.parse_postfix()?;
        let range = match call.kind {
            ExprKind::Call(_, args) => args,
            _ => return Err(self.unexpected("`(`")),
        };
        if range.is_empty() || range.len() > 3 {
            return Err(Diagnostic::new("range の引数は 1〜3 個です")
                .primary(call.span, format!("{} 個渡されました", range.len()))
                .note("range(終わり) / range(始め, 終わり) / range(始め, 終わり, 刻み)"));
        }
        let body = self.parse_loop_body()?;
        let save = self.pos;
        let els = if *self.peek() == Tok::Else {
            self.bump();
            Some(self.parse_block()?)
        } else {
            self.pos = save;
            None
        };
        Ok(Stmt::For { var, range, range_span: call.span, body, els })
    }

    /// 型: `i64`、`Player`、`own T`、`ref T`、`ref mut T`
    fn parse_type(&mut self) -> R<TypeExpr> {
        let base = self.depth;
        self.deeper()?;
        let result = self.parse_type_in();
        self.depth = base;
        result
    }

    fn parse_type_in(&mut self) -> R<TypeExpr> {
        let start = self.span();
        let kind = match self.peek().clone() {
            Tok::Own => {
                self.bump();
                TypeKind::Own(Box::new(self.parse_type()?))
            }
            Tok::Ref => {
                self.bump();
                let mutable = self.eat(Tok::Mut);
                TypeKind::Ref(Box::new(self.parse_type()?), mutable)
            }
            Tok::Ident(name) => {
                self.bump();
                if self.eat(Tok::Lt) {
                    let mut args = Vec::new();
                    loop {
                        args.push(self.parse_type()?);
                        if !self.eat(Tok::Comma) {
                            break;
                        }
                    }
                    self.expect(Tok::Gt, "`>`")?;
                    TypeKind::App(name, args)
                } else {
                    TypeKind::Name(name)
                }
            }
            Tok::Fn => {
                // `fn bool(i64, i64)`: 関数の型。`(` がすぐ来れば戻り値なし
                self.bump();
                let ret = if *self.peek() == Tok::LParen { None } else { Some(Box::new(self.parse_type()?)) };
                self.expect(Tok::LParen, "`(`")?;
                let mut params = Vec::new();
                while *self.peek() != Tok::RParen {
                    params.push(self.parse_type()?);
                    if !self.eat(Tok::Comma) {
                        break;
                    }
                }
                self.expect(Tok::RParen, "`)`")?;
                TypeKind::Fn(params, ret)
            }
            Tok::LParen => {
                // `(own i64)[]`: かっこでまとめる（`own i64[]` は「own のリスト」ではなく「リストの own」）
                self.bump();
                if self.eat(Tok::RParen) {
                    // `()`: 値なし（`task<()>`・`fn ()(i64)` と書くより `fn (i64)`）
                    TypeKind::Name("()".to_string())
                } else {
                    let inner = self.parse_type()?;
                    self.expect(Tok::RParen, "`)`")?;
                    inner.kind
                }
            }
            _ => return Err(self.unexpected("型")),
        };
        let mut t = TypeExpr { kind, span: start.join(self.prev_span()) };
        // `i64[3]`、`i64[3][2]`（長さ 3 の配列が 2 つ）、`i64[]`（リスト）、`i64[str]`（辞書）
        while self.eat(Tok::LBrack) {
            // Each postfix adds a recursive AST node even though parsing is a loop.
            self.deeper()?;
            let kind = if self.eat(Tok::RBrack) {
                TypeKind::List(Box::new(t))
            } else if matches!(self.peek(), Tok::Ident(_)) {
                let key = self.parse_type()?;
                self.expect(Tok::RBrack, "`]`")?;
                TypeKind::Map(Box::new(t), Box::new(key))
            } else {
                let n = self.array_len()?;
                self.expect(Tok::RBrack, "`]`")?;
                TypeKind::Array(Box::new(t), n)
            };
            t = TypeExpr { kind, span: start.join(self.prev_span()) };
        }
        Ok(t)
    }

    /// 配列の長さ（そのまま書いた数）
    fn array_len(&mut self) -> R<u64> {
        match self.peek().clone() {
            Tok::Int(n) => {
                self.bump();
                Ok(n as u64)
            }
            _ => Err(self.unexpected("配列の長さ（数）")),
        }
    }

    pub fn parse_expr(&mut self) -> R<Expr> {
        self.parse_binary(1)
    }

    /// `if` / `while` の条件。ここでは構造体リテラルを書けない（括弧で囲めば書ける）。
    fn parse_cond(&mut self) -> R<Expr> {
        self.with_struct_lit(false, |p| p.parse_expr())
    }

    fn with_struct_lit<T>(&mut self, allowed: bool, f: impl FnOnce(&mut Self) -> R<T>) -> R<T> {
        let saved = std::mem::replace(&mut self.no_struct_lit, !allowed);
        let r = f(self);
        self.no_struct_lit = saved;
        r
    }

    fn parse_binary(&mut self, min_prec: u8) -> R<Expr> {
        let base = self.depth;
        let mut lhs = self.parse_cast()?;
        loop {
            // `>>` は `>` 2 つ（型の引数の `Option<Option<i64>>` と区別するため、字句解析では分けておく）
            let shr = *self.peek() == Tok::Gt && *self.peek2() == Tok::Gt && {
                let (a, b) = (self.toks[self.pos].span, self.toks[(self.pos + 1).min(self.toks.len() - 1)].span);
                a.line == b.line && a.col + 1 == b.col
            };
            let info = if shr { Some((BinOp::Shr, 8)) } else { binop_info(self.peek()) };
            let (op, prec) = match info {
                Some(x) if x.1 >= min_prec => x,
                _ => break,
            };
            self.deeper()?;
            self.bump();
            if shr {
                self.bump();
            }
            let rhs = self.parse_binary(prec + 1)?;
            let span = lhs.span.join(rhs.span);
            lhs = Expr::new(ExprKind::Binary(op, Box::new(lhs), Box::new(rhs)), span);
        }
        self.depth = base;
        Ok(lhs)
    }

    /// `e as T`。前置演算子より弱く、`*` や `+` より強い（Rust と同じ。`-x as u8` は `(-x) as u8`）
    fn parse_cast(&mut self) -> R<Expr> {
        let base = self.depth;
        let mut e = self.parse_unary()?;
        while *self.peek() == Tok::As {
            self.deeper()?;
            self.bump();
            let ty = self.parse_type()?;
            let span = e.span.join(ty.span);
            e = Expr::new(ExprKind::Cast(Box::new(e), ty), span);
        }
        self.depth = base;
        Ok(e)
    }

    /// 前置演算子。
    /// `ref` と `*` は強く結びつく（`ref a`, `*r + 1` は `(*r) + 1`）。
    /// `.f` はさらに強い（`ref p.x` は `ref (p.x)`、`*p.x` は `*(p.x)`）。
    /// `own` は右側の式全体を取る（`own n * 2` は `own (n * 2)`）。
    fn parse_unary(&mut self) -> R<Expr> {
        self.nested(|p| p.parse_unary_in())
    }

    fn parse_unary_in(&mut self) -> R<Expr> {
        match self.peek().clone() {
            Tok::Minus => {
                let start = self.bump().span;
                let e = self.parse_unary()?;
                let span = start.join(e.span);
                Ok(Expr::new(ExprKind::Unary(UnOp::Neg, Box::new(e)), span))
            }
            Tok::Bang => {
                let start = self.bump().span;
                let e = self.parse_unary()?;
                let span = start.join(e.span);
                Ok(Expr::new(ExprKind::Unary(UnOp::Not, Box::new(e)), span))
            }
            Tok::Tilde => {
                let start = self.bump().span;
                let e = self.parse_unary()?;
                let span = start.join(e.span);
                Ok(Expr::new(ExprKind::Unary(UnOp::BitNot, Box::new(e)), span))
            }
            Tok::Star => {
                let start = self.bump().span;
                let e = self.parse_unary()?;
                let span = start.join(e.span);
                Ok(Expr::new(ExprKind::Deref(Box::new(e), Cell::new(Check::Unvisited)), span))
            }
            Tok::Ref => {
                let start = self.bump().span;
                let mutable = self.eat(Tok::Mut);
                let e = self.parse_unary()?;
                let span = start.join(e.span);
                Ok(Expr::new(ExprKind::Ref(Box::new(e), mutable), span))
            }
            Tok::Own => {
                let start = self.bump().span;
                let e = self.parse_expr()?;
                let span = start.join(e.span);
                Ok(Expr::new(ExprKind::Own(Box::new(e)), span))
            }
            _ => self.parse_postfix(),
        }
    }

    /// `.f` と `?`（Err / None ならそのまま return する）
    fn parse_postfix(&mut self) -> R<Expr> {
        let base = self.depth;
        let mut e = self.parse_primary()?;
        loop {
            if matches!(self.peek(), Tok::Question | Tok::LBrack | Tok::Dot) {
                self.deeper()?;
            }
            if self.eat(Tok::Question) {
                let span = e.span.join(self.toks[self.pos - 1].span);
                e = Expr::new(ExprKind::Try(Box::new(e)), span);
                continue;
            }
            if *self.peek() == Tok::LBrack {
                self.bump();
                let idx = self.with_struct_lit(true, |p| p.parse_expr())?;
                let end = self.expect(Tok::RBrack, "`]`")?;
                let span = e.span.join(end);
                e = Expr::new(ExprKind::Index(Box::new(e), Box::new(idx), Cell::new(Check::Unvisited)), span);
                continue;
            }
            if *self.peek() != Tok::Dot {
                break;
            }
            self.bump();
            let fspan = self.span();
            let name = self.expect_ident("フィールド名")?;
            if *self.peek() == Tok::LParen {
                // `e.name(...)` はメソッド呼び出し
                let (args, end) = self.parse_args()?;
                lambdas_borrow(&args);
                let span = e.span.join(end);
                e = Expr::new(ExprKind::MethodCall(Box::new(e), name, args, RefCell::default()), span);
                continue;
            }
            let span = e.span.join(fspan);
            e = Expr::new(ExprKind::Field(Box::new(e), name, Cell::new(Check::Unvisited)), span);
        }
        self.depth = base;
        Ok(e)
    }

    fn parse_primary(&mut self) -> R<Expr> {
        match self.peek().clone() {
            Tok::If => self.parse_if(true),
            Tok::Match => self.parse_match(true),
            Tok::Int(n) => {
                let span = self.bump().span;
                Ok(Expr::new(ExprKind::Int(n), span))
            }
            Tok::Float(t) => {
                let span = self.bump().span;
                let v = t.parse::<f64>().map_err(|_| Diagnostic::new("小数の書き方がおかしいです").primary(span, "ここ"))?;
                Ok(Expr::new(ExprKind::Float(v), span))
            }
            Tok::Str(s) => {
                let span = self.bump().span;
                Ok(Expr::new(ExprKind::Str(s), span))
            }
            Tok::LBrack => {
                // `[1, 2, 3]` / `[0; 5]`
                let start = self.bump().span;
                let mut items = Vec::new();
                if *self.peek() != Tok::RBrack {
                    let first = self.with_struct_lit(true, |p| p.parse_expr())?;
                    if self.eat(Tok::Semi) {
                        let n = self.array_len()?;
                        let end = self.expect(Tok::RBrack, "`]`")?;
                        return Ok(Expr::new(ExprKind::ArrayRepeat(Box::new(first), n, Cell::new(false)), start.join(end)));
                    }
                    if self.eat(Tok::Colon) {
                        // `["a": 1, "b": 2]`: 辞書
                        let mut pairs = vec![(first, self.with_struct_lit(true, |p| p.parse_expr())?)];
                        while self.eat(Tok::Comma) {
                            if *self.peek() == Tok::RBrack {
                                break;
                            }
                            let k = self.with_struct_lit(true, |p| p.parse_expr())?;
                            self.expect(Tok::Colon, "`:`")?;
                            pairs.push((k, self.with_struct_lit(true, |p| p.parse_expr())?));
                        }
                        let end = self.expect(Tok::RBrack, "`]`")?;
                        return Ok(Expr::new(ExprKind::MapLit(pairs), start.join(end)));
                    }
                    items.push(first);
                    while self.eat(Tok::Comma) {
                        if *self.peek() == Tok::RBrack {
                            break;
                        }
                        items.push(self.with_struct_lit(true, |p| p.parse_expr())?);
                    }
                }
                let end = self.expect(Tok::RBrack, "`]`")?;
                Ok(Expr::new(ExprKind::ArrayLit(items, Cell::new(Seq::Array)), start.join(end)))
            }
            Tok::Interp(parts) => {
                let span = self.bump().span;
                let mut out = Vec::new();
                for p in parts {
                    match p {
                        StrPart::Lit(s) => out.push(InterpPart::Lit(s)),
                        StrPart::Code(toks) => {
                            // 埋め込みの中の式を、同じ規則（enum・構造体の名前）で読む
                            let mut sub = Parser {
                                toks,
                                pos: 0,
                                no_struct_lit: false,
                                loop_depth: 0,
                                in_value_if: true,
                                enum_names: self.enum_names.clone(),
                                struct_names: self.struct_names.clone(),
                                enum_variants: self.enum_variants.clone(),
                                depth: self.depth,
                            };
                            let e = sub.parse_expr()?;
                            if *sub.peek() != Tok::Eof {
                                return Err(sub.unexpected("`}`（埋め込みの終わり）"));
                            }
                            out.push(InterpPart::Expr(e));
                        }
                    }
                }
                Ok(Expr::new(ExprKind::Interp(out), span))
            }
            Tok::True => {
                let span = self.bump().span;
                Ok(Expr::new(ExprKind::Bool(true), span))
            }
            Tok::False => {
                let span = self.bump().span;
                Ok(Expr::new(ExprKind::Bool(false), span))
            }
            Tok::Ident(name) if name == "None" => {
                let span = self.bump().span;
                Ok(self.variant("Option", "None", Vec::new(), span))
            }
            Tok::Ident(name) if name == "Some" && *self.peek2() == Tok::LParen => {
                let start = self.bump().span;
                let (args, end) = self.parse_args()?;
                Ok(self.variant("Option", "Some", args, start.join(end)))
            }
            Tok::Ident(name) if (name == "Ok" || name == "Err") && *self.peek2() == Tok::LParen => {
                let start = self.bump().span;
                let (args, end) = self.parse_args()?;
                Ok(self.variant("Result", &name, args, start.join(end)))
            }
            Tok::Ident(name) if self.struct_names.contains(&name) && *self.peek2() == Tok::Dot => {
                // `Point.origin(...)`: 静的メソッドの呼び出し。`構造体名.名前` という関数を呼ぶ
                let start = self.bump().span;
                self.bump();
                let f = self.expect_ident("静的メソッドの名前")?;
                if *self.peek() != Tok::LParen {
                    return Err(Diagnostic::new("構造体名のあとには静的メソッドの呼び出しを書きます")
                        .primary(start.join(self.prev_span()), "`Point.origin()` のように `(...)` が必要"));
                }
                let (args, end) = self.parse_args()?;
                lambdas_borrow(&args);
                Ok(Expr::new(ExprKind::Call(format!("{}.{}", name, f), args), start.join(end)))
            }
            Tok::Ident(name) if self.enum_names.contains(&name) && *self.peek2() == Tok::Dot => {
                // `Shape.Circle(5)` / `Shape.Empty`
                let start = self.bump().span;
                self.bump();
                let variant = self.expect_ident("種類の名前")?;
                let is_static = matches!(self.enum_variants.get(&name), Some(vs) if !vs.contains(&variant));
                if is_static && *self.peek() == Tok::LParen {
                    // `Shape.make(...)`: enum の静的メソッドの呼び出し
                    let (args, end) = self.parse_args()?;
                    lambdas_borrow(&args);
                    return Ok(Expr::new(ExprKind::Call(format!("{}.{}", name, variant), args), start.join(end)));
                }
                let (args, end) = if *self.peek() == Tok::LParen {
                    self.parse_args()?
                } else {
                    (Vec::new(), self.prev_span())
                };
                Ok(self.variant(&name, &variant, args, start.join(end)))
            }
            Tok::Ident(name) => {
                let start = self.bump().span;
                if *self.peek() == Tok::LParen {
                    self.bump();
                    let mut args = Vec::new();
                    while *self.peek() != Tok::RParen {
                        args.push(self.with_struct_lit(true, |p| p.parse_expr())?);
                        if *self.peek() == Tok::Comma {
                            self.bump();
                        } else {
                            break;
                        }
                    }
                    let end = self.expect(Tok::RParen, "`)`")?;
                    // spawn のクロージャは、取り込んだ値をほかのスレッドへ移すので借りない
                    if name != "spawn" {
                        lambdas_borrow(&args);
                    }
                    Ok(Expr::new(ExprKind::Call(name, args), start.join(end)))
                } else if *self.peek() == Tok::LBrace && !self.no_struct_lit {
                    self.parse_struct_lit(name, start)
                } else {
                    Ok(Expr::new(ExprKind::Var(name), start))
                }
            }
            Tok::LParen if self.at_lambda() => self.parse_lambda(),
            Tok::LParen if *self.peek2() == Tok::RParen => {
                let start = self.bump().span;
                let end = self.bump().span;
                Ok(Expr::new(ExprKind::Unit, start.join(end)))
            }
            Tok::LParen => {
                let start = self.bump().span;
                let inner = self.with_struct_lit(true, |p| p.parse_expr())?;
                let end = self.expect(Tok::RParen, "`)`")?;
                Ok(Expr::new(inner.kind, start.join(end)))
            }
            _ => Err(self.unexpected("式")),
        }
    }

    /// `( 式, 式 )` を読む。閉じ括弧の位置も返す
    fn parse_args(&mut self) -> R<(Vec<Expr>, Span)> {
        self.expect(Tok::LParen, "`(`")?;
        let mut args = Vec::new();
        while *self.peek() != Tok::RParen {
            args.push(self.with_struct_lit(true, |p| p.parse_expr())?);
            if !self.eat(Tok::Comma) {
                break;
            }
        }
        let end = self.expect(Tok::RParen, "`)`")?;
        Ok((args, end))
    }

    fn variant(&self, enum_name: &str, variant: &str, args: Vec<Expr>, span: Span) -> Expr {
        let kind = ExprKind::Variant {
            enum_name: enum_name.to_string(),
            variant: variant.to_string(),
            args,
            copy: Cell::new(Some(false)),
        };
        Expr::new(kind, span)
    }

    fn parse_struct_lit(&mut self, name: String, start: Span) -> R<Expr> {
        self.expect(Tok::LBrace, "`{`")?;
        let mut fields = Vec::new();
        while *self.peek() != Tok::RBrace {
            let span = self.span();
            let fname = self.expect_ident("フィールド名")?;
            self.expect(Tok::Colon, "`:`")?;
            let value = self.with_struct_lit(true, |p| p.parse_expr())?;
            fields.push(FieldInit { name: fname, span, value });
            if *self.peek() == Tok::Comma {
                self.bump();
            } else {
                break;
            }
        }
        let end = self.expect(Tok::RBrace, "`}`")?;
        Ok(Expr::new(ExprKind::StructLit(name, fields), start.join(end)))
    }
}

/// `モジュール名 . 名前` の 3 つのトークンを、1 つの名前 `モジュール名.名前` にする（文字列の埋め込みの中も）。
/// その名前がモジュールにない・`pub` でないならエラー
fn merge_paths(toks: Vec<Token>, deps: &HashMap<String, ModuleInfo>) -> R<Vec<Token>> {
    let mut out: Vec<Token> = Vec::with_capacity(toks.len());
    let mut it = toks.into_iter().peekable();
    while let Some(mut t) = it.next() {
        if let Tok::Interp(parts) = &mut t.tok {
            for p in parts.iter_mut() {
                if let StrPart::Code(inner) = p {
                    *inner = merge_paths(std::mem::take(inner), deps)?;
                }
            }
        }
        let module = match &t.tok {
            // `x.geom.f` のようにフィールド名として書いたものはモジュールではない
            Tok::Ident(m)
                if deps.contains_key(m)
                    && matches!(it.peek(), Some(d) if d.tok == Tok::Dot)
                    && !matches!(out.last(), Some(p) if p.tok == Tok::Dot) =>
            {
                m.clone()
            }
            _ => {
                out.push(t);
                continue;
            }
        };
        let dot = it.next().expect("peeked");
        let Some(Token { tok: Tok::Ident(name), span }) = it.next() else {
            return Err(Diagnostic::new(format!("`{}.` のあとに名前がありません", module))
                .primary(dot.span, format!("`{}.名前` と書いてください", module)));
        };
        let full = t.span.join(span);
        match deps[&module].names.get(&name) {
            None => {
                return Err(Diagnostic::new(format!("モジュール `{}` に `{}` はありません", module, name))
                    .primary(full, "この名前は定義されていない"))
            }
            Some((_, false)) => {
                return Err(Diagnostic::new(format!("`{}.{}` は pub ではありません", module, name))
                    .primary(full, "ほかのファイルからは使えない")
                    .note(format!("使えるようにするなら、{}.jw の定義の前に `pub` をつけてください", module)))
            }
            Some(_) => {}
        }
        out.push(Token { tok: Tok::Ident(format!("{}.{}", module, name)), span: full });
    }
    Ok(out)
}

/// `{ }` で終わる文か（後ろに `;` が要らない）
fn is_block_like(s: &Stmt) -> bool {
    match s {
        Stmt::While { .. } | Stmt::Loop { .. } | Stmt::For { .. } | Stmt::ForEach { .. } => true,
        Stmt::Expr(e) => matches!(e.kind, ExprKind::If { .. } | ExprKind::Match { .. }),
        _ => false,
    }
}

/// true と false の両方に case があるか（それなら default がなくても必ずどれかに当たる）
fn covers_both_bools(arms: &[MatchArm]) -> bool {
    let has = |b| arms.iter().any(|a| a.pats.iter().any(|(p, _)| *p == Pat::Bool(b)));
    has(true) && has(false)
}

/// `const` の値を、ほかの定数を使っていれば置き換えて決め、関数の中で名前を使っている場所を値に置き換える。定数の名前を返す
fn resolve_consts(consts: &mut [(String, Expr)], fns: &mut [FnDef]) -> R<Vec<String>> {
    for f in fns.iter() {
        if let Some((n, v)) = consts.iter().find(|(n, _)| *n == f.name) {
            return Err(Diagnostic::new(format!("`{}` は定数の名前です", n))
                .label(v.span, "ここで定数にした")
                .primary(f.span, "関数に定数と同じ名前はつけられない"));
        }
    }
    // 前の定数を使ってよい（後ろの定数や自分を使うとエラー）
    for i in 0..consts.len() {
        let (before, rest) = consts.split_at_mut(i);
        let (name, value) = &mut rest[0];
        let mut bad = None;
        walk_expr_mut(value, &mut |e| {
            if let ExprKind::Var(n) = &e.kind {
                match before.iter().find(|(b, _)| b == n) {
                    Some((_, v)) => {
                        let span = e.span;
                        *e = v.clone();
                        e.span = span;
                    }
                    None if bad.is_none() => bad = Some((n.clone(), e.span)),
                    None => {}
                }
            }
            true
        });
        if let Some((n, span)) = bad {
            return Err(Diagnostic::new(format!("定数 `{}` の値に使えない名前 `{}` があります", name, n))
                .primary(span, "定数の値には、数・文字列・真偽値と、それより前に書いた定数だけが使えます"));
        }
        if let Some(span) = not_constant(value) {
            return Err(Diagnostic::new(format!("定数 `{}` の値が定数ではありません", name))
                .primary(span, "定数の値には、数・文字列・真偽値と、その計算だけが書けます"));
        }
    }
    if !consts.is_empty() {
        for f in fns.iter_mut() {
            walk_block_mut(&mut f.body, &mut |e| {
                if let ExprKind::Var(n) = &e.kind {
                    if let Some((_, v)) = consts.iter().find(|(c, _)| c == n) {
                        let span = e.span;
                        *e = v.clone();
                        e.span = span;
                    }
                }
                true
            });
        }
    }
    Ok(consts.iter().map(|(n, _)| n.clone()).collect())
}

/// 定数の値として書けない式の位置（数・小数・文字列・真偽値と、それらの演算・`as` だけ書ける）
fn not_constant(e: &Expr) -> Option<Span> {
    match &e.kind {
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Str(_) | ExprKind::Bool(_) => None,
        ExprKind::Unary(_, x) | ExprKind::Cast(x, _) => not_constant(x),
        ExprKind::Binary(_, l, r) => not_constant(l).or_else(|| not_constant(r)),
        _ => Some(e.span),
    }
}

fn binop_info(t: &Tok) -> Option<(BinOp, u8)> {
    Some(match t {
        Tok::OrOr => (BinOp::Or, 1),
        Tok::AndAnd => (BinOp::And, 2),
        Tok::EqEq => (BinOp::Eq, 3),
        Tok::NotEq => (BinOp::Ne, 3),
        Tok::Lt => (BinOp::Lt, 4),
        Tok::Le => (BinOp::Le, 4),
        Tok::Gt => (BinOp::Gt, 4),
        Tok::Ge => (BinOp::Ge, 4),
        // ビット演算は比べるより強く、足し算より弱い（Rust と同じ順）
        Tok::Pipe => (BinOp::BitOr, 5),
        Tok::Caret => (BinOp::BitXor, 6),
        Tok::Amp => (BinOp::BitAnd, 7),
        Tok::Shl => (BinOp::Shl, 8),
        Tok::Plus => (BinOp::Add, 9),
        Tok::Minus => (BinOp::Sub, 9),
        Tok::Star => (BinOp::Mul, 10),
        Tok::Slash => (BinOp::Div, 10),
        Tok::Percent => (BinOp::Rem, 10),
        _ => return None,
    })
}

// ---------------------------------------------------------------- メソッドの中の名前

/// メソッドの本体で名前を書き換えるための情報
/// 構造体・enum のメソッドを、`型名.メソッド名` という名前の関数にする。
/// 本体のフィールド名・メソッド名を `this.名前` に書き換える。`taken` はメソッドにつけられない名前（フィールド・enum の種類）
fn finish_methods(
    name: &str,
    fields: &[String],
    taken: &[String],
    methods: Vec<(FnDef, Span)>,
    statics: Vec<(FnDef, Span)>,
) -> R<Vec<FnDef>> {
    let names: Vec<String> = methods.iter().map(|(m, _)| m.name.clone()).collect();
    let static_names: Vec<String> = statics.iter().map(|(m, _)| m.name.clone()).collect();
    let mut fns = Vec::new();
    let count = methods.len();
    for (i, (mut m, mspan)) in methods.into_iter().chain(statics).enumerate() {
        let is_static = i >= count;
        if fields.contains(&m.name) {
            return Err(Diagnostic::new(format!("メソッド `{}` がフィールドと同じ名前です", m.name))
                .primary(mspan, "別の名前にしてください"));
        }
        if taken.contains(&m.name) {
            return Err(Diagnostic::new(format!("メソッド `{}` が種類と同じ名前です", m.name))
                .primary(mspan, "別の名前にしてください"));
        }
        // メソッドの中では、フィールドと同じ型のメソッドを `this.` なしで使える。
        // 静的メソッドは `型名.` なしで使える（静的メソッドの中には this がない）
        let ctx = ThisCtx {
            fields: if is_static { &[] } else { fields },
            methods: if is_static { &[] } else { &names },
            statics: &static_names,
            owner: name,
        };
        let skip = if is_static { 0 } else { 1 };
        let mut scopes: Vec<Vec<String>> = vec![m.params.iter().skip(skip).map(|p| p.name.clone()).collect()];
        bind_this_block(&mut m.body, &ctx, &mut scopes);
        m.name = format!("{}.{}", name, m.name);
        fns.push(m);
    }
    Ok(fns)
}

struct ThisCtx<'a> {
    fields: &'a [String],
    methods: &'a [String],
    statics: &'a [String],
    /// 構造体の名前
    owner: &'a str,
}

/// メソッドの本体で、フィールドの名前を `this.名前` に、同じ構造体のメソッドの呼び出しを
/// `this.名前(...)` に書き換える（Java と同じく `this.` を省ける）。
/// 引数やローカル変数に同じ名前があれば、そちらが優先（`scopes` で見分ける）
fn bind_this_block(b: &mut Block, ctx: &ThisCtx, scopes: &mut Vec<Vec<String>>) {
    scopes.push(Vec::new());
    for s in &mut b.stmts {
        match s {
            Stmt::Let { name, init, .. } => {
                bind_this(init, ctx, scopes);
                scopes.last_mut().expect("scope").push(name.clone());
            }
            Stmt::Assign { target, value, .. } => {
                bind_this(value, ctx, scopes);
                bind_this(target, ctx, scopes);
            }
            Stmt::Expr(e) => bind_this(e, ctx, scopes),
            Stmt::While { cond, body } => {
                bind_this(cond, ctx, scopes);
                bind_this_block(body, ctx, scopes);
            }
            Stmt::Loop { body } => bind_this_block(body, ctx, scopes),
            Stmt::For { var, range, body, els, .. } => {
                for a in range {
                    bind_this(a, ctx, scopes);
                }
                scopes.push(vec![var.clone()]);
                bind_this_block(body, ctx, scopes);
                scopes.pop();
                if let Some(els) = els {
                    bind_this_block(els, ctx, scopes);
                }
            }
            Stmt::ForEach { key, var, iter, body, els, .. } => {
                bind_this(iter, ctx, scopes);
                scopes.push(key.iter().map(|(k, _)| k.clone()).chain(std::iter::once(var.clone())).collect());
                bind_this_block(body, ctx, scopes);
                scopes.pop();
                if let Some(els) = els {
                    bind_this_block(els, ctx, scopes);
                }
            }
            Stmt::Break(_) | Stmt::Continue(_) => {}
            Stmt::Return { value, .. } => {
                if let Some(e) = value {
                    bind_this(e, ctx, scopes);
                }
            }
        }
    }
    scopes.pop();
}

fn bind_this(e: &mut Expr, ctx: &ThisCtx, scopes: &mut Vec<Vec<String>>) {
    let local = |n: &str, scopes: &Vec<Vec<String>>| scopes.iter().flatten().any(|x| x == n);
    let this = |span: Span| Box::new(Expr::new(ExprKind::Var("this".to_string()), span));
    match &mut e.kind {
        ExprKind::Var(n) if ctx.fields.contains(n) && !local(n, scopes) => {
            let n = n.clone();
            e.kind = ExprKind::Field(this(e.span), n, Cell::new(Check::Unvisited));
        }
        ExprKind::Call(n, args) if ctx.statics.contains(n) => {
            for a in args.iter_mut() {
                bind_this(a, ctx, scopes);
            }
            *n = format!("{}.{}", ctx.owner, n);
        }
        ExprKind::Call(n, args) if ctx.methods.contains(n) => {
            for a in args.iter_mut() {
                bind_this(a, ctx, scopes);
            }
            let (n, args) = (n.clone(), std::mem::take(args));
            e.kind = ExprKind::MethodCall(this(e.span), n, args, RefCell::default());
        }
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::Str(_) | ExprKind::Unit | ExprKind::Var(_) => {}
        ExprKind::Cast(i, _)
        | ExprKind::Try(i)
        | ExprKind::Own(i)
        | ExprKind::Ref(i, _)
        | ExprKind::Deref(i, _)
        | ExprKind::Field(i, _, _)
        | ExprKind::Unary(_, i) => bind_this(i, ctx, scopes),
        ExprKind::Variant { args, .. } | ExprKind::Call(_, args) => {
            for a in args {
                bind_this(a, ctx, scopes);
            }
        }
        ExprKind::Interp(parts) => {
            for p in parts {
                if let InterpPart::Expr(x) = p {
                    bind_this(x, ctx, scopes);
                }
            }
        }
        ExprKind::Index(b, i, _) => {
            bind_this(b, ctx, scopes);
            bind_this(i, ctx, scopes);
        }
        ExprKind::ArrayLit(items, _) => {
            for x in items {
                bind_this(x, ctx, scopes);
            }
        }
        ExprKind::ArrayRepeat(x, _, _) => bind_this(x, ctx, scopes),
        ExprKind::MapLit(pairs) => {
            for (k, v) in pairs {
                bind_this(k, ctx, scopes);
                bind_this(v, ctx, scopes);
            }
        }
        ExprKind::Lambda(l) => {
            // メソッドの中のクロージャでも、フィールド名は this.名前 になる（this を取り込む）
            let l = Rc::get_mut(l).expect("lambdas are not shared while parsing");
            scopes.push(l.params.iter().map(|p| p.name.clone()).collect());
            bind_this_block(&mut l.body, ctx, scopes);
            scopes.pop();
        }
        ExprKind::MethodCall(recv, _, args, _) => {
            bind_this(recv, ctx, scopes);
            for a in args {
                bind_this(a, ctx, scopes);
            }
        }
        ExprKind::StructLit(_, inits) => {
            for i in inits {
                bind_this(&mut i.value, ctx, scopes);
            }
        }
        ExprKind::If { cond, then, els } => {
            bind_this(cond, ctx, scopes);
            bind_this_block(then, ctx, scopes);
            if let Some(b) = els {
                bind_this_block(b, ctx, scopes);
            }
        }
        ExprKind::Match { scrut, arms, default } => {
            bind_this(scrut, ctx, scopes);
            for a in arms {
                let binds = a
                    .pats
                    .iter()
                    .flat_map(|(p, _)| p.names().into_iter().map(|(b, _)| b))
                    .collect();
                scopes.push(binds);
                bind_this_block(&mut a.body, ctx, scopes);
                scopes.pop();
            }
            if let Some(d) = default {
                bind_this_block(d, ctx, scopes);
            }
        }
        ExprKind::Binary(_, l, r) => {
            bind_this(l, ctx, scopes);
            bind_this(r, ctx, scopes);
        }
    }
}

/// 関数・メソッドの引数にその場で書いたクロージャは、ムーブする外の変数を借りて取り込む
fn lambdas_borrow(args: &[Expr]) {
    for a in args {
        if let ExprKind::Lambda(l) = &a.kind {
            l.borrows.set(true);
        }
    }
}

/// `enum 名前 { ... }` ごとに、種類の名前（中かっこの 1 段目で、文の始めにある名前）を集める
fn scan_variants(toks: &[Token]) -> HashMap<String, Vec<String>> {
    let mut out = HashMap::new();
    for (k, w) in toks.windows(2).enumerate() {
        let (Tok::Enum, Tok::Ident(n)) = (&w[0].tok, &w[1].tok) else { continue };
        let Some(open) = toks[k..].iter().position(|t| t.tok == Tok::LBrace) else { continue };
        let mut depth = 0;
        let mut names = Vec::new();
        let mut at_start = false;
        for t in &toks[k + open..] {
            match &t.tok {
                Tok::LBrace => {
                    depth += 1;
                    at_start = depth == 1;
                    continue;
                }
                Tok::RBrace => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    at_start = depth == 1;
                    continue;
                }
                Tok::Semi if depth == 1 => {
                    at_start = true;
                    continue;
                }
                Tok::Ident(v) if depth == 1 && at_start => names.push(v.clone()),
                _ => {}
            }
            at_start = false;
        }
        out.insert(n.clone(), names);
    }
    out
}
