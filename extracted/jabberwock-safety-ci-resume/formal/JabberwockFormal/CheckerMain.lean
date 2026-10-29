import Lean.Data.Json
import JabberwockFormal.ElisionCheckFast

set_option autoImplicit false
set_option warningAsError true

/-!
# 検査器の実行ファイル（`lake exe elision-check 入力.json ...`）

`jwc --ir-cert` の JSON を読み、関数ごとに
1. IR を模型の操作に読み替える（`Map`。ここが証明の外に残る「読み替え表」。analyze.rs の場合分けと 1 対 1 に書く）
2. ブロックの入口の事実と Pinned の表を自分で計算する（`facts` / `pinTable`。結果は下で確かめ直すので、正しさは要らない）
3. `verifyFunc` で確かめる（ここは `ElisionCheck` で証明済み）
を行う。どれかの関数が通らなければ、終了コード 1 で理由を出す。
-/

open Lean (Json)
open JabberwockFormal.ElisionSoundness
open JabberwockFormal.ElisionCheck
open JabberwockFormal.ElisionCheckFast

namespace JabberwockFormal.CheckerMain

/-! ## IR（jwc/ircert.jw の形） -/

mutual
inductive Operand where
  | copy (l : Nat) (proj : List Proj)
  | move (l : Nat)
  | read (l : Nat)
  | const (i : Option Nat)
inductive Proj where
  | deref (req : Bool)
  | field
  | variant
  | index (o : Operand)
end

inductive Rvalue where
  | use (o : Operand)
  | scalar (os : List Operand)
  | ops (os : List Operand)
  | ref (l : Nat) (proj : List Proj) (follows : Bool)
  | own (o : Operand)
  | agg (os : List Operand)
  | take (l : Nat) (proj : List Proj)
  | readPlace (l : Nat) (proj : List Proj)
  | copyOut (l : Nat) (proj : List Proj)
  | seq (name : String) (l : Nat) (proj : List Proj) (os : List Operand)
  | call (f : Nat) (os : List Operand)
  | dyn (fns : List Nat) (l : Nat) (proj : List Proj) (os : List Operand)
  | closure (f : Nat) (os : List Operand)
  | callValue (fv : Operand) (os : List Operand)
  | builtin (name : String) (os : List Operand)

inductive Inst where
  | assign (l : Nat) (proj : List Proj) (v : Rvalue)
  | drop (l : Nat)
  | discard (o : Operand)

inductive Term where
  | goto (succ : List Nat)
  | operand (o : Operand) (succ : List Nat)
  | switch (l : Nat) (proj : List Proj) (succ : List Nat)

def Term.succ : Term → List Nat
  | .goto s => s
  | .operand _ s => s
  | .switch _ _ s => s

structure Func where
  name : String
  params : Nat
  captures : Nat
  ret : String
  locals : Array String
  blocks : Array (List Inst × Term)

