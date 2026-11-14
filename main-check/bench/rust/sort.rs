fn main() {
    let mut v: Vec<i64> = Vec::new();
    let mut x: i64 = 7;
    for _ in 0..3000000 { x = (x * 1103515245 + 12345) % 2147483648; v.push(x); }
    v.sort();
    let mut sum = 0;
    let mut i = 0;
    while i < v.len() { sum += v[i]; i += 1000; }
    println!("{} {} {}", v[0], v[v.len() - 1], sum);
}
