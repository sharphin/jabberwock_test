//! スレッド（`spawn` / `task` / `chan`）。インタプリタ用。
//!
//! スレッドごとに別の `Interp`（別のヒープ）を本物のスレッドで動かす。構文木は `Rc` を含むので
//! スレッドをまたげない。新しいスレッドはソースから読み直す（`World.load`）。
//! スレッドのあいだで渡す値は、ヒープから切り離した形 `Portable` にする（own の中身ごと運ぶ）。
//!
//! 待つ操作（`recv`・`join`・終わりの待ち合わせ）は、ぜんぶ 1 つの錠 `World.state` と 1 つの条件変数で待つ。
//! 状態が変わるたびに `epoch` を進めて `blocked` を 0 に戻すので、
//! 「生きているスレッドが全員、今の状態のまま待っている」（= デッドロック）を正しく数えられる。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use crate::ast::{IntTy, Program};
use crate::diag::{Diagnostic, Span};

/// ヒープから切り離した値（ほかのスレッドへ渡す形）
#[derive(Debug)]
pub enum Portable {
    Unit,
    Int(i128, IntTy),
    Float(f64),
    Bool(bool),
    Str(String),
    Own(Box<Portable>),
    Struct(String, Vec<Portable>),
    Enum { variant: String, fields: Vec<Portable>, copy: bool },
    Array(Vec<Portable>, bool),
    Map(Vec<(Portable, Portable)>),
    Chan(Arc<Chan>),
    Task(Arc<Task>),
}

/// チャンネル。中身は `World.state` の錠を持っているときだけ触る
#[derive(Debug, Default)]
pub struct Chan {
    inner: Mutex<ChanInner>,
}

#[derive(Debug, Default)]
struct ChanInner {
    queue: VecDeque<Portable>,
    closed: bool,
}

/// spawn したスレッド。結果は `World.state` の錠を持っているときだけ触る
#[derive(Debug, Default)]
pub struct Task {
    inner: Mutex<TaskInner>,
}

#[derive(Debug, Default)]
struct TaskInner {
    /// 終わったら Some（Err はエラーで止まった）
    result: Option<Result<Portable, ()>>,
    joined: bool,
}

pub struct State {
    /// 動いているスレッドの数（main も数える）
    alive: usize,
    /// 今の epoch のまま待っているスレッドの数
    blocked: usize,
    epoch: u64,
    /// どれかのスレッドがエラーで止まった（最初のエラー）
    pub abort: Option<Diagnostic>,
    /// 出力（print の行。行ごとにまとめて足すので混ざらない）
    pub output: String,
    /// 標準エラーへの出力（eprint の行）
    pub errput: String,
    /// `exit(n)` で終わった（終了コード）
    pub exit: Option<i32>,
    /// 終わったスレッドのヒープに残っていた数（リーク検査）
    pub leaked: usize,
    /// デッドロックを見つけた（main が自分の待っている位置で知らせる）
    deadlock: bool,
}

thread_local! {
    /// spawn で作ったスレッドか（main でないか）
    static IS_WORKER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// 同時に動かしてよいスレッドの数（main のほか）。環境変数 `JW_MAX_THREADS`（スレッドを作れないときのテスト用。jw_rt.c の `max_threads` と同じ）
pub fn max_threads() -> Option<usize> {
    std::env::var("JW_MAX_THREADS").ok().and_then(|s| s.trim().parse().ok())
}

/// このスレッドを spawn で作ったものとして印をつける
pub fn mark_worker() {
    IS_WORKER.with(|w| w.set(true));
}

fn deadlock_error(span: Span) -> Diagnostic {
    Diagnostic::new("すべてのスレッドが待ちのまま止まりました (deadlock)")
        .primary(span, "ここで待っているが、値を送ったり終わったりできるスレッドがもうない")
        .note("チャンネルの close し忘れや、join・recv の順番を確かめてください")
}

/// 1 回の実行で、すべてのスレッドが共有するもの
pub struct World {
    state: Mutex<State>,
    cv: Condvar,
    /// エラーで止まったスレッドがあるか（`state.abort` の速い写し。関数呼び出しのたびに見る）
    pub aborted: AtomicBool,
    pub echo: bool,
    /// 新しいスレッドのためにプログラムを読み直す
    pub load: Box<dyn Fn() -> Program + Send + Sync>,
    /// IR で動かしているとき: 新しいスレッドのために IR を作り直す
    pub load_ir: Option<Box<dyn Fn() -> crate::ir::Program + Send + Sync>>,
}

impl std::fmt::Debug for World {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "World")
    }
}

