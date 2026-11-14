import JabberwockFormal.CoreMutBorrow

set_option autoImplicit false
set_option warningAsError true

namespace JabberwockFormal.CoreGenBox

open JabberwockFormal.CoreMutBorrow (nth setNth lastOpt init nth_lt nth_append_lt nth_append_len
  length_setNth nth_setNth_eq nth_setNth_ne split_last init_append lastOpt_append)

/-!
持ち主の箱と世代つき ref を一つにした模型（docs/core-type-safety.md）。

jabberwock の `own` と `ref` の使い方に近い形にする。

* `mkBox e` : ヒープの空き区画（割り当ての方針が選ぶ。なければ末尾に新しい区画）に値を置き、箱を返す。
* 箱は変数に入れて持つ。`move i` で持ち主が変わり、元の変数は死ぬ（中身は空になる）。
* let の終わりに、変数が生きた箱を持っていれば区画を解放し、世代を 1 進める。
* `addr i` : 変数 i の箱の区画の番号と今の世代の組を ref として返す。ref は let の外へ持ち出してよい。
* `deref e` : ref の世代が区画の世代と同じときだけ読む。違えば実行時の誤り（`err`）。
* `peek i` : 生きた箱の中身を読む（持ち主が生きているので検査はいらない）。
* 箱の中身と ref の指す先は、箱を含まない型（`Plain`）に限る。

区画の「持ち主の記録」は検証のための記録で、評価はこれを読まない（let で値を変数に入れるときに書き換える）。

定理 `eval_safe`: 型が付く式を、文脈と合う環境・ヒープで、条件を満たすどの割り当ての方針で評価しても stuck にならない。
つまり移動済みの変数を使わず、解放済みの区画を読まず、使い回された区画を別の型として読まない。

これは核の言語の模型の定理で、jwc・Rust 版の検査、IR・LLVM・実行時ライブラリとの対応の証明ではない。
世代は `Nat` で、有限の上限と語の符号化は CoreGenRefBound・CoreGenRefWord（箱のない模型）で扱った。
-/

inductive Ty where
  | int
  | bool
  | box (t : Ty)
  | ref (t : Ty)

/-- 箱を含まない型 -/
def Plain : Ty → Prop
  | .int => True
  | .bool => True
  | .box _ => False
  | .ref t => Plain t

inductive Val where
  | int (n : Int)
  | bool (b : Bool)
  /-- 区画 l を持つ箱 -/
  | box (l : Nat)
  /-- 区画の番号 l と、作ったときの世代 g -/
  | ref (l g : Nat)

/-- 区画: 世代と中身（持ち主の記録と値）。中身が none なら空き -/
structure Cell where
  gen : Nat
  val : Option (Nat × Val)

abbrev Heap := List Cell
/-- 環境: 変数の位置 → 値（移動済みなら none） -/
abbrev Env := List (Option Val)
/-- 文脈: 型と、生きているか -/
abbrev Ctx := List (Ty × Bool)

inductive Expr where
  | int (n : Int)
  | bool (b : Bool)
  | var (i : Nat)
  | move (i : Nat)
  | peek (i : Nat)
  | addr (i : Nat)
  | deref (e : Expr)
  | mkBox (e : Expr)
  | add (a b : Expr)
  | letE (bound body : Expr)

/-! ## 型 -/

/-- 型付け `HasTy Γ e t Γ'`: Γ' は評価の後の文脈（移動した変数は死ぬ）。let の結果の型に制限はない -/
inductive HasTy : Ctx → Expr → Ty → Ctx → Prop where
  | int (Γ : Ctx) (n : Int) : HasTy Γ (.int n) .int Γ
  | bool (Γ : Ctx) (b : Bool) : HasTy Γ (.bool b) .bool Γ
  | var (Γ : Ctx) (i : Nat) (t : Ty) (hi : nth Γ i = some (t, true)) (hc : Plain t) :
      HasTy Γ (.var i) t Γ
  | move (Γ : Ctx) (i : Nat) (t : Ty) (hi : nth Γ i = some (.box t, true)) :
      HasTy Γ (.move i) (.box t) (setNth Γ i (.box t, false))
  | peek (Γ : Ctx) (i : Nat) (t : Ty) (hi : nth Γ i = some (.box t, true)) :
      HasTy Γ (.peek i) t Γ
  | addr (Γ : Ctx) (i : Nat) (t : Ty) (hi : nth Γ i = some (.box t, true)) :
      HasTy Γ (.addr i) (.ref t) Γ
  | deref (Γ Γ' : Ctx) (e : Expr) (t : Ty) (he : HasTy Γ e (.ref t) Γ') :
      HasTy Γ (.deref e) t Γ'
  | mkBox (Γ Γ' : Ctx) (e : Expr) (t : Ty) (he : HasTy Γ e t Γ') (hc : Plain t) :
      HasTy Γ (.mkBox e) (.box t) Γ'
  | add (Γ Γ1 Γ2 : Ctx) (a b : Expr) (ha : HasTy Γ a .int Γ1) (hb : HasTy Γ1 b .int Γ2) :
      HasTy Γ (.add a b) .int Γ2
  | letE (Γ Γ1 Γ2 : Ctx) (x body : Expr) (s t : Ty) (p : Ty × Bool) (hx : HasTy Γ x s Γ1)
      (hb : HasTy (Γ1 ++ [(s, true)]) body t (Γ2 ++ [p])) : HasTy Γ (.letE x body) t Γ2

/-! ## 評価 -/

inductive Res where
  | ok (v : Val) (env : Env) (H : Heap)
  | err
  | fuel
  | stuck

def Res.bind : Res → (Val → Env → Heap → Res) → Res
  | .ok v env H, f => f v env H
  | .err, _ => .err
  | .fuel, _ => .fuel
  | .stuck, _ => .stuck

def checked (z : Int) (env : Env) (H : Heap) : Res :=
  if -(2 : Int) ^ 63 ≤ z ∧ z < (2 : Int) ^ 63 then .ok (.int z) env H else .err

def addOp : Val → Val → Env → Heap → Res
  | .int p, .int q, env, H => checked (p + q) env H
  | _, _, _, _ => .stuck

def readVar (env : Env) (H : Heap) (i : Nat) : Res :=
  match nth env i with
  | some (some v) => .ok v env H
  | _ => .stuck

def moveOut (env : Env) (H : Heap) (i : Nat) : Res :=
  match nth env i with
  | some (some (.box l)) => .ok (.box l) (setNth env i none) H
  | _ => .stuck

