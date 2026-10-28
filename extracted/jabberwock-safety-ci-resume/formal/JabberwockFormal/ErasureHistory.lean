import JabberwockFormal.ChangeBudget

set_option autoImplicit false
set_option warningAsError true

namespace JabberwockFormal.ErasureHistory

open JabberwockFormal.FiniteMeetProgress

/-- Record of a single strict reachable-block update and the local whose
    fact disappeared. The equality premises are proof certificates supplied
    by the existing changed_update_erases_fact theorem. -/
structure Step {n : Nat} (old incoming : Facts n) where
  key : Fin n
  fact : Nat
  wasPresent : old key = some fact
  nowAbsent : update old incoming key = none

/-- Every strict meet gives a concrete erasure certificate. -/
theorem strict_step_exists {n : Nat} (old incoming : Facts n)
    (changed : update old incoming ≠ old) :
    Nonempty (Step old incoming) := by
  obtain ⟨key, fact, present, erased⟩ :=
    changed_update_erases_fact old incoming changed
  exact ⟨⟨key, fact, present, erased⟩⟩

/-- The erased local remains absent after every subsequent meet, regardless
    of which predecessor facts arrive. -/
theorem certified_erasure_persists {n : Nat}
    (old incoming : Facts n) (step : Step old incoming)
    (rest : List (Facts n)) :
    updateMany (update old incoming) rest step.key = none := by
  exact absent_after_updates (update old incoming) rest step.key step.nowAbsent

/-- A later strict update cannot erase the same local a second time.
    This is the injectivity obligation for a future numerical count of
    distinct changed steps. -/
theorem later_step_uses_different_key {n : Nat}
    (old incoming : Facts n) (first : Step old incoming)
    (between : List (Facts n)) (nextIncoming : Facts n)
    (later : Step (updateMany (update old incoming) between) nextIncoming) :
    later.key ≠ first.key := by
  intro equal
  have absent := certified_erasure_persists old incoming first between
  have present := later.wasPresent
  rw [equal] at present
  rw [absent] at present
  cases present

end JabberwockFormal.ErasureHistory
