import JabberwockFormal.ErasureHistory

set_option autoImplicit false
set_option warningAsError true

namespace JabberwockFormal.MeetChangeCount

open JabberwockFormal.FiniteMeetProgress

/-- Count *actual strict state changes* of a reachable block when processing
    a concrete finite list of predecessor facts. Unchanged merges count zero. -/
noncomputable def changeCount {n : Nat} (initial : Facts n) :
    List (Facts n) → Nat
  | [] => 0
  | incoming :: rest => by
      classical
      exact (if update initial incoming = initial then 0 else 1) +
        changeCount (update initial incoming) rest

/-- Every input edge can cause at most one change. This is a list-length
    bound, distinct from the stronger number-of-locals bound. -/
theorem changeCount_le_history_length {n : Nat}
    (initial : Facts n) (history : List (Facts n)) :
    changeCount initial history ≤ history.length := by
  classical
  induction history generalizing initial with
  | nil =>
      exact Nat.le_refl 0
  | cons incoming rest ih =>
      by_cases stable : update initial incoming = initial
      · simpa [changeCount, stable] using
          Nat.le_trans (ih (update initial incoming)) (Nat.le_succ rest.length)
      · have bound := Nat.succ_le_succ (ih (update initial incoming))
        simpa only [changeCount, if_neg stable, List.length_cons,
          Nat.succ_eq_add_one, Nat.add_comm] using bound

/-- An empty reachable entry has no changes, no matter how many incoming
    predecessor edges it processes. -/
theorem absent_entry_has_zero_changes {n : Nat}
    (initial : Facts n) (history : List (Facts n))
    (absent : ∀ key : Fin n, initial key = none) :
    changeCount initial history = 0 := by
  classical
  induction history generalizing initial with
  | nil =>
      rfl
  | cons incoming rest ih =>
      have stable : update initial incoming = initial :=
        empty_entry_stable incoming initial absent
      simpa [changeCount, stable] using ih initial absent

/-- When the initial entry is already empty, its change count is zero;
    in particular, the n=0 case has the sharp numeric bound zero. -/
theorem zero_locals_have_zero_changes
    (initial : Facts 0) (history : List (Facts 0)) :
    changeCount initial history = 0 := by
  apply absent_entry_has_zero_changes
  intro key
  exact Fin.elim0 key

/-- A strict change contributes exactly one, then counting continues from
    the actual updated state. -/
theorem strict_change_count {n : Nat}
    (initial incoming : Facts n) (rest : List (Facts n))
    (strict : update initial incoming ≠ initial) :
    changeCount initial (incoming :: rest) =
      changeCount (update initial incoming) rest + 1 := by
  classical
  simp [changeCount, strict, Nat.add_comm]

/-- Counting changes over concatenated predecessor histories agrees with
    counting the first history and continuing from its resulting entry. -/
theorem changeCount_append {n : Nat}
    (initial : Facts n) (before after : List (Facts n)) :
    changeCount initial (before ++ after) =
      changeCount initial before +
        changeCount (updateMany initial before) after := by
  classical
  induction before generalizing initial with
  | nil =>
      simp only [List.nil_append, changeCount, updateMany, Nat.zero_add]
  | cons incoming rest ih =>
      simp only [List.cons_append, changeCount, updateMany]
      rw [ih]
      exact (Nat.add_assoc _ _ _).symm

/-- With exactly one local, the *actual* number of strict updates in any
    finite predecessor history is at most one, not merely in two steps. -/
theorem one_local_changeCount_le_one
    (initial : Facts 1) (history : List (Facts 1)) :
    changeCount initial history ≤ 1 := by
  classical
  induction history generalizing initial with
  | nil =>
      exact Nat.zero_le 1
  | cons incoming rest ih =>
      by_cases stable : update initial incoming = initial
      · simpa [changeCount, stable] using ih (update initial incoming)
      · obtain ⟨erasedKey, _, _, erased⟩ :=
          changed_update_erases_fact initial incoming stable
        have allAbsent : ∀ key : Fin 1,
            update initial incoming key = none := by
          intro key
          have same : key = erasedKey := Subsingleton.elim key erasedKey
          rw [same]
          exact erased
        have noFurtherChanges :=
          absent_entry_has_zero_changes
            (update initial incoming) rest allAbsent
        simp [changeCount, stable, noFurtherChanges]

/-- If the concrete history counter reports no strict changes, the
    reachable entry after processing that history is unchanged. This
    connects the numeric counter to the actual abstract entry state. -/
theorem zero_changeCount_preserves_entry {n : Nat}
    (initial : Facts n) (history : List (Facts n))
    (zero : changeCount initial history = 0) :
    updateMany initial history = initial := by
  classical
  induction history generalizing initial with
  | nil =>
      rfl
  | cons incoming rest ih =>
      by_cases stable : update initial incoming = initial
      · have tailZero : changeCount (update initial incoming) rest = 0 := by
          simpa [changeCount, stable] using zero
        calc
          updateMany initial (incoming :: rest) =
              updateMany (update initial incoming) rest := rfl
          _ = update initial incoming := ih (update initial incoming) tailZero
          _ = initial := stable
      · have impossible : False := by
          have countPositive :
              changeCount initial (incoming :: rest) =
                changeCount (update initial incoming) rest + 1 :=
            strict_change_count initial incoming rest stable
          rw [countPositive] at zero
          exact Nat.noConfusion zero
        exact False.elim impossible

end JabberwockFormal.MeetChangeCount
