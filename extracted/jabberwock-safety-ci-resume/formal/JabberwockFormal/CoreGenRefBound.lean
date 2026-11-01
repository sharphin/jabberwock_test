import JabberwockFormal.CoreGenRef

set_option autoImplicit false
set_option warningAsError true

namespace JabberwockFormal.CoreGenRefBound

open JabberwockFormal.CoreGenRef

/-!
世代つき ref の模型（`CoreGenRef`）に、有限の世代の上限 `limit` をつなぐ（docs/core-type-safety.md）。

`CoreGenRef` の世代は `Nat` で、上限を見なくても安全だった。ここでは、割り当ての方針が
「空きで世代が上限未満の区画だけを選ぶ」（`PickOk limit`）なら、次が成り立つことを示す。

* どの区画の世代も上限以下で、生きている区画の世代は上限未満（`Bounded`）。
* よって ref に入る世代はいつも上限未満で、上限の値（実行時ライブラリの `GEN_RETIRED`）は ref に出ない。
* 世代が上限に達した区画（退役した区画）は、その後ずっと空きのままで、使い回されない。

世代の値を機械の整数（64 ビット、全 1 を退役）に符号化する部分は RuntimeGeneration* の模型が扱い、
ここでは上限を一つの自然数として扱う。
-/

/-- ref に入る世代が上限未満 -/
def RefLt (limit : Nat) : Val → Prop
  | .int _ => True
  | .bool _ => True
  | .ref _ g => g < limit

/-- 区画の世代は上限以下。生きている区画は世代が上限未満で、中身の ref の世代も上限未満 -/
def Bounded (limit : Nat) (H : Heap) : Prop :=
  ∀ (l : Nat) (c : Cell), nth H l = some c → c.gen ≤ limit ∧
    ∀ (o : Nat) (v : Val), c.val = some (o, v) → c.gen < limit ∧ RefLt limit v

/-- 評価の結果: 値の ref の世代は上限未満、ヒープは Bounded、評価の前のヒープから進んだもの -/
def BSafe (limit : Nat) (H0 : Heap) : Res → Prop
  | .ok v H => RefLt limit v ∧ Bounded limit H ∧ HeapLe H0 H
  | _ => True

theorem bounded_nil (limit : Nat) : Bounded limit [] := by
  unfold Bounded
  intro l c h
  have h' : (none : Option Cell) = some c := by
    cases l <;> exact h
  cases h'

theorem heapLe_refl (H : Heap) : HeapLe H H := by
  unfold HeapLe
  intro l c h
  exact Exists.intro c (And.intro h (And.intro (Nat.le_refl _) (fun _ _ hp => hp)))

theorem heapLe_trans {A B C : Heap} (h1 : HeapLe A B) (h2 : HeapLe B C) : HeapLe A C := by
  unfold HeapLe
  intro l c hc
  obtain ⟨c1, hc1, hg1, hv1⟩ := h1 l c hc
  obtain ⟨c2, hc2, hg2, hv2⟩ := h2 l c1 hc1
  refine Exists.intro c2 (And.intro hc2 (And.intro (Nat.le_trans hg1 hg2) ?_))
  intro heq p hp
  have e1 : c.gen = c1.gen := by omega
  have e2 : c1.gen = c2.gen := by omega
  exact hv2 e2 p (hv1 e1 p hp)

theorem release_heapLe (H : Heap) (l : Nat) : HeapLe H (release H l) := by
  cases hc : nth H l with
  | none =>
    simp only [release, hc]
    exact heapLe_refl H
  | some c =>
    simp only [release, hc]
    exact heapLe_release hc

theorem alloc_heapLe {limit : Nat} {pick : Heap → Option Nat} (hp : PickOk limit pick) {H : Heap}
    {p : Nat × Val} : HeapLe H (alloc pick H p).2 := by
  cases hpk : pick H with
  | none =>
    simp only [alloc, hpk]
    exact heapLe_append H _
  | some l =>
    obtain ⟨c, hc, hcv, _⟩ := hp H l hpk
    simp only [alloc, hpk, allocAt, hc]
    exact heapLe_fill p hc hcv

