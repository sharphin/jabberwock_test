fn a(i: usize, j: usize) -> f64 { 1.0 / ((i + j) * (i + j + 1) / 2 + i + 1) as f64 }

fn times(v: &[f64], out: &mut [f64]) {
    for i in 0..v.len() {
        let mut s = 0.0;
        for j in 0..v.len() { s += a(i, j) * v[j]; }
        out[i] = s;
    }
}

fn times_t(v: &[f64], out: &mut [f64]) {
    for i in 0..v.len() {
        let mut s = 0.0;
        for j in 0..v.len() { s += a(j, i) * v[j]; }
        out[i] = s;
    }
}

fn times_ata(v: &[f64], tmp: &mut [f64], out: &mut [f64]) { times(v, tmp); times_t(tmp, out); }

fn main() {
    let n = 2000;
    let (mut u, mut v, mut tmp) = (vec![1.0; n], vec![0.0; n], vec![0.0; n]);
    for _ in 0..10 { times_ata(&u, &mut tmp, &mut v); times_ata(&v, &mut tmp, &mut u); }
    let (mut vbv, mut vv) = (0.0, 0.0);
    for i in 0..n { vbv += u[i] * v[i]; vv += v[i] * v[i]; }
    println!("{}", ((vbv / vv as f64).sqrt() * 1e9) as i64);
}
