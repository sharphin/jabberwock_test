set_option autoImplicit false
set_option warningAsError true

namespace JabberwockFormal.CoreTypeSafety

/-!
型安全性の核（docs/core-type-safety.md）。

jabberwock の小さな核の言語（i64 の整数・真偽値・変数・四則のうち足し算・引き算・割り算・比べる・if・let・
引数 1 つのトップレベルの関数の呼び出し）について、型が付くプログラムは「行き詰まらない」ことを証明する。
評価は燃料（呼べる深さ）つきの big-step で、結果は次の 4 つ:

* `ok v`   : 値 v（型どおり）
* `err`    : 仕様で決めた実行時エラー（i64 のあふれ・0 で割る）
* `fuel`   : 燃料切れ（止まらないかもしれない計算を、ここで打ち切った）
* `stuck`  : 行き詰まり（型の合わない値への操作・ない変数・ない関数）

定理 `eval_safe`: 型が付く式を、型どおりの値を持つ環境で評価すると、結果は stuck ではなく、
ok なら値の型は式の型と同じ。燃料はいくつでもよい。

これは核の言語の模型の定理で、jwc・Rust 版のコンパイラ、IR、LLVM、実行時ライブラリとの対応の証明ではない。
-/

inductive Ty where
  | int
  | bool
  deriving DecidableEq

inductive Expr where
  | int (n : Int)
  | bool (b : Bool)
  | var (i : Nat)
  | add (a b : Expr)
  | sub (a b : Expr)
  | div (a b : Expr)
  | lt (a b : Expr)
  | eq (a b : Expr)
  | ite (c a b : Expr)
  | letE (bound body : Expr)
  | call (f : Nat) (arg : Expr)

inductive Val where
  | int (n : Int)
  | bool (b : Bool)

def Val.ty : Val → Ty
  | .int _ => .int
  | .bool _ => .bool

/-- トップレベルの関数（引数 1 つ） -/
structure Fn where
  param : Ty
  ret : Ty
  body : Expr

/-- リストの i 番目 -/
def nth {α : Type} : List α → Nat → Option α
  | [], _ => none
  | a :: _, 0 => some a
  | _ :: l, i + 1 => nth l i

theorem nth_map {α β : Type} (f : α → β) (l : List α) (i : Nat) :
    nth (l.map f) i = (nth l i).map f := by
  induction l generalizing i with
  | nil => rfl
  | cons a l ih =>
    cases i with
    | zero => rfl
    | succ i => exact ih i

/-- 型の規則。Γ は変数の型（0 番がいちばん内側の let か関数の引数） -/
inductive HasTy (fs : List Fn) : List Ty → Expr → Ty → Prop where
  | int (Γ : List Ty) (n : Int) : HasTy fs Γ (.int n) .int
  | bool (Γ : List Ty) (b : Bool) : HasTy fs Γ (.bool b) .bool
  | var (Γ : List Ty) (i : Nat) (t : Ty) (h : nth Γ i = some t) : HasTy fs Γ (.var i) t
  | add (Γ : List Ty) (a b : Expr) (ha : HasTy fs Γ a .int) (hb : HasTy fs Γ b .int) :
      HasTy fs Γ (.add a b) .int
  | sub (Γ : List Ty) (a b : Expr) (ha : HasTy fs Γ a .int) (hb : HasTy fs Γ b .int) :
      HasTy fs Γ (.sub a b) .int
  | div (Γ : List Ty) (a b : Expr) (ha : HasTy fs Γ a .int) (hb : HasTy fs Γ b .int) :
      HasTy fs Γ (.div a b) .int
  | lt (Γ : List Ty) (a b : Expr) (ha : HasTy fs Γ a .int) (hb : HasTy fs Γ b .int) :
      HasTy fs Γ (.lt a b) .bool
  | eq (Γ : List Ty) (a b : Expr) (s : Ty) (ha : HasTy fs Γ a s) (hb : HasTy fs Γ b s) :
      HasTy fs Γ (.eq a b) .bool
  | ite (Γ : List Ty) (c a b : Expr) (t : Ty) (hc : HasTy fs Γ c .bool)
      (ha : HasTy fs Γ a t) (hb : HasTy fs Γ b t) : HasTy fs Γ (.ite c a b) t
  | letE (Γ : List Ty) (x body : Expr) (s t : Ty) (hx : HasTy fs Γ x s)
      (hbody : HasTy fs (s :: Γ) body t) : HasTy fs Γ (.letE x body) t
  | call (Γ : List Ty) (f : Nat) (a : Expr) (fn : Fn) (hf : nth fs f = some fn)
      (ha : HasTy fs Γ a fn.param) : HasTy fs Γ (.call f a) fn.ret

