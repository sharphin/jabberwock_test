import JabberwockFormal.ElisionDecision

set_option autoImplicit false
set_option warningAsError true

namespace JabberwockFormal.CfgMeetElision

open JabberwockFormal.CheckElision
open JabberwockFormal.ElisionDecision

/-- A nonempty joined fact must occur on both incoming edges. -/
theorem joined_fact_from_both (left right : AnalyzerFacts)
    (key : Nat) (fact : AnalyzerFact)
    (joined : meetFacts left right key = some fact) :
    left key = some fact ∧ right key = some fact := by
  by_cases h : left key = right key
  · have hl : left key = some fact := by
      simpa [meetFacts, h] using joined
    exact ⟨hl, h.symm.trans hl⟩
  · simp [meetFacts, h] at joined

/-- An Own fact cannot be created by joining two paths. -/
theorem joined_own_from_both (left right : AnalyzerFacts) (key : Nat)
    (joined : meetFacts left right key = some .own) :
    left key = some .own ∧ right key = some .own :=
  joined_fact_from_both left right key .own joined

/-- Positive analyzer liveness at the join is supported on both edges.
    This uses the previously kernel-checked liveness lemmas, including
    dependent RefTo owner facts. -/
theorem joined_live_from_both (left right : AnalyzerFacts) (key : Nat)
    (live : analyzerIsLive (meetFacts left right)
      (meetFacts left right key) = true) :
    analyzerIsLive left (left key) = true ∧
      analyzerIsLive right (right key) = true := by
  exact ⟨meet_live_implies_left left right key live,
    meet_live_implies_right left right key live⟩

/-- The join cannot create a new check-elision candidate on either edge.
    Unlike liveness alone, this also covers the Own alternative in safeFact. -/
theorem joined_safe_from_both (left right : AnalyzerFacts) (key : Nat)
    (safe : safeFact (meetFacts left right)
      (meetFacts left right key) = true) :
    safeFact left (left key) = true ∧
      safeFact right (right key) = true := by
  by_cases owned : meetFacts left right key = some .own
  · obtain ⟨hl, hr⟩ := joined_own_from_both left right key owned
    simp [safeFact, hl, hr]
  · have live : analyzerIsLive (meetFacts left right)
        (meetFacts left right key) = true := by
      simpa [safeFact, owned] using safe
    obtain ⟨hl, hr⟩ := joined_live_from_both left right key live
    simp [safeFact, hl, hr]

/-- The final first-dereference elision decision at a CFG join
    requires the same decision to be possible on both incoming paths. -/
theorem joined_elision_from_both (left right : AnalyzerFacts)
    (key index : Nat)
    (elided : elideFirst (meetFacts left right)
      (meetFacts left right key) index = true) :
    elideFirst left (left key) index = true ∧
      elideFirst right (right key) index = true := by
  obtain ⟨first, safe⟩ :=
    (elision_iff_first_and_safe _ _ _).mp elided
  obtain ⟨hl, hr⟩ := joined_safe_from_both left right key safe
  constructor
  · exact (elision_iff_first_and_safe _ _ _).mpr ⟨first, hl⟩
  · exact (elision_iff_first_and_safe _ _ _).mpr ⟨first, hr⟩

/-- Contrapositive safety condition: if either reachable edge cannot
    justify elision, joining cannot justify it either. -/
theorem unsafe_edge_blocks_join (left right : AnalyzerFacts)
    (key index : Nat)
    (blocked : elideFirst left (left key) index = false ∨
      elideFirst right (right key) index = false) :
    elideFirst (meetFacts left right)
      (meetFacts left right key) index = false := by
  cases h : elideFirst (meetFacts left right)
      (meetFacts left right key) index with
  | false => rfl
  | true =>
      obtain ⟨hl, hr⟩ := joined_elision_from_both left right key index h
      cases blocked with
      | inl bad => simp [bad] at hl
      | inr bad => simp [bad] at hr

end JabberwockFormal.CfgMeetElision
