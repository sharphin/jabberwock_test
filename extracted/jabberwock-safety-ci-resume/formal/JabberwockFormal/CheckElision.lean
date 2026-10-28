import JabberwockFormal.EmittedGate

set_option autoImplicit false
set_option warningAsError true

namespace JabberwockFormal.CheckElision

/-!
A narrow, executable semantics for a generation-checked read and its
unchecked counterpart. The precondition for check elimination must be
proved independently at each real compiler transformation site.

This is NOT a proof about actual LLVM memory loads, pointer provenance,
concurrent writes, or either compiler's check-elision implementation.
-/

abbrev Word := BitVec 64

/-- The checked read either yields the payload or refuses the access. -/
def checkedRead (saved current : Word) (payload : Nat) : Option Nat :=
  if JabberwockFormal.EmittedGate.llvmContinues saved current
  then some payload else none

/-- Eliminating the check has precisely the indicated extra obligation. -/
def uncheckedRead (_saved _current : Word) (payload : Nat) : Option Nat :=
  some payload

/-- A valid generation witness suffices for check elimination in this model. -/
theorem elision_preserves_valid_read (saved current : Word) (payload : Nat)
    (valid : JabberwockFormal.EmittedGate.valid saved current) :
    checkedRead saved current payload = uncheckedRead saved current payload := by
  have h := (JabberwockFormal.EmittedGate.llvm_continues_iff saved current).mpr valid
  simp [checkedRead, uncheckedRead, h]

/-- Invalid generations must be rejected by the checked variant. -/
theorem checked_read_rejects_invalid (saved current : Word) (payload : Nat)
    (invalid : ¬ JabberwockFormal.EmittedGate.valid saved current) :
    checkedRead saved current payload = none := by
  have h : JabberwockFormal.EmittedGate.llvmContinues saved current = false := by
    cases decision : JabberwockFormal.EmittedGate.llvmContinues saved current with
    | false => rfl
    | true =>
        exact False.elim (invalid
          ((JabberwockFormal.EmittedGate.llvm_continues_iff saved current).mp decision))
  simp [checkedRead, h]

/-- Without the generation witness, unchecked reads can change the result. -/
theorem elision_changes_invalid_read (saved current : Word) (payload : Nat)
    (invalid : ¬ JabberwockFormal.EmittedGate.valid saved current) :
    checkedRead saved current payload ≠ uncheckedRead saved current payload := by
  rw [checked_read_rejects_invalid saved current payload invalid]
  simp [uncheckedRead]

/-- In particular, retirement is never an acceptable elision witness. -/
theorem retired_read_must_keep_check (saved : Word) (payload : Nat) :
    checkedRead saved JabberwockFormal.EmittedGate.retired payload = none := by
  simp [checkedRead, JabberwockFormal.EmittedGate.no_retired_gate]


/-- A valid generation is exactly the condition under which this one-read
    optimization agrees with the checked program. This is deliberately
    an iff, preventing unsound promotion based on only one direction. -/
theorem elision_equivalent_iff_valid (saved current : Word) (payload : Nat) :
    checkedRead saved current payload = uncheckedRead saved current payload ↔
      JabberwockFormal.EmittedGate.valid saved current := by
  constructor
  · intro equal
    by_cases good : JabberwockFormal.EmittedGate.valid saved current
    · exact good
    · exact False.elim
        ((elision_changes_invalid_read saved current payload good) equal)
  · intro good
    exact elision_preserves_valid_read saved current payload good

/-- Even a matching saved marker must not turn retired storage into a value. -/
theorem retired_equal_generation_rejected (payload : Nat) :
    checkedRead JabberwockFormal.EmittedGate.retired
      JabberwockFormal.EmittedGate.retired payload = none :=
  retired_read_must_keep_check _ payload


/-- One abstract access records its saved/current generation and payload.
    The real compiler-to-model correspondence is an additional obligation. -/
structure Access where
  saved : Word
  current : Word
  payload : Nat

