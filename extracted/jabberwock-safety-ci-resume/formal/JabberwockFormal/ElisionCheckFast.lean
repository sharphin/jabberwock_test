import JabberwockFormal.ElisionCheck

set_option autoImplicit false
set_option warningAsError true

namespace JabberwockFormal.ElisionCheckFast

/-!
# 検査器の速い版（配列で事実を持つ）

`ElisionCheck.verifyFunc` は事実を関数で持つので、大きな関数（jwc 自身には 2000 を超える変数と
8 万を超える命令の関数がある）では遅すぎる。ここでは事実を配列で持ち、操作ごとに配列を作り直す版を書き、
「速い版が通れば、証明済みの `verifyFunc` も通る」ことを証明する（`verifyFast_sound`）。
-/

open JabberwockFormal.ElisionSoundness
open JabberwockFormal.ElisionCheck

abbrev FA := Array (Option Fact)

theorem toFacts_mapIdx (a : FA) (g : Nat → Option Fact → Option Fact) (hg : ∀ i, g i none = none)
    (z : Nat) : toFacts (a.mapIdx g) z = g z (toFacts a z) := by
  by_cases h : z < a.size
  · simp [toFacts, Array.getD, h]
  · simp [toFacts, Array.getD, h, hg]

theorem toFacts_set (a : FA) (x : Nat) (v : Option Fact) (hx : x < a.size) (z : Nat) :
    toFacts (a.set! x v) z = if z = x then v else toFacts a z := by
  by_cases e : z = x
  · subst e; simp [toFacts, Array.getD, hx]
  · by_cases h : z < a.size
    · simp [toFacts, Array.getD, h, e, Ne.symm e]
    · simp [toFacts, Array.getD, h, e]

def killAllA (a : FA) (x : Local) : FA := a.mapIdx fun z v =>
  if z = x then none else
  match v with
  | some f => if mentions f x then none else some f
  | none => none

def killValueA (a : FA) (x : Local) : FA := a.mapIdx fun z v =>
  if z = x then none else
  match v with
  | some f => if mentionsValue f x then none else some f
  | none => none

def killElemsA (a : FA) : FA := a.mapIdx fun _ v =>
  match v with
  | some f => if isElem f then none else some f
  | none => none

def killElemsIfA (holds : Bool) (a : FA) : FA := if holds then killElemsA a else a

def setFactA (a : FA) (x : Local) (f : Option Fact) : FA :=
  match f with
  | some (.refTo y) => if y = x then a else a.set! x f
  | some (.refSlot y) => if y = x then a else a.set! x f
  | some (.refElem y) => if y = x then a else a.set! x f
  | some g => a.set! x (some g)
  | none => a

theorem killAllA_eq (a : FA) (x : Local) : toFacts (killAllA a x) = killAll (toFacts a) x := by
  funext z
  rw [killAllA, toFacts_mapIdx _ _ (fun i => by by_cases e : i = x <;> simp [e])]
  rfl

theorem killValueA_eq (a : FA) (x : Local) : toFacts (killValueA a x) = killValue (toFacts a) x := by
  funext z
  rw [killValueA, toFacts_mapIdx _ _ (fun i => by by_cases e : i = x <;> simp [e])]
  rfl

theorem killElemsA_eq (a : FA) : toFacts (killElemsA a) = killElems (toFacts a) := by
  funext z
  rw [killElemsA, toFacts_mapIdx _ _ (fun _ => rfl)]
  rfl

theorem killElemsIfA_eq (holds : Bool) (a : FA) :
    toFacts (killElemsIfA holds a) = killElemsIf holds (toFacts a) := by
  cases holds
  · rfl
  · exact killElemsA_eq a

theorem size_killAllA (a : FA) (x : Local) : (killAllA a x).size = a.size := by simp [killAllA]
theorem size_killValueA (a : FA) (x : Local) : (killValueA a x).size = a.size := by simp [killValueA]
theorem size_killElemsIfA (holds : Bool) (a : FA) : (killElemsIfA holds a).size = a.size := by
  cases holds <;> simp [killElemsIfA, killElemsA]

theorem size_setFactA (a : FA) (x : Local) (f : Option Fact) : (setFactA a x f).size = a.size := by
  unfold setFactA
  rcases f with _ | (_ | y | y | _ | y)
  · rfl
  · simp
  · by_cases e : y = x <;> simp [e]
  · by_cases e : y = x <;> simp [e]
  · simp
  · by_cases e : y = x <;> simp [e]

