import JabberwockFormal.Jwc.ReferenceAccess
import JabberwockFormal.Rust.ReferenceAccess

set_option autoImplicit false
set_option warningAsError true

namespace JabberwockFormal.EmittedGate

/-- Boolean semantics of the emitted LLVM generation-check sequence.
    This assumes the loaded metadata is a valid accessible i64 address;
    pointer lifetime, atomic concurrency and optimization are not modeled. -/
abbrev Word := BitVec 64
def retired : Word := 18446744073709551615

def llvmDifferent (current saved : Word) : Bool := decide (current ≠ saved)
def llvmRetired (current : Word) : Bool := decide (current = retired)
def llvmBad (saved current : Word) : Bool :=
  llvmDifferent current saved || llvmRetired current

def llvmContinues (saved current : Word) : Bool := !llvmBad saved current

def valid (saved current : Word) : Prop :=
  current = saved ∧ current ≠ retired

theorem llvm_continues_iff (saved current : Word) :
    llvmContinues saved current = true ↔ valid saved current := by
  by_cases same : current = saved <;> by_cases dead : current = retired <;>
    simp [llvmContinues, llvmBad, llvmDifferent, llvmRetired, valid,
          same, dead]

theorem llvm_bad_iff (saved current : Word) :
    llvmBad saved current = true ↔ ¬ valid saved current := by
  by_cases same : current = saved <;> by_cases dead : current = retired <;>
    simp [llvmBad, llvmDifferent, llvmRetired, valid, same, dead]

theorem jwc_gate_correspondence (saved current : Word) :
    JabberwockFormal.Jwc.ReferenceAccess.emittedGate saved current =
      llvmContinues saved current := by
  by_cases same : current = saved <;> by_cases dead : current = retired <;>
    simp [JabberwockFormal.Jwc.ReferenceAccess.emittedGate,
          llvmContinues, llvmBad, llvmDifferent, llvmRetired,
          retired, JabberwockFormal.Jwc.ReferenceAccess.retired, same, dead]

theorem rust_gate_correspondence (saved current : Word) :
    JabberwockFormal.Rust.ReferenceAccess.emittedGate saved current =
      llvmContinues saved current := by
  by_cases same : current = saved <;> by_cases dead : current = retired <;>
    simp [JabberwockFormal.Rust.ReferenceAccess.emittedGate,
          llvmContinues, llvmBad, llvmDifferent, llvmRetired,
          retired, JabberwockFormal.Rust.ReferenceAccess.retired, same, dead]

theorem cross_compiler_gate_equivalence (saved current : Word) :
    JabberwockFormal.Jwc.ReferenceAccess.emittedGate saved current =
      JabberwockFormal.Rust.ReferenceAccess.emittedGate saved current := by
  rw [jwc_gate_correspondence, rust_gate_correspondence]

theorem no_retired_gate (saved : Word) :
    llvmContinues saved retired = false := by
  simp [llvmContinues, llvmBad, llvmRetired]

/-- A symbolic interpretation of the exact LLVM instruction pattern emitted
    by both compilers: icmp ne; icmp eq -1; or; conditional reject.
    This is an instruction-level semantic model, not an LLVM interpreter. -/
structure GateRegisters where
  different : Bool
  isRetired : Bool
  reject : Bool

def executeGateInstructions (saved current : Word) : GateRegisters :=
  let different := decide (current ≠ saved)
  let isRetired := decide (current = retired)
  let reject := different || isRetired
  ⟨different, isRetired, reject⟩

inductive GateBranch where
  | continue
  | reject
  deriving DecidableEq

def executeGateBranch (saved current : Word) : GateBranch :=
  if (executeGateInstructions saved current).reject then .reject else .continue

/-- The symbolic instruction sequence implements the emitted guard predicate. -/
theorem instructions_implement_gate (saved current : Word) :
    (executeGateInstructions saved current).reject = llvmBad saved current := by
  rfl

/-- The conditional branch reaches the address-producing continuation
    precisely on a valid, non-retired generation. -/
theorem branch_continues_iff_valid (saved current : Word) :
    executeGateBranch saved current = .continue ↔ valid saved current := by
  change (if llvmBad saved current then GateBranch.reject else .continue) =
    .continue ↔ valid saved current
  cases h : llvmBad saved current with
  | false =>
      have good : valid saved current :=
        (llvm_continues_iff saved current).mp (by simp [llvmContinues, h])
      simp [good]
  | true =>
      have invalid : ¬ valid saved current := by
        intro good
        have continues := (llvm_continues_iff saved current).mpr good
        simp [llvmContinues, h] at continues
      simp [invalid]

/-- Bind the jwc local gate model to the instruction-level branch. -/
theorem jwc_gate_matches_instruction_branch (saved current : Word) :
    (JabberwockFormal.Jwc.ReferenceAccess.emittedGate saved current = true) ↔
      executeGateBranch saved current = .continue := by
  rw [jwc_gate_correspondence, llvm_continues_iff, branch_continues_iff_valid]

/-- Bind the Rust local gate model to the very same instruction semantics. -/
theorem rust_gate_matches_instruction_branch (saved current : Word) :
    (JabberwockFormal.Rust.ReferenceAccess.emittedGate saved current = true) ↔
      executeGateBranch saved current = .continue := by
  rw [rust_gate_correspondence, llvm_continues_iff, branch_continues_iff_valid]

end JabberwockFormal.EmittedGate
