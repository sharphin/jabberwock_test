set_option autoImplicit false
set_option warningAsError true

namespace JabberwockFormal.CoreBorrow

/-!
借用の期間を足した模型（docs/core-type-safety.md）。

`CoreOwnership` の箱に、読み取り専用の ref と借用の期間を足す。

* 変数は「レベル」（環境の先頭からの位置）で指す。let と borrow は環境の末尾に足し、終わると末尾を外す。
* `borrow i body` : 変数 i の箱への ref を末尾の新しい変数に入れて body を評価する。
  body の間、i は凍る（`frozen k`、k は ref を入れた位置）。凍った変数は移動できないが、`peek` はできる。
* `deref e` : ref の指す変数を読む。指す先が移動済みなら行き詰まり（宙に浮いた ref）。
* ref はコピーできるが、let・borrow・関数の外へは持ち出せない（結果の型は ref でない）。

不変条件の要: ref の値は凍った箱を指し、その凍結の位置 k は ref を持つ位置以下。
よって borrow の終わりに末尾を外すと、その箱を指す ref は残らず、凍結を解いてよい。

定理 `eval_safe`: 型が付く式を文脈と合う環境で評価すると stuck にならない（移動済みの変数・宙に浮いた ref を使わない）。

これは核の言語の模型の定理で、jwc・Rust 版の所有権・借用の検査、世代つき ref の実行時検査、
IR・LLVM・実行時ライブラリとの対応の証明ではない。
-/

inductive Ty where
  | int
  | bool
  | box
  | ref
  deriving DecidableEq

/-- 変数の状態: 生きている・凍っている（位置 k の ref が借りている）・死んだ（移動済み） -/
inductive St where
  | alive
  | frozen (k : Nat)
  | dead

def Live : St → Prop
  | .dead => False
  | _ => True

inductive Expr where
  | int (n : Int)
  | bool (b : Bool)
  | var (i : Nat)
  | move (i : Nat)
  | peek (i : Nat)
  | deref (e : Expr)
  | mkBox (e : Expr)
  | unbox (e : Expr)
  | add (a b : Expr)
  | lt (a b : Expr)
  | ite (c a b : Expr)
  | letE (bound body : Expr)
  | borrow (i : Nat) (body : Expr)
  | call (f : Nat) (arg : Expr)

inductive Val where
  | int (n : Int)
  | bool (b : Bool)
  | box (n : Int)
  | ref (j : Nat)

abbrev Env := List (Option Val)
abbrev Ctx := List (Ty × St)

structure Fn where
  param : Ty
  ret : Ty
  body : Expr

def nth {α : Type} : List α → Nat → Option α
  | [], _ => none
  | a :: _, 0 => some a
  | _ :: l, i + 1 => nth l i

def setNth {α : Type} : List α → Nat → α → List α
  | [], _, _ => []
  | _ :: l, 0, a => a :: l
  | x :: l, i + 1, a => x :: setNth l i a

/-- 末尾を外す -/
def init {α : Type} : List α → List α
  | [] => []
  | [_] => []
  | a :: b :: l => a :: init (b :: l)

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

theorem nth_append_gt {α : Type} {l : List α} {x : α} {j : Nat} (h : l.length < j) :
    nth (l ++ [x]) j = none := by
  induction l generalizing j with
  | nil =>
    cases j with
    | zero => exact absurd h (Nat.lt_irrefl 0)
    | succ _ => rfl
  | cons _ l ih =>
    cases j with
    | zero => exact absurd h (Nat.not_lt_zero _)
    | succ j => exact ih (Nat.lt_of_succ_lt_succ h)

theorem length_setNth {α : Type} (l : List α) (i : Nat) (x : α) : (setNth l i x).length = l.length := by
  induction l generalizing i with
  | nil => rfl
  | cons _ l ih =>
    cases i with
    | zero => rfl
    | succ i => exact congrArg Nat.succ (ih i)

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

