struct Node { key: i64, left: Option<Box<Node>>, right: Option<Box<Node>> }

fn insert(mut cur: &mut Option<Box<Node>>, key: i64) {
    while let Some(n) = cur {
        cur = if key < n.key { &mut n.left } else { &mut n.right };
    }
    *cur = Some(Box::new(Node { key, left: None, right: None }));
}

fn contains(mut cur: &Option<Box<Node>>, key: i64) -> bool {
    while let Some(n) = cur {
        if n.key == key { return true; }
        cur = if key < n.key { &n.left } else { &n.right };
    }
    false
}

fn main() {
    let mut root = None;
    let mut x: i64 = 42;
    for _ in 0..1000000 { x = (x * 1103515245 + 12345) % 2147483648; insert(&mut root, x); }
    let mut found = 0;
    x = 42;
    for _ in 0..2000000 { x = (x * 1103515245 + 12345) % 2147483648; if contains(&root, x) { found += 1; } }
    println!("{}", found);
}
