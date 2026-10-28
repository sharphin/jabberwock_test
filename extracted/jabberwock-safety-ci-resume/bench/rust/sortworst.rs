fn sorted_sum(mut v: Vec<i64>) -> i64 {
    v.sort();
    let mut s = 0;
    let mut i = 0;
    while i < v.len() { s += v[i] * (i % 7) as i64; i += 997; }
    s
}

fn main() {
    let n = 3000000i64;
    let (mut a, mut b, mut c, mut d) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for i in 0..n { a.push(i); b.push(n - i); c.push(42); d.push(i % 1000); }
    println!("{} {} {} {}", sorted_sum(a), sorted_sum(b), sorted_sum(c), sorted_sum(d));
}
