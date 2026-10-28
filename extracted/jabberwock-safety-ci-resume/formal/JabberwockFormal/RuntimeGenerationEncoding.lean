import Lean.Elab.Tactic.Omega

set_option autoImplicit false
set_option warningAsError true

namespace JabberwockFormal.RuntimeGenerationEncoding

/-
  Arithmetic model of runtime/jw_rt.c:
    GEN_COUNTER_MASK = 2^62 - 1
    bit 62 is the retained marker
    UINT64_MAX is the retired sentinel
  The low two state bits are part of the masked counter, as in the C code.
  This proves the arithmetic encoding only; it does not prove C pointer
  lifetime, atomic ordering, or the allocator's storage invariants.
-/

def radix : Nat := 2 ^ 62
def maxCounter : Nat := radix - 1
/-- 4 * 2^62 - 1 = 2^64 - 1, the C runtime's GEN_RETIRED sentinel. -/
def retiredWord : Nat := 4 * radix - 1

/-- Valid non-retired packed word: optional bit-62 marker plus a 62-bit field. -/
def pack (retained : Bool) (counter : Nat) : Nat :=
  (if retained then radix else 0) + counter

/-- Exact arithmetic condition checked by gen_has_room on the masked field. -/
def hasRoom (counter step : Nat) : Prop :=
  counter + step ≤ maxCounter

instance (counter step : Nat) : Decidable (hasRoom counter step) := by
  unfold hasRoom
  infer_instance

/-- Parameterized form of the runtime capacity check. The parameter
    permits reasoning about JW_TEST_GENERATION_LIMIT without assuming the
    production default limit. -/
def capacityCheckAt (limit counter step : Nat) : Prop :=
  step ≤ limit ∧ counter ≤ limit - step

theorem capacityCheckAt_iff_sum (limit counter step : Nat) :
    capacityCheckAt limit counter step ↔ counter + step ≤ limit := by
  unfold capacityCheckAt
  omega

/-- The normalized sum bound is equivalent to the subtraction check used by
    gen_has_room, provided the counter and step are natural numbers. -/
def cCapacityCheck (counter step : Nat) : Prop :=
  step ≤ maxCounter ∧ counter ≤ maxCounter - step

theorem cCapacityCheck_iff_hasRoom (counter step : Nat) :
    cCapacityCheck counter step ↔ hasRoom counter step := by
  unfold cCapacityCheck hasRoom
  omega

/-- On success, gen_advance increments the packed word; otherwise it retires it. -/
def advance (retained : Bool) (counter step : Nat) : Nat :=
  if hasRoom counter step then pack retained (counter + step) else retiredWord

theorem hasRoom_bounds_sum {counter step : Nat}
    (room : hasRoom counter step) :
    counter + step ≤ maxCounter := room

theorem hasRoom_prevents_counter_wrap {counter step : Nat}
    (room : hasRoom counter step) :
    counter + step < radix := by
  have bound := hasRoom_bounds_sum room
  have radixPositive : 0 < radix := by
    unfold radix
    decide
  have radixOne : 1 ≤ radix := by
    unfold radix
    decide
  have successorBound : maxCounter + 1 = radix := by
    unfold maxCounter
    exact Nat.sub_add_cancel radixOne
  omega

theorem packed_word_is_below_retired
    (retained : Bool) {counter : Nat}
    (bounded : counter < radix) :
    pack retained counter < retiredWord := by
  cases retained <;> simp [pack, retiredWord, radix] at * <;> omega

/-- A successful positive increment cannot recreate the previous packed word. -/
theorem advance_changes_word
    (retained : Bool) {counter step : Nat}
    (room : hasRoom counter step) (positive : 0 < step) :
    advance retained counter step ≠ pack retained counter := by
  unfold advance
  rw [if_pos room]
  intro same
  cases retained <;> simp [pack] at same <;> omega

/-- The saved pre-free word cannot pass the runtime's equality gate after a
    successful increment, because the packed generation word has changed. -/
def accepts (saved current : Nat) : Prop :=
  current ≠ retiredWord ∧ saved = current

theorem old_word_rejected_after_advance
    (retained : Bool) {counter step : Nat}
    (room : hasRoom counter step) (positive : 0 < step) :
    ¬ accepts (pack retained counter) (advance retained counter step) := by
  unfold accepts advance
  rw [if_pos room]
  intro accepted
  have changed : pack retained (counter + step) ≠ pack retained counter := by
    simpa [advance, room] using
      (advance_changes_word retained room positive)
  exact changed accepted.2.symm

/-- If gen_has_room fails, the runtime retires the block; retired words never
    satisfy the reference-generation gate, regardless of the saved word. -/
theorem old_word_rejected_on_retirement
    (saved : Nat) (retained : Bool) (counter step : Nat)
    (noRoom : ¬ hasRoom counter step) :
    ¬ accepts saved (advance retained counter step) := by
  unfold accepts advance
  rw [if_neg noRoom]
  simp

