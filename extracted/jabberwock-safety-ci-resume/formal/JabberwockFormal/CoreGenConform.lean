import JabberwockFormal.CoreGenFun

set_option autoImplicit false
set_option warningAsError true

namespace JabberwockFormal.CoreGenConform

open JabberwockFormal.CoreMutBorrow (nth setNth lastOpt init)
open JabberwockFormal.CoreGenFun

/-!
関数つきの模型（`CoreGenFun`）の型規則を計算する関数 `infer` と、それが型規則どおりであること（`infer_sound`）。
jwc の検査器・実行ファイルと小さな例で答えを比べる仕組み（`scripts/core_conform.py`・実行ファイル `core-conform`）の
模型の側（docs/core-type-safety.md）。

* `infer_sound`: `infer` が型を返せば、型規則 `HasTy` で型が付く。逆向き（型が付けば必ず返す）は証明しない。
  比べる仕組みは「模型で型が付く」を `infer` の答えで決めるので、使うのはこの向きだけ。
* `confFns`: 比べるときに使う関数の表（4 つ）。どの本体にも型が付く（`confFns_ok`）。
* `conform_no_stuck`: `infer` が型を返す式は、空の環境・空のヒープから評価して stuck にならない。

jwc と答えが合うことは証明しない（比べるのは例の上だけ）。
-/

/-- 型が等しいか -/
def tyEq : Ty → Ty → Bool
  | .int, .int => true
  | .bool, .bool => true
  | .box a, .box b => tyEq a b
  | .ref true a, .ref true b => tyEq a b
  | .ref false a, .ref false b => tyEq a b
  | _, _ => false

