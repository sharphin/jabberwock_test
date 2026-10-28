import JabberwockFormal.CfgMeetElision

set_option autoImplicit false
set_option warningAsError true

namespace JabberwockFormal.LoopConvergence

open JabberwockFormal.CheckElision
open JabberwockFormal.ElisionDecision
open JabberwockFormal.CfgMeetElision

/-- An incoming predecessor is merged into an already reachable block.
    An unreachable block is initialized from its first predecessor instead.
    The distinction mirrors Option<Facts> / has[] in the real analyzers. -/
def mergeEntry (old : Option AnalyzerFacts) (incoming : AnalyzerFacts) :
    AnalyzerFacts :=
  match old with
  | none => incoming
  | some previous => meetFacts previous incoming

/-- Repeated predecessors at a reachable block. -/
def mergeMore (initial : AnalyzerFacts) : List AnalyzerFacts → AnalyzerFacts
  | [] => initial
  | incoming :: rest => mergeMore (meetFacts initial incoming) rest

/-- Once a block is reachable, an additional predecessor cannot introduce
    a new first-dereference elision candidate. -/
theorem predecessor_cannot_create_elision
    (old incoming : AnalyzerFacts) (key index : Nat)
    (elided : elideFirst (meetFacts old incoming)
      (meetFacts old incoming key) index = true) :
    elideFirst old (old key) index = true :=
  (joined_elision_from_both old incoming key index elided).1

/-- A finite run of repeated joins cannot introduce a check-elision
    justification that was absent in the original reachable entry. -/
theorem repeated_joins_cannot_create_elision
    (initial : AnalyzerFacts) (incoming : List AnalyzerFacts)
    (key index : Nat)
    (elided : elideFirst (mergeMore initial incoming)
      (mergeMore initial incoming key) index = true) :
    elideFirst initial (initial key) index = true := by
  induction incoming generalizing initial with
  | nil => exact elided
  | cons head tail ih =>
      exact predecessor_cannot_create_elision initial head key index
        (ih (meetFacts initial head) elided)

/-- A known reachable block never gains a new concrete fact from one meet. -/
theorem merge_preserves_old_fact
    (old incoming : AnalyzerFacts) (key : Nat) (fact : AnalyzerFact)
    (newFact : mergeEntry (some old) incoming key = some fact) :
    old key = some fact := by
  exact (joined_fact_from_both old incoming key fact newFact).1

/-- This proves stability of the abstract intersection step only.
    Transfer functions, CFG reachability, changing entry assumptions and the
    interprocedural Pinned fixed point still require separate refinement. -/
theorem extra_join_blocks_previously_unsafe
    (initial : AnalyzerFacts) (incoming : List AnalyzerFacts)
    (key index : Nat)
    (blocked : elideFirst initial (initial key) index = false) :
    elideFirst (mergeMore initial incoming)
      (mergeMore initial incoming key) index = false := by
  cases h : elideFirst (mergeMore initial incoming)
      (mergeMore initial incoming key) index with
  | false => rfl
  | true =>
      have old := repeated_joins_cannot_create_elision initial incoming key index h
      simp [blocked] at old

end JabberwockFormal.LoopConvergence
