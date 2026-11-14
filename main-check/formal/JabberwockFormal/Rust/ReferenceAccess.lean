import JabberwockFormal.Generation
import JabberwockFormal.Ownership

set_option autoImplicit false
set_option warningAsError true

namespace JabberwockFormal.Rust.ReferenceAccess

/-- A 64-bit generation word, including the all-ones retirement marker. -/
abbrev Word := BitVec 64

def retired : Word := 18446744073709551615

/-- Specification of the epoch gate only. Metadata lifetime and payload layout are premises outside this model. -/
def Valid (saved current : Word) : Prop := current = saved ∧ current ≠ retired

instance (saved current : Word) : Decidable (Valid saved current) :=
  inferInstanceAs (Decidable (current = saved ∧ current ≠ retired))

/-- Explicit formalization of this compiler's emitted comparison/branch decision.
    The correspondence to its emitter is documented and hash-bound, not extracted or proved. -/
def emittedGate (saved current : Word) : Bool :=
  if current ≠ saved ∨ current = retired then false else true

theorem gate_success_iff (saved current : Word) :
    emittedGate saved current = true ↔ Valid saved current := by
  by_cases same : current = saved <;> by_cases dead : current = retired <;>
    simp [emittedGate, Valid, same, dead]

/-- Local semantics preservation: the emitted Boolean decision implements the specification. -/
theorem gate_semantics (saved current : Word) :
    emittedGate saved current = decide (Valid saved current) := by
  by_cases same : current = saved <;> by_cases dead : current = retired <;>
    simp [emittedGate, Valid, same, dead]

inductive Fault where
  | dangling
  | readOnly
  deriving DecidableEq, Repr

def checkedRead (saved current : Word) (payload : Nat) : Except Fault Nat :=
  if emittedGate saved current then .ok payload else .error .dangling

/-- Conditional memory property: no successful modeled payload read after an invalid gate.
    This does not prove that a real pointer is allocated or race-free. -/
theorem read_requires_valid (saved current : Word) (payload value : Nat)
    (success : checkedRead saved current payload = .ok value) : Valid saved current := by
  by_cases accepted : emittedGate saved current = true
  · exact (gate_success_iff saved current).mp accepted
  · simp [checkedRead, accepted] at success

def checkedWrite (writable : Bool) (saved current : Word) : Except Fault Unit :=
  if writable then
    if emittedGate saved current then .ok () else .error .dangling
  else .error .readOnly

/-- The modeled read-only type cannot grant write permission. Whole-language type soundness is outside this theorem. -/
theorem readonly_write_rejected (saved current : Word) :
    checkedWrite false saved current = .error .readOnly := by
  simp [checkedWrite]

theorem successful_write_is_writable (writable : Bool) (saved current : Word)
    (success : checkedWrite writable saved current = .ok ()) : writable = true := by
  cases writable with
  | false => simp [checkedWrite] at success
  | true => rfl

/-- In particular, an equal saved/current generation must not revive retired metadata. -/
theorem matching_retired_rejected : emittedGate retired retired = false := by
  simp [emittedGate]

theorem retired_read_rejected (saved : Word) (payload : Nat) :
    checkedRead saved retired payload = .error .dangling := by
  by_cases same : retired = saved <;> simp [checkedRead, emittedGate, same]

/-- Ownership transition modeled for this compiler. This is not the interpreter or C implementation. -/
def moveOwner (s : JabberwockFormal.Ownership.Store) (source target : Nat) :=
  JabberwockFormal.Ownership.checkedMove s source target

theorem move_preserves_unique {s result : JabberwockFormal.Ownership.Store}
    {source target : Nat} (unique : JabberwockFormal.Ownership.Unique s)
    (success : moveOwner s source target = .ok result) :
    JabberwockFormal.Ownership.Unique result := by
  exact JabberwockFormal.Ownership.checked_move_preserves_unique unique success

end JabberwockFormal.Rust.ReferenceAccess
