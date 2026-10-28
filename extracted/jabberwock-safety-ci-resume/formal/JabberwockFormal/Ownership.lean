import Lean.Elab.Tactic.Omega

set_option autoImplicit false
set_option warningAsError true

namespace JabberwockFormal.Ownership

/-- Locations are abstract ownership positions; keys identify allocations, not epochs. -/
abbrev Store := Nat → Option Nat

def Unique (s : Store) : Prop :=
  ∀ a b key, s a = some key → s b = some key → a = b

/-- Successful move; callers must first check that source != target and target is empty. -/
def moveOwner (s : Store) (source target : Nat) : Store :=
  fun location =>
    if location = source then none
    else if location = target then s source
    else s location

theorem move_empties_source (s : Store) (source target : Nat) :
    moveOwner s source target source = none := by
  simp [moveOwner]

theorem move_preserves_identity (s : Store) (source target : Nat)
    (distinct : source ≠ target) :
    moveOwner s source target target = s source := by
  simp [moveOwner, Ne.symm distinct]

theorem move_preserves_other (s : Store) (source target location : Nat)
    (notSource : location ≠ source) (notTarget : location ≠ target) :
    moveOwner s source target location = s location := by
  simp [moveOwner, notSource, notTarget]

theorem move_preserves_unique {s : Store} {source target : Nat}
    (unique : Unique s) (distinct : source ≠ target) :
    Unique (moveOwner s source target) := by
  intro a b key ha hb
  by_cases aSource : a = source
  · simp [moveOwner, aSource] at ha
  by_cases bSource : b = source
  · simp [moveOwner, bSource] at hb
  by_cases aTarget : a = target
  · by_cases bTarget : b = target
    · exact aTarget.trans bTarget.symm
    · have hsource : s source = some key := by
        simpa [moveOwner, aSource, aTarget, Ne.symm distinct] using ha
      have hother : s b = some key := by
        simpa [moveOwner, bSource, bTarget, Ne.symm distinct] using hb
      exact False.elim (bSource (unique b source key hother hsource))
  · by_cases bTarget : b = target
    · have hsource : s source = some key := by
        simpa [moveOwner, bSource, bTarget, Ne.symm distinct] using hb
      have hother : s a = some key := by
        simpa [moveOwner, aSource, aTarget] using ha
      exact False.elim (aSource (unique a source key hother hsource))
    · apply unique a b key
      · simpa [moveOwner, aSource, aTarget] using ha
      · simpa [moveOwner, bSource, bTarget] using hb

inductive MoveFault where
  | sameLocation
  | sourceEmpty
  | targetOccupied
  deriving DecidableEq, Repr

/-- Occupied destinations must be cleaned up by the language evaluator first. -/
def checkedMove (s : Store) (source target : Nat) : Except MoveFault Store :=
  if source = target then .error .sameLocation
  else match s source with
    | none => .error .sourceEmpty
    | some _ => if s target = none then .ok (moveOwner s source target)
                else .error .targetOccupied

theorem checked_move_preserves_unique {s result : Store} {source target : Nat}
    (unique : Unique s) (success : checkedMove s source target = .ok result) :
    Unique result := by
  by_cases same : source = target
  · simp [checkedMove, same] at success
  · cases entry : s source with
    | none => simp [checkedMove, same, entry] at success
    | some key =>
      by_cases empty : s target = none
      · have eq : moveOwner s source target = result := by
          simpa [checkedMove, same, entry, empty] using success
        rw [← eq]
        exact move_preserves_unique unique same
      · simp [checkedMove, same, entry, empty] at success

theorem empty_source_rejected (s : Store) (source target : Nat)
    (distinct : source ≠ target) (empty : s source = none) :
    checkedMove s source target = .error .sourceEmpty := by
  simp [checkedMove, distinct, empty]

end JabberwockFormal.Ownership