/-- 割り当ての後も Bounded: 選ばれた区画は世代が上限未満、新しい区画は世代 0（上限は 1 以上） -/
theorem alloc_bounded {limit : Nat} {pick : Heap → Option Nat} (hp : PickOk limit pick)
    (hpos : 0 < limit) {H : Heap} {o : Nat} {v : Val} (hb : Bounded limit H) (hr : RefLt limit v) :
    Bounded limit (alloc pick H (o, v)).2 := by
  cases hpk : pick H with
  | none =>
    simp only [alloc, hpk]
    unfold Bounded
    intro l0 c0 h0
    by_cases hlt : l0 < H.length
    · rw [nth_append_lt hlt] at h0
      exact hb l0 c0 h0
    · have h1 := nth_lt h0
      simp only [List.length_append, List.length_cons, List.length_nil] at h1
      have heq : l0 = H.length := by omega
      rw [heq, nth_append_len] at h0
      have hc0 : ({ gen := 0, val := some (o, v) } : Cell) = c0 := Option.some.inj h0
      rw [← hc0]
      refine And.intro (Nat.zero_le _) ?_
      intro o' v' hv'
      have hp' : (o, v) = (o', v') := Option.some.inj hv'
      have hvv : v = v' := congrArg Prod.snd hp'
      rw [← hvv]
      exact And.intro hpos hr
  | some l =>
    obtain ⟨c, hc, _, hlt⟩ := hp H l hpk
    simp only [alloc, hpk, allocAt, hc]
    unfold Bounded
    intro l0 c0 h0
    by_cases heq : l0 = l
    · rw [heq, nth_setNth_eq (nth_lt hc)] at h0
      have hc0 : ({ gen := c.gen, val := some (o, v) } : Cell) = c0 := Option.some.inj h0
      rw [← hc0]
      refine And.intro (Nat.le_of_lt hlt) ?_
      intro o' v' hv'
      have hp' : (o, v) = (o', v') := Option.some.inj hv'
      have hvv : v = v' := congrArg Prod.snd hp'
      rw [← hvv]
      exact And.intro hlt hr
    · rw [nth_setNth_ne heq] at h0
      exact hb l0 c0 h0

/-- 解放の後も Bounded: 解放する区画は生きていたので世代は上限未満、1 進めても上限以下 -/
theorem release_bounded {limit : Nat} {env : Env} {H : Heap} {Γ : Ctx} {s : Ty} {l : Nat}
    (ha : Agree (env ++ [l]) H (Γ ++ [s])) (hb : Bounded limit H) : Bounded limit (release H l) := by
  have hlen : env.length = Γ.length := by
    have h := ha.len
    simp only [List.length_append, List.length_cons, List.length_nil] at h
    omega
  obtain ⟨l0, c, v0, he, hc, hcv, _⟩ := ha.slot Γ.length s (nth_append_len Γ s)
  have hl0 : l0 = l := by
    rw [← hlen, nth_append_len] at he
    exact (Option.some.inj he).symm
  rw [hl0] at hc
  have hglt : c.gen < limit := ((hb l c hc).2 Γ.length v0 hcv).1
  simp only [release, hc]
  unfold Bounded
  intro l1 c1 h1
  by_cases heq : l1 = l
  · rw [heq, nth_setNth_eq (nth_lt hc)] at h1
    have hc1 : ({ gen := c.gen + 1, val := none } : Cell) = c1 := Option.some.inj h1
    rw [← hc1]
    refine And.intro (Nat.succ_le_of_lt hglt) ?_
    intro o v hv
    have hv' : (none : Option (Nat × Val)) = some (o, v) := hv
    cases hv'
  · rw [nth_setNth_ne heq] at h1
    exact hb l1 c1 h1

