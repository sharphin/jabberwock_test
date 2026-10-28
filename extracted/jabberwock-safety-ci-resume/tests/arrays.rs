//! 配列のテスト

use jabberwock::{run_source, run_source_with};

fn ok(src: &str) -> String {
    let o = run_source(src, false);
    if let Some(d) = &o.error {
        panic!("unexpected error:\n{}", d.render("test.jw", src));
    }
    assert_eq!(o.live_objects, 0, "heap objects leaked");
    let off = run_source_with(src, false, false);
    assert_eq!(o.output, off.output, "elision changed the output");
    o.output
}

fn err(src: &str) -> jabberwock::diag::Diagnostic {
    run_source(src, false).error.expect("expected an error")
}

#[test]
fn literals_index_and_assignment() {
    let src = "
fn main() {
    let mut a = [1, 2, 3];
    a[0] = 10;
    a[2] += 5;
    print(a, a[0], a[1] + a[2], a.len());
    let u8[3] b = [250, 0, 1];
    print(b[0] + 10);
    let mut z = [0; 4];
    for i in range(0, z.len()) {
        z[i] = i * i;
    }
    print(z);
    let i64[0] empty = [];
    print(empty.len());
}";
    assert_eq!(ok(src), "[10, 2, 8] 10 10 3\n4\n[0, 1, 4, 9]\n0\n");
}

#[test]
fn arrays_of_structs_and_nested_arrays() {
    let src = "
struct P {
    i64 x;

    fn mut bump() { x++; }
}

fn main() {
    let mut ps = [P { x: 1 }, P { x: 2 }];
    ps[1].x = 20;
    ps[0].bump();
    print(ps[0].x, ps[1].x);
    let mut grid = [[0; 3]; 2];
    grid[1][2] = 7;
    print(grid);
}";
    assert_eq!(ok(src), "2 20\n[[0, 0, 0], [0, 0, 7]]\n");
}

#[test]
fn arrays_copy_or_move_like_their_elements() {
    let src = "
fn i64 sum(i64[3] a) {
    a[0] + a[1] + a[2]
}

fn main() {
    let a = [1, 2, 3];
    let b = a;
    print(sum(a), b[0]);
}";
    assert_eq!(ok(src), "6 1\n");
    // own を持つ配列はムーブ
    let d = err("
fn main() {
    let a = [own 1, own 2];
    let b = a;
    print(a);
}");
    assert!(d.title.contains("ムーブ"), "{}", d.title);
}

#[test]
fn arrays_of_own_borrow_and_free() {
    let src = "
fn main() {
    let mut a = [own 1, own 2, own 3];
    let r = ref a[1];
    print(*r);
    a[1] = own 20;
    print(*a[1]);
    let o = own [own 5, own 6];
    print(*o[0] + *o[1], o.len());
}";
    assert_eq!(ok(src), "2\n20\n11 2\n");
    // 要素を置き換えると、それを借りた ref は無効になる
    let d = err("
fn main() {
    let mut a = [own 1, own 2];
    let r = ref a[0];
    a[0] = own 9;
    print(*r);
}");
    assert!(d.title.contains("dangling"), "{}", d.title);
}

#[test]
fn constant_index_out_of_range_is_found_before_running() {
    let d = err("
fn main() {
    let a = [1, 2, 3];
    print(\"これは出ない\");
    print(a[3]);
}");
    assert!(d.title.contains("範囲の外"), "{}", d.title);
    let d = err("fn main() {\n    let mut a = [1, 2, 3];\n    a[1 + 2] = 0;\n}");
    assert!(d.title.contains("範囲の外"), "{}", d.title);
    let d = err("fn main() {\n    let a = [1, 2, 3];\n    print(a[-1]);\n}");
    assert!(d.title.contains("範囲の外"), "{}", d.title);
    // 定数でなければ実行時に見つける
    let o = run_source("fn main() {\n    let a = [1, 2, 3];\n    let i = 5;\n    print(\"ここまで\");\n    print(a[i]);\n}", false);
    assert_eq!(o.output, "ここまで\n");
    assert!(o.error.unwrap().title.contains("範囲の外"));
}

#[test]
fn array_errors() {
    let e = |body: &str| err(&format!("fn main() {{\n{}\n}}", body));
    assert!(e("let a = [1, \"x\"];").title.contains("型が合いません"));
    assert!(e("let a = [];").title.contains("空の配列"));
    assert!(e("let i64[2] a = [1, 2, 3];").title.contains("型が合いません"));
    assert!(e("let a = [own 1; 3];").title.contains("複製できない"));
    assert!(e("let a = [1, 2];\nlet b = a[true];").title.contains("添字は整数"));
    assert!(e("let x = 1;\nprint(x[0]);").title.contains("配列とリストだけ"));
    assert!(e("let a = [1, 2];\na[0] = 5;").title.contains("mut ではない"));
    assert!(e("let a = [own 1];\nlet b = a[0];").title.contains("取り出せません"));
    assert!(e("let a = [1];\nprint(a.push(2));").title.contains("メソッド `push` はありません"));
}

#[test]
fn arrays_are_at_most_one_mebibyte() {
    // ちょうど 1 MiB（i64 が 131072 個）までは作れる。実行ファイルでもスタックに置ける
    assert_eq!(ok("fn main() {\n    let mut a = [0; 131072];\n    a[131071] = 7;\n    print(a.len(), a[131071]);\n}"), "131072 7\n");
    let e = |src: &str| err(src).title;
    assert_eq!(e("fn main() { let a = [0; 131073]; }"), "配列が大きすぎます");
    assert_eq!(e("fn main() { let a = [0; 9223372036854775807]; }"), "配列が大きすぎます");
    assert_eq!(e("fn main() { let a = [[0; 1000]; 1000]; }"), "配列が大きすぎます");
    assert_eq!(e("fn f(i64[200000] a) {}\nfn main() {}"), "配列が大きすぎます");
    // 構造体と enum の中の配列も数える
    assert_eq!(e("struct S { i64[100000] a; }\nfn main() { let S[2] b = [S { a: [0; 100000] }, S { a: [0; 100000] }]; }"), "配列が大きすぎます");
    assert_eq!(e("fn main() { let Option<i64[100000]>[2] b = [None, None]; }"), "配列が大きすぎます");
    // リストは 2^62 バイトまで（それより小さければ作れる）
    assert_eq!(e("fn main() { let i64[] a = [0; 9223372036854775807]; }"), "リストが大きすぎます");
    assert_eq!(ok("fn main() {\n    let i64[] a = [1; 1000000];\n    print(a.len());\n}"), "1000000\n");
}