/-- Observable results of a sequence of checked accesses. -/
def checkedTrace (accesses : List Access) : List (Option Nat) :=
  accesses.map (fun a => checkedRead a.saved a.current a.payload)

/-- Observable results of the same sequence with every check omitted. -/
def uncheckedTrace (accesses : List Access) : List (Option Nat) :=
  accesses.map (fun a => uncheckedRead a.saved a.current a.payload)

/-- Check elimination over a trace is correct if *every* eliminated check
    has its own valid-generation witness. A single successful read is not
    sufficient evidence for all subsequent accesses. -/
theorem valid_trace_elision_preserves_results
    (accesses : List Access)
    (validEverywhere : ∀ a ∈ accesses,
      JabberwockFormal.EmittedGate.valid a.saved a.current) :
    checkedTrace accesses = uncheckedTrace accesses := by
  induction accesses with
  | nil => rfl
  | cons a rest ih =>
      have headValid : JabberwockFormal.EmittedGate.valid a.saved a.current :=
        validEverywhere a (by simp)
      have tailValid : ∀ b ∈ rest,
          JabberwockFormal.EmittedGate.valid b.saved b.current := by
        intro b member
        exact validEverywhere b (by simp [member])
      have headEqual := elision_preserves_valid_read
        a.saved a.current a.payload headValid
      have tailEqual := ih tailValid
      simpa [checkedTrace, uncheckedTrace, headEqual] using
        congrArg (fun t => checkedRead a.saved a.current a.payload :: t) tailEqual


/-- The optimizer chooses which accesses omit generation checks.
    Keeping a check is always sound; an omitted check requires validity. -/
structure PlannedAccess where
  access : Access
  elide : Bool

def optimizedRead (p : PlannedAccess) : Option Nat :=
  if p.elide then
    uncheckedRead p.access.saved p.access.current p.access.payload
  else
    checkedRead p.access.saved p.access.current p.access.payload

def checkedPlan (plan : List PlannedAccess) : List (Option Nat) :=
  plan.map (fun p => checkedRead p.access.saved p.access.current p.access.payload)

def optimizedPlan (plan : List PlannedAccess) : List (Option Nat) :=
  plan.map optimizedRead

/-- A per-access soundness obligation. This is the condition that the real
    compiler analysis must establish before eliding a given check. -/
def PlanValid (plan : List PlannedAccess) : Prop :=
  ∀ p ∈ plan, p.elide = true →
    JabberwockFormal.EmittedGate.valid p.access.saved p.access.current

private theorem optimized_read_eq_iff (p : PlannedAccess) :
    checkedRead p.access.saved p.access.current p.access.payload =
      optimizedRead p ↔
    (p.elide = true →
      JabberwockFormal.EmittedGate.valid p.access.saved p.access.current) := by
  cases h : p.elide with
  | false =>
      simp [optimizedRead, h]
  | true =>
      simp only [optimizedRead, h, ite_true]
      constructor
      · intro equal _
        exact (elision_equivalent_iff_valid
          p.access.saved p.access.current p.access.payload).mp equal
      · intro valid
        exact (elision_equivalent_iff_valid
          p.access.saved p.access.current p.access.payload).mpr (valid trivial)

/-- Exact characterization of selective check elimination over a whole trace,
    not just the sufficient condition for eliding every check. -/
theorem plan_semantics_iff_valid (plan : List PlannedAccess) :
    checkedPlan plan = optimizedPlan plan ↔ PlanValid plan := by
  induction plan with
  | nil =>
      simp [checkedPlan, optimizedPlan, PlanValid]
  | cons p rest ih =>
      simp only [checkedPlan, optimizedPlan, List.map_cons, List.cons.injEq]
      constructor
      · intro equal
        have head := (optimized_read_eq_iff p).mp equal.1
        have tail := ih.mp equal.2
        intro q member elided
        rcases List.mem_cons.mp member with equal | later
        · subst q
          exact head elided
        · exact tail q later elided
      · intro valid
        constructor
        · apply (optimized_read_eq_iff p).mpr
          intro elided
          exact valid p (List.mem_cons_self ..) elided
        · apply ih.mpr
          intro q member elided
          exact valid q (List.mem_cons_of_mem p member) elided