theorem setFactA_eq (a : FA) (x : Local) (f : Option Fact) (hx : x < a.size) :
    toFacts (setFactA a x f) = setFact (toFacts a) x f := by
  funext z
  unfold setFactA setFact
  rcases f with _ | (_ | y | y | _ | y)
  · rfl
  · rw [toFacts_set _ _ _ hx]
  · by_cases e : y = x
    · simp [e]
    · simp only [e, if_false]; rw [toFacts_set _ _ _ hx]
  · by_cases e : y = x
    · simp [e]
    · simp only [e, if_false]; rw [toFacts_set _ _ _ hx]
  · rw [toFacts_set _ _ _ hx]
  · by_cases e : y = x
    · simp [e]
    · simp only [e, if_false]; rw [toFacts_set _ _ _ hx]

/-- 操作の中の変数（事実を入れる先が配列の中にあること） -/
def target : Op → Option Local
  | .assignOwn x _ | .copyRef x _ | .moveRef x _ | .borrowOwn x _ | .reborrow x _ _
  | .borrowSlot x _ | .borrowElemOwn x _ | .borrowElemRef x _ | .borrowElemList x _ => some x
  | _ => none

def inRange (n : Nat) (o : Op) : Bool :=
  match target o with
  | some x => x < n
  | none => true

def transferA (a : FA) : Op → FA
  | .drop x holds => killAllA (killElemsIfA holds a) x
  | .moveOut x => killAllA a x
  | .assignOwn x holds => setFactA (killValueA (killElemsIfA holds a) x) x (some .own)
  | .assignNone x holds => killValueA (killElemsIfA holds a) x
  | .copyRef x s => setFactA (killValueA a x) x (toFacts a s)
  | .moveRef x s => setFactA (killValueA (killAllA a s) x) x (toFacts a s)
  | .borrowOwn r y =>
      setFactA (killValueA a r) r (if toFacts a y = some .own then some (.refTo y) else none)
  | .reborrow r s live =>
      setFactA (killValueA a r) r
        (if toFacts a s = some .own then none
         else if live && !isLive (toFacts a) (toFacts a s) then none else toFacts a s)
  | .borrowSlot r y => setFactA (killValueA a r) r (some (.refSlot y))
  | .borrowElemOwn r y =>
      setFactA (killValueA a r) r (if toFacts a y = some .own then some (.refElem y) else none)
  | .borrowElemRef r y =>
      setFactA (killValueA a r) r (if toFacts a y = some .pinned then some (.refElem y) else none)
  | .borrowElemList r y => setFactA (killValueA a r) r (some (.refElem y))
  | .writeInside holds => killElemsIfA holds a
  | .elems => killElemsA a
  | .pure => a
  | .access _ => a

theorem size_transferA (a : FA) (o : Op) : (transferA a o).size = a.size := by
  cases o <;> simp [transferA, size_setFactA, size_killValueA, size_killAllA, size_killElemsIfA,
    killElemsA]

theorem transferA_eq (a : FA) (o : Op) (r : inRange a.size o = true) :
    toFacts (transferA a o) = transfer (toFacts a) o := by
  cases o with
  | drop x holds => simp [transferA, transfer, killAllA_eq, killElemsIfA_eq]
  | moveOut x => simp [transferA, transfer, killAllA_eq]
  | assignOwn x holds =>
    simp only [inRange, target, decide_eq_true_eq] at r
    simp only [transferA, transfer]
    rw [setFactA_eq _ _ _ (by rw [size_killValueA, size_killElemsIfA]; exact r), killValueA_eq,
      killElemsIfA_eq]
  | assignNone x holds => simp [transferA, transfer, killValueA_eq, killElemsIfA_eq]
  | copyRef x s =>
    simp only [inRange, target, decide_eq_true_eq] at r
    simp only [transferA, transfer]
    rw [setFactA_eq _ _ _ (by rw [size_killValueA]; exact r), killValueA_eq]
  | moveRef x s =>
    simp only [inRange, target, decide_eq_true_eq] at r
    simp only [transferA, transfer]
    rw [setFactA_eq _ _ _ (by rw [size_killValueA, size_killAllA]; exact r), killValueA_eq, killAllA_eq]
  | borrowOwn x y =>
    simp only [inRange, target, decide_eq_true_eq] at r
    simp only [transferA, transfer]
    rw [setFactA_eq _ _ _ (by rw [size_killValueA]; exact r), killValueA_eq]
  | reborrow x y live =>
    simp only [inRange, target, decide_eq_true_eq] at r
    simp only [transferA, transfer]
    rw [setFactA_eq _ _ _ (by rw [size_killValueA]; exact r), killValueA_eq]
  | borrowSlot x y =>
    simp only [inRange, target, decide_eq_true_eq] at r
    simp only [transferA, transfer]
    rw [setFactA_eq _ _ _ (by rw [size_killValueA]; exact r), killValueA_eq]
  | borrowElemOwn x y =>
    simp only [inRange, target, decide_eq_true_eq] at r
    simp only [transferA, transfer]
    rw [setFactA_eq _ _ _ (by rw [size_killValueA]; exact r), killValueA_eq]
  | borrowElemRef x y =>
    simp only [inRange, target, decide_eq_true_eq] at r
    simp only [transferA, transfer]
    rw [setFactA_eq _ _ _ (by rw [size_killValueA]; exact r), killValueA_eq]
  | borrowElemList x y =>
    simp only [inRange, target, decide_eq_true_eq] at r
    simp only [transferA, transfer]
    rw [setFactA_eq _ _ _ (by rw [size_killValueA]; exact r), killValueA_eq]
  | writeInside holds => simp [transferA, transfer, killElemsIfA_eq]
  | elems => simp [transferA, transfer, killElemsA_eq]
  | pure => rfl
  | access _ => rfl

