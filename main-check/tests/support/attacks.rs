/// 秘密の値。攻撃のプログラムは、解放した区画やリストの外にこれを置いてから、古い ref や範囲の外を読もうとする
pub const SECRET: &str = "4242424242";

/// (名前, プログラム, 止まるときのエラーの題に含まれる文)
pub const ATTACKS: &[(&str, &str, &str)] = &[
    ("sent-own-reuse", include_str!("../safety/sent-own-reuse.jw"), "無効になった参照"),
    ("sent-list-reuse", include_str!("../safety/sent-list-reuse.jw"), "無効になった参照"),
    ("sent-view-reuse", include_str!("../safety/sent-view-reuse.jw"), "無効になった参照"),
    ("drop_then_reuse", "fn main() {\n    let mut a = own 1;\n    let r = ref a;\n    drop(a);\n    let b = own 4242424242;\n    print(*r);\n    print(*b);\n}", "無効になった参照"),
    ("reassign_then_reuse", "fn main() {\n    let mut a = own 1;\n    let r = ref a;\n    a = own 4242424242;\n    print(*r);\n}", "無効になった参照"),
    ("moved_into_call", "fn eat(own i64 x) {}\nfn main() {\n    let a = own 1;\n    let r = ref a;\n    eat(a);\n    let b = own 4242424242;\n    print(*r, *b);\n}", "無効になった参照"),
    ("field_after_drop", "struct P { i64 x; i64 y; }\nfn main() {\n    let mut p = own P { x: 1, y: 2 };\n    let r = ref p.y;\n    drop(p);\n    let q = own P { x: 4242424242, y: 4242424242 };\n    print(*r, q.x);\n}", "無効になった参照"),
    ("local_escapes", "struct H { ref i64 r; }\nfn H keep() {\n    let x = 7;\n    H { r: ref x }\n}\nfn main() {\n    let h = keep();\n    let s = own 4242424242;\n    print(*h.r, *s);\n}", "無効になった参照"),
    ("block_ends", "fn main() {\n    let z = 0;\n    let mut r = ref z;\n    if true {\n        let a = own 1;\n        r = ref a;\n    }\n    let b = own 4242424242;\n    print(*r, *b);\n}", "無効になった参照"),
    ("push_moves_elements", "struct P { i64 x; }\nfn main() {\n    let P[] mut ps = [P { x: 1 }];\n    let r = ref ps[0];\n    for i in range(100) { ps.push(P { x: 4242424242 }); }\n    print(r.x);\n}", "無効になった参照"),
    ("remove_shifts", "fn main() {\n    let i64[] mut l = [1, 4242424242];\n    let r = ref l[0];\n    l.remove(0);\n    print(*r);\n}", "無効になった参照"),
    ("sort_moves", "fn main() {\n    let i64[] mut l = [3, 1, 2];\n    let r = ref l[0];\n    l.sort();\n    l.push(4242424242);\n    print(*r);\n}", "無効になった参照"),
    ("drain_frees", "fn main() {\n    let (own i64)[] mut l = [own 1, own 2];\n    let r = ref l[0];\n    for x in l.drain() { drop(x); }\n    let k = own 4242424242;\n    print(*r, *k);\n}", "無効になった参照"),
    ("match_ref_then_replace", "struct P { i64 x; }\nfn main() {\n    let mut o = Some(own P { x: 1 });\n    let r = match ref o { case Some(p) -> p; case None -> return; };\n    o = Some(own P { x: 4242424242 });\n    print(r.x);\n}", "無効になった参照"),
    ("sent_to_thread", "fn main() {\n    let mut a = own 4242424242;\n    let r = ref a;\n    let t = spawn(() -> *a);\n    t.join();\n    print(*r);\n}", "無効になった参照"),
    ("pop_then_read", "fn main() {\n    let i64[] mut l = [1];\n    l.push(4242424242);\n    l.pop();\n    let i = 1;\n    print(l[i]);\n}", "範囲の外"),
    ("index_wraps", "fn main() {\n    let a = [1, 2, 3];\n    let i = 9223372036854775807;\n    print(a[i + 1]);\n}", "範囲の外"),
    ("negative_index", "fn main() {\n    let i64[] l = [4242424242];\n    let i = 0 - 1;\n    print(l[i]);\n}", "範囲の外"),
    ("slice_past_end", "fn main() {\n    let s = \"4242424242\";\n    let t = s.slice(0, 2);\n    print(t.slice(1, 5));\n}", "範囲の外"),
];

