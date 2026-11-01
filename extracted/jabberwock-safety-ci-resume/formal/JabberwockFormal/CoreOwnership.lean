set_option autoImplicit false
set_option warningAsError true

namespace JabberwockFormal.CoreOwnership

/-!
型安全性の核に所有権を足した模型（docs/core-type-safety.md）。

`CoreTypeSafety` の核に、整数を 1 つ持つ所有の箱（`own` の値）を足す。

* `mkBox e`  : 整数から箱を作る
* `move i`   : 変数 i の箱を移動する（変数 i はそのあと死ぬ）
* `peek i`   : 変数 i の箱の中身を ref で読む（移動しない）
* `unbox e`  : 箱を受け取って中身を取り出す（箱は使い切る）
* `var i`    : 整数・真偽値の変数のコピー（箱はコピーできない）

実行時の環境は「値」か「移動済み（none）」の列。移動済みの変数を読む・移動する・覗く、
箱をコピーする、といった操作はすべて行き詰まり（stuck）になる。
型の規則は変数の生き死に（Bool）を追い、移動した変数をそのあと使う式には型を付けない。

定理 `eval_safe`: 型が付く式を、型と生き死にが合う環境で評価すると、結果は stuck ではなく、
値なら型どおりで、評価後の環境も評価後の型の文脈と合う。
系 `no_use_after_move`: 型が付くプログラムは移動済みの変数を使わない（使えば stuck だから）。

これは核の言語の模型の定理で、jwc・Rust 版の所有権検査、IR、LLVM、実行時ライブラリとの対応の証明ではない。
-/

inductive Ty where
  | int
  | bool
  | box
  deriving DecidableEq

inductive Expr where
  | int (n : Int)
  | bool (b : Bool)
  | var (i : Nat)
  | move (i : Nat)
  | peek (i : Nat)
  | mkBox (e : Expr)
  | unbox (e : Expr)
  | add (a b : Expr)
  | lt (a b : Expr)
  | ite (c a b : Expr)
  | letE (bound body : Expr)
  | call (f : Nat) (arg : Expr)

inductive Val where
  | int (n : Int)
  | bool (b : Bool)
  | box (n : Int)

def Val.ty : Val → Ty
  | .int _ => .int
  | .bool _ => .bool
  | .box _ => .box

/-- 実行時の環境: 各変数は値か、移動済み（none） -/
abbrev Env := List (Option Val)

/-- 型の文脈: 各変数の型と、まだ生きているか -/
abbrev Ctx := List (Ty × Bool)

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

/-- 文脈の i 番目を死んだことにする -/
def kill : Ctx → Nat → Ctx
  | [], _ => []
  | (t, _) :: Γ, 0 => (t, false) :: Γ
  | p :: Γ, i + 1 => p :: kill Γ i

/-- 型の規則。HasTy fs Γ e t Γ' は「文脈 Γ で e は型 t、評価したあとの文脈は Γ'」 -/
inductive HasTy (fs : List Fn) : Ctx → Expr → Ty → Ctx → Prop where
  | int (Γ : Ctx) (n : Int) : HasTy fs Γ (.int n) .int Γ
  | bool (Γ : Ctx) (b : Bool) : HasTy fs Γ (.bool b) .bool Γ
  | var (Γ : Ctx) (i : Nat) (t : Ty) (h : nth Γ i = some (t, true)) (hc : t ≠ .box) :
      HasTy fs Γ (.var i) t Γ
  | move (Γ : Ctx) (i : Nat) (h : nth Γ i = some (.box, true)) :
      HasTy fs Γ (.move i) .box (kill Γ i)
  | peek (Γ : Ctx) (i : Nat) (h : nth Γ i = some (.box, true)) :
      HasTy fs Γ (.peek i) .int Γ
  | mkBox (Γ Γ1 : Ctx) (e : Expr) (he : HasTy fs Γ e .int Γ1) :
      HasTy fs Γ (.mkBox e) .box Γ1
  | unbox (Γ Γ1 : Ctx) (e : Expr) (he : HasTy fs Γ e .box Γ1) :
      HasTy fs Γ (.unbox e) .int Γ1
  | add (Γ Γ1 Γ2 : Ctx) (a b : Expr) (ha : HasTy fs Γ a .int Γ1) (hb : HasTy fs Γ1 b .int Γ2) :
      HasTy fs Γ (.add a b) .int Γ2
  | lt (Γ Γ1 Γ2 : Ctx) (a b : Expr) (ha : HasTy fs Γ a .int Γ1) (hb : HasTy fs Γ1 b .int Γ2) :
      HasTy fs Γ (.lt a b) .bool Γ2
  | ite (Γ Γ1 Γ2 : Ctx) (c a b : Expr) (t : Ty) (hc : HasTy fs Γ c .bool Γ1)
      (ha : HasTy fs Γ1 a t Γ2) (hb : HasTy fs Γ1 b t Γ2) : HasTy fs Γ (.ite c a b) t Γ2
  | letE (Γ Γ1 Γ2 : Ctx) (x body : Expr) (s t : Ty) (p : Ty × Bool) (hx : HasTy fs Γ x s Γ1)
      (hbody : HasTy fs ((s, true) :: Γ1) body t (p :: Γ2)) : HasTy fs Γ (.letE x body) t Γ2
  | call (Γ Γ1 : Ctx) (f : Nat) (a : Expr) (fn : Fn) (hf : nth fs f = some fn)
      (ha : HasTy fs Γ a fn.param Γ1) : HasTy fs Γ (.call f a) fn.ret Γ1