/-- Checked read paths using each compiler's modeled generation predicate. -/
def jwcCheckedRead (a : Access) : Option Nat :=
  if JabberwockFormal.Jwc.ReferenceAccess.emittedGate a.saved a.current
  then some a.payload else none

def rustCheckedRead (a : Access) : Option Nat :=
  if JabberwockFormal.Rust.ReferenceAccess.emittedGate a.saved a.current
  then some a.payload else none

/-- Each compiler's modeled reference gate agrees with the common
    instruction-level evaluation for every bit-vector generation pair. -/
theorem jwc_read_refines_gate (a : Access) :
    jwcCheckedRead a = checkedRead a.saved a.current a.payload := by
  simp [jwcCheckedRead, checkedRead,
    JabberwockFormal.EmittedGate.jwc_gate_correspondence]

theorem rust_read_refines_gate (a : Access) :
    rustCheckedRead a = checkedRead a.saved a.current a.payload := by
  simp [rustCheckedRead, checkedRead,
    JabberwockFormal.EmittedGate.rust_gate_correspondence]

/-- The two independently defined gate models yield identical checked traces. -/
theorem modeled_compilers_agree_on_checked_plan (plan : List PlannedAccess) :
    plan.map (fun p => jwcCheckedRead p.access) =
    plan.map (fun p => rustCheckedRead p.access) := by
  apply List.map_congr_left
  intro p _
  rw [jwc_read_refines_gate, rust_read_refines_gate]

/-- The actual optimizer must establish PlanValid from its own analysis.
    Assuming that separate implementation proof, either modeled compiler
    can omit selected checks without changing its checked trace. -/
theorem jwc_selective_elision_refinement (plan : List PlannedAccess)
    (safe : PlanValid plan) :
    plan.map (fun p => jwcCheckedRead p.access) = optimizedPlan plan := by
  rw [← (plan_semantics_iff_valid plan).mpr safe]
  apply List.map_congr_left
  intro p _
  exact jwc_read_refines_gate p.access

theorem rust_selective_elision_refinement (plan : List PlannedAccess)
    (safe : PlanValid plan) :
    plan.map (fun p => rustCheckedRead p.access) = optimizedPlan plan := by
  rw [← (plan_semantics_iff_valid plan).mpr safe]
  apply List.map_congr_left
  intro p _
  exact rust_read_refines_gate p.access

/-- A minimal formal image of the real analyzers' is_live decision:
    Pinned, RefSlot, or RefTo(x) when the owner fact for x is Own.
    This is only the decision predicate; the correctness of inferred facts and
    their propagation through the real CFG remains a separate obligation. -/
inductive AnalyzerFact where
  | own
  | refTo (owner : Nat)
  | refSlot (slot : Nat)
  | pinned
  | refElem (list : Nat)
  deriving DecidableEq

abbrev AnalyzerFacts := Nat → Option AnalyzerFact

def analyzerIsLive (facts : AnalyzerFacts) : Option AnalyzerFact → Bool
  | some .pinned => true
  | some (.refSlot _) => true
  | some (.refTo x) => decide (facts x = some .own)
  | _ => false

/-- Interpret a positive abstract fact as a real generation witness. The
    invariant must be proved for each CFG transfer, meet and function call. -/
def FactInvariant (facts : AnalyzerFacts) (access : Access)
    (fact : Option AnalyzerFact) : Prop :=
  (fact = some .pinned ∨ (∃ slot, fact = some (.refSlot slot)) ∨
   (∃ owner, fact = some (.refTo owner) ∧ facts owner = some .own)) →
    JabberwockFormal.EmittedGate.valid access.saved access.current

