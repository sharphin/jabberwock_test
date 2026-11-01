import JabberwockFormal.CoreMutBorrow

set_option autoImplicit false
set_option warningAsError true

namespace JabberwockFormal.CoreGenFun

open JabberwockFormal.CoreMutBorrow (nth setNth lastOpt init nth_lt nth_append_lt nth_append_len
  length_setNth nth_setNth_eq nth_setNth_ne split_last init_append lastOpt_append)

/-!
持ち主の箱・世代つき ref・ref を通した書き込みの模型（`CoreGenBoxMut`）に、関数呼び出しを足した模型
（docs/core-type-safety.md）。

* 関数は表 `fns` に並べる。関数は引数を一つとり、本体は引数だけを変数に持つ文脈で型が付く（再帰してよい）。
* `call f e` : e の値を新しい枠の変数 0 に入れて本体を評価する。本体が終わったら、引数の変数が生きた箱を
  持っていれば区画を解放する（let の終わりと同じ）。
* 持ち主の記録は（枠の深さ, 変数の位置）の組にする。呼び出しごとに深さが 1 増える。
* 箱も ref も引数・戻り値にしてよい。ref は呼び出しを越えて持ち出せ、解放された区画の ref は読むと・書くと
  実行時の誤り（`err`）。

定理 `eval_safe`: 表のどの関数の本体にも型が付くとき、型が付く式を、文脈と合う環境・ヒープで、
条件を満たすどの割り当ての方針で評価しても stuck にならない。さらに、評価は呼び出し元の枠の変数が持つ区画を
壊さない（`Lower`）。

ref mut の重なりは禁じない（`CoreGenBoxMut` と同じ）。jwc・Rust 版の借用の検査、箱の中の箱、有限の世代、
関数の値（クロージャ）、複数の引数は扱わない。
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

/-- 持ち主の記録: 枠の深さと、その枠の変数の位置 -/
abbrev Own := Nat × Nat

/-- 区画: 世代と中身（持ち主の記録・型の記録・値）。中身が none なら空き -/
structure Cell where
  gen : Nat
  val : Option (Own × Ty × Val)

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
  | call (f : Nat) (e : Expr)

/-- 関数: 引数の型・戻り値の型・本体（本体の変数 0 が引数） -/
structure Fn where
  param : Ty
  ret : Ty
  body : Expr

/-! ## 型 -/

