#!/usr/bin/env python3
"""関数つきの模型（formal/JabberwockFormal/CoreGenFun.lean）と jwc の答えを、小さな例の上で比べる。

1. 決まった種（--seed）から、模型の式を作る（型に沿って作るので多くは型が付く。一部はムーブした変数も
   使う作り方にして、型が付かないものも混ぜる）。
   例は実行のたびに作り直し、リポジトリには置かない。
2. Lean の実行ファイル `core-conform`（formal/JabberwockFormal/ConformMain.lean）が、式ごとに
   模型の型の検査（`infer`。型規則どおりであることは `infer_sound` で証明済み）と評価の答えを出す。
3. 式を jabberwock のプログラムに直し、jwc で作って動かす。
4. 比べる:
   - 模型で型が付き、評価が値 n → jwc の実行ファイルが n を出して終了コード 0 なら MATCH。
   - 模型で型が付き、評価が実行時の誤り（解放された区画の ref） → jwc の実行ファイルが終了コード 1 で
     `error:` を出せば MATCH。
   - 上の 2 つで答えが違う → MISMATCH（失敗）。
   - 模型で型が付くのに jwc が作れない → JWC_REJECTS（失敗ではない。jwc の方が厳しい）。
   - 模型で型が付かない → UNTYPED_JWC_REJECTS / UNTYPED_JWC_RUNTIME_ERROR / UNTYPED_JWC_RUNS
     （数えるだけ。jabberwock はムーブした変数を使うと実行時に止める）。
   - 燃料切れ → FUEL（数えるだけ）。模型で型が付くのに stuck → TOOL_ERROR（`conform_no_stuck` に反する）。

これは例の上の突き合わせで、証明ではない。整数の定数は 0 以上（ときどき足すとあふれる大きい数。
模型も仕様も i64 の幅で一周する）。

使い方: python3 scripts/core_conform.py --jwc <jwc の実行ファイル> [--clang clang] [--count 300] [--seed 20261101] [--build]
型の付く例を jwc が 1 本も作れなければ TOOL_ERROR（道具が動いていない）。
結果: target/core-conform/summary.json。MISMATCH が無ければ終了コード 0、MISMATCH があれば 1、道具が動かなければ 2。
"""
import argparse
import concurrent.futures
import json
import os
import pathlib
import random
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent

# confFns（CoreGenConform.lean）と同じ順・同じ意味の関数
FNS_SRC = """fn ref i64 f0(own i64 x0) { ref x0 }
fn i64 f1(ref mut i64 x0) { *x0 = 7; 0 }
fn i64 f2(own i64 x0) { *x0 }
fn own i64 f3(own i64 x0) { x0 }
"""

INT = ("int",)
BOOL = ("bool",)
BOX_INT = ("box", INT)
BOX_BOOL = ("box", BOOL)
REF = ("ref", False, INT)
REF_MUT = ("ref", True, INT)


def ty_json(t):
    if t[0] == "box":
        return {"k": "box", "t": ty_json(t[1])}
    if t[0] == "ref":
        return {"k": "ref", "m": t[1], "t": ty_json(t[2])}
    return {"k": t[0]}


def plain(t):
    if t[0] == "box":
        return False
    if t[0] == "ref":
        return plain(t[2])
    return True