/-- Once the implementation's fact invariant is established, every positive
    is_live result is sufficient to safely omit the runtime generation check. -/
theorem analyzer_live_implies_valid
    (facts : AnalyzerFacts) (access : Access) (fact : Option AnalyzerFact)
    (invariant : FactInvariant facts access fact)
    (live : analyzerIsLive facts fact = true) :
    JabberwockFormal.EmittedGate.valid access.saved access.current := by
  apply invariant
  cases fact with
  | none => simp [analyzerIsLive] at live
  | some f =>
      cases f with
      | own => simp [analyzerIsLive] at live
      | pinned => exact Or.inl rfl
      | refSlot slot => exact Or.inr (Or.inl ⟨slot, rfl⟩)
      | refElem xs => simp [analyzerIsLive] at live
      | refTo owner =>
          have owns : facts owner = some .own := by
            simpa [analyzerIsLive] using live
          exact Or.inr (Or.inr ⟨owner, rfl, owns⟩)

/-- An analyzer-generated decision certificate is sufficient for modeled
    trace equivalence, *provided* the independently proved fact invariants. -/
structure AnalyzedAccess where
  access : Access
  facts : AnalyzerFacts
  fact : Option AnalyzerFact
  invariant : FactInvariant facts access fact

def analyzedPlan (accesses : List AnalyzedAccess) : List PlannedAccess :=
  accesses.map (fun item => ⟨item.access, analyzerIsLive item.facts item.fact⟩)

theorem analyzed_plan_valid (accesses : List AnalyzedAccess) :
    PlanValid (analyzedPlan accesses) := by
  intro p member elided
  unfold analyzedPlan at member
  obtain ⟨a, belongs, equal⟩ := List.mem_map.mp member
  subst p
  exact analyzer_live_implies_valid a.facts a.access a.fact a.invariant elided

theorem analyzed_plan_preserves_evaluation (accesses : List AnalyzedAccess) :
    checkedPlan (analyzedPlan accesses) =
      optimizedPlan (analyzedPlan accesses) := by
  exact (plan_semantics_iff_valid _).mpr (analyzed_plan_valid accesses)

/-- A pointwise representation of the real analyzers' kill_all: removing
    an owner also invalidates every RefTo, RefSlot and RefElem fact
    depending on it. Other facts are unchanged. -/
def killOwnerFacts (facts : AnalyzerFacts) (owner : Nat) : AnalyzerFacts :=
  fun key =>
    if key = owner then none
    else match facts key with
      | some (.refTo target) =>
          if target = owner then none else facts key
      | some (.refSlot target) =>
          if target = owner then none else facts key
      | some (.refElem target) =>
          if target = owner then none else facts key
      | other => other

/-- Removing an owner cannot create a new positive liveness result for
    any fact that was already present in the old analysis. -/
theorem kill_owner_cannot_create_liveness
    (facts : AnalyzerFacts) (owner key : Nat) :
    analyzerIsLive (killOwnerFacts facts owner) ((killOwnerFacts facts owner) key) = true →
      analyzerIsLive facts (facts key) = true := by
  intro live
  by_cases same : key = owner
  · subst key
    simp [killOwnerFacts, analyzerIsLive] at live
  · cases h : facts key with
    | none =>
        simp [killOwnerFacts, same, h, analyzerIsLive] at live
    | some fact =>
        cases fact with
        | own => simp [killOwnerFacts, same, h, analyzerIsLive] at live
        | pinned => simp [analyzerIsLive]
        | refSlot slot => simp [analyzerIsLive]
        | refElem slot => simp [killOwnerFacts, same, h, analyzerIsLive] at live
        | refTo target =>
            by_cases affected : target = owner
            · simp [killOwnerFacts, same, h, affected, analyzerIsLive] at live
            · have oldOwn : facts target = some .own := by
                have newOwn : killOwnerFacts facts owner target = some .own := by
                  simpa [killOwnerFacts, same, h, affected, analyzerIsLive] using live
                by_cases targetOwner : target = owner
                · exact False.elim (affected targetOwner)
                · cases ht : facts target with
                  | none => simp [killOwnerFacts, targetOwner, ht] at newOwn
                  | some kind =>
                      cases kind <;> simp [killOwnerFacts, targetOwner, ht] at newOwn
                      exact Eq.refl _
              simp [analyzerIsLive, oldOwn]

