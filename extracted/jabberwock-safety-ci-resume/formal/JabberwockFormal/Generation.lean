import Lean.Elab.Tactic.Omega

set_option autoImplicit false
set_option warningAsError true

namespace JabberwockFormal.Generation

/-- Metadata only: payloads, addresses, threads and machine encoding are not modeled. -/
inductive Phase where
  | live
  | vacant
  | retired
  deriving DecidableEq, Repr

structure Cell where
  epoch : Nat
  phase : Phase
  deriving DecidableEq, Repr

/-- `Nat` prevents implicit arithmetic wrap. The bound is a separate invariant. -/
def WellFormed (limit : Nat) (c : Cell) : Prop := c.epoch ≤ limit

def Valid (epoch : Nat) (c : Cell) : Prop :=
  c.phase = .live ∧ epoch = c.epoch

/-- A conservative implementation may retire before the last representable epoch. -/
inductive Step (limit : Nat) : Cell → Cell → Prop where
  | keep (c : Cell) : Step limit c c
  | allocate (g : Nat) (bound : g ≤ limit) :
      Step limit ⟨g, .vacant⟩ ⟨g, .live⟩
  | invalidate (g : Nat) (room : g < limit) :
      Step limit ⟨g, .live⟩ ⟨g + 1, .live⟩
  | release (g : Nat) (room : g < limit) :
      Step limit ⟨g, .live⟩ ⟨g + 1, .vacant⟩
  | retire (g : Nat) (bound : g ≤ limit) :
      Step limit ⟨g, .live⟩ ⟨g, .retired⟩

inductive Steps (limit : Nat) : Cell → Cell → Prop where
  | refl (c : Cell) : Steps limit c c
  | tail {a b c : Cell} : Steps limit a b → Step limit b c → Steps limit a c

theorem step_preserves_bound {limit : Nat} {a b : Cell}
    (wf : WellFormed limit a) (step : Step limit a b) : WellFormed limit b := by
  cases step <;> simp_all [WellFormed] <;> omega

theorem step_epoch_monotone {limit : Nat} {a b : Cell}
    (step : Step limit a b) : a.epoch ≤ b.epoch := by
  cases step <;> simp <;> omega

theorem retired_is_absorbing {limit : Nat} {a b : Cell}
    (retired : a.phase = .retired) (step : Step limit a b) : b = a := by
  cases step <;> simp_all

theorem steps_preserve_bound {limit : Nat} {a b : Cell}
    (wf : WellFormed limit a) (steps : Steps limit a b) : WellFormed limit b := by
  induction steps with
  | refl => exact wf
  | tail _ step ih => exact step_preserves_bound ih step

theorem steps_epoch_monotone {limit : Nat} {a b : Cell}
    (steps : Steps limit a b) : a.epoch ≤ b.epoch := by
  induction steps with
  | refl => exact Nat.le_refl _
  | tail _ step ih => exact Nat.le_trans ih (step_epoch_monotone step)

theorem retired_stays_retired {limit : Nat} {a b : Cell}
    (retired : a.phase = .retired) (steps : Steps limit a b) : b.phase = .retired := by
  induction steps with
  | refl => exact retired
  | tail _ step ih => rw [retired_is_absorbing ih step]; exact ih

/-- Every invalidating operation must establish this postcondition. -/
def Invalidates (before after : Cell) : Prop :=
  before.phase = .live ∧
    (before.epoch < after.epoch ∨ after.phase = .retired)

theorem invalidation_postcondition (limit g : Nat) (room : g < limit) :
    Step limit ⟨g, .live⟩ ⟨g + 1, .live⟩ ∧
    Invalidates ⟨g, .live⟩ ⟨g + 1, .live⟩ := by
  exact ⟨.invalidate g room, rfl, Or.inl (Nat.lt_succ_self g)⟩

theorem release_postcondition (limit g : Nat) (room : g < limit) :
    Step limit ⟨g, .live⟩ ⟨g + 1, .vacant⟩ ∧
    Invalidates ⟨g, .live⟩ ⟨g + 1, .vacant⟩ := by
  exact ⟨.release g room, rfl, Or.inl (Nat.lt_succ_self g)⟩

