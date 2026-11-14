struct P { x: f64, y: f64, vx: f64, vy: f64 }

fn step(ps: &mut [P], dt: f64) {
    for i in 0..ps.len() {
        let p = &mut ps[i];
        p.vy -= 9.8 * dt;
        p.x += p.vx * dt;
        p.y += p.vy * dt;
        if p.y < 0.0 { p.y = -p.y; p.vy = -p.vy * 0.9; }
    }
}

fn main() {
    let mut ps = Vec::new();
    for i in 0..10000 { ps.push(P { x: i as f64, y: 10.0 + (i % 13) as f64, vx: 1.0, vy: 0.0 }); }
    for _ in 0..10000 { step(&mut ps, 0.01); }
    let s: f64 = ps.iter().map(|p| p.y).sum();
    println!("{}", (s * 1000.0) as i64);
}
