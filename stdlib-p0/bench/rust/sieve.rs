fn main() {
    let n = 20000000usize;
    let mut comp: Vec<bool> = Vec::new();
    for _ in 0..=n { comp.push(false); }
    let mut count = 0;
    for i in 2..=n {
        if !comp[i] {
            count += 1;
            let mut j = i * i;
            while j <= n { comp[j] = true; j += i; }
        }
    }
    println!("{}", count);
}