theorem retirement_postcondition (limit g : Nat) (bound : g ≤ limit) :
    Step limit ⟨g, .live⟩ ⟨g, .retired⟩ ∧
    Invalidates ⟨g, .live⟩ ⟨g, .retired⟩ := by
  exact ⟨.retire g bound, rfl, Or.inr rfl⟩

/-- The bound can be zero: even the first invalidation can retire without wrapping. -/
theorem invalidation_available {limit : Nat} {c : Cell}
    (wf : WellFormed limit c) (live : c.phase = .live) :
    ∃ next, Step limit c next ∧ Invalidates c next := by
  cases c with
  | mk g phase =>
    cases live
    by_cases room : g < limit
    · exact ⟨⟨g + 1, .live⟩, invalidation_postcondition limit g room⟩
    · exact ⟨⟨g, .retired⟩, retirement_postcondition limit g wf⟩

/-- After a genuine invalidation, no finite allowed continuation revives the old ref. -/
theorem no_resurrection {limit epoch : Nat} {before after final : Cell}
    (borrowed : Valid epoch before) (invalidated : Invalidates before after)
    (future : Steps limit after final) : ¬ Valid epoch final := by
  intro valid
  rcases borrowed with ⟨_, sameEpoch⟩
  rcases invalidated with ⟨_, newer | retired⟩
  · have monotone := steps_epoch_monotone future
    rcases valid with ⟨_, finalEpoch⟩
    omega
  · have stillRetired := retired_stays_retired retired future
    have live := valid.1
    rw [stillRetired] at live
    cases live

inductive Fault where
  | dangling
  | readOnly
  deriving DecidableEq, Repr

/-- Abstract checked access: no payload access is allowed before this gate succeeds. -/
def checkedRead (epoch : Nat) (c : Cell) : Except Fault Unit :=
  if c.phase = .live ∧ epoch = c.epoch then .ok () else .error .dangling

theorem checked_read_iff (epoch : Nat) (c : Cell) :
    checkedRead epoch c = .ok () ↔ Valid epoch c := by
  simp [checkedRead, Valid]

/-- A previously borrowed epoch cannot pass the checked read after any
    invalidation and any finite sequence of modeled transitions. -/
theorem no_successful_read_after_invalidation
    {limit epoch : Nat} {before after final : Cell}
    (borrowed : Valid epoch before)
    (invalidated : Invalidates before after)
    (future : Steps limit after final) :
    checkedRead epoch final ≠ .ok () := by
  intro accepted
  exact (no_resurrection borrowed invalidated future)
    ((checked_read_iff epoch final).mp accepted)

theorem retired_read_rejected (epoch : Nat) (c : Cell) (retired : c.phase = .retired) :
    checkedRead epoch c = .error .dangling := by
  simp [checkedRead, retired]

inductive Permission where
  | readOnly
  | readWrite
  deriving DecidableEq, Repr

def checkedWrite (permission : Permission) (epoch : Nat) (c : Cell) : Except Fault Unit :=
  match permission with
  | .readOnly => .error .readOnly
  | .readWrite => checkedRead epoch c

theorem read_only_write_rejected (epoch : Nat) (c : Cell) :
    checkedWrite .readOnly epoch c = .error .readOnly := rfl

/-- `ref mut` grants permission, not exclusivity. There is no alias-count premise. -/
theorem writable_access_iff (epoch : Nat) (c : Cell) :
    checkedWrite .readWrite epoch c = .ok () ↔ Valid epoch c := by
  exact checked_read_iff epoch c

/-- Writable access also cannot succeed after any modeled invalidation,
    provided the original reference was valid before that operation. -/
theorem no_successful_write_after_invalidation
    {limit epoch : Nat} {before after final : Cell}
    (borrowed : Valid epoch before)
    (invalidated : Invalidates before after)
    (future : Steps limit after final) :
    checkedWrite .readWrite epoch final ≠ .ok () := by
  intro accepted
  exact (no_resurrection borrowed invalidated future)
    ((writable_access_iff epoch final).mp accepted)


def wrappingNext (modulus epoch : Nat) : Nat := (epoch + 1) % modulus

theorem wrapping_revives_at_two : wrappingNext 2 (wrappingNext 2 0) = 0 := by
  decide

end JabberwockFormal.Generation
