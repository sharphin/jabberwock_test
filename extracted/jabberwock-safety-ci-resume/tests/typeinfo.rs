//! 型検査が、式ごとの型と型の引数を記録すること（IR への書き出しの土台）

use jabberwock::ast::{walk_block, Expr, ExprKind};
use jabberwock::types::{is_const_tree, Ty};

/// 型の記録がない式（数だけの式の中は、根だけ記録すればよい）
fn untyped(src: &str) -> Vec<String> {
    let (prog, info) = match jabberwock::typed(src, &jabberwock::no_modules) {
        Ok(x) => x,
        Err(d) => panic!("{}", d.render("t.jw", src)),
    };
    let mut missing = Vec::new();
    for f in &prog.fns {
        walk_block(&f.body, &mut |e: &Expr| {
            if info.ty(e).is_none() {
                missing.push(format!("{}:{} {:?}", e.span.line, e.span.col, e.kind));
            }
            !is_const_tree(e)
        });
    }
    missing
}

#[test]
fn every_expression_in_the_examples_is_typed() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/examples");
    let mut n = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if !matches!(path.extension(), Some(x) if x == "jw") {
            continue;
        }
        let src = std::fs::read_to_string(&path).unwrap();
        let missing = untyped(&src);
        assert!(missing.is_empty(), "{}: {:#?}", path.display(), missing);
        n += 1;
    }
    assert!(n > 5);
}

#[test]
fn records_types_and_type_arguments() {
    let src = "
struct Pair<T> {
    T a;
    T b;
}

fn T first<T>(ref Pair<T> p) { p.a }

fn main() {
    let p = Pair { a: 1, b: 2 };
    let x = first(p);
    let f = (i64 y) -> y + x;
    let m = [\"k\": f(1)];
    for (k, v) in m { print(k, v); }
    print(x);
}";
    assert!(untyped(src).is_empty(), "{:#?}", untyped(src));
    let (prog, info) = jabberwock::typed(src, &jabberwock::no_modules).unwrap();
    let main = prog.fns.iter().find(|f| f.name == "main").unwrap();
    let mut found = false;
    walk_block(&main.body, &mut |e: &Expr| {
        if let ExprKind::Call(name, _) = &e.kind {
            if name == "first" {
                assert_eq!(info.ty(e), Some(&Ty::Int(jabberwock::ast::IntTy::I64)));
                assert_eq!(info.targs[&(e as *const Expr)]["T"], Ty::Int(jabberwock::ast::IntTy::I64));
                found = true;
            }
        }
        true
    });
    assert!(found);
}
