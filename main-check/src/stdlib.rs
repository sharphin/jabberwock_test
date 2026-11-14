//! 標準ライブラリ: jabberwock で書いたモジュール（リポジトリの `std/`）をコンパイラの中に入れておく。
//! `use std.名前;` で読む（`modules::Loader`）。jwc は同じものを jwc/stdlib.jw（作り直すのは
//! `python3 scripts/sync_stdlib.py`）から読む。
//! Rust 版は凍結なので、jwc だけの機能（資源・ネットワークなど）を使うモジュールは読めても型検査で止まる。

/// (モジュールの名前, ソース)。名前の順
pub const MODULES: &[(&str, &str)] = &[
    ("base64", include_str!("../std/base64.jw")),
    ("browser", include_str!("../std/browser.jw")),
    ("cli", include_str!("../std/cli.jw")),
    ("clipboard", include_str!("../std/clipboard.jw")),
    ("crypto", include_str!("../std/crypto.jw")),
    ("csv", include_str!("../std/csv.jw")),
    ("date", include_str!("../std/date.jw")),
    ("deque", include_str!("../std/deque.jw")),
    ("env", include_str!("../std/env.jw")),
    ("fs", include_str!("../std/fs.jw")),
    ("gzip", include_str!("../std/gzip.jw")),
    ("hash", include_str!("../std/hash.jw")),
    ("heap", include_str!("../std/heap.jw")),
    ("http", include_str!("../std/http.jw")),
    ("io", include_str!("../std/io.jw")),
    ("json", include_str!("../std/json.jw")),
    ("lists", include_str!("../std/lists.jw")),
    ("log", include_str!("../std/log.jw")),
    ("math", include_str!("../std/math.jw")),
    ("net", include_str!("../std/net.jw")),
    ("pg", include_str!("../std/pg.jw")),
    ("process", include_str!("../std/process.jw")),
    ("rand", include_str!("../std/rand.jw")),
    ("random", include_str!("../std/random.jw")),
    ("regex", include_str!("../std/regex.jw")),
    ("sets", include_str!("../std/sets.jw")),
    ("text", include_str!("../std/text.jw")),
    ("time", include_str!("../std/time.jw")),
    ("tls", include_str!("../std/tls.jw")),
    ("url", include_str!("../std/url.jw")),
];

/// `std.名前` のソース
pub fn source(name: &str) -> Option<&'static str> {
    MODULES.iter().find(|(n, _)| *n == name).map(|(_, s)| *s)
}

/// 標準ライブラリのモジュールの名前
pub fn names() -> Vec<&'static str> {
    MODULES.iter().map(|(n, _)| *n).collect()
}
