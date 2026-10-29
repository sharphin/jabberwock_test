set_option autoImplicit false
set_option warningAsError true

namespace JabberwockFormal.ElisionSoundness

/-!
# 検査を消す判断（`src/ir/analyze.rs`）の正しさ

解析が「この `*r` は世代の検査を消してよい」と判断したら、その時点で `r` の ref は本当に生きている
（借りたときの世代と、区画の今の世代が等しい）ことを証明する。

## 模型
- 区画には世代 `gen` と「種類」`cls` がある。種類は、その区画を解放・使い回しできるのは誰かを表す
  （own の変数 `x` の先 `root x`・変数 `x` そのものの場所 `slot x`・リストの中身 `list`・
  値の中の own の先 `nested`・呼び出し元の区画 `caller`・空き `free`）。
- 命令は、触ってよい種類の区画だけ世代と種類を変えられる（`Touch`）。これが言語の決まり
  （唯一の持ち主・呼ばれた関数は呼び出し元の変数に触れない・リストの要素を動かす操作の一覧）を
  模型にしたもので、この証明の前提になる。前提の一覧は docs/elision-proof.md。
- 解析の規則（`killAll` など）は analyze.rs の関数を一つずつ写したもの。
-/

abbrev Local := Nat
abbrev Block := Nat

/-- analyze.rs の `Fact` -/
inductive Fact where
  | own
  | refTo (x : Local)
  | refSlot (x : Local)
  | pinned
  | refElem (x : Local)
  deriving DecidableEq, Repr

abbrev Facts := Local → Option Fact

/-- 区画の種類（誰が解放・使い回しできるか） -/
inductive Cls where
  | root (x : Local)
  | slot (x : Local)
  | list
  | nested
  | caller
  | free
  deriving DecidableEq, Repr

/-- 実行の状態。`root x` は own の変数 `x` が持つ区画、`refv x` は ref の変数 `x` が持つ (区画, 借りたときの世代) -/
structure St where
  gen : Block → Nat
  cls : Block → Cls
  root : Local → Option Block
  refv : Local → Option (Block × Nat)

/-! ## 解析の規則（analyze.rs の写し） -/

/-- `kill_all` が消す事実（x を借りた ref） -/
def mentions : Fact → Local → Bool
  | .refTo y, x => y == x
  | .refSlot y, x => y == x
  | .refElem y, x => y == x
  | _, _ => false

/-- `kill_value` が消す事実（x の値を借りた ref。x の場所を借りた ref は残る） -/
def mentionsValue : Fact → Local → Bool
  | .refTo y, x => y == x
  | .refElem y, x => y == x
  | _, _ => false

def isElem : Fact → Bool
  | .refElem _ => true
  | _ => false

/-- `kill_all`: x がなくなった（drop・ムーブ） -/
def killAll (F : Facts) (x : Local) : Facts := fun z =>
  if z = x then none else
  match F z with
  | some f => if mentions f x then none else some f
  | none => none

/-- `kill_value`: x に入れ直した -/
def killValue (F : Facts) (x : Local) : Facts := fun z =>
  if z = x then none else
  match F z with
  | some f => if mentionsValue f x then none else some f
  | none => none

/-- `kill_elems`: リストの要素が動いたかもしれない -/
def killElems (F : Facts) : Facts := fun z =>
  match F z with
  | some f => if isElem f then none else some f
  | none => none

def killElemsIf (holds : Bool) (F : Facts) : Facts := if holds then killElems F else F

/-- `is_live` -/
def isLive (F : Facts) : Option Fact → Bool
  | some .pinned => true
  | some (.refSlot _) => true
  | some (.refTo x) => F x == some .own
  | _ => false

/-- `place` の判断: 先頭の `*` の検査を消してよいか -/
def safeDeref (F : Facts) (f : Option Fact) : Bool :=
  f == some .own || (match f with | some (.refElem _) => true | _ => false) || isLive F f

/-- `Inst::Assign` の最後: x の事実を f にする（自分自身を借りた事実は入れない。
    analyze.rs は RefElem(x) を x に入れることがあるが、ここでは入れない＝より慎重） -/
def setFact (F : Facts) (x : Local) (f : Option Fact) : Facts :=
  match f with
  | some (.refTo y) => if y = x then F else fun z => if z = x then f else F z
  | some (.refSlot y) => if y = x then F else fun z => if z = x then f else F z
  | some (.refElem y) => if y = x then F else fun z => if z = x then f else F z
  | some g => fun z => if z = x then some g else F z
  | none => F

/-! ## 命令 -/

/-- 解析から見た命令の形。analyze.rs のどの場合にあたるかはコメントに書く -/
inductive Op where
  /-- `Inst::Drop`・`Inst::Discard` のムーブ・スコープの終わり。`holds` は `may_hold_list` -/
  | drop (x : Local) (holds : Bool)
  /-- `Operand::Move` / `Operand::Read`（値はほかへ渡る。解放はしない） -/
  | moveOut (x : Local)
  /-- own の変数への代入（`Rvalue::Own` や own のムーブ。元の値は先の命令で読み終わっている） -/
  | assignOwn (x : Local) (holds : Bool)
  /-- 事実の付かない代入（数・構造体・copy_out など） -/
  | assignNone (x : Local) (holds : Bool)
  /-- `x = copy s`（ref の写し） -/
  | copyRef (x s : Local)
  /-- `x = move s`（ref のムーブ） -/
  | moveRef (x s : Local)
  /-- `Rvalue::Ref`: own の変数 y の先（とその中のフィールド） -/
  | borrowOwn (r y : Local)
  /-- `Rvalue::Ref`: ref s の借り直し。`live` は `*s` を通る形（`is_live` を求める） -/
  | reborrow (r s : Local) (live : Bool)
  /-- `Rvalue::Ref`: 変数 y そのもの（とその中のフィールド） -/
  | borrowSlot (r y : Local)
  /-- `Rvalue::Ref`: own の変数 y の先から `*` を通らずに届く要素（その場の配列か、リストの中身） -/
  | borrowElemOwn (r y : Local)
  /-- `Rvalue::Ref`: ref y の先から `*` を通らずに届く要素（y の先と同じ区画か、リストの中身） -/
  | borrowElemRef (r y : Local)
  /-- `Rvalue::Ref`: リストの変数 y の要素（リストの中身） -/
  | borrowElemList (r y : Local)
  /-- 場所の中への代入（`place.proj` が空でない）。`holds` は `value_may_hold` -/
  | writeInside (holds : Bool)
  /-- 要素を動かしうる操作（ふつうの呼び出し・関数の値の呼び出し・リストの操作・入出力の組み込み） -/
  | elems
  /-- 何も解放しない操作（数の計算・`scalar_call`・print などの組み込み） -/
  | pure
  /-- `*r` を読む・書く（ここで検査を消すかを決める） -/
  | access (r : Local)

