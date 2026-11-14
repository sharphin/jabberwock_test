import Lean.Elab.Tactic.Omega

set_option autoImplicit false
set_option warningAsError true

namespace JabberwockFormal.RuntimeGenerationBitVec

/-
  BitVec-level model of the uint64_t arithmetic in runtime/jw_rt.c.
  Unlike the Nat model, this module uses 64-bit modular operations directly.
  The source matcher binds this model to a narrow, exact C source pattern;
  that structural binding is not a proof of C compiler correctness or of
  pointer lifetime, concurrency, allocator behavior, or the whole runtime.
-/

abbrev Word := BitVec 64

def radix : Nat := 2 ^ 62
def limitMaximum : Nat := radix - 1
def wordMaximum : Nat := 2 ^ 64 - 1

def retiredWord : Word := BitVec.ofNat 64 wordMaximum
def counterMask : Word := BitVec.ofNat 64 limitMaximum

/-- Exact mathematical interpretation of the C unsigned comparisons in
    gen_has_room. The step bound is explicit so limit - step is non-wrapping
    on every accepted path. The masked expression remains a BitVec operation. -/
def guardSpec (gen step limit : Word) : Prop :=
  gen ≠ retiredWord ∧
  step ≤ limit ∧
  (gen &&& counterMask) ≤ limit - step

def guard (gen step limit : Word) : Bool :=
  decide (gen ≠ retiredWord) && step.ule limit &&
    (gen &&& counterMask).ule (limit - step)

theorem guard_iff_spec (gen step limit : Word) :
    guard gen step limit = true ↔ guardSpec gen step limit := by
  simp [guard, guardSpec, BitVec.ule_eq_decide_le, and_assoc]

theorem guard_rejects_retired (step limit : Word) :
    guard retiredWord step limit = false := by
  simp [guard]

/-- The C expression `gen & GEN_COUNTER_MASK` is exactly the
    low-62-bit counter extraction, represented as a natural-number modulus.
    This is the bridge from the concrete 64-bit word to the Nat arithmetic
    model used by RuntimeGenerationEncoding. -/
theorem masked_counter_matches_mod (gen : Word) :
    (gen &&& counterMask).toNat = gen.toNat % radix := by
  rw [BitVec.toNat_and]
  have maskNat : counterMask.toNat = 2 ^ 62 - 1 := by
    simp [counterMask, limitMaximum, radix]
  rw [maskNat]
  exact Nat.and_two_pow_sub_one_eq_mod gen.toNat 62

/-- On the accepted branch, uint64_t subtraction is ordinary natural
    subtraction: the explicit unsigned comparison prevents underflow. -/
theorem unsigned_subtraction_matches_nat
    (limit step : Word) (ordered : step ≤ limit) :
    (limit - step).toNat = limit.toNat - step.toNat := by
  exact BitVec.toNat_sub_of_le ordered

/-- The complete BitVec guard specification agrees with the Nat
    counter-capacity check. This connects the concrete mask and unsigned
    subtraction to the arithmetic abstraction; it is still not a proof that
    a C compiler or the complete runtime refines this model. -/
theorem guardSpec_matches_nat (gen step limit : Word) :
    guardSpec gen step limit ↔
      gen ≠ retiredWord ∧ step.toNat ≤ limit.toNat ∧
        gen.toNat % radix ≤ limit.toNat - step.toNat := by
  constructor
  · rintro ⟨notRetired, stepLe, counterLe⟩
    have stepLeNat : step.toNat ≤ limit.toNat := by
      rw [BitVec.le_def] at stepLe
      exact stepLe
    have counterLeNat :
        (gen &&& counterMask).toNat ≤ (limit - step).toNat := by
      rw [BitVec.le_def] at counterLe
      exact counterLe
    have subNat := unsigned_subtraction_matches_nat limit step stepLe
    rw [masked_counter_matches_mod] at counterLeNat
    rw [subNat] at counterLeNat
    exact ⟨notRetired, stepLeNat, counterLeNat⟩
  · rintro ⟨notRetired, stepLeNat, counterLeNat⟩
    have stepLe : step ≤ limit := by
      rw [BitVec.le_def]
      exact stepLeNat
    have subNat := unsigned_subtraction_matches_nat limit step stepLe
    have counterLe : (gen &&& counterMask) ≤ limit - step := by
      rw [BitVec.le_def]
      rw [masked_counter_matches_mod, subNat]
      exact counterLeNat
    exact ⟨notRetired, stepLe, counterLe⟩

