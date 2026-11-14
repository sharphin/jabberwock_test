import JabberwockFormal.CoreMutBorrow

set_option autoImplicit false
set_option warningAsError true

namespace JabberwockFormal.CoreGenBoxMut

open JabberwockFormal.CoreMutBorrow (nth setNth lastOpt init nth_lt nth_append_lt nth_append_len
  length_setNth nth_setNth_eq nth_setNth_ne split_last init_append lastOpt_append)

/-!
持ち主の箱と世代つき ref の模型（`CoreGenBox`）に、ref を通した書き込み（`ref mut`）を足した模型
（docs/core-type-safety.md）。

* `addr m i` : 変数 i の箱への ref。m = true なら書き込める ref（`ref mut`）。
* `store a b` : a（書き込める ref）の指す区画に b の値を書く。世代が区画の世代と違えば実行時の誤り（`err`）。
  結果は 0（模型の都合。言語では値を持たない）。
* `mkBox t e` : 区画に型 t の記録をつけて値を置く。型の記録は検証のための記録で、評価はこれを読まない
  （持ち主の記録と同じ）。

区画の型の記録は、区画が生きているあいだ変わらない（書き込みは型の記録をそのまま残す）。
ヒープ全体の不変条件 `HeapOk`: 生きた区画の中身は、箱を含まない型（`Plain`）で、その型に合う。

定理 `eval_safe`: 型が付く式を、文脈と合う環境・ヒープで、条件を満たすどの割り当ての方針で評価しても stuck にならない。
書き込みのあとも、移動済みの変数を使わず、解放済みの区画を読み書きせず、使い回された区画を別の型として読まない。

`ref mut` の重なり（同じ区画への書き込める ref が二つ以上あること）はこの模型では禁じない。
世代の検査と区画の型の記録だけで、重なっても stuck にならないことを示す。jwc・Rust 版の借用の検査
（重なりの禁止）は扱わない。関数、箱の中の箱、有限の世代の上限も扱わない。
-/

inductive Ty where
  | int
  | bool
  | box (t : Ty)
  /-- m = true なら書き込める ref -/
  | ref (m : Bool) (t : Ty)

/-- 箱を含まない型 -/
def Plain : Ty → Prop
  | .int => True
  | .bool => True
  | .box _ => False
  | .ref _ t => Plain t

inductive Val where
  | int (n : Int)
  | bool (b : Bool)
  /-- 区画 l を持つ箱 -/
  | box (l : Nat)
  /-- 区画の番号 l と、作ったときの世代 g -/
  | ref (l g : Nat)

/-- 区画: 世代と中身（持ち主の記録・型の記録・値）。中身が none なら空き -/
structure Cell where
  gen : Nat
  val : Option (Nat × Ty × Val)

abbrev Heap := List Cell
/-- 環境: 変数の位置 → 値（移動済みなら none） -/
abbrev Env := List (Option Val)
/-- 文脈: 型と、生きているか -/
abbrev Ctx := List (Ty × Bool)

inductive Expr where
  | int (n : Int)
  | bool (b : Bool)
  | var (i : Nat)
  | move (i : Nat)
  | peek (i : Nat)
  | addr (m : Bool) (i : Nat)
  | deref (e : Expr)
  | store (a b : Expr)
  | mkBox (t : Ty) (e : Expr)
  | add (a b : Expr)
  | letE (bound body : Expr)

/-! ## 型 -/

/-- 型付け `HasTy Γ e t Γ'`: Γ' は評価の後の文脈（移動した変数は死ぬ） -/
inductive HasTy : Ctx → Expr → Ty → Ctx → Prop where
  | int (Γ : Ctx) (n : Int) : HasTy Γ (.int n) .int Γ
  | bool (Γ : Ctx) (b : Bool) : HasTy Γ (.bool b) .bool Γ
  | var (Γ : Ctx) (i : Nat) (t : Ty) (hi : nth Γ i = some (t, true)) (hc : Plain t) :
      HasTy Γ (.var i) t Γ
  | move (Γ : Ctx) (i : Nat) (t : Ty) (hi : nth Γ i = some (.box t, true)) :
      HasTy Γ (.move i) (.box t) (setNth Γ i (.box t, false))
  | peek (Γ : Ctx) (i : Nat) (t : Ty) (hi : nth Γ i = some (.box t, true)) :
      HasTy Γ (.peek i) t Γ
  | addr (Γ : Ctx) (m : Bool) (i : Nat) (t : Ty) (hi : nth Γ i = some (.box t, true)) :
      HasTy Γ (.addr m i) (.ref m t) Γ
  | deref (Γ Γ' : Ctx) (e : Expr) (m : Bool) (t : Ty) (he : HasTy Γ e (.ref m t) Γ') :
      HasTy Γ (.deref e) t Γ'
  | store (Γ Γ1 Γ2 : Ctx) (a b : Expr) (t : Ty) (ha : HasTy Γ a (.ref true t) Γ1)
      (hb : HasTy Γ1 b t Γ2) : HasTy Γ (.store a b) .int Γ2
  | mkBox (Γ Γ' : Ctx) (e : Expr) (t : Ty) (he : HasTy Γ e t Γ') (hc : Plain t) :
      HasTy Γ (.mkBox t e) (.box t) Γ'
  | add (Γ Γ1 Γ2 : Ctx) (a b : Expr) (ha : HasTy Γ a .int Γ1) (hb : HasTy Γ1 b .int Γ2) :
      HasTy Γ (.add a b) .int Γ2
  | letE (Γ Γ1 Γ2 : Ctx) (x body : Expr) (s t : Ty) (p : Ty × Bool) (hx : HasTy Γ x s Γ1)
      (hb : HasTy (Γ1 ++ [(s, true)]) body t (Γ2 ++ [p])) : HasTy Γ (.letE x body) t Γ2

/-! ## 評価 -/

inductive Res where
  | ok (v : Val) (env : Env) (H : Heap)
  | err
  | fuel
  | stuck

def Res.bind : Res → (Val → Env → Heap → Res) → Res
  | .ok v env H, f => f v env H
  | .err, _ => .err
  | .fuel, _ => .fuel
  | .stuck, _ => .stuck

def checked (z : Int) (env : Env) (H : Heap) : Res :=
  if -(2 : Int) ^ 63 ≤ z ∧ z < (2 : Int) ^ 63 then .ok (.int z) env H else .err

def addOp : Val → Val → Env → Heap → Res
  | .int p, .int q, env, H => checked (p + q) env H
  | _, _, _, _ => .stuck

def readVar (env : Env) (H : Heap) (i : Nat) : Res :=
  match nth env i with
  | some (some v) => .ok v env H
  | _ => .stuck

def moveOut (env : Env) (H : Heap) (i : Nat) : Res :=
  match nth env i with
  | some (some (.box l)) => .ok (.box l) (setNth env i none) H
  | _ => .stuck

/-- 区画 l の中身を読む -/
def contents (H : Heap) (l : Nat) (env : Env) : Res :=
  match nth H l with
  | some c =>
    match c.val with
    | some (_, _, w) => .ok w env H
    | none => .stuck
  | none => .stuck

def peekAt (env : Env) (H : Heap) (i : Nat) : Res :=
  match nth env i with
  | some (some (.box l)) => contents H l env
  | _ => .stuck

def addrOf (env : Env) (H : Heap) (i : Nat) : Res :=
  match nth env i with
  | some (some (.box l)) =>
    match nth H l with
    | some c => .ok (.ref l c.gen) env H
    | none => .stuck
  | _ => .stuck

/-- ref を読む: 世代が合わなければ実行時の誤り -/
def derefOp : Val → Env → Heap → Res
  | .ref l g, env, H =>
    match nth H l with
    | some c => if c.gen = g then contents H l env else .err
    | none => .stuck
  | _, _, _ => .stuck

/-- ref を通して書く: 世代が合わなければ実行時の誤り。持ち主の記録と型の記録はそのまま -/
def storeOp : Val → Val → Env → Heap → Res
  | .ref l g, w, env, H =>
    match nth H l with
    | some c =>
      if c.gen = g then
        match c.val with
        | some (o, t, _) => .ok (.int 0) env (setNth H l ⟨c.gen, some (o, t, w)⟩)
        | none => .stuck
      else .err
    | none => .stuck
  | _, _, _, _ => .stuck