/-- どの関数の本体も、引数（生きている）だけの文脈で戻り値の型を持つ -/
def ProgOk (fs : List Fn) : Prop :=
  ∀ (f : Nat) (fn : Fn), nth fs f = some fn →
    ∃ out : Ctx, HasTy fs [(fn.param, true)] fn.body fn.ret out

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

/-- コピーできるのは整数と真偽値だけ。箱のコピーは行き詰まり -/
def copyVal : Val → Env → Res
  | .int n, env => .ok (.int n) env
  | .bool b, env => .ok (.bool b) env
  | .box _, _ => .stuck

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

/-- 移動: 値を取り出し、変数を移動済みにする -/
def moveOut (env : Env) (i : Nat) : Res :=
  match nth env i with
  | some (some v) => moveVal v (setNth env i none)
  | _ => .stuck

def peekBox (env : Env) (i : Nat) : Res :=
  match nth env i with
  | some (some v) => peekVal v env
  | _ => .stuck

def mkBoxOp : Val → Env → Res
  | .int n, env => .ok (.box n) env
  | _, _ => .stuck

def unboxOp : Val → Env → Res
  | .box n, env => .ok (.int n) env
  | _, _ => .stuck

/-- let の本体のあと、その変数を環境から外す -/
def popEnv (w : Val) : Env → Res
  | _ :: rest => .ok w rest
  | [] => .stuck

def eval (fs : List Fn) : Nat → Env → Expr → Res
  | 0, _, _ => .fuel
  | _ + 1, env, .int n => .ok (.int n) env
  | _ + 1, env, .bool b => .ok (.bool b) env
  | _ + 1, env, .var i => readCopy env i
  | _ + 1, env, .move i => moveOut env i
  | _ + 1, env, .peek i => peekBox env i
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
    (eval fs k env x).bind fun v env1 => (eval fs k (some v :: env1) body).bind popEnv
  | k + 1, env, .call f a =>
    (eval fs k env a).bind fun v env1 =>
      match nth fs f with
      | some fn => (eval fs k [some v] fn.body).bind fun w _ => .ok w env1
      | none => .stuck

/-- 環境が文脈と合う: 生きている変数は、型どおりの値を持つ（死んだ変数は何でもよい） -/
inductive Agree : Env → Ctx → Prop where
  | nil : Agree [] []
  | cons (s : Option Val) (t : Ty) (alive : Bool) (env : Env) (Γ : Ctx)
      (hs : alive = true → ∃ v : Val, s = some v ∧ v.ty = t) (rest : Agree env Γ) :
      Agree (s :: env) ((t, alive) :: Γ)

def Safe (t : Ty) (Γ : Ctx) : Res → Prop
  | .ok v env => v.ty = t ∧ Agree env Γ
  | .err => True
  | .fuel => True
  | .stuck => False