class Gen:
    """型に沿って式を作る。文脈は [型, 生きているか] の並び（模型の Ctx と同じ）"""

    def __init__(self, rng, sloppy=False):
        self.rng = rng
        self.sloppy = sloppy

    def lit(self):
        """整数の定数: たいていは小さい数。ときどき足すとあふれる大きい数（i64 の幅で一周する）"""
        if self.rng.random() < 0.2:
            return self.rng.choice([9223372036854775807, 4611686018427387904])
        return self.rng.randrange(10)

    def live(self, ctx, t):
        return [i for i, (u, alive) in enumerate(ctx) if (alive or self.sloppy) and u == t]

    def gen(self, ctx, t, d):
        """型 t の式と、評価の後の文脈を返す"""
        r = self.rng
        opts = []
        if t == INT:
            opts.append(lambda: ({"k": "int", "n": self.lit()}, ctx))
        if t == BOOL:
            opts.append(lambda: ({"k": "bool", "b": r.random() < 0.5}, ctx))
        if plain(t):
            for i in self.live(ctx, t):
                opts.append(lambda i=i: ({"k": "var", "i": i}, ctx))
        if t[0] == "box":
            for i in self.live(ctx, t):
                opts.append(lambda i=i: self.move(ctx, i))
        if t[0] == "ref":
            for i in self.live(ctx, ("box", t[2])):
                opts.append(lambda i=i: ({"k": "addr", "m": t[1], "i": i}, ctx))
        if plain(t):
            for i in self.live(ctx, ("box", t)):
                opts.append(lambda i=i: ({"k": "peek", "i": i}, ctx))
        if d > 0:
            if t == INT:
                opts.append(lambda: self.add(ctx, d))
                opts.append(lambda: self.store(ctx, d))
                opts.append(lambda: self.call(ctx, 1, REF_MUT, d))
                opts.append(lambda: self.call(ctx, 2, BOX_INT, d))
            if t == BOX_INT:
                opts.append(lambda: self.call(ctx, 3, BOX_INT, d))
            if t == REF:
                opts.append(lambda: self.call(ctx, 0, BOX_INT, d))
            if plain(t) and t[0] != "ref":
                opts.append(lambda: self.deref(ctx, ("ref", r.random() < 0.5, t), d))
            if t[0] == "box" and plain(t[1]):
                opts.append(lambda: self.mkbox(ctx, t, d))
            opts.append(lambda: self.let(ctx, t, d))
        if not opts:
            # この型の式が作れない（例: 生きた箱が無い）。新しい箱を作る
            if t[0] == "box":
                return self.mkbox(ctx, t, 0)
            if t[0] == "ref":
                return self.let_box_ref(ctx, t)
            return {"k": "int", "n": 0}, ctx
        return r.choice(opts)()

    def move(self, ctx, i):
        c = [list(x) for x in ctx]
        c[i][1] = False
        return {"k": "move", "i": i}, [tuple(x) for x in c]

    def add(self, ctx, d):
        a, c1 = self.gen(ctx, INT, d - 1)
        b, c2 = self.gen(c1, INT, d - 1)
        return {"k": "add", "a": a, "b": b}, c2

    def store(self, ctx, d):
        a, c1 = self.gen(ctx, REF_MUT, d - 1)
        b, c2 = self.gen(c1, INT, d - 1)
        return {"k": "store", "a": a, "b": b}, c2

    def call(self, ctx, f, param, d):
        e, c1 = self.gen(ctx, param, d - 1)
        return {"k": "call", "f": f, "e": e}, c1

    def deref(self, ctx, rt, d):
        e, c1 = self.gen(ctx, rt, d - 1)
        return {"k": "deref", "e": e}, c1

    def mkbox(self, ctx, t, d):
        e, c1 = self.gen(ctx, t[1], max(d - 1, 0))
        return {"k": "box", "t": ty_json(t[1]), "e": e}, c1

    def let(self, ctx, t, d):
        s = self.rng.choice([INT, BOOL, BOX_INT, BOX_INT, BOX_INT, BOX_BOOL, REF, REF_MUT])
        x, c1 = self.gen(ctx, s, d - 1)
        body, c2 = self.gen(c1 + [(s, True)], t, d - 1)
        return {"k": "let", "x": x, "body": body}, c2[:-1]

    def let_box_ref(self, ctx, t):
        x, c1 = self.mkbox(ctx, ("box", t[2]), 0)
        n = len(c1)
        return {"k": "let", "x": x, "body": {"k": "addr", "m": t[1], "i": n}}, c1


