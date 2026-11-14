import JabberwockFormal.CheckElision

set_option autoImplicit false
set_option warningAsError true

namespace JabberwockFormal.ElisionDecision

open JabberwockFormal.CheckElision

/-- The first-projection decision in src/ir/analyze.rs and jwc/analyze.jw.
    The Cfg fact propagation is NOT assumed sound by this definition. -/
def safeFact (facts : AnalyzerFacts) (root : Option AnalyzerFact) : Bool :=
  decide (root = some .own) || analyzerIsLive facts root

def elideFirst (facts : AnalyzerFacts) (root : Option AnalyzerFact)
    (projectionIndex : Nat) : Bool :=
  (projectionIndex == 0) && safeFact facts root

/-- IR Rust writes Check::Elided exactly for first projections with positive
    facts. Both implementations use the same predicate at this site. -/
theorem elision_iff_first_and_safe
    (facts : AnalyzerFacts) (root : Option AnalyzerFact) (i : Nat) :
    elideFirst facts root i = true ↔
      i = 0 ∧ safeFact facts root = true := by
  simp [elideFirst]

theorem later_projections_keep_check
    (facts : AnalyzerFacts) (root : Option AnalyzerFact) (i : Nat)
    (later : i ≠ 0) :
    elideFirst facts root i = false := by
  simp [elideFirst, later]

/-- This premise is the still-unproved connection from actual CFG/dataflow
    facts to memory lifetime and generation validity. 'Own' must also carry
    a real validity witness; a positive fact alone is insufficient. -/
def FactSoundAtAccess (facts : AnalyzerFacts) (root : Option AnalyzerFact)
    (a : Access) : Prop :=
  safeFact facts root = true →
    JabberwockFormal.EmittedGate.valid a.saved a.current

theorem elision_requires_valid
    (facts : AnalyzerFacts) (root : Option AnalyzerFact)
    (i : Nat) (a : Access)
    (sound : FactSoundAtAccess facts root a)
    (elided : elideFirst facts root i = true) :
    JabberwockFormal.EmittedGate.valid a.saved a.current := by
  exact sound ((elision_iff_first_and_safe facts root i).mp elided).2

/-- Under the explicitly stated invariant, the selected runtime check
    can be skipped without changing the abstract read observation. -/
theorem sound_elision_preserves_read
    (facts : AnalyzerFacts) (root : Option AnalyzerFact)
    (i : Nat) (a : Access)
    (sound : FactSoundAtAccess facts root a)
    (elided : elideFirst facts root i = true) :
    checkedRead a.saved a.current a.payload =
      uncheckedRead a.saved a.current a.payload := by
  exact elision_preserves_valid_read a.saved a.current a.payload
    (elision_requires_valid facts root i a sound elided)

end JabberwockFormal.ElisionDecision
