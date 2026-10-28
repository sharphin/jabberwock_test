import JabberwockFormal.Generation
import JabberwockFormal.Ownership

set_option autoImplicit false
set_option warningAsError true

namespace JabberwockFormal.SafetyComposition

/-!
Composition of two independently established *abstract* invariants:
generation-based rejection after invalidation, and uniqueness of owner moves.
This model does not represent actual C heap data, memory ordering, or FFI.
A source-to-model refinement remains an independent obligation for G2/G4.
-/

/-- A checked move cannot introduce a duplicate owner, while invalidation
    prevents reuse of the previously valid borrowed epoch. -/
theorem move_and_invalidation_are_safe
    {limit epoch : Nat}
    {before after final : JabberwockFormal.Generation.Cell}
    {store next : JabberwockFormal.Ownership.Store}
    {source destination : Nat}
    (unique : JabberwockFormal.Ownership.Unique store)
    (moved : JabberwockFormal.Ownership.checkedMove
      store source destination = .ok next)
    (borrowed : JabberwockFormal.Generation.Valid epoch before)
    (invalidated : JabberwockFormal.Generation.Invalidates before after)
    (future : JabberwockFormal.Generation.Steps limit after final) :
    JabberwockFormal.Ownership.Unique next ∧
    JabberwockFormal.Generation.checkedRead epoch final ≠ .ok () ∧
    JabberwockFormal.Generation.checkedWrite
      .readWrite epoch final ≠ .ok () := by
  exact ⟨JabberwockFormal.Ownership.checked_move_preserves_unique unique moved,
    JabberwockFormal.Generation.no_successful_read_after_invalidation
      borrowed invalidated future,
    JabberwockFormal.Generation.no_successful_write_after_invalidation
      borrowed invalidated future⟩

/-- Retirement is absorbing, and it rejects reads as well as writes at every
    later modeled step, including a reference with the same old epoch. -/
theorem retirement_rejects_future_access
    {limit epoch : Nat}
    {retired final : JabberwockFormal.Generation.Cell}
    (dead : retired.phase = .retired)
    (future : JabberwockFormal.Generation.Steps limit retired final) :
    JabberwockFormal.Generation.checkedRead epoch final =
      .error .dangling ∧
    JabberwockFormal.Generation.checkedWrite .readWrite epoch final =
      .error .dangling := by
  have stillDead :=
    JabberwockFormal.Generation.retired_stays_retired dead future
  constructor
  · exact JabberwockFormal.Generation.retired_read_rejected epoch final stillDead
  · exact JabberwockFormal.Generation.retired_read_rejected epoch final stillDead

end JabberwockFormal.SafetyComposition