theorem bbind {limit : Nat} {s : Ty} {env1 : Env} {Γ1 : Ctx} {H0 : Heap} {r : Res}
    {f : Val → Heap → Res} (hs : Safe s env1 Γ1 r) (hr : BSafe limit H0 r)
    (hf : ∀ (v : Val) (H1 : Heap), ValOk H1 s v → Agree env1 H1 Γ1 → RefLt limit v →
      Bounded limit H1 → HeapLe H0 H1 → BSafe limit H0 (f v H1)) :
    BSafe limit H0 (r.bind f) := by
  cases r with
  | ok v H1 => exact hf v H1 hs.1 hs.2 hr.1 hr.2.1 hr.2.2
  | err => exact True.intro
  | fuel => exact True.intro
  | stuck => exact True.intro

theorem checked_bounded {limit : Nat} {H H2 : Heap} (z : Int) (hb : Bounded limit H2)
    (hle : HeapLe H H2) : BSafe limit H (checked z H2) := by
  unfold checked
  split
  · exact And.intro True.intro (And.intro hb hle)
  · exact True.intro

theorem addOp_bounded {limit : Nat} {H H2 : Heap} (x y : Val) (hb : Bounded limit H2)
    (hle : HeapLe H H2) : BSafe limit H (addOp x y H2) := by
  cases x with
  | bool _ => exact True.intro
  | ref _ _ => exact True.intro
  | int p =>
    cases y with
    | bool _ => exact True.intro
    | ref _ _ => exact True.intro
    | int q => exact checked_bounded (p + q) hb hle

theorem readVar_bounded {limit : Nat} {env : Env} {H : Heap} (i : Nat) (hb : Bounded limit H) :
    BSafe limit H (readVar env H i) := by
  cases he : nth env i with
  | none =>
    simp only [readVar, he]
    exact True.intro
  | some l =>
    cases hc : nth H l with
    | none =>
      simp only [readVar, he, hc]
      exact True.intro
    | some c =>
      cases hv : c.val with
      | none =>
        simp only [readVar, he, hc, hv]
        exact True.intro
      | some p =>
        obtain ⟨o, v⟩ := p
        simp only [readVar, he, hc, hv]
        exact And.intro ((hb l c hc).2 o v hv).2 (And.intro hb (heapLe_refl H))

theorem derefOp_bounded {limit : Nat} {H H1 : Heap} (v : Val) (hb : Bounded limit H1)
    (hle : HeapLe H H1) : BSafe limit H (derefOp v H1) := by
  cases v with
  | int _ => exact True.intro
  | bool _ => exact True.intro
  | ref l g =>
    cases hc : nth H1 l with
    | none =>
      simp only [derefOp, hc]
      exact True.intro
    | some c =>
      simp only [derefOp, hc]
      by_cases heq : c.gen = g
      · rw [if_pos heq]
        cases hv : c.val with
        | none =>
          simp only [hv]
          exact True.intro
        | some p =>
          obtain ⟨o, w⟩ := p
          simp only [hv]
          exact And.intro ((hb l c hc).2 o w hv).2 (And.intro hb hle)
      · rw [if_neg heq]
        exact True.intro

/-! ## 主定理 -/

