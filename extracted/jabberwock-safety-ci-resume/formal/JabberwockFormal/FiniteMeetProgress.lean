import JabberwockFormal.LoopConvergence

set_option autoImplicit false
set_option warningAsError true

namespace JabberwockFormal.FiniteMeetProgress

/-- Finite analysis facts indexed by the local variable number. -/
abbrev Facts (n : Nat) := Fin n → Option Nat

/-- A reachable block retains a fact exactly when both edges agree. -/
def update {n : Nat} (old incoming : Facts n) : Facts n :=
  fun key => if old key = incoming key then old key else none

theorem retained_from_old {n : Nat} (old incoming : Facts n)
    (key : Fin n) (fact : Nat)
    (h : update old incoming key = some fact) :
    old key = some fact := by
  by_cases same : old key = incoming key
  · simpa [update, same] using h
  · simp [update, same] at h

theorem retained_from_incoming {n : Nat} (old incoming : Facts n)
    (key : Fin n) (fact : Nat)
    (h : update old incoming key = some fact) :
    incoming key = some fact := by
  have ho := retained_from_old old incoming key fact h
  by_cases same : old key = incoming key
  · exact same.symm.trans ho
  · simp [update, same] at h

theorem update_idempotent {n : Nat} (old incoming : Facts n) :
    update (update old incoming) incoming = update old incoming := by
  funext key
  by_cases same : old key = incoming key
  · simp [update, same]
  · simp [update, same]

/-- A missing fact remains missing after a reachable-block meet. -/
theorem absence_persists {n : Nat} (old incoming : Facts n)
    (key : Fin n) (absent : old key = none) :
    update old incoming key = none := by
  simp [update, absent]

/-- Any change to a reachable entry erases some existing positive fact. -/
theorem changed_update_erases_fact {n : Nat} (old incoming : Facts n)
    (changed : update old incoming ≠ old) :
    ∃ key : Fin n, ∃ fact : Nat,
      old key = some fact ∧ update old incoming key = none := by
  have differing : ∃ key : Fin n, update old incoming key ≠ old key := by
    apply Classical.byContradiction
    intro noDifference
    apply changed
    funext key
    by_cases equal : update old incoming key = old key
    · exact equal
    · exact False.elim (noDifference ⟨key, equal⟩)
  obtain ⟨key, different⟩ := differing
  cases oldValue : old key with
  | none =>
      exact False.elim (different ((absence_persists old incoming key oldValue).trans oldValue.symm))
  | some fact =>
      have erased : update old incoming key = none := by
        by_cases same : old key = incoming key
        · exact False.elim (different (by simp [update, same]))
        · simp [update, same]
      exact ⟨key, fact, oldValue, erased⟩

/-- Apply any finite sequence of predecessor merges. -/
def updateMany {n : Nat} (old : Facts n) : List (Facts n) → Facts n
  | [] => old
  | incoming :: rest => updateMany (update old incoming) rest

/-- Repeated merges never resurrect a removed fact. -/
theorem absent_after_updates {n : Nat} (old : Facts n)
    (incoming : List (Facts n)) (key : Fin n)
    (absent : old key = none) :
    updateMany old incoming key = none := by
  induction incoming generalizing old with
  | nil => exact absent
  | cons head tail ih =>
      exact ih (update old head) (absence_persists old head key absent)

/-- A changed reachable entry consumes at least one fact slot permanently:
    all later predecessor merges leave that slot absent. This is the key
    per-block progress invariant needed for a future numeric worklist bound. -/
theorem changed_update_permanently_erases_fact {n : Nat}
    (old incoming : Facts n) (later : List (Facts n))
    (changed : update old incoming ≠ old) :
    ∃ key : Fin n, ∃ fact : Nat,
      old key = some fact ∧
      update old incoming key = none ∧
      updateMany (update old incoming) later key = none := by
  obtain ⟨key, fact, present, erased⟩ :=
    changed_update_erases_fact old incoming changed
  exact ⟨key, fact, present, erased,
    absent_after_updates (update old incoming) later key erased⟩

/-- An entry with no facts left is stable under every later meet. -/
theorem empty_entry_stable {n : Nat} (incoming : Facts n)
    (empty : Facts n) (allAbsent : ∀ key, empty key = none) :
    update empty incoming = empty := by
  funext key
  exact (absence_persists empty incoming key (allAbsent key)).trans
    (allAbsent key).symm

/-- A concrete variable can lose its fact only once: after its first
    removal, every later finite sequence of joins keeps that slot empty. -/
theorem no_second_loss_at_same_local {n : Nat}
    (old incoming : Facts n) (later : List (Facts n))
    (key : Fin n)
    (erased : update old incoming key = none) :
    updateMany (update old incoming) later key = none := by
  exact absent_after_updates (update old incoming) later key erased

/-- For a block with no locals, even an arbitrary incoming edge cannot
    change the entry. This is the base case of the finite-state bound. -/
theorem zero_locals_cannot_change
    (old incoming : Facts 0) :
    update old incoming = old := by
  funext key
  exact Fin.elim0 key

/-- Once a particular key has lost its fact, no later incoming edge can
    cause a second loss at that same key. This is a per-variable bound
    (at most one erasure), not yet the global n-variable count theorem. -/
theorem no_second_erasure_at_key {n : Nat}
    (old first second : Facts n) (key : Fin n)
    (erased : update old first key = none) :
    ¬ ∃ fact : Nat,
      update old first key = some fact ∧
      update (update old first) second key = none := by
  intro claimed
  obtain ⟨fact, present, _⟩ := claimed
  rw [erased] at present
  cases present

