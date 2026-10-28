use std::collections::HashMap;

fn main() {
    let mut counts: HashMap<String, i64> = HashMap::new();
    let mut x: i64 = 1;
    for _ in 0..5000000 {
        x = (x * 1103515245 + 12345) % 2147483648;
        let w = format!("w{}", x % 5000);
        *counts.entry(w).or_insert(0) += 1;
    }
    let best = counts.values().copied().max().unwrap_or(0);
    println!("{} {}", counts.len(), best);
}
