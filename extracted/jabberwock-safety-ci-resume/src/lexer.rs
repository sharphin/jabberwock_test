use crate::diag::{Diagnostic, Span};

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    /// 小数（`1.5`）。書いたままの文字
    Float(String),
    Int(i128),
    Str(String),
    /// `{...}` の埋め込みがある文字列 `"x = {x}"`
    Interp(Vec<StrPart>),
    Ident(String),
    // キーワード
    Fn,
    Let,
    If,
    Else,
    While,
    Loop,
    For,
    In,
    Break,
    Continue,
    Match,
    Case,
    Default,
    Return,
    Struct,
    Enum,
    Static,
    Interface,
    Use,
    Pub,
    Const,
    Own,
    As,
    Ref,
    Mut,
    True,
    False,
    // 記号
    LParen,
    LBrack,
    RBrack,
    RParen,
    LBrace,
    RBrace,
    Comma,
    Arrow,
    Dot,
    Colon,
    Semi,
    Assign,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    PlusPlus,
    MinusMinus,
    EqEq,
    NotEq,
    Lt,
    Le,
    Gt,
    Ge,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Bang,
    Question,
    AndAnd,
    OrOr,
    Amp,
    Pipe,
    Caret,
    Tilde,
    Shl,
    Eof,
}

impl Tok {
    /// エラーメッセージ用の表示
    pub fn describe(&self) -> String {
        match self {
            Tok::Int(n) => format!("整数 `{}`", n),
            Tok::Float(t) => format!("小数 `{}`", t),
            Tok::Str(_) | Tok::Interp(_) => "文字列".to_string(),
            Tok::Ident(s) => format!("名前 `{}`", s),
            Tok::Eof => "ファイルの終わり".to_string(),
            other => format!("`{}`", other.symbol()),
        }
    }

    fn symbol(&self) -> &'static str {
        match self {
            Tok::Fn => "fn",
            Tok::Let => "let",
            Tok::If => "if",
            Tok::Else => "else",
            Tok::While => "while",
            Tok::Loop => "loop",
            Tok::For => "for",
            Tok::In => "in",
            Tok::Break => "break",
            Tok::Continue => "continue",
            Tok::Match => "match",
            Tok::Case => "case",
            Tok::Default => "default",
            Tok::Return => "return",
            Tok::Struct => "struct",
            Tok::Enum => "enum",
            Tok::Static => "static",
            Tok::Interface => "interface",
            Tok::Use => "use",
            Tok::Pub => "pub",
            Tok::Const => "const",
            Tok::Own => "own",
            Tok::As => "as",
            Tok::Ref => "ref",
            Tok::Mut => "mut",
            Tok::True => "true",
            Tok::False => "false",
            Tok::LParen => "(",
            Tok::LBrack => "[",
            Tok::RBrack => "]",
            Tok::RParen => ")",
            Tok::LBrace => "{",
            Tok::RBrace => "}",
            Tok::Comma => ",",
            Tok::Arrow => "->",
            Tok::Dot => ".",
            Tok::Colon => ":",
            Tok::Semi => ";",
            Tok::Assign => "=",
            Tok::PlusEq => "+=",
            Tok::MinusEq => "-=",
            Tok::StarEq => "*=",
            Tok::SlashEq => "/=",
            Tok::PercentEq => "%=",
            Tok::PlusPlus => "++",
            Tok::MinusMinus => "--",
            Tok::EqEq => "==",
            Tok::NotEq => "!=",
            Tok::Lt => "<",
            Tok::Le => "<=",
            Tok::Gt => ">",
            Tok::Ge => ">=",
            Tok::Plus => "+",
            Tok::Minus => "-",
            Tok::Star => "*",
            Tok::Slash => "/",
            Tok::Percent => "%",
            Tok::Bang => "!",
            Tok::Question => "?",
            Tok::AndAnd => "&&",
            Tok::OrOr => "||",
            Tok::Amp => "&",
            Tok::Pipe => "|",
            Tok::Caret => "^",
            Tok::Tilde => "~",
            Tok::Shl => "<<",
            _ => "?",
        }
    }
}