/-- Model of the complete gen_has_room guard from jw_rt.c after extracting
    the masked counter: the block must not be retired, the step must fit,
    and the subtraction-based capacity check must pass. -/
def runtimeGuard (retired : Bool) (counter step : Nat) : Prop :=
  retired = false ∧ cCapacityCheck counter step

instance (retired : Bool) (counter step : Nat) :
    Decidable (runtimeGuard retired counter step) := by
  unfold runtimeGuard cCapacityCheck
  infer_instance

/-- Model of gen_advance: a failed guard retires the block; a successful
    guard advances the packed generation word. -/
def runtimeAdvance (retired retained : Bool) (counter step : Nat) : Nat :=
  if runtimeGuard retired counter step
  then pack retained (counter + step)
  else retiredWord

theorem runtimeGuard_matches_arithmetic {counter step : Nat} :
    runtimeGuard false counter step ↔ hasRoom counter step := by
  simp [runtimeGuard, cCapacityCheck_iff_hasRoom]

/-- Under the exact successful C guard, the packed generation changes, so
    the saved old generation cannot satisfy the runtime equality check. -/
theorem runtimeAdvance_rejects_old_word
    (retained : Bool) {counter step : Nat}
    (room : runtimeGuard false counter step) (positive : 0 < step) :
    ¬ accepts (pack retained counter)
      (runtimeAdvance false retained counter step) := by
  unfold accepts runtimeAdvance
  rw [if_pos room]
  intro accepted
  have changed : pack retained (counter + step) ≠ pack retained counter := by
    cases retained <;> simp [pack] at * <;> omega
  exact changed accepted.2.symm

/-- If the C capacity guard fails, gen_advance retires the block and the
    reference gate rejects any saved generation word. -/
theorem runtimeAdvance_rejects_after_retirement
    (saved : Nat) (retained : Bool) (counter step : Nat)
    (noRoom : ¬ runtimeGuard false counter step) :
    ¬ accepts saved (runtimeAdvance false retained counter step) := by
  unfold accepts runtimeAdvance
  rw [if_neg noRoom]
  simp

/-- Extract the low 62-bit generation counter from a packed runtime word.
    For a valid packed word this is exactly the C expression gen & MASK. -/
def wordCounter (word : Nat) : Nat := word % radix

/-- Concrete C guard over the packed generation word. The input word is
    modeled as a natural-number encoding; valid-word and uint64 representation
    assumptions are stated separately rather than hidden in this predicate. -/
def concreteGuard (word step : Nat) : Prop :=
  word ≠ retiredWord ∧ step ≤ maxCounter ∧ wordCounter word ≤ maxCounter - step

instance (word step : Nat) : Decidable (concreteGuard word step) := by
  unfold concreteGuard wordCounter
  infer_instance

/-- The low-bit mask extraction of a packed valid word yields its counter. -/
theorem packed_counter_mod
    (retained : Bool) {counter : Nat}
    (bounded : counter < radix) :
    wordCounter (pack retained counter) = counter := by
  cases retained <;> simp [wordCounter, pack, Nat.mod_eq_of_lt bounded]

/-- Addition of a step to a packed word agrees with adding to its counter
    while using unbounded natural-number arithmetic. The guard supplies the
    range condition needed to interpret this as non-wrapping uint64 addition. -/
theorem packed_word_step
    (retained : Bool) (counter step : Nat) :
    pack retained counter + step = pack retained (counter + step) := by
  cases retained <;> simp [pack] <;> omega

/-- For valid packed words, the concrete masked C guard is equivalent to the
    abstract counter-capacity predicate. -/
theorem concreteGuard_matches_abstract
    (retained : Bool) {counter step : Nat}
    (bounded : counter < radix) :
    concreteGuard (pack retained counter) step ↔ hasRoom counter step := by
  have notRetired : pack retained counter ≠ retiredWord := by
    have below := packed_word_is_below_retired retained bounded
    omega
  have modulo := packed_counter_mod retained bounded
  unfold concreteGuard
  rw [modulo]
  constructor
  · rintro ⟨_, stepBound, counterBound⟩
    exact (cCapacityCheck_iff_hasRoom counter step).1 ⟨stepBound, counterBound⟩
  · intro room
    have capacity : cCapacityCheck counter step :=
      (cCapacityCheck_iff_hasRoom counter step).2 room
    exact ⟨notRetired, capacity.1, capacity.2⟩

/-- Concrete packed-word update and the abstract update produce the same
    mathematical word for a valid input word. This bridges the masked guard
    and update expression, but not C atomics, pointers, or allocator behavior. -/
def concreteAdvance (word step : Nat) : Nat :=
  if concreteGuard word step then word + step else retiredWord

