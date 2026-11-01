set_option autoImplicit false
set_option warningAsError true

namespace JabberwockFormal.CoreGenRef

/-!
世代つき ref の実行時検査の模型（docs/core-type-safety.md）。

`CoreBorrow` の ref は借用の期間の中でしか使えなかった。ここでは ref を外へ持ち出せる代わりに、
読むときに世代を確かめる。

* 変数の値は区画（`Cell`）に置く。区画は世代 `gen` と中身（持ち主の変数の位置と値）を持つ。中身がなければ空き。
* `letE x body` : 割り当ての方針 `pick` が選んだ空き区画（なければ末尾に新しい区画）に x の値を置いて body を評価する。
  body が終わると区画を空きにし、世代を 1 進める。空いた区画はあとで別の型の値に使い回されうる。
* `addr i` : 変数 i の区画の番号と今の世代の組を ref として返す。ref は let の外へ持ち出してよい（結果の型に制限がない）。
* `deref e` : ref の世代が区画の今の世代と同じときだけ中身を読む。違えば実行時の誤り（`err`）で止まる。
* 割り当ての方針は任意。条件は「空きで、世代が上限 `limit` 未満の区画だけを選ぶ」こと（上限に達した区画は退役して使い回さない）。

不変条件の要: ref の世代は区画の世代以下で、等しければ区画は生きていて、中身は ref の型に合う。
区画の世代は減らず、空けると増えるので、古い ref は古いままで、使い回された区画を読むことはない。

定理 `eval_safe`: 型が付く式を、どの割り当ての方針で評価しても stuck にならない
（解放済みの区画を読まず、使い回された区画を別の型として読まない）。

世代は `Nat` で、桁あふれしない。実行時ライブラリの有限の世代・退役の符号化は別の模型
（Generation・RuntimeGeneration*）が扱う。これは核の言語の模型の定理で、jwc・Rust 版の検査、
IR・LLVM・実行時ライブラリとの対応の証明ではない。
-/

inductive Ty where
  | int
  | bool
  | ref (t : Ty)

inductive Val where
  | int (n : Int)
  | bool (b : Bool)
  /-- 区画の番号 l と、作ったときの世代 g -/
  | ref (l g : Nat)

/-- 区画: 世代と中身（持ち主の変数の位置と値）。中身が none なら空き -/
structure Cell where
  gen : Nat
  val : Option (Nat × Val)

/-- ヒープ: 区画の列 -/
abbrev Heap := List Cell
/-- 環境: 変数の位置 → 区画の番号 -/
abbrev Env := List Nat
abbrev Ctx := List Ty

inductive Expr where
  | int (n : Int)
  | bool (b : Bool)
  | var (i : Nat)
  | add (a b : Expr)
  | letE (bound body : Expr)
  | addr (i : Nat)
  | deref (e : Expr)

def nth {α : Type} : List α → Nat → Option α
  | [], _ => none
  | a :: _, 0 => some a
  | _ :: l, i + 1 => nth l i

def setNth {α : Type} : List α → Nat → α → List α
  | [], _, _ => []
  | _ :: l, 0, a => a :: l
  | x :: l, i + 1, a => x :: setNth l i a

/-! ## リストの補題 -/

theorem nth_lt {α : Type} {l : List α} {j : Nat} {x : α} (h : nth l j = some x) : j < l.length := by
  induction l generalizing j with
  | nil =>
    have h' : (none : Option α) = some x := by
      cases j <;> exact h
    cases h'
  | cons _ l ih =>
    cases j with
    | zero => exact Nat.succ_pos _
    | succ j => exact Nat.succ_lt_succ (ih h)

theorem nth_append_lt {α : Type} {l : List α} {x : α} {j : Nat} (h : j < l.length) :
    nth (l ++ [x]) j = nth l j := by
  induction l generalizing j with
  | nil => exact absurd h (Nat.not_lt_zero _)
  | cons _ l ih =>
    cases j with
    | zero => rfl
    | succ j => exact ih (Nat.lt_of_succ_lt_succ h)

theorem nth_append_len {α : Type} (l : List α) (x : α) : nth (l ++ [x]) l.length = some x := by
  induction l with
  | nil => rfl
  | cons _ l ih => exact ih

