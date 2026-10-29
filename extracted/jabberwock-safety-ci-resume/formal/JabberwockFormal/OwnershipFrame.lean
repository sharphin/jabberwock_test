set_option autoImplicit false
set_option warningAsError true

namespace JabberwockFormal.OwnershipFrame

/-!
# 持ち主の木: 上書き・解放で消えるのは、その場所が持つものだけ

`ElisionSoundness` の前提 1（唯一の持ち主）と 2（呼ばれた関数は呼び出し元の変数の先・場所を解放しない）を、
持ち主の木の模型で証明する。

- 区画（変数の場所も区画とみなす）ごとに、それを持つ own・リストの値が置かれている場所（親の区画と、その中の道筋）が
  高々 1 つある（`parent`。唯一の持ち主）。変数の場所には親がない。循環は前提にしない（`Acyclic` は一部の補題だけで使う）。
- 場所 (p, path) を上書き・解放すると、そこより下に置かれた own・リストが持つ区画と、その子孫が解放される（`Freed`）。
- 型の決まり: ref で書き換えられる場所は、own の型の変数の場所そのもの（`(slot x, [])`）ではない
  （`ref a` は a の先を指す。`ref mut own T` は変数からは作れない）。
-/

abbrev Block := Nat
abbrev Path := List Nat

/-- 持ち主の木 -/
structure Heap where
  /-- 区画 c を持つ own・リストの値が置かれた場所（親の区画と、その中の道筋）。変数の場所は none -/
  parent : Block → Option (Block × Path)

/-- 親をたどると止まる（循環しない）。ムーブした own の先を ref で書き換えると循環を作れる
    （`r = ref mut *a; (*r).next = Some(a)`）ので、主な定理はこれを前提にしない -/
def Acyclic (h : Heap) (rank : Block → Nat) : Prop :=
  ∀ c p path, h.parent c = some (p, path) → rank p < rank c

/-- 場所 (p, path) を上書き・解放したときに解放される区画 -/
inductive Freed (h : Heap) (p : Block) (path : Path) : Block → Prop where
  /-- (p, path) より下に置かれた own・リストが持つ区画 -/
  | direct {c : Block} {q : Path} : h.parent c = some (p, q) → path <+: q → Freed h p path c
  /-- 解放された区画の中に置かれた own・リストが持つ区画 -/
  | child {c d : Block} {q : Path} : h.parent c = some (d, q) → Freed h p path d → Freed h p path c

theorem freed_rank {h : Heap} {rank : Block → Nat} (ac : Acyclic h rank)
    {p : Block} {path : Path} {c : Block}
    (f : Freed h p path c) : rank p < rank c := by
  induction f with
  | direct hp _ => exact ac _ _ _ hp
  | child hp _ ih => exact Nat.lt_trans ih (ac _ _ _ hp)

/-- 循環がなければ、上書きした区画そのものは解放されない（中身だけが入れ替わる）。
    循環があると解放されうるが、その区画を指す ref は実行時の検査が残る（事実は付かない） -/
theorem target_survives {h : Heap} {rank : Block → Nat} (ac : Acyclic h rank) (p : Block) (path : Path) :
    ¬ Freed h p path p := by
  intro f
  exact Nat.lt_irrefl _ (freed_rank ac f)

/-- 変数の場所（親のない区画）は、どこを上書きしても解放されない。解放されるのは、その変数の drop・スコープの終わりだけ -/
theorem slot_survives {h : Heap} {s : Block} (top : h.parent s = none) (p : Block) (path : Path) :
    ¬ Freed h p path s := by
  intro f
  cases f with
  | direct hp _ => simp [top] at hp
  | child hp _ => simp [top] at hp

/-- own の変数 x の先（親が `(slot x, [])`）が解放されるのは、x の場所そのものを上書き・解放したときだけ -/
theorem root_freed_only_by_its_slot {h : Heap} {s r : Block} (top : h.parent s = none)
    (root : h.parent r = some (s, [])) {p : Block} {path : Path} (f : Freed h p path r) :
    p = s ∧ path = [] := by
  cases f with
  | direct hp pre =>
    rw [root] at hp
    cases hp
    exact ⟨rfl, List.prefix_nil.mp pre⟩
  | child hp f' =>
    rw [root] at hp
    cases hp
    exact absurd f' (slot_survives top _ _)

/-! ## 呼ばれた関数ができること