/// 埋め込みのある文字列の部品
#[derive(Debug, Clone, PartialEq)]
pub enum StrPart {
    Lit(String),
    /// `{` と `}` の中の式のトークン（最後は Eof）
    Code(Vec<Token>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
}

/// `file` 番目のファイルとして字句解析する（位置にファイルの番号をつける）
pub fn lex_file(src: &str, file: usize) -> Result<Vec<Token>, Diagnostic> {
    fn mark(toks: &mut [Token], file: usize) {
        for t in toks {
            t.span.file = file;
            if let Tok::Interp(parts) = &mut t.tok {
                for p in parts {
                    if let StrPart::Code(inner) = p {
                        mark(inner, file);
                    }
                }
            }
        }
    }
    match lex(src) {
        Ok(mut toks) => {
            mark(&mut toks, file);
            Ok(toks)
        }
        Err(mut d) => {
            for l in &mut d.labels {
                l.span.file = file;
            }
            Err(d)
        }
    }
}

pub fn lex(src: &str) -> Result<Vec<Token>, Diagnostic> {
    let chars: Vec<char> = src.chars().collect();
    let mut out: Vec<Token> = Vec::new();
    let mut i = 0usize;
    let mut line = 1usize;
    let mut col = 1usize;

    while i < chars.len() {
        let c = chars[i];
        let start_col = col;

        match c {
            ' ' | '\t' | '\r' => {
                i += 1;
                col += 1;
            }
            '\n' => {
                // 文の区切りは `;`。改行はただの空白
                i += 1;
                line += 1;
                col = 1;
            }
            '/' if chars.get(i + 1) == Some(&'/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                    col += 1;
                }
            }
            // `0xff`・`0b1010`・`0o17`（`_` で区切ってよい）
            '0' if matches!(chars.get(i + 1), Some('x' | 'b' | 'o')) => {
                let radix = match chars[i + 1] {
                    'x' => 16,
                    'b' => 2,
                    _ => 8,
                };
                let s = i;
                i += 2;
                col += 2;
                let mut digits = String::new();
                while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    if chars[i] != '_' {
                        digits.push(chars[i]);
                    }
                    i += 1;
                    col += 1;
                }
                let span = Span { file: 0, line, col: start_col, len: i - s };
                if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
                    return Err(Diagnostic::new("数の書き方が正しくありません")
                        .primary(span, format!("{} 進数に使えない文字があります", radix)));
                }
                let n = u64::from_str_radix(&digits, radix)
                    .map(i128::from)
                    .map_err(|_| Diagnostic::new("整数が大きすぎます").primary(span, "どの整数型にも収まりません"))?;
                out.push(Token { tok: Tok::Int(n), span });
            }
            '0'..='9' => {
                let s = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || (chars[i] == '_' && i > s)) {
                    i += 1;
                    col += 1;
                }
                // `1.5` は小数（`.` のあとに数字が続くときだけ）
                if chars.get(i) == Some(&'.') && matches!(chars.get(i + 1), Some(c) if c.is_ascii_digit()) {
                    i += 1;
                    col += 1;
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                        col += 1;
                    }
                    let text: String = chars[s..i].iter().collect();
                    let span = Span { file: 0, line, col: start_col, len: i - s };
                    out.push(Token { tok: Tok::Float(text), span });
                    continue;
                }
                let text: String = chars[s..i].iter().filter(|c| **c != '_').collect();
                let span = Span { file: 0, line, col: start_col, len: i - s };
                // 型は型検査が決める（相手の型に合わせる）。ここでは u64 に収まるかだけ見る
                let n = text
                    .parse::<u64>()
                    .map(i128::from)
                    .map_err(|_| Diagnostic::new("整数が大きすぎます").primary(span, "どの整数型にも収まりません"))?;
                out.push(Token { tok: Tok::Int(n), span });
            }
            c if c.is_alphabetic() || c == '_' => {
                let s = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                    col += 1;
                }
                let text: String = chars[s..i].iter().collect();
                let tok = match text.as_str() {
                    "fn" => Tok::Fn,
                    "let" => Tok::Let,
                    "if" => Tok::If,
                    "else" => Tok::Else,
                    "while" => Tok::While,
                    "loop" => Tok::Loop,
                    "for" => Tok::For,
                    "in" => Tok::In,
                    "break" => Tok::Break,
                    "continue" => Tok::Continue,
                    "match" => Tok::Match,
                    "case" => Tok::Case,
                    "default" => Tok::Default,
                    "return" => Tok::Return,
                    "struct" => Tok::Struct,
                    "enum" => Tok::Enum,
                    "static" => Tok::Static,
                    "interface" => Tok::Interface,
                    "use" => Tok::Use,
                    "pub" => Tok::Pub,
                    "const" => Tok::Const,
                    "own" => Tok::Own,
                    "as" => Tok::As,
                    "ref" => Tok::Ref,
                    "mut" => Tok::Mut,
                    "true" => Tok::True,
                    "false" => Tok::False,
                    _ => Tok::Ident(text),
                };
                out.push(Token {
                    tok,
                    span: Span { file: 0, line, col: start_col, len: i - s },
                });
            }
            '"' => {
                // `"""..."""` は途中で改行できる（開きの直後の改行は含めない）
                let triple = chars.get(i + 1) == Some(&'"') && chars.get(i + 2) == Some(&'"');
                let start_line = line;
                let open = if triple { 3 } else { 1 };
                i += open;
                col += open;
                if triple && chars.get(i) == Some(&'\n') {
                    i += 1;
                    line += 1;
                    col = 1;
                }
                let mut text = String::new();
                let mut parts: Vec<StrPart> = Vec::new();
                loop {
                    if i >= chars.len() || (chars[i] == '\n' && !triple) {
                        let why = if triple {
                            "この `\"\"\"` に対応する閉じ `\"\"\"` がありません"
                        } else {
                            "この `\"` に対応する閉じ `\"` が同じ行にありません（改行を含むなら `\"\"\"...\"\"\"`）"
                        };
                        return Err(Diagnostic::new("文字列が閉じていません")
                            .primary(Span { file: 0, line: start_line, col: start_col, len: open }, why));
                    }
                    let ch = chars[i];
                    if ch == '\n' {
                        text.push('\n');
                        i += 1;
                        line += 1;
                        col = 1;
                        continue;
                    }
                    if ch == '"' && (!triple || (chars.get(i + 1) == Some(&'"') && chars.get(i + 2) == Some(&'"'))) {
                        i += open;
                        col += open;
                        break;
                    }
                    if (ch == '{' || ch == '}') && chars.get(i + 1) == Some(&ch) {
                        // `{{` / `}}` は `{` / `}` そのもの
                        text.push(ch);
                        i += 2;
                        col += 2;
                        continue;
                    }
                    if ch == '}' {
                        return Err(Diagnostic::new("文字列の中の `}` に対応する `{` がありません")
                            .primary(Span { file: 0, line, col, len: 1 }, "`}` そのものを書くなら `}}` と書いてください"));
                    }
                    if ch == '{' {
                        // 埋め込み: 対応する `}` までを式として字句解析する
                        let open_col = col;
                        let mut depth = 1;
                        let mut j = i + 1;
                        while j < chars.len() && chars[j] != '\n' {
                            match chars[j] {
                                '{' => depth += 1,
                                '}' => {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                                '"' => {
                                    return Err(Diagnostic::new("埋め込みの `{` が閉じていません")
                                        .primary(Span { file: 0, line, col: open_col, len: 1 }, "この `{` の `}` より先に `\"` がある")
                                        .note("`{` そのものは `{{` と書きます。`{ }` の中には文字列を書けないので、先に変数に入れてください"))
                                }
                                _ => {}
                            }
                            j += 1;
                        }
                        if j >= chars.len() || chars[j] != '}' {
                            return Err(Diagnostic::new("埋め込みの `{` が閉じていません").primary(
                                Span { file: 0, line, col: open_col, len: 1 },
                                "`{` そのものを書くなら `{{` と書いてください",
                            ));
                        }
                        let code: String = chars[i + 1..j].iter().collect();
                        if code.trim().is_empty() {
                            return Err(Diagnostic::new("埋め込みの `{ }` が空です")
                                .primary(Span { file: 0, line, col: open_col, len: j - i + 1 }, "中に式を書いてください"));
                        }
                        let mut toks = lex(&code)?;
                        for t in &mut toks {
                            // 部分の字句解析の位置を、元のファイルでの位置に直す
                            t.span = Span { file: 0, line, col: t.span.col + open_col, len: t.span.len };
                        }
                        if !text.is_empty() {
                            parts.push(StrPart::Lit(std::mem::take(&mut text)));
                        }
                        parts.push(StrPart::Code(toks));
                        col += j - i + 1;
                        i = j + 1;
                        continue;
                    }
                    if ch == '\\' {
                        let esc = chars.get(i + 1).copied();
                        // `\u{1F600}`: 番号で書いた文字
                        if esc == Some('u') && chars.get(i + 2) == Some(&'{') {
                            let close = (i + 3..chars.len()).find(|&k| chars[k] == '}' || chars[k] == '"' || chars[k] == '\n');
                            let code = close.filter(|&k| chars[k] == '}').and_then(|k| {
                                let hex: String = chars[i + 3..k].iter().collect();
                                u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32).map(|c| (c, k))
                            });
                            let Some((c, k)) = code else {
                                return Err(Diagnostic::new("文字の番号のエスケープが正しくありません").primary(
                                    Span { file: 0, line, col, len: 2 },
                                    "`\\u{41}` のように、{ } の中に 16 進数で文字の番号を書きます",
                                ));
                            };
                            text.push(c);
                            col += k + 1 - i;
                            i = k + 1;
                            continue;
                        }
                        let mapped = match esc {
                            Some('n') => '\n',
                            Some('t') => '\t',
                            Some('r') => '\r',
                            Some('0') => '\0',
                            Some('"') => '"',
                            Some('\\') => '\\',
                            _ => {
                                return Err(Diagnostic::new("未知のエスケープです").primary(
                                    Span { file: 0, line, col, len: 2 },
                                    "使えるのは \\n \\t \\r \\0 \\\" \\\\ \\u{...} です",
                                ));
                            }
                        };
                        text.push(mapped);
                        i += 2;
                        col += 2;
                    } else {
                        text.push(ch);
                        i += 1;
                        col += 1;
                    }
                }
                let tok = if parts.is_empty() {
                    Tok::Str(text)
                } else {
                    if !text.is_empty() {
                        parts.push(StrPart::Lit(text));
                    }
                    Tok::Interp(parts)
                };
                let len = if line == start_line { col - start_col } else { open };
                out.push(Token { tok, span: Span { file: 0, line: start_line, col: start_col, len } });
            }
            _ => {
                let next = chars.get(i + 1).copied();
                let (tok, n) = match (c, next) {
                    ('=', Some('=')) => (Tok::EqEq, 2),
                    ('!', Some('=')) => (Tok::NotEq, 2),
                    ('<', Some('=')) => (Tok::Le, 2),
                    ('<', Some('<')) => (Tok::Shl, 2),
                    ('>', Some('=')) => (Tok::Ge, 2),
                    ('&', Some('&')) => (Tok::AndAnd, 2),
                    ('|', Some('|')) => (Tok::OrOr, 2),
                    ('-', Some('>')) => (Tok::Arrow, 2),
                    ('+', Some('=')) => (Tok::PlusEq, 2),
                    ('-', Some('=')) => (Tok::MinusEq, 2),
                    ('*', Some('=')) => (Tok::StarEq, 2),
                    ('/', Some('=')) => (Tok::SlashEq, 2),
                    ('%', Some('=')) => (Tok::PercentEq, 2),
                    ('+', Some('+')) => (Tok::PlusPlus, 2),
                    ('-', Some('-')) => (Tok::MinusMinus, 2),
                    ('=', _) => (Tok::Assign, 1),
                    ('<', _) => (Tok::Lt, 1),
                    ('>', _) => (Tok::Gt, 1),
                    ('(', _) => (Tok::LParen, 1),
                    ('[', _) => (Tok::LBrack, 1),
                    (']', _) => (Tok::RBrack, 1),
                    (')', _) => (Tok::RParen, 1),
                    ('{', _) => (Tok::LBrace, 1),
                    ('}', _) => (Tok::RBrace, 1),
                    (',', _) => (Tok::Comma, 1),
                    ('.', _) => (Tok::Dot, 1),
                    (':', _) => (Tok::Colon, 1),
                    (';', _) => (Tok::Semi, 1),
                    ('+', _) => (Tok::Plus, 1),
                    ('-', _) => (Tok::Minus, 1),
                    ('*', _) => (Tok::Star, 1),
                    ('/', _) => (Tok::Slash, 1),
                    ('%', _) => (Tok::Percent, 1),
                    ('!', _) => (Tok::Bang, 1),
                    ('?', _) => (Tok::Question, 1),
                    ('&', _) => (Tok::Amp, 1),
                    ('|', _) => (Tok::Pipe, 1),
                    ('^', _) => (Tok::Caret, 1),
                    ('~', _) => (Tok::Tilde, 1),
                    _ => {
                        return Err(Diagnostic::new("使えない文字です").primary(
                            Span { file: 0, line, col, len: 1 },
                            format!("`{}` はこの言語にありません", c),
                        ));
                    }
                };
                out.push(Token {
                    tok,
                    span: Span { file: 0, line, col: start_col, len: n },
                });
                i += n;
                col += n;
            }
        }
    }

    out.push(Token {
        tok: Tok::Eof,
        span: Span { file: 0, line, col, len: 1 },
    });
    Ok(out)
}
