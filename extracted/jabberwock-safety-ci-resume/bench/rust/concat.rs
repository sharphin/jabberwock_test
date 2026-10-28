use std::fmt::Write;

fn main() {
    let mut s = String::new();
    for i in 0..3000000 { write!(s, "v{},", i % 1000).unwrap(); }
    let mut lines = 0;
    for i in 0..1000 {
        let mut line = format!("%t{} = add i64 %a, %b", i);
        line += "\n";
        lines += line.len();
    }
    println!("{} {}", s.len(), lines);
}
