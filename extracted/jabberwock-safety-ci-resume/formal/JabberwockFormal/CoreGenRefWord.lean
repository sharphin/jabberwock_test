import JabberwockFormal.CoreGenRefBound
import JabberwockFormal.RuntimeGenerationEncoding

set_option autoImplicit false
set_option warningAsError true

namespace JabberwockFormal.CoreGenRefWord

open JabberwockFormal.CoreGenRef
open JabberwockFormal.CoreGenRefBound
open JabberwockFormal.RuntimeGenerationEncoding

/-!
世代つき ref の模型（`CoreGenRef`・`CoreGenRefBound`）を、実行時ライブラリの世代の語の符号化
（`RuntimeGenerationEncoding`: 62 ビットの数え、62 番のビットの印、全 1 の `GEN_RETIRED`）につなぐ
（docs/core-type-safety.md）。

上限を `radix`（2^62）に置く。数えが `maxCounter`（2^62 - 1）を超えるときに区画を退役させる jw_rt.c の規則と同じ。
区画の語は、世代が上限未満なら `pack r gen`、上限なら `retiredWord` とする（r は 62 番のビットの印）。

* 型が付く式が作る ref の語は、退役の語にならない。
* 模型の読むときの検査（世代が等しいか）は、実行時の語の検査（`accepts`: 退役でなく、語が等しい）と同じ答えを返す。
* 退役した区画の語は、どの保存された語も通さない。

印 r は区画ごとに変わらないものとして扱う。C のポインタの寿命、原子的な順序、割り当て器のリストの形は扱わない。
-/

/-- 区画の世代の語: 上限未満なら数えと印を詰めた語、上限なら退役の語 -/
def encode (r : Bool) (c : Cell) : Nat :=
  if c.gen < radix then pack r c.gen else retiredWord

theorem radix_pos : 0 < radix := by
  unfold radix
  decide

/-- 上限を 2^62 にした割り当ての方針でも、上限の定理の前提（0 < 上限）が満たされる -/
theorem eval_bounded_word (pick : Heap → Option Nat) (hp : PickOk radix pick) (k : Nat) (env : Env)
    (H : Heap) (Γ : Ctx) (e : Expr) (t : Ty) (hag : Agree env H Γ) (h : HasTy Γ e t)
    (hb : Bounded radix H) : BSafe radix H (eval pick k env H e) :=
  eval_bounded pick hp radix_pos k env H Γ e t hag h hb

/-- 型が付く式の結果の ref の語は、退役の語（GEN_RETIRED）にならない -/
theorem result_word_not_retired (pick : Heap → Option Nat) (hp : PickOk radix pick) (r : Bool)
    (k : Nat) (env : Env) (H H' : Heap) (Γ : Ctx) (e : Expr) (t : Ty) (l g : Nat)
    (hag : Agree env H Γ) (h : HasTy Γ e t) (hb : Bounded radix H)
    (hr : eval pick k env H e = .ok (.ref l g) H') : pack r g ≠ retiredWord :=
  live_word_not_retired r (result_ref_lt pick hp radix_pos k env H H' Γ e t l g hag h hb hr)

/-- 模型の検査（derefOp の世代の比較）と実行時の語の検査は同じ答え: 世代が上限未満の ref について。
ref の世代は `result_ref_lt`・`Bounded` により上限未満 -/
theorem gate_matches (r : Bool) {g : Nat} (c : Cell) (hg : g < radix) :
    accepts (pack r g) (encode r c) ↔ c.gen = g := by
  unfold encode
  by_cases h : c.gen < radix
  · rw [if_pos h]
    have hne := live_word_not_retired r h
    unfold accepts
    constructor
    · intro ha
      have heq := ha.2
      cases r <;> simp [pack] at heq <;> omega
    · intro heq
      refine And.intro hne ?_
      rw [heq]
  · rw [if_neg h]
    unfold accepts
    constructor
    · intro ha
      exact absurd rfl ha.1
    · intro heq
      exact absurd heq (by omega)

/-- 退役した区画（世代が上限）の語は、どの保存された語も通さない -/
theorem retired_rejects (r : Bool) (saved : Nat) (c : Cell) (hret : c.gen = radix) :
    ¬ accepts saved (encode r c) := by
  unfold encode accepts
  have h : ¬ c.gen < radix := by omega
  rw [if_neg h]
  intro ha
  exact ha.1 rfl

end JabberwockFormal.CoreGenRefWord
