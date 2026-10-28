use std::sync::mpsc;
use std::thread;

fn main() {
    let (ping_tx, ping_rx) = mpsc::channel::<i64>();
    let (pong_tx, pong_rx) = mpsc::channel::<i64>();
    let n = 50000;
    let t = thread::spawn(move || { for v in ping_rx { pong_tx.send(v + 1).unwrap(); } });
    let mut v = 0;
    for _ in 0..n { ping_tx.send(v).unwrap(); v = pong_rx.recv().unwrap(); }
    drop(ping_tx);
    t.join().unwrap();
    println!("{}", v);
}