theorem agree_nth {env : Env} {Γ : Ctx} (ha : Agree env Γ) :
    ∀ {i : Nat} {t : Ty}, nth Γ i = some (t, true) → ∃ v : Val, nth env i = some (some v) ∧ v.ty = t := by
  induction ha with
  | nil =>
    intro i t h
    have h' : (none : Option (Ty × Bool)) = some (t, true) := by
      cases i <;> exact h
    cases h'
  | cons s t' alive env Γ hs _ ih =>
    intro i t h
    cases i with
    | zero =>
      have h' : some (t', alive) = some (t, true) := h
      cases h'
      obtain ⟨v, hv, hvt⟩ := hs rfl
      exact ⟨v, congrArg some hv, hvt⟩
    | succ i => exact ih h

theorem agree_kill {env : Env} {Γ : Ctx} (ha : Agree env Γ) :
    ∀ i : Nat, Agree (setNth env i none) (kill Γ i) := by
  induction ha with
  | nil =>
    intro i
    cases i <;> exact Agree.nil
  | cons s t alive env Γ hs rest ih =>
    intro i
    cases i with
    | zero => exact Agree.cons none t false env Γ (fun h => by cases h) rest
    | succ i => exact Agree.cons s t alive (setNth env i none) (kill Γ i) hs (ih i)

theorem bind_safe {s t : Ty} {Γ1 Γ2 : Ctx} {r : Res} {f : Val → Env → Res} (hr : Safe s Γ1 r)
    (hf : ∀ (v : Val) (env : Env), v.ty = s → Agree env Γ1 → Safe t Γ2 (f v env)) :
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
  · exact And.intro rfl ha
  · exact True.intro

theorem addOp_safe {Γ : Ctx} {x y : Val} {env : Env} (hx : x.ty = .int) (hy : y.ty = .int)
    (ha : Agree env Γ) : Safe .int Γ (addOp x y env) := by
  cases x with
  | bool _ => cases hx
  | box _ => cases hx
  | int p =>
    cases y with
    | bool _ => cases hy
    | box _ => cases hy
    | int q => exact checked_safe (p + q) ha

theorem ltOp_safe {Γ : Ctx} {x y : Val} {env : Env} (hx : x.ty = .int) (hy : y.ty = .int)
    (ha : Agree env Γ) : Safe .bool Γ (ltOp x y env) := by
  cases x with
  | bool _ => cases hx
  | box _ => cases hx
  | int _ =>
    cases y with
    | bool _ => cases hy
    | box _ => cases hy
    | int _ => exact And.intro rfl ha

theorem readCopy_safe {env : Env} {Γ : Ctx} {i : Nat} {t : Ty} (ha : Agree env Γ)
    (h : nth Γ i = some (t, true)) (hc : t ≠ .box) : Safe t Γ (readCopy env i) := by
  obtain ⟨v, hv, ht⟩ := agree_nth ha h
  simp only [readCopy, hv]
  cases v with
  | int _ => exact And.intro ht ha
  | bool _ => exact And.intro ht ha
  | box _ => exact absurd ht.symm hc

theorem moveOut_safe {env : Env} {Γ : Ctx} {i : Nat} (ha : Agree env Γ)
    (h : nth Γ i = some (.box, true)) : Safe .box (kill Γ i) (moveOut env i) := by
  obtain ⟨v, hv, ht⟩ := agree_nth ha h
  simp only [moveOut, hv]
  cases v with
  | int _ => cases ht
  | bool _ => cases ht
  | box _ => exact And.intro rfl (agree_kill ha i)

theorem peekBox_safe {env : Env} {Γ : Ctx} {i : Nat} (ha : Agree env Γ)
    (h : nth Γ i = some (.box, true)) : Safe .int Γ (peekBox env i) := by
  obtain ⟨v, hv, ht⟩ := agree_nth ha h
  simp only [peekBox, hv]
  cases v with
  | int _ => cases ht
  | bool _ => cases ht
  | box _ => exact And.intro rfl ha

