import JabberwockFormal.ElisionSoundness

set_option autoImplicit false
set_option warningAsError true

namespace JabberwockFormal.ElisionCheck

open JabberwockFormal.ElisionSoundness

/-!
# 1 本ずつ確かめる検査器（その正しさ）

コンパイルした関数ごとに、IR を模型の操作の列（`COp`）に読み替えたもの（読み替えは `Checker/` にある）を受け取り、
- ブロックの入口の事実（検査器が自分で計算したもの）が、合流の条件を満たすか（`subB`）
- 検査を消した `*r`（`elided r`）で、事実から安全と言えるか
- Pinned の引数に渡す値（`live`）が、生きた ref か
- 型の決まり（`Pre`）を満たすか
を確かめる（`verifyFunc`）。ここでは「`verifyFunc` が true なら、実際の実行で、消した検査はどれも必ず通り、
Pinned の引数には必ず生きた ref が渡る」ことを証明する（`verify_elided` / `verify_live`）。
-/

/-- 検査器が読む命令: 模型の操作と、確かめること -/
inductive COp where
  | op (o : Op)
  /-- ここで `*r` の検査が消されている -/
  | elided (r : Local)
  /-- 関数 f の i 番目の引数として a を渡す（f の i 番目が Pinned なら、a は生きた ref でなければならない） -/
  | live (a : Local) (f i : Nat)
  /-- 関数 f の i 番目の引数に、生きた ref と言えない値を渡す（f の i 番目は Pinned であってはならない） -/
  | notLive (f i : Nat)
  /-- 関数 f が値として使われる・スレッドの入口・main（どれも Pinned の引数を持ってはならない） -/
  | escape (f : Nat)
  /-- 読み替えられない形（検査器は通さない） -/
  | bad (why : String)

def opsOf : List COp → List Op
  | [] => []
  | .op o :: rest => o :: opsOf rest
  | _ :: rest => opsOf rest

def preB (F : Facts) : Op → Bool
  | .copyRef _ src => F src != some .own
  | .moveRef x src => F src != some .own && x != src
  | _ => true

theorem pre_of_preB {F : Facts} {o : Op} (h : preB F o = true) : Pre F o := by
  cases o <;> simp_all [preB, Pre]

/-- 関数ごとの Pinned の引数（p f i が true なら、f の i 番目は Pinned） -/
abbrev PinTable := Nat → Nat → Bool

/-- 事実をたどりながら、確かめることを確かめる。だめなら none -/
def runC (P : PinTable) : Facts → List COp → Option Facts
  | F, [] => some F
  | F, .op o :: rest => if preB F o then runC P (transfer F o) rest else none
  | F, .elided r :: rest => if safeDeref F (F r) then runC P F rest else none
  | F, .live a f i :: rest => if !P f i || isLive F (F a) then runC P F rest else none
  | F, .notLive f i :: rest => if !P f i then runC P F rest else none
  | F, .escape _ :: rest => runC P F rest
  | _, .bad _ :: _ => none

