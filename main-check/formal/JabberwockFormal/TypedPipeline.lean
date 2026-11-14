import JabberwockFormal.ScopedGoals

set_option autoImplicit false
set_option warningAsError true

namespace JabberwockFormal.TypedPipeline

/-!
A deliberately small, intrinsically typed source and two compiler stages.
The intermediate pass performs constant folding; the back end changes
representation to a target expression. Both stages preserve evaluation.
This is a G1/G3 specification fragment, NOT a correspondence proof for
jwc, the Rust compiler, LLVM, or the runtime.
-/

inductive Ty where
  | nat
  | bool

inductive Expr : Ty → Type where
  | number (n : Nat) : Expr .nat
  | truth (b : Bool) : Expr .bool
  | add (a b : Expr .nat) : Expr .nat
  | equal (a b : Expr .nat) : Expr .bool
  | choose {t : Ty} (condition : Expr .bool) (yes no : Expr t) : Expr t

abbrev Value : Ty → Type
  | .nat => Nat
  | .bool => Bool

def eval : {t : Ty} → Expr t → Value t
  | _, .number n => n
  | _, .truth b => b
  | _, .add a b => eval (t := .nat) a + eval (t := .nat) b
  | _, .equal a b => decide (eval (t := .nat) a = eval (t := .nat) b)
  | _, .choose condition yes no =>
      if eval (t := .bool) condition then eval yes else eval no

/-- A separate IR representation retains its result type in the constructor. -/
inductive IR : Ty → Type where
  | integer (n : Nat) : IR .nat
  | boolean (b : Bool) : IR .bool
  | sum (a b : IR .nat) : IR .nat
  | eq (a b : IR .nat) : IR .bool
  | select {t : Ty} (condition : IR .bool) (yes no : IR t) : IR t

def irEval : {t : Ty} → IR t → Value t
  | _, .integer n => n
  | _, .boolean b => b
  | _, .sum a b => irEval (t := .nat) a + irEval (t := .nat) b
  | _, .eq a b => decide (irEval (t := .nat) a = irEval (t := .nat) b)
  | _, .select condition yes no =>
      if irEval (t := .bool) condition then irEval yes else irEval no

def lower : {t : Ty} → Expr t → IR t
  | _, .number n => .integer n
  | _, .truth b => .boolean b
  | _, .add a b => .sum (lower a) (lower b)
  | _, .equal a b => .eq (lower a) (lower b)
  | _, .choose condition yes no =>
      .select (lower condition) (lower yes) (lower no)

/-- Type preservation is enforced by the indexed lowering signature. -/
theorem lower_preserves_result_type {t : Ty} (e : Expr t) :
    ∃ result : IR t, result = lower e := by
  exact ⟨lower e, rfl⟩

/-- Behavioral preservation for every expression in this small typed language. -/
theorem lower_preserves_evaluation {t : Ty} (e : Expr t) :
    irEval (lower e) = eval e := by
  induction e with
  | number _ => rfl
  | truth _ => rfl
  | add a b ha hb => simp [lower, irEval, eval, ha, hb]
  | equal a b ha hb => simp [lower, irEval, eval, ha, hb]
  | choose condition yes no hc hy hn =>
      simp [lower, irEval, eval, hc, hy, hn]


/-- A typed IR rewrite that folds literal addition, literal equality and
    literal conditionals, leaving nonconstant operations intact. -/
def fold : {t : Ty} → IR t → IR t
  | _, .integer n => .integer n
  | _, .boolean b => .boolean b
  | _, .sum a b =>
      let a' := fold a
      let b' := fold b
      match a', b' with
      | .integer x, .integer y => .integer (x + y)
      | x, y => .sum x y
  | _, .eq a b =>
      let a' := fold a
      let b' := fold b
      match a', b' with
      | .integer x, .integer y => .boolean (decide (x = y))
      | x, y => .eq x y
  | _, .select c yes no =>
      let c' := fold c
      let yes' := fold yes
      let no' := fold no
      match c' with
      | .boolean true => yes'
      | .boolean false => no'
      | other => .select other yes' no'

/-- Evaluation of the literal-add rewrite, including the fallback case. -/
private theorem sum_step_sound (a b : IR .nat) :
    irEval (match a, b with
      | .integer x, .integer y => IR.integer (x + y)
      | x, y => IR.sum x y) = irEval (t := .nat) a + irEval (t := .nat) b := by
  cases a <;> cases b <;> rfl

/-- Evaluation of the literal-equality rewrite, including the fallback case. -/
private theorem eq_step_sound (a b : IR .nat) :
    irEval (match a, b with
      | .integer x, .integer y => IR.boolean (decide (x = y))
      | x, y => IR.eq x y) = decide (irEval (t := .nat) a = irEval (t := .nat) b) := by
  cases a <;> cases b <;> rfl

/-- Evaluation of constant branch selection for any target result type. -/
private theorem select_step_sound {t : Ty} (c : IR .bool) (yes no : IR t) :
    irEval (match c with
      | .boolean true => yes
      | .boolean false => no
      | other => IR.select other yes no) =
    (if irEval (t := .bool) c then irEval yes else irEval no) := by
  cases c with
  | boolean b => cases b <;> rfl
  | eq a b => rfl
  | select c a b => rfl

/-- General semantic preservation for every well-typed term in the modeled IR.
    Unlike the earlier literal-only lemmas, this covers arbitrary nesting of
    arithmetic, equality and conditional rewrites. It still does not certify
    the Rust or jwc optimizer implementations. -/