theorem nth_setNth_eq {α : Type} {l : List α} {i : Nat} {x : α} (h : i < l.length) :
    nth (setNth l i x) i = some x := by
  induction l generalizing i with
  | nil => exact absurd h (Nat.not_lt_zero _)
  | cons _ l ih =>
    cases i with
    | zero => rfl
    | succ i => exact ih (Nat.lt_of_succ_lt_succ h)

theorem nth_setNth_ne {α : Type} {l : List α} {i j : Nat} {x : α} (h : j ≠ i) :
    nth (setNth l i x) j = nth l j := by
  induction l generalizing i j with
  | nil => rfl
  | cons _ l ih =>
    cases i with
    | zero =>
      cases j with
      | zero => exact absurd rfl h
      | succ _ => rfl
    | succ i =>
      cases j with
      | zero => rfl
      | succ j => exact ih (fun e => h (congrArg Nat.succ e))

/-! ## 型 -/

/-- 型付け。let の結果の型に制限はない（ref を外へ持ち出してよい） -/
inductive HasTy : Ctx → Expr → Ty → Prop where
  | int (Γ : Ctx) (n : Int) : HasTy Γ (.int n) .int
  | bool (Γ : Ctx) (b : Bool) : HasTy Γ (.bool b) .bool
  | var (Γ : Ctx) (i : Nat) (t : Ty) (hi : nth Γ i = some t) : HasTy Γ (.var i) t
  | add (Γ : Ctx) (a b : Expr) (ha : HasTy Γ a .int) (hb : HasTy Γ b .int) : HasTy Γ (.add a b) .int
  | letE (Γ : Ctx) (x body : Expr) (s t : Ty) (hx : HasTy Γ x s) (hb : HasTy (Γ ++ [s]) body t) :
      HasTy Γ (.letE x body) t
  | addr (Γ : Ctx) (i : Nat) (t : Ty) (hi : nth Γ i = some t) : HasTy Γ (.addr i) (.ref t)
  | deref (Γ : Ctx) (e : Expr) (t : Ty) (he : HasTy Γ e (.ref t)) : HasTy Γ (.deref e) t

/-! ## 評価 -/

inductive Res where
  | ok (v : Val) (H : Heap)
  | err
  | fuel
  | stuck

def Res.bind : Res → (Val → Heap → Res) → Res
  | .ok v H, f => f v H
  | .err, _ => .err
  | .fuel, _ => .fuel
  | .stuck, _ => .stuck

def checked (z : Int) (H : Heap) : Res :=
  if -(2 : Int) ^ 63 ≤ z ∧ z < (2 : Int) ^ 63 then .ok (.int z) H else .err

def addOp : Val → Val → Heap → Res
  | .int p, .int q, H => checked (p + q) H
  | _, _, _ => .stuck

/-- 変数 i を読む -/
def readVar (env : Env) (H : Heap) (i : Nat) : Res :=
  match nth env i with
  | some l =>
    match nth H l with
    | some c =>
      match c.val with
      | some (_, v) => .ok v H
      | none => .stuck
    | none => .stuck
  | none => .stuck

/-- 変数 i への ref: 区画の番号と今の世代 -/
def addrOf (env : Env) (H : Heap) (i : Nat) : Res :=
  match nth env i with
  | some l =>
    match nth H l with
    | some c => .ok (.ref l c.gen) H
    | none => .stuck
  | none => .stuck

/-- ref を読む: 世代が合わなければ実行時の誤り。合えば中身を読む（中身がなければ行き詰まり） -/
def derefOp : Val → Heap → Res
  | .ref l g, H =>
    match nth H l with
    | some c =>
      if c.gen = g then
        match c.val with
        | some (_, v) => .ok v H
        | none => .stuck
      else .err
    | none => .stuck
  | _, _ => .stuck

/-- 区画 l に中身 p を置く（世代はそのまま） -/
def allocAt (H : Heap) (l : Nat) (p : Nat × Val) : Heap :=
  match nth H l with
  | some c => setNth H l ⟨c.gen, some p⟩
  | none => H

/-- 割り当て: 方針が選んだ区画、なければ末尾に世代 0 の新しい区画 -/
def alloc (pick : Heap → Option Nat) (H : Heap) (p : Nat × Val) : Nat × Heap :=
  match pick H with
  | some l => (l, allocAt H l p)
  | none => (H.length, H ++ [⟨0, some p⟩])