/-- The executable BitVec guard and its Nat-level interpretation agree. -/
theorem guard_matches_nat (gen step limit : Word) :
    guard gen step limit = true ↔
      gen ≠ retiredWord ∧ step.toNat ≤ limit.toNat ∧
        gen.toNat % radix ≤ limit.toNat - step.toNat := by
  rw [guard_iff_spec]
  exact guardSpec_matches_nat gen step limit

/-- If the packed runtime word has bit 63 clear and the configured limit is
    below 2^62, a successful guard proves that uint64_t addition does not wrap.
    This is an arithmetic theorem about the BitVec model, not a pointer-safety
    or concurrency theorem. -/
theorem accepted_increment_no_wrap
    (gen step limit : Word)
    (validGen : gen.toNat < 2 ^ 63)
    (validLimit : limit.toNat < radix)
    (accepted : guard gen step limit = true) :
    (gen + step).toNat = gen.toNat + step.toNat := by
  have spec := (guard_iff_spec gen step limit).1 accepted
  have stepLe := spec.2.1
  have stepLeNat : step.toNat ≤ limit.toNat := by
    rw [BitVec.le_def] at stepLe
    exact stepLe
  have stepBound : step.toNat < radix :=
    Nat.lt_of_le_of_lt stepLeNat validLimit
  have numericBound : 2 ^ 63 + 2 ^ 62 < 2 ^ 64 := by
    decide
  have sumBelow : gen.toNat + step.toNat < 2 ^ 63 + 2 ^ 62 := by
    exact Nat.add_lt_add validGen stepBound
  have sumBound : gen.toNat + step.toNat < 2 ^ 64 := by
    exact Nat.lt_trans sumBelow numericBound
  exact BitVec.toNat_add_of_lt sumBound

/-- A positive accepted increment changes the concrete 64-bit word. -/
theorem accepted_increment_changes_word
    (gen step limit : Word)
    (validGen : gen.toNat < 2 ^ 63)
    (validLimit : limit.toNat < radix)
    (accepted : guard gen step limit = true)
    (positive : 0 < step.toNat) :
    gen + step ≠ gen := by
  intro same
  have natSame := congrArg BitVec.toNat same
  rw [accepted_increment_no_wrap gen step limit validGen validLimit accepted] at natSame
  omega

/-- Executable model of the check in jw_view_target: the target is returned
    only when its current generation is not retired and equals the snapshot
    saved in the view cell. This models the guard, not the C pointer access. -/
def viewGate (saved current : Word) : Bool :=
  current != retiredWord && current == saved

theorem viewGate_matches_spec (saved current : Word) :
    viewGate saved current = true ↔
      current ≠ retiredWord ∧ current = saved := by
  simp [viewGate]

/-- A retired target is rejected independently of the saved generation. -/
theorem viewGate_rejects_retired (saved : Word) :
    viewGate saved retiredWord = false := by
  simp [viewGate]

/-- If a positive, accepted generation advance occurs, a view holding the old
    generation cannot pass the equality gate on the updated generation. -/
theorem viewGate_rejects_after_advance
    (gen step limit : Word)
    (validGen : gen.toNat < 2 ^ 63)
    (validLimit : limit.toNat < radix)
    (accepted : guard gen step limit = true)
    (positive : 0 < step.toNat) :
    viewGate gen (gen + step) = false := by
  simp [viewGate, accepted_increment_changes_word
    gen step limit validGen validLimit accepted positive]

end JabberwockFormal.RuntimeGenerationBitVec