def allocAt (H : Heap) (l : Nat) (p : Nat × Ty × Val) : Heap :=
  match nth H l with
  | some c => setNth H l ⟨c.gen, some p⟩
  | none => H

def alloc (pick : Heap → Option Nat) (H : Heap) (p : Nat × Ty × Val) : Nat × Heap :=
  match pick H with
  | some l => (l, allocAt H l p)
  | none => (H.length, H ++ [⟨0, some p⟩])

def release (H : Heap) (l : Nat) : Heap :=
  match nth H l with
  | some c => setNth H l ⟨c.gen + 1, none⟩
  | none => H

/-- 箱の区画の持ち主の記録を o に書き換える（検証のための記録） -/
def claim (H : Heap) (v : Val) (o : Nat) : Heap :=
  match v with
  | .box l =>
    match nth H l with
    | some c =>
      match c.val with
      | some (_, t, w) => setNth H l ⟨c.gen, some (o, t, w)⟩
      | none => H
    | none => H
  | _ => H

/-- 変数の終わり: 生きた箱を持っていれば区画を解放する -/
def dropSlot (H : Heap) : Option (Option Val) → Heap
  | some (some (.box l)) => release H l
  | _ => H

def eval (pick : Heap → Option Nat) : Nat → Env → Heap → Expr → Res
  | 0, _, _, _ => .fuel
  | _ + 1, env, H, .int n => .ok (.int n) env H
  | _ + 1, env, H, .bool b => .ok (.bool b) env H
  | _ + 1, env, H, .var i => readVar env H i
  | _ + 1, env, H, .move i => moveOut env H i
  | _ + 1, env, H, .peek i => peekAt env H i
  | _ + 1, env, H, .addr _ i => addrOf env H i
  | k + 1, env, H, .deref e => (eval pick k env H e).bind derefOp
  | k + 1, env, H, .store a b =>
    (eval pick k env H a).bind fun v env1 H1 =>
      (eval pick k env1 H1 b).bind fun w env2 H2 => storeOp v w env2 H2
  | k + 1, env, H, .mkBox t e =>
    (eval pick k env H e).bind fun v env1 H1 =>
      .ok (.box (alloc pick H1 (env1.length, t, v)).1) env1 (alloc pick H1 (env1.length, t, v)).2
  | k + 1, env, H, .add a b =>
    (eval pick k env H a).bind fun x env1 H1 =>
      (eval pick k env1 H1 b).bind fun y env2 H2 => addOp x y env2 H2
  | k + 1, env, H, .letE x body =>
    (eval pick k env H x).bind fun v env1 H1 =>
      (eval pick k (env1 ++ [some v]) (claim H1 v env1.length) body).bind fun w env2 H2 =>
        .ok w (init env2) (dropSlot H2 (lastOpt env2))

/-- 割り当ての方針の条件: 空き区画だけを選ぶ -/
def PickOk (pick : Heap → Option Nat) : Prop :=
  ∀ (H : Heap) (l : Nat), pick H = some l → ∃ c : Cell, nth H l = some c ∧ c.val = none

/-! ## 不変条件 -/

/-- 値が型に合う。箱は型の記録が t の生きた区画を指す。ref は世代が区画の世代以下で、
等しければ区画が生きていて型の記録が t -/
def ValOk (H : Heap) : Ty → Val → Prop
  | .int, .int _ => True
  | .bool, .bool _ => True
  | .box t, .box l => ∃ (c : Cell) (o : Nat) (w : Val), nth H l = some c ∧ c.val = some (o, t, w)
  | .ref _ t, .ref l g => ∃ c : Cell, nth H l = some c ∧ g ≤ c.gen ∧
      (g = c.gen → ∃ (o : Nat) (w : Val), c.val = some (o, t, w))
  | _, _ => False

/-- H' の生きた区画の中身は、箱を含まない型で、H でその型に合う -/
def ContentsOk (H H' : Heap) : Prop :=
  ∀ (l : Nat) (c : Cell) (o : Nat) (t : Ty) (w : Val), nth H' l = some c →
    c.val = some (o, t, w) → Plain t ∧ ValOk H t w

/-- ヒープの不変条件: 生きた区画の中身が型の記録に合う -/
def HeapOk (H : Heap) : Prop := ContentsOk H H

/-- 箱の値なら、その区画の持ち主の記録は j -/
def Owns (H : Heap) (j : Nat) : Val → Prop
  | .box l => ∃ (c : Cell) (t : Ty) (w : Val), nth H l = some c ∧ c.val = some (j, t, w)
  | _ => True

/-- 評価の結果の箱は、生きた変数が持っていない（持ち主の記録が生きた変数でない） -/
def Loose (H : Heap) (Γ : Ctx) : Val → Prop
  | .box l => ∃ (c : Cell) (o : Nat) (t : Ty) (w : Val), nth H l = some c ∧ c.val = some (o, t, w) ∧
      ∀ t' : Ty, nth Γ o ≠ some (t', true)
  | _ => True

/-- 環境とヒープが文脈に合う: 生きた変数は値を持ち、型に合い、箱なら持ち主の記録が自分。死んだ変数は空。
ヒープの中身は型の記録に合う -/
structure Agree (env : Env) (H : Heap) (Γ : Ctx) : Prop where
  len : env.length = Γ.length
  live : ∀ (j : Nat) (t : Ty), nth Γ j = some (t, true) →
    ∃ v : Val, nth env j = some (some v) ∧ ValOk H t v ∧ Owns H j v
  dead : ∀ (j : Nat) (t : Ty), nth Γ j = some (t, false) → nth env j = some none
  heap : HeapOk H