/-- 解放: 区画を空きにして世代を 1 進める -/
def release (H : Heap) (l : Nat) : Heap :=
  match nth H l with
  | some c => setNth H l ⟨c.gen + 1, none⟩
  | none => H

def eval (pick : Heap → Option Nat) : Nat → Env → Heap → Expr → Res
  | 0, _, _, _ => .fuel
  | _ + 1, _, H, .int n => .ok (.int n) H
  | _ + 1, _, H, .bool b => .ok (.bool b) H
  | _ + 1, env, H, .var i => readVar env H i
  | _ + 1, env, H, .addr i => addrOf env H i
  | k + 1, env, H, .deref e => (eval pick k env H e).bind derefOp
  | k + 1, env, H, .add a b =>
    (eval pick k env H a).bind fun x H1 => (eval pick k env H1 b).bind fun y H2 => addOp x y H2
  | k + 1, env, H, .letE x body =>
    (eval pick k env H x).bind fun v H1 =>
      (eval pick k (env ++ [(alloc pick H1 (env.length, v)).1]) (alloc pick H1 (env.length, v)).2 body).bind
        fun w H2 => .ok w (release H2 (alloc pick H1 (env.length, v)).1)

/-- 割り当ての方針の条件: 空きで、世代が上限未満の区画だけを選ぶ -/
def PickOk (limit : Nat) (pick : Heap → Option Nat) : Prop :=
  ∀ (H : Heap) (l : Nat), pick H = some l → ∃ c : Cell, nth H l = some c ∧ c.val = none ∧ c.gen < limit

/-! ## 不変条件 -/

/-- 値が型に合う。ref は、世代が区画の世代以下で、等しければ区画が生きていて中身が型に合う -/
def ValOk (H : Heap) : Ty → Val → Prop
  | .int, .int _ => True
  | .bool, .bool _ => True
  | .ref t, .ref l g => ∃ c : Cell, nth H l = some c ∧ g ≤ c.gen ∧
      (g = c.gen → ∃ (o : Nat) (v : Val), c.val = some (o, v) ∧ ValOk H t v)
  | _, _ => False

/-- 環境とヒープが文脈に合う: 変数 j の区画は生きていて、持ち主は j、中身は型に合う -/
structure Agree (env : Env) (H : Heap) (Γ : Ctx) : Prop where
  len : env.length = Γ.length
  slot : ∀ (j : Nat) (t : Ty), nth Γ j = some t →
    ∃ (l : Nat) (c : Cell) (v : Val), nth env j = some l ∧ nth H l = some c ∧
      c.val = some (j, v) ∧ ValOk H t v

def Safe (t : Ty) (env : Env) (Γ : Ctx) : Res → Prop
  | .ok v H => ValOk H t v ∧ Agree env H Γ
  | .err => True
  | .fuel => True
  | .stuck => False