/-- プログラム（関数の表）の型が合っている: どの関数の本体も、引数の型の下で戻り値の型を持つ -/
def ProgOk (fs : List Fn) : Prop :=
  ∀ (f : Nat) (fn : Fn), nth fs f = some fn → HasTy fs [fn.param] fn.body fn.ret

inductive Res where
  | ok (v : Val)
  | err
  | fuel
  | stuck

def Res.bind : Res → (Val → Res) → Res
  | .ok v, f => f v
  | .err, _ => .err
  | .fuel, _ => .fuel
  | .stuck, _ => .stuck

/-- i64 に入るなら値、入らなければ実行時エラー（あふれ） -/
def checked (z : Int) : Res :=
  if -(2 : Int) ^ 63 ≤ z ∧ z < (2 : Int) ^ 63 then .ok (.int z) else .err

/-- 整数どうしの演算。整数でなければ行き詰まり -/
def intOp (g : Int → Int → Res) : Val → Val → Res
  | .int x, .int y => g x y
  | _, _ => .stuck

/-- 割り算: 0 で割れば実行時エラー。商は 0 の方へ切り捨て（i64 の最小値 / -1 はあふれ） -/
def divInt (x y : Int) : Res :=
  if y = 0 then .err else checked (x.tdiv y)

/-- 同じ型の値どうしの == -/
def eqV : Val → Val → Res
  | .int x, .int y => .ok (.bool (x == y))
  | .bool x, .bool y => .ok (.bool (x == y))
  | _, _ => .stuck

def lookup (env : List Val) (i : Nat) : Res :=
  match nth env i with
  | some v => .ok v
  | none => .stuck

/-- 燃料 k つきの評価。部分式・呼んだ関数の本体は燃料 k - 1 で評価する -/
def eval (fs : List Fn) : Nat → List Val → Expr → Res
  | 0, _, _ => .fuel
  | _ + 1, _, .int n => .ok (.int n)
  | _ + 1, _, .bool b => .ok (.bool b)
  | _ + 1, env, .var i => lookup env i
  | k + 1, env, .add a b =>
    (eval fs k env a).bind fun x => (eval fs k env b).bind fun y => intOp (fun p q => checked (p + q)) x y
  | k + 1, env, .sub a b =>
    (eval fs k env a).bind fun x => (eval fs k env b).bind fun y => intOp (fun p q => checked (p - q)) x y
  | k + 1, env, .div a b =>
    (eval fs k env a).bind fun x => (eval fs k env b).bind fun y => intOp divInt x y
  | k + 1, env, .lt a b =>
    (eval fs k env a).bind fun x => (eval fs k env b).bind fun y =>
      intOp (fun p q => .ok (.bool (decide (p < q)))) x y
  | k + 1, env, .eq a b =>
    (eval fs k env a).bind fun x => (eval fs k env b).bind fun y => eqV x y
  | k + 1, env, .ite c a b =>
    (eval fs k env c).bind fun v =>
      match v with
      | .bool true => eval fs k env a
      | .bool false => eval fs k env b
      | .int _ => .stuck
  | k + 1, env, .letE x body =>
    (eval fs k env x).bind fun v => eval fs k (v :: env) body
  | k + 1, env, .call f a =>
    (eval fs k env a).bind fun v =>
      match nth fs f with
      | some fn => eval fs k [v] fn.body
      | none => .stuck

/-- 結果が型 t について安全: 行き詰まりではなく、値なら型が t -/
def Safe (t : Ty) : Res → Prop
  | .ok v => v.ty = t
  | .err => True
  | .fuel => True
  | .stuck => False

theorem bind_safe {s t : Ty} {r : Res} {f : Val → Res} (hr : Safe s r)
    (hf : ∀ v : Val, v.ty = s → Safe t (f v)) : Safe t (r.bind f) := by
  cases r with
  | ok v => exact hf v hr
  | err => exact True.intro
  | fuel => exact True.intro
  | stuck => exact False.elim hr

theorem checked_safe (z : Int) : Safe .int (checked z) := by
  unfold checked
  split
  · rfl
  · exact True.intro

theorem intOp_safe {t : Ty} {g : Int → Int → Res} (hg : ∀ x y : Int, Safe t (g x y))
    {x y : Val} (hx : x.ty = .int) (hy : y.ty = .int) : Safe t (intOp g x y) := by
  cases x with
  | bool _ => cases hx
  | int a =>
    cases y with
    | bool _ => cases hy
    | int c => exact hg a c

theorem divInt_safe (x y : Int) : Safe .int (divInt x y) := by
  unfold divInt
  split
  · exact True.intro
  · exact checked_safe _