呼ばれた関数が呼び出し元から見える区画に対してできるのは、次のどれか。
- ref を通した書き換え（場所 (p, path)。型の決まりで、own の型の変数の場所そのものではない）
- 自分の変数の drop・スコープの終わり（自分の変数の場所 (s', [])。呼び出し元の変数の場所ではない）
- 受け取った own・リストの解放（受け取った値は呼ばれた関数の変数の場所の下にある。上と同じ）
新しい区画を作ることは、今ある区画の世代を変えない。 -/

/-- 呼び出し元の変数: 場所 slot（親なし）と、own の型なら先 root -/
structure CallerLocal (h : Heap) where
  slot : Block
  top : h.parent slot = none

/-- 解放する場所 (p, path) が、呼び出し元のどの変数の場所そのものでもない -/
def NotCallerSlot {h : Heap} (callers : List (CallerLocal h)) (p : Block) (path : Path) : Prop :=
  ∀ x, x ∈ callers → ¬ (p = x.slot ∧ path = [])

/-- 主な定理: 呼ばれた関数の解放は、呼び出し元の変数の場所も、own の変数の先も解放しない -/
theorem callee_keeps_caller_locals {h : Heap} {callers : List (CallerLocal h)}
    {p : Block} {path : Path} (ok : NotCallerSlot callers p path)
    (x : CallerLocal h) (mem : x ∈ callers) :
    ¬ Freed h p path x.slot ∧ (∀ r, h.parent r = some (x.slot, []) → ¬ Freed h p path r) := by
  refine ⟨slot_survives x.top p path, ?_⟩
  intro r root f
  exact ok x mem (root_freed_only_by_its_slot x.top root f)

/-- 自分の変数の drop は、呼び出し元の変数の場所でない場所 (s', []) の解放 -/
theorem own_drop_is_not_caller_slot {h : Heap} {callers : List (CallerLocal h)} {s' : Block}
    (fresh : ∀ x, x ∈ callers → s' ≠ x.slot) : NotCallerSlot callers s' [] :=
  fun x mem e => fresh x mem e.1

/-- ref を通した書き換えは、型の決まり（own の型の変数の場所そのものは ref で書けない）から、
    呼び出し元の own の変数の先を解放しない。own でない変数の場所を ref で書き換えても、場所そのものは残る -/
theorem ref_write_keeps_roots {h : Heap} {s r p : Block} {path : Path}
    (top : h.parent s = none) (root : h.parent r = some (s, []))
    (typed : ¬ (p = s ∧ path = [])) : ¬ Freed h p path r :=
  fun f => typed (root_freed_only_by_its_slot top root f)

/-! ## 呼ばれた関数の実行全体

呼ばれた関数の一歩は、ある場所 (p, path) より下の区画だけを変える（解放・ムーブで付け替え・世代を進める）。
新しく作った区画（それまで生きていなかったもの）は何をしてもよい。 -/

structure St where
  heap : Heap
  gen : Block → Nat
  alive : Block → Prop

/-- (p, path) より下でない生きた区画は、親も世代もそのまま -/
def Step (p : Block) (path : Path) (s s' : St) : Prop :=
  ∀ c, s.alive c → ¬ Freed s.heap p path c →
    s'.heap.parent c = s.heap.parent c ∧ s'.gen c = s.gen c ∧ s'.alive c

/-- 呼ばれた関数の実行: 一歩ずつ、触る場所が呼び出し元の変数の場所そのもの（slots のどれかと []）でない -/
inductive CalleeRun (slots : List Block) : St → St → Prop where
  | done (s : St) : CalleeRun slots s s
  | step {s s' s'' : St} (p : Block) (path : Path) :
      (∀ x, x ∈ slots → ¬ (p = x ∧ path = [])) → Step p path s s' →
      CalleeRun slots s' s'' → CalleeRun slots s s''

/-- 主な定理: 呼ばれた関数がどう動いても、呼び出し元の変数の場所と、own の変数の先の世代は変わらない
    （ElisionSoundness の `Exec` で、呼び出しが `root`・`slot` の区画に触れないとした前提） -/
theorem callee_preserves_caller {slots : List Block} {s s' : St} (run : CalleeRun slots s s')
    (x : Block) (mem : x ∈ slots) (top : s.heap.parent x = none) (liveX : s.alive x) :
    s'.heap.parent x = none ∧ s'.gen x = s.gen x ∧ s'.alive x ∧
    (∀ r, s.alive r → s.heap.parent r = some (x, []) →
      s'.heap.parent r = some (x, []) ∧ s'.gen r = s.gen r ∧ s'.alive r) := by
  revert top liveX
  induction run with
  | done => exact fun top liveX => ⟨top, rfl, liveX, fun r a h => ⟨h, rfl, a⟩⟩
  | @step s0 s1 s2 p path ok st _ ih =>
    intro top liveX
    have notSlot := ok x mem
    have kx := st x liveX (slot_survives top p path)
    have top1 : s1.heap.parent x = none := kx.1.trans top
    rcases ih top1 kx.2.2 with ⟨t, g, a, roots⟩
    refine ⟨t, g.trans kx.2.1, a, ?_⟩
    intro r ar hr
    have kr := st r ar (fun f => notSlot (root_freed_only_by_its_slot top hr f))
    rcases roots r kr.2.2 (kr.1.trans hr) with ⟨t', g', a'⟩
    exact ⟨t', g'.trans kr.2.1, a'⟩

end JabberwockFormal.OwnershipFrame