/-- ヒープの進み方: 区画は消えず、世代は減らず、世代が同じなら生きていた中身は変わらない -/
def HeapLe (H H' : Heap) : Prop :=
  ∀ (l : Nat) (c : Cell), nth H l = some c → ∃ c' : Cell, nth H' l = some c' ∧ c.gen ≤ c'.gen ∧
    (c.gen = c'.gen → ∀ p : Nat × Val, c.val = some p → c'.val = some p)

theorem valOk_mono {H H' : Heap} (hle : HeapLe H H') :
    ∀ (t : Ty) (v : Val), ValOk H t v → ValOk H' t v := by
  intro t
  induction t with
  | int =>
    intro v hv
    cases v <;> exact hv
  | bool =>
    intro v hv
    cases v <;> exact hv
  | ref t ih =>
    intro v hv
    cases v with
    | int _ => exact False.elim hv
    | bool _ => exact False.elim hv
    | ref l g =>
      have hv' : ∃ c : Cell, nth H l = some c ∧ g ≤ c.gen ∧
          (g = c.gen → ∃ (o : Nat) (w : Val), c.val = some (o, w) ∧ ValOk H t w) := hv
      obtain ⟨c, hc, hg, hlive⟩ := hv'
      obtain ⟨c', hc', hgen, hval⟩ := hle l c hc
      show ∃ c : Cell, nth H' l = some c ∧ g ≤ c.gen ∧
          (g = c.gen → ∃ (o : Nat) (w : Val), c.val = some (o, w) ∧ ValOk H' t w)
      refine Exists.intro c' (And.intro hc' (And.intro (Nat.le_trans hg hgen) ?_))
      intro heq
      have h1 : g = c.gen := by omega
      have h2 : c.gen = c'.gen := by omega
      obtain ⟨o, w, hw, hok⟩ := hlive h1
      exact Exists.intro o (Exists.intro w (And.intro (hval h2 (o, w) hw) (ih w hok)))

theorem heapLe_append (H : Heap) (x : Cell) : HeapLe H (H ++ [x]) := by
  unfold HeapLe
  intro l c h
  refine Exists.intro c (And.intro ?_ (And.intro (Nat.le_refl _) (fun _ _ hp => hp)))
  rw [nth_append_lt (nth_lt h)]
  exact h

theorem heapLe_fill {H : Heap} {l : Nat} {c : Cell} (p : Nat × Val) (hc : nth H l = some c)
    (hv : c.val = none) : HeapLe H (setNth H l ⟨c.gen, some p⟩) := by
  unfold HeapLe
  intro l0 c0 h0
  by_cases heq : l0 = l
  · rw [heq] at h0
    have h1 : c0 = c := Option.some.inj (h0.symm.trans hc)
    rw [h1, heq]
    refine Exists.intro ⟨c.gen, some p⟩
      (And.intro (nth_setNth_eq (nth_lt hc)) (And.intro (Nat.le_refl _) ?_))
    intro _ q hq
    rw [hv] at hq
    cases hq
  · refine Exists.intro c0 (And.intro ?_ (And.intro (Nat.le_refl _) (fun _ _ hq => hq)))
    rw [nth_setNth_ne heq]
    exact h0

theorem heapLe_release {H : Heap} {l : Nat} {c : Cell} (hc : nth H l = some c) :
    HeapLe H (setNth H l ⟨c.gen + 1, none⟩) := by
  unfold HeapLe
  intro l0 c0 h0
  by_cases heq : l0 = l
  · rw [heq] at h0
    have h1 : c0 = c := Option.some.inj (h0.symm.trans hc)
    rw [h1, heq]
    refine Exists.intro ⟨c.gen + 1, none⟩
      (And.intro (nth_setNth_eq (nth_lt hc)) (And.intro (Nat.le_succ _) ?_))
    intro hg
    have hg' : c.gen = c.gen + 1 := hg
    exact absurd hg' (by omega)
  · refine Exists.intro c0 (And.intro ?_ (And.intro (Nat.le_refl _) (fun _ _ hq => hq)))
    rw [nth_setNth_ne heq]
    exact h0

/-- 末尾に変数を足す: 新しい区画は生きていて持ち主は末尾、ほかの生きた区画はそのまま -/
theorem agree_push {env : Env} {H H' : Heap} {Γ : Ctx} {l : Nat} {c' : Cell} {t : Ty} {v : Val}
    (ha : Agree env H Γ) (hv : ValOk H t v) (hle : HeapLe H H')
    (hkeep : ∀ (l0 : Nat) (c0 : Cell) (p : Nat × Val), nth H l0 = some c0 → c0.val = some p →
      nth H' l0 = some c0)
    (hnew : nth H' l = some c') (hc' : c'.val = some (Γ.length, v)) :
    Agree (env ++ [l]) H' (Γ ++ [t]) := by
  have hlen : env.length = Γ.length := ha.len
  constructor
  · simp only [List.length_append, List.length_cons, List.length_nil]
    omega
  · intro j t' hj
    have hj' : j < Γ.length + 1 := by
      have h := nth_lt hj
      simp only [List.length_append, List.length_cons, List.length_nil] at h
      omega
    by_cases hlt : j < Γ.length
    · rw [nth_append_lt hlt] at hj
      obtain ⟨l0, c0, v0, he, hc, hcv, hok⟩ := ha.slot j t' hj
      refine ⟨l0, c0, v0, ?_, hkeep l0 c0 (j, v0) hc hcv, hcv, valOk_mono hle t' v0 hok⟩
      rw [nth_append_lt (by omega : j < env.length)]
      exact he
    · have hjj : j = Γ.length := by omega
      rw [hjj, nth_append_len] at hj
      have htt : t = t' := Option.some.inj hj
      rw [← htt]
      have he : nth (env ++ [l]) Γ.length = some l := by
        rw [← hlen]
        exact nth_append_len env l
      rw [hjj]
      exact ⟨l, c', v, he, hnew, hc', valOk_mono hle t v hv⟩

/-- let の割り当ての後も文脈に合う（方針は条件を満たせば何でもよい） -/
theorem alloc_ok {limit : Nat} {pick : Heap → Option Nat} (hp : PickOk limit pick) {env : Env}
    {H : Heap} {Γ : Ctx} {t : Ty} {v : Val} (ha : Agree env H Γ) (hv : ValOk H t v) :
    Agree (env ++ [(alloc pick H (env.length, v)).1]) (alloc pick H (env.length, v)).2 (Γ ++ [t]) := by
  have hlen : env.length = Γ.length := ha.len
  cases hpk : pick H with
  | none =>
    simp only [alloc, hpk]
    refine agree_push ha hv (heapLe_append H _) ?_ (nth_append_len H _) ?_
    · intro l0 c0 _ h0 _
      rw [nth_append_lt (nth_lt h0)]
      exact h0
    · show some (env.length, v) = some (Γ.length, v)
      rw [hlen]
  | some l =>
    obtain ⟨c, hc, hcv, _⟩ := hp H l hpk
    simp only [alloc, hpk, allocAt, hc]
    refine agree_push ha hv (heapLe_fill (env.length, v) hc hcv) ?_ (nth_setNth_eq (nth_lt hc)) ?_
    · intro l0 c0 p h0 hp0
      by_cases heq : l0 = l
      · rw [heq] at h0
        have h1 : c0 = c := Option.some.inj (h0.symm.trans hc)
        rw [h1, hcv] at hp0
        cases hp0
      · rw [nth_setNth_ne heq]
        exact h0
    · show some (env.length, v) = some (Γ.length, v)
      rw [hlen]

/-- let の終わりに区画を解放しても、外の変数の区画は残り、持ち出した値（古くなった ref も）は型に合う -/
theorem release_ok {env : Env} {H : Heap} {Γ : Ctx} {s t : Ty} {l : Nat} {w : Val}
    (ha : Agree (env ++ [l]) H (Γ ++ [s])) (hw : ValOk H t w) :
    Safe t env Γ (.ok w (release H l)) := by
  have hlen : env.length = Γ.length := by
    have h := ha.len
    simp only [List.length_append, List.length_cons, List.length_nil] at h
    omega
  obtain ⟨l0, c, v0, he, hc, hcv, _⟩ := ha.slot Γ.length s (nth_append_len Γ s)
  have hl0 : l0 = l := by
    rw [← hlen, nth_append_len] at he
    exact (Option.some.inj he).symm
  rw [hl0] at hc
  have hrel : release H l = setNth H l ⟨c.gen + 1, none⟩ := by
    simp only [release, hc]
  rw [hrel]
  have hle := heapLe_release hc
  refine And.intro (valOk_mono hle t w hw) ?_
  constructor
  · exact hlen
  · intro j t' hj
    have hlt : j < Γ.length := nth_lt hj
    obtain ⟨l1, c1, v1, he1, hc1, hcv1, hok1⟩ :=
      ha.slot j t' (by rw [nth_append_lt hlt]; exact hj)
    have hne : l1 ≠ l := by
      intro heq
      rw [heq] at hc1
      have h4 : c = c1 := Option.some.inj (hc.symm.trans hc1)
      rw [← h4] at hcv1
      have h2 : (Γ.length, v0) = (j, v1) := Option.some.inj (hcv.symm.trans hcv1)
      have h3 : Γ.length = j := congrArg Prod.fst h2
      omega
    refine ⟨l1, c1, v1, ?_, ?_, hcv1, valOk_mono hle t' v1 hok1⟩
    · rw [nth_append_lt (by omega : j < env.length)] at he1
      exact he1
    · rw [nth_setNth_ne hne]
      exact hc1

/-! ## 一つずつの操作の安全性 -/

theorem bind_safe {s t : Ty} {env1 env2 : Env} {Γ1 Γ2 : Ctx} {r : Res} {f : Val → Heap → Res}
    (hr : Safe s env1 Γ1 r)
    (hf : ∀ (v : Val) (H : Heap), ValOk H s v → Agree env1 H Γ1 → Safe t env2 Γ2 (f v H)) :
    Safe t env2 Γ2 (r.bind f) := by
  cases r with
  | ok v H => exact hf v H hr.1 hr.2
  | err => exact True.intro
  | fuel => exact True.intro
  | stuck => exact False.elim hr

theorem checked_safe {env : Env} {H : Heap} {Γ : Ctx} (z : Int) (ha : Agree env H Γ) :
    Safe .int env Γ (checked z H) := by
  unfold checked
  split
  · exact And.intro True.intro ha
  · exact True.intro

theorem addOp_safe {env : Env} {H Ha Hb : Heap} {Γ : Ctx} {x y : Val} (hx : ValOk Ha .int x)
    (hy : ValOk Hb .int y) (ha : Agree env H Γ) : Safe .int env Γ (addOp x y H) := by
  cases x with
  | bool _ => exact False.elim hx
  | ref _ _ => exact False.elim hx
  | int p =>
    cases y with
    | bool _ => exact False.elim hy
    | ref _ _ => exact False.elim hy
    | int q => exact checked_safe (p + q) ha

theorem readVar_safe {env : Env} {H : Heap} {Γ : Ctx} {i : Nat} {t : Ty} (ha : Agree env H Γ)
    (hi : nth Γ i = some t) : Safe t env Γ (readVar env H i) := by
  obtain ⟨l, c, v, he, hc, hcv, hok⟩ := ha.slot i t hi
  simp only [readVar, he, hc, hcv]
  exact And.intro hok ha

theorem addrOf_safe {env : Env} {H : Heap} {Γ : Ctx} {i : Nat} {t : Ty} (ha : Agree env H Γ)
    (hi : nth Γ i = some t) : Safe (.ref t) env Γ (addrOf env H i) := by
  obtain ⟨l, c, v, he, hc, hcv, hok⟩ := ha.slot i t hi
  simp only [addrOf, he, hc]
  refine And.intro ?_ ha
  exact Exists.intro c (And.intro hc (And.intro (Nat.le_refl _)
    (fun _ => Exists.intro i (Exists.intro v (And.intro hcv hok)))))

theorem derefOp_safe {env : Env} {H : Heap} {Γ : Ctx} {t : Ty} {v : Val} (hv : ValOk H (.ref t) v)
    (ha : Agree env H Γ) : Safe t env Γ (derefOp v H) := by
  cases v with
  | int _ => exact False.elim hv
  | bool _ => exact False.elim hv
  | ref l g =>
    have hv' : ∃ c : Cell, nth H l = some c ∧ g ≤ c.gen ∧
        (g = c.gen → ∃ (o : Nat) (w : Val), c.val = some (o, w) ∧ ValOk H t w) := hv
    obtain ⟨c, hc, _, hlive⟩ := hv'
    simp only [derefOp, hc]
    by_cases heq : c.gen = g
    · rw [if_pos heq]
      obtain ⟨o, w, hw, hok⟩ := hlive heq.symm
      simp only [hw]
      exact And.intro hok ha
    · rw [if_neg heq]
      exact True.intro

/-! ## 主定理 -/

/-- 型安全性と世代の検査: 型が付く式を、文脈と合う環境・ヒープで、条件を満たすどの割り当ての方針で評価しても、結果は Safe -/
theorem eval_safe {limit : Nat} (pick : Heap → Option Nat) (hp : PickOk limit pick) :
    ∀ (k : Nat) (env : Env) (H : Heap) (Γ : Ctx) (e : Expr) (t : Ty),
      Agree env H Γ → HasTy Γ e t → Safe t env Γ (eval pick k env H e) := by
  intro k
  induction k with
  | zero =>
    intro env H Γ e t _ _
    cases e <;> exact True.intro
  | succ k ih =>
    intro env H Γ e t hag h
    cases h with
    | int _ _ => exact And.intro True.intro hag
    | bool _ _ => exact And.intro True.intro hag
    | var _ _ _ hi =>
      simp only [eval]
      exact readVar_safe hag hi
    | addr _ _ _ hi =>
      simp only [eval]
      exact addrOf_safe hag hi
    | deref _ e _ he =>
      simp only [eval]
      apply bind_safe (ih env H Γ e (.ref t) hag he)
      intro v H1 hv ha1
      exact derefOp_safe hv ha1
    | add _ a b ha hb =>
      simp only [eval]
      apply bind_safe (ih env H Γ a .int hag ha)
      intro x H1 hx ha1
      apply bind_safe (ih env H1 Γ b .int ha1 hb)
      intro y H2 hy ha2
      exact addOp_safe hx hy ha2
    | letE _ x body s _ hx hb =>
      simp only [eval]
      apply bind_safe (ih env H Γ x s hag hx)
      intro v H1 hv ha1
      apply bind_safe (ih _ _ (Γ ++ [s]) body t (alloc_ok hp ha1 hv) hb)
      intro w H2 hw ha2
      exact release_ok ha2 hw

/-- 型が付く式は、どれだけの燃料でも、どの割り当ての方針でも行き詰まらない
（解放済みの区画も、使い回された区画の別の型の中身も読まない） -/
theorem no_stuck {limit : Nat} (pick : Heap → Option Nat) (hp : PickOk limit pick) (k : Nat)
    (env : Env) (H : Heap) (Γ : Ctx) (e : Expr) (t : Ty) (hag : Agree env H Γ) (h : HasTy Γ e t) :
    eval pick k env H e ≠ .stuck := by
  intro hs
  have hsafe := eval_safe pick hp k env H Γ e t hag h
  rw [hs] at hsafe
  exact hsafe

/-! ## 意味を持つことの確認 -/

/-- 世代が合わない ref を読むと、必ず実行時の誤りで止まる -/
theorem stale_ref_err {H : Heap} {l g : Nat} {c : Cell} (hc : nth H l = some c) (hg : c.gen ≠ g) :
    derefOp (.ref l g) H = .err := by
  simp only [derefOp, hc]
  rw [if_neg hg]

/-- いつも末尾に新しい区画を足す方針は条件を満たす（上の定理の前提が空でないことの確認） -/
theorem pick_none_ok (limit : Nat) : PickOk limit (fun _ => none) := by
  unfold PickOk
  intro _ l h
  have h' : (none : Option Nat) = some l := h
  cases h'

/-- 外へ持ち出した ref: `deref (let x = 1 in &x)`。型が付く -/
theorem escaped_typed : HasTy [] (.deref (.letE (.int 1) (.addr 0))) .int :=
  HasTy.deref _ _ _ (HasTy.letE _ _ _ .int (.ref .int) (HasTy.int _ _) (HasTy.addr _ 0 .int rfl))

/-- 外へ持ち出した ref を読むと、区画は解放済み（世代 1）で ref は世代 0 なので、実行時の誤りで止まる -/
theorem escaped_ref_err :
    eval (fun _ => none) 3 [] [] (.deref (.letE (.int 1) (.addr 0))) = .err := rfl

/-- 先頭の区画が空きなら使い回す方針（世代の上限は見ない。計算の例のためだけ） -/
def reuseFirst : Heap → Option Nat
  | ⟨_, none⟩ :: _ => some 0
  | _ => none

/-- 使い回し: `let r = (let x = 1 in &x) in deref r`。r は x の区画（世代 1）に置かれ、
古い ref（世代 0）を読むと実行時の誤りで止まる（検査がなければ r 自身を int として読んでしまう） -/
theorem reused_typed : HasTy [] (.letE (.letE (.int 1) (.addr 0)) (.deref (.var 0))) .int :=
  HasTy.letE _ _ _ (.ref .int) .int
    (HasTy.letE _ _ _ .int (.ref .int) (HasTy.int _ _) (HasTy.addr _ 0 .int rfl))
    (HasTy.deref _ _ _ (HasTy.var _ 0 (.ref .int) rfl))

theorem reused_ref_err :
    eval reuseFirst 4 [] [] (.letE (.letE (.int 1) (.addr 0)) (.deref (.var 0))) = .err := rfl

end JabberwockFormal.CoreGenRef
