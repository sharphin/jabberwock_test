struct Tree { l: Option<Box<Tree>>, r: Option<Box<Tree>> }

fn make(d: i32) -> Box<Tree> {
    if d == 0 { Box::new(Tree { l: None, r: None }) } else { Box::new(Tree { l: Some(make(d - 1)), r: Some(make(d - 1)) }) }
}

fn check(t: &Tree) -> i64 {
    match (&t.l, &t.r) { (Some(a), Some(b)) => 1 + check(a) + check(b), _ => 1 }
}

fn main() {
    let n = 18;
    { let s = make(n + 1); println!("{} {}", n + 1, check(&s)); }
    let long_lived = make(n);
    let mut d = 4;
    while d <= n {
        let iters = 1i64 << (n - d + 4);
        let mut chk = 0;
        for _ in 0..iters { chk += check(&make(d)); }
        println!("{} {} {}", iters, d, chk);
        d += 2;
    }
    println!("{} {}", n, check(&long_lived));
}
