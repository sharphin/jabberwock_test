//! 並べ替えの手順（interp と IR の exec が使う）。実行時ライブラリ（runtime/jw_rt.c の `jw_sort` / `jw_sort_unstable`）と
//! **同じ手順で、同じ順に比べる**（`sort_by` の関数は何回・どの順で呼ばれたかが print やエラーで見えるため）。
//! 要素そのものではなく、要素の番号の並びを並べ替えて返す（比べる関数は番号を 2 つ受け取る）。
//!
//! 安定（`sort` / `sort_by`）: 並んだ部分（run。真に減っていれば逆にする）を見つけ、短ければ二分挿入ソートで min_run まで伸ばし、
//! powersort の決まりで run どうしを合わせる（Timsort の仲間。比べる回数が少ない。いつも n log n、並んでいれば n）。
//! 安定でない（`sort_unstable` / `sort_unstable_by`）: 全体が並んでいればそのまま。ほかは pdqsort に近いクイックソート
//! （深さが上限を超えたらヒープソート）。
//! pivot は 9 個の中央値の中央値で、場所は n から決まる疑似乱数（周期のある並びで偏らないように）。
//! 前の pivot と同じ値の pivot を選んだら、等しい要素をまとめて片づける（同じ値が多いとき速い）。
//! 比べる関数がでたらめでも（`a < b` と `b < a` が両方真など）、範囲の外を読まず、必ず終わる。

use crate::diag::Diagnostic;

type R<T> = Result<T, Diagnostic>;

/// これ以下の長さは挿入ソート
const SMALL: usize = 20;

/// 番号の並び `0..n` を、`less(cx, a, b)`（a を b より前に置くか）で並べた並びを返す
pub(crate) fn sort<T, F>(cx: &mut T, n: usize, stable: bool, less: &mut F) -> R<Vec<usize>>
where
    F: FnMut(&mut T, usize, usize) -> R<bool>,
{
    let mut s = Sorter { cx, less, a: (0..n).collect(), s: Vec::new() };
    if stable {
        s.stable()?;
    } else {
        s.unstable()?;
    }
    Ok(s.a)
}

struct Sorter<'a, T, F> {
    cx: &'a mut T,
    less: &'a mut F,
    a: Vec<usize>,
    s: Vec<usize>,
}

/// 自然な run がこれより短いときは、二分挿入ソートでこの長さまで伸ばす（Timsort と同じ決め方。32〜64）
fn min_run(mut n: usize) -> usize {
    let mut r = 0;
    while n >= 64 {
        r |= n & 1;
        n >>= 1;
    }
    n + r
}

/// クイックソートの深さの上限（2 × log2 n）
fn depth_limit(n: usize) -> u32 {
    2 * (usize::BITS - 1 - n.leading_zeros())
}

