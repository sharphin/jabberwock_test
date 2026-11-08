fn main() {
    let n = 10;
    let mut perm = vec![0usize; n];
    let mut perm1: Vec<usize> = (0..n).collect();
    let mut count = vec![0usize; n];
    let (mut r, mut checksum, mut maxflips, mut permcount) = (n, 0i64, 0i64, 0i64);
    loop {
        while r != 1 { count[r - 1] = r; r -= 1; }
        perm.copy_from_slice(&perm1);
        let mut flips = 0;
        let mut k = perm[0];
        while k != 0 { perm[..=k].reverse(); flips += 1; k = perm[0]; }
        if flips > maxflips { maxflips = flips; }
        checksum += if permcount % 2 == 0 { flips } else { -flips };
        loop {
            if r == n { println!("{} {}", checksum, maxflips); return; }
            let p0 = perm1[0];
            for i in 0..r { perm1[i] = perm1[i + 1]; }
            perm1[r] = p0;
            count[r] -= 1;
            if count[r] > 0 { break; }
            r += 1;
        }
        permcount += 1;
    }
}