/-- 型安全性と所有権: 型が付く式を、文脈と合う環境で評価した結果は Safe -/
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
    | int _ _ => exact And.intro rfl hag
    | bool _ _ => exact And.intro rfl hag
    | var _ _ _ hi hc =>
      simp only [eval]
      exact readCopy_safe hag hi hc
    | move _ _ hi =>
      simp only [eval]
      exact moveOut_safe hag hi
    | peek _ _ hi =>
      simp only [eval]
      exact peekBox_safe hag hi
    | mkBox _ Γ1 e he =>
      simp only [eval]
      apply bind_safe (ih env Γ e .int Γ1 hag he)
      intro v env1 hv ha1
      cases v with
      | int _ => exact And.intro rfl ha1
      | bool _ => cases hv
      | box _ => cases hv
    | unbox _ Γ1 e he =>
      simp only [eval]
      apply bind_safe (ih env Γ e .box Γ1 hag he)
      intro v env1 hv ha1
      cases v with
      | int _ => cases hv
      | bool _ => cases hv
      | box _ => exact And.intro rfl ha1
    | add _ Γ1 Γ2 a b ha hb =>
      simp only [eval]
      apply bind_safe (ih env Γ a .int Γ1 hag ha)
      intro x env1 hx ha1
      apply bind_safe (ih env1 Γ1 b .int Γ2 ha1 hb)
      intro y env2 hy ha2
      exact addOp_safe hx hy ha2
    | lt _ Γ1 Γ2 a b ha hb =>
      simp only [eval]
      apply bind_safe (ih env Γ a .int Γ1 hag ha)
      intro x env1 hx ha1
      apply bind_safe (ih env1 Γ1 b .int Γ2 ha1 hb)
      intro y env2 hy ha2
      exact ltOp_safe hx hy ha2
    | ite _ Γ1 Γ2 c a b _ hc ha hb =>
      simp only [eval]
      apply bind_safe (ih env Γ c .bool Γ1 hag hc)
      intro v env1 hv ha1
      cases v with
      | int _ => cases hv
      | box _ => cases hv
      | bool bv =>
        cases bv
        · exact ih env1 Γ1 b _ Γ2 ha1 hb
        · exact ih env1 Γ1 a _ Γ2 ha1 ha
    | letE _ Γ1 Γ2 x body s _ p hx hbody =>
      simp only [eval]
      apply bind_safe (ih env Γ x s Γ1 hag hx)
      intro v env1 hv ha1
      apply bind_safe (ih (some v :: env1) ((s, true) :: Γ1) body _ (p :: Γ2)
        (Agree.cons (some v) s true env1 Γ1 (fun _ => ⟨v, rfl, hv⟩) ha1) hbody)
      intro w env2 hw ha2
      cases ha2 with
      | cons _ _ _ _ _ _ rest => exact And.intro hw rest
    | call _ Γ1 f a fn hf ha =>
      simp only [eval]
      apply bind_safe (ih env Γ a fn.param Γ1 hag ha)
      intro v env1 hv ha1
      simp only [hf]
      obtain ⟨out, hbody⟩ := hp f fn hf
      apply bind_safe (ih [some v] [(fn.param, true)] fn.body fn.ret out
        (Agree.cons (some v) fn.param true [] [] (fun _ => ⟨v, rfl, hv⟩) Agree.nil) hbody)
      intro w _ hw _
      exact And.intro hw ha1

/-- 移動済みの変数を読む・移動する・覗くと行き詰まる -/
theorem moved_access_stuck (env : Env) (i : Nat) (h : nth env i = some none) :
    readCopy env i = .stuck ∧ moveOut env i = .stuck ∧ peekBox env i = .stuck := by
  refine ⟨?_, ?_, ?_⟩
  · simp only [readCopy, h]
  · simp only [moveOut, h]
  · simp only [peekBox, h]

/-- 型が付く式は、どれだけの燃料で評価しても行き詰まらない（移動済みの変数を使わない・箱をコピーしない） -/
theorem no_use_after_move (fs : List Fn) (hp : ProgOk fs) (k : Nat) (env : Env) (Γ : Ctx)
    (e : Expr) (t : Ty) (Γ' : Ctx) (hag : Agree env Γ) (h : HasTy fs Γ e t Γ') :
    eval fs k env e ≠ .stuck := by
  intro hs
  have hsafe := eval_safe fs hp k env Γ e t Γ' hag h
  rw [hs] at hsafe
  exact hsafe

end JabberwockFormal.CoreOwnership