instance : Inhabited Func := ⟨{ name := "", params := 0, captures := 0, ret := "", locals := #[], blocks := #[] }⟩
instance : Inhabited Code := ⟨{ blocks := #[], succ := #[] }⟩

/-! ## JSON を読む -/

def field (j : Json) (k : String) : Except String Json := j.getObjVal? k
def nat (j : Json) (k : String) : Except String Nat := do (← field j k).getNat?
def str (j : Json) (k : String) : Except String String := do (← field j k).getStr?
def bool (j : Json) (k : String) : Except String Bool := do (← field j k).getBool?
def arr (j : Json) (k : String) : Except String (Array Json) := do (← field j k).getArr?

mutual
partial def readOperand (j : Json) : Except String Operand := do
  match ← str j "k" with
  | "copy" => let p ← field j "p"; return .copy (← nat p "l") (← readProjs p)
  | "move" => return .move (← nat j "l")
  | "read" => return .read (← nat j "l")
  | "const" => return .const ((nat j "i").toOption)
  | k => throw s!"知らない値の形: {k}"
partial def readProjs (p : Json) : Except String (List Proj) := do
  let ps ← arr p "p"
  ps.toList.mapM fun q => do
    match ← str q "k" with
    | "deref" => return .deref (← bool q "req")
    | "field" => return .field
    | "variant" => return .variant
    | "index" => return .index (← readOperand (← field q "o"))
    | k => throw s!"知らない場所の形: {k}"
end

def readOps (j : Json) : Except String (List Operand) := do
  (← arr j "os").toList.mapM readOperand

def readPlace (j : Json) (k : String) : Except String (Nat × List Proj) := do
  let p ← field j k
  return (← nat p "l", ← readProjs p)

def readRvalue (j : Json) : Except String Rvalue := do
  match ← str j "k" with
  | "use" => return .use (← readOperand (← field j "o"))
  | "scalar" => return .scalar (← readOps j)
  | "ops" => return .ops (← readOps j)
  | "ref" => let (l, p) ← readPlace j "p"; return .ref l p (← bool j "follows")
  | "own" => return .own (← readOperand (← field j "o"))
  | "agg" => return .agg (← readOps j)
  | "take" => let (l, p) ← readPlace j "p"; return .take l p
  | "read_place" => let (l, p) ← readPlace j "p"; return .readPlace l p
  | "copy_out" => let (l, p) ← readPlace j "p"; return .copyOut l p
  | "seq" => let (l, p) ← readPlace j "p"; return .seq (← str j "name") l p (← readOps j)
  | "call" => return .call (← nat j "f") (← readOps j)
  | "dyn" =>
    let (l, p) ← readPlace j "this"
    let fns ← (← arr j "fns").toList.mapM (·.getNat?)
    return .dyn fns l p (← readOps j)
  | "closure" => return .closure (← nat j "f") (← readOps j)
  | "callvalue" => return .callValue (← readOperand (← field j "fv")) (← readOps j)
  | "builtin" => return .builtin (← str j "name") (← readOps j)
  | k => throw s!"知らない右辺の形: {k}"

def readInst (j : Json) : Except String Inst := do
  match ← str j "k" with
  | "assign" => let (l, p) ← readPlace j "p"; return .assign l p (← readRvalue (← field j "v"))
  | "drop" => return .drop (← nat j "l")
  | "discard" => return .discard (← readOperand (← field j "o"))
  | k => throw s!"知らない命令の形: {k}"

def readTerm (j : Json) : Except String Term := do
  let succ ← (← arr j "succ").toList.mapM (·.getNat?)
  match ← str j "k" with
  | "goto" => return .goto succ
  | "operand" => return .operand (← readOperand (← field j "o")) succ
  | "switch" => let (l, p) ← readPlace j "p"; return .switch l p succ
  | k => throw s!"知らない終わりの形: {k}"

def readFunc (j : Json) : Except String Func := do
  let locals ← (← arr j "locals").mapM (·.getStr?)
  let blocks ← (← arr j "blocks").mapM fun b => do
    let insts ← (← arr b "insts").toList.mapM readInst
    return (insts, ← readTerm (← field b "term"))
  return { name := ← str j "name", params := ← nat j "params", captures := ← nat j "captures",
           ret := ← str j "ret", locals, blocks }

def readProgram (text : String) : Except String (Array Func) := do
  let j ← Json.parse text
  (← arr j "fns").mapM readFunc

/-! ## 読み替え表（IR → 模型の操作）

analyze.rs のどの場合にあたるかをコメントに書く。分からない形は `bad`（検査器は通さない）。
読み替えは analyze.rs より慎重でよい（そのときは、analyze.rs が消した検査を確かめられずに落ちる）。 -/

def scalarTags : List String := ["Int", "Float", "Bool", "Str", "Unit", "Never"]

/-- `src/ir/analyze.rs` の `pure_builtin` と同じ一覧 -/
def pureBuiltins : List String :=
  ["print", "eprint", "concat", "append", "assert",
   "int.abs", "int.max", "int.min",
   "f64.abs", "f64.max", "f64.min", "f64.sqrt", "f64.to_bits",
   "str.len", "str.byte_len", "str.chars", "str.bytes", "str.code", "str.lines", "str.split",
   "str.trim", "str.to_upper", "str.to_lower", "str.contains", "str.starts_with", "str.ends_with",
   "str.find", "str.slice", "str.replace", "str.repeat", "str.to_int", "str.to_float"]

/-- 要素を動かさないリストの操作（analyze.rs の `SeqOp`） -/
def readOnlySeq : List String := ["len", "contains", "get", "keys"]

structure Ctx where
  fns : Array Func
  f : Func
  /-- 引数を置く名前のない変数の始まり（変数の数） -/
  base : Nat

namespace Ctx
def tag (c : Ctx) (l : Nat) : String := c.f.locals.getD l "?"
def isOwn (c : Ctx) (l : Nat) : Bool := c.tag l == "Own"
def isRef (c : Ctx) (l : Nat) : Bool := c.tag l == "Ref"
def isList (c : Ctx) (l : Nat) : Bool := c.tag l == "List"
/-- `may_hold_list` -/
def mayHold (c : Ctx) (l : Nat) : Bool := !(scalarTags.contains (c.tag l) || c.tag l == "Ref")
/-- `scalar_call` -/
def scalarFn (c : Ctx) (g : Nat) : Bool :=
  match c.fns[g]? with
  | none => false
  | some h => h.captures == 0 && scalarTags.contains h.ret &&
      (List.range h.params).all (fun i => scalarTags.contains (h.locals.getD i "?"))
def arity (c : Ctx) (g : Nat) : Nat := (c.fns[g]?.map (·.params)).getD 0
end Ctx

def isFieldOnly : List Proj → Bool
  | [] => true
  | .field :: r => isFieldOnly r
  | _ => false

def isElemPath : List Proj → Bool
  | [] => true
  | .field :: r => isElemPath r
  | .index _ :: r => isElemPath r
  | _ => false

mutual
/-- `place`: 先頭の `*` の判断（消されていれば elided）、2 段目以降は消されていてはならない、添字の値を読む -/
partial def placeOps (c : Ctx) (l : Nat) (proj : List Proj) : List COp :=
  let first := match proj with
    | .deref false :: _ => [COp.elided l]
    | _ => []
  let later := if (proj.drop 1).any (fun | .deref false => true | _ => false)
    then [COp.bad "2 段目以降の * の検査が消されている"] else []
  let idx := proj.flatMap (fun | .index o => operandOps c o | _ => [])
  first ++ later ++ idx

/-- `operand` -/
partial def operandOps (c : Ctx) : Operand → List COp
  | .copy l proj =>
    (if proj.isEmpty && c.isOwn l then [COp.bad s!"own の変数 _{l} の copy"] else []) ++ placeOps c l proj
  | .move l => [.op (.moveOut l)]
  | .read l => [.op (.moveOut l)]
  | .const _ => []
end

def operandsOps (c : Ctx) (os : List Operand) : List COp := os.flatMap (operandOps c)

/-- 表示・計算のために読むだけの値（`show!`・数の計算）。own の変数を copy で読んでも持ち主は増えない -/
def viewOps (c : Ctx) : Operand → List COp
  | .copy l proj => placeOps c l proj
  | o => operandOps c o

/-- `ref_fact` の場合分け: ref を x に入れる操作 -/
def refOp (c : Ctx) (x l : Nat) (proj : List Proj) (follows : Bool) : COp :=
  match proj, follows with
  | [], true =>
    if c.isOwn l then .op (.borrowOwn x l)
    else if c.isRef l then .op (.reborrow x l false)
    else .op (.assignNone x false)
  | [], false =>
    if c.isOwn l then .bad s!"own の変数 _{l} そのものを指す ref"
    else .op (.borrowSlot x l)
  | .deref _ :: rest, false =>
    if isFieldOnly rest then
      (if c.isOwn l then .op (.borrowOwn x l)
       else if c.isRef l then .op (.reborrow x l true)
       else .op (.assignNone x false))
    else if isElemPath rest then
      (if c.isOwn l then .op (.borrowElemOwn x l)
       else if c.isRef l then .op (.borrowElemRef x l)
       else .op (.assignNone x false))
    else .op (.assignNone x false)
  | .field :: _, false =>
    if isFieldOnly proj && !(c.isOwn l || c.isRef l) then .op (.borrowSlot x l)
    else .op (.assignNone x false)
  | .index _ :: rest, false =>
    if isElemPath rest && c.isList l then .op (.borrowElemList x l)
    else .op (.assignNone x false)
  | _, _ => .op (.assignNone x false)

/-- 呼び出しの引数: 読んだ値を名前のない変数 t に置く（analyze.rs の `operands`）。
    返すのは、読む操作と、t が生きた ref でありうるか -/
def argOps (c : Ctx) (t : Nat) : Operand → List COp × Bool
  | .copy l [] =>
    if c.isOwn l then ([.bad s!"own の変数 _{l} の copy"], false) else ([.op (.copyRef t l)], true)
  | .copy l proj => (placeOps c l proj, false)
  | .move l => if c.isOwn l then ([.op (.moveOut l)], false) else ([.op (.moveRef t l)], true)
  | .read l => if c.isOwn l then ([.op (.moveOut l)], false) else ([.op (.moveRef t l)], true)
  | .const _ => ([], false)

/-- 呼び出す関数 g の off 番目からの引数に、置いた値を渡す（Pinned なら生きた ref であること） -/
def argChecks (c : Ctx) (g off : Nat) (cands : List Bool) : List COp :=
  (cands.zipIdx).map fun (ok, i) =>
    if ok then .live (c.base + off + i) g (off + i) else .notLive g (off + i)

def cleanup (c : Ctx) (off : Nat) (cands : List Bool) : List COp :=
  (cands.zipIdx).filterMap fun (ok, i) => if ok then some (.op (.moveOut (c.base + off + i))) else none

/-- 関数 g を値として使った・スレッドの入口にした: Pinned の引数を持ってはならない -/
def escapeOps (c : Ctx) (g : Nat) : List COp :=
  (List.range (c.arity g)).map fun i => .notLive g i

def argsOf (c : Ctx) (off : Nat) (os : List Operand) : List COp × List Bool :=
  let ps := os.zipIdx.map fun (o, i) => argOps c (c.base + off + i) o
  (ps.flatMap (·.1), ps.map (·.2))

/-- `value_may_hold` -/
def valueHolds (c : Ctx) : Rvalue → Bool
  | .scalar _ | .ref .. => false
  | .use (.const _) => false
  | .use (.move l) | .use (.read l) => c.mayHold l
  | .use (.copy l []) => c.mayHold l
  | _ => true

/-- 右辺を計算する操作（`rvalue`。値の事実は代入の方で扱う） -/
def valueOps (c : Ctx) : Rvalue → List COp
  | .use o => operandOps c o
  | .scalar os => os.flatMap (viewOps c)
  | .ops os | .agg os => operandsOps c os
  | .ref l proj _ => placeOps c l proj
  | .own o => operandOps c o
  | .take l proj => placeOps c l proj ++ (if proj.isEmpty then [.op (.moveOut l)] else [])
  | .readPlace l proj | .copyOut l proj => placeOps c l proj
  | .seq name l proj os =>
    operandsOps c os ++ placeOps c l proj ++ (if readOnlySeq.contains name then [] else [.op .elems])
  | .call g os =>
    let (consume, cands) := argsOf c 0 os
    consume ++ argChecks c g 0 cands ++ [.op (if c.scalarFn g then .pure else .elems)] ++ cleanup c 0 cands
  | .dyn gs l proj os =>
    let (consume, cands) := argsOf c 1 os
    let this := c.base
    consume ++ placeOps c l proj ++ [refOp c this l proj false] ++
      gs.flatMap (fun g => .live this g 0 :: argChecks c g 1 cands) ++
      [.op .elems, .op (.moveOut this)] ++ cleanup c 1 cands
  | .closure g os => operandsOps c os ++ escapeOps c g
  | .callValue fv os => operandsOps c os ++ operandOps c fv ++ [.op .elems]
  | .builtin name os =>
    let spawn := if name == "spawn" then
        (match os.head? with
         | some (.const (some g)) => escapeOps c g
         | _ => [COp.bad "spawn の関数が分からない"])
      else []
    operandsOps c os ++ spawn ++ [.op (if pureBuiltins.contains name then .pure else .elems)]

/-- own の変数に入れたとき、その変数が新しい持ち主になる右辺（ほかは事実を付けない） -/
def ownSource (c : Ctx) : Rvalue → Bool
  | .use (.move l) | .use (.read l) => c.isOwn l
  | .own _ | .call .. | .dyn .. | .callValue .. | .builtin .. => true
  -- 場所から取り出す（`take`）・リストから取り除く（remove・pop など）: 元の場所は持ち主でなくなる
  | .take .. | .seq .. => true
  | _ => false

/-- `Inst::Assign` -/
def assignOps (c : Ctx) (x : Nat) (proj : List Proj) (v : Rvalue) : List COp :=
  if !proj.isEmpty then
    valueOps c v ++ placeOps c x proj ++ [.op (.writeInside (valueHolds c v))]
  else if c.isOwn x then
    match v with
    | .copyOut .. => valueOps c v ++ [.op (.assignNone x (c.mayHold x))]
    | _ => if ownSource c v then valueOps c v ++ [.op (.assignOwn x (c.mayHold x))]
           -- 新しい持ち主と言えない値（たどり着かない枝の `()` など）は、事実を付けずに入れる
           else valueOps c v ++ [.op (.assignNone x (c.mayHold x))]
  else if c.isRef x then
    match v with
    | .use (.copy l []) =>
      if c.isOwn l then [.bad s!"own の変数 _{l} の copy"] else [.op (.copyRef x l)]
    | .use (.move l) | .use (.read l) =>
      if c.isOwn l then [.bad s!"own の変数 _{l} を ref の変数に入れる"]
      else if l == x then [.bad "自分自身へのムーブ"]
      else [.op (.moveRef x l)]
    | .ref l rp follows => placeOps c l rp ++ [refOp c x l rp follows]
    | _ => valueOps c v ++ [.op (.assignNone x false)]
  else valueOps c v ++ [.op (.assignNone x (c.mayHold x))]

def instOps (c : Ctx) : Inst → List COp
  | .assign x proj v => assignOps c x proj v
  | .drop l => [.op (.drop l (c.mayHold l))]
  | .discard o =>
    let holds := match o with
      | .move l | .read l => c.mayHold l
      | .copy l [] => c.mayHold l
      | .const _ => false
      | .copy _ _ => true
    operandOps c o ++ [.op (if holds then .elems else .pure)]

def termOps (c : Ctx) : Term → List COp
  | .goto _ => []
  | .operand o _ => operandOps c o
  | .switch l proj _ => placeOps c l proj

/-- 引数を置く変数のぶんを足した、変数の数 -/
def width (f : Func) : Nat :=
  let rec opsLen : Rvalue → Nat
    | .call _ os | .dyn _ _ _ os => os.length + 1
    | _ => 0
  let insts := f.blocks.toList.flatMap (·.1)
  f.locals.size + 1 + insts.foldl (fun m i => match i with
    | .assign _ _ v => max m (opsLen v)
    | _ => m) 0

def codeOf (fns : Array Func) (f : Func) : Code :=
  let c : Ctx := { fns, f, base := f.locals.size }
  { blocks := f.blocks.map fun (is, t) => is.flatMap (instOps c) ++ termOps c t
    succ := f.blocks.map fun (_, t) => t.succ }

def ownParam (f : Func) (i : Nat) : Bool :=
  f.locals.getD i "?" == "Own" && i + f.captures < f.params

/-! ## 事実と Pinned の表を計算する（確かめ直すので、正しさは要らない） -/

def tab (N : Nat) (F : Facts) : Array (Option Fact) := (List.range N).toArray.map F

def meetArr (a b : Array (Option Fact)) : Array (Option Fact) :=
  a.zipWith (fun x y => if x == y then x else none) b

partial def facts (N : Nat) (start : Facts) (code : Code) : Entry := Id.run do
  let n := code.blocks.size
  let mut E : Entry := Array.replicate n none
  if n == 0 then return E
  E := E.set! 0 (some (tab N start))
  let mut work := [0]
  let mut fuel := 100000
  while fuel > 0 do
    fuel := fuel - 1
    match work with
    | [] => break
    | b :: rest =>
      work := rest
      let some eb := E[b]?.join | continue
      let out := (opsOf (code.blocks.getD b [])).foldl transferA eb
      for d in code.succ.getD b [] do
        if d < n then
          let next := match E[d]?.join with
            | none => out
            | some old => meetArr old out
          if E[d]?.join != some next then
            E := E.set! d (some next)
            work := d :: work
  return E

/-- 確かめることを、通らなくても先へ進みながら集める（Pinned の表を決めるため） -/
def collect (F : FA) : List COp → List (Nat × Nat × Bool)
  | [] => []
  | .op o :: rest => collect (transferA F o) rest
  | .live a f i :: rest => (f, i, isLive (toFacts F) (toFacts F a)) :: collect F rest
  | .notLive f i :: rest => (f, i, false) :: collect F rest
  | _ :: rest => collect F rest

def pinnedOf (tbl : Array (Array Bool)) : PinTable := fun f i => (tbl[f]?.bind (·[i]?)).getD false

def startOf (P : PinTable) (fi : Nat) (f : Func) : Facts :=
  startFacts (fun i => P fi i) (ownParam f) f.params

/-- analyze.rs と同じく、全部 true から始めて、反例が出たら false にする -/
partial def pinTable (fns : Array Func) (codes : Array Code) : Array (Array Bool) := Id.run do
  let mut tbl : Array (Array Bool) := fns.map fun f => Array.replicate f.params true
  for _ in [0:100] do
    let P := pinnedOf tbl
    let mut seen : Array (Array (Option Bool)) := fns.map fun f => Array.replicate f.params none
    for fi in [0:fns.size] do
      let f := fns[fi]!
      let code := codes[fi]!
      let N := width f
      let E := facts N (startOf P fi f) code
      for b in [0:code.blocks.size] do
        if let some eb := E[b]?.join then
          for (g, i, ok) in collect eb (code.blocks.getD b []) do
            if h : g < seen.size then
              let row := seen[g]
              if i < row.size then
                seen := seen.set g (row.set! i (some ((row[i]!).getD true && ok)))
    -- main は呼ばれ方が分からない。一度も呼ばれない関数の引数も何も仮定しない
    let next := seen.zipIdx.map fun (row, fi) =>
      row.map fun o => fns[fi]!.name != "main" && o.getD false
    if next == tbl then break
    tbl := next
  return tbl

/-! ## 実行 -/

def countElided (code : Code) : Nat :=
  code.blocks.foldl (fun n b => n + (b.filter (fun | .elided _ => true | _ => false)).length) 0

/-- 通らないとき、どこで通らないかを探す（表示用） -/
def whyFail (N : Nat) (P : PinTable) (code : Code) (E : Entry) : String := Id.run do
  for b in [0:code.blocks.size] do
    if let some eb := E[b]?.join then
      let mut F := eb
      for (op, k) in (code.blocks.getD b []).zipIdx do
        match runA P F [op] with
        | none =>
          let what := match op with
            | .bad why => why
            | .elided r => s!"消された *_{r} を事実から安全と言えない（_{r} の事実: {repr (toFacts F r)}）"
            | .live a g i => s!"関数 {g} の {i} 番目は Pinned だが、渡す値 _{a} が生きた ref と言えない"
            | .notLive g i => s!"関数 {g} の {i} 番目は Pinned だが、生きた ref でない値を渡す"
            | .op _ => "型の決まり（own の写し・自分自身へのムーブ）を満たさない"
            | .escape _ => "?"
          return s!"bb{b} の {k} 番目: {what}"
        | some F' => F := F'
  return s!"入口・合流の条件（N = {N}）"

def checkFile (path : String) : IO Bool := do
  let text ← IO.FS.readFile path
  match readProgram text with
  | .error e =>
    IO.eprintln s!"{path}: 読めません: {e}"
    return false
  | .ok fns =>
    let codes := fns.map (codeOf fns)
    let tbl := pinTable fns codes
    let P := pinnedOf tbl
    let mut ok := true
    let mut elided := 0
    for fi in [0:fns.size] do
      let f := fns[fi]!
      let code := codes[fi]!
      let N := width f
      let start := startOf P fi f
      let E := facts N start code
      let mainOk := f.name != "main" || (List.range f.params).all (fun i => !P fi i)
      if mainOk && verifyFast N P start code E then
        elided := elided + countElided code
      else
        ok := false
        IO.eprintln s!"{path}: 関数 {f.name}（{fi}）: 通りません: {whyFail N P code E}"
    if ok then IO.println s!"{path}: VERIFIED（関数 {fns.size}・消した検査 {elided}）"
    return ok

end JabberwockFormal.CheckerMain

def main (args : List String) : IO UInt32 := do
  let mut ok := true
  for a in args do
    let r ← JabberwockFormal.CheckerMain.checkFile a
    ok := ok && r
  return if ok then 0 else 1