/-- 区画 l の中身を読む -/
def contents (H : Heap) (l : Nat) (env : Env) : Res :=
  match nth H l with
  | some c =>
    match c.val with
    | some (_, w) => .ok w env H
    | none => .stuck
  | none => .stuck

def peekAt (env : Env) (H : Heap) (i : Nat) : Res :=
  match nth env i with
  | some (some (.box l)) => contents H l env
  | _ => .stuck

def addrOf (env : Env) (H : Heap) (i : Nat) : Res :=
  match nth env i with
  | some (some (.box l)) =>
    match nth H l with
    | some c => .ok (.ref l c.gen) env H
    | none => .stuck
  | _ => .stuck

/-- ref を読む: 世代が合わなければ実行時の誤り -/
def derefOp : Val → Env → Heap → Res
  | .ref l g, env, H =>
    match nth H l with
    | some c => if c.gen = g then contents H l env else .err
    | none => .stuck
  | _, _, _ => .stuck

def allocAt (H : Heap) (l : Nat) (p : Nat × Val) : Heap :=
  match nth H l with
  | some c => setNth H l ⟨c.gen, some p⟩
  | none => H

def alloc (pick : Heap → Option Nat) (H : Heap) (p : Nat × Val) : Nat × Heap :=
  match pick H with
  | some l => (l, allocAt H l p)
  | none => (H.length, H ++ [⟨0, some p⟩])

def release (H : Heap) (l : Nat) : Heap :=
  match nth H l with
  | some c => setNth H l ⟨c.gen + 1, none⟩
  | none => H

/-- 箱の区画の持ち主の記録を o に書き換える（検証のための記録） -/
def claim (H : Heap) (v : Val) (o : Nat) : Heap :=
  match v with
  | .box l =>
    match nth H l with
    | some c =>
      match c.val with
      | some (_, w) => setNth H l ⟨c.gen, some (o, w)⟩
      | none => H
    | none => H
  | _ => H

/-- 変数の終わり: 生きた箱を持っていれば区画を解放する -/
def dropSlot (H : Heap) : Option (Option Val) → Heap
  | some (some (.box l)) => release H l
  | _ => H

def eval (pick : Heap → Option Nat) : Nat → Env → Heap → Expr → Res
  | 0, _, _, _ => .fuel
  | _ + 1, env, H, .int n => .ok (.int n) env H
  | _ + 1, env, H, .bool b => .ok (.bool b) env H
  | _ + 1, env, H, .var i => readVar env H i
  | _ + 1, env, H, .move i => moveOut env H i
  | _ + 1, env, H, .peek i => peekAt env H i
  | _ + 1, env, H, .addr i => addrOf env H i
  | k + 1, env, H, .deref e => (eval pick k env H e).bind derefOp
  | k + 1, env, H, .mkBox e =>
    (eval pick k env H e).bind fun v env1 H1 =>
      .ok (.box (alloc pick H1 (env1.length, v)).1) env1 (alloc pick H1 (env1.length, v)).2
  | k + 1, env, H, .add a b =>
    (eval pick k env H a).bind fun x env1 H1 =>
      (eval pick k env1 H1 b).bind fun y env2 H2 => addOp x y env2 H2
  | k + 1, env, H, .letE x body =>
    (eval pick k env H x).bind fun v env1 H1 =>
      (eval pick k (env1 ++ [some v]) (claim H1 v env1.length) body).bind fun w env2 H2 =>
        .ok w (init env2) (dropSlot H2 (lastOpt env2))

/-- 割り当ての方針の条件: 空き区画だけを選ぶ -/
def PickOk (pick : Heap → Option Nat) : Prop :=
  ∀ (H : Heap) (l : Nat), pick H = some l → ∃ c : Cell, nth H l = some c ∧ c.val = none

/-! ## 不変条件 -/

/-- 値が型に合う。箱は生きた区画を指し、中身が型に合う。ref は世代が区画の世代以下で、等しければ区画が生きている -/
def ValOk (H : Heap) : Ty → Val → Prop
  | .int, .int _ => True
  | .bool, .bool _ => True
  | .box t, .box l => Plain t ∧ ∃ (c : Cell) (o : Nat) (w : Val), nth H l = some c ∧
      c.val = some (o, w) ∧ ValOk H t w
  | .ref t, .ref l g => Plain t ∧ ∃ c : Cell, nth H l = some c ∧ g ≤ c.gen ∧
      (g = c.gen → ∃ (o : Nat) (w : Val), c.val = some (o, w) ∧ ValOk H t w)
  | _, _ => False

/-- 箱の値なら、その区画の持ち主の記録は j -/
def Owns (H : Heap) (j : Nat) : Val → Prop
  | .box l => ∃ (c : Cell) (w : Val), nth H l = some c ∧ c.val = some (j, w)
  | _ => True

/-- 評価の結果の箱は、生きた変数が持っていない（持ち主の記録が生きた変数でない） -/
def Loose (H : Heap) (Γ : Ctx) : Val → Prop
  | .box l => ∃ (c : Cell) (o : Nat) (w : Val), nth H l = some c ∧ c.val = some (o, w) ∧
      ∀ t : Ty, nth Γ o ≠ some (t, true)
  | _ => True

/-- 環境とヒープが文脈に合う: 生きた変数は値を持ち、型に合い、箱なら持ち主の記録が自分。死んだ変数は空 -/
structure Agree (env : Env) (H : Heap) (Γ : Ctx) : Prop where
  len : env.length = Γ.length
  live : ∀ (j : Nat) (t : Ty), nth Γ j = some (t, true) →
    ∃ v : Val, nth env j = some (some v) ∧ ValOk H t v ∧ Owns H j v
  dead : ∀ (j : Nat) (t : Ty), nth Γ j = some (t, false) → nth env j = some none

def Safe (t : Ty) (Γ : Ctx) : Res → Prop
  | .ok v env H => ValOk H t v ∧ Agree env H Γ ∧ Loose H Γ v
  | .err => True
  | .fuel => True
  | .stuck => False

/-- ヒープの進み方: 区画は消えず、世代は減らず、世代が同じなら生きていた中身の値は変わらない（持ち主の記録は変わってよい） -/
def HeapLe (H H' : Heap) : Prop :=
  ∀ (l : Nat) (c : Cell), nth H l = some c → ∃ c' : Cell, nth H' l = some c' ∧ c.gen ≤ c'.gen ∧
    (c.gen = c'.gen → ∀ (o : Nat) (w : Val), c.val = some (o, w) → ∃ o' : Nat, c'.val = some (o', w))