theorem eqV_safe {s : Ty} {x y : Val} (hx : x.ty = s) (hy : y.ty = s) : Safe .bool (eqV x y) := by
  cases x with
  | int _ =>
    cases y with
    | int _ => rfl
    | bool _ =>
      simp only [Val.ty] at hx hy
      subst hx
      cases hy
  | bool _ =>
    cases y with
    | int _ =>
      simp only [Val.ty] at hx hy
      subst hx
      cases hy
    | bool _ => rfl

/-- 型安全性（行き詰まらない・型を保つ）: 型が付く式を、型どおりの環境で評価した結果は Safe -/
theorem eval_safe (fs : List Fn) (hp : ProgOk fs) :
    ∀ (k : Nat) (env : List Val) (Γ : List Ty) (e : Expr) (t : Ty),
      env.map Val.ty = Γ → HasTy fs Γ e t → Safe t (eval fs k env e) := by
  intro k
  induction k with
  | zero =>
    intro env Γ e t _ _
    cases e <;> exact True.intro
  | succ k ih =>
    intro env Γ e t henv h
    cases h with
    | int _ _ => rfl
    | bool _ _ => rfl
    | var _ i _ hi =>
      subst henv
      rw [nth_map] at hi
      cases hv : nth env i with
      | none =>
        rw [hv] at hi
        cases hi
      | some v =>
        rw [hv] at hi
        have hvt : v.ty = t := Option.some.inj hi
        simp only [eval, lookup, hv]
        exact hvt
    | add _ a b ha hb =>
      simp only [eval]
      apply bind_safe (ih env Γ a .int henv ha)
      intro x hx
      apply bind_safe (ih env Γ b .int henv hb)
      intro y hy
      exact intOp_safe (fun p q => checked_safe (p + q)) hx hy
    | sub _ a b ha hb =>
      simp only [eval]
      apply bind_safe (ih env Γ a .int henv ha)
      intro x hx
      apply bind_safe (ih env Γ b .int henv hb)
      intro y hy
      exact intOp_safe (fun p q => checked_safe (p - q)) hx hy
    | div _ a b ha hb =>
      simp only [eval]
      apply bind_safe (ih env Γ a .int henv ha)
      intro x hx
      apply bind_safe (ih env Γ b .int henv hb)
      intro y hy
      exact intOp_safe divInt_safe hx hy
    | lt _ a b ha hb =>
      simp only [eval]
      apply bind_safe (ih env Γ a .int henv ha)
      intro x hx
      apply bind_safe (ih env Γ b .int henv hb)
      intro y hy
      exact intOp_safe (t := .bool) (g := fun p q => .ok (.bool (decide (p < q)))) (fun _ _ => rfl) hx hy
    | eq _ a b s ha hb =>
      simp only [eval]
      apply bind_safe (ih env Γ a s henv ha)
      intro x hx
      apply bind_safe (ih env Γ b s henv hb)
      intro y hy
      exact eqV_safe hx hy
    | ite _ c a b _ hc ha hb =>
      simp only [eval]
      apply bind_safe (ih env Γ c .bool henv hc)
      intro v hv
      cases v with
      | int _ => cases hv
      | bool bv =>
        cases bv
        · exact ih env Γ b _ henv hb
        · exact ih env Γ a _ henv ha
    | letE _ x body s _ hx hbody =>
      simp only [eval]
      apply bind_safe (ih env Γ x s henv hx)
      intro v hv
      exact ih (v :: env) (s :: Γ) body _ (by simp [hv, henv]) hbody
    | call _ f a fn hf ha =>
      simp only [eval]
      apply bind_safe (ih env Γ a fn.param henv ha)
      intro v hv
      simp only [hf]
      exact ih [v] [fn.param] fn.body fn.ret (by simp [hv]) (hp f fn hf)

/-- 型が付く式は、どれだけの燃料で評価しても行き詰まらない -/
theorem never_stuck (fs : List Fn) (hp : ProgOk fs) (k : Nat) (env : List Val) (Γ : List Ty)
    (e : Expr) (t : Ty) (henv : env.map Val.ty = Γ) (h : HasTy fs Γ e t) :
    eval fs k env e ≠ .stuck := by
  intro hs
  have hsafe := eval_safe fs hp k env Γ e t henv h
  rw [hs] at hsafe
  exact hsafe

/-- 閉じた式（main の本体など）: 結果が値なら、その型は式の型 -/
theorem closed_value_has_type (fs : List Fn) (hp : ProgOk fs) (k : Nat) (e : Expr) (t : Ty)
    (h : HasTy fs [] e t) (v : Val) (hv : eval fs k [] e = .ok v) : v.ty = t := by
  have hsafe := eval_safe fs hp k [] [] e t rfl h
  rw [hv] at hsafe
  exact hsafe

end JabberwockFormal.CoreTypeSafety