/// 操作ごとに必ず最後の ref の使用まで到達する、型の通る攻撃。
/// 名前で到達した操作を数える。ランダムなプログラムの先行エラーに隠れない。
pub fn generated(seed: u64) -> Vec<(String, String, &'static str)> {
    let value = 10 + seed % 1000;
    let mut out = Vec::new();
    let ops = [
        ("push", "xs.push(8);"), ("pop", "xs.pop();"),
        ("insert", "xs.insert(0, 8);"), ("remove", "xs.remove(0);"),
        ("drain", "xs.drain();"), ("reverse", "xs.reverse();"),
        ("sort", "xs.sort();"), ("sort_by", "xs.sort_by((a, b) -> a > b);"),
        ("sort_unstable", "xs.sort_unstable();"),
        ("sort_unstable_by", "xs.sort_unstable_by((a, b) -> a < b);"),
        ("replace", "xs = [9, 8];"), ("drop", "drop(xs);"),
        ("send", "let t = spawn(() -> xs.len()); t.join();"),
    ];
    for (name, op) in ops {
        // 分岐・ループを通るときも、合流で無効化を忘れていないこと。
        for (flow, action) in [("direct", op.to_string()), ("branch", format!("if true {{ {} }}", op)),
                               ("loop", format!("for i in range(1) {{ {} }}", op))] {
            // drop とスレッド送信はループの2回目にムーブ済みになるため直接・分岐で検査する。
            if flow == "loop" && (name == "drop" || name == "send") { continue; }
            let src = format!("fn main() {{ let i64[] mut xs = [{}, 2, 3]; let r = ref xs[0]; {} print(\"reached\"); print(*r); }}", value, action);
            out.push((format!("list-{}-{}", name, flow), src, "無効になった参照"));
        }
    }
    for (name, op) in [
        ("own-drop", "drop(p);"), ("own-replace", "p = own 9;"),
        ("own-call", "eat(p);"), ("own-send", "let t = spawn(() -> *p); t.join();"),
    ] {
        for (flow, action) in [("direct", op.to_string()), ("branch", format!("if true {{ {} }}", op))] {
            out.push((format!("{}-{}", name, flow), format!("fn eat(own i64 x) {{}} fn main() {{ let mut p = own {}; let r = ref p; {} let q = own 4242424242; print(\"reached\"); print(*r); }}", value, action), "無効になった参照"));
        }
    }
    for index in ["-1", "3", "9223372036854775807", "-9223372036854775808"] {
        for kind in ["array", "list"] {
            let decl = if kind == "array" { "let xs = [1, 2, 3];" } else { "let i64[] xs = [1, 2, 3];" };
            out.push((format!("{}-index-{}", kind, index), format!("fn main() {{ {} let i = {}; print(\"reached\"); print(xs[i]); }}", decl, index), "範囲の外"));
        }
    }
    for (name, body) in [
        ("array-write", "let mut a = [1, 2]; let i = -1; a[i] = 9;"),
        ("list-write", "let i64[] mut a = [1, 2]; let u64 i = 18446744073709551615; a[i] = 9;"),
        ("array-ref", "let a = [1, 2]; let i = 2; let r = ref a[i]; print(*r);"),
        ("list-ref", "let i64[] a = []; let i = 0; let r = ref a[i]; print(*r);"),
        ("nested-array", "let a = [[1, 2], [3, 4]]; let i = 0; let j = -1; print(a[i][j]);"),
        ("nested-list", "let i64[][] a = [[1], [2]]; let i = 1; let j = 1; print(a[i][j]);"),
    ] {
        out.push((name.to_string(), format!("fn main() {{ print(\"reached\"); {} }}", body), "範囲の外"));
    }
    out
}