/-- ヒープの進み方: 区画は消えず、世代は減らず、世代が同じなら生きていた区画は生きたままで型の記録は同じ
（値と持ち主の記録は変わってよい） -/
def HeapLe (H H' : Heap) : Prop :=
  ∀ (l : Nat) (c : Cell), nth H l = some c → ∃ c' : Cell, nth H' l = some c' ∧ c.gen ≤ c'.gen ∧
    (c.gen = c'.gen → ∀ (o : Nat) (t : Ty) (w : Val), c.val = some (o, t, w) →
      ∃ (o' : Nat) (w' : Val), c'.val = some (o', t, w'))

/-- 区画 l が生きていれば、生きたままで持ち主の記録と型の記録が同じ（値は変わってよい） -/
def Keep (H H' : Heap) (l : Nat) : Prop :=
  ∀ (c : Cell) (o : Nat) (t : Ty) (w : Val), nth H l = some c → c.val = some (o, t, w) →
    ∃ (c' : Cell) (w' : Val), nth H' l = some c' ∧ c'.val = some (o, t, w')

/-- 結果が安全: 値が型に合い、環境とヒープが文脈に合い、結果の箱は誰も持たず、ヒープは H0 から進んだもの -/
def Safe (t : Ty) (Γ : Ctx) (H0 : Heap) : Res → Prop
  | .ok v env H => ValOk H t v ∧ Agree env H Γ ∧ Loose H Γ v ∧ HeapLe H0 H
  | .err => True
  | .fuel => True
  | .stuck => False

/-! ## ヒープの補題 -/

theorem triple_inj {o o' : Nat} {t t' : Ty} {w w' : Val}
    (h : (some (o, t, w) : Option (Nat × Ty × Val)) = some (o', t', w')) :
    o = o' ∧ t = t' ∧ w = w' := by
  have h1 : (o, t, w) = (o', t', w') := Option.some.inj h
  exact ⟨congrArg Prod.fst h1, congrArg (fun p => p.2.1) h1, congrArg (fun p => p.2.2) h1⟩

theorem heapLe_refl (H : Heap) : HeapLe H H := by
  unfold HeapLe
  intro l c h
  exact ⟨c, h, Nat.le_refl _, fun _ o _ w hp => ⟨o, w, hp⟩⟩

theorem heapLe_trans {H1 H2 H3 : Heap} (h12 : HeapLe H1 H2) (h23 : HeapLe H2 H3) : HeapLe H1 H3 := by
  unfold HeapLe
  intro l c h1
  obtain ⟨c2, h2, hg12, hv12⟩ := h12 l c h1
  obtain ⟨c3, h3, hg23, hv23⟩ := h23 l c2 h2
  refine ⟨c3, h3, Nat.le_trans hg12 hg23, ?_⟩
  intro heq o t w hw
  have e12 : c.gen = c2.gen := by omega
  have e23 : c2.gen = c3.gen := by omega
  obtain ⟨o2, w2, hw2⟩ := hv12 e12 o t w hw
  exact hv23 e23 o2 t w2 hw2

theorem heapLe_append (H : Heap) (x : Cell) : HeapLe H (H ++ [x]) := by
  unfold HeapLe
  intro l c h
  refine ⟨c, ?_, Nat.le_refl _, fun _ o _ w hp => ⟨o, w, hp⟩⟩
  rw [nth_append_lt (nth_lt h)]
  exact h

/-- 区画を一つ書き換える: 世代が減らず、世代が同じなら生きたままで型の記録が同じ -/
theorem heapLe_set {H : Heap} {l : Nat} {c c' : Cell} (hc : nth H l = some c) (hg : c.gen ≤ c'.gen)
    (hv : c.gen = c'.gen → ∀ (o : Nat) (t : Ty) (w : Val), c.val = some (o, t, w) →
      ∃ (o' : Nat) (w' : Val), c'.val = some (o', t, w')) : HeapLe H (setNth H l c') := by
  unfold HeapLe
  intro l0 c0 h0
  by_cases heq : l0 = l
  · rw [heq] at h0
    have h1 : c0 = c := Option.some.inj (h0.symm.trans hc)
    rw [h1, heq]
    exact ⟨c', nth_setNth_eq (nth_lt hc), hg, hv⟩
  · refine ⟨c0, ?_, Nat.le_refl _, fun _ o _ w hq => ⟨o, w, hq⟩⟩
    rw [nth_setNth_ne heq]
    exact h0

theorem heapLe_fill {H : Heap} {l : Nat} {c : Cell} (p : Nat × Ty × Val) (hc : nth H l = some c)
    (hv : c.val = none) : HeapLe H (setNth H l ⟨c.gen, some p⟩) := by
  refine heapLe_set (c' := ⟨c.gen, some p⟩) hc (Nat.le_refl _) ?_
  intro _ _ _ _ hq
  rw [hv] at hq
  cases hq

theorem release_heapLe (H : Heap) (l : Nat) : HeapLe H (release H l) := by
  cases hc : nth H l with
  | none =>
    simp only [release, hc]
    exact heapLe_refl H
  | some c =>
    simp only [release, hc]
    refine heapLe_set (c' := ⟨c.gen + 1, none⟩) hc (Nat.le_succ _) ?_
    intro hg
    have hg' : c.gen = c.gen + 1 := hg
    exact absurd hg' (by omega)

theorem release_ne {H : Heap} {l l1 : Nat} (h : l1 ≠ l) : nth (release H l) l1 = nth H l1 := by
  cases hc : nth H l with
  | none => simp only [release, hc]
  | some c =>
    simp only [release, hc]
    exact nth_setNth_ne h

theorem keep_of_eq {H H' : Heap} {l : Nat} (h : nth H' l = nth H l) : Keep H H' l := by
  unfold Keep
  intro c o t w hc hv
  refine ⟨c, w, ?_, hv⟩
  rw [h]
  exact hc

/-- 箱を含まない型の値は、ヒープが進んでも型に合う -/
theorem val_mono_plain {H H' : Heap} {t : Ty} {v : Val} (hle : HeapLe H H') (hp : Plain t)
    (hv : ValOk H t v) : ValOk H' t v := by
  cases v with
  | box _ =>
    cases t with
    | box _ => exact False.elim hp
    | int => exact False.elim hv
    | bool => exact False.elim hv
    | ref _ _ => exact False.elim hv
  | int _ =>
    cases t with
    | int => exact hv
    | bool => exact False.elim hv
    | box _ => exact False.elim hv
    | ref _ _ => exact False.elim hv
  | bool _ =>
    cases t with
    | bool => exact hv
    | int => exact False.elim hv
    | box _ => exact False.elim hv
    | ref _ _ => exact False.elim hv
  | ref l g =>
    cases t with
    | ref _ t =>
      have hv' : ∃ c : Cell, nth H l = some c ∧ g ≤ c.gen ∧
          (g = c.gen → ∃ (o : Nat) (w : Val), c.val = some (o, t, w)) := hv
      obtain ⟨c, hc, hg, hlive⟩ := hv'
      obtain ⟨c', hc', hgen, hval⟩ := hle l c hc
      show ∃ c : Cell, nth H' l = some c ∧ g ≤ c.gen ∧
          (g = c.gen → ∃ (o : Nat) (w : Val), c.val = some (o, t, w))
      refine ⟨c', hc', Nat.le_trans hg hgen, ?_⟩
      intro heq
      have h1 : g = c.gen := by omega
      have h2 : c.gen = c'.gen := by omega
      obtain ⟨o, w, hw⟩ := hlive h1
      exact hval h2 o t w hw
    | int => exact False.elim hv
    | bool => exact False.elim hv
    | box _ => exact False.elim hv

/-- 中身の条件を進んだヒープへ移す -/
theorem heapOk_step {H H' : Heap} (hle : HeapLe H H') (hc : ContentsOk H H') : HeapOk H' := by
  unfold HeapOk ContentsOk
  intro l c o t w h1 h2
  have hc' := hc l c o t w h1 h2
  exact ⟨hc'.1, val_mono_plain hle hc'.1 hc'.2⟩

/-- 区画を一つ書き換えたヒープの中身: 新しい区画の中身が合えば合う -/
theorem contents_set {H : Heap} {l : Nat} {c' : Cell} (hok : HeapOk H) (hl : l < H.length)
    (hnew : ∀ (o : Nat) (t : Ty) (w : Val), c'.val = some (o, t, w) → Plain t ∧ ValOk H t w) :
    ContentsOk H (setNth H l c') := by
  unfold ContentsOk
  intro l1 c1 o t w h1 h2
  by_cases heq : l1 = l
  · rw [heq, nth_setNth_eq hl] at h1
    have h3 : c' = c1 := Option.some.inj h1
    rw [← h3] at h2
    exact hnew o t w h2
  · rw [nth_setNth_ne heq] at h1
    exact hok l1 c1 o t w h1 h2

theorem release_contents {H : Heap} (hok : HeapOk H) (l : Nat) : ContentsOk H (release H l) := by
  cases hc : nth H l with
  | none =>
    simp only [release, hc]
    exact hok
  | some c =>
    simp only [release, hc]
    apply contents_set hok (nth_lt hc)
    intro o t w h
    have h0 : (none : Option (Nat × Ty × Val)) = some (o, t, w) := h
    cases h0

theorem claim_heapLe (H : Heap) (v : Val) (o : Nat) : HeapLe H (claim H v o) := by
  cases v with
  | box l =>
    cases hc : nth H l with
    | none =>
      simp only [claim, hc]
      exact heapLe_refl H
    | some c =>
      cases hv : c.val with
      | none =>
        simp only [claim, hc, hv]
        exact heapLe_refl H
      | some p =>
        obtain ⟨o0, t0, w⟩ := p
        simp only [claim, hc, hv]
        refine heapLe_set (c' := ⟨c.gen, some (o, t0, w)⟩) hc (Nat.le_refl _) ?_
        intro _ o1 t1 w1 h2
        rw [hv] at h2
        obtain ⟨_, ht, _⟩ := triple_inj h2
        rw [← ht]
        exact ⟨o, w, rfl⟩
  | int _ => exact heapLe_refl H
  | bool _ => exact heapLe_refl H
  | ref _ _ => exact heapLe_refl H

theorem claim_ne {H : Heap} {v : Val} {o l : Nat} (h : ∀ l' : Nat, v = .box l' → l' ≠ l) :
    nth (claim H v o) l = nth H l := by
  cases v with
  | box l' =>
    have hne : l' ≠ l := h l' rfl
    cases hc : nth H l' with
    | none => simp only [claim, hc]
    | some c =>
      cases hv : c.val with
      | none => simp only [claim, hc, hv]
      | some p =>
        obtain ⟨o0, t0, w⟩ := p
        simp only [claim, hc, hv]
        exact nth_setNth_ne (Ne.symm hne)
  | int _ => rfl
  | bool _ => rfl
  | ref _ _ => rfl

theorem claim_self {H : Heap} {l : Nat} {c : Cell} {o n : Nat} {t : Ty} {w : Val}
    (hc : nth H l = some c) (hcv : c.val = some (o, t, w)) :
    nth (claim H (.box l) n) l = some ⟨c.gen, some (n, t, w)⟩ := by
  simp only [claim, hc, hcv]
  exact nth_setNth_eq (nth_lt hc)

theorem claim_contents {H : Heap} (hok : HeapOk H) (v : Val) (n : Nat) :
    ContentsOk H (claim H v n) := by
  cases v with
  | box l =>
    cases hc : nth H l with
    | none =>
      simp only [claim, hc]
      exact hok
    | some c =>
      cases hv : c.val with
      | none =>
        simp only [claim, hc, hv]
        exact hok
      | some p =>
        obtain ⟨o0, t0, w0⟩ := p
        simp only [claim, hc, hv]
        apply contents_set hok (nth_lt hc)
        intro o t w h
        obtain ⟨_, ht, hw⟩ := triple_inj h
        rw [← ht, ← hw]
        exact hok l c o0 t0 w0 hc hv
  | int _ => exact hok
  | bool _ => exact hok
  | ref _ _ => exact hok

theorem alloc_spec {pick : Heap → Option Nat} (hp : PickOk pick) (H : Heap) (q : Nat × Ty × Val) :
    HeapLe H (alloc pick H q).2 ∧
    (∀ (l : Nat) (c : Cell) (p : Nat × Ty × Val), nth H l = some c → c.val = some p →
      nth (alloc pick H q).2 l = some c) ∧
    (∃ c' : Cell, nth (alloc pick H q).2 (alloc pick H q).1 = some c' ∧ c'.val = some q) ∧
    (∀ (l : Nat) (c : Cell) (p : Nat × Ty × Val), nth (alloc pick H q).2 l = some c →
      c.val = some p → nth H l = some c ∨ p = q) := by
  cases hpk : pick H with
  | none =>
    simp only [alloc, hpk]
    refine ⟨heapLe_append H _, ?_, ⟨_, nth_append_len H _, rfl⟩, ?_⟩
    · intro l c _ h _
      rw [nth_append_lt (nth_lt h)]
      exact h
    · intro l c p h hv
      by_cases hlt : l < H.length
      · rw [nth_append_lt hlt] at h
        exact Or.inl h
      · have h1 := nth_lt h
        simp only [List.length_append, List.length_cons, List.length_nil] at h1
        have hl : l = H.length := by omega
        rw [hl, nth_append_len] at h
        have h2 : (⟨0, some q⟩ : Cell) = c := Option.some.inj h
        rw [← h2] at hv
        have h3 : some q = some p := hv
        exact Or.inr (Option.some.inj h3).symm
  | some l =>
    obtain ⟨c, hc, hcv⟩ := hp H l hpk
    simp only [alloc, hpk, allocAt, hc]
    refine ⟨heapLe_fill q hc hcv, ?_, ⟨_, nth_setNth_eq (nth_lt hc), rfl⟩, ?_⟩
    · intro l0 c0 p h0 hp0
      by_cases heq : l0 = l
      · rw [heq] at h0
        have h1 : c0 = c := Option.some.inj (h0.symm.trans hc)
        rw [h1, hcv] at hp0
        cases hp0
      · rw [nth_setNth_ne heq]
        exact h0
    · intro l0 c0 p h0 hp0
      by_cases heq : l0 = l
      · rw [heq, nth_setNth_eq (nth_lt hc)] at h0
        have h2 : (⟨c.gen, some q⟩ : Cell) = c0 := Option.some.inj h0
        rw [← h2] at hp0
        have h3 : some q = some p := hp0
        exact Or.inr (Option.some.inj h3).symm
      · rw [nth_setNth_ne heq] at h0
        exact Or.inl h0

/-! ## 値の補題 -/

/-- 値の移し替え: 箱なら、その区画が生きたままで型の記録が同じなら型に合ったまま -/
theorem val_transport {H H' : Heap} {t : Ty} {v : Val} (hle : HeapLe H H')
    (hs : ∀ l : Nat, v = .box l → Keep H H' l) (hv : ValOk H t v) : ValOk H' t v := by
  cases v with
  | box l =>
    cases t with
    | box t =>
      have hv' : ∃ (c : Cell) (o : Nat) (w : Val), nth H l = some c ∧ c.val = some (o, t, w) := hv
      obtain ⟨c, o, w, hc, hcv⟩ := hv'
      obtain ⟨c', w', hc', hcv'⟩ := hs l rfl c o t w hc hcv
      show ∃ (c : Cell) (o : Nat) (w : Val), nth H' l = some c ∧ c.val = some (o, t, w)
      exact ⟨c', o, w', hc', hcv'⟩
    | int => exact False.elim hv
    | bool => exact False.elim hv
    | ref _ _ => exact False.elim hv
  | int _ =>
    cases t with
    | int => exact hv
    | bool => exact False.elim hv
    | box _ => exact False.elim hv
    | ref _ _ => exact False.elim hv
  | bool _ =>
    cases t with
    | bool => exact hv
    | int => exact False.elim hv
    | box _ => exact False.elim hv
    | ref _ _ => exact False.elim hv
  | ref l g =>
    cases t with
    | ref _ t =>
      have hv' : ∃ c : Cell, nth H l = some c ∧ g ≤ c.gen ∧
          (g = c.gen → ∃ (o : Nat) (w : Val), c.val = some (o, t, w)) := hv
      obtain ⟨c, hc, hg, hlive⟩ := hv'
      obtain ⟨c', hc', hgen, hval⟩ := hle l c hc
      show ∃ c : Cell, nth H' l = some c ∧ g ≤ c.gen ∧
          (g = c.gen → ∃ (o : Nat) (w : Val), c.val = some (o, t, w))
      refine ⟨c', hc', Nat.le_trans hg hgen, ?_⟩
      intro heq
      have h1 : g = c.gen := by omega
      have h2 : c.gen = c'.gen := by omega
      obtain ⟨o, w, hw⟩ := hlive h1
      exact hval h2 o t w hw
    | int => exact False.elim hv
    | bool => exact False.elim hv
    | box _ => exact False.elim hv

/-- ref の値は、ヒープが進めば型に合ったまま -/
theorem ref_mono {H H' : Heap} {m : Bool} {t : Ty} {v : Val} (hle : HeapLe H H')
    (hv : ValOk H (.ref m t) v) : ValOk H' (.ref m t) v := by
  refine val_transport hle ?_ hv
  intro l hl
  subst hl
  exact False.elim hv

theorem owns_transport {H H' : Heap} {j : Nat} {v : Val}
    (hs : ∀ l : Nat, v = .box l → Keep H H' l) (ho : Owns H j v) : Owns H' j v := by
  cases v with
  | box l =>
    have ho' : ∃ (c : Cell) (t : Ty) (w : Val), nth H l = some c ∧ c.val = some (j, t, w) := ho
    obtain ⟨c, t, w, hc, hcv⟩ := ho'
    obtain ⟨c', w', hc', hcv'⟩ := hs l rfl c j t w hc hcv
    show ∃ (c : Cell) (t : Ty) (w : Val), nth H' l = some c ∧ c.val = some (j, t, w)
    exact ⟨c', t, w', hc', hcv'⟩
  | int _ => exact ho
  | bool _ => exact ho
  | ref _ _ => exact ho

theorem loose_transport {H H' : Heap} {Γ : Ctx} {v : Val}
    (hs : ∀ l : Nat, v = .box l → Keep H H' l) (hl : Loose H Γ v) : Loose H' Γ v := by
  cases v with
  | box l =>
    have hl' : ∃ (c : Cell) (o : Nat) (t : Ty) (w : Val), nth H l = some c ∧
        c.val = some (o, t, w) ∧ ∀ t' : Ty, nth Γ o ≠ some (t', true) := hl
    obtain ⟨c, o, t, w, hc, hcv, hnl⟩ := hl'
    obtain ⟨c', w', hc', hcv'⟩ := hs l rfl c o t w hc hcv
    show ∃ (c : Cell) (o : Nat) (t : Ty) (w : Val), nth H' l = some c ∧
        c.val = some (o, t, w) ∧ ∀ t' : Ty, nth Γ o ≠ some (t', true)
    exact ⟨c', o, t, w', hc', hcv', hnl⟩
  | int _ => exact hl
  | bool _ => exact hl
  | ref _ _ => exact hl

/-- 箱を含まない型の値は箱でない -/
theorem loose_plain {H : Heap} {Γ : Ctx} {t : Ty} {v : Val} (hp : Plain t) (hv : ValOk H t v) :
    Loose H Γ v := by
  cases v with
  | box _ =>
    cases t with
    | box _ => exact False.elim hp
    | int => exact False.elim hv
    | bool => exact False.elim hv
    | ref _ _ => exact False.elim hv
  | int _ => exact True.intro
  | bool _ => exact True.intro
  | ref _ _ => exact True.intro

theorem loose_pop {H : Heap} {Γ : Ctx} {p : Ty × Bool} {v : Val} (hl : Loose H (Γ ++ [p]) v) :
    Loose H Γ v := by
  cases v with
  | box l =>
    have hl' : ∃ (c : Cell) (o : Nat) (t : Ty) (w : Val), nth H l = some c ∧
        c.val = some (o, t, w) ∧ ∀ t' : Ty, nth (Γ ++ [p]) o ≠ some (t', true) := hl
    obtain ⟨c, o, t, w, hc, hcv, hnl⟩ := hl'
    show ∃ (c : Cell) (o : Nat) (t : Ty) (w : Val), nth H l = some c ∧
        c.val = some (o, t, w) ∧ ∀ t' : Ty, nth Γ o ≠ some (t', true)
    refine ⟨c, o, t, w, hc, hcv, ?_⟩
    intro t' ht
    have hlt := nth_lt ht
    exact hnl t' (by rw [nth_append_lt hlt]; exact ht)
  | int _ => exact hl
  | bool _ => exact hl
  | ref _ _ => exact hl

/-! ## 環境の補題 -/

/-- 生きた変数の持つ区画が生きたままで持ち主・型の記録が同じなら、環境は文脈に合ったまま -/
theorem agree_transport {env : Env} {H H' : Heap} {Γ : Ctx} (ha : Agree env H Γ) (hle : HeapLe H H')
    (hkeep : ∀ (j l : Nat) (c : Cell) (t : Ty) (w : Val), nth H l = some c → c.val = some (j, t, w) →
      (∃ t' : Ty, nth Γ j = some (t', true)) → Keep H H' l)
    (hok : HeapOk H') : Agree env H' Γ := by
  constructor
  · exact ha.len
  · intro j t hj
    obtain ⟨v, he, hv, ho⟩ := ha.live j t hj
    have hs : ∀ l : Nat, v = .box l → Keep H H' l := by
      intro l hvl
      subst hvl
      have ho' : ∃ (c : Cell) (t' : Ty) (w : Val), nth H l = some c ∧ c.val = some (j, t', w) := ho
      obtain ⟨c, t', w, hc, hcv⟩ := ho'
      exact hkeep j l c t' w hc hcv ⟨t, hj⟩
    exact ⟨v, he, val_transport hle hs hv, owns_transport hs ho⟩
  · exact ha.dead
  · exact hok

theorem agree_kill {env : Env} {H : Heap} {Γ : Ctx} {i : Nat} {t : Ty} (ha : Agree env H Γ)
    (hi : nth Γ i = some (.box t, true)) :
    Agree (setNth env i none) H (setNth Γ i (.box t, false)) := by
  constructor
  · rw [length_setNth, length_setNth]
    exact ha.len
  · intro j t' hj
    by_cases heq : j = i
    · rw [heq, nth_setNth_eq (nth_lt hi)] at hj
      have hb : false = true := congrArg Prod.snd (Option.some.inj hj)
      cases hb
    · rw [nth_setNth_ne heq] at hj
      obtain ⟨v, he, hok, ho⟩ := ha.live j t' hj
      refine ⟨v, ?_, hok, ho⟩
      rw [nth_setNth_ne heq]
      exact he
  · intro j t' hj
    by_cases heq : j = i
    · rw [heq]
      have hlt : i < env.length := by
        rw [ha.len]
        exact nth_lt hi
      exact nth_setNth_eq hlt
    · rw [nth_setNth_ne heq] at hj
      rw [nth_setNth_ne heq]
      exact ha.dead j t' hj
  · exact ha.heap

theorem agree_pop {env : Env} {H : Heap} {Γ : Ctx} {x : Option Val} {p : Ty × Bool}
    (ha : Agree (env ++ [x]) H (Γ ++ [p])) (hlen : env.length = Γ.length) : Agree env H Γ := by
  constructor
  · exact hlen
  · intro j t hj
    have hlt := nth_lt hj
    obtain ⟨v, he, hok, ho⟩ := ha.live j t (by rw [nth_append_lt hlt]; exact hj)
    rw [nth_append_lt (by omega : j < env.length)] at he
    exact ⟨v, he, hok, ho⟩
  · intro j t hj
    have hlt := nth_lt hj
    have he := ha.dead j t (by rw [nth_append_lt hlt]; exact hj)
    rw [nth_append_lt (by omega : j < env.length)] at he
    exact he
  · exact ha.heap

theorem claim_valOk {H : Heap} {s : Ty} {v : Val} {n : Nat} (hv : ValOk H s v) :
    ValOk (claim H v n) s v := by
  cases v with
  | box l =>
    cases s with
    | box t =>
      have hv' : ∃ (c : Cell) (o : Nat) (w : Val), nth H l = some c ∧ c.val = some (o, t, w) := hv
      obtain ⟨c, _, w, hc, hcv⟩ := hv'
      show ∃ (c : Cell) (o : Nat) (w : Val), nth (claim H (.box l) n) l = some c ∧
        c.val = some (o, t, w)
      exact ⟨⟨c.gen, some (n, t, w)⟩, n, w, claim_self hc hcv, rfl⟩
    | int => exact False.elim hv
    | bool => exact False.elim hv
    | ref _ _ => exact False.elim hv
  | int _ => exact val_transport (claim_heapLe H _ n) (fun _ h => by cases h) hv
  | bool _ => exact val_transport (claim_heapLe H _ n) (fun _ h => by cases h) hv
  | ref _ _ => exact val_transport (claim_heapLe H _ n) (fun _ h => by cases h) hv

theorem claim_owns {H : Heap} {s : Ty} {v : Val} {n : Nat} (hv : ValOk H s v) :
    Owns (claim H v n) n v := by
  cases v with
  | box l =>
    cases s with
    | box t =>
      have hv' : ∃ (c : Cell) (o : Nat) (w : Val), nth H l = some c ∧ c.val = some (o, t, w) := hv
      obtain ⟨c, _, w, hc, hcv⟩ := hv'
      show ∃ (c : Cell) (t : Ty) (w : Val), nth (claim H (.box l) n) l = some c ∧
        c.val = some (n, t, w)
      exact ⟨⟨c.gen, some (n, t, w)⟩, t, w, claim_self hc hcv, rfl⟩
    | int => exact False.elim hv
    | bool => exact False.elim hv
    | ref _ _ => exact False.elim hv
  | int _ => exact True.intro
  | bool _ => exact True.intro
  | ref _ _ => exact True.intro

/-- let の始め: 値を末尾の変数に入れ、箱なら持ち主の記録をその変数にする -/
theorem agree_push_claim {env : Env} {H : Heap} {Γ : Ctx} {s : Ty} {v : Val} (ha : Agree env H Γ)
    (hv : ValOk H s v) (hl : Loose H Γ v) :
    Agree (env ++ [some v]) (claim H v env.length) (Γ ++ [(s, true)]) := by
  have hlen : env.length = Γ.length := ha.len
  have hkeep : ∀ (j l : Nat) (c : Cell) (t : Ty) (w : Val), nth H l = some c →
      c.val = some (j, t, w) → (∃ t' : Ty, nth Γ j = some (t', true)) →
      Keep H (claim H v env.length) l := by
    intro j l c t w hc hcv halive
    apply keep_of_eq
    apply claim_ne
    intro l' hvl heq
    subst hvl
    have hl' : ∃ (c : Cell) (o : Nat) (t : Ty) (w : Val), nth H l' = some c ∧
        c.val = some (o, t, w) ∧ ∀ t' : Ty, nth Γ o ≠ some (t', true) := hl
    obtain ⟨c1, o, _, _, h1, hv1, hnl⟩ := hl'
    rw [heq] at h1
    have h4 : c = c1 := Option.some.inj (hc.symm.trans h1)
    rw [← h4, hcv] at hv1
    obtain ⟨h3, _, _⟩ := triple_inj hv1
    obtain ⟨t', ht⟩ := halive
    rw [h3] at ht
    exact hnl t' ht
  have hok' : HeapOk (claim H v env.length) :=
    heapOk_step (claim_heapLe H v env.length) (claim_contents ha.heap v env.length)
  have hagH : Agree env (claim H v env.length) Γ :=
    agree_transport ha (claim_heapLe H v env.length) hkeep hok'
  constructor
  · simp only [List.length_append, List.length_cons, List.length_nil]
    omega
  · intro j t hj
    have hj' : j < Γ.length + 1 := by
      have h := nth_lt hj
      simp only [List.length_append, List.length_cons, List.length_nil] at h
      omega
    by_cases hlt : j < Γ.length
    · rw [nth_append_lt hlt] at hj
      obtain ⟨v0, he, hok, ho⟩ := hagH.live j t hj
      refine ⟨v0, ?_, hok, ho⟩
      rw [nth_append_lt (by omega : j < env.length)]
      exact he
    · have hjj : j = Γ.length := by omega
      rw [hjj, nth_append_len] at hj
      have htt : (s, true) = (t, true) := Option.some.inj hj
      have hst : s = t := congrArg Prod.fst htt
      rw [← hst, hjj, ← hlen]
      exact ⟨v, nth_append_len env (some v), claim_valOk hv, claim_owns hv⟩
  · intro j t hj
    by_cases hlt : j < Γ.length
    · rw [nth_append_lt hlt] at hj
      rw [nth_append_lt (by omega : j < env.length)]
      exact ha.dead j t hj
    · have h := nth_lt hj
      simp only [List.length_append, List.length_cons, List.length_nil] at h
      have hjj : j = Γ.length := by omega
      rw [hjj, nth_append_len] at hj
      have htt : (s, true) = (t, false) := Option.some.inj hj
      have hb : true = false := congrArg Prod.snd htt
      cases hb
  · exact hok'

/-- let の終わり: 変数が生きた箱を持っていれば区画を解放する。外の変数の箱と、結果の箱は別の区画 -/
theorem let_end {env : Env} {H : Heap} {Γ : Ctx} {p : Ty × Bool} {x : Option Val} {t : Ty} {w : Val}
    (ha : Agree (env ++ [x]) H (Γ ++ [p])) (hlen : env.length = Γ.length) (hw : ValOk H t w)
    (hl : Loose H (Γ ++ [p]) w) : Safe t Γ H (.ok w env (dropSlot H (some x))) := by
  have hpop := agree_pop ha hlen
  have hlp : Loose H Γ w := loose_pop hl
  obtain ⟨tp, b⟩ := p
  cases b with
  | false =>
    have hx := ha.dead Γ.length tp (nth_append_len Γ _)
    rw [← hlen, nth_append_len] at hx
    have hx' : x = none := Option.some.inj hx
    subst hx'
    exact And.intro hw (And.intro hpop (And.intro hlp (heapLe_refl H)))
  | true =>
    obtain ⟨v', hx, _, ho⟩ := ha.live Γ.length tp (nth_append_len Γ _)
    rw [← hlen, nth_append_len] at hx
    have hx' : x = some v' := Option.some.inj hx
    subst hx'
    cases v' with
    | box l =>
      have ho' : ∃ (c : Cell) (t0 : Ty) (w0 : Val), nth H l = some c ∧
          c.val = some (Γ.length, t0, w0) := ho
      obtain ⟨c, _, _, hc, hcv⟩ := ho'
      show Safe t Γ H (.ok w env (release H l))
      have hle := release_heapLe H l
      have hother : ∀ (l1 : Nat) (c1 : Cell) (o : Nat) (t1 : Ty) (w1 : Val), nth H l1 = some c1 →
          c1.val = some (o, t1, w1) → o ≠ Γ.length → nth (release H l) l1 = nth H l1 := by
        intro l1 c1 o t1 w1 h1 hv1 hne
        apply release_ne
        intro heq
        rw [heq] at h1
        have h4 : c = c1 := Option.some.inj (hc.symm.trans h1)
        rw [← h4, hcv] at hv1
        exact hne (triple_inj hv1).1.symm
      have hok' : HeapOk (release H l) := heapOk_step hle (release_contents ha.heap l)
      have hag' : Agree env (release H l) Γ := by
        refine agree_transport hpop hle ?_ hok'
        intro j l1 c1 t1 w1 h1 hv1 halive
        apply keep_of_eq
        apply hother l1 c1 j t1 w1 h1 hv1
        obtain ⟨t2, ht2⟩ := halive
        have := nth_lt ht2
        omega
      have hwl : ∀ lw : Nat, w = .box lw → Keep H (release H l) lw := by
        intro lw hwb
        subst hwb
        have hl' : ∃ (c : Cell) (o : Nat) (t : Ty) (w : Val), nth H lw = some c ∧
            c.val = some (o, t, w) ∧ ∀ t' : Ty, nth (Γ ++ [(tp, true)]) o ≠ some (t', true) := hl
        obtain ⟨c1, o, t1, w1, h1, hv1, hnl⟩ := hl'
        apply keep_of_eq
        refine hother lw c1 o t1 w1 h1 hv1 ?_
        intro heq
        rw [heq] at hnl
        exact hnl tp (nth_append_len Γ _)
      exact And.intro (val_transport hle hwl hw)
        (And.intro hag' (And.intro (loose_transport hwl hlp) hle))
    | int _ => exact And.intro hw (And.intro hpop (And.intro hlp (heapLe_refl H)))
    | bool _ => exact And.intro hw (And.intro hpop (And.intro hlp (heapLe_refl H)))
    | ref _ _ => exact And.intro hw (And.intro hpop (And.intro hlp (heapLe_refl H)))

/-! ## 一つずつの操作の安全性 -/

/-- 結果の基準のヒープを、それより前のヒープに替える -/
theorem safe_base {t : Ty} {Γ : Ctx} {H0 H : Heap} {r : Res} (h0 : HeapLe H0 H) (hr : Safe t Γ H r) :
    Safe t Γ H0 r := by
  cases r with
  | ok _ _ _ =>
    exact And.intro hr.1 (And.intro hr.2.1 (And.intro hr.2.2.1 (heapLe_trans h0 hr.2.2.2)))
  | err => exact True.intro
  | fuel => exact True.intro
  | stuck => exact False.elim hr

theorem bind_safe {s t : Ty} {Γ1 Γ2 : Ctx} {H0 : Heap} {r : Res} {f : Val → Env → Heap → Res}
    (hr : Safe s Γ1 H0 r)
    (hf : ∀ (v : Val) (env : Env) (H : Heap), ValOk H s v → Agree env H Γ1 → Loose H Γ1 v →
      HeapLe H0 H → Safe t Γ2 H (f v env H)) :
    Safe t Γ2 H0 (r.bind f) := by
  cases r with
  | ok v env H => exact safe_base hr.2.2.2 (hf v env H hr.1 hr.2.1 hr.2.2.1 hr.2.2.2)
  | err => exact True.intro
  | fuel => exact True.intro
  | stuck => exact False.elim hr

theorem checked_safe {env : Env} {H : Heap} {Γ : Ctx} (z : Int) (ha : Agree env H Γ) :
    Safe .int Γ H (checked z env H) := by
  unfold checked
  split
  · exact And.intro True.intro (And.intro ha (And.intro True.intro (heapLe_refl H)))
  · exact True.intro

theorem addOp_safe {env : Env} {H Ha Hb : Heap} {Γ : Ctx} {x y : Val} (hx : ValOk Ha .int x)
    (hy : ValOk Hb .int y) (ha : Agree env H Γ) : Safe .int Γ H (addOp x y env H) := by
  cases x with
  | bool _ => exact False.elim hx
  | box _ => exact False.elim hx
  | ref _ _ => exact False.elim hx
  | int p =>
    cases y with
    | bool _ => exact False.elim hy
    | box _ => exact False.elim hy
    | ref _ _ => exact False.elim hy
    | int q => exact checked_safe (p + q) ha

theorem readVar_safe {env : Env} {H : Heap} {Γ : Ctx} {i : Nat} {t : Ty} (ha : Agree env H Γ)
    (hi : nth Γ i = some (t, true)) (hp : Plain t) : Safe t Γ H (readVar env H i) := by
  obtain ⟨v, he, hok, _⟩ := ha.live i t hi
  simp only [readVar, he]
  exact And.intro hok (And.intro ha (And.intro (loose_plain hp hok) (heapLe_refl H)))

theorem moveOut_safe {env : Env} {H : Heap} {Γ : Ctx} {i : Nat} {t : Ty} (ha : Agree env H Γ)
    (hi : nth Γ i = some (.box t, true)) :
    Safe (.box t) (setNth Γ i (.box t, false)) H (moveOut env H i) := by
  obtain ⟨v, he, hok, ho⟩ := ha.live i (.box t) hi
  cases v with
  | box l =>
    simp only [moveOut, he]
    have ho' : ∃ (c : Cell) (t0 : Ty) (w : Val), nth H l = some c ∧ c.val = some (i, t0, w) := ho
    obtain ⟨c, t0, w, hc, hcv⟩ := ho'
    refine And.intro hok (And.intro (agree_kill ha hi) (And.intro ?_ (heapLe_refl H)))
    show ∃ (c : Cell) (o : Nat) (t' : Ty) (w : Val), nth H l = some c ∧ c.val = some (o, t', w) ∧
      ∀ t'' : Ty, nth (setNth Γ i (.box t, false)) o ≠ some (t'', true)
    refine ⟨c, i, t0, w, hc, hcv, ?_⟩
    intro t'' ht
    rw [nth_setNth_eq (nth_lt hi)] at ht
    have hb : false = true := congrArg Prod.snd (Option.some.inj ht)
    cases hb
  | int _ => exact False.elim hok
  | bool _ => exact False.elim hok
  | ref _ _ => exact False.elim hok

theorem peekAt_safe {env : Env} {H : Heap} {Γ : Ctx} {i : Nat} {t : Ty} (ha : Agree env H Γ)
    (hi : nth Γ i = some (.box t, true)) : Safe t Γ H (peekAt env H i) := by
  obtain ⟨v, he, hok, _⟩ := ha.live i (.box t) hi
  cases v with
  | box l =>
    have hv' : ∃ (c : Cell) (o : Nat) (w : Val), nth H l = some c ∧ c.val = some (o, t, w) := hok
    obtain ⟨c, o, w, hc, hcv⟩ := hv'
    have hw := ha.heap l c o t w hc hcv
    simp only [peekAt, he, contents, hc, hcv]
    exact And.intro hw.2 (And.intro ha (And.intro (loose_plain hw.1 hw.2) (heapLe_refl H)))
  | int _ => exact False.elim hok
  | bool _ => exact False.elim hok
  | ref _ _ => exact False.elim hok

theorem addrOf_safe {env : Env} {H : Heap} {Γ : Ctx} {i : Nat} {m : Bool} {t : Ty}
    (ha : Agree env H Γ) (hi : nth Γ i = some (.box t, true)) :
    Safe (.ref m t) Γ H (addrOf env H i) := by
  obtain ⟨v, he, hok, _⟩ := ha.live i (.box t) hi
  cases v with
  | box l =>
    have hv' : ∃ (c : Cell) (o : Nat) (w : Val), nth H l = some c ∧ c.val = some (o, t, w) := hok
    obtain ⟨c, o, w, hc, hcv⟩ := hv'
    simp only [addrOf, he, hc]
    refine And.intro ?_ (And.intro ha (And.intro True.intro (heapLe_refl H)))
    show ∃ c' : Cell, nth H l = some c' ∧ c.gen ≤ c'.gen ∧
      (c.gen = c'.gen → ∃ (o' : Nat) (w' : Val), c'.val = some (o', t, w'))
    exact ⟨c, hc, Nat.le_refl _, fun _ => ⟨o, w, hcv⟩⟩
  | int _ => exact False.elim hok
  | bool _ => exact False.elim hok
  | ref _ _ => exact False.elim hok

theorem derefOp_safe {env : Env} {H : Heap} {Γ : Ctx} {m : Bool} {t : Ty} {v : Val}
    (hv : ValOk H (.ref m t) v) (ha : Agree env H Γ) : Safe t Γ H (derefOp v env H) := by
  cases v with
  | ref l g =>
    have hv' : ∃ c : Cell, nth H l = some c ∧ g ≤ c.gen ∧
        (g = c.gen → ∃ (o : Nat) (w : Val), c.val = some (o, t, w)) := hv
    obtain ⟨c, hc, _, hlive⟩ := hv'
    simp only [derefOp, hc]
    by_cases heq : c.gen = g
    · rw [if_pos heq]
      obtain ⟨o, w, hw⟩ := hlive heq.symm
      have hwk := ha.heap l c o t w hc hw
      simp only [contents, hc, hw]
      exact And.intro hwk.2 (And.intro ha (And.intro (loose_plain hwk.1 hwk.2) (heapLe_refl H)))
    · rw [if_neg heq]
      exact True.intro
  | int _ => exact False.elim hv
  | bool _ => exact False.elim hv
  | box _ => exact False.elim hv

/-- ref を通した書き込み: 世代が合えば、同じ型の値を書き、持ち主・型の記録は変えない -/
theorem storeOp_safe {env : Env} {H : Heap} {Γ : Ctx} {t : Ty} {v w : Val}
    (hv : ValOk H (.ref true t) v) (hw : ValOk H t w) (ha : Agree env H Γ) :
    Safe .int Γ H (storeOp v w env H) := by
  cases v with
  | ref l g =>
    have hv' : ∃ c : Cell, nth H l = some c ∧ g ≤ c.gen ∧
        (g = c.gen → ∃ (o : Nat) (w : Val), c.val = some (o, t, w)) := hv
    obtain ⟨c, hc, _, hlive⟩ := hv'
    simp only [storeOp, hc]
    by_cases heq : c.gen = g
    · rw [if_pos heq]
      obtain ⟨o, w0, hcv⟩ := hlive heq.symm
      simp only [hcv]
      have hpl : Plain t := (ha.heap l c o t w0 hc hcv).1
      have hlt : l < H.length := nth_lt hc
      have hle : HeapLe H (setNth H l ⟨c.gen, some (o, t, w)⟩) := by
        refine heapLe_set (c' := ⟨c.gen, some (o, t, w)⟩) hc (Nat.le_refl _) ?_
        intro _ o1 t1 w1 h
        rw [hcv] at h
        obtain ⟨_, ht, _⟩ := triple_inj h
        rw [← ht]
        exact ⟨o, w, rfl⟩
      have hkeep : ∀ l1 : Nat, Keep H (setNth H l ⟨c.gen, some (o, t, w)⟩) l1 := by
        intro l1
        by_cases hl1 : l1 = l
        · rw [hl1]
          unfold Keep
          intro c1 o1 t1 w1 h1 hv1
          have h2 : c = c1 := Option.some.inj (hc.symm.trans h1)
          rw [← h2, hcv] at hv1
          obtain ⟨ho, ht, _⟩ := triple_inj hv1
          rw [← ho, ← ht]
          exact ⟨⟨c.gen, some (o, t, w)⟩, w, nth_setNth_eq hlt, rfl⟩
        · exact keep_of_eq (nth_setNth_ne hl1)
      have hok' : HeapOk (setNth H l ⟨c.gen, some (o, t, w)⟩) := by
        apply heapOk_step hle
        apply contents_set ha.heap hlt
        intro o1 t1 w1 h
        obtain ⟨_, ht, hw1⟩ := triple_inj h
        rw [← ht, ← hw1]
        exact ⟨hpl, hw⟩
      exact And.intro True.intro
        (And.intro (agree_transport ha hle (fun _ l1 _ _ _ _ _ _ => hkeep l1) hok')
          (And.intro True.intro hle))
    · rw [if_neg heq]
      exact True.intro
  | int _ => exact False.elim hv
  | bool _ => exact False.elim hv
  | box _ => exact False.elim hv

theorem mkBox_safe {pick : Heap → Option Nat} (hp : PickOk pick) {env : Env} {H : Heap} {Γ : Ctx}
    {t : Ty} {v : Val} (ha : Agree env H Γ) (hv : ValOk H t v) (hpl : Plain t) :
    Safe (.box t) Γ H
      (.ok (.box (alloc pick H (env.length, t, v)).1) env (alloc pick H (env.length, t, v)).2) := by
  obtain ⟨hle, hkeep, hnew, hsrc⟩ := alloc_spec hp H (env.length, t, v)
  obtain ⟨c', hc', hcv'⟩ := hnew
  have hlen : env.length = Γ.length := ha.len
  have hok' : HeapOk (alloc pick H (env.length, t, v)).2 := by
    apply heapOk_step hle
    unfold ContentsOk
    intro l c o t' w h1 h2
    cases hsrc l c (o, t', w) h1 h2 with
    | inl hold => exact ha.heap l c o t' w hold h2
    | inr heq =>
      have h3 : (some (o, t', w) : Option (Nat × Ty × Val)) = some (env.length, t, v) :=
        congrArg some heq
      obtain ⟨_, ht, hw⟩ := triple_inj h3
      rw [ht, hw]
      exact ⟨hpl, hv⟩
  refine And.intro ?_ (And.intro ?_ (And.intro ?_ hle))
  · show ∃ (c : Cell) (o : Nat) (w : Val),
      nth (alloc pick H (env.length, t, v)).2 (alloc pick H (env.length, t, v)).1 = some c ∧
      c.val = some (o, t, w)
    exact ⟨c', env.length, v, hc', hcv'⟩
  · refine agree_transport ha hle ?_ hok'
    intro j l c t' w hc hcv _
    unfold Keep
    intro c0 o0 t0 w0 h0 hv0
    exact ⟨c0, w0, hkeep l c0 _ h0 hv0, hv0⟩
  · show ∃ (c : Cell) (o : Nat) (t' : Ty) (w : Val),
      nth (alloc pick H (env.length, t, v)).2 (alloc pick H (env.length, t, v)).1 = some c ∧
      c.val = some (o, t', w) ∧ ∀ t'' : Ty, nth Γ o ≠ some (t'', true)
    refine ⟨c', env.length, t, v, hc', hcv', ?_⟩
    intro t'' ht
    rw [hlen] at ht
    have := nth_lt ht
    omega

/-! ## 主定理 -/

/-- 型安全性・所有権・世代の検査（書き込みつき）: 型が付く式を、文脈と合う環境・ヒープで、
条件を満たすどの割り当ての方針で評価しても、結果は Safe -/
theorem eval_safe {pick : Heap → Option Nat} (hp : PickOk pick) :
    ∀ (k : Nat) (env : Env) (H : Heap) (Γ : Ctx) (e : Expr) (t : Ty) (Γ' : Ctx),
      Agree env H Γ → HasTy Γ e t Γ' → Safe t Γ' H (eval pick k env H e) := by
  intro k
  induction k with
  | zero =>
    intro env H Γ e t Γ' _ _
    cases e <;> exact True.intro
  | succ k ih =>
    intro env H Γ e t Γ' hag h
    cases h with
    | int _ _ => exact And.intro True.intro (And.intro hag (And.intro True.intro (heapLe_refl H)))
    | bool _ _ => exact And.intro True.intro (And.intro hag (And.intro True.intro (heapLe_refl H)))
    | var _ _ _ hi hc =>
      simp only [eval]
      exact readVar_safe hag hi hc
    | move _ _ _ hi =>
      simp only [eval]
      exact moveOut_safe hag hi
    | peek _ _ _ hi =>
      simp only [eval]
      exact peekAt_safe hag hi
    | addr _ _ _ _ hi =>
      simp only [eval]
      exact addrOf_safe hag hi
    | deref _ _ e m _ he =>
      simp only [eval]
      apply bind_safe (ih env H Γ e (.ref m t) Γ' hag he)
      intro v env1 H1 hv ha1 _ _
      exact derefOp_safe hv ha1
    | store _ Γ1 _ a b t0 ha hb =>
      simp only [eval]
      apply bind_safe (ih env H Γ a (.ref true t0) Γ1 hag ha)
      intro v env1 H1 hv ha1 _ _
      apply bind_safe (ih env1 H1 Γ1 b t0 Γ' ha1 hb)
      intro w env2 H2 hw ha2 _ hle2
      exact storeOp_safe (ref_mono hle2 hv) hw ha2
    | mkBox _ _ e t0 he hc =>
      simp only [eval]
      apply bind_safe (ih env H Γ e t0 Γ' hag he)
      intro v env1 H1 hv ha1 _ _
      exact mkBox_safe hp ha1 hv hc
    | add _ Γ1 _ a b ha hb =>
      simp only [eval]
      apply bind_safe (ih env H Γ a .int Γ1 hag ha)
      intro x env1 H1 hx ha1 _ _
      apply bind_safe (ih env1 H1 Γ1 b .int Γ' ha1 hb)
      intro y env2 H2 hy ha2 _ _
      exact addOp_safe hx hy ha2
    | letE _ Γ1 _ x body s _ p hx hb =>
      simp only [eval]
      apply bind_safe (ih env H Γ x s Γ1 hag hx)
      intro v env1 H1 hv ha1 hl1 _
      apply safe_base (claim_heapLe H1 v env1.length)
      apply bind_safe (ih _ _ (Γ1 ++ [(s, true)]) body t (Γ' ++ [p]) (agree_push_claim ha1 hv hl1) hb)
      intro w env2 H2 hw ha2 hl2 _
      obtain ⟨env', x', he, hlen⟩ := split_last env2 Γ'.length (by
        have h2 := ha2.len
        simp only [List.length_append, List.length_cons, List.length_nil] at h2
        omega)
      subst he
      simp only [init_append, lastOpt_append]
      exact let_end ha2 hlen hw hl2

/-- 型が付く式は、どれだけの燃料でも、どの割り当ての方針でも行き詰まらない
（書き込みのあとも、移動済みの変数・解放済みの区画・使い回された区画の別の型の中身を使わない） -/
theorem no_stuck {pick : Heap → Option Nat} (hp : PickOk pick) (k : Nat) (env : Env) (H : Heap)
    (Γ : Ctx) (e : Expr) (t : Ty) (Γ' : Ctx) (hag : Agree env H Γ) (h : HasTy Γ e t Γ') :
    eval pick k env H e ≠ .stuck := by
  intro hs
  have hsafe := eval_safe hp k env H Γ e t Γ' hag h
  rw [hs] at hsafe
  exact hsafe

/-! ## 意味を持つことの確認 -/

theorem pick_none_ok : PickOk (fun _ => none) := by
  unfold PickOk
  intro _ l h
  have h' : (none : Option Nat) = some l := h
  cases h'

/-- 書き込める ref を通して書き、持ち主から読む:
`let b = box 1 in let r = &mut b in let _ = (*r = 5) in *b`。型が付く -/
theorem store_typed :
    HasTy [] (.letE (.mkBox .int (.int 1)) (.letE (.addr true 0)
      (.letE (.store (.var 1) (.int 5)) (.peek 0)))) .int [] :=
  HasTy.letE [] [] [] _ _ (.box .int) .int (.box .int, true)
    (HasTy.mkBox _ _ _ _ (HasTy.int _ _) True.intro)
    (HasTy.letE _ _ _ _ _ (.ref true .int) .int (.ref true .int, true)
      (HasTy.addr _ true 0 .int rfl)
      (HasTy.letE _ _ _ _ _ .int .int (.int, true)
        (HasTy.store _ _ _ _ _ .int (HasTy.var _ 1 (.ref true .int) rfl True.intro) (HasTy.int _ _))
        (HasTy.peek _ 0 .int rfl)))

/-- 書いた値（5）が持ち主から読め、let の終わりに区画が解放される（世代 1） -/
theorem store_then_peek :
    eval (fun _ => none) 6 [] [] (.letE (.mkBox .int (.int 1)) (.letE (.addr true 0)
      (.letE (.store (.var 1) (.int 5)) (.peek 0)))) = .ok (.int 5) [] [⟨1, none⟩] := rfl

/-- 箱への書き込める ref を let の外へ持ち出して書く: `*(let b = box 1 in &mut b) = 2`。型が付く -/
theorem stale_store_typed :
    HasTy [] (.store (.letE (.mkBox .int (.int 1)) (.addr true 0)) (.int 2)) .int [] :=
  HasTy.store [] [] [] _ _ .int
    (HasTy.letE [] [] [] _ _ (.box .int) (.ref true .int) (.box .int, true)
      (HasTy.mkBox _ _ _ _ (HasTy.int _ _) True.intro)
      (HasTy.addr _ true 0 .int rfl))
    (HasTy.int _ _)

/-- 解放された区画（世代 1）へ、持ち出した ref（世代 0）で書くと、実行時の誤りで止まる -/
theorem stale_store_err :
    eval (fun _ => none) 4 [] [] (.store (.letE (.mkBox .int (.int 1)) (.addr true 0)) (.int 2)) =
      .err := rfl

end JabberwockFormal.CoreGenBoxMut
