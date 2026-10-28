//! リスト `i64[]` と `for x in a` のテスト

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
fn lists_grow_and_shrink() {
    let src = "
fn main() {
    let i64[] mut v = [];
    for i in range(5) {
        v.push(i * 10);
    }
    print(v, v.len(), v[2]);
    v[0] = 7;
    print(v.pop(), v.pop(), v.len());
    let str[] mut names = [\"a\", \"b\"];
    names.push(\"c\");
    print(names);
    let i64[] mut e = [];
    print(e.pop());
    let i64[] z = [0; 3];
    print(z);
}";
    assert_eq!(ok(src), "[0, 10, 20, 30, 40] 5 20\nSome(40) Some(30) 3\n[a, b, c]\nNone\n[0, 0, 0]\n");
}

#[test]
fn lists_of_own_and_passing_lists() {
    let src = "
fn add(ref mut i64[] v, i64 n) {
    v.push(n);
}

fn i64[] make() {
    [1, 2, 3]
}

fn main() {
    let own i64[] mut v = own make();
    add(ref mut v, 4);
    print(v.len(), v[3]);
    let (own i64)[] mut owners = [];
    owners.push(own 1);
    owners.push(own 2);
    let r = ref owners[0];
    print(*r);
    match owners.pop() {
        case Some(p) -> print(*p);
        case None -> print(\"空\");
    }
    print(owners.len(), make().len());
}";
    assert_eq!(ok(src), "4 4\n1\n2\n1 3\n");
}

#[test]
fn lists_always_move() {
    let d = err("
fn main() {
    let i64[] a = [1, 2];
    let b = a;
    print(a);
}");
    assert!(d.title.contains("ムーブ"), "{}", d.title);
}

#[test]
fn for_each_over_arrays_and_lists() {
    let src = "
fn main() {
    let a = [1, 2, 3];
    let mut total = 0;
    for x in a {
        total += x;
    }
    print(total, a);
    let str[] mut words = [\"x\", \"y\"];
    words.push(\"z\");
    for w in words {
        print(\"word {w}\");
    }
    let i64[] empty = [];
    for x in empty {
        print(x);
    } else {
        print(\"空でした\");
    }
    for x in [10, 20, 30] {
        if x == 20 {
            continue;
        }
        print(x);
    }
}";
    assert_eq!(ok(src), "6 [1, 2, 3]\nword x\nword y\nword z\n空でした\n10\n30\n");
}

#[test]
fn for_each_borrows_own_elements() {
    let src = "
fn main() {
    let ps = [own 1, own 2, own 3];
    let mut sum = 0;
    for p in ps {
        sum += *p;
        if *p == 2 {
            break;
        }
    }
    print(sum);
    let o = own [own 5, own 6];
    for p in o {
        print(*p);
    }
}";
    assert_eq!(ok(src), "3\n5\n6\n");
    // ループの中で要素を取り出すと、その要素を借りた ref は無効になる
    let o = run_source("
fn main() {
    let (own i64)[] mut v = [own 1, own 2];
    for p in v {
        v.pop();
        print(*p);
    }
}", false);
    assert_eq!(o.output, "1\n");
    assert!(o.error.unwrap().title.contains("dangling"));
    assert_eq!(o.live_objects, 0);
}

#[test]
fn list_errors() {
    let e = |body: &str| err(&format!("fn main() {{\n{}\n}}", body));
    assert!(e("let i64[] v = [];\nv.push(1);").title.contains("mut ではない"));
    assert!(e("let i64[] mut v = [];\nv.push(\"a\");").title.contains("型が合いません"));
    assert!(e("let i64[] mut v = [];\nv.shuffle();").title.contains("メソッド `shuffle` はありません"));
    assert!(e("let i64[] v = [1, 2];\nlet i64[2] w = v;").title.contains("型が合いません"));
    assert!(e("for x in 5 {\n}").title.contains("配列・リスト・辞書"));
    assert!(e("let a = [1];\nfor x in a {\n    x = 2;\n}").title.contains("mut ではない"));
    // 構造体の要素は読むだけ
    let d = err("
struct P {
    i64 x;
}
fn main() {
    let ps = [P { x: 1 }];
    for p in ps {
        p.x = 2;
    }
}");
    assert!(d.title.contains("読むだけの ref"), "{}", d.title);
    assert!(e("let i64[] mut v = [];\nv.insert(\"a\", 1);").title.contains("添字は整数"));
    assert!(e("let i64[] mut v = [];\nv.remove();").title.contains("引数"));
    let o = run_source("fn main() {\n    let i64[] mut v = [1];\n    v.insert(3, 1);\n}", false);
    assert!(o.error.unwrap().title.contains("insert の位置"));
    assert_eq!(o.live_objects, 0);
    let o = run_source("fn main() {\n    let i64[] mut v = [1];\n    v.remove(1);\n}", false);
    assert!(o.error.unwrap().title.contains("範囲の外"));
    // リストの範囲外は実行時エラー
    let o = run_source("fn main() {\n    let i64[] v = [1];\n    print(v[3]);\n}", false);
    assert!(o.error.unwrap().title.contains("範囲の外"));
}

#[test]
fn insert_and_remove() {
    let src = "
fn main() {
    let i64[] mut v = [1, 2, 3];
    v.insert(0, 0);
    v.insert(4, 9);
    print(v);
    print(v.remove(1), v);
    let (own i64)[] mut boxes = [own 1, own 2];
    let b = boxes.remove(0);
    boxes.insert(1, own 3);
    print(*b, boxes.len(), *boxes[1]);
}";
    assert_eq!(ok(src), "[0, 1, 2, 3, 9]\n1 [0, 2, 3, 9]\n1 2 3\n");
}

#[test]
fn for_each_over_structs_reads_a_snapshot() {
    let src = "
struct C {
    str w;
    own i64 b;
}

fn main() {
    let C[] mut cs = [C { w: \"a\", b: own 10 }, C { w: \"b\", b: own 20 }];
    for c in cs {
        print(c.w, *c.b);
    }
    let mut total = 0;
    for c in cs {
        total += *c.b;
    }
    print(total);
}";
    assert_eq!(ok(src), "a 10\nb 20\n30\n");
    // ループの中で要素を取り除くと、その要素の own は解放され、たどるとダングリング
    let o = run_source("
struct C {
    own i64 b;
}

fn main() {
    let C[] mut cs = [C { b: own 1 }];
    for c in cs {
        cs.remove(0);
        print(*c.b);
    }
}", false);
    assert!(o.error.unwrap().title.contains("dangling"));
    assert_eq!(o.live_objects, 0);
}

#[test]
fn drain_and_reverse() {
    // `for x in a.drain()` は要素を前から値として受け取る（a は空になる）。途中で抜けたら残りは捨てる
    let src = "struct Job { i64 id; own i64 data; }
fn Job[] make() { [Job { id: 1, data: own 10 }, Job { id: 2, data: own 20 }] }
fn i64 take(Job j) { j.id + *j.data }
fn main() {
    let Job[] mut jobs = [Job { id: 1, data: own 1 }, Job { id: 2, data: own 2 }, Job { id: 3, data: own 3 }];
    let mut t = 0;
    for (i, j) in jobs.drain() {
        t += take(j) * (i + 1);
    }
    print(t, jobs.len());
    jobs.push(Job { id: 9, data: own 9 });
    for j in jobs.drain() { if j.id == 9 { break; } }
    for j in make() {
        print(take(j));
        if j2() { break; }
    }
    let (own i64)[] mut xs = [own 1, own 2, own 3];
    xs.reverse();
    print(xs);
    for x in xs.drain() { print(*x); if *x == 2 { break; } }
    for x in xs.drain() { print(*x); } else { print(\"空\"); }
    let chan<Job> c = chan();
    let Job[] mut js = [Job { id: 5, data: own 5 }];
    let task<i64>[] mut ts = [];
    for j in js.drain() {
        ts.push(spawn(() -> take(j)));
    }
    for t2 in ts.drain() { print(t2.join()); }
}
fn bool j2() { false }
";
    assert_eq!(ok(src), "28 0\n11\n22\n[3, 2, 1]\n3\n2\n空\n10\n");
}

#[test]
fn map_and_filter() {
    // map の関数は要素を for x in a の x と同じ形で受け取る。filter はコピーできる要素だけ
    let src = "struct P { str name; i64 age; own i64 score; }
fn i64 dbl(i64 x) { x * 2 }
fn main() {
    let xs = [1, 2, 3, 4, 5];
    let k = 3;
    print(xs.filter((x) -> x > k), xs.map((x) -> x * x), xs.map(dbl));
    let P[] ps = [P { name: \"a\", age: 30, score: own 5 }, P { name: \"b\", age: 20, score: own 7 }];
    let names = ps.map((p) -> p.name);
    let total = ps.map((p) -> *p.score + p.age);
    print(names, total);
    let (own i64)[] os = [own 1, own 2];
    print(os.map((o) -> *o + 10));
    print(\"a,b,,c\".split(\",\").filter((s) -> s != \"\").map((s) -> s.to_upper()));
    let ys = ps.map((p) -> own (p.age));
    print(ys);
    let m = [\"x\": 1];
    print(xs.map((x) -> x + m[\"x\"]), m);
}
";
    assert_eq!(
        ok(src),
        "[4, 5] [1, 4, 9, 16, 25] [2, 4, 6, 8, 10]\n[a, b] [35, 27]\n[11, 12]\n[A, B, C]\n[30, 20]\n[2, 3, 4, 5, 6] [x: 1]\n"
    );
    let d = jabberwock::run_source("
struct P { i64 a; }
fn main() {
    let P[] ps = [P { a: 1 }];
    let qs = ps.filter((p) -> p.a > 0);
}", false).error.expect("expected an error");
    assert_eq!(d.title, "`P` の要素は filter で選べません");
}

#[test]
fn join_strings() {
    let o = run_source("
fn main() {
    let str[] v = [\"a\", \"bc\", \"\", \"日本\"];
    let str[] none = [];
    print(v.join(\", \"), v.join(\"\"), none.join(\"-\"), \"a b c\".split(\" \").join(\"+\"));
}", false);
    assert!(o.error.is_none(), "{:?}", o.error.map(|d| d.title));
    assert_eq!(o.output, "a, bc, , 日本 abc日本  a+b+c\n");
    assert_eq!(o.live_objects, 0);
    let e = run_source("fn main() { let i64[] v = [1]; print(v.join(\",\")); }", false).error.expect("error");
    assert!(e.title.contains("join"));
}

#[test]
fn for_with_index_over_a_list_inside_an_element() {
    let src = "
struct S { str[] xs; }
fn main() {
    let S[] a = [S { xs: [\"a\", \"b\"] }, S { xs: [\"c\"] }];
    for j in range(2) {
        for (i, x) in a[j].xs { print(\"{j} {i} {x}\"); }
    }
    let (own S)[] b = [own S { xs: [\"z\", \"y\"] }];
    for (i, x) in b[0].xs { print(\"{i} {x}\"); }
}";
    assert_eq!(ok(src), "0 0 a\n0 1 b\n1 0 c\n0 z\n1 y\n");
    assert!(jabberwock::ir_text(src, &|_: &str| None).is_ok());
}