/-- The fact join used at a CFG merge: retain a fact only if both
    incoming paths agree. -/
def meetFacts (left right : AnalyzerFacts) : AnalyzerFacts :=
  fun key => if left key = right key then left key else none

/-- A joined liveness claim is supported by both predecessor analyses.
    Consequently an intersection cannot introduce a previously absent proof
    that an owner is live. -/
theorem meet_live_implies_left
    (left right : AnalyzerFacts) (key : Nat)
    (live : analyzerIsLive (meetFacts left right)
      ((meetFacts left right) key) = true) :
    analyzerIsLive left (left key) = true := by
  by_cases h : left key = right key
  · have hk : meetFacts left right key = left key := by
      simp [meetFacts, h]
    rw [hk] at live
    cases f : left key with
    | none =>
        simp [analyzerIsLive, f] at live
    | some fact =>
        cases fact with
        | own => simp [analyzerIsLive, f] at live
        | pinned => simp [analyzerIsLive]
        | refSlot slot => simp [analyzerIsLive]
        | refElem slot => simp [analyzerIsLive, f] at live
        | refTo owner =>
            have joinedOwn : meetFacts left right owner = some .own := by
              simpa [analyzerIsLive, f] using live
            have oldOwn : left owner = some .own := by
              by_cases ho : left owner = right owner
              · simpa [meetFacts, ho] using joinedOwn
              · simp [meetFacts, ho] at joinedOwn
            simp [analyzerIsLive, oldOwn]
  · simp [meetFacts, h, analyzerIsLive] at live

theorem meet_live_implies_right
    (left right : AnalyzerFacts) (key : Nat)
    (live : analyzerIsLive (meetFacts left right)
      ((meetFacts left right) key) = true) :
    analyzerIsLive right (right key) = true := by
  have comm : meetFacts left right = meetFacts right left := by
    funext k
    simp only [meetFacts]
    by_cases h : left k = right k
    · simp [h]
    · have hr : right k ≠ left k := Ne.symm h
      simp [h, hr]
  rw [comm] at live
  exact meet_live_implies_left right left key live

/-- A straight-line transfer of an owner's destruction never retains
    the stale fact that the destroyed owner is still present. -/
theorem killed_owner_absent (facts : AnalyzerFacts) (owner : Nat) :
    killOwnerFacts facts owner owner = none := by
  simp [killOwnerFacts]

/-- Each direct borrower of a destroyed owner is removed, independent
    of other facts in the analysis state. -/
theorem killed_owner_invalidates_direct_borrow
    (facts : AnalyzerFacts) (owner borrower : Nat)
    (distinct : borrower ≠ owner)
    (borrow : facts borrower = some (.refTo owner)) :
    killOwnerFacts facts owner borrower = none := by
  simp [killOwnerFacts, distinct, borrow]

/-- A conservative ownership transfer does not create a new check-elision
    candidate at the destruction site. -/
theorem killed_borrow_cannot_elide
    (facts : AnalyzerFacts) (owner borrower : Nat)
    (distinct : borrower ≠ owner)
    (borrow : facts borrower = some (.refTo owner)) :
    analyzerIsLive (killOwnerFacts facts owner)
      (killOwnerFacts facts owner borrower) = false := by
  simp [killOwnerFacts, distinct, borrow, analyzerIsLive]

/-- Overwriting an owner invalidates direct RefTo and RefElem aliases,
    but preserves RefSlot aliases to the storage slot itself. -/
