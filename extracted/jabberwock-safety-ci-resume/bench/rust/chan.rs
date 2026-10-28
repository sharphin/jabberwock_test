use std::sync::mpsc;
use std::thread;

// jabberwock と同じ「上限なし」のチャンネル（mpsc::channel）
fn main() {
    let (tx, rx) = mpsc::channel::<i64>();
    let n = 5000000;
    let t = thread::spawn(move || { for i in 0..n { tx.send(i).unwrap(); } });
    let mut sum = 0;
    for v in rx { sum += v; }
    t.join().unwrap();
    println!("{}", sum);
}