/// `exit(n)` で止まるときのエラー（表示はされない。終了コードは `State.exit`）
pub fn exit_marker() -> Diagnostic {
    Diagnostic::new("（exit で終わった）")
}

/// どれかのスレッドがエラーで止まったので、ほかのスレッドも止まるときのエラー（表示はされない）
pub fn aborted_marker() -> Diagnostic {
    Diagnostic::new("（ほかのスレッドのエラーで止まった）")
}

impl World {
    pub fn new(echo: bool, load: Box<dyn Fn() -> Program + Send + Sync>) -> Arc<World> {
        Arc::new(World {
            state: Mutex::new(State { alive: 1, blocked: 0, epoch: 0, abort: None, output: String::new(), errput: String::new(), exit: None, leaked: 0, deadlock: false }),
            cv: Condvar::new(),
            aborted: AtomicBool::new(false),
            echo,
            load,
            load_ir: None,
        })
    }

    pub fn new_ir(load_ir: Box<dyn Fn() -> crate::ir::Program + Send + Sync>) -> Arc<World> {
        let mut w = World::new(false, Box::new(|| unreachable!("IR threads load IR")));
        Arc::get_mut(&mut w).expect("new").load_ir = Some(load_ir);
        w
    }

    pub fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 状態が変わった: 待っているスレッドを起こす
    fn changed(&self, st: &mut State) {
        st.epoch += 1;
        st.blocked = 0;
        self.cv.notify_all();
    }

    pub fn print_line(&self, line: &str) {
        let mut st = self.lock();
        st.output.push_str(line);
        st.output.push('\n');
        if self.echo {
            println!("{}", line);
        }
    }

    pub fn eprint_line(&self, line: &str) {
        let mut st = self.lock();
        st.errput.push_str(line);
        st.errput.push('\n');
        if self.echo {
            eprintln!("{}", line);
        }
    }

    /// `exit(code)`: ほかのスレッドも止めて、終了コードを覚える（先にエラーで止まっていればそちら）
    pub fn exit(&self, code: i32) -> Diagnostic {
        {
            let mut st = self.lock();
            if st.abort.is_none() {
                st.exit = Some(code);
            }
        }
        self.fail(exit_marker());
        exit_marker()
    }

    /// エラーで止まった。最初のエラーだけ覚えて、ほかのスレッドを起こす
    pub fn fail(&self, d: Diagnostic) {
        let mut st = self.lock();
        if st.abort.is_none() && d.title != aborted_marker().title {
            st.abort = Some(d);
        }
        self.aborted.store(true, Ordering::SeqCst);
        self.changed(&mut st);
    }

    /// `ready` が Some を返すまで待つ。全員が待ちのままならデッドロックのエラー。
    /// どのスレッドが見つけても、main が自分の待っている位置で知らせる（表示が実行のたびに変わらないように）
    fn wait<T>(&self, span: Span, mut ready: impl FnMut(&mut State) -> Option<T>) -> Result<T, Diagnostic> {
        let main = !IS_WORKER.with(|w| w.get());
        let mut st = self.lock();
        loop {
            if st.abort.is_some() {
                return Err(aborted_marker());
            }
            if main && st.deadlock {
                return Err(deadlock_error(span));
            }
            if let Some(v) = ready(&mut st) {
                return Ok(v);
            }
            st.blocked += 1;
            if st.blocked >= st.alive {
                st.deadlock = true;
                self.changed(&mut st);
                if main {
                    return Err(deadlock_error(span));
                }
            }
            let e = st.epoch;
            st = self.cv.wait(st).unwrap_or_else(|e| e.into_inner());
            if st.epoch == e {
                // 起こされたが何も変わっていない（見かけの目覚め）
                st.blocked -= 1;
            }
        }
    }