def make_cases(seed, count):
    rng = random.Random(seed)
    cases = []
    while len(cases) < count:
        # 15% は、ムーブした変数も使ってよい作り方にする（模型では型が付かない。jwc は実行時に止めるはず）
        g = Gen(rng, sloppy=rng.random() < 0.15)
        if rng.random() < 0.6:
            # 先に箱を 1 つか 2 つ作っておく（ref の元がないと、関数の外に持ち出す ref ばかりになる）
            n = 1 if rng.random() < 0.5 else 2
            ctx = [(BOX_INT, True)] * n
            e, _ = g.gen(ctx, INT, rng.randrange(1, 5))
            for _ in range(n):
                b = {"k": "box", "t": ty_json(INT), "e": {"k": "int", "n": rng.randrange(10)}}
                e = {"k": "let", "x": b, "body": e}
        else:
            e, _ = g.gen([], INT, rng.randrange(1, 5))
        cases.append(e)
    return cases


class Translate:
    """模型の式を jabberwock の式に直す。変数の番号 i は名前 x{i}（文脈の位置そのまま）"""

    def __init__(self):
        self.tmp = 0

    def ex(self, e, n):
        """n: 今の文脈の長さ（let の変数の位置）"""
        k = e["k"]
        if k == "int":
            return str(e["n"])
        if k == "bool":
            return "true" if e["b"] else "false"
        if k in ("var", "move"):
            return f"x{e['i']}"
        if k == "peek":
            return f"*x{e['i']}"
        if k == "addr":
            return ("ref mut " if e["m"] else "ref ") + f"x{e['i']}"
        if k == "deref":
            inner = e["e"]
            if inner["k"] in ("var", "move"):
                return f"*x{inner['i']}"
            return f"*({self.ex(inner, n)})"
        if k == "store":
            a, b = e["a"], e["b"]
            if a["k"] == "var":
                return f"if true {{ *x{a['i']} = {self.ex(b, n)}; 0 }} else {{ exit(3) }}"
            t = f"t{self.tmp}"
            self.tmp += 1
            return (f"if true {{ let mut {t} = {self.ex(a, n)}; *{t} = {self.ex(b, n)}; 0 }}"
                    " else { exit(3) }")
        if k == "box":
            return f"own ({self.ex(e['e'], n)})"
        if k == "add":
            return f"({self.ex(e['a'], n)} + {self.ex(e['b'], n)})"
        if k == "let":
            x = self.ex(e["x"], n)
            body = self.ex(e["body"], n + 1)
            return f"if true {{ let mut x{n} = {x}; {body} }} else {{ exit(3) }}"
        if k == "call":
            return f"f{e['f']}({self.ex(e['e'], n)})"
        raise ValueError(f"知らない式: {k}")


def program(e):
    return FNS_SRC + "\nfn main() {\n    print(" + Translate().ex(e, 0) + ");\n}\n"


def run_jwc(jwc, clang, d, i, src):
    p = d / f"case{i:04}.jw"
    exe = d / (f"case{i:04}" + (".exe" if os.name == "nt" else ""))
    p.write_text(src, encoding="utf-8")
    b = subprocess.run([str(jwc), "build", "--clang", clang, "-o", str(exe), str(p)],
                       capture_output=True, text=True, timeout=300)
    if b.returncode != 0:
        return {"built": False, "build_err": (b.stderr or b.stdout).strip().splitlines()[:3]}
    r = subprocess.run([str(exe)], capture_output=True, text=True, timeout=60)
    exe.unlink()
    return {"built": True, "code": r.returncode, "stdout": r.stdout,
            "stderr": r.stderr.strip().splitlines()[:1]}