/-- 型付け `HasTy fns Γ e t Γ'`: Γ' は評価の後の文脈（移動した変数は死ぬ） -/
inductive HasTy (fns : List Fn) : Ctx → Expr → Ty → Ctx → Prop where
  | int (Γ : Ctx) (n : Int) : HasTy fns Γ (.int n) .int Γ
  | bool (Γ : Ctx) (b : Bool) : HasTy fns Γ (.bool b) .bool Γ
  | var (Γ : Ctx) (i : Nat) (t : Ty) (hi : nth Γ i = some (t, true)) (hc : Plain t) :
      HasTy fns Γ (.var i) t Γ
  | move (Γ : Ctx) (i : Nat) (t : Ty) (hi : nth Γ i = some (.box t, true)) :
      HasTy fns Γ (.move i) (.box t) (setNth Γ i (.box t, false))
  | peek (Γ : Ctx) (i : Nat) (t : Ty) (hi : nth Γ i = some (.box t, true)) :
      HasTy fns Γ (.peek i) t Γ
  | addr (Γ : Ctx) (m : Bool) (i : Nat) (t : Ty) (hi : nth Γ i = some (.box t, true)) :
      HasTy fns Γ (.addr m i) (.ref m t) Γ
  | deref (Γ Γ' : Ctx) (e : Expr) (m : Bool) (t : Ty) (he : HasTy fns Γ e (.ref m t) Γ') :
      HasTy fns Γ (.deref e) t Γ'
  | store (Γ Γ1 Γ2 : Ctx) (a b : Expr) (t : Ty) (ha : HasTy fns Γ a (.ref true t) Γ1)
      (hb : HasTy fns Γ1 b t Γ2) : HasTy fns Γ (.store a b) .int Γ2
  | mkBox (Γ Γ' : Ctx) (e : Expr) (t : Ty) (he : HasTy fns Γ e t Γ') (hc : Plain t) :
      HasTy fns Γ (.mkBox t e) (.box t) Γ'
  | add (Γ Γ1 Γ2 : Ctx) (a b : Expr) (ha : HasTy fns Γ a .int Γ1) (hb : HasTy fns Γ1 b .int Γ2) :
      HasTy fns Γ (.add a b) .int Γ2
  | letE (Γ Γ1 Γ2 : Ctx) (x body : Expr) (s t : Ty) (p : Ty × Bool) (hx : HasTy fns Γ x s Γ1)
      (hb : HasTy fns (Γ1 ++ [(s, true)]) body t (Γ2 ++ [p])) : HasTy fns Γ (.letE x body) t Γ2
  | call (Γ Γ' : Ctx) (f : Nat) (e : Expr) (fn : Fn) (hf : nth fns f = some fn)
      (he : HasTy fns Γ e fn.param Γ') : HasTy fns Γ (.call f e) fn.ret Γ'

/-- 表のどの関数の本体にも、引数だけの文脈で戻り値の型が付く -/
def FnsOk (fns : List Fn) : Prop :=
  ∀ (f : Nat) (fn : Fn), nth fns f = some fn →
    ∃ b : Bool, HasTy fns [(fn.param, true)] fn.body fn.ret [(fn.param, b)]

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

def allocAt (H : Heap) (l : Nat) (p : Own × Ty × Val) : Heap :=
  match nth H l with
  | some c => setNth H l ⟨c.gen, some p⟩
  | none => H

def alloc (pick : Heap → Option Nat) (H : Heap) (p : Own × Ty × Val) : Nat × Heap :=
  match pick H with
  | some l => (l, allocAt H l p)
  | none => (H.length, H ++ [⟨0, some p⟩])

def release (H : Heap) (l : Nat) : Heap :=
  match nth H l with
  | some c => setNth H l ⟨c.gen + 1, none⟩
  | none => H

/-- 箱の区画の持ち主の記録を o に書き換える（検証のための記録） -/
def claim (H : Heap) (v : Val) (o : Own) : Heap :=
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

/-- 評価: 燃料 k、枠の深さ d -/
def eval (pick : Heap → Option Nat) (fns : List Fn) : Nat → Nat → Env → Heap → Expr → Res
  | 0, _, _, _, _ => .fuel
  | _ + 1, _, env, H, .int n => .ok (.int n) env H
  | _ + 1, _, env, H, .bool b => .ok (.bool b) env H
  | _ + 1, _, env, H, .var i => readVar env H i
  | _ + 1, _, env, H, .move i => moveOut env H i
  | _ + 1, _, env, H, .peek i => peekAt env H i
  | _ + 1, _, env, H, .addr _ i => addrOf env H i
  | k + 1, d, env, H, .deref e => (eval pick fns k d env H e).bind derefOp
  | k + 1, d, env, H, .store a b =>
    (eval pick fns k d env H a).bind fun v env1 H1 =>
      (eval pick fns k d env1 H1 b).bind fun w env2 H2 => storeOp v w env2 H2
  | k + 1, d, env, H, .mkBox t e =>
    (eval pick fns k d env H e).bind fun v env1 H1 =>
      .ok (.box (alloc pick H1 ((d, env1.length), t, v)).1) env1
        (alloc pick H1 ((d, env1.length), t, v)).2
  | k + 1, d, env, H, .add a b =>
    (eval pick fns k d env H a).bind fun x env1 H1 =>
      (eval pick fns k d env1 H1 b).bind fun y env2 H2 => addOp x y env2 H2
  | k + 1, d, env, H, .letE x body =>
    (eval pick fns k d env H x).bind fun v env1 H1 =>
      (eval pick fns k d (env1 ++ [some v]) (claim H1 v (d, env1.length)) body).bind
        fun w env2 H2 => .ok w (init env2) (dropSlot H2 (lastOpt env2))
  | k + 1, d, env, H, .call f e =>
    (eval pick fns k d env H e).bind fun v env1 H1 =>
      match nth fns f with
      | some fn =>
        (eval pick fns k (d + 1) [some v] (claim H1 v (d + 1, 0)) fn.body).bind
          fun w env2 H2 => .ok w env1 (dropSlot H2 (lastOpt env2))
      | none => .stuck

/-- 割り当ての方針の条件: 空き区画だけを選ぶ -/
def PickOk (pick : Heap → Option Nat) : Prop :=
  ∀ (H : Heap) (l : Nat), pick H = some l → ∃ c : Cell, nth H l = some c ∧ c.val = none

/-! ## 不変条件 -/

/-- 値が型に合う。箱は型の記録が t の生きた区画を指す。ref は世代が区画の世代以下で、
等しければ区画が生きていて型の記録が t -/
def ValOk (H : Heap) : Ty → Val → Prop
  | .int, .int _ => True
  | .bool, .bool _ => True
  | .box t, .box l => ∃ (c : Cell) (o : Own) (w : Val), nth H l = some c ∧ c.val = some (o, t, w)
  | .ref _ t, .ref l g => ∃ c : Cell, nth H l = some c ∧ g ≤ c.gen ∧
      (g = c.gen → ∃ (o : Own) (w : Val), c.val = some (o, t, w))
  | _, _ => False

/-- H' の生きた区画の中身は、箱を含まない型で、H でその型に合う -/
def ContentsOk (H H' : Heap) : Prop :=
  ∀ (l : Nat) (c : Cell) (o : Own) (t : Ty) (w : Val), nth H' l = some c →
    c.val = some (o, t, w) → Plain t ∧ ValOk H t w

/-- ヒープの不変条件: 生きた区画の中身が型の記録に合う -/
def HeapOk (H : Heap) : Prop := ContentsOk H H

/-- 箱の値なら、その区画の持ち主の記録は j -/
def Owns (H : Heap) (j : Own) : Val → Prop
  | .box l => ∃ (c : Cell) (t : Ty) (w : Val), nth H l = some c ∧ c.val = some (j, t, w)
  | _ => True

/-- 深さ d の枠の評価の結果の箱は、持ち主の記録の深さが d 以上で、深さが d なら生きた変数でない -/
def Loose (H : Heap) (Γ : Ctx) (d : Nat) : Val → Prop
  | .box l => ∃ (c : Cell) (o : Own) (t : Ty) (w : Val), nth H l = some c ∧ c.val = some (o, t, w) ∧
      d ≤ o.1 ∧ (o.1 = d → ∀ t' : Ty, nth Γ o.2 ≠ some (t', true))
  | _ => True

/-- 箱の値なら、その区画の持ち主の記録は深さ d の枠の生きた変数でない -/
def FreeOf (H : Heap) (Γ : Ctx) (d : Nat) (v : Val) : Prop :=
  ∀ (l : Nat) (c : Cell) (o : Own) (t : Ty) (w : Val), v = .box l → nth H l = some c →
    c.val = some (o, t, w) → o.1 = d → ∀ t' : Ty, nth Γ o.2 ≠ some (t', true)

/-- 深さ d の枠の環境とヒープが文脈に合う: 生きた変数は値を持ち、型に合い、箱なら持ち主の記録が (d, 自分)。
死んだ変数は空。ヒープの中身は型の記録に合う -/
structure Agree (env : Env) (H : Heap) (Γ : Ctx) (d : Nat) : Prop where
  len : env.length = Γ.length
  live : ∀ (j : Nat) (t : Ty), nth Γ j = some (t, true) →
    ∃ v : Val, nth env j = some (some v) ∧ ValOk H t v ∧ Owns H (d, j) v
  dead : ∀ (j : Nat) (t : Ty), nth Γ j = some (t, false) → nth env j = some none
  heap : HeapOk H

/-- ヒープの進み方: 区画は消えず、世代は減らず、世代が同じなら生きていた区画は生きたままで型の記録は同じ
（値と持ち主の記録は変わってよい） -/
def HeapLe (H H' : Heap) : Prop :=
  ∀ (l : Nat) (c : Cell), nth H l = some c → ∃ c' : Cell, nth H' l = some c' ∧ c.gen ≤ c'.gen ∧
    (c.gen = c'.gen → ∀ (o : Own) (t : Ty) (w : Val), c.val = some (o, t, w) →
      ∃ (o' : Own) (w' : Val), c'.val = some (o', t, w'))

/-- 区画 l が生きていれば、生きたままで持ち主の記録と型の記録が同じ（値は変わってよい） -/
def Keep (H H' : Heap) (l : Nat) : Prop :=
  ∀ (c : Cell) (o : Own) (t : Ty) (w : Val), nth H l = some c → c.val = some (o, t, w) →
    ∃ (c' : Cell) (w' : Val), nth H' l = some c' ∧ c'.val = some (o, t, w')

/-- 深さ d より浅い枠（呼び出し元）の変数が持つ区画は、生きたままで持ち主・型の記録が同じ -/
def Lower (H H' : Heap) (d : Nat) : Prop :=
  ∀ (l : Nat) (c : Cell) (o : Own) (t : Ty) (w : Val), nth H l = some c → c.val = some (o, t, w) →
    o.1 < d → ∃ (c' : Cell) (w' : Val), nth H' l = some c' ∧ c'.val = some (o, t, w')

/-- 深さ d の枠の結果が安全: 値が型に合い、環境とヒープが文脈に合い、結果の箱は誰も持たず、
ヒープは H0 から進んだもので、呼び出し元の区画を壊さない -/
def Safe (t : Ty) (Γ : Ctx) (d : Nat) (H0 : Heap) : Res → Prop
  | .ok v env H => ValOk H t v ∧ Agree env H Γ d ∧ Loose H Γ d v ∧ HeapLe H0 H ∧ Lower H0 H d
  | .err => True
  | .fuel => True
  | .stuck => False

/-! ## ヒープの補題 -/

theorem triple_inj {o o' : Own} {t t' : Ty} {w w' : Val}
    (h : (some (o, t, w) : Option (Own × Ty × Val)) = some (o', t', w')) :
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
    (hv : c.gen = c'.gen → ∀ (o : Own) (t : Ty) (w : Val), c.val = some (o, t, w) →
      ∃ (o' : Own) (w' : Val), c'.val = some (o', t, w')) : HeapLe H (setNth H l c') := by
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

theorem heapLe_fill {H : Heap} {l : Nat} {c : Cell} (p : Own × Ty × Val) (hc : nth H l = some c)
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
          (g = c.gen → ∃ (o : Own) (w : Val), c.val = some (o, t, w)) := hv
      obtain ⟨c, hc, hg, hlive⟩ := hv'
      obtain ⟨c', hc', hgen, hval⟩ := hle l c hc
      show ∃ c : Cell, nth H' l = some c ∧ g ≤ c.gen ∧
          (g = c.gen → ∃ (o : Own) (w : Val), c.val = some (o, t, w))
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
    (hnew : ∀ (o : Own) (t : Ty) (w : Val), c'.val = some (o, t, w) → Plain t ∧ ValOk H t w) :
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
    have h0 : (none : Option (Own × Ty × Val)) = some (o, t, w) := h
    cases h0

theorem claim_heapLe (H : Heap) (v : Val) (o : Own) : HeapLe H (claim H v o) := by
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

theorem claim_ne {H : Heap} {v : Val} {l : Nat} {o : Own} (h : ∀ l' : Nat, v = .box l' → l' ≠ l) :
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

theorem claim_self {H : Heap} {l : Nat} {c : Cell} {o n : Own} {t : Ty} {w : Val}
    (hc : nth H l = some c) (hcv : c.val = some (o, t, w)) :
    nth (claim H (.box l) n) l = some ⟨c.gen, some (n, t, w)⟩ := by
  simp only [claim, hc, hcv]
  exact nth_setNth_eq (nth_lt hc)

theorem claim_contents {H : Heap} (hok : HeapOk H) (v : Val) (n : Own) :
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

theorem alloc_spec {pick : Heap → Option Nat} (hp : PickOk pick) (H : Heap) (q : Own × Ty × Val) :
    HeapLe H (alloc pick H q).2 ∧
    (∀ (l : Nat) (c : Cell) (p : Own × Ty × Val), nth H l = some c → c.val = some p →
      nth (alloc pick H q).2 l = some c) ∧
    (∃ c' : Cell, nth (alloc pick H q).2 (alloc pick H q).1 = some c' ∧ c'.val = some q) ∧
    (∀ (l : Nat) (c : Cell) (p : Own × Ty × Val), nth (alloc pick H q).2 l = some c →
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

/-! ## 区画を壊さないことの補題 -/

theorem keep_trans {H1 H2 H3 : Heap} {l : Nat} (h12 : Keep H1 H2 l) (h23 : Keep H2 H3 l) :
    Keep H1 H3 l := by
  unfold Keep
  intro c o t w hc hv
  obtain ⟨c2, w2, h2, hv2⟩ := h12 c o t w hc hv
  exact h23 c2 o t w2 h2 hv2

theorem lower_refl (H : Heap) (d : Nat) : Lower H H d := by
  unfold Lower
  intro _ c _ _ w hc hv _
  exact ⟨c, w, hc, hv⟩

theorem lower_trans {H1 H2 H3 : Heap} {d : Nat} (h12 : Lower H1 H2 d) (h23 : Lower H2 H3 d) :
    Lower H1 H3 d := by
  unfold Lower
  intro l c o t w hc hv hlt
  obtain ⟨c2, w2, h2, hv2⟩ := h12 l c o t w hc hv hlt
  exact h23 l c2 o t w2 h2 hv2 hlt

theorem lower_mono {H H' : Heap} {d d' : Nat} (hd : d' ≤ d) (h : Lower H H' d) : Lower H H' d' := by
  unfold Lower
  intro l c o t w hc hv hlt
  exact h l c o t w hc hv (by omega)

theorem lower_of_keep {H H' : Heap} {d : Nat} (h : ∀ l : Nat, Keep H H' l) : Lower H H' d := by
  unfold Lower
  intro l c o t w hc hv _
  exact h l c o t w hc hv

theorem lower_of_eq {H H' : Heap} {d : Nat}
    (h : ∀ (l : Nat) (c : Cell) (o : Own) (t : Ty) (w : Val), nth H l = some c →
      c.val = some (o, t, w) → o.1 < d → nth H' l = nth H l) : Lower H H' d := by
  unfold Lower
  intro l c o t w hc hv hlt
  refine ⟨c, w, ?_, hv⟩
  rw [h l c o t w hc hv hlt]
  exact hc

theorem lower_keep {H H' : Heap} {d l : Nat} (h : Lower H H' d)
    (hcond : ∀ (c : Cell) (o : Own) (t : Ty) (w : Val), nth H l = some c → c.val = some (o, t, w) →
      o.1 < d) : Keep H H' l := by
  unfold Keep
  intro c o t w hc hv
  exact h l c o t w hc hv (hcond c o t w hc hv)

/-! ## 値の補題 -/

/-- 値の移し替え: 箱なら、その区画が生きたままで型の記録が同じなら型に合ったまま -/
theorem val_transport {H H' : Heap} {t : Ty} {v : Val} (hle : HeapLe H H')
    (hs : ∀ l : Nat, v = .box l → Keep H H' l) (hv : ValOk H t v) : ValOk H' t v := by
  cases v with
  | box l =>
    cases t with
    | box t =>
      have hv' : ∃ (c : Cell) (o : Own) (w : Val), nth H l = some c ∧ c.val = some (o, t, w) := hv
      obtain ⟨c, o, w, hc, hcv⟩ := hv'
      obtain ⟨c', w', hc', hcv'⟩ := hs l rfl c o t w hc hcv
      show ∃ (c : Cell) (o : Own) (w : Val), nth H' l = some c ∧ c.val = some (o, t, w)
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
          (g = c.gen → ∃ (o : Own) (w : Val), c.val = some (o, t, w)) := hv
      obtain ⟨c, hc, hg, hlive⟩ := hv'
      obtain ⟨c', hc', hgen, hval⟩ := hle l c hc
      show ∃ c : Cell, nth H' l = some c ∧ g ≤ c.gen ∧
          (g = c.gen → ∃ (o : Own) (w : Val), c.val = some (o, t, w))
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

theorem owns_transport {H H' : Heap} {j : Own} {v : Val}
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

theorem loose_transport {H H' : Heap} {Γ : Ctx} {d : Nat} {v : Val}
    (hs : ∀ l : Nat, v = .box l → Keep H H' l) (hl : Loose H Γ d v) : Loose H' Γ d v := by
  cases v with
  | box l =>
    have hl' : ∃ (c : Cell) (o : Own) (t : Ty) (w : Val), nth H l = some c ∧
        c.val = some (o, t, w) ∧ d ≤ o.1 ∧ (o.1 = d → ∀ t' : Ty, nth Γ o.2 ≠ some (t', true)) := hl
    obtain ⟨c, o, t, w, hc, hcv, hd, hnl⟩ := hl'
    obtain ⟨c', w', hc', hcv'⟩ := hs l rfl c o t w hc hcv
    show ∃ (c : Cell) (o : Own) (t : Ty) (w : Val), nth H' l = some c ∧
        c.val = some (o, t, w) ∧ d ≤ o.1 ∧ (o.1 = d → ∀ t' : Ty, nth Γ o.2 ≠ some (t', true))
    exact ⟨c', o, t, w', hc', hcv', hd, hnl⟩
  | int _ => exact hl
  | bool _ => exact hl
  | ref _ _ => exact hl

/-- 箱を含まない型の値は箱でない -/
theorem loose_plain {H : Heap} {Γ : Ctx} {d : Nat} {t : Ty} {v : Val} (hp : Plain t)
    (hv : ValOk H t v) : Loose H Γ d v := by
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

theorem loose_pop {H : Heap} {Γ : Ctx} {d : Nat} {p : Ty × Bool} {v : Val}
    (hl : Loose H (Γ ++ [p]) d v) : Loose H Γ d v := by
  cases v with
  | box l =>
    have hl' : ∃ (c : Cell) (o : Own) (t : Ty) (w : Val), nth H l = some c ∧
        c.val = some (o, t, w) ∧ d ≤ o.1 ∧
        (o.1 = d → ∀ t' : Ty, nth (Γ ++ [p]) o.2 ≠ some (t', true)) := hl
    obtain ⟨c, o, t, w, hc, hcv, hd, hnl⟩ := hl'
    show ∃ (c : Cell) (o : Own) (t : Ty) (w : Val), nth H l = some c ∧
        c.val = some (o, t, w) ∧ d ≤ o.1 ∧ (o.1 = d → ∀ t' : Ty, nth Γ o.2 ≠ some (t', true))
    refine ⟨c, o, t, w, hc, hcv, hd, ?_⟩
    intro hod t' ht
    have hlt := nth_lt ht
    exact hnl hod t' (by rw [nth_append_lt hlt]; exact ht)
  | int _ => exact hl
  | bool _ => exact hl
  | ref _ _ => exact hl

/-- 呼び出し先（深さ d + 1）の結果の箱は、呼び出し元（深さ d）のどの変数も持っていない -/
theorem loose_up {H : Heap} {Γ Γ' : Ctx} {d : Nat} {v : Val} (hl : Loose H Γ (d + 1) v) :
    Loose H Γ' d v := by
  cases v with
  | box l =>
    have hl' : ∃ (c : Cell) (o : Own) (t : Ty) (w : Val), nth H l = some c ∧
        c.val = some (o, t, w) ∧ d + 1 ≤ o.1 ∧
        (o.1 = d + 1 → ∀ t' : Ty, nth Γ o.2 ≠ some (t', true)) := hl
    obtain ⟨c, o, t, w, hc, hcv, hd, _⟩ := hl'
    show ∃ (c : Cell) (o : Own) (t : Ty) (w : Val), nth H l = some c ∧
        c.val = some (o, t, w) ∧ d ≤ o.1 ∧ (o.1 = d → ∀ t' : Ty, nth Γ' o.2 ≠ some (t', true))
    refine ⟨c, o, t, w, hc, hcv, by omega, ?_⟩
    intro hod
    exact absurd hod (by omega)
  | int _ => exact hl
  | bool _ => exact hl
  | ref _ _ => exact hl

theorem loose_free {H : Heap} {Γ : Ctx} {d : Nat} {v : Val} (hl : Loose H Γ d v) : FreeOf H Γ d v := by
  unfold FreeOf
  intro l c o t w hvl hc hcv hod t' ht
  subst hvl
  have hl' : ∃ (c : Cell) (o : Own) (t : Ty) (w : Val), nth H l = some c ∧
      c.val = some (o, t, w) ∧ d ≤ o.1 ∧ (o.1 = d → ∀ t' : Ty, nth Γ o.2 ≠ some (t', true)) := hl
  obtain ⟨c1, o1, _, _, h1, hv1, _, hnl⟩ := hl'
  have h4 : c = c1 := Option.some.inj (hc.symm.trans h1)
  rw [← h4, hcv] at hv1
  obtain ⟨hoo, _, _⟩ := triple_inj hv1
  rw [← hoo] at hnl
  exact hnl hod t' ht

theorem free_nil {H : Heap} {d : Nat} {v : Val} : FreeOf H [] d v := by
  unfold FreeOf
  intro _ _ _ _ _ _ _ _ _ t' ht
  have h : (none : Option (Ty × Bool)) = some (t', true) := ht
  cases h

/-- 持ち主の記録を書き換える箱の区画は、深さ d の枠の生きた変数の区画と別 -/
theorem claim_keep_live {H : Heap} {Γ : Ctx} {d : Nat} {v : Val} {n : Own} {l : Nat} {c : Cell}
    {j : Nat} {t : Ty} {w : Val} (hfree : FreeOf H Γ d v) (hc : nth H l = some c)
    (hcv : c.val = some ((d, j), t, w)) (halive : ∃ t' : Ty, nth Γ j = some (t', true)) :
    nth (claim H v n) l = nth H l := by
  apply claim_ne
  intro l' hvl heq
  rw [← heq] at hc
  obtain ⟨t', ht'⟩ := halive
  exact hfree l' c (d, j) t w hvl hc hcv rfl t' ht'

/-- 持ち主の記録を書き換えるのは、深さ d 以上の箱の区画だけ -/
theorem claim_lower {H : Heap} {Γ : Ctx} {d : Nat} {v : Val} {n : Own} (hl : Loose H Γ d v) :
    Lower H (claim H v n) d := by
  apply lower_of_eq
  intro l c o _ _ hc hcv hlt
  apply claim_ne
  intro l' hvl heq
  subst hvl
  have hl' : ∃ (c : Cell) (o : Own) (t : Ty) (w : Val), nth H l' = some c ∧
      c.val = some (o, t, w) ∧ d ≤ o.1 ∧ (o.1 = d → ∀ t' : Ty, nth Γ o.2 ≠ some (t', true)) := hl
  obtain ⟨c1, o1, _, _, h1, hv1, hd1, _⟩ := hl'
  rw [heq] at h1
  have h4 : c = c1 := Option.some.inj (hc.symm.trans h1)
  rw [← h4, hcv] at hv1
  obtain ⟨hoo, _, _⟩ := triple_inj hv1
  rw [← hoo] at hd1
  omega

/-! ## 環境の補題 -/

/-- 生きた変数の持つ区画が生きたままで持ち主・型の記録が同じなら、環境は文脈に合ったまま -/
theorem agree_transport {env : Env} {H H' : Heap} {Γ : Ctx} {d : Nat} (ha : Agree env H Γ d)
    (hle : HeapLe H H')
    (hkeep : ∀ (j l : Nat) (c : Cell) (t : Ty) (w : Val), nth H l = some c →
      c.val = some ((d, j), t, w) → (∃ t' : Ty, nth Γ j = some (t', true)) → Keep H H' l)
    (hok : HeapOk H') : Agree env H' Γ d := by
  constructor
  · exact ha.len
  · intro j t hj
    obtain ⟨v, he, hv, ho⟩ := ha.live j t hj
    have hs : ∀ l : Nat, v = .box l → Keep H H' l := by
      intro l hvl
      subst hvl
      have ho' : ∃ (c : Cell) (t' : Ty) (w : Val), nth H l = some c ∧
          c.val = some ((d, j), t', w) := ho
      obtain ⟨c, t', w, hc, hcv⟩ := ho'
      exact hkeep j l c t' w hc hcv ⟨t, hj⟩
    exact ⟨v, he, val_transport hle hs hv, owns_transport hs ho⟩
  · exact ha.dead
  · exact hok

theorem agree_nil {H : Heap} {d : Nat} (hok : HeapOk H) : Agree [] H [] d := by
  constructor
  · rfl
  · intro _ t hj
    have h : (none : Option (Ty × Bool)) = some (t, true) := hj
    cases h
  · intro _ t hj
    have h : (none : Option (Ty × Bool)) = some (t, false) := hj
    cases h
  · exact hok

theorem agree_kill {env : Env} {H : Heap} {Γ : Ctx} {d i : Nat} {t : Ty} (ha : Agree env H Γ d)
    (hi : nth Γ i = some (.box t, true)) :
    Agree (setNth env i none) H (setNth Γ i (.box t, false)) d := by
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

theorem agree_pop {env : Env} {H : Heap} {Γ : Ctx} {d : Nat} {x : Option Val} {p : Ty × Bool}
    (ha : Agree (env ++ [x]) H (Γ ++ [p]) d) (hlen : env.length = Γ.length) : Agree env H Γ d := by
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

theorem claim_valOk {H : Heap} {s : Ty} {v : Val} {n : Own} (hv : ValOk H s v) :
    ValOk (claim H v n) s v := by
  cases v with
  | box l =>
    cases s with
    | box t =>
      have hv' : ∃ (c : Cell) (o : Own) (w : Val), nth H l = some c ∧ c.val = some (o, t, w) := hv
      obtain ⟨c, _, w, hc, hcv⟩ := hv'
      show ∃ (c : Cell) (o : Own) (w : Val), nth (claim H (.box l) n) l = some c ∧
        c.val = some (o, t, w)
      exact ⟨⟨c.gen, some (n, t, w)⟩, n, w, claim_self hc hcv, rfl⟩
    | int => exact False.elim hv
    | bool => exact False.elim hv
    | ref _ _ => exact False.elim hv
  | int _ => exact val_transport (claim_heapLe H _ n) (fun _ h => by cases h) hv
  | bool _ => exact val_transport (claim_heapLe H _ n) (fun _ h => by cases h) hv
  | ref _ _ => exact val_transport (claim_heapLe H _ n) (fun _ h => by cases h) hv

theorem claim_owns {H : Heap} {s : Ty} {v : Val} {n : Own} (hv : ValOk H s v) :
    Owns (claim H v n) n v := by
  cases v with
  | box l =>
    cases s with
    | box t =>
      have hv' : ∃ (c : Cell) (o : Own) (w : Val), nth H l = some c ∧ c.val = some (o, t, w) := hv
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

/-- let の始め・呼び出しの始め: 値を末尾の変数に入れ、箱なら持ち主の記録をその変数にする -/
theorem agree_push_claim {env : Env} {H : Heap} {Γ : Ctx} {d : Nat} {s : Ty} {v : Val}
    (ha : Agree env H Γ d) (hv : ValOk H s v) (hfree : FreeOf H Γ d v) :
    Agree (env ++ [some v]) (claim H v (d, env.length)) (Γ ++ [(s, true)]) d := by
  have hlen : env.length = Γ.length := ha.len
  have hok' : HeapOk (claim H v (d, env.length)) :=
    heapOk_step (claim_heapLe H v (d, env.length)) (claim_contents ha.heap v (d, env.length))
  have hagH : Agree env (claim H v (d, env.length)) Γ d :=
    agree_transport ha (claim_heapLe H v (d, env.length))
      (fun _ _ _ _ _ hc hcv halive => keep_of_eq (claim_keep_live hfree hc hcv halive)) hok'
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

/-- 変数の終わり（let・呼び出し）: 解放で変わるのは、持ち主の記録が (d, 末尾の位置) で末尾の変数が生きている区画だけ -/
theorem drop_spec {env : Env} {H : Heap} {Γ : Ctx} {p : Ty × Bool} {x : Option Val} {d : Nat}
    (ha : Agree (env ++ [x]) H (Γ ++ [p]) d) (hlen : env.length = Γ.length) :
    HeapLe H (dropSlot H (some x)) ∧ HeapOk (dropSlot H (some x)) ∧
    ∀ (l : Nat) (c : Cell) (o : Own) (t : Ty) (w : Val), nth H l = some c → c.val = some (o, t, w) →
      (o = (d, Γ.length) → p.2 = false) → nth (dropSlot H (some x)) l = nth H l := by
  obtain ⟨tp, b⟩ := p
  cases b with
  | false =>
    have hx := ha.dead Γ.length tp (nth_append_len Γ _)
    rw [← hlen, nth_append_len] at hx
    have hx' : x = none := Option.some.inj hx
    subst hx'
    exact ⟨heapLe_refl H, ha.heap, fun _ _ _ _ _ _ _ _ => rfl⟩
  | true =>
    obtain ⟨v', hx, _, ho⟩ := ha.live Γ.length tp (nth_append_len Γ _)
    rw [← hlen, nth_append_len] at hx
    have hx' : x = some v' := Option.some.inj hx
    subst hx'
    cases v' with
    | box l =>
      have ho' : ∃ (c : Cell) (t0 : Ty) (w0 : Val), nth H l = some c ∧
          c.val = some ((d, Γ.length), t0, w0) := ho
      obtain ⟨c, _, _, hc, hcv⟩ := ho'
      show HeapLe H (release H l) ∧ HeapOk (release H l) ∧
        ∀ (l1 : Nat) (c1 : Cell) (o : Own) (t : Ty) (w : Val), nth H l1 = some c1 →
          c1.val = some (o, t, w) → (o = (d, Γ.length) → (tp, true).2 = false) →
          nth (release H l) l1 = nth H l1
      refine ⟨release_heapLe H l, heapOk_step (release_heapLe H l) (release_contents ha.heap l), ?_⟩
      intro l1 c1 o t1 w1 h1 hv1 hne
      apply release_ne
      intro heq
      rw [heq] at h1
      have h4 : c = c1 := Option.some.inj (hc.symm.trans h1)
      rw [← h4, hcv] at hv1
      obtain ⟨hoo, _, _⟩ := triple_inj hv1
      have hb : true = false := hne hoo.symm
      cases hb
    | int _ => exact ⟨heapLe_refl H, ha.heap, fun _ _ _ _ _ _ _ _ => rfl⟩
    | bool _ => exact ⟨heapLe_refl H, ha.heap, fun _ _ _ _ _ _ _ _ => rfl⟩
    | ref _ _ => exact ⟨heapLe_refl H, ha.heap, fun _ _ _ _ _ _ _ _ => rfl⟩

/-- let の終わり: 変数が生きた箱を持っていれば区画を解放する。外の変数の箱と、結果の箱は別の区画 -/
theorem let_end {env : Env} {H : Heap} {Γ : Ctx} {p : Ty × Bool} {x : Option Val} {t : Ty} {w : Val}
    {d : Nat} (ha : Agree (env ++ [x]) H (Γ ++ [p]) d) (hlen : env.length = Γ.length)
    (hw : ValOk H t w) (hl : Loose H (Γ ++ [p]) d w) :
    Safe t Γ d H (.ok w env (dropSlot H (some x))) := by
  obtain ⟨hle, hok', hsame⟩ := drop_spec ha hlen
  have hpop := agree_pop ha hlen
  have hlp : Loose H Γ d w := loose_pop hl
  have hwl : ∀ lw : Nat, w = .box lw → Keep H (dropSlot H (some x)) lw := by
    intro lw hwb
    subst hwb
    have hl' : ∃ (c : Cell) (o : Own) (t : Ty) (w : Val), nth H lw = some c ∧
        c.val = some (o, t, w) ∧ d ≤ o.1 ∧
        (o.1 = d → ∀ t' : Ty, nth (Γ ++ [p]) o.2 ≠ some (t', true)) := hl
    obtain ⟨c1, o, t1, w1, h1, hv1, _, hnl⟩ := hl'
    apply keep_of_eq
    refine hsame lw c1 o t1 w1 h1 hv1 ?_
    intro heq
    subst heq
    obtain ⟨tp, b⟩ := p
    cases b with
    | false => rfl
    | true => exact False.elim (hnl rfl tp (nth_append_len Γ _))
  have hkeep : ∀ (j l : Nat) (c : Cell) (t : Ty) (w : Val), nth H l = some c →
      c.val = some ((d, j), t, w) → (∃ t' : Ty, nth Γ j = some (t', true)) →
      Keep H (dropSlot H (some x)) l := by
    intro j l c t1 w1 hc hcv halive
    apply keep_of_eq
    refine hsame l c (d, j) t1 w1 hc hcv ?_
    intro heq
    have hj : j = Γ.length := congrArg Prod.snd heq
    obtain ⟨t2, ht2⟩ := halive
    have := nth_lt ht2
    exact absurd hj (by omega)
  have hlow : Lower H (dropSlot H (some x)) d := by
    apply lower_of_eq
    intro l c o t1 w1 hc hcv hlt
    refine hsame l c o t1 w1 hc hcv ?_
    intro heq
    have h1 : o.1 = d := by rw [heq]
    exact absurd h1 (by omega)
  exact And.intro (val_transport hle hwl hw)
    (And.intro (agree_transport hpop hle hkeep hok')
      (And.intro (loose_transport hwl hlp) (And.intro hle hlow)))

/-! ## 一つずつの操作の安全性 -/

/-- 結果の基準のヒープを、それより前のヒープに替える -/
theorem safe_base {t : Ty} {Γ : Ctx} {d : Nat} {H0 H : Heap} {r : Res} (h0 : HeapLe H0 H)
    (hl0 : Lower H0 H d) (hr : Safe t Γ d H r) : Safe t Γ d H0 r := by
  cases r with
  | ok _ _ _ =>
    exact And.intro hr.1 (And.intro hr.2.1 (And.intro hr.2.2.1
      (And.intro (heapLe_trans h0 hr.2.2.2.1) (lower_trans hl0 hr.2.2.2.2))))
  | err => exact True.intro
  | fuel => exact True.intro
  | stuck => exact False.elim hr

theorem bind_safe {s t : Ty} {Γ1 Γ2 : Ctx} {d1 d2 : Nat} {H0 : Heap} {r : Res}
    {f : Val → Env → Heap → Res} (hd : d2 ≤ d1) (hr : Safe s Γ1 d1 H0 r)
    (hf : ∀ (v : Val) (env : Env) (H : Heap), ValOk H s v → Agree env H Γ1 d1 → Loose H Γ1 d1 v →
      HeapLe H0 H → Lower H0 H d1 → Safe t Γ2 d2 H (f v env H)) :
    Safe t Γ2 d2 H0 (r.bind f) := by
  cases r with
  | ok v env H =>
    exact safe_base hr.2.2.2.1 (lower_mono hd hr.2.2.2.2)
      (hf v env H hr.1 hr.2.1 hr.2.2.1 hr.2.2.2.1 hr.2.2.2.2)
  | err => exact True.intro
  | fuel => exact True.intro
  | stuck => exact False.elim hr

theorem checked_safe {env : Env} {H : Heap} {Γ : Ctx} {d : Nat} (z : Int) (ha : Agree env H Γ d) :
    Safe .int Γ d H (checked z env H) := by
  unfold checked
  split
  · exact And.intro True.intro (And.intro ha (And.intro True.intro
      (And.intro (heapLe_refl H) (lower_refl H d))))
  · exact True.intro

theorem addOp_safe {env : Env} {H Ha Hb : Heap} {Γ : Ctx} {d : Nat} {x y : Val}
    (hx : ValOk Ha .int x) (hy : ValOk Hb .int y) (ha : Agree env H Γ d) :
    Safe .int Γ d H (addOp x y env H) := by
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

theorem readVar_safe {env : Env} {H : Heap} {Γ : Ctx} {d i : Nat} {t : Ty} (ha : Agree env H Γ d)
    (hi : nth Γ i = some (t, true)) (hp : Plain t) : Safe t Γ d H (readVar env H i) := by
  obtain ⟨v, he, hok, _⟩ := ha.live i t hi
  simp only [readVar, he]
  exact And.intro hok (And.intro ha (And.intro (loose_plain hp hok)
    (And.intro (heapLe_refl H) (lower_refl H d))))

theorem moveOut_safe {env : Env} {H : Heap} {Γ : Ctx} {d i : Nat} {t : Ty} (ha : Agree env H Γ d)
    (hi : nth Γ i = some (.box t, true)) :
    Safe (.box t) (setNth Γ i (.box t, false)) d H (moveOut env H i) := by
  obtain ⟨v, he, hok, ho⟩ := ha.live i (.box t) hi
  cases v with
  | box l =>
    simp only [moveOut, he]
    have ho' : ∃ (c : Cell) (t0 : Ty) (w : Val), nth H l = some c ∧
        c.val = some ((d, i), t0, w) := ho
    obtain ⟨c, t0, w, hc, hcv⟩ := ho'
    refine And.intro hok (And.intro (agree_kill ha hi) (And.intro ?_
      (And.intro (heapLe_refl H) (lower_refl H d))))
    show ∃ (c : Cell) (o : Own) (t' : Ty) (w : Val), nth H l = some c ∧ c.val = some (o, t', w) ∧
      d ≤ o.1 ∧ (o.1 = d → ∀ t'' : Ty, nth (setNth Γ i (.box t, false)) o.2 ≠ some (t'', true))
    refine ⟨c, (d, i), t0, w, hc, hcv, Nat.le_refl _, ?_⟩
    intro _ t'' ht
    have ht' : nth (setNth Γ i (.box t, false)) i = some (t'', true) := ht
    rw [nth_setNth_eq (nth_lt hi)] at ht'
    have hb : false = true := congrArg Prod.snd (Option.some.inj ht')
    cases hb
  | int _ => exact False.elim hok
  | bool _ => exact False.elim hok
  | ref _ _ => exact False.elim hok

theorem peekAt_safe {env : Env} {H : Heap} {Γ : Ctx} {d i : Nat} {t : Ty} (ha : Agree env H Γ d)
    (hi : nth Γ i = some (.box t, true)) : Safe t Γ d H (peekAt env H i) := by
  obtain ⟨v, he, hok, _⟩ := ha.live i (.box t) hi
  cases v with
  | box l =>
    have hv' : ∃ (c : Cell) (o : Own) (w : Val), nth H l = some c ∧ c.val = some (o, t, w) := hok
    obtain ⟨c, o, w, hc, hcv⟩ := hv'
    have hw := ha.heap l c o t w hc hcv
    simp only [peekAt, he, contents, hc, hcv]
    exact And.intro hw.2 (And.intro ha (And.intro (loose_plain hw.1 hw.2)
      (And.intro (heapLe_refl H) (lower_refl H d))))
  | int _ => exact False.elim hok
  | bool _ => exact False.elim hok
  | ref _ _ => exact False.elim hok

theorem addrOf_safe {env : Env} {H : Heap} {Γ : Ctx} {d i : Nat} {m : Bool} {t : Ty}
    (ha : Agree env H Γ d) (hi : nth Γ i = some (.box t, true)) :
    Safe (.ref m t) Γ d H (addrOf env H i) := by
  obtain ⟨v, he, hok, _⟩ := ha.live i (.box t) hi
  cases v with
  | box l =>
    have hv' : ∃ (c : Cell) (o : Own) (w : Val), nth H l = some c ∧ c.val = some (o, t, w) := hok
    obtain ⟨c, o, w, hc, hcv⟩ := hv'
    simp only [addrOf, he, hc]
    refine And.intro ?_ (And.intro ha (And.intro True.intro
      (And.intro (heapLe_refl H) (lower_refl H d))))
    show ∃ c' : Cell, nth H l = some c' ∧ c.gen ≤ c'.gen ∧
      (c.gen = c'.gen → ∃ (o' : Own) (w' : Val), c'.val = some (o', t, w'))
    exact ⟨c, hc, Nat.le_refl _, fun _ => ⟨o, w, hcv⟩⟩
  | int _ => exact False.elim hok
  | bool _ => exact False.elim hok
  | ref _ _ => exact False.elim hok

theorem derefOp_safe {env : Env} {H : Heap} {Γ : Ctx} {d : Nat} {m : Bool} {t : Ty} {v : Val}
    (hv : ValOk H (.ref m t) v) (ha : Agree env H Γ d) : Safe t Γ d H (derefOp v env H) := by
  cases v with
  | ref l g =>
    have hv' : ∃ c : Cell, nth H l = some c ∧ g ≤ c.gen ∧
        (g = c.gen → ∃ (o : Own) (w : Val), c.val = some (o, t, w)) := hv
    obtain ⟨c, hc, _, hlive⟩ := hv'
    simp only [derefOp, hc]
    by_cases heq : c.gen = g
    · rw [if_pos heq]
      obtain ⟨o, w, hw⟩ := hlive heq.symm
      have hwk := ha.heap l c o t w hc hw
      simp only [contents, hc, hw]
      exact And.intro hwk.2 (And.intro ha (And.intro (loose_plain hwk.1 hwk.2)
        (And.intro (heapLe_refl H) (lower_refl H d))))
    · rw [if_neg heq]
      exact True.intro
  | int _ => exact False.elim hv
  | bool _ => exact False.elim hv
  | box _ => exact False.elim hv

/-- ref を通した書き込み: 世代が合えば、同じ型の値を書き、持ち主・型の記録は変えない -/
theorem storeOp_safe {env : Env} {H : Heap} {Γ : Ctx} {d : Nat} {t : Ty} {v w : Val}
    (hv : ValOk H (.ref true t) v) (hw : ValOk H t w) (ha : Agree env H Γ d) :
    Safe .int Γ d H (storeOp v w env H) := by
  cases v with
  | ref l g =>
    have hv' : ∃ c : Cell, nth H l = some c ∧ g ≤ c.gen ∧
        (g = c.gen → ∃ (o : Own) (w : Val), c.val = some (o, t, w)) := hv
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
          (And.intro True.intro (And.intro hle (lower_of_keep hkeep))))
    · rw [if_neg heq]
      exact True.intro
  | int _ => exact False.elim hv
  | bool _ => exact False.elim hv
  | box _ => exact False.elim hv

theorem mkBox_safe {pick : Heap → Option Nat} (hp : PickOk pick) {env : Env} {H : Heap} {Γ : Ctx}
    {d : Nat} {t : Ty} {v : Val} (ha : Agree env H Γ d) (hv : ValOk H t v) (hpl : Plain t) :
    Safe (.box t) Γ d H
      (.ok (.box (alloc pick H ((d, env.length), t, v)).1) env
        (alloc pick H ((d, env.length), t, v)).2) := by
  obtain ⟨hle, hkeep, hnew, hsrc⟩ := alloc_spec hp H ((d, env.length), t, v)
  obtain ⟨c', hc', hcv'⟩ := hnew
  have hlen : env.length = Γ.length := ha.len
  have hok' : HeapOk (alloc pick H ((d, env.length), t, v)).2 := by
    apply heapOk_step hle
    unfold ContentsOk
    intro l c o t' w h1 h2
    cases hsrc l c (o, t', w) h1 h2 with
    | inl hold => exact ha.heap l c o t' w hold h2
    | inr heq =>
      have h3 : (some (o, t', w) : Option (Own × Ty × Val)) = some ((d, env.length), t, v) :=
        congrArg some heq
      obtain ⟨_, ht, hw⟩ := triple_inj h3
      rw [ht, hw]
      exact ⟨hpl, hv⟩
  have hk : ∀ l : Nat, Keep H (alloc pick H ((d, env.length), t, v)).2 l := by
    intro l
    unfold Keep
    intro c0 _ _ w0 h0 hv0
    exact ⟨c0, w0, hkeep l c0 _ h0 hv0, hv0⟩
  refine And.intro ?_ (And.intro ?_ (And.intro ?_ (And.intro hle (lower_of_keep hk))))
  · show ∃ (c : Cell) (o : Own) (w : Val),
      nth (alloc pick H ((d, env.length), t, v)).2 (alloc pick H ((d, env.length), t, v)).1 =
        some c ∧ c.val = some (o, t, w)
    exact ⟨c', (d, env.length), v, hc', hcv'⟩
  · exact agree_transport ha hle (fun _ l _ _ _ _ _ _ => hk l) hok'
  · show ∃ (c : Cell) (o : Own) (t' : Ty) (w : Val),
      nth (alloc pick H ((d, env.length), t, v)).2 (alloc pick H ((d, env.length), t, v)).1 =
        some c ∧ c.val = some (o, t', w) ∧ d ≤ o.1 ∧
        (o.1 = d → ∀ t'' : Ty, nth Γ o.2 ≠ some (t'', true))
    refine ⟨c', (d, env.length), t, v, hc', hcv', Nat.le_refl _, ?_⟩
    intro _ t'' ht
    have ht' : nth Γ env.length = some (t'', true) := ht
    rw [hlen] at ht'
    have := nth_lt ht'
    omega

/-! ## 主定理 -/

/-- 型安全性・所有権・世代の検査（書き込み・関数呼び出しつき）: 表のどの関数の本体にも型が付くとき、
型が付く式を、文脈と合う環境・ヒープで、条件を満たすどの割り当ての方針で評価しても、結果は Safe -/
theorem eval_safe {pick : Heap → Option Nat} (hp : PickOk pick) {fns : List Fn} (hfns : FnsOk fns) :
    ∀ (k d : Nat) (env : Env) (H : Heap) (Γ : Ctx) (e : Expr) (t : Ty) (Γ' : Ctx),
      Agree env H Γ d → HasTy fns Γ e t Γ' → Safe t Γ' d H (eval pick fns k d env H e) := by
  intro k
  induction k with
  | zero =>
    intro d env H Γ e t Γ' _ _
    cases e <;> exact True.intro
  | succ k ih =>
    intro d env H Γ e t Γ' hag h
    cases h with
    | int _ _ =>
      exact And.intro True.intro (And.intro hag (And.intro True.intro
        (And.intro (heapLe_refl H) (lower_refl H d))))
    | bool _ _ =>
      exact And.intro True.intro (And.intro hag (And.intro True.intro
        (And.intro (heapLe_refl H) (lower_refl H d))))
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
      apply bind_safe (Nat.le_refl d) (ih d env H Γ e (.ref m t) Γ' hag he)
      intro v env1 H1 hv ha1 _ _ _
      exact derefOp_safe hv ha1
    | store _ Γ1 _ a b t0 ha hb =>
      simp only [eval]
      apply bind_safe (Nat.le_refl d) (ih d env H Γ a (.ref true t0) Γ1 hag ha)
      intro v env1 H1 hv ha1 _ _ _
      apply bind_safe (Nat.le_refl d) (ih d env1 H1 Γ1 b t0 Γ' ha1 hb)
      intro w env2 H2 hw ha2 _ hle2 _
      exact storeOp_safe (ref_mono hle2 hv) hw ha2
    | mkBox _ _ e t0 he hc =>
      simp only [eval]
      apply bind_safe (Nat.le_refl d) (ih d env H Γ e t0 Γ' hag he)
      intro v env1 H1 hv ha1 _ _ _
      exact mkBox_safe hp ha1 hv hc
    | add _ Γ1 _ a b ha hb =>
      simp only [eval]
      apply bind_safe (Nat.le_refl d) (ih d env H Γ a .int Γ1 hag ha)
      intro x env1 H1 hx ha1 _ _ _
      apply bind_safe (Nat.le_refl d) (ih d env1 H1 Γ1 b .int Γ' ha1 hb)
      intro y env2 H2 hy ha2 _ _ _
      exact addOp_safe hx hy ha2
    | letE _ Γ1 _ x body s _ p hx hb =>
      simp only [eval]
      apply bind_safe (Nat.le_refl d) (ih d env H Γ x s Γ1 hag hx)
      intro v env1 H1 hv ha1 hl1 _ _
      apply safe_base (claim_heapLe H1 v (d, env1.length)) (claim_lower hl1)
      apply bind_safe (Nat.le_refl d)
        (ih d _ _ (Γ1 ++ [(s, true)]) body t (Γ' ++ [p]) (agree_push_claim ha1 hv (loose_free hl1)) hb)
      intro w env2 H2 hw ha2 hl2 _ _
      obtain ⟨env', x', he, hlen⟩ := split_last env2 Γ'.length (by
        have h2 := ha2.len
        simp only [List.length_append, List.length_cons, List.length_nil] at h2
        omega)
      subst he
      simp only [init_append, lastOpt_append]
      exact let_end ha2 hlen hw hl2
    | call _ _ f e fn hf he =>
      simp only [eval]
      apply bind_safe (Nat.le_refl d) (ih d env H Γ e fn.param Γ' hag he)
      intro v env1 H1 hv ha1 hl1 _ _
      simp only [hf]
      obtain ⟨b, hbody⟩ := hfns f fn hf
      have hagc : Agree ([] ++ [some v]) (claim H1 v (d + 1, ([] : Env).length))
          ([] ++ [(fn.param, true)]) (d + 1) :=
        agree_push_claim (agree_nil ha1.heap) hv free_nil
      apply safe_base (claim_heapLe H1 v (d + 1, 0)) (claim_lower hl1)
      apply bind_safe (Nat.le_succ d)
        (ih (d + 1) [some v] (claim H1 v (d + 1, 0)) [(fn.param, true)] fn.body fn.ret
          [(fn.param, b)] hagc hbody)
      intro w env2 H2 hw ha2 hl2 hle2 hlow2
      obtain ⟨env', x', hev, hlen⟩ := split_last env2 0 (by
        have h2 := ha2.len
        simp only [List.length_cons, List.length_nil] at h2
        omega)
      subst hev
      simp only [lastOpt_append]
      obtain ⟨hleD, hokD, hsame⟩ := drop_spec (Γ := []) ha2 hlen
      have hwl : ∀ lw : Nat, w = .box lw → Keep H2 (dropSlot H2 (some x')) lw := by
        intro lw hwb
        subst hwb
        have hl' : ∃ (c : Cell) (o : Own) (t : Ty) (w : Val), nth H2 lw = some c ∧
            c.val = some (o, t, w) ∧ d + 1 ≤ o.1 ∧
            (o.1 = d + 1 → ∀ t' : Ty, nth [(fn.param, b)] o.2 ≠ some (t', true)) := hl2
        obtain ⟨c1, o, t1, w1, h1, hv1, _, hnl⟩ := hl'
        apply keep_of_eq
        refine hsame lw c1 o t1 w1 h1 hv1 ?_
        intro heq
        subst heq
        cases b with
        | false => rfl
        | true => exact False.elim (hnl rfl fn.param rfl)
      have hkeep : ∀ (j l : Nat) (c : Cell) (t : Ty) (w : Val), nth H1 l = some c →
          c.val = some ((d, j), t, w) → (∃ t' : Ty, nth Γ' j = some (t', true)) →
          Keep H1 (dropSlot H2 (some x')) l := by
        intro j l c t1 w1 hc hcv halive
        have heq1 : nth (claim H1 v (d + 1, 0)) l = nth H1 l :=
          claim_keep_live (loose_free hl1) hc hcv halive
        have k1 : Keep H1 (claim H1 v (d + 1, 0)) l := keep_of_eq heq1
        have k2 : Keep (claim H1 v (d + 1, 0)) H2 l := by
          apply lower_keep hlow2
          intro c0 o0 t0 w0 h0 hv0
          rw [heq1, hc] at h0
          have h5 : c = c0 := Option.some.inj h0
          rw [← h5, hcv] at hv0
          obtain ⟨hoo, _, _⟩ := triple_inj hv0
          rw [← hoo]
          exact Nat.lt_succ_self d
        have k12 := keep_trans k1 k2
        obtain ⟨c2, w2, h2, hv2⟩ := k12 c (d, j) t1 w1 hc hcv
        have k3 : Keep H2 (dropSlot H2 (some x')) l := by
          apply keep_of_eq
          refine hsame l c2 (d, j) t1 w2 h2 hv2 ?_
          intro heq
          have h6 : d = d + 1 := congrArg Prod.fst heq
          exact absurd h6 (by omega)
        exact keep_trans k12 k3
      have hle1 : HeapLe H1 (dropSlot H2 (some x')) :=
        heapLe_trans (claim_heapLe H1 v (d + 1, 0)) (heapLe_trans hle2 hleD)
      have hlow : Lower H2 (dropSlot H2 (some x')) d := by
        apply lower_of_eq
        intro l c o t1 w1 hc hcv hlt
        refine hsame l c o t1 w1 hc hcv ?_
        intro heq
        have h1 : o.1 = d + 1 := by rw [heq]
        exact absurd h1 (by omega)
      exact And.intro (val_transport hleD hwl hw)
        (And.intro (agree_transport ha1 hle1 hkeep hokD)
          (And.intro (loose_transport hwl (loose_up hl2)) (And.intro hleD hlow)))

/-- 型が付く式は、どれだけの燃料でも、どの割り当ての方針でも行き詰まらない -/
theorem no_stuck {pick : Heap → Option Nat} (hp : PickOk pick) {fns : List Fn} (hfns : FnsOk fns)
    (k d : Nat) (env : Env) (H : Heap) (Γ : Ctx) (e : Expr) (t : Ty) (Γ' : Ctx)
    (hag : Agree env H Γ d) (h : HasTy fns Γ e t Γ') : eval pick fns k d env H e ≠ .stuck := by
  intro hs
  have hsafe := eval_safe hp hfns k d env H Γ e t Γ' hag h
  rw [hs] at hsafe
  exact hsafe

/-! ## 意味を持つことの確認 -/

theorem pick_none_ok : PickOk (fun _ => none) := by
  unfold PickOk
  intro _ l h
  have h' : (none : Option Nat) = some l := h
  cases h'

/-- 箱を受けとり、その箱への ref を返す関数 `fn f(b: box int) -> ref int { &b }` だけの表 -/
def escapeFns : List Fn := [⟨.box .int, .ref false .int, .addr false 0⟩]

theorem escapeFns_ok : FnsOk escapeFns := by
  unfold FnsOk
  intro f fn hf
  cases f with
  | zero =>
    have h : some (⟨.box .int, .ref false .int, .addr false 0⟩ : Fn) = some fn := hf
    cases h
    exact ⟨true, HasTy.addr _ false 0 .int rfl⟩
  | succ _ =>
    have h : (none : Option Fn) = some fn := hf
    cases h

/-- `*f(box 1)` に型が付く -/
theorem escape_typed :
    HasTy escapeFns [] (.deref (.call 0 (.mkBox .int (.int 1)))) .int [] :=
  HasTy.deref _ _ _ false .int
    (HasTy.call [] [] 0 _ ⟨.box .int, .ref false .int, .addr false 0⟩ rfl
      (HasTy.mkBox _ _ _ _ (HasTy.int _ _) True.intro))

/-- 関数の終わりに引数の箱の区画が解放され（世代 1）、返された ref（世代 0）を読むと実行時の誤りで止まる -/
theorem escape_err :
    eval (fun _ => none) escapeFns 4 0 [] [] (.deref (.call 0 (.mkBox .int (.int 1)))) = .err := rfl

/-- 書き込める ref を受けとって 7 を書く関数 `fn g(r: ref mut int) -> int { *r = 7 }` だけの表 -/
def writeFns : List Fn := [⟨.ref true .int, .int, .store (.var 0) (.int 7)⟩]

theorem writeFns_ok : FnsOk writeFns := by
  unfold FnsOk
  intro f fn hf
  cases f with
  | zero =>
    have h : some (⟨.ref true .int, .int, .store (.var 0) (.int 7)⟩ : Fn) = some fn := hf
    cases h
    exact ⟨true, HasTy.store _ _ _ _ _ .int (HasTy.var _ 0 (.ref true .int) rfl True.intro)
      (HasTy.int _ _)⟩
  | succ _ =>
    have h : (none : Option Fn) = some fn := hf
    cases h

/-- `let b = box 1 in let _ = g(&mut b) in *b` に型が付く -/
theorem write_typed :
    HasTy writeFns [] (.letE (.mkBox .int (.int 1)) (.letE (.call 0 (.addr true 0)) (.peek 0)))
      .int [] :=
  HasTy.letE [] [] [] _ _ (.box .int) .int (.box .int, true)
    (HasTy.mkBox _ _ _ _ (HasTy.int _ _) True.intro)
    (HasTy.letE _ _ _ _ _ .int .int (.int, true)
      (HasTy.call _ _ 0 _ ⟨.ref true .int, .int, .store (.var 0) (.int 7)⟩ rfl
        (HasTy.addr _ true 0 .int rfl))
      (HasTy.peek _ 0 .int rfl))

/-- 関数が ref を通して書いた値（7）が、呼び出し元の持ち主から読め、最後に区画が解放される -/
theorem write_then_peek :
    eval (fun _ => none) writeFns 6 0 [] []
      (.letE (.mkBox .int (.int 1)) (.letE (.call 0 (.addr true 0)) (.peek 0))) =
      .ok (.int 7) [] [⟨1, none⟩] := rfl

end JabberwockFormal.CoreGenFun