theorem split_last {α : Type} (l : List α) :
    ∀ n : Nat, l.length = n + 1 → ∃ (l' : List α) (x : α), l = l' ++ [x] ∧ l'.length = n := by
  induction l with
  | nil =>
    intro n h
    exact absurd h.symm (Nat.succ_ne_zero n)
  | cons a l ih =>
    intro n h
    cases l with
    | nil =>
      refine ⟨[], a, rfl, ?_⟩
      have h' : 0 + 1 = n + 1 := h
      show 0 = n
      omega
    | cons b l' =>
      cases n with
      | zero =>
        have h' : l'.length + 1 + 1 = 0 + 1 := h
        exact absurd h' (by omega)
      | succ n =>
        have h' : (b :: l').length = n + 1 := by
          have h2 : l'.length + 1 + 1 = n + 1 + 1 := h
          show l'.length + 1 = n + 1
          omega
        obtain ⟨l'', x, he, hl⟩ := ih n h'
        refine ⟨a :: l'', x, ?_, ?_⟩
        · show a :: (b :: l') = a :: (l'' ++ [x])
          rw [he]
        · show l''.length + 1 = n + 1
          rw [hl]

theorem init_append {α : Type} (l : List α) (x : α) : init (l ++ [x]) = l := by
  induction l with
  | nil => rfl
  | cons a l ih =>
    cases l with
    | nil => rfl
    | cons b l' =>
      show a :: init ((b :: l') ++ [x]) = a :: b :: l'
      rw [ih]

/-! ## 型の規則 -/

inductive HasTy (fs : List Fn) : Ctx → Expr → Ty → Ctx → Prop where
  | int (Γ : Ctx) (n : Int) : HasTy fs Γ (.int n) .int Γ
  | bool (Γ : Ctx) (b : Bool) : HasTy fs Γ (.bool b) .bool Γ
  | var (Γ : Ctx) (i : Nat) (t : Ty) (h : nth Γ i = some (t, .alive)) (hc : t ≠ .box) :
      HasTy fs Γ (.var i) t Γ
  | move (Γ : Ctx) (i : Nat) (h : nth Γ i = some (.box, .alive)) :
      HasTy fs Γ (.move i) .box (setNth Γ i (.box, .dead))
  | peek (Γ : Ctx) (i : Nat) (st : St) (h : nth Γ i = some (.box, st)) (hl : Live st) :
      HasTy fs Γ (.peek i) .int Γ
  | deref (Γ Γ1 : Ctx) (e : Expr) (he : HasTy fs Γ e .ref Γ1) : HasTy fs Γ (.deref e) .int Γ1
  | mkBox (Γ Γ1 : Ctx) (e : Expr) (he : HasTy fs Γ e .int Γ1) : HasTy fs Γ (.mkBox e) .box Γ1
  | unbox (Γ Γ1 : Ctx) (e : Expr) (he : HasTy fs Γ e .box Γ1) : HasTy fs Γ (.unbox e) .int Γ1
  | add (Γ Γ1 Γ2 : Ctx) (a b : Expr) (ha : HasTy fs Γ a .int Γ1) (hb : HasTy fs Γ1 b .int Γ2) :
      HasTy fs Γ (.add a b) .int Γ2
  | lt (Γ Γ1 Γ2 : Ctx) (a b : Expr) (ha : HasTy fs Γ a .int Γ1) (hb : HasTy fs Γ1 b .int Γ2) :
      HasTy fs Γ (.lt a b) .bool Γ2
  | ite (Γ Γ1 Γ2 : Ctx) (c a b : Expr) (t : Ty) (hc : HasTy fs Γ c .bool Γ1)
      (ha : HasTy fs Γ1 a t Γ2) (hb : HasTy fs Γ1 b t Γ2) : HasTy fs Γ (.ite c a b) t Γ2
  | letE (Γ Γ1 Γ2 : Ctx) (x body : Expr) (s t : Ty) (p : Ty × St) (hx : HasTy fs Γ x s Γ1)
      (hbody : HasTy fs (Γ1 ++ [(s, .alive)]) body t (Γ2 ++ [p])) (hnr : t ≠ .ref) :
      HasTy fs Γ (.letE x body) t Γ2
  | borrow (Γ : Ctx) (i : Nat) (body : Expr) (t : Ty) (Γ2 : Ctx) (p : Ty × St)
      (hi : nth Γ i = some (.box, .alive))
      (hbody : HasTy fs (setNth Γ i (.box, .frozen Γ.length) ++ [(.ref, .alive)]) body t (Γ2 ++ [p]))
      (hnr : t ≠ .ref) (hfr : nth Γ2 i = some (.box, .frozen Γ.length)) :
      HasTy fs Γ (.borrow i body) t (setNth Γ2 i (.box, .alive))
  | call (Γ Γ1 : Ctx) (f : Nat) (a : Expr) (fn : Fn) (hf : nth fs f = some fn)
      (hpr : fn.param ≠ .ref) (hrr : fn.ret ≠ .ref) (ha : HasTy fs Γ a fn.param Γ1) :
      HasTy fs Γ (.call f a) fn.ret Γ1

def ProgOk (fs : List Fn) : Prop :=
  ∀ (f : Nat) (fn : Fn), nth fs f = some fn →
    ∃ out : Ctx, HasTy fs [(fn.param, .alive)] fn.body fn.ret out

theorem hasTy_length {fs : List Fn} {Γ : Ctx} {e : Expr} {t : Ty} {Γ' : Ctx}
    (h : HasTy fs Γ e t Γ') : Γ'.length = Γ.length := by
  induction h with
  | int => rfl
  | bool => rfl
  | var => rfl
  | move Γ i _ => exact length_setNth Γ i _
  | peek => rfl
  | deref _ _ _ _ ih => exact ih
  | mkBox _ _ _ _ ih => exact ih
  | unbox _ _ _ _ ih => exact ih
  | add _ _ _ _ _ _ _ ih1 ih2 => exact ih2.trans ih1
  | lt _ _ _ _ _ _ _ ih1 ih2 => exact ih2.trans ih1
  | ite _ _ _ _ _ _ _ _ _ _ ihc iha _ => exact iha.trans ihc
  | letE _ _ _ _ _ _ _ _ _ _ _ ih1 ih2 =>
    simp only [List.length_append, List.length_cons, List.length_nil] at ih2
    omega
  | borrow _ _ _ _ _ _ _ _ _ _ ih =>
    simp only [List.length_append, List.length_cons, List.length_nil, length_setNth] at ih ⊢
    omega
  | call _ _ _ _ _ _ _ _ _ ih => exact ih

/-! ## 評価 -/

inductive Res where
  | ok (v : Val) (env : Env)
  | err
  | fuel
  | stuck

def Res.bind : Res → (Val → Env → Res) → Res
  | .ok v env, f => f v env
  | .err, _ => .err
  | .fuel, _ => .fuel
  | .stuck, _ => .stuck

def checked (z : Int) (env : Env) : Res :=
  if -(2 : Int) ^ 63 ≤ z ∧ z < (2 : Int) ^ 63 then .ok (.int z) env else .err

def addOp : Val → Val → Env → Res
  | .int p, .int q, env => checked (p + q) env
  | _, _, _ => .stuck

def ltOp : Val → Val → Env → Res
  | .int p, .int q, env => .ok (.bool (decide (p < q))) env
  | _, _, _ => .stuck

/-- コピー: 箱はコピーできない -/
def copyVal : Val → Env → Res
  | .box _, _ => .stuck
  | v, env => .ok v env

def moveVal : Val → Env → Res
  | .box n, env => .ok (.box n) env
  | _, _ => .stuck

def peekVal : Val → Env → Res
  | .box n, env => .ok (.int n) env
  | _, _ => .stuck

def readCopy (env : Env) (i : Nat) : Res :=
  match nth env i with
  | some (some v) => copyVal v env
  | _ => .stuck

def moveOut (env : Env) (i : Nat) : Res :=
  match nth env i with
  | some (some v) => moveVal v (setNth env i none)
  | _ => .stuck

/-- 位置 j の箱の中身を読む。移動済みなら行き詰まり -/
def peekAt (env : Env) (j : Nat) : Res :=
  match nth env j with
  | some (some v) => peekVal v env
  | _ => .stuck

def derefOp : Val → Env → Res
  | .ref j, env => peekAt env j
  | _, _ => .stuck

def mkBoxOp : Val → Env → Res
  | .int n, env => .ok (.box n) env
  | _, _ => .stuck

def unboxOp : Val → Env → Res
  | .box n, env => .ok (.int n) env
  | _, _ => .stuck

def eval (fs : List Fn) : Nat → Env → Expr → Res
  | 0, _, _ => .fuel
  | _ + 1, env, .int n => .ok (.int n) env
  | _ + 1, env, .bool b => .ok (.bool b) env
  | _ + 1, env, .var i => readCopy env i
  | _ + 1, env, .move i => moveOut env i
  | _ + 1, env, .peek i => peekAt env i
  | k + 1, env, .deref e => (eval fs k env e).bind derefOp
  | k + 1, env, .mkBox e => (eval fs k env e).bind mkBoxOp
  | k + 1, env, .unbox e => (eval fs k env e).bind unboxOp
  | k + 1, env, .add a b =>
    (eval fs k env a).bind fun x env1 => (eval fs k env1 b).bind fun y env2 => addOp x y env2
  | k + 1, env, .lt a b =>
    (eval fs k env a).bind fun x env1 => (eval fs k env1 b).bind fun y env2 => ltOp x y env2
  | k + 1, env, .ite c a b =>
    (eval fs k env c).bind fun v env1 =>
      match v with
      | .bool true => eval fs k env1 a
      | .bool false => eval fs k env1 b
      | _ => .stuck
  | k + 1, env, .letE x body =>
    (eval fs k env x).bind fun v env1 =>
      (eval fs k (env1 ++ [some v]) body).bind fun w env2 => .ok w (init env2)
  | k + 1, env, .borrow i body =>
    (eval fs k (env ++ [some (.ref i)]) body).bind fun w env2 => .ok w (init env2)
  | k + 1, env, .call f a =>
    (eval fs k env a).bind fun v env1 =>
      match nth fs f with
      | some fn => (eval fs k [some v] fn.body).bind fun w _ => .ok w env1
      | none => .stuck

/-! ## 不変条件 -/

/-- 値が型に合う。ref は、凍った箱で凍結の位置が m 以下のものを指す -/
def ValOk (Γ : Ctx) (m : Nat) : Val → Ty → Prop
  | .int _, .int => True
  | .bool _, .bool => True
  | .box _, .box => True
  | .ref j, .ref => ∃ k : Nat, nth Γ j = some (.box, .frozen k) ∧ k ≤ m
  | _, _ => False

/-- 位置 m の変数の中身が合う: 死んでいなければ、値があって型に合う -/
def SlotOk (Γ : Ctx) (m : Nat) (s : Option Val) (t : Ty) (st : St) : Prop :=
  Live st → ∃ v : Val, s = some v ∧ ValOk Γ m v t

structure Agree (env : Env) (Γ : Ctx) : Prop where
  len : env.length = Γ.length
  wf : ∀ (j k : Nat), nth Γ j = some (.box, .frozen k) → j < k
  slot : ∀ (j : Nat) (t : Ty) (st : St), nth Γ j = some (t, st) →
    ∃ s : Option Val, nth env j = some s ∧ SlotOk Γ j s t st

def Safe (t : Ty) (Γ : Ctx) : Res → Prop
  | .ok v env => ValOk Γ Γ.length v t ∧ Agree env Γ
  | .err => True
  | .fuel => True
  | .stuck => False

theorem valOk_transport {Γ Γ' : Ctx} {m : Nat} {v : Val} {t : Ty}
    (H : ∀ (j k : Nat), nth Γ j = some (.box, .frozen k) → k ≤ m → nth Γ' j = some (.box, .frozen k))
    (h : ValOk Γ m v t) : ValOk Γ' m v t := by
  cases v with
  | ref j =>
    cases t with
    | ref =>
      have h' : ∃ k : Nat, nth Γ j = some (Ty.box, St.frozen k) ∧ k ≤ m := h
      obtain ⟨k, hk, hkm⟩ := h'
      exact Exists.intro k (And.intro (H j k hk hkm) hkm)
    | int => exact h
    | bool => exact h
    | box => exact h
  | int _ => cases t <;> exact h
  | bool _ => cases t <;> exact h
  | box _ => cases t <;> exact h

theorem valOk_nonref {Γ Γ' : Ctx} {m m' : Nat} {v : Val} {t : Ty} (hnr : t ≠ .ref)
    (h : ValOk Γ m v t) : ValOk Γ' m' v t := by
  cases v <;> cases t <;> first | exact h | exact absurd rfl hnr

theorem valOk_mono {Γ : Ctx} {m m' : Nat} {v : Val} {t : Ty} (h : ValOk Γ m v t) (hm : m ≤ m') :
    ValOk Γ m' v t := by
  cases v with
  | ref j =>
    cases t with
    | ref =>
      have h' : ∃ k : Nat, nth Γ j = some (Ty.box, St.frozen k) ∧ k ≤ m := h
      obtain ⟨k, hk, hkm⟩ := h'
      exact Exists.intro k (And.intro hk (Nat.le_trans hkm hm))
    | int => exact h
    | bool => exact h
    | box => exact h
  | int _ => cases t <;> exact h
  | bool _ => cases t <;> exact h
  | box _ => cases t <;> exact h

theorem slotOk_transport {Γ Γ' : Ctx} {m : Nat} {s : Option Val} {t : Ty} {st : St}
    (H : ∀ (j k : Nat), nth Γ j = some (.box, .frozen k) → k ≤ m → nth Γ' j = some (.box, .frozen k))
    (h : SlotOk Γ m s t st) : SlotOk Γ' m s t st := by
  unfold SlotOk at h ⊢
  intro hl
  obtain ⟨v, hs, hv⟩ := h hl
  exact ⟨v, hs, valOk_transport H hv⟩

theorem slotOk_live {Γ : Ctx} {m : Nat} {s : Option Val} {v : Val} {t : Ty} {st : St}
    (hs : s = some v) (hv : ValOk Γ m v t) : SlotOk Γ m s t st := by
  unfold SlotOk
  intro _
  exact ⟨v, hs, hv⟩

theorem slotOk_dead {Γ : Ctx} {m : Nat} {s : Option Val} {t : Ty} : SlotOk Γ m s t .dead := by
  unfold SlotOk
  intro h
  exact False.elim h

/-- 箱の変数の状態を変えても、中身は箱のまま -/
theorem slotOk_box {Γ Γ' : Ctx} {m : Nat} {s : Option Val} {st st' : St}
    (h : SlotOk Γ m s .box st) (hl : Live st) : SlotOk Γ' m s .box st' := by
  unfold SlotOk at h ⊢
  intro _
  obtain ⟨v, hs, hv⟩ := h hl
  exact ⟨v, hs, valOk_nonref (by intro e; cases e) hv⟩

theorem agree_push {env : Env} {Γ : Ctx} {v : Val} {s : Ty} (ha : Agree env Γ)
    (hv : ValOk Γ Γ.length v s) : Agree (env ++ [some v]) (Γ ++ [(s, .alive)]) := by
  have H : ∀ (m j k : Nat), nth Γ j = some (.box, .frozen k) → k ≤ m →
      nth (Γ ++ [(s, .alive)]) j = some (.box, .frozen k) := by
    intro _ j k hj _
    rw [nth_append_lt (nth_lt hj)]
    exact hj
  refine ⟨?_, ?_, ?_⟩
  · simp only [List.length_append, List.length_cons, List.length_nil, ha.len]
  · intro j k hj
    rcases Nat.lt_trichotomy j Γ.length with hlt | heq | hgt
    · rw [nth_append_lt hlt] at hj
      exact ha.wf j k hj
    · subst heq
      rw [nth_append_len] at hj
      cases hj
    · rw [nth_append_gt hgt] at hj
      cases hj
  · intro j t st hj
    rcases Nat.lt_trichotomy j Γ.length with hlt | heq | hgt
    · rw [nth_append_lt hlt] at hj
      obtain ⟨s', hs', hslot⟩ := ha.slot j t st hj
      refine ⟨s', ?_, slotOk_transport (H j) hslot⟩
      rw [nth_append_lt (by rw [ha.len]; exact hlt)]
      exact hs'
    · subst heq
      rw [nth_append_len] at hj
      cases hj
      refine ⟨some v, ?_, slotOk_live rfl (valOk_transport (H Γ.length) hv)⟩
      rw [← ha.len]
      exact nth_append_len env (some v)
    · rw [nth_append_gt hgt] at hj
      cases hj

theorem agree_pop {env : Env} {Γ : Ctx} {x : Option Val} {p : Ty × St}
    (ha : Agree (env ++ [x]) (Γ ++ [p])) (hl : env.length = Γ.length) : Agree env Γ := by
  refine ⟨hl, ?_, ?_⟩
  · intro j k hj
    exact ha.wf j k (by rw [nth_append_lt (nth_lt hj)]; exact hj)
  · intro j t st hj
    have hlt := nth_lt hj
    have hj' : nth (Γ ++ [p]) j = some (t, st) := by
      rw [nth_append_lt hlt]
      exact hj
    obtain ⟨s, hs, hslot⟩ := ha.slot j t st hj'
    refine ⟨s, ?_, ?_⟩
    · rw [nth_append_lt (by rw [hl]; exact hlt)] at hs
      exact hs
    · refine slotOk_transport ?_ hslot
      intro j' k hj'' hk
      have hjk := ha.wf j' k hj''
      have hj'lt : j' < Γ.length := by omega
      rw [nth_append_lt hj'lt] at hj''
      exact hj''

theorem agree_kill {env : Env} {Γ : Ctx} {i : Nat} (ha : Agree env Γ)
    (hi : nth Γ i = some (.box, .alive)) :
    Agree (setNth env i none) (setNth Γ i (.box, .dead)) := by
  have hlt := nth_lt hi
  have H : ∀ (m j k : Nat), nth Γ j = some (.box, .frozen k) → k ≤ m →
      nth (setNth Γ i (.box, .dead)) j = some (.box, .frozen k) := by
    intro _ j k hj _
    have hne : j ≠ i := by
      intro e
      subst e
      rw [hi] at hj
      cases hj
    rw [nth_setNth_ne hne]
    exact hj
  refine ⟨?_, ?_, ?_⟩
  · rw [length_setNth, length_setNth]
    exact ha.len
  · intro j k hj
    by_cases hji : j = i
    · subst hji
      rw [nth_setNth_eq hlt] at hj
      cases hj
    · rw [nth_setNth_ne hji] at hj
      exact ha.wf j k hj
  · intro j t st hj
    by_cases hji : j = i
    · subst hji
      rw [nth_setNth_eq hlt] at hj
      cases hj
      refine ⟨none, ?_, slotOk_dead⟩
      exact nth_setNth_eq (by rw [ha.len]; exact hlt)
    · rw [nth_setNth_ne hji] at hj
      obtain ⟨s, hs, hslot⟩ := ha.slot j t st hj
      refine ⟨s, ?_, slotOk_transport (H j) hslot⟩
      rw [nth_setNth_ne hji]
      exact hs

theorem agree_freeze {env : Env} {Γ : Ctx} {i : Nat} (ha : Agree env Γ)
    (hi : nth Γ i = some (.box, .alive)) :
    Agree env (setNth Γ i (.box, .frozen Γ.length)) := by
  have hlt := nth_lt hi
  have H : ∀ (m j k : Nat), nth Γ j = some (.box, .frozen k) → k ≤ m →
      nth (setNth Γ i (.box, .frozen Γ.length)) j = some (.box, .frozen k) := by
    intro _ j k hj _
    have hne : j ≠ i := by
      intro e
      subst e
      rw [hi] at hj
      cases hj
    rw [nth_setNth_ne hne]
    exact hj
  refine ⟨?_, ?_, ?_⟩
  · rw [length_setNth]
    exact ha.len
  · intro j k hj
    by_cases hji : j = i
    · subst hji
      rw [nth_setNth_eq hlt] at hj
      cases hj
      exact hlt
    · rw [nth_setNth_ne hji] at hj
      exact ha.wf j k hj
  · intro j t st hj
    by_cases hji : j = i
    · subst hji
      rw [nth_setNth_eq hlt] at hj
      cases hj
      obtain ⟨s, hs, hslot⟩ := ha.slot _ .box .alive hi
      exact ⟨s, hs, slotOk_box hslot True.intro⟩
    · rw [nth_setNth_ne hji] at hj
      obtain ⟨s, hs, hslot⟩ := ha.slot j t st hj
      exact ⟨s, hs, slotOk_transport (H j) hslot⟩

theorem agree_unfreeze {env : Env} {Γ : Ctx} {i : Nat} (ha : Agree env Γ)
    (hi : nth Γ i = some (.box, .frozen Γ.length)) :
    Agree env (setNth Γ i (.box, .alive)) := by
  have hlt := nth_lt hi
  refine ⟨?_, ?_, ?_⟩
  · rw [length_setNth]
    exact ha.len
  · intro j k hj
    by_cases hji : j = i
    · subst hji
      rw [nth_setNth_eq hlt] at hj
      cases hj
    · rw [nth_setNth_ne hji] at hj
      exact ha.wf j k hj
  · intro j t st hj
    by_cases hji : j = i
    · subst hji
      rw [nth_setNth_eq hlt] at hj
      cases hj
      obtain ⟨s, hs, hslot⟩ := ha.slot _ .box (.frozen Γ.length) hi
      exact ⟨s, hs, slotOk_box hslot True.intro⟩
    · rw [nth_setNth_ne hji] at hj
      have hjlt := nth_lt hj
      obtain ⟨s, hs, hslot⟩ := ha.slot j t st hj
      refine ⟨s, hs, slotOk_transport ?_ hslot⟩
      intro j' k hj' hk
      have hne : j' ≠ i := by
        intro e
        subst e
        rw [hi] at hj'
        cases hj'
        omega
      rw [nth_setNth_ne hne]
      exact hj'

theorem agree_single {v : Val} {t : Ty} (hv : ValOk [(t, .alive)] 0 v t) :
    Agree [some v] [(t, .alive)] := by
  refine ⟨rfl, ?_, ?_⟩
  · intro j k hj
    cases j with
    | zero =>
      have h' : some (t, St.alive) = some (Ty.box, St.frozen k) := hj
      cases h'
    | succ j =>
      have h' : (none : Option (Ty × St)) = some (Ty.box, St.frozen k) := by
        cases j <;> exact hj
      cases h'
  · intro j t' st hj
    cases j with
    | zero =>
      have h' : some (t, St.alive) = some (t', st) := hj
      cases h'
      exact ⟨some v, rfl, slotOk_live rfl hv⟩
    | succ j =>
      have h' : (none : Option (Ty × St)) = some (t', st) := by
        cases j <;> exact hj
      cases h'

/-! ## 一つずつの操作の安全性 -/

theorem bind_safe {s t : Ty} {Γ1 Γ2 : Ctx} {r : Res} {f : Val → Env → Res} (hr : Safe s Γ1 r)
    (hf : ∀ (v : Val) (env : Env), ValOk Γ1 Γ1.length v s → Agree env Γ1 → Safe t Γ2 (f v env)) :
    Safe t Γ2 (r.bind f) := by
  cases r with
  | ok v env => exact hf v env hr.1 hr.2
  | err => exact True.intro
  | fuel => exact True.intro
  | stuck => exact False.elim hr

theorem checked_safe {Γ : Ctx} {env : Env} (z : Int) (ha : Agree env Γ) :
    Safe .int Γ (checked z env) := by
  unfold checked
  split
  · exact And.intro True.intro ha
  · exact True.intro

theorem addOp_safe {Γ Γa Γb : Ctx} {ma mb : Nat} {x y : Val} {env : Env} (hx : ValOk Γa ma x .int)
    (hy : ValOk Γb mb y .int) (ha : Agree env Γ) : Safe .int Γ (addOp x y env) := by
  cases x with
  | bool _ => exact False.elim hx
  | box _ => exact False.elim hx
  | ref _ => exact False.elim hx
  | int p =>
    cases y with
    | bool _ => exact False.elim hy
    | box _ => exact False.elim hy
    | ref _ => exact False.elim hy
    | int q => exact checked_safe (p + q) ha

theorem ltOp_safe {Γ Γa Γb : Ctx} {ma mb : Nat} {x y : Val} {env : Env} (hx : ValOk Γa ma x .int)
    (hy : ValOk Γb mb y .int) (ha : Agree env Γ) : Safe .bool Γ (ltOp x y env) := by
  cases x with
  | bool _ => exact False.elim hx
  | box _ => exact False.elim hx
  | ref _ => exact False.elim hx
  | int _ =>
    cases y with
    | bool _ => exact False.elim hy
    | box _ => exact False.elim hy
    | ref _ => exact False.elim hy
    | int _ => exact And.intro True.intro ha

theorem readCopy_safe {env : Env} {Γ : Ctx} {i : Nat} {t : Ty} (ha : Agree env Γ)
    (hi : nth Γ i = some (t, .alive)) (hc : t ≠ .box) : Safe t Γ (readCopy env i) := by
  obtain ⟨s, hs, hslot⟩ := ha.slot i t .alive hi
  unfold SlotOk at hslot
  obtain ⟨v, hsv, hv⟩ := hslot True.intro
  subst hsv
  have hle : i ≤ Γ.length := Nat.le_of_lt (nth_lt hi)
  simp only [readCopy, hs]
  cases v with
  | box _ =>
    cases t with
    | box => exact absurd rfl hc
    | int => exact False.elim hv
    | bool => exact False.elim hv
    | ref => exact False.elim hv
  | int _ => exact And.intro (valOk_mono hv hle) ha
  | bool _ => exact And.intro (valOk_mono hv hle) ha
  | ref _ => exact And.intro (valOk_mono hv hle) ha

theorem moveOut_safe {env : Env} {Γ : Ctx} {i : Nat} (ha : Agree env Γ)
    (hi : nth Γ i = some (.box, .alive)) : Safe .box (setNth Γ i (.box, .dead)) (moveOut env i) := by
  obtain ⟨s, hs, hslot⟩ := ha.slot i .box .alive hi
  unfold SlotOk at hslot
  obtain ⟨v, hsv, hv⟩ := hslot True.intro
  subst hsv
  simp only [moveOut, hs]
  cases v with
  | box _ => exact And.intro True.intro (agree_kill ha hi)
  | int _ => exact False.elim hv
  | bool _ => exact False.elim hv
  | ref _ => exact False.elim hv

theorem peekAt_safe {env : Env} {Γ : Ctx} {i : Nat} {st : St} (ha : Agree env Γ)
    (hi : nth Γ i = some (.box, st)) (hl : Live st) : Safe .int Γ (peekAt env i) := by
  obtain ⟨s, hs, hslot⟩ := ha.slot i .box st hi
  unfold SlotOk at hslot
  obtain ⟨v, hsv, hv⟩ := hslot hl
  subst hsv
  simp only [peekAt, hs]
  cases v with
  | box _ => exact And.intro True.intro ha
  | int _ => exact False.elim hv
  | bool _ => exact False.elim hv
  | ref _ => exact False.elim hv

/-! ## 主定理 -/

/-- 型安全性・所有権・借用の期間: 型が付く式を、文脈と合う環境で評価した結果は Safe -/
theorem eval_safe (fs : List Fn) (hp : ProgOk fs) :
    ∀ (k : Nat) (env : Env) (Γ : Ctx) (e : Expr) (t : Ty) (Γ' : Ctx),
      Agree env Γ → HasTy fs Γ e t Γ' → Safe t Γ' (eval fs k env e) := by
  intro k
  induction k with
  | zero =>
    intro env Γ e t Γ' _ _
    cases e <;> exact True.intro
  | succ k ih =>
    intro env Γ e t Γ' hag h
    cases h with
    | int _ _ => exact And.intro True.intro hag
    | bool _ _ => exact And.intro True.intro hag
    | var _ _ _ hi hc =>
      simp only [eval]
      exact readCopy_safe hag hi hc
    | move _ _ hi =>
      simp only [eval]
      exact moveOut_safe hag hi
    | peek _ _ _ hi hl =>
      simp only [eval]
      exact peekAt_safe hag hi hl
    | deref _ _ e he =>
      simp only [eval]
      apply bind_safe (ih env Γ e .ref Γ' hag he)
      intro v env1 hv ha1
      cases v with
      | ref j =>
        have hv' : ∃ k : Nat, nth Γ' j = some (Ty.box, St.frozen k) ∧ k ≤ Γ'.length := hv
        obtain ⟨kk, hk, _⟩ := hv'
        exact peekAt_safe ha1 hk True.intro
      | int _ => exact False.elim hv
      | bool _ => exact False.elim hv
      | box _ => exact False.elim hv
    | mkBox _ _ e he =>
      simp only [eval]
      apply bind_safe (ih env Γ e .int Γ' hag he)
      intro v env1 hv ha1
      cases v with
      | int _ => exact And.intro True.intro ha1
      | bool _ => exact False.elim hv
      | box _ => exact False.elim hv
      | ref _ => exact False.elim hv
    | unbox _ _ e he =>
      simp only [eval]
      apply bind_safe (ih env Γ e .box Γ' hag he)
      intro v env1 hv ha1
      cases v with
      | box _ => exact And.intro True.intro ha1
      | int _ => exact False.elim hv
      | bool _ => exact False.elim hv
      | ref _ => exact False.elim hv
    | add _ Γ1 _ a b ha hb =>
      simp only [eval]
      apply bind_safe (ih env Γ a .int Γ1 hag ha)
      intro x env1 hx ha1
      apply bind_safe (ih env1 Γ1 b .int Γ' ha1 hb)
      intro y env2 hy ha2
      exact addOp_safe hx hy ha2
    | lt _ Γ1 _ a b ha hb =>
      simp only [eval]
      apply bind_safe (ih env Γ a .int Γ1 hag ha)
      intro x env1 hx ha1
      apply bind_safe (ih env1 Γ1 b .int Γ' ha1 hb)
      intro y env2 hy ha2
      exact ltOp_safe hx hy ha2
    | ite _ Γ1 _ c a b _ hc ha hb =>
      simp only [eval]
      apply bind_safe (ih env Γ c .bool Γ1 hag hc)
      intro v env1 hv ha1
      cases v with
      | int _ => exact False.elim hv
      | box _ => exact False.elim hv
      | ref _ => exact False.elim hv
      | bool bv =>
        cases bv
        · exact ih env1 Γ1 b _ Γ' ha1 hb
        · exact ih env1 Γ1 a _ Γ' ha1 ha
    | letE _ Γ1 _ x body s _ p hx hbody hnr =>
      simp only [eval]
      apply bind_safe (ih env Γ x s Γ1 hag hx)
      intro v env1 hv ha1
      apply bind_safe (ih (env1 ++ [some v]) (Γ1 ++ [(s, .alive)]) body _ (Γ' ++ [p])
        (agree_push ha1 hv) hbody)
      intro w env2 hw ha2
      obtain ⟨env', x', he, hl⟩ := split_last env2 Γ'.length (by
        have h2 := ha2.len
        simp only [List.length_append, List.length_cons, List.length_nil] at h2
        omega)
      subst he
      simp only [init_append]
      exact And.intro (valOk_nonref hnr hw) (agree_pop ha2 hl)
    | borrow _ i body _ Γ2 p hi hbody hnr hfr =>
      simp only [eval]
      have hlen := hasTy_length hbody
      simp only [List.length_append, List.length_cons, List.length_nil, length_setNth] at hlen
      have hL : Γ2.length = Γ.length := by omega
      have hag1 : Agree (env ++ [some (Val.ref i)])
          (setNth Γ i (Ty.box, St.frozen Γ.length) ++ [(Ty.ref, St.alive)]) := by
        apply agree_push (agree_freeze hag hi)
        rw [length_setNth]
        exact Exists.intro Γ.length (And.intro (nth_setNth_eq (nth_lt hi)) (Nat.le_refl _))
      apply bind_safe (ih (env ++ [some (Val.ref i)]) _ body _ (Γ2 ++ [p]) hag1 hbody)
      intro w env2 hw ha2
      obtain ⟨env', x', he, hl⟩ := split_last env2 Γ2.length (by
        have h2 := ha2.len
        simp only [List.length_append, List.length_cons, List.length_nil] at h2
        omega)
      subst he
      simp only [init_append]
      have hfr2 : nth Γ2 i = some (Ty.box, St.frozen Γ2.length) := by
        rw [hL]
        exact hfr
      exact And.intro (valOk_nonref hnr hw) (agree_unfreeze (agree_pop ha2 hl) hfr2)
    | call _ _ f a fn hf hpr hrr ha =>
      simp only [eval]
      apply bind_safe (ih env Γ a fn.param Γ' hag ha)
      intro v env1 hv ha1
      simp only [hf]
      obtain ⟨out, hbody⟩ := hp f fn hf
      apply bind_safe (ih [some v] [(fn.param, .alive)] fn.body fn.ret out
        (agree_single (valOk_nonref hpr hv)) hbody)
      intro w _ hw _
      exact And.intro (valOk_nonref hrr hw) ha1

/-- ref の指す先が移動済みなら、読むと行き詰まる（上の定理が意味を持つことの確認） -/
theorem dangling_deref_stuck (env : Env) (j : Nat) (h : nth env j = some none) :
    derefOp (.ref j) env = .stuck := by
  show peekAt env j = .stuck
  simp only [peekAt, h]

/-- 型が付く式は、どれだけの燃料で評価しても行き詰まらない（移動済みの変数も宙に浮いた ref も使わない） -/
theorem no_dangling (fs : List Fn) (hp : ProgOk fs) (k : Nat) (env : Env) (Γ : Ctx)
    (e : Expr) (t : Ty) (Γ' : Ctx) (hag : Agree env Γ) (h : HasTy fs Γ e t Γ') :
    eval fs k env e ≠ .stuck := by
  intro hs
  have hsafe := eval_safe fs hp k env Γ e t Γ' hag h
  rw [hs] at hsafe
  exact hsafe

end JabberwockFormal.CoreBorrow
