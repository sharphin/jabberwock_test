import JabberwockFormal.ElisionDecision

set_option autoImplicit false
set_option warningAsError true

namespace JabberwockFormal.OwnershipInvalidation

open JabberwockFormal.CheckElision
open JabberwockFormal.ElisionDecision

/-- A removed analysis entry cannot justify omitting any check. This is an
    analyzer-decision theorem, not an assertion about physical memory. -/
theorem missing_fact_requires_check (facts : AnalyzerFacts) (key index : Nat)
    (missing : facts key = none) :
    elideFirst facts (facts key) index = false := by
  simp [elideFirst, safeFact, analyzerIsLive, missing]

/-- Dropping/moving an owner removes the owner's own evidence. -/
theorem owner_removed (facts : AnalyzerFacts) (owner : Nat) :
    killOwnerFacts facts owner owner = none := by
  simp [killOwnerFacts]

/-- A direct borrower must not survive owner destruction. -/
theorem direct_borrow_removed
    (facts : AnalyzerFacts) (owner borrower : Nat)
    (different : borrower ≠ owner)
    (borrow : facts borrower = some (.refTo owner)) :
    killOwnerFacts facts owner borrower = none := by
  simp [killOwnerFacts, different, borrow]

/-- A borrower of the destroyed variable slot must also be invalidated. -/
theorem slot_borrow_removed
    (facts : AnalyzerFacts) (owner borrower : Nat)
    (different : borrower ≠ owner)
    (borrow : facts borrower = some (.refSlot owner)) :
    killOwnerFacts facts owner borrower = none := by
  simp [killOwnerFacts, different, borrow]

/-- An element borrower dependent on the owner is invalidated as well. -/
theorem element_borrow_removed
    (facts : AnalyzerFacts) (owner borrower : Nat)
    (different : borrower ≠ owner)
    (borrow : facts borrower = some (.refElem owner)) :
    killOwnerFacts facts owner borrower = none := by
  simp [killOwnerFacts, different, borrow]

/-- After owner destruction, the former owner cannot cause first-projection
    check elision. -/
theorem destroyed_owner_requires_check
    (facts : AnalyzerFacts) (owner index : Nat) :
    elideFirst (killOwnerFacts facts owner)
      (killOwnerFacts facts owner owner) index = false := by
  exact missing_fact_requires_check _ _ _ (owner_removed facts owner)

/-- No removed direct borrower can justify an unchecked dereference. -/
theorem destroyed_direct_borrow_requires_check
    (facts : AnalyzerFacts) (owner borrower index : Nat)
    (different : borrower ≠ owner)
    (borrow : facts borrower = some (.refTo owner)) :
    elideFirst (killOwnerFacts facts owner)
      (killOwnerFacts facts owner borrower) index = false := by
  exact missing_fact_requires_check _ _ _ 
    (direct_borrow_removed facts owner borrower different borrow)

theorem destroyed_slot_borrow_requires_check
    (facts : AnalyzerFacts) (owner borrower index : Nat)
    (different : borrower ≠ owner)
    (borrow : facts borrower = some (.refSlot owner)) :
    elideFirst (killOwnerFacts facts owner)
      (killOwnerFacts facts owner borrower) index = false := by
  exact missing_fact_requires_check _ _ _
    (slot_borrow_removed facts owner borrower different borrow)

theorem destroyed_element_borrow_requires_check
    (facts : AnalyzerFacts) (owner borrower index : Nat)
    (different : borrower ≠ owner)
    (borrow : facts borrower = some (.refElem owner)) :
    elideFirst (killOwnerFacts facts owner)
      (killOwnerFacts facts owner borrower) index = false := by
  exact missing_fact_requires_check _ _ _
    (element_borrow_removed facts owner borrower different borrow)

end JabberwockFormal.OwnershipInvalidation
