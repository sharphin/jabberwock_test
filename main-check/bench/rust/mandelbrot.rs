fn main() {
    let n = 1600usize;
    let mut bits = vec![0u8; n * n / 8];
    let mut pos = 0;
    for y in 0..n {
        let ci = 2.0 * y as f64 / n as f64 - 1.0;
        for x in (0..n).step_by(8) {
            let mut b8 = 0u32;
            for b in 0..8 {
                let cr = 2.0 * (x + b) as f64 / n as f64 - 1.5;
                let (mut zr, mut zi) = (0.0f64, 0.0f64);
                let mut inside = true;
                for _ in 0..50 {
                    let tr = zr * zr - zi * zi + cr;
                    zi = 2.0 * zr * zi + ci;
                    zr = tr;
                    if zr * zr + zi * zi > 4.0 { inside = false; break; }
                }
                b8 = (b8 << 1) | inside as u32;
            }
            bits[pos] = b8 as u8;
            pos += 1;
        }
    }
    let sum: i64 = bits.iter().map(|&v| v as i64 * 7 % 255).sum();
    println!("{} {}", pos, sum);
}