/-- 解析の規則（analyze.rs の `inst` / `rvalue` / `operand` の写し） -/
def transfer (F : Facts) : Op → Facts
  | .drop x holds => killAll (killElemsIf holds F) x
  | .moveOut x => killAll F x
  | .assignOwn x holds => setFact (killValue (killElemsIf holds F) x) x (some .own)
  | .assignNone x holds => killValue (killElemsIf holds F) x
  | .copyRef x s => setFact (killValue F x) x (F s)
  | .moveRef x s => setFact (killValue (killAll F s) x) x (F s)
  | .borrowOwn r y =>
      setFact (killValue F r) r (if F y = some .own then some (.refTo y) else none)
  | .reborrow r s live =>
      setFact (killValue F r) r
        (if F s = some .own then none else if live && !isLive F (F s) then none else F s)
  | .borrowSlot r y => setFact (killValue F r) r (some (.refSlot y))
  | .borrowElemOwn r y =>
      setFact (killValue F r) r (if F y = some .own then some (.refElem y) else none)
  | .borrowElemRef r y =>
      setFact (killValue F r) r (if F y = some .pinned then some (.refElem y) else none)
  | .borrowElemList r y => setFact (killValue F r) r (some (.refElem y))
  | .writeInside holds => killElemsIf holds F
  | .elems => killElems F
  | .pure => F
  | .access _ => F

/-! ## 命令の動き（言語の決まりを前提として書いたもの） -/

