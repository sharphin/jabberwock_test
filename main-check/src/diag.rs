//! 診断メッセージ（エラー表示）。
//!
//! 「ここで借りた → ここで解放された → ここで使った」を時系列で見せるために、
//! 1つのエラーが複数のラベル付き位置を持てるようにしている。
//! これがこの言語の「エラーとの付き合いやすさ」の土台。

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    /// どのファイルか（0 が最初に渡したファイル。モジュールは読んだ順に 1, 2, ...）
    pub file: usize,
    pub line: usize, // 1始まり
    pub col: usize,  // 1始まり（文字単位）
    pub len: usize,
}

impl Span {
    /// 同じ行にある2つの範囲をつなぐ。別の行なら self をそのまま返す。
    pub fn join(self, other: Span) -> Span {
        if self.file == other.file && self.line == other.line && other.col + other.len > self.col {
            Span {
                file: self.file,
                line: self.line,
                col: self.col,
                len: other.col + other.len - self.col,
            }
        } else {
            self
        }
    }
}

#[derive(Debug, Clone)]
pub struct Label {
    pub span: Span,
    pub text: String,
    pub primary: bool,
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub title: String,
    pub labels: Vec<Label>,
    pub note: Option<String>,
    /// 警告か（実行は止めない）
    pub warning: bool,
}

impl Diagnostic {
    pub fn new(title: impl Into<String>) -> Self {
        Diagnostic {
            title: title.into(),
            labels: Vec::new(),
            note: None,
            warning: false,
        }
    }

    /// 警告（実行は止めない）
    pub fn warning(title: impl Into<String>) -> Self {
        Diagnostic { warning: true, ..Diagnostic::new(title) }
    }

    /// 補足の位置（過去の出来事など）
    pub fn label(mut self, span: Span, text: impl Into<String>) -> Self {
        self.labels.push(Label {
            span,
            text: text.into(),
            primary: false,
        });
        self
    }

    /// 問題が表面化した位置
    pub fn primary(mut self, span: Span, text: impl Into<String>) -> Self {
        self.labels.push(Label {
            span,
            text: text.into(),
            primary: true,
        });
        self
    }

    pub fn note(mut self, text: impl Into<String>) -> Self {
        self.note = Some(text.into());
        self
    }

    pub fn render(&self, file: &str, src: &str) -> String {
        self.render_in(&[(file, src)])
    }

    /// 複数のファイル（`Span.file` の番号の順）にまたがる診断を出す
    pub fn render_in(&self, files: &[(&str, &str)]) -> String {
        let file_of = |i: usize| files.get(i).or(files.first()).copied().unwrap_or(("?", ""));
        let kind = if self.warning { "warning" } else { "error" };
        let mut out = format!("{}: {}\n", kind, self.title);

        let mut labels: Vec<&Label> = self.labels.iter().collect();
        // 主な位置のファイルを先に、あとはファイルごとに行の順
        let main_file = self.labels.iter().find(|l| l.primary).or(self.labels.first()).map_or(0, |l| l.span.file);
        labels.sort_by_key(|l| (l.span.file != main_file, l.span.file, l.span.line, l.span.col));

        let width = labels
            .iter()
            .map(|l| l.span.line)
            .max()
            .map(|n| n.to_string().len())
            .unwrap_or(1);
        let pad = " ".repeat(width);

        if let Some(p) = self.labels.iter().find(|l| l.primary).or(self.labels.first()) {
            out.push_str(&format!("{pad}--> {}:{}:{}\n", file_of(p.span.file).0, p.span.line, p.span.col));
            out.push_str(&format!("{pad} |\n"));
        }

        let mut last_line = 0usize;
        let mut cur_file = main_file;
        for l in &labels {
            if l.span.file != cur_file {
                // 別のファイルの位置
                cur_file = l.span.file;
                last_line = 0;
                out.push_str(&format!("{pad}--> {}:{}:{}\n", file_of(cur_file).0, l.span.line, l.span.col));
                out.push_str(&format!("{pad} |\n"));
            }
            let lines: Vec<&str> = file_of(cur_file).1.lines().collect();
            let ln = l.span.line;
            let text_line: Vec<char> = lines
                .get(ln.saturating_sub(1))
                .copied()
                .unwrap_or("")
                .replace('\t', " ")
                .chars()
                .collect();

            if ln != last_line {
                if last_line != 0 && ln > last_line + 1 {
                    out.push_str(&format!("{pad} ...\n"));
                }
                let shown: String = text_line.iter().collect();
                out.push_str(&format!("{:>width$} | {}\n", ln, shown, width = width));
                last_line = ln;
            }

            let start = l.span.col.saturating_sub(1);
            let lead: usize = text_line.iter().take(start).map(|c| display_width(*c)).sum();
            let mark: usize = text_line
                .iter()
                .skip(start)
                .take(l.span.len.max(1))
                .map(|c| display_width(*c))
                .sum::<usize>()
                .max(1);
            let ch = if l.primary { "^" } else { "-" };
            out.push_str(&format!(
                "{pad} | {}{} {}\n",
                " ".repeat(lead),
                ch.repeat(mark),
                l.text
            ));
        }

        if let Some(n) = &self.note {
            out.push_str(&format!("{pad} |\n{pad} = note: {}\n", n));
        }
        out
    }
}

/// 全角文字を2桁として数える簡易版（下線の位置合わせ用）
fn display_width(c: char) -> usize {
    if c.is_ascii() {
        1
    } else {
        2
    }
}
