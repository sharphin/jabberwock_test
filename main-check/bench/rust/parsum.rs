use std::thread;

fn count(w: i64, t: i64, n: i64) -> i64 {
    let mut c = 0;
    let mut blk = w;
    while blk * 1000 < n {
        for x in (blk * 1000).max(2)..(blk * 1000 + 1000).min(n) {
            let mut p = true;
            let mut d = 2;
            while d * d <= x { if x % d == 0 { p = false; break; } d += 1; }
            if p { c += 1; }
        }
        blk += t;
    }
    c
}

fn main() {
    let t: i64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(4);
    let n = 3000000;
    let ts: Vec<_> = (0..t).map(|w| thread::spawn(move || count(w, t, n))).collect();
    let total: i64 = ts.into_iter().map(|h| h.join().unwrap()).sum();
    println!("{}", total);
}