theorem tyEq_sound (a : Ty) : ∀ b : Ty, tyEq a b = true → a = b := by
  induction a with
  | int =>
    intro b h
    cases b
    all_goals first | rfl | cases h
  | bool =>
    intro b h
    cases b
    all_goals first | rfl | cases h
  | box a ih =>
    intro b h
    cases b
    case box b =>
      have h' : tyEq a b = true := h
      rw [ih b h']
    all_goals cases h
  | ref m a ih =>
    intro b h
    cases b
    case ref n b =>
      cases m <;> cases n
      · have h' : tyEq a b = true := h
        rw [ih b h']
      · cases h
      · cases h
      · have h' : tyEq a b = true := h
        rw [ih b h']
    all_goals cases h

/-- 箱を含まない型か -/
def isPlain : Ty → Bool
  | .int => true
  | .bool => true
  | .box _ => false
  | .ref _ t => isPlain t

theorem isPlain_sound : ∀ t : Ty, isPlain t = true → Plain t
  | .int, _ => True.intro
  | .bool, _ => True.intro
  | .box _, h => by
    have h' : false = true := h
    cases h'
  | .ref _ t, h => isPlain_sound t h

/-- 型の検査: 型と、評価の後の文脈を返す。型が付かなければ none -/
def infer (fns : List Fn) : Ctx → Expr → Option (Ty × Ctx)
  | Γ, .int _ => some (.int, Γ)
  | Γ, .bool _ => some (.bool, Γ)
  | Γ, .var i =>
    match nth Γ i with
    | some (t, true) => if isPlain t then some (t, Γ) else none
    | _ => none
  | Γ, .move i =>
    match nth Γ i with
    | some (.box t, true) => some (.box t, setNth Γ i (.box t, false))
    | _ => none
  | Γ, .peek i =>
    match nth Γ i with
    | some (.box t, true) => some (t, Γ)
    | _ => none
  | Γ, .addr m i =>
    match nth Γ i with
    | some (.box t, true) => some (.ref m t, Γ)
    | _ => none
  | Γ, .deref e =>
    match infer fns Γ e with
    | some (.ref _ t, Γ') => some (t, Γ')
    | _ => none
  | Γ, .store a b =>
    match infer fns Γ a with
    | some (.ref true t, Γ1) =>
      match infer fns Γ1 b with
      | some (t', Γ2) => if tyEq t' t then some (.int, Γ2) else none
      | none => none
    | _ => none
  | Γ, .mkBox t e =>
    match infer fns Γ e with
    | some (t', Γ') => if tyEq t' t then (if isPlain t then some (.box t, Γ') else none) else none
    | none => none
  | Γ, .add a b =>
    match infer fns Γ a with
    | some (.int, Γ1) =>
      match infer fns Γ1 b with
      | some (.int, Γ2) => some (.int, Γ2)
      | _ => none
    | _ => none
  | Γ, .letE x body =>
    match infer fns Γ x with
    | some (s, Γ1) =>
      match infer fns (Γ1 ++ [(s, true)]) body with
      | some (t, Γ3) =>
        match lastOpt Γ3 with
        | some _ => some (t, init Γ3)
        | none => none
      | none => none
    | none => none
  | Γ, .call f e =>
    match nth fns f with
    | some fn =>
      match infer fns Γ e with
      | some (t', Γ') => if tyEq t' fn.param then some (fn.ret, Γ') else none
      | none => none
    | none => none

theorem lastOpt_split {α : Type} : ∀ (l : List α) (p : α), lastOpt l = some p → l = init l ++ [p]
  | [], p, h => by
    have h' : (none : Option α) = some p := h
    cases h'
  | [a], p, h => by
    have h' : some a = some p := h
    cases h'
    rfl
  | a :: b :: l, p, h => by
    have h' : lastOpt (b :: l) = some p := h
    have ih := lastOpt_split (b :: l) p h'
    show a :: b :: l = (a :: init (b :: l)) ++ [p]
    rw [List.cons_append, ← ih]

theorem letE_of {fns : List Fn} {Γ Γ1 Γ3 : Ctx} {x body : Expr} {s t : Ty} {p : Ty × Bool}
    (hx : HasTy fns Γ x s Γ1) (hb : HasTy fns (Γ1 ++ [(s, true)]) body t Γ3)
    (hp : lastOpt Γ3 = some p) : HasTy fns Γ (.letE x body) t (init Γ3) := by
  have hs := lastOpt_split Γ3 p hp
  have hb' : HasTy fns (Γ1 ++ [(s, true)]) body t (init Γ3 ++ [p]) := by
    rw [← hs]
    exact hb
  exact HasTy.letE Γ Γ1 (init Γ3) x body s t p hx hb'

/-- `infer` が型を返せば、型規則で型が付く -/
theorem infer_sound (fns : List Fn) (e : Expr) :
    ∀ (Γ : Ctx) (t : Ty) (Γ' : Ctx), infer fns Γ e = some (t, Γ') → HasTy fns Γ e t Γ' := by
  induction e with
  | int _ =>
    intro Γ t Γ' h
    simp only [infer] at h
    cases h
    exact HasTy.int _ _
  | bool _ =>
    intro Γ t Γ' h
    simp only [infer] at h
    cases h
    exact HasTy.bool _ _
  | var _ =>
    intro Γ t Γ' h
    simp only [infer] at h
    split at h
    · split at h
      · cases h
        exact HasTy.var _ _ _ (by assumption) (isPlain_sound _ (by assumption))
      · cases h
    · cases h
  | move _ =>
    intro Γ t Γ' h
    simp only [infer] at h
    split at h
    · cases h
      exact HasTy.move _ _ _ (by assumption)
    · cases h
  | peek _ =>
    intro Γ t Γ' h
    simp only [infer] at h
    split at h
    · cases h
      exact HasTy.peek _ _ _ (by assumption)
    · cases h
  | addr _ _ =>
    intro Γ t Γ' h
    simp only [infer] at h
    split at h
    · cases h
      exact HasTy.addr _ _ _ _ (by assumption)
    · cases h
  | deref _ ih =>
    intro Γ t Γ' h
    simp only [infer] at h
    split at h
    · cases h
      exact HasTy.deref _ _ _ _ _ (ih _ _ _ (by assumption))
    · cases h
  | store _ _ iha ihb =>
    intro Γ t Γ' h
    simp only [infer] at h
    split at h
    · split at h
      · split at h
        · cases h
          have heq := tyEq_sound _ _ (by assumption)
          subst heq
          exact HasTy.store _ _ _ _ _ _ (iha _ _ _ (by assumption)) (ihb _ _ _ (by assumption))
        · cases h
      · cases h
    · cases h
  | mkBox _ _ ih =>
    intro Γ t Γ' h
    simp only [infer] at h
    split at h
    · split at h
      · split at h
        · cases h
          have heq := tyEq_sound _ _ (by assumption)
          subst heq
          exact HasTy.mkBox _ _ _ _ (ih _ _ _ (by assumption)) (isPlain_sound _ (by assumption))
        · cases h
      · cases h
    · cases h
  | add _ _ iha ihb =>
    intro Γ t Γ' h
    simp only [infer] at h
    split at h
    · split at h
      · cases h
        exact HasTy.add _ _ _ _ _ (iha _ _ _ (by assumption)) (ihb _ _ _ (by assumption))
      · cases h
    · cases h
  | letE _ _ ihx ihb =>
    intro Γ t Γ' h
    simp only [infer] at h
    split at h
    · split at h
      · split at h
        · cases h
          exact letE_of (ihx _ _ _ (by assumption)) (ihb _ _ _ (by assumption)) (by assumption)
        · cases h
      · cases h
    · cases h
  | call _ _ ih =>
    intro Γ t Γ' h
    simp only [infer] at h
    split at h
    · split at h
      · split at h
        · cases h
          have heq := tyEq_sound _ _ (by assumption)
          subst heq
          exact HasTy.call _ _ _ _ _ (by assumption) (ih _ _ _ (by assumption))
        · cases h
      · cases h
    · cases h

/-! ## 比べるときの関数の表 -/

/-- 0: `fn ref i64 f0(own i64 x0) { ref x0 }`（引数の箱の ref を返す。呼び出しの後は無効）
1: `fn i64 f1(ref mut i64 x0) { *x0 = 7; 0 }`
2: `fn i64 f2(own i64 x0) { *x0 }`
3: `fn own i64 f3(own i64 x0) { x0 }` -/
def confFns : List Fn :=
  [⟨.box .int, .ref false .int, .addr false 0⟩,
   ⟨.ref true .int, .int, .store (.var 0) (.int 7)⟩,
   ⟨.box .int, .int, .peek 0⟩,
   ⟨.box .int, .box .int, .move 0⟩]

theorem confFns_ok : FnsOk confFns := by
  unfold FnsOk
  intro f fn hf
  match f, hf with
  | 0, hf =>
    have h : some (⟨.box .int, .ref false .int, .addr false 0⟩ : Fn) = some fn := hf
    cases h
    exact ⟨true, infer_sound _ _ _ _ _ rfl⟩
  | 1, hf =>
    have h : some (⟨.ref true .int, .int, .store (.var 0) (.int 7)⟩ : Fn) = some fn := hf
    cases h
    exact ⟨true, infer_sound _ _ _ _ _ rfl⟩
  | 2, hf =>
    have h : some (⟨.box .int, .int, .peek 0⟩ : Fn) = some fn := hf
    cases h
    exact ⟨true, infer_sound _ _ _ _ _ rfl⟩
  | 3, hf =>
    have h : some (⟨.box .int, .box .int, .move 0⟩ : Fn) = some fn := hf
    cases h
    exact ⟨false, infer_sound _ _ _ _ _ rfl⟩
  | _ + 4, hf =>
    have h : (none : Option Fn) = some fn := hf
    cases h

theorem heapOk_nil : HeapOk [] := by
  intro l c o t w hl _
  have h : (none : Option Cell) = some c := hl
  cases h

/-- `infer` が型を返す式は、空の環境・空のヒープから、どの燃料で評価しても stuck にならない -/
theorem conform_no_stuck (e : Expr) (t : Ty) (Γ' : Ctx) (h : infer confFns [] e = some (t, Γ'))
    (k : Nat) : eval (fun _ => none) confFns k 0 [] [] e ≠ .stuck :=
  no_stuck pick_none_ok confFns_ok k 0 [] [] [] e t Γ' (agree_nil heapOk_nil)
    (infer_sound _ _ _ _ _ h)

/-- 例: `*f0(own 1)` に型が付き、評価は実行時の誤り -/
theorem conform_escape :
    infer confFns [] (.deref (.call 0 (.mkBox .int (.int 1)))) = some (.int, []) := rfl

end JabberwockFormal.CoreGenConform