def judge(m, j):
    if not m["typed"]:
        if not j["built"]:
            return "UNTYPED_JWC_REJECTS"
        return "UNTYPED_JWC_RUNTIME_ERROR" if j["code"] != 0 else "UNTYPED_JWC_RUNS"
    ev = m["eval"]
    if ev == "stuck":
        return "TOOL_ERROR"
    if ev == "fuel":
        return "FUEL"
    if not j["built"]:
        return "JWC_REJECTS"
    if ev == "ok":
        return "MATCH" if j["code"] == 0 and j["stdout"] == f"{m['val']}\n" else "MISMATCH"
    if ev == "err":
        ok = j["code"] == 1 and j["stdout"] == "" and any(s.startswith("error:") for s in j["stderr"])
        return "MATCH" if ok else "MISMATCH"
    return "MISMATCH"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--jwc", required=True)
    ap.add_argument("--count", type=int, default=300)
    ap.add_argument("--seed", type=int, default=20261101)
    ap.add_argument("--out", default=str(ROOT / "target" / "core-conform"))
    exe = "core-conform" + (".exe" if os.name == "nt" else "")
    ap.add_argument("--model", default=str(ROOT / "formal" / ".lake" / "build" / "bin" / exe))
    ap.add_argument("--build", action="store_true")
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("--clang", default=os.environ.get("JW_CLANG", "clang"),
                    help="jwc build に渡す clang（既定は JW_CLANG か clang。Lean の clang を拾わないように）")
    a = ap.parse_args()
    out = pathlib.Path(a.out)
    (out / "programs").mkdir(parents=True, exist_ok=True)
    summary = {"status": "TOOL_ERROR", "seed": a.seed, "count": a.count, "counts": {},
               "results": [], "note": "例の上の突き合わせで、証明ではない"}

    def save(code):
        (out / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2),
                                          encoding="utf-8")
        return code

    if a.build:
        r = subprocess.run(["lake", "build", "core-conform"], cwd=ROOT / "formal")
        if r.returncode != 0:
            summary["error"] = "lake build core-conform に失敗"
            return save(2)
    cases = make_cases(a.seed, a.count)
    (out / "cases.json").write_text(json.dumps(cases), encoding="utf-8")
    try:
        r = subprocess.run([a.model, str(out / "cases.json")], capture_output=True, text=True,
                           timeout=600)
    except OSError as e:
        summary["error"] = f"core-conform が動きません: {e}"
        return save(2)
    if r.returncode != 0:
        summary["error"] = f"core-conform の終了コード {r.returncode}: {r.stderr.strip()}"
        return save(2)
    model = [json.loads(line) for line in r.stdout.splitlines() if line.strip()]
    if len(model) != len(cases) or any(m["i"] != i for i, m in enumerate(model)):
        summary["error"] = "core-conform の答えの数が例の数と合いません"
        return save(2)
    jwc = pathlib.Path(a.jwc).resolve()
    srcs = [program(e) for e in cases]
    with concurrent.futures.ThreadPoolExecutor(max_workers=a.jobs) as pool:
        futs = [pool.submit(run_jwc, jwc, a.clang, out / "programs", i, s) for i, s in enumerate(srcs)]
        try:
            jres = [f.result() for f in futs]
        except (OSError, subprocess.TimeoutExpired) as e:
            summary["error"] = f"jwc が動きません: {e}"
            return save(2)
    counts = {}
    for i, (m, j) in enumerate(zip(model, jres)):
        v = judge(m, j)
        counts[v] = counts.get(v, 0) + 1
        summary["results"].append({"i": i, "verdict": v, "model": m, "jwc": j,
                                   "program": f"programs/case{i:04}.jw"})
    summary["counts"] = counts
    typed_built = sum(1 for m, j in zip(model, jres) if m["typed"] and j["built"])
    if typed_built == 0:
        # 型が付く例を jwc が 1 本も作れないのは、jwc か clang が動いていないとみなす（MATCH にしない）
        first = next((j["build_err"] for j in jres if not j["built"]), None)
        summary["error"] = f"jwc が型の付く例を 1 本も作れません: {first}"
        print(f"core-conform: TOOL_ERROR {summary['error']}")
        summary["status"] = "TOOL_ERROR"
        return save(2)
    if counts.get("TOOL_ERROR"):
        summary["status"] = "TOOL_ERROR"
        code = 2
    elif counts.get("MISMATCH"):
        summary["status"] = "MISMATCH"
        code = 1
    else:
        summary["status"] = "MATCH"
        code = 0
    print(f"core-conform: {summary['status']} {json.dumps(counts, ensure_ascii=False)}")
    for x in summary["results"]:
        if x["verdict"] in ("MISMATCH", "TOOL_ERROR"):
            print(f"  {x['verdict']}: {x['program']} 模型={x['model']} jwc={x['jwc']}")
    return save(code)


if __name__ == "__main__":
    sys.exit(main())
