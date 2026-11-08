struct Node { v: i64, next: Option<Box<Node>> }

fn sum(mut cur: &Option<Box<Node>>) -> i64 {
    let mut s = 0;
    while let Some(n) = cur { s += n.v; cur = &n.next; }
    s
}

fn main() {
    let mut head = None;
    for i in 0..1000 { head = Some(Box::new(Node { v: i, next: head })); }
    let mut total = 0;
    for _ in 0..100000 { total += sum(&head) % 1000; }
    println!("{}", total);
}