/// pivot の候補の 9 つの場所（0..n）。n だけで決まる
fn samples(n: usize) -> [usize; 9] {
    let mut r = (n as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let mut out = [0; 9];
    for o in out.iter_mut() {
        r ^= r >> 29;
        r = r.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        r ^= r >> 32;
        *o = (r % n as u64) as usize;
    }
    out
}

/// powersort: 隣り合う 2 つの run（s1 から n1 個と、そのあと n2 個）の境目の深さ
fn power(s1: usize, n1: usize, n2: usize, n: usize) -> u32 {
    let n = n as u64;
    let mut a = 2 * s1 as u64 + n1 as u64;
    let mut b = a + n1 as u64 + n2 as u64;
    let mut p = 0;
    loop {
        p += 1;
        let (x, y) = (a >= n, b >= n);
        if x != y {
            return p;
        }
        if x {
            a -= n;
            b -= n;
        }
        a <<= 1;
        b <<= 1;
    }
}

impl<'a, T, F> Sorter<'a, T, F>
where
    F: FnMut(&mut T, usize, usize) -> R<bool>,
{
    fn lt(&mut self, x: usize, y: usize) -> R<bool> {
        (self.less)(self.cx, x, y)
    }

    /// lo から n 個を挿入ソート
    fn insertion(&mut self, lo: usize, n: usize) -> R<()> {
        for i in 1..n {
            let x = self.a[lo + i];
            let mut j = i;
            while j > 0 && self.lt(x, self.a[lo + j - 1])? {
                self.a[lo + j] = self.a[lo + j - 1];
                j -= 1;
            }
            self.a[lo + j] = x;
        }
        Ok(())
    }

    /// i から始まる run の長さと、真に減っているか（減っていれば逆にすると安定に並ぶ）
    fn run_at(&mut self, i: usize) -> R<(usize, bool)> {
        let n = self.a.len();
        if i + 1 >= n {
            return Ok((n - i, false));
        }
        let mut k = i + 2;
        if self.lt(self.a[i + 1], self.a[i])? {
            while k < n && self.lt(self.a[k], self.a[k - 1])? {
                k += 1;
            }
            Ok((k - i, true))
        } else {
            while k < n && !self.lt(self.a[k], self.a[k - 1])? {
                k += 1;
            }
            Ok((k - i, false))
        }
    }

    // ---------------------------------------------------------------- 安定

    fn stable(&mut self) -> R<()> {
        let n = self.a.len();
        if n < 64 {
            return self.binsert(0, 1, n);
        }
        self.s = vec![0; n];
        let minrun = min_run(n);
        // (始め, 長さ, 次の run との境目の深さ)
        let mut runs: Vec<(usize, usize, u32)> = Vec::new();
        let mut i = 0;
        while i < n {
            let (mut len, desc) = self.run_at(i)?;
            if desc {
                self.a[i..i + len].reverse();
            }
            if len < minrun {
                let f = minrun.min(n - i);
                self.binsert(i, len, f)?;
                len = f;
            }
            let e = i + len;
            if let Some(&(s1, n1, _)) = runs.last() {
                let p = power(s1, n1, e - i, n);
                while runs.len() > 1 && runs[runs.len() - 2].2 > p {
                    self.merge_top(&mut runs)?;
                }
                runs.last_mut().expect("a run").2 = p;
            }
            runs.push((i, e - i, 0));
            i = e;
        }
        while runs.len() > 1 {
            self.merge_top(&mut runs)?;
        }
        Ok(())
    }

    /// lo から n 個のうち、前の done 個が並んでいるとき、残りを二分探索で挿し入れる（同じものの後ろへ入れるので安定）
    fn binsert(&mut self, lo: usize, done: usize, n: usize) -> R<()> {
        for i in done.max(1)..n {
            let x = self.a[lo + i];
            let (mut l, mut r) = (0, i);
            while l < r {
                let m = l + (r - l) / 2;
                if self.lt(x, self.a[lo + m])? {
                    r = m;
                } else {
                    l = m + 1;
                }
            }
            self.a.copy_within(lo + l..lo + i, lo + l + 1);
            self.a[lo + l] = x;
        }
        Ok(())
    }

    fn merge_top(&mut self, runs: &mut Vec<(usize, usize, u32)>) -> R<()> {
        let (_, n2, _) = runs.pop().expect("two runs");
        let top = runs.last_mut().expect("two runs");
        let (lo, n1) = (top.0, top.1);
        top.1 += n2;
        self.merge(lo, lo + n1, lo + n1 + n2)
    }

    /// 並んだ [lo, mid) と [mid, hi) を合わせる（左を写して前から）
    fn merge(&mut self, lo: usize, mid: usize, hi: usize) -> R<()> {
        if mid == lo || mid == hi || !self.lt(self.a[mid], self.a[mid - 1])? {
            return Ok(());
        }
        let n1 = mid - lo;
        self.s[..n1].copy_from_slice(&self.a[lo..mid]);
        let (mut i, mut j, mut k) = (0, mid, lo);
        while i < n1 && j < hi {
            if self.lt(self.a[j], self.s[i])? {
                self.a[k] = self.a[j];
                j += 1;
            } else {
                self.a[k] = self.s[i];
                i += 1;
            }
            k += 1;
        }
        while i < n1 {
            self.a[k] = self.s[i];
            i += 1;
            k += 1;
        }
        Ok(())
    }

    // ---------------------------------------------------------------- 安定でない

    fn unstable(&mut self) -> R<()> {
        let n = self.a.len();
        if n < 2 {
            return Ok(());
        }
        let (len, desc) = self.run_at(0)?;
        if len == n {
            if desc {
                self.a.reverse();
            }
            return Ok(());
        }
        self.pdq(0, n, None, depth_limit(n))
    }

    /// 9 個の中央値の中央値の場所
    fn pivot_at(&mut self, lo: usize, n: usize) -> R<usize> {
        let pos = samples(n);
        let mut m = [0; 3];
        for (g, mg) in m.iter_mut().enumerate() {
            *mg = self.m3_at(lo + pos[3 * g], lo + pos[3 * g + 1], lo + pos[3 * g + 2])?;
        }
        self.m3_at(m[0], m[1], m[2])
    }

    /// 3 つの場所のうち、値が中央のものの場所
    fn m3_at(&mut self, i: usize, j: usize, k: usize) -> R<usize> {
        let (mut i, mut j) = (i, j);
        if self.lt(self.a[j], self.a[i])? {
            std::mem::swap(&mut i, &mut j);
        }
        if self.lt(self.a[k], self.a[j])? {
            j = k;
            if self.lt(self.a[j], self.a[i])? {
                j = i;
            }
        }
        Ok(j)
    }

    fn pdq(&mut self, mut lo: usize, mut n: usize, mut anc: Option<usize>, mut limit: u32) -> R<()> {
        while n > SMALL {
            if limit == 0 {
                return self.heapsort(lo, n);
            }
            limit -= 1;
            let m = self.pivot_at(lo, n)?;
            self.a.swap(lo, m);
            let p = self.a[lo];
            if let Some(q) = anc {
                if !self.lt(q, p)? {
                    let k = self.part_le(lo, n, p)?;
                    lo += k;
                    n -= k;
                    anc = None;
                    continue;
                }
            }
            let mid = self.part_lt(lo, n, p)?;
            self.pdq(lo, mid - lo, anc, limit)?;
            anc = Some(p);
            n = lo + n - mid - 1;
            lo = mid + 1;
        }
        self.insertion(lo, n)
    }

    /// p（lo にある）より前のものを左へ。p を間に置いて、その場所を返す
    fn part_lt(&mut self, lo: usize, n: usize, p: usize) -> R<usize> {
        let (mut i, mut j) = (lo + 1, lo + n - 1);
        loop {
            while i <= j && self.lt(self.a[i], p)? {
                i += 1;
            }
            while i <= j && !self.lt(self.a[j], p)? {
                j -= 1;
            }
            if i > j {
                break;
            }
            self.a.swap(i, j);
            i += 1;
            j -= 1;
        }
        self.a.swap(lo, i - 1);
        Ok(i - 1)
    }

    /// p（lo にある）より後ろでないものを左へ。その数（p を含む）を返す
    fn part_le(&mut self, lo: usize, n: usize, p: usize) -> R<usize> {
        let (mut i, mut j) = (lo + 1, lo + n - 1);
        loop {
            while i <= j && !self.lt(p, self.a[i])? {
                i += 1;
            }
            while i <= j && self.lt(p, self.a[j])? {
                j -= 1;
            }
            if i > j {
                break;
            }
            self.a.swap(i, j);
            i += 1;
            j -= 1;
        }
        Ok(i - lo)
    }

    fn heapsort(&mut self, lo: usize, n: usize) -> R<()> {
        for start in (0..n / 2).rev() {
            self.sift(lo, start, n)?;
        }
        for end in (1..n).rev() {
            self.a.swap(lo, lo + end);
            self.sift(lo, 0, end)?;
        }
        Ok(())
    }

    fn sift(&mut self, lo: usize, mut root: usize, end: usize) -> R<()> {
        loop {
            let mut child = 2 * root + 1;
            if child >= end {
                return Ok(());
            }
            if child + 1 < end && self.lt(self.a[lo + child], self.a[lo + child + 1])? {
                child += 1;
            }
            if !self.lt(self.a[lo + root], self.a[lo + child])? {
                return Ok(());
            }
            self.a.swap(lo + root, lo + child);
            root = child;
        }
    }
}
