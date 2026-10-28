import JabberwockFormal.Generation
import Lean.Elab.Tactic.Omega

set_option autoImplicit false
set_option warningAsError true

namespace JabberwockFormal.RuntimeGenerationRefinement

/-- Arithmetic abstraction of runtime/jw_rt.c:gen_has_room.
    The C implementation additionally masks state bits, uses uint64_t,
    and reserves UINT64_MAX for retirement; those implementation links
    are NOT proved by this arithmetic model. -/
def HasRoom (limit generation step : Nat) : Prop :=
  step ≤ limit ∧ generation ≤ limit - step

/-- A successful capacity check guarantees that increasing the generation
    counter stays inside its specified usable range. -/
theorem hasRoom_prevents_overflow
    {limit generation step : Nat}
    (room : HasRoom limit generation step) :
    generation + step ≤ limit := by
  unfold HasRoom at room
  omega

/-- A nonzero generation step strictly advances the epoch without wrapping. -/
theorem positive_step_increases_generation
    {limit generation step : Nat}
    (room : HasRoom limit generation step)
    (positive : 0 < step) :
    generation < generation + step ∧ generation + step ≤ limit := by
  constructor
  · omega
  · exact hasRoom_prevents_overflow room

/-- The runtime's one-step free/invalidating increment can be represented
    by the abstract Generation release transition when it has room.
    This is conditional arithmetic refinement, NOT proof of the C memory
    allocator or pointer lifetime. -/
theorem one_step_release_matches_model
    {limit generation : Nat}
    (room : HasRoom limit generation 1) :
    JabberwockFormal.Generation.Step limit
      ⟨generation, .live⟩ ⟨generation + 1, .vacant⟩ := by
  apply JabberwockFormal.Generation.Step.release
  have bound := hasRoom_prevents_overflow room
  omega

/-- When the model has no capacity for a further increment, retirement
    is still an allowed transition for a currently live in-range cell. -/
theorem no_capacity_can_retire
    {limit generation : Nat}
    (bounded : generation ≤ limit) :
    JabberwockFormal.Generation.Step limit
      ⟨generation, .live⟩ ⟨generation, .retired⟩ := by
  exact JabberwockFormal.Generation.Step.retire generation bounded

/-- A released block may be reused at the new epoch, but the saved epoch
    of a reference issued before release cannot pass the generation gate.
    This models the logical epoch transition; physical storage, C atomics,
    and freelist correctness remain independent refinement obligations. -/
theorem released_then_reused_rejects_old_reference
    {limit generation : Nat}
    (room : HasRoom limit generation 1) :
    JabberwockFormal.Generation.checkedRead generation
      ⟨generation + 1, .live⟩ ≠ .ok () := by
  have released :=
    JabberwockFormal.Generation.release_postcondition limit generation
      (by
        have bound := hasRoom_prevents_overflow room
        omega)
  have capacity : generation + 1 ≤ limit :=
    hasRoom_prevents_overflow room
  have reused : JabberwockFormal.Generation.Steps limit
      ⟨generation + 1, .vacant⟩ ⟨generation + 1, .live⟩ :=
    .tail (.refl _) (.allocate (generation + 1) capacity)
  exact JabberwockFormal.Generation.no_successful_read_after_invalidation
    (borrowed := ⟨rfl, rfl⟩)
    released.2 reused

/-- Retirement is a terminal state for old references in the generation
    model, regardless of the saved epoch. -/
theorem retired_block_rejects_old_reference
    (generation saved : Nat) :
    JabberwockFormal.Generation.checkedRead saved
      ⟨generation, .retired⟩ = .error .dangling := by
  exact JabberwockFormal.Generation.retired_read_rejected
    saved ⟨generation, .retired⟩ rfl

end JabberwockFormal.RuntimeGenerationRefinement
