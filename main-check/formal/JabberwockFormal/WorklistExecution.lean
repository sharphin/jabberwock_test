import JabberwockFormal.FiniteMeetProgress

set_option autoImplicit false
set_option warningAsError true

namespace JabberwockFormal.WorklistExecution

open JabberwockFormal.FiniteMeetProgress

/-- A work item carries the already computed incoming facts for a reachable
    CFG block. The transfer from actual instructions is NOT modeled here. -/
structure Incoming (blocks locals : Nat) where
  target : Fin blocks
  facts : Facts locals

/-- One abstract state per reachable block. The unvisited/visited distinction
    in the actual compiler remains a separate proof obligation. -/
abbrev Entries (blocks locals : Nat) := Fin blocks → Facts locals

/-- A successful finite execution ends with an empty pending queue.
    Fuel exhaustion is reported distinctly; it is never called convergence. -/
inductive Outcome (blocks locals : Nat) where
  | done (entries : Entries blocks locals)
  | exhausted (entries : Entries blocks locals) (pending : List (Incoming blocks locals))

/-- A bounded worklist executor for precomputed incoming edges. It mirrors
    the meet update but does not schedule new successor edges. -/
def run {blocks locals : Nat}
    (fuel : Nat) (entries : Entries blocks locals)
    (pending : List (Incoming blocks locals)) : Outcome blocks locals :=
  match fuel, pending with
  | _, [] => .done entries
  | 0, _ => .exhausted entries pending
  | fuel + 1, item :: rest =>
      let old := entries item.target
      let next := update old item.facts
      let updated := fun block => if block = item.target then next else entries block
      run fuel updated rest

/-- A report of convergence never hides pending work. -/
theorem empty_queue_done {blocks locals : Nat}
    (fuel : Nat) (entries : Entries blocks locals) :
    run fuel entries [] = .done entries := by
  cases fuel <;> rfl

/-- A nonempty worklist cannot be mistaken for convergence if fuel is zero. -/
theorem exhausted_reports_pending {blocks locals : Nat}
    (entries : Entries blocks locals) (item : Incoming blocks locals)
    (rest : List (Incoming blocks locals)) :
    run 0 entries (item :: rest) = .exhausted entries (item :: rest) := by
  rfl

/-- A finite precomputed edge list can always be completely processed if
    enough fuel is supplied. This does not establish that dynamic scheduling
    in the real analyzer creates only finitely many work items. -/
theorem enough_fuel_finishes {blocks locals : Nat}
    (entries : Entries blocks locals)
    (pending : List (Incoming blocks locals)) :
    ∃ finalEntries, run pending.length entries pending = .done finalEntries := by
  induction pending generalizing entries with
  | nil =>
      exact ⟨entries, rfl⟩
  | cons item rest ih =>
      let next : Entries blocks locals :=
        fun block =>
          if block = item.target then update (entries item.target) item.facts
          else entries block
      obtain ⟨finalEntries, doneProof⟩ := ih next
      exact ⟨finalEntries, doneProof⟩

end JabberwockFormal.WorklistExecution