/-! ## ヒープの補題 -/

theorem heapLe_refl (H : Heap) : HeapLe H H := by
  unfold HeapLe
  intro l c h
  exact ⟨c, h, Nat.le_refl _, fun _ o _ hp => ⟨o, hp⟩⟩

theorem heapLe_append (H : Heap) (x : Cell) : HeapLe H (H ++ [x]) := by
  unfold HeapLe
  intro l c h
  refine ⟨c, ?_, Nat.le_refl _, fun _ o _ hp => ⟨o, hp⟩⟩
  rw [nth_append_lt (nth_lt h)]
  exact h

theorem heapLe_fill {H : Heap} {l : Nat} {c : Cell} (p : Nat × Val) (hc : nth H l = some c)
    (hv : c.val = none) : HeapLe H (setNth H l ⟨c.gen, some p⟩) := by
  unfold HeapLe
  intro l0 c0 h0
  by_cases heq : l0 = l
  · rw [heq] at h0
    have h1 : c0 = c := Option.some.inj (h0.symm.trans hc)
    rw [h1, heq]
    refine ⟨⟨c.gen, some p⟩, nth_setNth_eq (nth_lt hc), Nat.le_refl _, ?_⟩
    intro _ _ _ hq
    rw [hv] at hq
    cases hq
  · refine ⟨c0, ?_, Nat.le_refl _, fun _ o _ hq => ⟨o, hq⟩⟩
    rw [nth_setNth_ne heq]
    exact h0

theorem heapLe_release {H : Heap} {l : Nat} {c : Cell} (hc : nth H l = some c) :
    HeapLe H (setNth H l ⟨c.gen + 1, none⟩) := by
  unfold HeapLe
  intro l0 c0 h0
  by_cases heq : l0 = l
  · rw [heq] at h0
    have h1 : c0 = c := Option.some.inj (h0.symm.trans hc)
    rw [h1, heq]
    refine ⟨⟨c.gen + 1, none⟩, nth_setNth_eq (nth_lt hc), Nat.le_succ _, ?_⟩
    intro hg
    have hg' : c.gen = c.gen + 1 := hg
    exact absurd hg' (by omega)
  · refine ⟨c0, ?_, Nat.le_refl _, fun _ o _ hq => ⟨o, hq⟩⟩
    rw [nth_setNth_ne heq]
    exact h0

theorem release_heapLe (H : Heap) (l : Nat) : HeapLe H (release H l) := by
  cases hc : nth H l with
  | none =>
    simp only [release, hc]
    exact heapLe_refl H
  | some c =>
    simp only [release, hc]
    exact heapLe_release hc

theorem release_ne {H : Heap} {l l1 : Nat} (h : l1 ≠ l) : nth (release H l) l1 = nth H l1 := by
  cases hc : nth H l with
  | none => simp only [release, hc]
  | some c =>
    simp only [release, hc]
    exact nth_setNth_ne h

theorem claim_heapLe (H : Heap) (v : Val) (o : Nat) : HeapLe H (claim H v o) := by
  cases v with
  | box l =>
    cases hc : nth H l with
    | none =>
      simp only [claim, hc]
      exact heapLe_refl H
    | some c =>
      cases hv : c.val with
      | none =>
        simp only [claim, hc, hv]
        exact heapLe_refl H
      | some p =>
        obtain ⟨o0, w⟩ := p
        simp only [claim, hc, hv]
        unfold HeapLe
        intro l0 c0 h0
        by_cases heq : l0 = l
        · rw [heq] at h0
          have h1 : c0 = c := Option.some.inj (h0.symm.trans hc)
          rw [h1, heq]
          refine ⟨⟨c.gen, some (o, w)⟩, nth_setNth_eq (nth_lt hc), Nat.le_refl _, ?_⟩
          intro _ o1 w1 h2
          rw [hv] at h2
          have h3 : (o0, w) = (o1, w1) := Option.some.inj h2
          have h4 : w = w1 := congrArg Prod.snd h3
          refine ⟨o, ?_⟩
          show some (o, w) = some (o, w1)
          rw [h4]
        · refine ⟨c0, ?_, Nat.le_refl _, fun _ o1 _ h2 => ⟨o1, h2⟩⟩
          rw [nth_setNth_ne heq]
          exact h0
  | int _ => exact heapLe_refl H
  | bool _ => exact heapLe_refl H
  | ref _ _ => exact heapLe_refl H

