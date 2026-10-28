//! スレッド（spawn / task / chan）のテスト。結果の順番が決まるもの（join と chan で待つもの）だけを書く

use jabberwock::{run_source, run_source_with};

fn ok(src: &str) -> String {
    let o = run_source(src, false);
    if let Some(d) = &o.error {
        panic!("unexpected error:\n{}", d.render("test.jw", src));
    }
    assert_eq!(o.live_objects, 0, "heap objects leaked");
    let off = run_source_with(src, false, false);
    assert_eq!(o.output, off.output, "elision changed the output");
    o.output
}

fn err(src: &str) -> jabberwock::diag::Diagnostic {
    run_source(src, false).error.expect("expected an error")
}

#[test]
fn spawn_and_join_return_values() {
    let src = "
fn i64 sum(i64[] xs) {
    let mut s = 0;
    for x in xs { s += x; }
    s
}

fn i64 seven() { 7 }

fn main() {
    let i64[] a = [1, 2, 3];
    let i64[] b = [4, 5, 6];
    let t1 = spawn(() -> sum(a));
    let t2 = spawn(() -> sum(b));
    print(t1.join() + t2.join());
    print(spawn(seven).join());
}";
    assert_eq!(ok(src), "21\n7\n");
}

#[test]
fn channels_carry_values_in_order() {
    let src = "
fn main() {
    let chan<str> c = chan();
    let w = spawn(() -> {
        for i in range(3) { c.send(\"msg {i}\"); }
        c.close();
    });
    loop {
        match c.recv() {
            case Some(m) -> print(m);
            case None -> break;
        }
    }
    w.join();
    print(c);
}";
    assert_eq!(ok(src), "msg 0\nmsg 1\nmsg 2\n<chan>\n");
}

#[test]
fn own_values_move_between_threads() {
    let src = "
struct Job { i64 id; own i64 data; }

fn main() {
    let chan<Job> jobs = chan();
    let chan<own i64> done = chan();
    let w = spawn(() -> {
        loop {
            match jobs.recv() {
                case Some(j) -> done.send(own (*j.data * 10 + j.id));
                case None -> break;
            }
        }
        done.close();
    });
    for i in range(3) {
        jobs.send(Job { id: i, data: own i });
    }
    jobs.close();
    let mut total = 0;
    loop {
        match done.recv() {
            case Some(x) -> { total += *x; }
            case None -> break;
        }
    }
    w.join();
    print(total);
}";
    assert_eq!(ok(src), "33\n");
}

#[test]
fn moved_own_in_the_closure_can_be_taken_out() {
    // spawn の関数は 1 回だけ呼ばれるので、取り込んだ own をそのまま返してよい
    let src = "
fn main() {
    let a = own 5;
    let t = spawn(() -> a);
    let b = t.join();
    print(*b);
}";
    assert_eq!(ok(src), "5\n");
}

#[test]
fn dropping_a_task_waits_for_it() {
    let src = "
fn main() {
    let chan<i64> c = chan();
    if true {
        let t = spawn(() -> { c.send(1); own 2 });
    }
    print(c.recv());
}";
    assert_eq!(ok(src), "Some(1)\n");
}

#[test]
fn refs_cannot_go_to_other_threads() {
    let d = err("
fn main() {
    let a = own 1;
    let r = ref a;
    let t = spawn(() -> *r);
    print(t.join());
}");
    assert_eq!(d.title, "`r` はほかのスレッドへ渡せません");
    let d = err("
fn main() {
    let chan<ref i64> c = chan();
}");
    assert_eq!(d.title, "チャンネルで送る値に `ref i64` は使えません");
    let d = err("
struct Holder { ref i64 r; }
fn main() {
    let chan<Holder> c = chan();
}");
    assert_eq!(d.title, "チャンネルで送る値に `Holder` は使えません");
    let d = err("
fn main() {
    let f = () -> 1;
    let t = spawn(f);
}");
    assert_eq!(d.title, "spawn にはその場で書いたクロージャか、関数の名前を渡してください");
    let d = err("
fn main() {
    let c = chan();
}");
    assert_eq!(d.title, "chan() の型が分かりません");
}

#[test]
fn a_ref_to_a_sent_value_is_dangling() {
    let d = err("
fn main() {
    let chan<own i64> c = chan();
    let a = own 1;
    let r = ref a;
    c.send(a);
    print(*r);
}");
    assert_eq!(d.title, "無効になった参照を使いました (dangling reference)");
}

#[test]
fn an_error_in_a_thread_stops_the_program() {
    let d = err("
fn i64 div(i64 a, i64 b) { a / b }

fn main() {
    let t = spawn(() -> div(1, 0));
    print(t.join());
}");
    assert_eq!(d.title, "0 で割りました");
}

#[test]
fn deadlocks_are_reported() {
    let d = err("
fn main() {
    let chan<i64> c = chan();
    print(c.recv());
}");
    assert_eq!(d.title, "すべてのスレッドが待ちのまま止まりました (deadlock)");
    let d = err("
fn main() {
    let chan<i64> a = chan();
    let chan<i64> b = chan();
    let t = spawn(() -> { let x = a.recv(); b.send(1); });
    print(b.recv());
    a.send(1);
}");
    assert_eq!(d.title, "すべてのスレッドが待ちのまま止まりました (deadlock)");
}

#[test]
fn channel_misuse_is_an_error() {
    let d = err("
fn main() {
    let chan<i64> c = chan();
    c.close();
    c.send(1);
}");
    assert_eq!(d.title, "閉じたチャンネルに送りました");
    let d = err("
fn main() {
    let t = spawn(() -> 1);
    print(t.join());
    print(t.join());
}");
    assert_eq!(d.title, "この task はもう join しました");
}

#[test]
fn many_threads_share_one_channel() {
    let src = "
fn main() {
    let chan<i64> c = chan();
    let task<i64>[] mut ts = [];
    for i in range(8) {
        ts.push(spawn(() -> { c.send(i); i * i }));
    }
    let mut squares = 0;
    for t in ts { squares += t.join(); }
    let mut sum = 0;
    for i in range(8) {
        match c.recv() {
            case Some(x) -> { sum += x; }
            case None -> {}
        }
    }
    print(squares, sum);
}";
    assert_eq!(ok(src), "140 28\n");
}

#[test]
fn tasks_without_a_value() {
    let src = "
fn main() {
    let chan<i64> c = chan();
    let task<()>[] mut ts = [];
    for i in range(3) {
        ts.push(spawn(() -> { c.send(i); }));
    }
    for t in ts.drain() { t.join(); }
    let mut s = 0;
    for i in range(3) {
        match c.recv() { case Some(x) -> { s += x; } case None -> {} }
    }
    print(s);
}";
    assert_eq!(ok(src), "3\n");
}

/// spawn を重ねながら出力し、own を別のスレッドへ渡して解放する。
/// 最初の spawn 後も出力の錠の印を読み書きするため、同じ値の書き込みにも同期が要る。
#[test]
fn nested_spawns_print_and_transfer_ownership_concurrently() {
    for workers in [2, 4, 8] {
        let src = format!("struct P {{ i64 x; }}
fn main() {{
    let chan<own P> c = chan();
    let task<()>[] mut ts = [];
    for i in range({workers}) {{
        ts.push(spawn(() -> {{
            for j in range(8) {{
                let t = spawn(() -> {{ print(\"tick\"); own P {{ x: i * 8 + j }} }});
                c.send(t.join());
            }}
        }}));
        print(\"tick\");
    }}
    let mut sum = 0;
    for k in range({count}) {{ match c.recv() {{ case Some(p) -> {{ sum += p.x; }} case None -> {{}} }} }}
    for t in ts {{ t.join(); }}
    print(sum);
}}", workers = workers, count = workers * 8);
        let count = workers * 8;
        let want = format!("{}{}\n", "tick\n".repeat(workers * 9), count * (count - 1) / 2);
        assert_eq!(ok(&src), want);
    }
}

/// 送信元の古い ref（または own の写し）を検査中に、受信側が世代を変えても競合しない。
#[test]
fn sent_refs_can_be_checked_while_the_receiver_reuses_blocks() {
    for src in [include_str!("safety/sent-own-reuse.jw"), include_str!("safety/sent-list-reuse.jw"), include_str!("safety/sent-view-reuse.jw")] {
        let o = run_source(src, false);
        let d = o.error.expect("sent reference must be rejected");
        assert!(d.title.contains("無効になった参照"), "{}", d.render("main.jw", src));
        assert!(o.output.is_empty());
    }
}
