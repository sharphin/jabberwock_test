import JabberwockFormal.EmittedGate

set_option autoImplicit false
set_option warningAsError true

namespace JabberwockFormal.EmittedGateOptions

open JabberwockFormal.EmittedGate

/-- A deliberately restricted observation of the check_ref continuation.
    Returning an address is not a proof that it points to live memory. -/
inductive Result where
  | address
  | dangling
  deriving DecidableEq, Repr

/-- Rust's early return when o.gen is disabled, followed by the generation
    guard when it is enabled. This models only the final branch outcome. -/
def rustDecision (genEnabled : Bool) (saved current : Word) : Result :=
  if genEnabled then
    if llvmBad saved current then .dangling else .address
  else .address

/-- jwc check_ref always emits the generation guard in the inspected function.
    Decisions about whether callers invoke it are outside this model. -/
def jwcDecision (saved current : Word) : Result :=
  if llvmBad saved current then .dangling else .address

theorem rust_enabled_agrees_jwc (saved current : Word) :
    rustDecision true saved current = jwcDecision saved current := by
  rfl

theorem rust_disabled_returns_address (saved current : Word) :
    rustDecision false saved current = .address := by
  rfl

theorem rust_enabled_address_iff_valid (saved current : Word) :
    rustDecision true saved current = .address ↔ valid saved current := by
  cases h : llvmBad saved current with
  | false =>
      have good : valid saved current :=
        (llvm_continues_iff saved current).mp (by simp [llvmContinues, h])
      simp [rustDecision, good, h]
  | true =>
      have bad : ¬ valid saved current := by
        intro good
        have ok := (llvm_continues_iff saved current).mpr good
        simp [llvmContinues, h] at ok
      simp [rustDecision, bad, h]

theorem jwc_address_iff_valid (saved current : Word) :
    jwcDecision saved current = .address ↔ valid saved current := by
  rw [← rust_enabled_agrees_jwc]
  exact rust_enabled_address_iff_valid saved current

/-- A retired generation is rejected only on the check-enabled path.
    Disabling the gate bypasses this check in the modeled Rust emitter. -/
theorem rust_retired_enabled_rejected (saved : Word) :
    rustDecision true saved retired = .dangling := by
  simp [rustDecision, llvmBad, llvmRetired]

theorem rust_retired_disabled_not_rejected (saved : Word) :
    rustDecision false saved retired = .address := by
  rfl

end JabberwockFormal.EmittedGateOptions
