fn main() {
    let w = 300usize;
    let n = w * w;
    let (mut start, mut edges) = (Vec::new(), Vec::new());
    for v in 0..n {
        start.push(edges.len());
        let (x, y) = (v % w, v / w);
        if x > 0 { edges.push(v - 1); }
        if x < w - 1 { edges.push(v + 1); }
        if y > 0 { edges.push(v - w); }
        if y < w - 1 { edges.push(v + w); }
    }
    start.push(edges.len());
    let mut dist = vec![0i64; n];
    let mut queue = vec![0usize; n];
    let mut total = 0i64;
    for round in 0..300 {
        for d in dist.iter_mut() { *d = -1; }
        let src = (round * 7919) % n;
        dist[src] = 0;
        queue[0] = src;
        let (mut head, mut tail) = (0, 1);
        while head < tail {
            let v = queue[head];
            head += 1;
            for e in start[v]..start[v + 1] {
                let u = edges[e];
                if dist[u] < 0 { dist[u] = dist[v] + 1; queue[tail] = u; tail += 1; }
            }
        }
        total += dist.iter().sum::<i64>();
    }
    println!("{}", total);
}
