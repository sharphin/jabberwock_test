fn main() {
    let n = 600;
    let (mut a, mut b, mut c) = (Vec::new(), Vec::new(), Vec::new());
    for i in 0..n * n { a.push((i % 7) as f64 * 0.5); b.push((i % 5) as f64 * 0.25); c.push(0.0); }
    for i in 0..n {
        for k in 0..n {
            let aik = a[i * n + k];
            for j in 0..n { c[i * n + j] += aik * b[k * n + j]; }
        }
    }
    let s: f64 = c.iter().sum();
    println!("{}", (s * 1000.0) as i64);
}
