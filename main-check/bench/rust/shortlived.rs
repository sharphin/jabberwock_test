struct P { x: i64, y: i64 }

fn main() {
    let mut ring: Vec<Box<P>> = (0..16).map(|_| Box::new(P { x: 0, y: 0 })).collect();
    let mut s = 0i64;
    for i in 0..100000000i64 {
        ring[(i % 16) as usize] = Box::new(P { x: i, y: i * 2 });
        s += ring[((i + 5) % 16) as usize].y - ring[((i + 3) % 16) as usize].x;
    }
    println!("{}", s);
}