theorem fold_preserves_evaluation {t : Ty} (ir : IR t) :
    irEval (fold ir) = irEval ir := by
  induction ir with
  | integer _ => rfl
  | boolean _ => rfl
  | sum a b ha hb =>
      change irEval (match fold a, fold b with
        | .integer x, .integer y => IR.integer (x + y)
        | x, y => IR.sum x y) = irEval (t := .nat) a + irEval (t := .nat) b
      rw [sum_step_sound, ha, hb]
  | eq a b ha hb =>
      change irEval (match fold a, fold b with
        | .integer x, .integer y => IR.boolean (decide (x = y))
        | x, y => IR.eq x y) = decide (irEval (t := .nat) a = irEval (t := .nat) b)
      rw [eq_step_sound, ha, hb]
  | select c yes no hc hy hn =>
      change irEval (match fold c with
        | .boolean true => fold yes
        | .boolean false => fold no
        | other => IR.select other (fold yes) (fold no)) =
        (if irEval (t := .bool) c then irEval yes else irEval no)
      rw [select_step_sound, hc, hy, hn]

/-- The optimizer cannot change the result type: its return type is IR t. -/
theorem fold_preserves_result_type {t : Ty} (ir : IR t) :
    ∃ optimized : IR t, optimized = fold ir := by
  exact ⟨fold ir, rfl⟩

/-- A standalone preservation obligation for each optimization category.
    No implementation correspondence with the real jwc/Rust passes is claimed. -/
theorem fold_constant_addition (a b : Nat) :
    irEval (fold (IR.sum (.integer a) (.integer b))) = a + b := by
  rfl

theorem fold_constant_equality (a b : Nat) :
    irEval (fold (IR.eq (.integer a) (.integer b))) = decide (a = b) := by
  rfl

theorem fold_constant_conditional {t : Ty} (yes no : IR t) :
    irEval (fold (IR.select (.boolean true) yes no)) =
      irEval (fold yes) := by
  rfl


/-- A third, independent typed expression representation for the modeled
    backend. This is not LLVM IR and does not claim LLVM refinement. -/
inductive Target : Ty → Type where
  | constantNat (n : Nat) : Target .nat
  | constantBool (b : Bool) : Target .bool
  | plus (a b : Target .nat) : Target .nat
  | compare (a b : Target .nat) : Target .bool
  | branch {t : Ty} (c : Target .bool) (yes no : Target t) : Target t

def targetEval : {t : Ty} → Target t → Value t
  | _, .constantNat n => n
  | _, .constantBool b => b
  | _, .plus a b => targetEval (t := .nat) a + targetEval (t := .nat) b
  | _, .compare a b => decide (targetEval (t := .nat) a = targetEval (t := .nat) b)
  | _, .branch c yes no =>
      if targetEval (t := .bool) c then targetEval yes else targetEval no

/-- The modeled backend preserves its result type by construction. -/
def emit : {t : Ty} → IR t → Target t
  | _, .integer n => .constantNat n
  | _, .boolean b => .constantBool b
  | _, .sum a b => .plus (emit a) (emit b)
  | _, .eq a b => .compare (emit a) (emit b)
  | _, .select c yes no => .branch (emit c) (emit yes) (emit no)

/-- Each modeled IR constructor has the same observation in target form. -/
theorem emit_preserves_evaluation {t : Ty} (ir : IR t) :
    targetEval (emit ir) = irEval ir := by
  induction ir with
  | integer _ => rfl
  | boolean _ => rfl
  | sum a b ha hb => simp [emit, targetEval, irEval, ha, hb]
  | eq a b ha hb => simp [emit, targetEval, irEval, ha, hb]
  | select c yes no hc hy hn =>
      simp [emit, targetEval, irEval, hc, hy, hn]

/-- Composition from typed source to a third target representation.
    This theorem is limited to the modeled Nat/Bool expression language. -/
theorem source_to_target_preserves_evaluation {t : Ty} (e : Expr t) :
    targetEval (emit (lower e)) = eval e := by
  rw [emit_preserves_evaluation, lower_preserves_evaluation]

/-- Composition including recursive optimization for every modeled expression. -/
theorem optimized_source_to_target_preserves_evaluation {t : Ty} (e : Expr t) :
    targetEval (emit (fold (lower e))) = eval e := by
  rw [emit_preserves_evaluation, fold_preserves_evaluation,
      lower_preserves_evaluation]

/-- Optimization stage output always remains a well-typed target term. -/
theorem folded_ir_emits_well_typed_target {t : Ty} (ir : IR t) :
    ∃ result : Target t, result = emit (fold ir) := by
  exact ⟨emit (fold ir), rfl⟩

/-- The modeled constant-addition optimization survives the whole pipeline,
    including the separate target representation. -/
theorem constant_addition_pipeline (a b : Nat) :
    targetEval (emit (fold (lower (Expr.add (.number a) (.number b))))) =
      a + b := by
  rfl

/-- Constant comparisons retain the Boolean result through all three stages. -/
theorem constant_equality_pipeline (a b : Nat) :
    targetEval (emit (fold (lower (Expr.equal (.number a) (.number b))))) =
      decide (a = b) := by
  rfl

/-- A constant branch can be eliminated without changing the modeled output,
    including when the selected branch itself requires optimization. -/
theorem constant_true_branch_pipeline {t : Ty} (yes no : Expr t) :
    targetEval (emit (fold (lower (Expr.choose (.truth true) yes no)))) =
      targetEval (emit (fold (lower yes))) := by
  rfl

theorem constant_false_branch_pipeline {t : Ty} (yes no : Expr t) :
    targetEval (emit (fold (lower (Expr.choose (.truth false) yes no)))) =
      targetEval (emit (fold (lower no))) := by
  rfl

end JabberwockFormal.TypedPipeline