/-- 種類が P でない区画は、世代も種類も変わらない -/
def Touch (P : Cls → Prop) (s s' : St) : Prop :=
  ∀ b, ¬ P (s.cls b) → s'.gen b = s.gen b ∧ s'.cls b = s.cls b

/-- x 以外の変数の持つものは変わらない -/
def Others (x : Local) (s s' : St) : Prop :=
  ∀ z, z ≠ x → s'.root z = s.root z ∧ s'.refv z = s.refv z

def Same (s s' : St) : Prop := ∀ z, s'.root z = s.root z ∧ s'.refv z = s.refv z

/-- 命令ごとに、触ってよい区画と、変わってよい変数 -/
def Exec : Op → St → St → Prop
  | .drop x holds, s, s' =>
      Touch (fun c => c = .root x ∨ c = .slot x ∨ c = .nested ∨ (holds = true ∧ c = .list) ∨ c = .free) s s' ∧
      Others x s s'
  | .moveOut x, s, s' =>
      Touch (fun c => c = .root x ∨ c = .slot x ∨ c = .nested ∨ c = .free) s s' ∧ Others x s s'
  | .assignOwn x holds, s, s' =>
      Touch (fun c => c = .root x ∨ c = .nested ∨ (holds = true ∧ c = .list) ∨ c = .free) s s' ∧
      Others x s s' ∧ ∃ b, s'.root x = some b ∧ s'.cls b = .root x
  | .assignNone x holds, s, s' =>
      Touch (fun c => c = .root x ∨ c = .nested ∨ (holds = true ∧ c = .list) ∨ c = .free) s s' ∧
      Others x s s'
  | .copyRef x src, s, s' =>
      Touch (fun c => c = .free) s s' ∧ Others x s s' ∧ s'.refv x = s.refv src
  | .moveRef x src, s, s' =>
      Touch (fun c => c = .root src ∨ c = .slot src ∨ c = .free) s s' ∧
      (∀ z, z ≠ x → z ≠ src → s'.root z = s.root z ∧ s'.refv z = s.refv z) ∧
      s'.refv x = s.refv src
  | .borrowOwn r y, s, s' =>
      Touch (fun c => c = .free) s s' ∧ Others r s s' ∧
      (∀ b, s.root y = some b → s'.refv r = some (b, s.gen b))
  | .reborrow r src _, s, s' =>
      Touch (fun c => c = .free) s s' ∧ Others r s s' ∧ s'.refv r = s.refv src
  | .borrowSlot r y, s, s' =>
      Touch (fun c => c = .free) s s' ∧ Others r s s' ∧
      ∃ b, s.cls b = .slot y ∧ s'.refv r = some (b, s.gen b)
  | .borrowElemOwn r y, s, s' =>
      Touch (fun c => c = .free) s s' ∧ Others r s s' ∧
      ∃ b, (s.cls b = .list ∨ s.root y = some b) ∧ s'.refv r = some (b, s.gen b)
  | .borrowElemRef r y, s, s' =>
      Touch (fun c => c = .free) s s' ∧ Others r s s' ∧
      ∃ b, (s.cls b = .list ∨ ∃ g0, s.refv y = some (b, g0)) ∧ s'.refv r = some (b, s.gen b)
  | .borrowElemList r _, s, s' =>
      Touch (fun c => c = .free) s s' ∧ Others r s s' ∧
      ∃ b, s.cls b = .list ∧ s'.refv r = some (b, s.gen b)
  | .writeInside holds, s, s' =>
      Touch (fun c => c = .nested ∨ (holds = true ∧ c = .list) ∨ c = .free) s s' ∧ Same s s'
  | .elems, s, s' =>
      Touch (fun c => c = .nested ∨ c = .list ∨ c = .free) s s' ∧ Same s s'
  | .pure, s, s' =>
      Touch (fun c => c = .nested ∨ c = .free) s s' ∧ Same s s'
  | .access _, s, s' => s' = s

/-! ## 事実が正しいこと -/

/-- 生きている ref: 借りたときの世代が、区画の今の世代と等しい -/
def LiveRef (s : St) (r : Local) (c : Cls) : Prop :=
  ∃ b g, s.refv r = some (b, g) ∧ s.cls b = c ∧ s.gen b = g

/-- RefElem(y) の区画の種類: リストの中身か、own の y の先（その場の配列）か、呼び出し元の区画（Pinned の y の先） -/
def ElemCls (y : Local) (c : Cls) : Prop := c = .list ∨ c = .root y ∨ c = .caller

/-- 事実がすべて本当であること -/
structure Sound (F : Facts) (s : St) : Prop where
  own : ∀ y, F y = some .own → ∃ b, s.root y = some b ∧ s.cls b = .root y
  refTo : ∀ r y, F r = some (.refTo y) → F y = some .own →
    ∃ b g, s.refv r = some (b, g) ∧ s.root y = some b ∧ s.gen b = g
  refSlot : ∀ r y, F r = some (.refSlot y) → LiveRef s r (.slot y)
  refElem : ∀ r y, F r = some (.refElem y) →
    ∃ b g, s.refv r = some (b, g) ∧ s.gen b = g ∧ ElemCls y (s.cls b)
  pinned : ∀ r, F r = some .pinned → LiveRef s r .caller
  noSelfTo : ∀ r, F r ≠ some (.refTo r)
  noSelfSlot : ∀ r, F r ≠ some (.refSlot r)
  noSelfElem : ∀ r, F r ≠ some (.refElem r)

/-- 消した検査が必ず通ること: own なら持ち主の区画があり、ref なら世代が等しい -/
def AccessOk (F : Facts) (s : St) (r : Local) : Prop :=
  (F r = some .own ∧ ∃ b, s.root r = some b ∧ s.cls b = .root r) ∨
  (∃ b g, s.refv r = some (b, g) ∧ s.gen b = g)

/-- 主な定理 1: 事実が正しければ、解析が「消してよい」とした `*r` は必ず通る -/
theorem elided_access_is_valid {F : Facts} {s : St} {r : Local}
    (sound : Sound F s) (safe : safeDeref F (F r) = true) : AccessOk F s r := by
  unfold safeDeref at safe
  cases h : F r with
  | none => simp [h, isLive] at safe
  | some f =>
    cases f with
    | own => exact Or.inl ⟨h, sound.own r h⟩
    | refTo y =>
      have hy : F y = some .own := by simpa [h, isLive] using safe
      rcases sound.refTo r y h hy with ⟨b, g, hv, _, hg⟩
      exact Or.inr ⟨b, g, hv, hg⟩
    | refSlot y =>
      rcases sound.refSlot r y h with ⟨b, g, hv, _, hg⟩
      exact Or.inr ⟨b, g, hv, hg⟩
    | pinned =>
      rcases sound.pinned r h with ⟨b, g, hv, _, hg⟩
      exact Or.inr ⟨b, g, hv, hg⟩
    | refElem y =>
      rcases sound.refElem r y h with ⟨b, g, hv, hg, _⟩
      exact Or.inr ⟨b, g, hv, hg⟩

/-! ## 合流（if のあとなど）: 事実が少なくても正しい -/

/-- G の事実はすべて F にもある -/
def Sub (G F : Facts) : Prop := ∀ z f, G z = some f → F z = some f

theorem sound_of_sub {F G : Facts} {s : St} (sound : Sound F s) (sub : Sub G F) : Sound G s where
  own y h := sound.own y (sub _ _ h)
  refTo r y h hy := sound.refTo r y (sub _ _ h) (sub _ _ hy)
  refSlot r y h := sound.refSlot r y (sub _ _ h)
  refElem r y h := sound.refElem r y (sub _ _ h)
  pinned r h := sound.pinned r (sub _ _ h)
  noSelfTo r h := sound.noSelfTo r (sub _ _ h)
  noSelfSlot r h := sound.noSelfSlot r (sub _ _ h)
  noSelfElem r h := sound.noSelfElem r (sub _ _ h)

/-- analyze.rs の `meet`: 両方で同じ事実だけを残す -/
def meet (F G : Facts) : Facts := fun z => if F z = G z then F z else none

theorem meet_sub_left (F G : Facts) : Sub (meet F G) F := by
  intro z f h
  unfold meet at h
  by_cases e : F z = G z <;> simp_all

theorem meet_sub_right (F G : Facts) : Sub (meet F G) G := by
  intro z f h
  unfold meet at h
  by_cases e : F z = G z <;> simp_all

/-! ## 規則の性質 -/

theorem killAll_some {F : Facts} {x z : Local} {f : Fact} (h : killAll F x z = some f) :
    z ≠ x ∧ F z = some f ∧ mentions f x = false := by
  unfold killAll at h
  by_cases e : z = x
  · simp [e] at h
  · simp only [e, if_false] at h
    cases hz : F z with
    | none => simp [hz] at h
    | some g =>
      rw [hz] at h
      by_cases m : mentions g x = true
      · simp [m] at h
      · simp only [m] at h
        simp at h
        subst h
        exact ⟨e, rfl, by simpa using m⟩

theorem killValue_some {F : Facts} {x z : Local} {f : Fact} (h : killValue F x z = some f) :
    z ≠ x ∧ F z = some f ∧ mentionsValue f x = false := by
  unfold killValue at h
  by_cases e : z = x
  · simp [e] at h
  · simp only [e, if_false] at h
    cases hz : F z with
    | none => simp [hz] at h
    | some g =>
      rw [hz] at h
      by_cases m : mentionsValue g x = true
      · simp [m] at h
      · simp only [m] at h
        simp at h
        subst h
        exact ⟨e, rfl, by simpa using m⟩

theorem killValue_self (F : Facts) (x : Local) : killValue F x x = none := by
  simp [killValue]

theorem killValue_noRefTo (F : Facts) (x r : Local) : killValue F x r ≠ some (.refTo x) := by
  intro h
  have := (killValue_some h).2.2
  simp [mentionsValue] at this

theorem killElems_some {F : Facts} {z : Local} {f : Fact} (h : killElems F z = some f) :
    F z = some f ∧ isElem f = false := by
  unfold killElems at h
  cases hz : F z with
  | none => simp [hz] at h
  | some g =>
    rw [hz] at h
    by_cases m : isElem g = true
    · simp [m] at h
    · simp only [m] at h
      simp at h
      subst h
      exact ⟨rfl, by simpa using m⟩

theorem killElemsIf_some {holds : Bool} {F : Facts} {z : Local} {f : Fact}
    (h : killElemsIf holds F z = some f) : F z = some f ∧ (holds = true → isElem f = false) := by
  cases holds with
  | false => exact ⟨h, by simp⟩
  | true =>
    have := killElems_some h
    exact ⟨this.1, fun _ => this.2⟩

/-! ## 事実を減らし、区画の一部だけが変わったとき -/

/-- 残した事実が、変わってよい区画も変わってよい変数も指さなければ、正しいまま -/
theorem frame {F G : Facts} {s s' : St} {P : Cls → Prop}
    (sound : Sound F s) (sub : Sub G F) (touch : Touch P s s')
    (hOwn : ∀ y, G y = some .own → s'.root y = s.root y ∧ ¬ P (.root y))
    (hTo : ∀ r y, G r = some (.refTo y) → G y = some .own →
      s'.refv r = s.refv r ∧ s'.root y = s.root y ∧ ¬ P (.root y))
    (hSlot : ∀ r y, G r = some (.refSlot y) → s'.refv r = s.refv r ∧ ¬ P (.slot y))
    (hElem : ∀ r y, G r = some (.refElem y) →
      s'.refv r = s.refv r ∧ ¬ (P .list ∨ P (.root y) ∨ P .caller))
    (hPin : ∀ r, G r = some .pinned → s'.refv r = s.refv r ∧ ¬ P .caller) :
    Sound G s' where
  own y h := by
    rcases sound.own y (sub _ _ h) with ⟨b, hb, hc⟩
    rcases hOwn y h with ⟨hr, hp⟩
    exact ⟨b, hr ▸ hb, (touch b (hc ▸ hp)).2.trans hc⟩
  refTo r y h hy := by
    rcases sound.refTo r y (sub _ _ h) (sub _ _ hy) with ⟨b, g, hv, hb, hg⟩
    rcases sound.own y (sub _ _ hy) with ⟨b', hb', hc⟩
    have : b' = b := by rw [hb] at hb'; exact (Option.some.inj hb').symm
    subst this
    rcases hTo r y h hy with ⟨hv', hr, hp⟩
    exact ⟨b', g, hv' ▸ hv, hr ▸ hb, (touch b' (hc ▸ hp)).1.trans hg⟩
  refSlot r y h := by
    rcases sound.refSlot r y (sub _ _ h) with ⟨b, g, hv, hc, hg⟩
    rcases hSlot r y h with ⟨hv', hp⟩
    have t := touch b (hc ▸ hp)
    exact ⟨b, g, hv' ▸ hv, t.2.trans hc, t.1.trans hg⟩
  refElem r y h := by
    rcases sound.refElem r y (sub _ _ h) with ⟨b, g, hv, hg, hc⟩
    rcases hElem r y h with ⟨hv', hp⟩
    have np : ¬ P (s.cls b) := by
      rcases hc with e | e | e <;> rw [e] <;> intro q <;> exact hp (by simp [q])
    have t := touch b np
    exact ⟨b, g, hv' ▸ hv, t.1.trans hg, t.2 ▸ hc⟩
  pinned r h := by
    rcases sound.pinned r (sub _ _ h) with ⟨b, g, hv, hc, hg⟩
    rcases hPin r h with ⟨hv', hp⟩
    have t := touch b (hc ▸ hp)
    exact ⟨b, g, hv' ▸ hv, t.2.trans hc, t.1.trans hg⟩
  noSelfTo r h := sound.noSelfTo r (sub _ _ h)
  noSelfSlot r h := sound.noSelfSlot r (sub _ _ h)
  noSelfElem r h := sound.noSelfElem r (sub _ _ h)

/-! ## 変数 x に新しい事実を入れたとき -/

/-- x に入れる事実が、命令のあとの状態で本当であること -/
def NewOk (K : Facts) (s : St) (x : Local) : Option Fact → Prop
  | some .own => ∃ b, s.root x = some b ∧ s.cls b = .root x
  | some (.refTo y) => y ≠ x → K y = some .own →
      ∃ b g, s.refv x = some (b, g) ∧ s.root y = some b ∧ s.gen b = g
  | some (.refSlot y) => y ≠ x → LiveRef s x (.slot y)
  | some (.refElem y) => y ≠ x → ∃ b g, s.refv x = some (b, g) ∧ s.gen b = g ∧ ElemCls y (s.cls b)
  | some .pinned => LiveRef s x .caller
  | none => True

theorem setFact_other {K : Facts} {x z : Local} {f : Option Fact} (h : z ≠ x) :
    setFact K x f z = K z := by
  unfold setFact
  rcases f with _ | (_ | y | y | _ | y)
  · rfl
  · simp [h]
  · by_cases e : y = x <;> simp [e, h]
  · by_cases e : y = x <;> simp [e, h]
  · simp [h]
  · by_cases e : y = x <;> simp [e, h]

theorem setFact_self {K : Facts} {x : Local} {f : Option Fact} (hx : K x = none) :
    setFact K x f x = (match f with
      | some (.refTo y) => if y = x then none else f
      | some (.refSlot y) => if y = x then none else f
      | some (.refElem y) => if y = x then none else f
      | _ => f) := by
  unfold setFact
  rcases f with _ | (_ | y | y | _ | y)
  · simp [hx]
  · simp
  · by_cases e : y = x <;> simp [e, hx]
  · by_cases e : y = x <;> simp [e, hx]
  · simp
  · by_cases e : y = x <;> simp [e, hx]

theorem set_sound {K : Facts} {s : St} {x : Local} {f : Option Fact}
    (sound : Sound K s) (hx : K x = none) (noTo : ∀ r, K r ≠ some (.refTo x))
    (ok : NewOk K s x f) : Sound (setFact K x f) s := by
  -- x 以外では K と同じ。x では f（自分自身を借りた事実なら none）
  have other : ∀ z, z ≠ x → setFact K x f z = K z := fun z h => setFact_other h
  have self := setFact_self (f := f) hx
  constructor
  · intro y h
    by_cases e : y = x
    · subst e
      rw [self] at h
      rcases f with _ | (_ | y' | y' | _ | y') <;> simp at h
      · exact ok
      all_goals (by_cases e' : y' = y <;> simp [e'] at h)
    · exact sound.own y (by rwa [other y e] at h)
  · intro r y h hy
    by_cases ey : y = x
    · subst ey
      -- y = x が own なら f は own。K に refTo x はないので r = x だが、x の事実は own
      rw [self] at hy
      by_cases er : r = y
      · subst er
        rw [self] at h
        rcases f with _ | (_ | y' | y' | _ | y') <;> simp at hy <;> simp at h
      · rw [other r er] at h
        exact absurd h (noTo r)
    · rw [other y ey] at hy
      by_cases er : r = x
      · subst er
        rw [self] at h
        rcases f with _ | (_ | y' | y' | _ | y') <;> simp at h
        · by_cases e' : y' = r
          · simp [e'] at h
          · simp [e'] at h
            subst h
            exact ok e' hy
        all_goals (by_cases e' : y' = r <;> simp [e'] at h)
      · rw [other r er] at h
        exact sound.refTo r y h hy
  · intro r y h
    by_cases er : r = x
    · subst er
      rw [self] at h
      rcases f with _ | (_ | y' | y' | _ | y') <;> simp at h
      all_goals (by_cases e' : y' = r <;> simp [e'] at h)
      subst h
      exact ok e'
    · rw [other r er] at h
      exact sound.refSlot r y h
  · intro r y h
    by_cases er : r = x
    · subst er
      rw [self] at h
      rcases f with _ | (_ | y' | y' | _ | y') <;> simp at h
      all_goals (by_cases e' : y' = r <;> simp [e'] at h)
      subst h
      exact ok e'
    · rw [other r er] at h
      exact sound.refElem r y h
  · intro r h
    by_cases er : r = x
    · subst er
      rw [self] at h
      rcases f with _ | (_ | y' | y' | _ | y') <;> simp at h
      · exact ok
      all_goals (by_cases e' : y' = r <;> simp [e'] at h)
    · rw [other r er] at h
      exact sound.pinned r h
  · intro r h
    by_cases er : r = x
    · subst er
      rw [self] at h
      rcases f with _ | (_ | y' | y' | _ | y') <;> simp at h
      all_goals (by_cases e' : y' = r <;> simp [e'] at h)
    · rw [other r er] at h
      exact sound.noSelfTo r h
  · intro r h
    by_cases er : r = x
    · subst er
      rw [self] at h
      rcases f with _ | (_ | y' | y' | _ | y') <;> simp at h
      all_goals (by_cases e' : y' = r <;> simp [e'] at h)
    · rw [other r er] at h
      exact sound.noSelfSlot r h
  · intro r h
    by_cases er : r = x
    · subst er
      rw [self] at h
      rcases f with _ | (_ | y' | y' | _ | y') <;> simp at h
      all_goals (by_cases e' : y' = r <;> simp [e'] at h)
    · rw [other r er] at h
      exact sound.noSelfElem r h

/-! ## 命令ごとに事実が正しいまま -/

/-- 命令の前提（型の決まり）: ref の写し・ムーブの元は own ではない（own の値の受け渡しは moveOut と assignOwn）。
    `x = move x` は作られない -/
def Pre (F : Facts) : Op → Prop
  | .copyRef _ src => F src ≠ some .own
  | .moveRef x src => F src ≠ some .own ∧ x ≠ src
  | _ => True

theorem sub_killAll (F : Facts) (x : Local) : Sub (killAll F x) F :=
  fun _ _ h => (killAll_some h).2.1

theorem sub_killValue (F : Facts) (x : Local) : Sub (killValue F x) F :=
  fun _ _ h => (killValue_some h).2.1

theorem sub_killElemsIf (holds : Bool) (F : Facts) : Sub (killElemsIf holds F) F :=
  fun _ _ h => (killElemsIf_some h).1

theorem sub_trans {F G H : Facts} (a : Sub H G) (b : Sub G F) : Sub H F :=
  fun z f h => b z f (a z f h)

/-- 写した ref の事実は、写した先でも本当 -/
theorem copied_ok {F : Facts} {s s' : St} {x src : Local} {P : Cls → Prop}
    (sound : Sound F s) (touch : Touch P s s') (notOwn : F src ≠ some .own)
    (hv : s'.refv x = s.refv src)
    (hRoot : ∀ y, y ≠ x → F y = some .own → F src = some (.refTo y) →
      s'.root y = s.root y ∧ ¬ P (.root y))
    (hSlot : ∀ y, F src = some (.refSlot y) → y ≠ x → ¬ P (.slot y))
    (hElemRoot : ∀ y, F src = some (.refElem y) → ¬ P (.root y))
    (hList : ¬ P .list) (hCaller : ¬ P .caller) (K : Facts) (sub : Sub K F) :
    NewOk K s' x (F src) := by
  cases h : F src with
  | none => trivial
  | some f =>
    cases f with
    | own => exact absurd h notOwn
    | refTo y =>
      intro ne hy
      have hy' := sub _ _ hy
      rcases sound.refTo src y h hy' with ⟨b, g, hb, hr, hg⟩
      rcases sound.own y hy' with ⟨b', hr', hc⟩
      have : b' = b := by rw [hr] at hr'; exact (Option.some.inj hr').symm
      subst this
      rcases hRoot y ne hy' h with ⟨er, hp⟩
      exact ⟨b', g, hv.trans hb, er.trans hr, (touch b' (hc ▸ hp)).1.trans hg⟩
    | refSlot y =>
      intro ne
      rcases sound.refSlot src y h with ⟨b, g, hb, hc, hg⟩
      have t := touch b (hc ▸ hSlot y h ne)
      exact ⟨b, g, hv.trans hb, t.2.trans hc, t.1.trans hg⟩
    | refElem y =>
      intro _
      rcases sound.refElem src y h with ⟨b, g, hb, hg, hc⟩
      have np : ¬ P (s.cls b) := by
        rcases hc with e | e | e <;> rw [e]
        · exact hList
        · exact hElemRoot y h
        · exact hCaller
      have t := touch b np
      exact ⟨b, g, hv.trans hb, t.1.trans hg, t.2 ▸ hc⟩
    | pinned =>
      rcases sound.pinned src h with ⟨b, g, hb, hc, hg⟩
      have t := touch b (hc ▸ hCaller)
      exact ⟨b, g, hv.trans hb, t.2.trans hc, t.1.trans hg⟩

/-- 主な定理 2: 命令の前に事実が正しければ、解析の規則を通したあとの事実も、命令のあとの状態で正しい -/
theorem step_sound {F : Facts} {s s' : St} {op : Op}
    (sound : Sound F s) (pre : Pre F op) (exec : Exec op s s') : Sound (transfer F op) s' := by
  cases op with
  | drop x holds =>
    rcases exec with ⟨touch, others⟩
    refine frame sound (sub_trans (sub_killAll _ x) (sub_killElemsIf holds F)) touch ?_ ?_ ?_ ?_ ?_
    · intro y h
      rcases killAll_some h with ⟨ne, _, _⟩
      exact ⟨(others y ne).1, by simp [ne]⟩
    · intro r y h _
      rcases killAll_some h with ⟨ne, _, m⟩
      have ny : y ≠ x := by simpa [mentions] using m
      exact ⟨(others r ne).2, (others y ny).1, by simp [ny]⟩
    · intro r y h
      rcases killAll_some h with ⟨ne, _, m⟩
      have ny : y ≠ x := by simpa [mentions] using m
      exact ⟨(others r ne).2, by simp [ny]⟩
    · intro r y h
      rcases killAll_some h with ⟨ne, h2, m⟩
      have ny : y ≠ x := by simpa [mentions] using m
      have := (killElemsIf_some h2).2
      refine ⟨(others r ne).2, ?_⟩
      cases holds <;> simp_all [isElem]
    · intro r h
      rcases killAll_some h with ⟨ne, _, _⟩
      exact ⟨(others r ne).2, by simp⟩
  | moveOut x =>
    rcases exec with ⟨touch, others⟩
    refine frame sound (sub_killAll F x) touch ?_ ?_ ?_ ?_ ?_
    · intro y h
      rcases killAll_some h with ⟨ne, _, _⟩
      exact ⟨(others y ne).1, by simp [ne]⟩
    · intro r y h _
      rcases killAll_some h with ⟨ne, _, m⟩
      have ny : y ≠ x := by simpa [mentions] using m
      exact ⟨(others r ne).2, (others y ny).1, by simp [ny]⟩
    · intro r y h
      rcases killAll_some h with ⟨ne, _, m⟩
      have ny : y ≠ x := by simpa [mentions] using m
      exact ⟨(others r ne).2, by simp [ny]⟩
    · intro r y h
      rcases killAll_some h with ⟨ne, _, m⟩
      have ny : y ≠ x := by simpa [mentions] using m
      exact ⟨(others r ne).2, by simp [ny]⟩
    · intro r h
      rcases killAll_some h with ⟨ne, _, _⟩
      exact ⟨(others r ne).2, by simp⟩
  | assignOwn x holds =>
    rcases exec with ⟨touch, others, newRoot⟩
    have k : Sound (killValue (killElemsIf holds F) x) s' := by
      refine frame sound (sub_trans (sub_killValue _ x) (sub_killElemsIf holds F)) touch ?_ ?_ ?_ ?_ ?_
      · intro y h
        rcases killValue_some h with ⟨ne, _, _⟩
        exact ⟨(others y ne).1, by simp [ne]⟩
      · intro r y h _
        rcases killValue_some h with ⟨ne, _, m⟩
        have ny : y ≠ x := by simpa [mentionsValue] using m
        exact ⟨(others r ne).2, (others y ny).1, by simp [ny]⟩
      · intro r y h
        rcases killValue_some h with ⟨ne, _, _⟩
        exact ⟨(others r ne).2, by simp⟩
      · intro r y h
        rcases killValue_some h with ⟨ne, h2, m⟩
        have ny : y ≠ x := by simpa [mentionsValue] using m
        have := (killElemsIf_some h2).2
        refine ⟨(others r ne).2, ?_⟩
        cases holds <;> simp_all [isElem]
      · intro r h
        rcases killValue_some h with ⟨ne, _, _⟩
        exact ⟨(others r ne).2, by simp⟩
    exact set_sound (f := some .own) k (killValue_self _ x) (killValue_noRefTo _ x) newRoot
  | assignNone x holds =>
    rcases exec with ⟨touch, others⟩
    refine frame sound (sub_trans (sub_killValue _ x) (sub_killElemsIf holds F)) touch ?_ ?_ ?_ ?_ ?_
    · intro y h
      rcases killValue_some h with ⟨ne, _, _⟩
      exact ⟨(others y ne).1, by simp [ne]⟩
    · intro r y h _
      rcases killValue_some h with ⟨ne, _, m⟩
      have ny : y ≠ x := by simpa [mentionsValue] using m
      exact ⟨(others r ne).2, (others y ny).1, by simp [ny]⟩
    · intro r y h
      rcases killValue_some h with ⟨ne, _, _⟩
      exact ⟨(others r ne).2, by simp⟩
    · intro r y h
      rcases killValue_some h with ⟨ne, h2, m⟩
      have ny : y ≠ x := by simpa [mentionsValue] using m
      have := (killElemsIf_some h2).2
      refine ⟨(others r ne).2, ?_⟩
      cases holds <;> simp_all [isElem]
    · intro r h
      rcases killValue_some h with ⟨ne, _, _⟩
      exact ⟨(others r ne).2, by simp⟩
  | copyRef x src =>
    rcases exec with ⟨touch, others, hv⟩
    have k : Sound (killValue F x) s' := by
      refine frame sound (sub_killValue F x) touch ?_ ?_ ?_ ?_ ?_
      · intro y h
        rcases killValue_some h with ⟨ne, _, _⟩
        exact ⟨(others y ne).1, by simp⟩
      · intro r y h _
        rcases killValue_some h with ⟨ne, _, m⟩
        have ny : y ≠ x := by simpa [mentionsValue] using m
        exact ⟨(others r ne).2, (others y ny).1, by simp⟩
      · intro r y h
        rcases killValue_some h with ⟨ne, _, _⟩
        exact ⟨(others r ne).2, by simp⟩
      · intro r y h
        rcases killValue_some h with ⟨ne, _, _⟩
        exact ⟨(others r ne).2, by simp⟩
      · intro r h
        rcases killValue_some h with ⟨ne, _, _⟩
        exact ⟨(others r ne).2, by simp⟩
    refine set_sound k (killValue_self _ x) (killValue_noRefTo _ x)
      (copied_ok sound touch pre hv ?_ ?_ (fun _ _ => by simp) (by simp) (by simp) _ (sub_killValue F x))
    · intro y ne _ _
      exact ⟨(others y ne).1, by simp⟩
    · intro y _ _
      simp
  | moveRef x src =>
    rcases pre with ⟨notOwn, ne⟩
    rcases exec with ⟨touch, others, hv⟩
    have k : Sound (killValue (killAll F src) x) s' := by
      refine frame sound (sub_trans (sub_killValue _ x) (sub_killAll F src)) touch ?_ ?_ ?_ ?_ ?_
      · intro y h
        rcases killValue_some h with ⟨nx, h2, _⟩
        rcases killAll_some h2 with ⟨ns, _, _⟩
        exact ⟨(others y nx ns).1, by simp [ns]⟩
      · intro r y h hy
        rcases killValue_some h with ⟨nx, h2, m⟩
        rcases killAll_some h2 with ⟨ns, _, m2⟩
        have nyx : y ≠ x := by simpa [mentionsValue] using m
        have nys : y ≠ src := by simpa [mentions] using m2
        exact ⟨(others r nx ns).2, (others y nyx nys).1, by simp [nys]⟩
      · intro r y h
        rcases killValue_some h with ⟨nx, h2, _⟩
        rcases killAll_some h2 with ⟨ns, _, m2⟩
        have nys : y ≠ src := by simpa [mentions] using m2
        exact ⟨(others r nx ns).2, by simp [nys]⟩
      · intro r y h
        rcases killValue_some h with ⟨nx, h2, _⟩
        rcases killAll_some h2 with ⟨ns, _, m2⟩
        have nys : y ≠ src := by simpa [mentions] using m2
        exact ⟨(others r nx ns).2, by simp [nys]⟩
      · intro r h
        rcases killValue_some h with ⟨nx, h2, _⟩
        rcases killAll_some h2 with ⟨ns, _, _⟩
        exact ⟨(others r nx ns).2, by simp⟩
    refine set_sound k (killValue_self _ x) (killValue_noRefTo _ x)
      (copied_ok sound touch notOwn hv ?_ ?_ ?_ (by simp) (by simp) _
        (sub_trans (sub_killValue _ x) (sub_killAll F src)))
    · intro y nyx _ h
      have nys : y ≠ src := fun e => sound.noSelfTo src (e ▸ h)
      exact ⟨(others y nyx nys).1, by simp [nys]⟩
    · intro y h _
      have nys : y ≠ src := fun e => sound.noSelfSlot src (e ▸ h)
      simp [nys]
    · intro y h
      have nys : y ≠ src := fun e => sound.noSelfElem src (e ▸ h)
      simp [nys]
  | borrowOwn r y =>
    rcases exec with ⟨touch, others, hv⟩
    have k : Sound (killValue F r) s' := by
      refine frame sound (sub_killValue F r) touch ?_ ?_ ?_ ?_ ?_
      · intro z h
        rcases killValue_some h with ⟨ne, _, _⟩
        exact ⟨(others z ne).1, by simp⟩
      · intro z w h _
        rcases killValue_some h with ⟨ne, _, m⟩
        have nw : w ≠ r := by simpa [mentionsValue] using m
        exact ⟨(others z ne).2, (others w nw).1, by simp⟩
      · intro z w h
        rcases killValue_some h with ⟨ne, _, _⟩
        exact ⟨(others z ne).2, by simp⟩
      · intro z w h
        rcases killValue_some h with ⟨ne, _, _⟩
        exact ⟨(others z ne).2, by simp⟩
      · intro z h
        rcases killValue_some h with ⟨ne, _, _⟩
        exact ⟨(others z ne).2, by simp⟩
    refine set_sound k (killValue_self _ r) (killValue_noRefTo _ r) ?_
    by_cases hy : F y = some .own
    · simp only [hy, if_true]
      intro ny _
      rcases sound.own y hy with ⟨b, hb, hc⟩
      exact ⟨b, s.gen b, hv b hb, (others y ny).1.trans hb,
        (touch b (by simp [hc])).1⟩
    · simp only [hy, if_false]
      trivial
  | reborrow r src live =>
    rcases exec with ⟨touch, others, hv⟩
    have k : Sound (killValue F r) s' := by
      refine frame sound (sub_killValue F r) touch ?_ ?_ ?_ ?_ ?_
      · intro z h
        rcases killValue_some h with ⟨ne, _, _⟩
        exact ⟨(others z ne).1, by simp⟩
      · intro z w h _
        rcases killValue_some h with ⟨ne, _, m⟩
        have nw : w ≠ r := by simpa [mentionsValue] using m
        exact ⟨(others z ne).2, (others w nw).1, by simp⟩
      · intro z w h
        rcases killValue_some h with ⟨ne, _, _⟩
        exact ⟨(others z ne).2, by simp⟩
      · intro z w h
        rcases killValue_some h with ⟨ne, _, _⟩
        exact ⟨(others z ne).2, by simp⟩
      · intro z h
        rcases killValue_some h with ⟨ne, _, _⟩
        exact ⟨(others z ne).2, by simp⟩
    refine set_sound k (killValue_self _ r) (killValue_noRefTo _ r) ?_
    by_cases ho : F src = some .own
    · simp only [ho, if_true]
      trivial
    · simp only [ho, if_false]
      have ok := copied_ok sound touch ho hv (fun y ne _ _ => ⟨(others y ne).1, by simp⟩)
        (fun _ _ _ => by simp) (fun _ _ => by simp) (by simp) (by simp) (killValue F r) (sub_killValue F r)
      by_cases l : (live && !isLive F (F src)) = true
      · simp only [l, if_true]
        trivial
      · simp only [l]
        exact ok
  | borrowSlot r y =>
    rcases exec with ⟨touch, others, b, hc, hv⟩
    have k : Sound (killValue F r) s' := by
      refine frame sound (sub_killValue F r) touch ?_ ?_ ?_ ?_ ?_
      · intro z h
        rcases killValue_some h with ⟨ne, _, _⟩
        exact ⟨(others z ne).1, by simp⟩
      · intro z w h _
        rcases killValue_some h with ⟨ne, _, m⟩
        have nw : w ≠ r := by simpa [mentionsValue] using m
        exact ⟨(others z ne).2, (others w nw).1, by simp⟩
      · intro z w h
        rcases killValue_some h with ⟨ne, _, _⟩
        exact ⟨(others z ne).2, by simp⟩
      · intro z w h
        rcases killValue_some h with ⟨ne, _, _⟩
        exact ⟨(others z ne).2, by simp⟩
      · intro z h
        rcases killValue_some h with ⟨ne, _, _⟩
        exact ⟨(others z ne).2, by simp⟩
    refine set_sound (f := some (.refSlot y)) k (killValue_self _ r) (killValue_noRefTo _ r) ?_
    intro _
    have t := touch b (by simp [hc])
    exact ⟨b, s.gen b, hv, t.2.trans hc, t.1⟩
  | borrowElemOwn r y =>
    rcases exec with ⟨touch, others, b, hc, hv⟩
    have k : Sound (killValue F r) s' := by
      refine frame sound (sub_killValue F r) touch ?_ ?_ ?_ ?_ ?_
      · intro z h
        rcases killValue_some h with ⟨ne, _, _⟩
        exact ⟨(others z ne).1, by simp⟩
      · intro z w h _
        rcases killValue_some h with ⟨ne, _, m⟩
        have nw : w ≠ r := by simpa [mentionsValue] using m
        exact ⟨(others z ne).2, (others w nw).1, by simp⟩
      · intro z w h
        rcases killValue_some h with ⟨ne, _, _⟩
        exact ⟨(others z ne).2, by simp⟩
      · intro z w h
        rcases killValue_some h with ⟨ne, _, _⟩
        exact ⟨(others z ne).2, by simp⟩
      · intro z h
        rcases killValue_some h with ⟨ne, _, _⟩
        exact ⟨(others z ne).2, by simp⟩
    refine set_sound k (killValue_self _ r) (killValue_noRefTo _ r) ?_
    by_cases hy : F y = some .own
    · simp only [hy, if_true]
      intro _
      have cb : ElemCls y (s.cls b) := by
        rcases hc with e | e
        · exact Or.inl e
        · rcases sound.own y hy with ⟨b', hb', hc'⟩
          rw [e] at hb'
          cases hb'
          exact Or.inr (Or.inl hc')
      have t := touch b (by rcases cb with e | e | e <;> simp [e])
      exact ⟨b, s.gen b, hv, t.1, t.2 ▸ cb⟩
    · simp only [hy, if_false]
      trivial
  | borrowElemRef r y =>
    rcases exec with ⟨touch, others, b, hc, hv⟩
    have k : Sound (killValue F r) s' := by
      refine frame sound (sub_killValue F r) touch ?_ ?_ ?_ ?_ ?_
      · intro z h
        rcases killValue_some h with ⟨ne, _, _⟩
        exact ⟨(others z ne).1, by simp⟩
      · intro z w h _
        rcases killValue_some h with ⟨ne, _, m⟩
        have nw : w ≠ r := by simpa [mentionsValue] using m
        exact ⟨(others z ne).2, (others w nw).1, by simp⟩
      · intro z w h
        rcases killValue_some h with ⟨ne, _, _⟩
        exact ⟨(others z ne).2, by simp⟩
      · intro z w h
        rcases killValue_some h with ⟨ne, _, _⟩
        exact ⟨(others z ne).2, by simp⟩
      · intro z h
        rcases killValue_some h with ⟨ne, _, _⟩
        exact ⟨(others z ne).2, by simp⟩
    refine set_sound k (killValue_self _ r) (killValue_noRefTo _ r) ?_
    by_cases hy : F y = some .pinned
    · simp only [hy, if_true]
      intro _
      have cb : ElemCls y (s.cls b) := by
        rcases hc with e | ⟨g0, e⟩
        · exact Or.inl e
        · rcases sound.pinned y hy with ⟨b', g', hb', hc', _⟩
          rw [e] at hb'
          cases hb'
          exact Or.inr (Or.inr hc')
      have t := touch b (by rcases cb with e | e | e <;> simp [e])
      exact ⟨b, s.gen b, hv, t.1, t.2 ▸ cb⟩
    · simp only [hy, if_false]
      trivial
  | borrowElemList r y =>
    rcases exec with ⟨touch, others, b, hc, hv⟩
    have k : Sound (killValue F r) s' := by
      refine frame sound (sub_killValue F r) touch ?_ ?_ ?_ ?_ ?_
      · intro z h
        rcases killValue_some h with ⟨ne, _, _⟩
        exact ⟨(others z ne).1, by simp⟩
      · intro z w h _
        rcases killValue_some h with ⟨ne, _, m⟩
        have nw : w ≠ r := by simpa [mentionsValue] using m
        exact ⟨(others z ne).2, (others w nw).1, by simp⟩
      · intro z w h
        rcases killValue_some h with ⟨ne, _, _⟩
        exact ⟨(others z ne).2, by simp⟩
      · intro z w h
        rcases killValue_some h with ⟨ne, _, _⟩
        exact ⟨(others z ne).2, by simp⟩
      · intro z h
        rcases killValue_some h with ⟨ne, _, _⟩
        exact ⟨(others z ne).2, by simp⟩
    refine set_sound (f := some (.refElem y)) k (killValue_self _ r) (killValue_noRefTo _ r) ?_
    intro _
    have t := touch b (by simp [hc])
    exact ⟨b, s.gen b, hv, t.1, t.2 ▸ Or.inl hc⟩
  | writeInside holds =>
    rcases exec with ⟨touch, same⟩
    refine frame sound (sub_killElemsIf holds F) touch ?_ ?_ ?_ ?_ ?_
    · intro y _
      exact ⟨(same y).1, by simp⟩
    · intro r y _ _
      exact ⟨(same r).2, (same y).1, by simp⟩
    · intro r y _
      exact ⟨(same r).2, by simp⟩
    · intro r y h
      have := (killElemsIf_some h).2
      refine ⟨(same r).2, ?_⟩
      cases holds <;> simp_all [isElem]
    · intro r _
      exact ⟨(same r).2, by simp⟩
  | elems =>
    rcases exec with ⟨touch, same⟩
    refine frame sound (fun _ _ h => (killElems_some h).1) touch ?_ ?_ ?_ ?_ ?_
    · intro y _
      exact ⟨(same y).1, by simp⟩
    · intro r y _ _
      exact ⟨(same r).2, (same y).1, by simp⟩
    · intro r y _
      exact ⟨(same r).2, by simp⟩
    · intro r y h
      have := (killElems_some h).2
      simp [isElem] at this
    · intro r _
      exact ⟨(same r).2, by simp⟩
  | pure =>
    rcases exec with ⟨touch, same⟩
    refine frame sound (fun _ _ h => h) touch ?_ ?_ ?_ ?_ ?_
    · intro y _
      exact ⟨(same y).1, by simp⟩
    · intro r y _ _
      exact ⟨(same r).2, (same y).1, by simp⟩
    · intro r y _
      exact ⟨(same r).2, by simp⟩
    · intro r y _
      exact ⟨(same r).2, by simp⟩
    · intro r _
      exact ⟨(same r).2, by simp⟩
  | access r =>
    subst exec
    exact sound

/-! ## 命令の列（基本ブロック） -/

def transferAll (F : Facts) : List Op → Facts
  | [] => F
  | op :: ops => transferAll (transfer F op) ops

/-- 命令の列を実際に実行した（各命令の前提も満たす） -/
def Run : Facts → List Op → St → St → Prop
  | _, [], s, s' => s' = s
  | F, op :: ops, s, s'' => Pre F op ∧ ∃ s', Exec op s s' ∧ Run (transfer F op) ops s' s''

theorem run_sound {F : Facts} {ops : List Op} {s s' : St}
    (sound : Sound F s) (run : Run F ops s s') : Sound (transferAll F ops) s' := by
  induction ops generalizing F s with
  | nil => subst run; exact sound
  | cons op ops ih =>
    rcases run with ⟨pre, m, exec, rest⟩
    exact ih (step_sound sound pre exec) rest

theorem run_append {F : Facts} {a b : List Op} {s s'' : St} (run : Run F (a ++ b) s s'') :
    ∃ s', Run F a s s' ∧ Run (transferAll F a) b s' s'' := by
  induction a generalizing F s with
  | nil => exact ⟨s, rfl, run⟩
  | cons op ops ih =>
    rcases run with ⟨pre, m, exec, rest⟩
    rcases ih rest with ⟨s', r1, r2⟩
    exact ⟨s', ⟨pre, m, exec, r1⟩, r2⟩

/-- ブロックの途中の `*r`: そこまでの規則で「消してよい」なら、実行でも必ず通る -/
theorem elided_in_block {F : Facts} {before after : List Op} {r : Local} {s s'' : St}
    (sound : Sound F s) (run : Run F (before ++ Op.access r :: after) s s'')
    (safe : safeDeref (transferAll F before) (transferAll F before r) = true) :
    ∃ s', Run F before s s' ∧ AccessOk (transferAll F before) s' r := by
  rcases run_append run with ⟨s', r1, _⟩
  exact ⟨s', r1, elided_access_is_valid (run_sound sound r1) safe⟩

/-! ## 制御の流れ（関数の中） -/

/-- 関数の中身: ブロックごとの命令の列と、次に進みうるブロック -/
structure Cfg where
  code : Nat → List Op
  succ : Nat → List Nat

/-- 解析の結果（ブロックの入口の事実）が止まった状態: 進みうる辺のすべてで、行き先の事実は出口の事実に含まれる -/
def Stable (g : Cfg) (E : Nat → Facts) : Prop :=
  ∀ b c, c ∈ g.succ b → Sub (E c) (transferAll (E b) (g.code b))

/-- analyze.rs の繰り返しが止まったとき: どの辺でも `meet(entry[c], out) == entry[c]`。これは Stable を意味する -/
theorem stable_of_meet {g : Cfg} {E : Nat → Facts}
    (fixed : ∀ b c, c ∈ g.succ b → meet (E c) (transferAll (E b) (g.code b)) = E c) : Stable g E := by
  intro b c hc
  have := meet_sub_right (E c) (transferAll (E b) (g.code b))
  rwa [fixed b c hc] at this

/-- ブロック b の入口の状態 s から、ブロックをいくつか進んで、ブロック c の入口の状態 s' に来た -/
inductive Path (g : Cfg) (E : Nat → Facts) : Nat → St → Nat → St → Prop where
  | here (b : Nat) (s : St) : Path g E b s b s
  | step {b c d : Nat} {s s' s'' : St} :
      Run (E b) (g.code b) s s' → c ∈ g.succ b → Path g E c s' d s'' → Path g E b s d s''

/-- 主な定理 3: 入口で事実が正しく、解析が止まっていれば、どのブロックの入口でも事実は正しい -/
theorem path_sound {g : Cfg} {E : Nat → Facts} (stable : Stable g E)
    {b d : Nat} {s s' : St} (sound : Sound (E b) s) (path : Path g E b s d s') : Sound (E d) s' := by
  induction path with
  | here => exact sound
  | step run hc _ ih =>
    exact ih (sound_of_sub (run_sound sound run) (stable _ _ hc))

/-- まとめ: 関数の入口（ブロック 0）で事実が正しく、解析が止まっていれば、どこで消した `*r` も必ず通る -/
theorem elided_access_in_function {g : Cfg} {E : Nat → Facts} (stable : Stable g E)
    {d : Nat} {s0 s s'' : St} {before after : List Op} {r : Local}
    (entry : Sound (E 0) s0) (path : Path g E 0 s0 d s)
    (block : g.code d = before ++ Op.access r :: after)
    (run : Run (E d) (g.code d) s s'')
    (safe : safeDeref (transferAll (E d) before) (transferAll (E d) before r) = true) :
    ∃ s', Run (E d) before s s' ∧ AccessOk (transferAll (E d) before) s' r := by
  rw [block] at run
  exact elided_in_block (path_sound stable entry path) run safe

/-! ## 呼び出しの引数

`operands` は、読んだ引数の値を名前のない変数（`base + i`）に置いたものとして事実を残し、
すべての引数を読み終えてから `is_live` を見る。後の引数のムーブ（`moveOut`）は `killAll` で
先の引数の事実も消す。したがって、呼び出しの直前に `is_live` が真なら、その引数の ref は生きている
（呼ばれた関数はそれを Pinned として扱ってよい。リストの要素は呼ばれた関数の中で守れないので含めない）。 -/

theorem live_arg_is_valid {F : Facts} {s : St} {a : Local}
    (sound : Sound F s) (live : isLive F (F a) = true) :
    ∃ b g, s.refv a = some (b, g) ∧ s.gen b = g ∧ s.cls b ≠ .list := by
  cases h : F a with
  | none => simp [h, isLive] at live
  | some f =>
    cases f with
    | own => simp [h, isLive] at live
    | refElem _ => simp [h, isLive] at live
    | refTo y =>
      have hy : F y = some .own := by simpa [h, isLive] using live
      rcases sound.refTo a y h hy with ⟨b, g, hv, hb, hg⟩
      rcases sound.own y hy with ⟨b', hb', hc⟩
      have : b' = b := by rw [hb] at hb'; exact (Option.some.inj hb').symm
      subst this
      exact ⟨b', g, hv, hg, by simp [hc]⟩
    | refSlot y =>
      rcases sound.refSlot a y h with ⟨b, g, hv, hc, hg⟩
      exact ⟨b, g, hv, hg, by simp [hc]⟩
    | pinned =>
      rcases sound.pinned a h with ⟨b, g, hv, hc, hg⟩
      exact ⟨b, g, hv, hg, by simp [hc]⟩

/-- 以前のやり方（読む前の事実を取っておき、読み終えた後の事実で `is_live` を見る）の反例。
    `f(r, y)` で r は y を借りた ref（RefSlot y）、y はムーブされる。
    取っておいた事実では「生きている」と判定されるが、y のムーブで y の場所の世代は進んでいる -/
theorem old_argument_check_is_unsound :
    ∃ (F : Facts) (s s' : St) (r y : Local),
      Sound F s ∧ Exec (.moveOut y) s s' ∧
      isLive (transfer F (.moveOut y)) (F r) = true ∧
      ¬ (∃ b g, s'.refv r = some (b, g) ∧ s'.gen b = g) ∧
      transfer F (.moveOut y) r = none := by
  let F : Facts := fun z => if z = 0 then some (.refSlot 1) else none
  let s : St := { gen := fun _ => 0, cls := fun _ => .slot 1, root := fun _ => none,
                  refv := fun z => if z = 0 then some (0, 0) else none }
  let s' : St := { s with gen := fun _ => 1 }
  refine ⟨F, s, s', 0, 1, ?_, ?_, ?_, ?_, ?_⟩
  · constructor
    · intro y h; by_cases e : y = 0 <;> simp [F, e] at h
    · intro r y h; by_cases e : r = 0 <;> simp [F, e] at h
    · intro r y h
      by_cases e : r = 0
      · subst e
        simp [F] at h
        subst h
        exact ⟨0, 0, by simp [s], rfl, rfl⟩
      · simp [F, e] at h
    · intro r y h; by_cases e : r = 0 <;> simp [F, e] at h
    · intro r h; by_cases e : r = 0 <;> simp [F, e] at h
    · intro r h; by_cases e : r = 0 <;> simp [F, e] at h
    · intro r h
      by_cases e : r = 0
      · subst e; simp [F] at h
      · simp [F, e] at h
    · intro r h; by_cases e : r = 0 <;> simp [F, e] at h
  · refine ⟨fun b _ => by simp [s] at *, fun z _ => ⟨rfl, rfl⟩⟩
  · simp [F, isLive]
  · rintro ⟨b, g, hv, hg⟩
    simp [s', s] at hv hg
    omega
  · simp [transfer, killAll, F, mentions]

end JabberwockFormal.ElisionSoundness