/-- Two consecutive strict updates cannot both erase the only possible
    local when the analysis has exactly one variable. This establishes
    the explicit numeric bound 1 for the one-local case. -/
theorem one_local_no_two_strict_updates
    (old first second : Facts 1)
    (firstChanged : update old first ≠ old) :
    update (update old first) second = update old first := by
  obtain ⟨key, fact, _, erased⟩ :=
    changed_update_erases_fact old first firstChanged
  funext other
  have same : other = key := Subsingleton.elim other key
  subst other
  exact (absence_persists (update old first) second key erased).trans erased.symm

/-- Every fact that remains after any finite history of reachable-block
    joins was already present in the initial entry. No incoming edge
    can invent a new justification for check elision. -/
theorem updateMany_retained_from_initial {n : Nat}
    (initial : Facts n) (history : List (Facts n))
    (key : Fin n) (fact : Nat)
    (retained : updateMany initial history key = some fact) :
    initial key = some fact := by
  induction history generalizing initial with
  | nil =>
      exact retained
  | cons head tail ih =>
      exact retained_from_old initial head key fact
        (ih (update initial head) retained)

/-- If a strict change happens after a finite history, the erased fact
    must have existed at the start of that history. It is absent after
    the change and stays absent through every future meet. -/
theorem historical_change_consumes_initial_fact {n : Nat}
    (initial : Facts n) (before after : List (Facts n))
    (incoming : Facts n)
    (strict : update (updateMany initial before) incoming ≠
      updateMany initial before) :
    ∃ key : Fin n, ∃ fact : Nat,
      initial key = some fact ∧
      update (updateMany initial before) incoming key = none ∧
      updateMany (update (updateMany initial before) incoming) after key = none := by
  obtain ⟨key, fact, present, erased⟩ :=
    changed_update_erases_fact (updateMany initial before) incoming strict
  exact ⟨key, fact,
    updateMany_retained_from_initial initial before key fact present,
    erased,
    absent_after_updates _ after key erased⟩

/-- Once every initially occupied key has been erased by a finite history,
    no further incoming edge can change the reachable block entry.
    Unlike a free-standing budget certificate, this theorem directly
    concerns the concrete updateMany history. -/
theorem exhausted_initial_facts_stable {n : Nat}
    (initial : Facts n) (history : List (Facts n))
    (erasedInitiallyPresent :
      ∀ key : Fin n, ∀ fact : Nat,
        initial key = some fact →
        updateMany initial history key = none)
    (incoming : Facts n) :
    update (updateMany initial history) incoming =
      updateMany initial history := by
  funext key
  cases oldValue : updateMany initial history key with
  | none =>
      exact absence_persists _ incoming key oldValue
  | some fact =>
      have initialPresent :=
        updateMany_retained_from_initial initial history key fact oldValue
      have erased := erasedInitiallyPresent key fact initialPresent
      rw [oldValue] at erased
      cases erased

/-- An exhausted entry stays fixed even after arbitrarily many further
    predecessor joins. -/
theorem exhausted_initial_facts_remain_stable {n : Nat}
    (initial : Facts n) (history later : List (Facts n))
    (erasedInitiallyPresent :
      ∀ key : Fin n, ∀ fact : Nat,
        initial key = some fact →
        updateMany initial history key = none) :
    updateMany (updateMany initial history) later =
      updateMany initial history := by
  have allAbsent : ∀ key : Fin n,
      updateMany initial history key = none := by
    intro key
    cases initialValue : initial key with
    | none =>
        exact absent_after_updates initial history key initialValue
    | some fact =>
        exact erasedInitiallyPresent key fact initialValue
  funext key
  exact (absent_after_updates _ later key (allAbsent key)).trans
    (allAbsent key).symm

/-- Splitting a sequence of predecessor meets does not change its result. -/
theorem updateMany_append {n : Nat}
    (old : Facts n) (before after : List (Facts n)) :
    updateMany old (before ++ after) =
      updateMany (updateMany old before) after := by
  induction before generalizing old with
  | nil =>
      rfl
  | cons head tail ih =>
      exact ih (update old head)

/-- A local erased at an arbitrary point in a merge history cannot be
    the witness for any later strict change, even with intervening merges. -/
theorem historical_strict_changes_have_distinct_keys {n : Nat}
    (initial : Facts n) (before between : List (Facts n))
    (firstIncoming secondIncoming : Facts n)
    (firstStrict :
      update (updateMany initial before) firstIncoming ≠
        updateMany initial before)
    (secondStrict :
      update
        (updateMany
          (update (updateMany initial before) firstIncoming)
          between)
        secondIncoming ≠
      updateMany
        (update (updateMany initial before) firstIncoming)
        between) :
    ∃ firstKey secondKey : Fin n, firstKey ≠ secondKey := by
  obtain ⟨firstKey, _, _, erased⟩ :=
    changed_update_erases_fact
      (updateMany initial before) firstIncoming firstStrict
  obtain ⟨secondKey, _, present, _⟩ :=
    changed_update_erases_fact
      (updateMany
        (update (updateMany initial before) firstIncoming)
        between) secondIncoming secondStrict
  refine ⟨firstKey, secondKey, ?_⟩
  intro same
  have absent := absent_after_updates
    (update (updateMany initial before) firstIncoming)
    between firstKey erased
  rw [← same] at present
  rw [absent] at present
  cases present

end JabberwockFormal.FiniteMeetProgress