theorem concreteAdvance_matches_abstract
    (retained : Bool) {counter step : Nat}
    (bounded : counter < radix) :
    concreteAdvance (pack retained counter) step = advance retained counter step := by
  by_cases room : hasRoom counter step
  · have concrete : concreteGuard (pack retained counter) step :=
      (concreteGuard_matches_abstract retained bounded).2 room
    simp [concreteAdvance, advance, concrete, room, packed_word_step]
  · have concrete : ¬ concreteGuard (pack retained counter) step :=
      fun h => room ((concreteGuard_matches_abstract retained bounded).1 h)
    simp [concreteAdvance, advance, concrete, room]

/-- The modulus of a C uint64_t value. We use Nat as a representation
    domain and reduce addition modulo 2^64 to model unsigned-word addition. -/
def uint64Modulus : Nat := 2 ^ 64

/-- Mathematical model of unsigned 64-bit addition, including wraparound. -/
def uint64Add (left right : Nat) : Nat :=
  (left + right) % uint64Modulus

/-- When the runtime capacity guard accepts an increment, the packed word
    addition is strictly below 2^64. Therefore C uint64_t addition cannot
    wrap on this successful path and agrees with the unbounded Nat model.
    This theorem covers arithmetic only; it does not establish that the C
    compiler implements the source expression or prove pointer/atomic safety. -/
theorem packed_word_step_no_uint64_wrap
    (retained : Bool) {counter step : Nat}
    (room : hasRoom counter step) :
    uint64Add (pack retained counter) step =
      pack retained (counter + step) := by
  have sumBound : counter + step < radix :=
    hasRoom_prevents_counter_wrap room
  have packedBound : pack retained (counter + step) < uint64Modulus := by
    have below := packed_word_is_below_retired retained sumBound
    have sentinelPlus : retiredWord + 1 = uint64Modulus := by
      unfold retiredWord uint64Modulus radix
      decide
    omega
  unfold uint64Add
  rw [packed_word_step]
  exact Nat.mod_eq_of_lt packedBound

/-- The no-wrap result also holds for any compile-time generation limit
    no greater than the production counter mask, including the runtime's
    bounded test override. -/
theorem packed_word_step_no_uint64_wrap_at_limit
    (retained : Bool) {counter step limit : Nat}
    (limitBound : limit ≤ maxCounter)
    (room : capacityCheckAt limit counter step) :
    uint64Add (pack retained counter) step =
      pack retained (counter + step) := by
  have sumBound : counter + step ≤ limit :=
    (capacityCheckAt_iff_sum limit counter step).1 room
  have defaultRoom : hasRoom counter step := by
    unfold hasRoom
    omega
  exact packed_word_step_no_uint64_wrap retained defaultRoom

/-- Every modeled live word differs from the retired sentinel. -/
theorem live_word_not_retired
    (retained : Bool) {counter : Nat}
    (bounded : counter < radix) :
    pack retained counter ≠ retiredWord := by
  have below := packed_word_is_below_retired retained bounded
  omega


/-- Model the block_free / remember_reuse policy: reserve room for both
    the release increment and a later reuse increment; if either transition
    cannot be represented, retire the block instead of returning it to a
    free list. This remains an arithmetic lifecycle model, not a proof of
    the allocator's concrete pointer/list behavior. -/
def freeThenReuse (retained : Bool) (counter : Nat) : Nat :=
  if hasRoom counter 2 then
    if hasRoom (counter + 1) 1 then pack retained (counter + 2)
    else retiredWord
  else retiredWord

/-- Reserving two generation increments guarantees the first increment
    leaves enough room for the reuse increment. -/
theorem room_two_implies_room_after_first {counter : Nat}
    (room : hasRoom counter 2) :
    hasRoom (counter + 1) 1 := by
  unfold hasRoom at *
  omega

/-- The concrete two-stage policy reduces to a two-step generation advance
    when capacity is available, and otherwise selects the retired sentinel. -/
theorem freeThenReuse_matches_two_step (retained : Bool) (counter : Nat) :
    freeThenReuse retained counter =
      if hasRoom counter 2 then pack retained (counter + 2) else retiredWord := by
  unfold freeThenReuse
  by_cases room : hasRoom counter 2
  · have nextRoom := room_two_implies_room_after_first room
    simp [room, nextRoom]
  · simp [room]

/-- A reference snapshot taken before free/reuse cannot be accepted after
    the two-step policy: the generation changed, or the block was retired. -/
theorem freeThenReuse_rejects_old_reference
    (retained : Bool) (counter : Nat) :
    ¬ accepts (pack retained counter) (freeThenReuse retained counter) := by
  unfold freeThenReuse
  by_cases room : hasRoom counter 2
  · have nextRoom := room_two_implies_room_after_first room
    rw [if_pos room, if_pos nextRoom]
    intro accepted
    have bound : counter + 2 < radix := by
      have sumBound := hasRoom_bounds_sum room
      unfold maxCounter radix at *
      omega
    have changed : pack retained (counter + 2) ≠ pack retained counter := by
      cases retained <;> simp [pack] <;> omega
    exact changed accepted.2.symm
  · rw [if_neg room]
    simp [accepts]

end JabberwockFormal.RuntimeGenerationEncoding