def overwriteOwnerFacts (facts : AnalyzerFacts) (owner : Nat) : AnalyzerFacts :=
  fun key =>
    if key = owner then none
    else match facts key with
      | some (.refTo target) =>
          if target = owner then none else facts key
      | some (.refElem target) =>
          if target = owner then none else facts key
      | other => other

theorem overwritten_owner_absent (facts : AnalyzerFacts) (owner : Nat) :
    overwriteOwnerFacts facts owner owner = none := by
  simp [overwriteOwnerFacts]

theorem overwritten_owner_invalidates_borrow
    (facts : AnalyzerFacts) (owner borrower : Nat)
    (distinct : borrower ≠ owner)
    (borrow : facts borrower = some (.refTo owner)) :
    overwriteOwnerFacts facts owner borrower = none := by
  simp [overwriteOwnerFacts, distinct, borrow]

theorem overwritten_owner_invalidates_element
    (facts : AnalyzerFacts) (owner borrower : Nat)
    (distinct : borrower ≠ owner)
    (borrow : facts borrower = some (.refElem owner)) :
    overwriteOwnerFacts facts owner borrower = none := by
  simp [overwriteOwnerFacts, distinct, borrow]

/-- Reassigning an owner does not revoke references to the stable local
    storage slot, unlike dropping or moving the slot itself. -/
theorem overwrite_preserves_other_slot
    (facts : AnalyzerFacts) (owner borrower slot : Nat)
    (distinct : borrower ≠ owner)
    (borrow : facts borrower = some (.refSlot slot)) :
    overwriteOwnerFacts facts owner borrower = some (.refSlot slot) := by
  simp [overwriteOwnerFacts, distinct, borrow]

/-- Invalidation of all references into movable list elements, matching
    the conservative kill_elems transfer in both IR analyzers. -/
def killElementFacts (facts : AnalyzerFacts) : AnalyzerFacts :=
  fun key => match facts key with
    | some (.refElem _) => none
    | other => other

theorem kill_elements_removes_element_facts
    (facts : AnalyzerFacts) (key list : Nat)
    (borrow : facts key = some (.refElem list)) :
    killElementFacts facts key = none := by
  simp [killElementFacts, borrow]

theorem kill_elements_preserves_owner_facts
    (facts : AnalyzerFacts) (key : Nat)
    (own : facts key = some .own) :
    killElementFacts facts key = some .own := by
  simp [killElementFacts, own]

theorem kill_elements_preserves_direct_borrows
    (facts : AnalyzerFacts) (key owner : Nat)
    (borrow : facts key = some (.refTo owner)) :
    killElementFacts facts key = some (.refTo owner) := by
  simp [killElementFacts, borrow]

/-- Eliminating element facts does not create positive liveness claims,
    including claims contingent on the existence of an owner fact. -/
theorem kill_elements_cannot_create_liveness
    (facts : AnalyzerFacts) (key : Nat)
    (live : analyzerIsLive (killElementFacts facts)
      (killElementFacts facts key) = true) :
    analyzerIsLive facts (facts key) = true := by
  cases h : facts key with
  | none => simp [killElementFacts, analyzerIsLive, h] at live
  | some fact =>
      cases fact with
      | own => simp [killElementFacts, analyzerIsLive, h] at live
      | pinned => simp [analyzerIsLive]
      | refSlot slot => simp [analyzerIsLive]
      | refElem list => simp [killElementFacts, analyzerIsLive, h] at live
      | refTo owner =>
          have present : killElementFacts facts owner = some .own := by
            simpa [killElementFacts, analyzerIsLive, h] using live
          have oldOwn : facts owner = some .own := by
            cases ho : facts owner with
            | none => simp [killElementFacts, ho] at present
            | some ownerFact =>
                cases ownerFact <;> simp [killElementFacts, ho] at present
                · exact Eq.refl _
          simp [analyzerIsLive, oldOwn]

end JabberwockFormal.CheckElision