/-- runC の配列版 -/
def runA (P : PinTable) : FA → List COp → Option FA
  | a, [] => some a
  | a, .op o :: rest =>
    if inRange a.size o && preB (toFacts a) o then runA P (transferA a o) rest else none
  | a, .elided r :: rest => if safeDeref (toFacts a) (toFacts a r) then runA P a rest else none
  | a, .live x f i :: rest => if !P f i || isLive (toFacts a) (toFacts a x) then runA P a rest else none
  | a, .notLive f i :: rest => if !P f i then runA P a rest else none
  | a, .escape _ :: rest => runA P a rest
  | _, .bad _ :: _ => none

theorem runA_sound {P : PinTable} {a b : FA} {cs : List COp} (h : runA P a cs = some b) :
    runC P (toFacts a) cs = some (toFacts b) := by
  induction cs generalizing a with
  | nil => simp [runA] at h; subst h; rfl
  | cons c cs ih =>
    cases c with
    | op o =>
      simp only [runA] at h
      split at h
      · rename_i ok
        simp only [Bool.and_eq_true] at ok
        simp only [runC, ok.2, if_true]
        rw [← transferA_eq a o ok.1]
        exact ih h
      · simp at h
    | elided r =>
      simp only [runA] at h
      split at h
      · rename_i ok; simp only [runC, ok, if_true]; exact ih h
      · simp at h
    | live x f i =>
      simp only [runA] at h
      split at h
      · rename_i ok; simp only [runC, ok, if_true]; exact ih h
      · simp at h
    | notLive f i =>
      simp only [runA] at h
      split at h
      · rename_i ok; simp only [runC, ok, if_true]; exact ih h
      · simp at h
    | escape f => simp only [runA] at h; simp only [runC]; exact ih h
    | bad _ => simp [runA] at h

def blockFast (N : Nat) (P : PinTable) (c : Code) (E : Entry) (b : Nat) : Bool :=
  match E[b]?.join with
  | none => true
  | some eb =>
    eb.size ≤ N &&
    match runA P eb (c.blocks.getD b []) with
    | none => false
    | some out => (c.succ.getD b []).all (fun d =>
        match E[d]?.join with
        | none => false
        | some ed => ed.size ≤ N && subB N (toFacts ed) (toFacts out))

theorem blockOk_of_fast {N : Nat} {P : PinTable} {c : Code} {E : Entry} {b : Nat}
    (h : blockFast N P c E b = true) : blockOk N P c E b = true := by
  unfold blockFast at h
  unfold blockOk
  split
  · rfl
  · rename_i eb heb
    rw [heb] at h
    simp only [Bool.and_eq_true] at h ⊢
    refine ⟨h.1, ?_⟩
    have h2 := h.2
    split at h2
    · simp at h2
    · rename_i out hrun
      rw [runA_sound hrun]
      exact h2

def verifyFast (N : Nat) (P : PinTable) (start : Facts) (c : Code) (E : Entry) : Bool :=
  match E[0]?.join with
  | none => false
  | some e0 => e0.size ≤ N && subB N (toFacts e0) start &&
      (List.range c.blocks.size).all (fun b => blockFast N P c E b) &&
      (List.range E.size).all (fun b => b < c.blocks.size || E[b]?.join.isNone)

/-- 速い版が通れば、証明済みの verifyFunc も通る（だから `verify_elided` / `verify_live` が使える） -/
theorem verifyFast_sound {N : Nat} {P : PinTable} {start : Facts} {c : Code} {E : Entry}
    (h : verifyFast N P start c E = true) : verifyFunc N P start c E = true := by
  unfold verifyFast at h
  unfold verifyFunc
  split
  · rename_i h0; rw [h0] at h; simp at h
  · rename_i e0 h0
    rw [h0] at h
    simp only [Bool.and_eq_true] at h ⊢
    refine ⟨⟨h.1.1, ?_⟩, h.2⟩
    exact List.all_eq_true.mpr fun b hb => blockOk_of_fast (List.all_eq_true.mp h.1.2 b hb)

end JabberwockFormal.ElisionCheckFast