theorem claim_ne {H : Heap} {v : Val} {o l : Nat} (h : ∀ l' : Nat, v = .box l' → l' ≠ l) :
    nth (claim H v o) l = nth H l := by
  cases v with
  | box l' =>
    have hne : l' ≠ l := h l' rfl
    cases hc : nth H l' with
    | none => simp only [claim, hc]
    | some c =>
      cases hv : c.val with
      | none => simp only [claim, hc, hv]
      | some p =>
        obtain ⟨o0, w⟩ := p
        simp only [claim, hc, hv]
        exact nth_setNth_ne (Ne.symm hne)
  | int _ => rfl
  | bool _ => rfl
  | ref _ _ => rfl

theorem claim_self {H : Heap} {l : Nat} {c : Cell} {o n : Nat} {w : Val} (hc : nth H l = some c)
    (hcv : c.val = some (o, w)) : nth (claim H (.box l) n) l = some ⟨c.gen, some (n, w)⟩ := by
  simp only [claim, hc, hcv]
  exact nth_setNth_eq (nth_lt hc)

theorem alloc_spec {pick : Heap → Option Nat} (hp : PickOk pick) (H : Heap) (q : Nat × Val) :
    HeapLe H (alloc pick H q).2 ∧
    (∀ (l : Nat) (c : Cell) (p : Nat × Val), nth H l = some c → c.val = some p →
      nth (alloc pick H q).2 l = some c) ∧
    ∃ c' : Cell, nth (alloc pick H q).2 (alloc pick H q).1 = some c' ∧ c'.val = some q := by
  cases hpk : pick H with
  | none =>
    simp only [alloc, hpk]
    refine ⟨heapLe_append H _, ?_, ⟨_, nth_append_len H _, rfl⟩⟩
    intro l c _ h _
    rw [nth_append_lt (nth_lt h)]
    exact h
  | some l =>
    obtain ⟨c, hc, hcv⟩ := hp H l hpk
    simp only [alloc, hpk, allocAt, hc]
    refine ⟨heapLe_fill q hc hcv, ?_, ⟨_, nth_setNth_eq (nth_lt hc), rfl⟩⟩
    intro l0 c0 p h0 hp0
    by_cases heq : l0 = l
    · rw [heq] at h0
      have h1 : c0 = c := Option.some.inj (h0.symm.trans hc)
      rw [h1, hcv] at hp0
      cases hp0
    · rw [nth_setNth_ne heq]
      exact h0

/-! ## 値の補題 -/

/-- 箱を含まない型の値は、ヒープが進んでも型に合う -/
theorem valOk_mono_plain {H H' : Heap} (hle : HeapLe H H') :
    ∀ t : Ty, Plain t → ∀ v : Val, ValOk H t v → ValOk H' t v := by
  intro t
  induction t with
  | int =>
    intro _ v hv
    cases v <;> exact hv
  | bool =>
    intro _ v hv
    cases v <;> exact hv
  | box _ _ =>
    intro hp
    exact False.elim hp
  | ref t ih =>
    intro hp v hv
    cases v with
    | int _ => exact False.elim hv
    | bool _ => exact False.elim hv
    | box _ => exact False.elim hv
    | ref l g =>
      have hv' : Plain t ∧ ∃ c : Cell, nth H l = some c ∧ g ≤ c.gen ∧
          (g = c.gen → ∃ (o : Nat) (w : Val), c.val = some (o, w) ∧ ValOk H t w) := hv
      obtain ⟨hpt, c, hc, hg, hlive⟩ := hv'
      obtain ⟨c', hc', hgen, hval⟩ := hle l c hc
      show Plain t ∧ ∃ c : Cell, nth H' l = some c ∧ g ≤ c.gen ∧
          (g = c.gen → ∃ (o : Nat) (w : Val), c.val = some (o, w) ∧ ValOk H' t w)
      refine ⟨hpt, c', hc', Nat.le_trans hg hgen, ?_⟩
      intro heq
      have h1 : g = c.gen := by omega
      have h2 : c.gen = c'.gen := by omega
      obtain ⟨o, w, hw, hok⟩ := hlive h1
      obtain ⟨o', hw'⟩ := hval h2 o w hw
      exact ⟨o', w, hw', ih hpt w hok⟩

/-- 値の移し替え: 箱なら、その区画が変わっていなければ型に合ったまま -/
theorem val_transport {H H' : Heap} {t : Ty} {v : Val} (hle : HeapLe H H')
    (hs : ∀ l : Nat, v = .box l → nth H' l = nth H l) (hv : ValOk H t v) : ValOk H' t v := by
  cases v with
  | box l =>
    cases t with
    | box t =>
      have hv' : Plain t ∧ ∃ (c : Cell) (o : Nat) (w : Val), nth H l = some c ∧
          c.val = some (o, w) ∧ ValOk H t w := hv
      obtain ⟨hp, c, o, w, hc, hcv, hok⟩ := hv'
      show Plain t ∧ ∃ (c : Cell) (o : Nat) (w : Val), nth H' l = some c ∧
          c.val = some (o, w) ∧ ValOk H' t w
      refine ⟨hp, c, o, w, ?_, hcv, valOk_mono_plain hle t hp w hok⟩
      rw [hs l rfl]
      exact hc
    | int => exact False.elim hv
    | bool => exact False.elim hv
    | ref _ => exact False.elim hv
  | int _ =>
    cases t with
    | int => exact hv
    | bool => exact False.elim hv
    | box _ => exact False.elim hv
    | ref _ => exact False.elim hv
  | bool _ =>
    cases t with
    | bool => exact hv
    | int => exact False.elim hv
    | box _ => exact False.elim hv
    | ref _ => exact False.elim hv
  | ref l g =>
    cases t with
    | ref t =>
      have hv' : Plain t ∧ ∃ c : Cell, nth H l = some c ∧ g ≤ c.gen ∧
          (g = c.gen → ∃ (o : Nat) (w : Val), c.val = some (o, w) ∧ ValOk H t w) := hv
      exact valOk_mono_plain hle (.ref t) hv'.1 (.ref l g) hv
    | int => exact False.elim hv
    | bool => exact False.elim hv
    | box _ => exact False.elim hv

theorem owns_transport {H H' : Heap} {j : Nat} {v : Val}
    (hs : ∀ l : Nat, v = .box l → nth H' l = nth H l) (ho : Owns H j v) : Owns H' j v := by
  cases v with
  | box l =>
    have ho' : ∃ (c : Cell) (w : Val), nth H l = some c ∧ c.val = some (j, w) := ho
    obtain ⟨c, w, hc, hcv⟩ := ho'
    show ∃ (c : Cell) (w : Val), nth H' l = some c ∧ c.val = some (j, w)
    refine ⟨c, w, ?_, hcv⟩
    rw [hs l rfl]
    exact hc
  | int _ => exact ho
  | bool _ => exact ho
  | ref _ _ => exact ho

theorem loose_transport {H H' : Heap} {Γ : Ctx} {v : Val}
    (hs : ∀ l : Nat, v = .box l → nth H' l = nth H l) (hl : Loose H Γ v) : Loose H' Γ v := by
  cases v with
  | box l =>
    have hl' : ∃ (c : Cell) (o : Nat) (w : Val), nth H l = some c ∧ c.val = some (o, w) ∧
        ∀ t : Ty, nth Γ o ≠ some (t, true) := hl
    obtain ⟨c, o, w, hc, hcv, hnl⟩ := hl'
    show ∃ (c : Cell) (o : Nat) (w : Val), nth H' l = some c ∧ c.val = some (o, w) ∧
        ∀ t : Ty, nth Γ o ≠ some (t, true)
    refine ⟨c, o, w, ?_, hcv, hnl⟩
    rw [hs l rfl]
    exact hc
  | int _ => exact hl
  | bool _ => exact hl
  | ref _ _ => exact hl

/-- 箱を含まない型の値は箱でない -/
theorem loose_plain {H : Heap} {Γ : Ctx} {t : Ty} {v : Val} (hp : Plain t) (hv : ValOk H t v) :
    Loose H Γ v := by
  cases v with
  | box _ =>
    cases t with
    | box _ => exact False.elim hp
    | int => exact False.elim hv
    | bool => exact False.elim hv
    | ref _ => exact False.elim hv
  | int _ => exact True.intro
  | bool _ => exact True.intro
  | ref _ _ => exact True.intro

theorem loose_pop {H : Heap} {Γ : Ctx} {p : Ty × Bool} {v : Val} (hl : Loose H (Γ ++ [p]) v) :
    Loose H Γ v := by
  cases v with
  | box l =>
    have hl' : ∃ (c : Cell) (o : Nat) (w : Val), nth H l = some c ∧ c.val = some (o, w) ∧
        ∀ t : Ty, nth (Γ ++ [p]) o ≠ some (t, true) := hl
    obtain ⟨c, o, w, hc, hcv, hnl⟩ := hl'
    show ∃ (c : Cell) (o : Nat) (w : Val), nth H l = some c ∧ c.val = some (o, w) ∧
        ∀ t : Ty, nth Γ o ≠ some (t, true)
    refine ⟨c, o, w, hc, hcv, ?_⟩
    intro t ht
    have hlt := nth_lt ht
    exact hnl t (by rw [nth_append_lt hlt]; exact ht)
  | int _ => exact hl
  | bool _ => exact hl
  | ref _ _ => exact hl

/-! ## 環境の補題 -/

/-- 生きた変数の持つ区画が書き換わらなければ、環境は文脈に合ったまま -/
theorem agree_transport {env : Env} {H H' : Heap} {Γ : Ctx} (ha : Agree env H Γ) (hle : HeapLe H H')
    (hsame : ∀ (j l : Nat) (c : Cell) (w : Val), nth H l = some c → c.val = some (j, w) →
      (∃ t : Ty, nth Γ j = some (t, true)) → nth H' l = nth H l) :
    Agree env H' Γ := by
  constructor
  · exact ha.len
  · intro j t hj
    obtain ⟨v, he, hok, ho⟩ := ha.live j t hj
    have hs : ∀ l : Nat, v = .box l → nth H' l = nth H l := by
      intro l hvl
      subst hvl
      have ho' : ∃ (c : Cell) (w : Val), nth H l = some c ∧ c.val = some (j, w) := ho
      obtain ⟨c, w, hc, hcv⟩ := ho'
      exact hsame j l c w hc hcv ⟨t, hj⟩
    exact ⟨v, he, val_transport hle hs hok, owns_transport hs ho⟩
  · exact ha.dead

theorem agree_kill {env : Env} {H : Heap} {Γ : Ctx} {i : Nat} {t : Ty} (ha : Agree env H Γ)
    (hi : nth Γ i = some (.box t, true)) :
    Agree (setNth env i none) H (setNth Γ i (.box t, false)) := by
  constructor
  · rw [length_setNth, length_setNth]
    exact ha.len
  · intro j t' hj
    by_cases heq : j = i
    · rw [heq, nth_setNth_eq (nth_lt hi)] at hj
      have hb : false = true := congrArg Prod.snd (Option.some.inj hj)
      cases hb
    · rw [nth_setNth_ne heq] at hj
      obtain ⟨v, he, hok, ho⟩ := ha.live j t' hj
      refine ⟨v, ?_, hok, ho⟩
      rw [nth_setNth_ne heq]
      exact he
  · intro j t' hj
    by_cases heq : j = i
    · rw [heq]
      have hlt : i < env.length := by
        rw [ha.len]
        exact nth_lt hi
      exact nth_setNth_eq hlt
    · rw [nth_setNth_ne heq] at hj
      rw [nth_setNth_ne heq]
      exact ha.dead j t' hj

theorem agree_pop {env : Env} {H : Heap} {Γ : Ctx} {x : Option Val} {p : Ty × Bool}
    (ha : Agree (env ++ [x]) H (Γ ++ [p])) (hlen : env.length = Γ.length) : Agree env H Γ := by
  constructor
  · exact hlen
  · intro j t hj
    have hlt := nth_lt hj
    obtain ⟨v, he, hok, ho⟩ := ha.live j t (by rw [nth_append_lt hlt]; exact hj)
    rw [nth_append_lt (by omega : j < env.length)] at he
    exact ⟨v, he, hok, ho⟩
  · intro j t hj
    have hlt := nth_lt hj
    have he := ha.dead j t (by rw [nth_append_lt hlt]; exact hj)
    rw [nth_append_lt (by omega : j < env.length)] at he
    exact he

theorem claim_valOk {H : Heap} {s : Ty} {v : Val} {n : Nat} (hv : ValOk H s v) :
    ValOk (claim H v n) s v := by
  cases v with
  | box l =>
    cases s with
    | box t =>
      have hv' : Plain t ∧ ∃ (c : Cell) (o : Nat) (w : Val), nth H l = some c ∧
          c.val = some (o, w) ∧ ValOk H t w := hv
      obtain ⟨hp, c, _, w, hc, hcv, hok⟩ := hv'
      show Plain t ∧ ∃ (c : Cell) (o : Nat) (w : Val), nth (claim H (.box l) n) l = some c ∧
          c.val = some (o, w) ∧ ValOk (claim H (.box l) n) t w
      exact ⟨hp, ⟨c.gen, some (n, w)⟩, n, w, claim_self hc hcv, rfl,
        valOk_mono_plain (claim_heapLe H (.box l) n) t hp w hok⟩
    | int => exact False.elim hv
    | bool => exact False.elim hv
    | ref _ => exact False.elim hv
  | int _ => exact val_transport (claim_heapLe H _ n) (fun _ h => by cases h) hv
  | bool _ => exact val_transport (claim_heapLe H _ n) (fun _ h => by cases h) hv
  | ref _ _ => exact val_transport (claim_heapLe H _ n) (fun _ h => by cases h) hv

theorem claim_owns {H : Heap} {s : Ty} {v : Val} {n : Nat} (hv : ValOk H s v) :
    Owns (claim H v n) n v := by
  cases v with
  | box l =>
    cases s with
    | box t =>
      have hv' : Plain t ∧ ∃ (c : Cell) (o : Nat) (w : Val), nth H l = some c ∧
          c.val = some (o, w) ∧ ValOk H t w := hv
      obtain ⟨_, c, _, w, hc, hcv, _⟩ := hv'
      show ∃ (c : Cell) (w : Val), nth (claim H (.box l) n) l = some c ∧ c.val = some (n, w)
      exact ⟨⟨c.gen, some (n, w)⟩, w, claim_self hc hcv, rfl⟩
    | int => exact False.elim hv
    | bool => exact False.elim hv
    | ref _ => exact False.elim hv
  | int _ => exact True.intro
  | bool _ => exact True.intro
  | ref _ _ => exact True.intro

/-- let の始め: 値を末尾の変数に入れ、箱なら持ち主の記録をその変数にする -/
theorem agree_push_claim {env : Env} {H : Heap} {Γ : Ctx} {s : Ty} {v : Val} (ha : Agree env H Γ)
    (hv : ValOk H s v) (hl : Loose H Γ v) :
    Agree (env ++ [some v]) (claim H v env.length) (Γ ++ [(s, true)]) := by
  have hlen : env.length = Γ.length := ha.len
  have hsame : ∀ (j l : Nat) (c : Cell) (w : Val), nth H l = some c → c.val = some (j, w) →
      (∃ t : Ty, nth Γ j = some (t, true)) → nth (claim H v env.length) l = nth H l := by
    intro j l c w hc hcv halive
    apply claim_ne
    intro l' hvl heq
    subst hvl
    have hl' : ∃ (c : Cell) (o : Nat) (w : Val), nth H l' = some c ∧ c.val = some (o, w) ∧
        ∀ t : Ty, nth Γ o ≠ some (t, true) := hl
    obtain ⟨c1, o, w1, h1, hv1, hnl⟩ := hl'
    rw [heq] at h1
    have h4 : c = c1 := Option.some.inj (hc.symm.trans h1)
    rw [← h4] at hv1
    have h2 : (j, w) = (o, w1) := Option.some.inj (hcv.symm.trans hv1)
    have h3 : j = o := congrArg Prod.fst h2
    obtain ⟨t, ht⟩ := halive
    rw [h3] at ht
    exact hnl t ht
  have hagH : Agree env (claim H v env.length) Γ := agree_transport ha (claim_heapLe H v env.length) hsame
  constructor
  · simp only [List.length_append, List.length_cons, List.length_nil]
    omega
  · intro j t hj
    have hj' : j < Γ.length + 1 := by
      have h := nth_lt hj
      simp only [List.length_append, List.length_cons, List.length_nil] at h
      omega
    by_cases hlt : j < Γ.length
    · rw [nth_append_lt hlt] at hj
      obtain ⟨v0, he, hok, ho⟩ := hagH.live j t hj
      refine ⟨v0, ?_, hok, ho⟩
      rw [nth_append_lt (by omega : j < env.length)]
      exact he
    · have hjj : j = Γ.length := by omega
      rw [hjj, nth_append_len] at hj
      have htt : (s, true) = (t, true) := Option.some.inj hj
      have hst : s = t := congrArg Prod.fst htt
      rw [← hst, hjj, ← hlen]
      exact ⟨v, nth_append_len env (some v), claim_valOk hv, claim_owns hv⟩
  · intro j t hj
    by_cases hlt : j < Γ.length
    · rw [nth_append_lt hlt] at hj
      rw [nth_append_lt (by omega : j < env.length)]
      exact ha.dead j t hj
    · have h := nth_lt hj
      simp only [List.length_append, List.length_cons, List.length_nil] at h
      have hjj : j = Γ.length := by omega
      rw [hjj, nth_append_len] at hj
      have htt : (s, true) = (t, false) := Option.some.inj hj
      have hb : true = false := congrArg Prod.snd htt
      cases hb

/-- let の終わり: 変数が生きた箱を持っていれば区画を解放する。外の変数の箱と、結果の箱は別の区画 -/
theorem let_end {env : Env} {H : Heap} {Γ : Ctx} {p : Ty × Bool} {x : Option Val} {t : Ty} {w : Val}
    (ha : Agree (env ++ [x]) H (Γ ++ [p])) (hlen : env.length = Γ.length) (hw : ValOk H t w)
    (hl : Loose H (Γ ++ [p]) w) : Safe t Γ (.ok w env (dropSlot H (some x))) := by
  have hpop := agree_pop ha hlen
  have hlp : Loose H Γ w := loose_pop hl
  obtain ⟨tp, b⟩ := p
  cases b with
  | false =>
    have hx := ha.dead Γ.length tp (nth_append_len Γ _)
    rw [← hlen, nth_append_len] at hx
    have hx' : x = none := Option.some.inj hx
    subst hx'
    exact And.intro hw (And.intro hpop hlp)
  | true =>
    obtain ⟨v', hx, _, ho⟩ := ha.live Γ.length tp (nth_append_len Γ _)
    rw [← hlen, nth_append_len] at hx
    have hx' : x = some v' := Option.some.inj hx
    subst hx'
    cases v' with
    | box l =>
      have ho' : ∃ (c : Cell) (w0 : Val), nth H l = some c ∧ c.val = some (Γ.length, w0) := ho
      obtain ⟨c, w0, hc, hcv⟩ := ho'
      show Safe t Γ (.ok w env (release H l))
      have hle := release_heapLe H l
      have hother : ∀ (l1 : Nat) (c1 : Cell) (o : Nat) (w1 : Val), nth H l1 = some c1 →
          c1.val = some (o, w1) → o ≠ Γ.length → nth (release H l) l1 = nth H l1 := by
        intro l1 c1 o w1 h1 hv1 hne
        apply release_ne
        intro heq
        rw [heq] at h1
        have h4 : c = c1 := Option.some.inj (hc.symm.trans h1)
        rw [← h4] at hv1
        have h2 : (Γ.length, w0) = (o, w1) := Option.some.inj (hcv.symm.trans hv1)
        exact hne (congrArg Prod.fst h2).symm
      have hag' : Agree env (release H l) Γ := by
        apply agree_transport hpop hle
        intro j l1 c1 w1 h1 hv1 halive
        apply hother l1 c1 j w1 h1 hv1
        obtain ⟨t1, ht1⟩ := halive
        have := nth_lt ht1
        omega
      have hwl : ∀ lw : Nat, w = .box lw → nth (release H l) lw = nth H lw := by
        intro lw hwb
        subst hwb
        have hl' : ∃ (c : Cell) (o : Nat) (w : Val), nth H lw = some c ∧ c.val = some (o, w) ∧
            ∀ t : Ty, nth (Γ ++ [(tp, true)]) o ≠ some (t, true) := hl
        obtain ⟨c1, o, w1, h1, hv1, hnl⟩ := hl'
        refine hother lw c1 o w1 h1 hv1 ?_
        intro heq
        rw [heq] at hnl
        exact hnl tp (nth_append_len Γ _)
      exact And.intro (val_transport hle hwl hw) (And.intro hag' (loose_transport hwl hlp))
    | int _ => exact And.intro hw (And.intro hpop hlp)
    | bool _ => exact And.intro hw (And.intro hpop hlp)
    | ref _ _ => exact And.intro hw (And.intro hpop hlp)

/-! ## 一つずつの操作の安全性 -/

theorem bind_safe {s t : Ty} {Γ1 Γ2 : Ctx} {r : Res} {f : Val → Env → Heap → Res}
    (hr : Safe s Γ1 r)
    (hf : ∀ (v : Val) (env : Env) (H : Heap), ValOk H s v → Agree env H Γ1 → Loose H Γ1 v →
      Safe t Γ2 (f v env H)) :
    Safe t Γ2 (r.bind f) := by
  cases r with
  | ok v env H => exact hf v env H hr.1 hr.2.1 hr.2.2
  | err => exact True.intro
  | fuel => exact True.intro
  | stuck => exact False.elim hr

theorem checked_safe {env : Env} {H : Heap} {Γ : Ctx} (z : Int) (ha : Agree env H Γ) :
    Safe .int Γ (checked z env H) := by
  unfold checked
  split
  · exact And.intro True.intro (And.intro ha True.intro)
  · exact True.intro

theorem addOp_safe {env : Env} {H Ha Hb : Heap} {Γ : Ctx} {x y : Val} (hx : ValOk Ha .int x)
    (hy : ValOk Hb .int y) (ha : Agree env H Γ) : Safe .int Γ (addOp x y env H) := by
  cases x with
  | bool _ => exact False.elim hx
  | box _ => exact False.elim hx
  | ref _ _ => exact False.elim hx
  | int p =>
    cases y with
    | bool _ => exact False.elim hy
    | box _ => exact False.elim hy
    | ref _ _ => exact False.elim hy
    | int q => exact checked_safe (p + q) ha

theorem readVar_safe {env : Env} {H : Heap} {Γ : Ctx} {i : Nat} {t : Ty} (ha : Agree env H Γ)
    (hi : nth Γ i = some (t, true)) (hp : Plain t) : Safe t Γ (readVar env H i) := by
  obtain ⟨v, he, hok, _⟩ := ha.live i t hi
  simp only [readVar, he]
  exact And.intro hok (And.intro ha (loose_plain hp hok))

theorem moveOut_safe {env : Env} {H : Heap} {Γ : Ctx} {i : Nat} {t : Ty} (ha : Agree env H Γ)
    (hi : nth Γ i = some (.box t, true)) :
    Safe (.box t) (setNth Γ i (.box t, false)) (moveOut env H i) := by
  obtain ⟨v, he, hok, ho⟩ := ha.live i (.box t) hi
  cases v with
  | box l =>
    simp only [moveOut, he]
    have ho' : ∃ (c : Cell) (w : Val), nth H l = some c ∧ c.val = some (i, w) := ho
    obtain ⟨c, w, hc, hcv⟩ := ho'
    refine And.intro hok (And.intro (agree_kill ha hi) ?_)
    show ∃ (c : Cell) (o : Nat) (w : Val), nth H l = some c ∧ c.val = some (o, w) ∧
      ∀ t' : Ty, nth (setNth Γ i (.box t, false)) o ≠ some (t', true)
    refine ⟨c, i, w, hc, hcv, ?_⟩
    intro t' ht
    rw [nth_setNth_eq (nth_lt hi)] at ht
    have hb : false = true := congrArg Prod.snd (Option.some.inj ht)
    cases hb
  | int _ => exact False.elim hok
  | bool _ => exact False.elim hok
  | ref _ _ => exact False.elim hok

theorem peekAt_safe {env : Env} {H : Heap} {Γ : Ctx} {i : Nat} {t : Ty} (ha : Agree env H Γ)
    (hi : nth Γ i = some (.box t, true)) : Safe t Γ (peekAt env H i) := by
  obtain ⟨v, he, hok, _⟩ := ha.live i (.box t) hi
  cases v with
  | box l =>
    have hv' : Plain t ∧ ∃ (c : Cell) (o : Nat) (w : Val), nth H l = some c ∧
        c.val = some (o, w) ∧ ValOk H t w := hok
    obtain ⟨hp, c, _, w, hc, hcv, hw⟩ := hv'
    simp only [peekAt, he, contents, hc, hcv]
    exact And.intro hw (And.intro ha (loose_plain hp hw))
  | int _ => exact False.elim hok
  | bool _ => exact False.elim hok
  | ref _ _ => exact False.elim hok

theorem addrOf_safe {env : Env} {H : Heap} {Γ : Ctx} {i : Nat} {t : Ty} (ha : Agree env H Γ)
    (hi : nth Γ i = some (.box t, true)) : Safe (.ref t) Γ (addrOf env H i) := by
  obtain ⟨v, he, hok, _⟩ := ha.live i (.box t) hi
  cases v with
  | box l =>
    have hv' : Plain t ∧ ∃ (c : Cell) (o : Nat) (w : Val), nth H l = some c ∧
        c.val = some (o, w) ∧ ValOk H t w := hok
    obtain ⟨hp, c, o, w, hc, hcv, hw⟩ := hv'
    simp only [addrOf, he, hc]
    refine And.intro ?_ (And.intro ha True.intro)
    show Plain t ∧ ∃ c' : Cell, nth H l = some c' ∧ c.gen ≤ c'.gen ∧
      (c.gen = c'.gen → ∃ (o' : Nat) (w' : Val), c'.val = some (o', w') ∧ ValOk H t w')
    exact ⟨hp, c, hc, Nat.le_refl _, fun _ => ⟨o, w, hcv, hw⟩⟩
  | int _ => exact False.elim hok
  | bool _ => exact False.elim hok
  | ref _ _ => exact False.elim hok

theorem derefOp_safe {env : Env} {H : Heap} {Γ : Ctx} {t : Ty} {v : Val} (hv : ValOk H (.ref t) v)
    (ha : Agree env H Γ) : Safe t Γ (derefOp v env H) := by
  cases v with
  | ref l g =>
    have hv' : Plain t ∧ ∃ c : Cell, nth H l = some c ∧ g ≤ c.gen ∧
        (g = c.gen → ∃ (o : Nat) (w : Val), c.val = some (o, w) ∧ ValOk H t w) := hv
    obtain ⟨hp, c, hc, _, hlive⟩ := hv'
    simp only [derefOp, hc]
    by_cases heq : c.gen = g
    · rw [if_pos heq]
      obtain ⟨_, w, hw, hok⟩ := hlive heq.symm
      simp only [contents, hc, hw]
      exact And.intro hok (And.intro ha (loose_plain hp hok))
    · rw [if_neg heq]
      exact True.intro
  | int _ => exact False.elim hv
  | bool _ => exact False.elim hv
  | box _ => exact False.elim hv

theorem mkBox_safe {pick : Heap → Option Nat} (hp : PickOk pick) {env : Env} {H : Heap} {Γ : Ctx}
    {t : Ty} {v : Val} (ha : Agree env H Γ) (hv : ValOk H t v) (hpl : Plain t) :
    Safe (.box t) Γ
      (.ok (.box (alloc pick H (env.length, v)).1) env (alloc pick H (env.length, v)).2) := by
  obtain ⟨hle, hkeep, c', hnew, hcv'⟩ := alloc_spec hp H (env.length, v)
  have hlen : env.length = Γ.length := ha.len
  refine And.intro ?_ (And.intro ?_ ?_)
  · show Plain t ∧ ∃ (c : Cell) (o : Nat) (w : Val),
      nth (alloc pick H (env.length, v)).2 (alloc pick H (env.length, v)).1 = some c ∧
      c.val = some (o, w) ∧ ValOk (alloc pick H (env.length, v)).2 t w
    exact ⟨hpl, c', env.length, v, hnew, hcv', valOk_mono_plain hle t hpl v hv⟩
  · apply agree_transport ha hle
    intro j l c w hc hcv _
    rw [hkeep l c (j, w) hc hcv, hc]
  · show ∃ (c : Cell) (o : Nat) (w : Val),
      nth (alloc pick H (env.length, v)).2 (alloc pick H (env.length, v)).1 = some c ∧
      c.val = some (o, w) ∧ ∀ t' : Ty, nth Γ o ≠ some (t', true)
    refine ⟨c', env.length, v, hnew, hcv', ?_⟩
    intro t' ht
    rw [hlen] at ht
    have := nth_lt ht
    omega

/-! ## 主定理 -/

/-- 型安全性・所有権・世代の検査: 型が付く式を、文脈と合う環境・ヒープで、条件を満たすどの割り当ての方針で評価しても、結果は Safe -/
theorem eval_safe {pick : Heap → Option Nat} (hp : PickOk pick) :
    ∀ (k : Nat) (env : Env) (H : Heap) (Γ : Ctx) (e : Expr) (t : Ty) (Γ' : Ctx),
      Agree env H Γ → HasTy Γ e t Γ' → Safe t Γ' (eval pick k env H e) := by
  intro k
  induction k with
  | zero =>
    intro env H Γ e t Γ' _ _
    cases e <;> exact True.intro
  | succ k ih =>
    intro env H Γ e t Γ' hag h
    cases h with
    | int _ _ => exact And.intro True.intro (And.intro hag True.intro)
    | bool _ _ => exact And.intro True.intro (And.intro hag True.intro)
    | var _ _ _ hi hc =>
      simp only [eval]
      exact readVar_safe hag hi hc
    | move _ _ _ hi =>
      simp only [eval]
      exact moveOut_safe hag hi
    | peek _ _ _ hi =>
      simp only [eval]
      exact peekAt_safe hag hi
    | addr _ _ _ hi =>
      simp only [eval]
      exact addrOf_safe hag hi
    | deref _ _ e _ he =>
      simp only [eval]
      apply bind_safe (ih env H Γ e (.ref t) Γ' hag he)
      intro v env1 H1 hv ha1 _
      exact derefOp_safe hv ha1
    | mkBox _ _ e t0 he hc =>
      simp only [eval]
      apply bind_safe (ih env H Γ e t0 Γ' hag he)
      intro v env1 H1 hv ha1 _
      exact mkBox_safe hp ha1 hv hc
    | add _ Γ1 _ a b ha hb =>
      simp only [eval]
      apply bind_safe (ih env H Γ a .int Γ1 hag ha)
      intro x env1 H1 hx ha1 _
      apply bind_safe (ih env1 H1 Γ1 b .int Γ' ha1 hb)
      intro y env2 H2 hy ha2 _
      exact addOp_safe hx hy ha2
    | letE _ Γ1 _ x body s _ p hx hb =>
      simp only [eval]
      apply bind_safe (ih env H Γ x s Γ1 hag hx)
      intro v env1 H1 hv ha1 hl1
      apply bind_safe (ih _ _ (Γ1 ++ [(s, true)]) body t (Γ' ++ [p]) (agree_push_claim ha1 hv hl1) hb)
      intro w env2 H2 hw ha2 hl2
      obtain ⟨env', x', he, hlen⟩ := split_last env2 Γ'.length (by
        have h2 := ha2.len
        simp only [List.length_append, List.length_cons, List.length_nil] at h2
        omega)
      subst he
      simp only [init_append, lastOpt_append]
      exact let_end ha2 hlen hw hl2

/-- 型が付く式は、どれだけの燃料でも、どの割り当ての方針でも行き詰まらない
（移動済みの変数・解放済みの区画・使い回された区画の別の型の中身を使わない） -/
theorem no_stuck {pick : Heap → Option Nat} (hp : PickOk pick) (k : Nat) (env : Env) (H : Heap)
    (Γ : Ctx) (e : Expr) (t : Ty) (Γ' : Ctx) (hag : Agree env H Γ) (h : HasTy Γ e t Γ') :
    eval pick k env H e ≠ .stuck := by
  intro hs
  have hsafe := eval_safe hp k env H Γ e t Γ' hag h
  rw [hs] at hsafe
  exact hsafe

/-! ## 意味を持つことの確認 -/

theorem pick_none_ok : PickOk (fun _ => none) := by
  unfold PickOk
  intro _ l h
  have h' : (none : Option Nat) = some l := h
  cases h'

/-- 箱への ref を let の外へ持ち出す: `deref (let b = box 1 in &b)`。型が付く -/
theorem escaped_typed :
    HasTy [] (.deref (.letE (.mkBox (.int 1)) (.addr 0))) .int [] :=
  HasTy.deref _ _ _ _
    (HasTy.letE [] [] [] _ _ (.box .int) (.ref .int) (.box .int, true)
      (HasTy.mkBox _ _ _ _ (HasTy.int _ _) True.intro)
      (HasTy.addr _ 0 .int rfl))

/-- let の終わりに箱の区画が解放され（世代 1）、持ち出した ref（世代 0）を読むと実行時の誤りで止まる -/
theorem escaped_ref_err :
    eval (fun _ => none) 4 [] [] (.deref (.letE (.mkBox (.int 1)) (.addr 0))) = .err := rfl

end JabberwockFormal.CoreGenBox
