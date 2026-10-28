import JabberwockFormal.WorklistExecution

set_option autoImplicit false
set_option warningAsError true

namespace JabberwockFormal.WorklistScheduling

open JabberwockFormal.FiniteMeetProgress
open JabberwockFormal.WorklistExecution

/-- Abstract scheduling decision for an already reachable block. The
    equality decision is classical: no executable comparison of functions
    or proof of a concrete scheduler is claimed here. -/
noncomputable def enqueueIfChanged {blocks locals : Nat}
    (old incoming : Facts locals)
    (item : Incoming blocks locals)
    (pending : List (Incoming blocks locals)) :
    List (Incoming blocks locals) := by
  classical
  exact if update old incoming = old then pending else item :: pending

theorem stable_entry_does_not_enqueue {blocks locals : Nat}
    (old incoming : Facts locals)
    (item : Incoming blocks locals)
    (pending : List (Incoming blocks locals))
    (stable : update old incoming = old) :
    enqueueIfChanged old incoming item pending = pending := by
  simp [enqueueIfChanged, stable]

theorem changed_entry_enqueues {blocks locals : Nat}
    (old incoming : Facts locals)
    (item : Incoming blocks locals)
    (pending : List (Incoming blocks locals))
    (changed : update old incoming ≠ old) :
    enqueueIfChanged old incoming item pending = item :: pending := by
  simp [enqueueIfChanged, changed]

/-- A changed reachable entry removes a known fact. This is an abstract
    progress witness, not yet a termination proof for dynamic CFG traversal. -/
theorem enqueue_requires_erasure {blocks locals : Nat}
    (old incoming : Facts locals)
    (item : Incoming blocks locals)
    (pending : List (Incoming blocks locals))
    (scheduled : enqueueIfChanged old incoming item pending =
      item :: pending)
    (notAlreadyHead : pending ≠ item :: pending) :
    ∃ key : Fin locals, ∃ fact : Nat,
      old key = some fact ∧ update old incoming key = none := by
  by_cases stable : update old incoming = old
  · have impossible : pending = item :: pending := by
      have unchanged := stable_entry_does_not_enqueue old incoming item pending stable
      exact unchanged.symm.trans scheduled
    exact False.elim (notAlreadyHead impossible)
  · exact changed_update_erases_fact old incoming stable

end JabberwockFormal.WorklistScheduling