/-- 実際の実行（模型の操作を順に） -/
inductive Steps : List Op → St → St → Prop where
  | nil (s : St) : Steps [] s s
  | cons {o : Op} {os : List Op} {s s' s'' : St} : Exec o s s' → Steps os s' s'' → Steps (o :: os) s s''

theorem steps_append {a b : List Op} {s s'' : St} (h : Steps (a ++ b) s s'') :
    ∃ s', Steps a s s' ∧ Steps b s' s'' := by
  induction a generalizing s with
  | nil => exact ⟨s, .nil s, h⟩
  | cons o os ih =>
    cases h with
    | cons e rest =>
      rcases ih rest with ⟨s', r1, r2⟩
      exact ⟨s', .cons e r1, r2⟩

theorem opsOf_append (a b : List COp) : opsOf (a ++ b) = opsOf a ++ opsOf b := by
  induction a with
  | nil => rfl
  | cons c cs ih => cases c <;> simp [opsOf, ih]

/-- runC が通れば、実行のあとも事実は正しい -/
theorem runC_sound {P : PinTable} {F out : Facts} {cs : List COp} {s s' : St}
    (ok : runC P F cs = some out) (sound : Sound F s) (steps : Steps (opsOf cs) s s') : Sound out s' := by
  induction cs generalizing F s with
  | nil =>
    cases steps
    simp [runC] at ok
    exact ok ▸ sound
  | cons c cs ih =>
    cases c with
    | op o =>
      simp only [runC] at ok
      split at ok
      · rename_i pb
        cases steps with
        | cons e rest => exact ih ok (step_sound sound (pre_of_preB pb) e) rest
      · simp at ok
    | elided r =>
      simp only [runC] at ok
      split at ok
      · exact ih ok sound steps
      · simp at ok
    | live a f i =>
      simp only [runC] at ok
      split at ok
      · exact ih ok sound steps
      · simp at ok
    | notLive f i =>
      simp only [runC] at ok
      split at ok
      · exact ih ok sound steps
      · simp at ok
    | escape f => exact ih ok sound steps
    | bad _ => simp [runC] at ok

/-- 前半を通れば、後半は前半の出口の事実から通る -/
theorem runC_append {P : PinTable} {F out : Facts} {a b : List COp}
    (ok : runC P F (a ++ b) = some out) : ∃ mid, runC P F a = some mid ∧ runC P mid b = some out := by
  induction a generalizing F with
  | nil => exact ⟨F, rfl, ok⟩
  | cons c cs ih =>
    cases c with
    | op o =>
      simp only [List.cons_append, runC] at ok ⊢
      split at ok
      · rename_i pb; simp only [pb, if_true]; exact ih ok
      · simp at ok
    | elided r =>
      simp only [List.cons_append, runC] at ok ⊢
      split at ok
      · rename_i pb; simp only [pb, if_true]; exact ih ok
      · simp at ok
    | live a f i =>
      simp only [List.cons_append, runC] at ok ⊢
      split at ok
      · rename_i pb; simp only [pb, if_true]; exact ih ok
      · simp at ok
    | notLive f i =>
      simp only [List.cons_append, runC] at ok ⊢
      split at ok
      · rename_i pb; simp only [pb, if_true]; exact ih ok
      · simp at ok
    | escape f => simpa [runC] using ih ok
    | bad _ => simp [runC] at ok

/-- ブロックの途中の `elided r`: そこまでの実行のあと、検査は必ず通る -/
theorem runC_elided {P : PinTable} {F out : Facts} {pre post : List COp} {r : Local} {s s1 : St}
    (ok : runC P F (pre ++ .elided r :: post) = some out) (sound : Sound F s)
    (steps : Steps (opsOf pre) s s1) : ∃ mid, Sound mid s1 ∧ AccessOk mid s1 r := by
  rcases runC_append ok with ⟨mid, h1, h2⟩
  have sm := runC_sound h1 sound steps
  simp only [runC] at h2
  split at h2
  · rename_i safe
    exact ⟨mid, sm, elided_access_is_valid sm safe⟩
  · simp at h2

/-- ブロックの途中の `live a f i` で f の i 番目が Pinned なら、そこで a は生きた ref（リストの要素でもない） -/
theorem runC_live {P : PinTable} {F out : Facts} {pre post : List COp} {a : Local} {f i : Nat}
    {s s1 : St} (ok : runC P F (pre ++ .live a f i :: post) = some out) (sound : Sound F s)
    (steps : Steps (opsOf pre) s s1) (pinned : P f i = true) :
    ∃ b g, s1.refv a = some (b, g) ∧ s1.gen b = g ∧ s1.cls b ≠ .list := by
  rcases runC_append ok with ⟨mid, h1, h2⟩
  have sm := runC_sound h1 sound steps
  simp only [runC, pinned] at h2
  split at h2
  · rename_i live
    simp at live
    exact live_arg_is_valid sm live
  · simp at h2

/-- `notLive f i` / `escape f` を通ったなら、f の i 番目は Pinned でない -/
theorem runC_notLive {P : PinTable} {F out : Facts} {pre post : List COp} {f i : Nat}
    (ok : runC P F (pre ++ .notLive f i :: post) = some out) : P f i = false := by
  rcases runC_append ok with ⟨mid, _, h2⟩
  simp only [runC] at h2
  split at h2
  · rename_i h; simpa using h
  · simp at h2

/-! ## 関数全体 -/

/-- 配列で持った事実（範囲の外は事実なし） -/
def toFacts (a : Array (Option Fact)) : Facts := fun z => a.getD z none

/-- G ⊆ F を、0 から N 未満の変数で確かめる -/
def subB (N : Nat) (G F : Facts) : Bool :=
  (List.range N).all (fun z => match G z with
    | none => true
    | some f => F z == some f)

theorem sub_of_subB {N : Nat} {a : Array (Option Fact)} {F : Facts} (size : a.size ≤ N)
    (h : subB N (toFacts a) F = true) : Sub (toFacts a) F := by
  intro z f hz
  have zlt : z < N := by
    by_cases lt : z < a.size
    · exact Nat.lt_of_lt_of_le lt size
    · simp [toFacts, Array.getD, lt] at hz
  have := List.all_eq_true.mp h z (List.mem_range.mpr zlt)
  rw [hz] at this
  simpa using this

/-- 関数の読み替え: ブロックごとの命令と次のブロック -/
structure Code where
  blocks : Array (List COp)
  succ : Array (List Nat)

/-- 検査器が計算したもの: たどり着くブロックの入口の事実（たどり着かないブロックは none） -/
abbrev Entry := Array (Option (Array (Option Fact)))

def blockOk (N : Nat) (P : PinTable) (c : Code) (E : Entry) (b : Nat) : Bool :=
  match E[b]?.join with
  | none => true
  | some eb =>
    eb.size ≤ N &&
    match runC P (toFacts eb) (c.blocks.getD b []) with
    | none => false
    | some out => (c.succ.getD b []).all (fun d =>
        match E[d]?.join with
        | none => false
        | some ed => ed.size ≤ N && subB N (toFacts ed) out)

/-- 主な検査: 入口（ブロック 0）の事実が、関数が始まったときに成り立つ事実 start に含まれ、どのブロックも通る -/
def verifyFunc (N : Nat) (P : PinTable) (start : Facts) (c : Code) (E : Entry) : Bool :=
  match E[0]?.join with
  | none => false
  | some e0 => e0.size ≤ N && subB N (toFacts e0) start &&
      (List.range c.blocks.size).all (fun b => blockOk N P c E b) &&
      (List.range E.size).all (fun b => b < c.blocks.size || E[b]?.join.isNone)

/-- 実際の実行がブロックからブロックへ進む（各ブロックの命令を全部実行して、次のブロックの入口へ） -/
inductive Path (c : Code) : Nat → St → Nat → St → Prop where
  | here (b : Nat) (s : St) : Path c b s b s
  | step {b d e : Nat} {s s' s'' : St} :
      Steps (opsOf (c.blocks.getD b [])) s s' → d ∈ c.succ.getD b [] → Path c d s' e s'' → Path c b s e s''

theorem blockOk_of_verify {N : Nat} {P : PinTable} {start : Facts} {c : Code} {E : Entry}
    (v : verifyFunc N P start c E = true) (b : Nat) : blockOk N P c E b = true := by
  unfold verifyFunc at v
  split at v
  · simp at v
  · simp only [Bool.and_eq_true] at v
    rcases v with ⟨⟨_, all⟩, outside⟩
    by_cases lt : b < c.blocks.size
    · exact List.all_eq_true.mp all b (List.mem_range.mpr lt)
    · unfold blockOk
      by_cases le : b < E.size
      · have := List.all_eq_true.mp outside b (List.mem_range.mpr le)
        simp [lt] at this
        simp [this]
      · have : E[b]? = none := by simp [le]
        simp [this]

/-- たどり着いたブロックの入口では、事実は検査器の計算どおりにあり、正しい -/
theorem path_entry {N : Nat} {P : PinTable} {start : Facts} {c : Code} {E : Entry}
    (v : verifyFunc N P start c E = true) {b d : Nat} {s s' : St} {eb : Array (Option Fact)}
    (hb : E[b]?.join = some eb) (sound : Sound (toFacts eb) s) (path : Path c b s d s') :
    ∃ ed, E[d]?.join = some ed ∧ Sound (toFacts ed) s' := by
  induction path generalizing eb with
  | here => exact ⟨eb, hb, sound⟩
  | @step b d e s s' s'' steps hd _ ih =>
    have ok := blockOk_of_verify v b
    unfold blockOk at ok
    rw [hb] at ok
    simp only [Bool.and_eq_true] at ok
    rcases ok with ⟨_, ok⟩
    split at ok
    · simp at ok
    · rename_i out hrun
      have so := runC_sound hrun sound steps
      have := List.all_eq_true.mp ok d hd
      split at this
      · simp at this
      · rename_i ed hed
        simp only [Bool.and_eq_true] at this
        exact ih hed (sound_of_sub so (sub_of_subB (of_decide_eq_true this.1) this.2))

theorem entry_block {N : Nat} {P : PinTable} {start : Facts} {c : Code} {E : Entry} {s0 : St}
    (v : verifyFunc N P start c E = true) (sound : Sound start s0) :
    ∃ e0, E[0]?.join = some e0 ∧ Sound (toFacts e0) s0 := by
  unfold verifyFunc at v
  split at v
  · simp at v
  · rename_i e0 h0
    simp only [Bool.and_eq_true] at v
    exact ⟨e0, h0, sound_of_sub sound (sub_of_subB (of_decide_eq_true v.1.1.1) v.1.1.2)⟩

theorem block_run {N : Nat} {P : PinTable} {c : Code} {E : Entry} {d : Nat} {ed : Array (Option Fact)}
    (ok : blockOk N P c E d = true) (hd : E[d]?.join = some ed) :
    ∃ out, runC P (toFacts ed) (c.blocks.getD d []) = some out := by
  unfold blockOk at ok
  rw [hd] at ok
  simp only [Bool.and_eq_true] at ok
  rcases ok with ⟨_, ok⟩
  split at ok
  · simp at ok
  · rename_i out h
    exact ⟨out, h⟩

/-- 主な定理 A: verifyFunc が通った関数では、関数の始まりで start が正しければ、
    どの実行でも、検査を消した `*r` に来たとき、その検査は必ず通る -/
theorem verify_elided {N : Nat} {P : PinTable} {start : Facts} {c : Code} {E : Entry}
    (v : verifyFunc N P start c E = true) {s0 s s1 : St} {d : Nat} {pre post : List COp} {r : Local}
    (entry : Sound start s0) (path : Path c 0 s0 d s)
    (block : c.blocks.getD d [] = pre ++ .elided r :: post) (steps : Steps (opsOf pre) s s1) :
    ∃ F, Sound F s1 ∧ AccessOk F s1 r := by
  rcases entry_block v entry with ⟨e0, h0, s0ok⟩
  rcases path_entry v h0 s0ok path with ⟨ed, hd, sd⟩
  rcases block_run (blockOk_of_verify v d) hd with ⟨out, hrun⟩
  rw [block] at hrun
  exact runC_elided hrun sd steps

/-- 主な定理 B: verifyFunc が通った関数で、Pinned の引数に渡す値は、渡すときに必ず生きた ref -/
theorem verify_live {N : Nat} {P : PinTable} {start : Facts} {c : Code} {E : Entry}
    (v : verifyFunc N P start c E = true) {s0 s s1 : St} {d : Nat} {pre post : List COp}
    {a : Local} {f i : Nat} (entry : Sound start s0) (path : Path c 0 s0 d s)
    (block : c.blocks.getD d [] = pre ++ .live a f i :: post) (steps : Steps (opsOf pre) s s1)
    (pinned : P f i = true) :
    ∃ b g, s1.refv a = some (b, g) ∧ s1.gen b = g ∧ s1.cls b ≠ .list := by
  rcases entry_block v entry with ⟨e0, h0, s0ok⟩
  rcases path_entry v h0 s0ok path with ⟨ed, hd, sd⟩
  rcases block_run (blockOk_of_verify v d) hd with ⟨out, hrun⟩
  rw [block] at hrun
  exact runC_live hrun sd steps pinned

/-! ## 関数の始まり

関数の始まりの事実 start は、Pinned の引数に Pinned、own の引数（取り込みでないもの）に Own。
呼ぶ側で `verify_live` が成り立っていれば、呼ばれた側から見て Pinned の引数の先は「呼び出し元の区画」
（`caller`。OwnershipFrame の `callee_preserves_caller` により、呼ばれた関数の中では変わらない）で、
世代は等しい。own の引数は、ムーブで受け取った、呼ばれた関数の変数が持つ区画。 -/

def startFacts (pinned : Nat → Bool) (ownParam : Nat → Bool) (params : Nat) : Facts := fun z =>
  if z < params then
    if pinned z then some .pinned else if ownParam z then some .own else none
  else none

theorem start_sound {pinned ownParam : Nat → Bool} {params : Nat} {s : St}
    (pin : ∀ z, z < params → pinned z = true → LiveRef s z .caller)
    (own : ∀ z, z < params → pinned z = false → ownParam z = true →
      ∃ b, s.root z = some b ∧ s.cls b = .root z) :
    Sound (startFacts pinned ownParam params) s where
  own y h := by
    unfold startFacts at h
    by_cases lt : y < params
    · by_cases p : pinned y = true
      · simp [lt, p] at h
      · by_cases o : ownParam y = true
        · exact own y lt (by simpa using p) o
        · simp [lt, p, o] at h
    · simp [lt] at h
  refTo r y h _ := by
    unfold startFacts at h
    by_cases lt : r < params <;> by_cases p : pinned r = true <;> by_cases o : ownParam r = true <;>
      simp [lt, p, o] at h
  refSlot r y h := by
    unfold startFacts at h
    by_cases lt : r < params <;> by_cases p : pinned r = true <;> by_cases o : ownParam r = true <;>
      simp [lt, p, o] at h
  refElem r y h := by
    unfold startFacts at h
    by_cases lt : r < params <;> by_cases p : pinned r = true <;> by_cases o : ownParam r = true <;>
      simp [lt, p, o] at h
  pinned r h := by
    unfold startFacts at h
    by_cases lt : r < params
    · by_cases p : pinned r = true
      · exact pin r lt p
      · by_cases o : ownParam r = true <;> simp [lt, p, o] at h
    · simp [lt] at h
  noSelfTo r h := by
    unfold startFacts at h
    by_cases lt : r < params <;> by_cases p : pinned r = true <;> by_cases o : ownParam r = true <;>
      simp [lt, p, o] at h
  noSelfSlot r h := by
    unfold startFacts at h
    by_cases lt : r < params <;> by_cases p : pinned r = true <;> by_cases o : ownParam r = true <;>
      simp [lt, p, o] at h
  noSelfElem r h := by
    unfold startFacts at h
    by_cases lt : r < params <;> by_cases p : pinned r = true <;> by_cases o : ownParam r = true <;>
      simp [lt, p, o] at h

end JabberwockFormal.ElisionCheck
