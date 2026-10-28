import JabberwockFormal.RuntimeGenerationBitVec

set_option autoImplicit false
set_option warningAsError true

namespace JabberwockFormal.UnsignedGuardSource

/-!
Value semantics of the side-effect-free uint64_t expression fragment extracted
from a Clang AST. This is not a C frontend or the semantics of arbitrary C.
The extractor must reject every other expression, statement, type and effect.
Clang parsing, target type widths and the extractor are explicit trust premises.
No heap, atomic, allocation or whole-runtime property follows from this module.
-/

abbrev Word := BitVec 64

inductive WordExpr where
  | gen
  | step
  | literal (value : Word)
  | band (left right : WordExpr)
  | sub (left right : WordExpr)
  | shl (value : WordExpr) (count : Fin 64)

inductive Predicate where
  | ne (left right : WordExpr)
  | ule (left right : WordExpr)
  | and (left right : Predicate)

def evalWord (gen step : Word) : WordExpr → Word
  | .gen => gen
  | .step => step
  | .literal value => value
  | .band left right => evalWord gen step left &&& evalWord gen step right
  | .sub left right => evalWord gen step left - evalWord gen step right
  | .shl value count => (evalWord gen step value) <<< count.val

/-- Value of C's short-circuit conjunction. Both admitted operands are pure
    and total, so evaluating this Boolean expression has the same value. -/
def eval (gen step : Word) : Predicate → Bool
  | .ne left right => decide (evalWord gen step left ≠ evalWord gen step right)
  | .ule left right => (evalWord gen step left).ule (evalWord gen step right)
  | .and left right => eval gen step left && eval gen step right

end JabberwockFormal.UnsignedGuardSource