    /// main の終わり: ほかのスレッドがすべて終わるのを待つ
    pub fn wait_all(&self, span: Span) -> Result<(), Diagnostic> {
        self.wait(span, |st| if st.alive == 1 { Some(()) } else { None })
    }

    /// エラーのあと: ほかのスレッドが止まるのを待つ（デッドロックは見ない。止まったスレッドは次の関数呼び出しか待ちで抜ける）
    pub fn wait_stopped(&self) {
        let mut st = self.lock();
        while st.alive > 1 {
            st = self.cv.wait(st).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// 上限（`max_threads`）の中で、もう 1 つスレッドを始めてよいか（`lock` で持った錠の中で呼ぶ）
    pub fn may_start(&self, st: &State) -> bool {
        !matches!(max_threads(), Some(m) if st.alive > m)
    }

    /// スレッドを始めた（`lock` で持った錠の中で呼ぶ）
    pub fn started_locked(&self, st: &mut State) {
        st.alive += 1;
        self.changed(st);
    }

    /// スレッドが終わった（結果を task に置く）
    pub fn finished(&self, task: &Task, result: Result<Portable, ()>, leaked: usize) {
        let mut st = self.lock();
        task.inner.lock().unwrap_or_else(|e| e.into_inner()).result = Some(result);
        st.alive -= 1;
        st.leaked += leaked;
        self.changed(&mut st);
    }

    /// `t.join()`。2 回目はエラー
    pub fn join(&self, task: &Task, span: Span) -> Result<Portable, Diagnostic> {
        {
            let _st = self.lock();
            let mut t = task.inner.lock().unwrap_or_else(|e| e.into_inner());
            if t.joined {
                return Err(Diagnostic::new("この task はもう join しました")
                    .primary(span, "join で受け取れるのは 1 回だけ"));
            }
            t.joined = true;
        }
        self.wait(span, |_| task.inner.lock().unwrap_or_else(|e| e.into_inner()).result.take())?
            .map_err(|_| aborted_marker())
    }

    /// task を join しないまま捨てた: 終わるのを待って、戻り値は捨てる
    pub fn forget(&self, task: &Task, span: Span) -> Result<Option<Portable>, Diagnostic> {
        {
            let _st = self.lock();
            let mut t = task.inner.lock().unwrap_or_else(|e| e.into_inner());
            if t.joined {
                return Ok(None);
            }
            t.joined = true;
        }
        let r = self.wait(span, |_| task.inner.lock().unwrap_or_else(|e| e.into_inner()).result.take())?;
        Ok(r.ok())
    }

    pub fn send(&self, ch: &Chan, v: Portable, span: Span) -> Result<(), Diagnostic> {
        let mut st = self.lock();
        let mut c = ch.inner.lock().unwrap_or_else(|e| e.into_inner());
        if c.closed {
            return Err(Diagnostic::new("閉じたチャンネルに送りました")
                .primary(span, "このチャンネルはもう close されている")
                .note("送り終わってから close してください"));
        }
        c.queue.push_back(v);
        drop(c);
        self.changed(&mut st);
        Ok(())
    }

    /// 値が来るまで待つ。閉じて空なら None
    pub fn recv(&self, ch: &Chan, span: Span) -> Result<Option<Portable>, Diagnostic> {
        self.wait(span, |_| {
            let mut c = ch.inner.lock().unwrap_or_else(|e| e.into_inner());
            match c.queue.pop_front() {
                Some(v) => Some(Some(v)),
                None if c.closed => Some(None),
                None => None,
            }
        })
    }

    pub fn close(&self, ch: &Chan, span: Span) -> Result<(), Diagnostic> {
        let mut st = self.lock();
        let mut c = ch.inner.lock().unwrap_or_else(|e| e.into_inner());
        if c.closed {
            return Err(Diagnostic::new("チャンネルを 2 回 close しました")
                .primary(span, "このチャンネルはもう close されている"));
        }
        c.closed = true;
        drop(c);
        self.changed(&mut st);
        Ok(())
    }
}

/// スレッドで動かす関数（読み直したプログラムの中で探す）
pub enum Entry {
    /// 名前のある関数
    Named(String),
    /// クロージャ（その式の位置で探す）
    Lambda(Span),
}
