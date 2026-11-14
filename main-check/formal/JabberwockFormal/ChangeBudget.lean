import JabberwockFormal.FiniteMeetProgress

set_option autoImplicit false
set_option warningAsError true

namespace JabberwockFormal.ChangeBudget

open JabberwockFormal.FiniteMeetProgress

/-- A strict join consumes one previously available fact slot.
    This is a *certificate-level* abstraction: constructing a certificate
    from the real analyzer's complete worklist is not yet proved. -/
inductive ErasureTrace : Nat → Nat → Prop where
  | finished (remaining : Nat) : ErasureTrace remaining 0
  | erased {remaining changes : Nat} :
      ErasureTrace remaining changes →
      ErasureTrace (remaining + 1) (changes + 1)

/-- A certificate containing m strict changes requires at least m distinct
    consumable fact slots. Hence n initial slots allow at most n changes. -/
theorem strict_changes_le_capacity {capacity changes : Nat}
    (trace : ErasureTrace capacity changes) :
    changes ≤ capacity := by
  induction trace with
  | finished remaining =>
      exact Nat.zero_le remaining
  | erased previous ih =>
      exact Nat.succ_le_succ ih

/-- A block with n locals cannot have a certificate for n+1 strict erasures.
    The missing refinement obligation is to produce such certificates for
    every real worklist transition from the corresponding disappearing fact. -/
theorem no_more_than_local_count (n : Nat) :
    ¬ ErasureTrace n (n + 1) := by
  intro trace
  have bound := strict_changes_le_capacity trace
  exact Nat.not_succ_le_self n bound

/-- A strict abstract join does provide an actual erased local, the
    necessary one-step evidence for constructing a capacity certificate. -/
theorem strict_join_has_erasure_witness {n : Nat}
    (old incoming : Facts n) (strict : update old incoming ≠ old) :
    ∃ key : Fin n, ∃ fact : Nat,
      old key = some fact ∧ update old incoming key = none :=
  changed_update_erases_fact old incoming strict

end JabberwockFormal.ChangeBudget
