import Lean.Data.Json
import JabberwockFormal.CoreGenConform

set_option autoImplicit false
set_option warningAsError true

/-!
# 模型と jwc を比べる実行ファイルの模型の側（`lake exe core-conform 例.json`）

`scripts/core_conform.py` が作った例（式の JSON の配列）を読み、式ごとに
1. 模型の型の検査（`infer`。型が付けば `HasTy` で型が付く: `infer_sound`）
2. 模型の評価（空の環境・空のヒープ、割り当ては毎回新しい区画、燃料 1000）
の答えを 1 行の JSON で出す。比べるのはスクリプトの側。
-/

open Lean (Json)
open JabberwockFormal.CoreGenFun
open JabberwockFormal.CoreGenConform

namespace JabberwockFormal.ConformMain

def field (j : Json) (k : String) : Except String Json := j.getObjVal? k
def str (j : Json) (k : String) : Except String String := do (← field j k).getStr?
def nat (j : Json) (k : String) : Except String Nat := do (← field j k).getNat?
def bool (j : Json) (k : String) : Except String Bool := do (← field j k).getBool?

partial def readTy (j : Json) : Except String Ty := do
  match ← str j "k" with
  | "int" => return .int
  | "bool" => return .bool
  | "box" => return .box (← readTy (← field j "t"))
  | "ref" => return .ref (← bool j "m") (← readTy (← field j "t"))
  | k => throw s!"知らない型: {k}"

partial def readExpr (j : Json) : Except String Expr := do
  match ← str j "k" with
  | "int" => return .int (← (← field j "n").getInt?)
  | "bool" => return .bool (← bool j "b")
  | "var" => return .var (← nat j "i")
  | "move" => return .move (← nat j "i")
  | "peek" => return .peek (← nat j "i")
  | "addr" => return .addr (← bool j "m") (← nat j "i")
  | "deref" => return .deref (← readExpr (← field j "e"))
  | "store" => return .store (← readExpr (← field j "a")) (← readExpr (← field j "b"))
  | "box" => return .mkBox (← readTy (← field j "t")) (← readExpr (← field j "e"))
  | "add" => return .add (← readExpr (← field j "a")) (← readExpr (← field j "b"))
  | "let" => return .letE (← readExpr (← field j "x")) (← readExpr (← field j "body"))
  | "call" => return .call (← nat j "f") (← readExpr (← field j "e"))
  | k => throw s!"知らない式: {k}"

def tyStr : Ty → String
  | .int => "int"
  | .bool => "bool"
  | .box t => "box " ++ tyStr t
  | .ref m t => (if m then "ref mut " else "ref ") ++ tyStr t

def resFields : Res → List (String × Json)
  | .ok (.int n) _ _ => [("eval", Json.str "ok"), ("val", Lean.toJson n)]
  | .ok _ _ _ => [("eval", Json.str "ok_other")]
  | .err => [("eval", Json.str "err")]
  | .fuel => [("eval", Json.str "fuel")]
  | .stuck => [("eval", Json.str "stuck")]

def run (path : String) : IO UInt32 := do
  let text ← IO.FS.readFile path
  match Json.parse text >>= Json.getArr? with
  | .error e =>
    IO.eprintln s!"{path}: 読めません: {e}"
    return 2
  | .ok cases =>
    let mut i : Nat := 0
    for c in cases do
      match readExpr c with
      | .error e =>
        IO.eprintln s!"{path}: {i} 番目の式が読めません: {e}"
        return 2
      | .ok e =>
        let tyFields : List (String × Json) :=
          match infer confFns [] e with
          | some (t, _) => [("typed", Json.bool true), ("ty", Json.str (tyStr t))]
          | none => [("typed", Json.bool false)]
        let r := eval (fun _ => none) confFns 1000 0 [] [] e
        IO.println (Json.mkObj ([("i", Lean.toJson i)] ++ tyFields ++ resFields r)).compress
      i := i + 1
    return 0

end JabberwockFormal.ConformMain

def main (args : List String) : IO UInt32 :=
  match args with
  | [p] => JabberwockFormal.ConformMain.run p
  | _ => do
    IO.eprintln "使い方: core-conform 例.json"
    return 2
