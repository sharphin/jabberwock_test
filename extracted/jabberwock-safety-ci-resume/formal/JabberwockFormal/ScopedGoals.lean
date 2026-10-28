import JabberwockFormal.Generation
import JabberwockFormal.Ownership
import JabberwockFormal.EmittedGate

set_option autoImplicit false
set_option warningAsError true

namespace JabberwockFormal.ScopedGoals

/-!
This module deliberately proves all five *shapes* of obligations for a
strictly delimited reference-and-arithmetic kernel. It does NOT establish
those properties for Jabberwock's complete AST/IR/LLVM/runtime.
The implementation refinement relation is separately open.
-/

/-- A typed source fragment: integers, addition and generation-checked reads.
    Typing is enforced by constructors, not an assumed external checker. -/
inductive Ty where
  | number
  deriving DecidableEq, Repr

inductive Expr : Ty → Type where
  | literal (value : Nat) : Expr .number
  | plus (left right : Expr .number) : Expr .number
  | read (saved current : BitVec 64) (payload : Nat) : Expr .number

/-- An independently represented target instruction tree for this fragment. -/
inductive Instr where
  | constant (value : Nat)
  | add (left right : Instr)
  | checkedLoad (saved current : BitVec 64) (payload : Nat)

/-- Observable success/failure, not a model of memory allocation or LLVM UB. -/
inductive Fault where
  | dangling
  deriving DecidableEq, Repr

def sourceEval : Expr .number → Except Fault Nat
  | .literal n => .ok n
  | .plus a b => do
      let x ← sourceEval a
      let y ← sourceEval b
      pure (x + y)
  | .read saved current payload =>
      if JabberwockFormal.EmittedGate.llvmContinues saved current then .ok payload
      else .error .dangling

def lower : Expr .number → Instr
  | .literal n => .constant n
  | .plus a b => .add (lower a) (lower b)
  | .read saved current payload => .checkedLoad saved current payload

def targetEval : Instr → Except Fault Nat
  | .constant n => .ok n
  | .add a b => do
      let x ← targetEval a
      let y ← targetEval b
      pure (x + y)
  | .checkedLoad saved current payload =>
      if JabberwockFormal.EmittedGate.llvmContinues saved current then .ok payload
      else .error .dangling

/-- G1, restricted: lowering preserves value or the dangling-reference failure. -/
theorem g1_scoped_semantics (e : Expr .number) :
    targetEval (lower e) = sourceEval e := by
  generalize ht : Ty.number = t at e ⊢
  induction e with
  | literal n =>
      simp [lower, targetEval, sourceEval]
  | plus a b ia ib =>
      simp [lower, targetEval, sourceEval, ia rfl, ib rfl]
  | read saved current payload =>
      simp [lower, targetEval, sourceEval]

/-- G2, restricted: successful target read implies an accepted generation gate. -/
theorem g2_scoped_read_safe (saved current : BitVec 64) (payload value : Nat)
    (h : targetEval (.checkedLoad saved current payload) = .ok value) :
    JabberwockFormal.EmittedGate.valid saved current := by
  by_cases good : JabberwockFormal.EmittedGate.llvmContinues saved current = true
  · exact (JabberwockFormal.EmittedGate.llvm_continues_iff saved current).mp good
  · simp [targetEval, good] at h

/-- G3, restricted: closed source expressions lower to the only well-typed
    target fragment and cannot create a type-mismatched value. -/
def WellTyped : Instr → Prop
  | .constant _ => True
  | .add a b => WellTyped a ∧ WellTyped b
  | .checkedLoad _ _ _ => True

theorem g3_scoped_type_preservation (e : Expr .number) :
    WellTyped (lower e) := by
  generalize ht : Ty.number = t at e ⊢
  induction e with
  | literal n => simp [lower, WellTyped]
  | plus a b ia ib =>
      simp [lower, WellTyped, ia rfl, ib rfl]
  | read saved current payload => simp [lower, WellTyped]

/-- G4, restricted: the abstract move protocol preserves ownership uniqueness. -/
theorem g4_scoped_ownership {s result : JabberwockFormal.Ownership.Store}
    {source target : Nat}
    (unique : JabberwockFormal.Ownership.Unique s)
    (move : JabberwockFormal.Ownership.checkedMove s source target = .ok result) :
    JabberwockFormal.Ownership.Unique result :=
  JabberwockFormal.Ownership.checked_move_preserves_unique unique move

/-- G5, restricted: LLVM predicate succeeds iff the epoch is permitted. -/
theorem g5_scoped_check_correct (saved current : BitVec 64) :
    JabberwockFormal.EmittedGate.llvmContinues saved current = true ↔
      JabberwockFormal.EmittedGate.valid saved current :=
  JabberwockFormal.EmittedGate.llvm_continues_iff saved current

/-- For this fragment only, the two compiler reference gates agree on the decision.
    The emitted LLVM source templates are inspected separately, not formally extracted. -/
theorem two_compiler_gate_agreement (saved current : BitVec 64) :
    JabberwockFormal.Jwc.ReferenceAccess.emittedGate saved current =
      JabberwockFormal.Rust.ReferenceAccess.emittedGate saved current :=
  JabberwockFormal.EmittedGate.cross_compiler_gate_equivalence saved current

end JabberwockFormal.ScopedGoals