/-- 有限の世代: 型が付く式を、Bounded なヒープで、上限を守る割り当ての方針で評価すると、
結果の ref の世代は上限未満で、評価後のヒープも Bounded（上限の世代は ref に出ない） -/
theorem eval_bounded {limit : Nat} (pick : Heap → Option Nat) (hp : PickOk limit pick)
    (hpos : 0 < limit) :
    ∀ (k : Nat) (env : Env) (H : Heap) (Γ : Ctx) (e : Expr) (t : Ty),
      Agree env H Γ → HasTy Γ e t → Bounded limit H → BSafe limit H (eval pick k env H e) := by
  intro k
  induction k with
  | zero =>
    intro env H Γ e t _ _ _
    cases e <;> exact True.intro
  | succ k ih =>
    intro env H Γ e t hag h hb0
    cases h with
    | int _ _ => exact And.intro True.intro (And.intro hb0 (heapLe_refl H))
    | bool _ _ => exact And.intro True.intro (And.intro hb0 (heapLe_refl H))
    | var _ i _ _ =>
      simp only [eval]
      exact readVar_bounded i hb0
    | addr _ i _ hi =>
      obtain ⟨l, c, v, he, hc, hcv, _⟩ := hag.slot i t hi
      simp only [eval, addrOf, he, hc]
      exact And.intro ((hb0 l c hc).2 i v hcv).1 (And.intro hb0 (heapLe_refl H))
    | deref _ e _ he =>
      simp only [eval]
      apply bbind (eval_safe pick hp k env H Γ e (.ref t) hag he) (ih env H Γ e (.ref t) hag he hb0)
      intro v H1 _ _ _ hb1 hle1
      exact derefOp_bounded v hb1 hle1
    | add _ a b ha hb =>
      simp only [eval]
      apply bbind (eval_safe pick hp k env H Γ a .int hag ha) (ih env H Γ a .int hag ha hb0)
      intro x H1 _ ha1 _ hb1 hle1
      apply bbind (eval_safe pick hp k env H1 Γ b .int ha1 hb) (ih env H1 Γ b .int ha1 hb hb1)
      intro y H2 _ _ _ hb2 hle2
      exact addOp_bounded x y hb2 (heapLe_trans hle1 hle2)
    | letE _ x body s _ hx hbody =>
      simp only [eval]
      apply bbind (eval_safe pick hp k env H Γ x s hag hx) (ih env H Γ x s hag hx hb0)
      intro v H1 hv1 ha1 hr1 hb1 hle1
      apply bbind (eval_safe pick hp k _ _ (Γ ++ [s]) body t (alloc_ok hp ha1 hv1) hbody)
        (ih _ _ (Γ ++ [s]) body t (alloc_ok hp ha1 hv1) hbody (alloc_bounded hp hpos hb1 hr1))
      intro w H3 _ ha3 hr3 hb3 hle3
      exact And.intro hr3 (And.intro (release_bounded ha3 hb3)
        (heapLe_trans hle1 (heapLe_trans (alloc_heapLe hp) (heapLe_trans hle3 (release_heapLe _ _)))))

/-- 評価の結果の ref は、上限の世代（退役の印）を持たない -/
theorem result_ref_lt {limit : Nat} (pick : Heap → Option Nat) (hp : PickOk limit pick)
    (hpos : 0 < limit) (k : Nat) (env : Env) (H H' : Heap) (Γ : Ctx) (e : Expr) (t : Ty)
    (l g : Nat) (hag : Agree env H Γ) (h : HasTy Γ e t) (hb : Bounded limit H)
    (hr : eval pick k env H e = .ok (.ref l g) H') : g < limit := by
  have hs := eval_bounded pick hp hpos k env H Γ e t hag h hb
  rw [hr] at hs
  exact hs.1

/-- 退役した区画（世代が上限）は、評価の後も退役したまま空きで、使い回されない -/
theorem retired_stays_vacant {limit : Nat} (pick : Heap → Option Nat) (hp : PickOk limit pick)
    (hpos : 0 < limit) (k : Nat) (env : Env) (H H' : Heap) (Γ : Ctx) (e : Expr) (t : Ty) (v : Val)
    (l : Nat) (c : Cell) (hag : Agree env H Γ) (h : HasTy Γ e t) (hb : Bounded limit H)
    (hr : eval pick k env H e = .ok v H') (hc : nth H l = some c) (hret : c.gen = limit) :
    ∃ c' : Cell, nth H' l = some c' ∧ c'.gen = limit ∧ c'.val = none := by
  have hs := eval_bounded pick hp hpos k env H Γ e t hag h hb
  rw [hr] at hs
  have hs' : RefLt limit v ∧ Bounded limit H' ∧ HeapLe H H' := hs
  obtain ⟨_, hb', hle⟩ := hs'
  obtain ⟨c', hc', hg, _⟩ := hle l c hc
  have hbound := hb' l c' hc'
  have hgen : c'.gen = limit := by
    have := hbound.1
    omega
  refine Exists.intro c' (And.intro hc' (And.intro hgen ?_))
  cases hv : c'.val with
  | none => rfl
  | some p =>
    obtain ⟨o, w⟩ := p
    have hlt := (hbound.2 o w hv).1
    exact absurd hgen (by omega)

end JabberwockFormal.CoreGenRefBound
