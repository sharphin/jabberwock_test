//! 文字型 char（`'a'`・比べる・`as`・`to_char`・判定のメソッド・辞書のキー・リスト・match）

use jabberwock::run_source;

fn ok(src: &str) -> String {
    let o = run_source(src, true);
    if let Some(d) = &o.error {
        panic!("unexpected error:\n{}", jabberwock::render(d, "main.jw", &o.files));
    }
    assert_eq!(o.live_objects, 0, "heap objects leaked");
    o.output
}

fn err(src: &str) -> String {
    let o = run_source(src, true);
    let d = o.error.expect("expected an error");
    jabberwock::render(&d, "main.jw", &o.files)
}

pub const CHARS: &str = r#"
struct Tok { char c; i64 n; }
fn char next(char c) { ((c as u32) + 1).to_char().unwrap_or('?') }
fn main() {
    let c = 'a';
    let char d = 'Z';
    print(c, d, '日', '\n' as u32, '\u{1F600}', '\'', '\\');
    print("x{c}y");
    print(c < d, c == 'a', c != d, c.to_upper(), d.to_lower(), '5'.is_digit(), ' '.is_space());
    print('q'.is_alpha(), '_'.is_alnum(), 'Q'.is_upper(), 'q'.is_lower(), '@'.is_alpha(), '['.is_alpha(), 'é'.is_alpha());
    let n = c as u32;
    print(n, next(c), 0xD800.to_char().is_none(), (-1).to_char().is_none(), 0x110000.to_char().is_none(), 0x10FFFF.to_char().is_some());
    let u8 b = 66;
    print(b as char, 'x'.to_str() + "!", 'é'.to_str().len(), 'é'.to_str().byte_len(), 'é' as u8);
    let char[] mut cs = ['c', 'a', 'b'];
    cs.sort();
    print(cs, cs.contains('b'), cs.index_of('c').unwrap_or(-1));
    let i64[char] mut m = [];
    m['x'] = 1;
    m['y'] = 2;
    print(m, m['y'], m.get('q').is_none());
    let t = Tok { c: 'k', n: 1 };
    print(t);
    for x in ['a', 'b', '0'] {
        match x {
            case 'a' -> print("is a");
            case 'b', '0' -> print("b or 0");
            default -> print("other");
        }
    }
    let Option<char> o = Some('w');
    print(o);
    print(m['z']);
}
"#;

#[test]
fn chars_work() {
    let o = run_source(CHARS, true);
    assert_eq!(
        o.output,
        "a Z 日 10 😀 ' \\\nxay\nfalse true true A z true true\ntrue false true true false false false\n97 b true true true true\nB x! 1 2 233\n[a, b, c] true 2\n[x: 1, y: 2] 2 true\nTok { c: k, n: 1 }\nis a\nb or 0\nb or 0\nSome(w)\n"
    );
    let shown = jabberwock::render(o.error.as_ref().expect("missing key"), "main.jw", &o.files);
    assert!(shown.contains("辞書にキー z がありません"), "{}", shown);
}

#[test]
fn chars_in_threads_and_closures() {
    let src = r#"
fn main() {
    let chan<char> ch = chan();
    let t = spawn(() -> { ch.send('z'); 'y' });
    let up = (char c) -> c.to_upper();
    print(ch.recv().unwrap_or('?'), up(t.join()));
}
"#;
    assert_eq!(ok(src), "z Y\n");
}

#[test]
fn char_errors() {
    let cases = [
        ("let c = 'a' + 1;", "文字は計算に使えません"),
        ("let c = -'a';", "文字は計算に使えません"),
        ("let c = 'a' << 1;", "文字は計算に使えません"),
        ("let char c = 97;", "文字と数は自動で変換しません"),
        ("let c = 'a'; let b = c == 97;", "文字と数は比べられません"),
        ("let i = 300; let c = i as char;", "`as char` にできるのは u8 だけです"),
        ("let c = 0xD800 as char;", "文字の番号ではありません"),
        ("let c = 'ab';", "文字が閉じていません"),
        ("let c = '';", "文字が空です"),
        ("let c = 'a' as f64;", "文字は f64 にできません"),
        ("let a = [1, 2]; let x = a['a'];", "添字は整数です"),
        ("let c = 'a'.foo();", "char にメソッド `foo` はありません"),
        ("let c = 'a'.is_digit(1);", "引数"),
        ("let c = '\\q';", "未知のエスケープです"),
        ("let i64 n = 'a';", "型が合いません"),
        ("for i in range('a', 'z') {}", "型が合いません"),
    ];
    for (body, want) in cases {
        let e = err(&format!("fn main() {{ {} }}", body));
        assert!(e.contains(want), "{}\n{}", body, e);
    }
}
