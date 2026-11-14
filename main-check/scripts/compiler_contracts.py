"""Specification expectations, not values copied from another compiler's output.

Every fixture is run separately through the selected compiler. IDs remain stable
in the report so a missing case/configuration cannot disappear in a Cargo total.
"""


def case(name, items, source, stdout="", error=None, reject=None, effect=None):
    return dict(id=name, items=items, source=source, stdout=stdout, error=error,
                reject=reject, effect=effect)


CASES = [
    case("arithmetic", [4, 5, 10, 12], "fn main() { print(2 + 3 * 4, (2 + 3) * 4); }", "14 20\n"),
    case("integer-wrap", [4, 5, 10], "fn add(i64 a) { print(a + 1); } fn main() { add(9223372036854775807); }", "-9223372036854775808\n"),
    case("short-circuit", [1, 5, 10], 'fn bool side() { print("BAD"); true } fn main() { print(false && side(), true || side()); }', "false true\n"),
    case("argument-order", [5, 10], "fn i64 mark(i64 n) { print(n); n } fn f(i64 a, i64 b) { print(a + b); } fn main() { f(mark(1), mark(2)); }", "1\n2\n3\n"),
    case("own-move-keeps-ref", [1, 2, 11], "fn main() { let a = own 7; let r = ref a; let b = a; print(*r, *b); }", "7 7\n"),
    case("own-drop-invalidates", [1, 2, 11], 'fn main() { let a = own 7; let r = ref a; drop(a); print("reached"); print(*r); }', "reached\n", error="dangling reference"),
    case("own-replace-invalidates", [1, 2, 11], 'fn main() { let mut a = own 7; let r = ref a; a = own 9; print("reached"); print(*r); }', "reached\n", error="dangling reference"),
    case("block-invalidates", [1, 2, 11], 'fn main() { let z = 0; let mut r = ref z; if true { let a = own 1; r = ref a; } print("reached"); print(*r); }', "reached\n", error="dangling reference"),
    case("array-negative-index", [2, 4, 9], 'fn main() { let a = [1, 2]; let i = -1; print("reached"); print(a[i]); }', "reached\n", error="範囲の外"),
    case("array-upper-index", [2, 4, 9], 'fn main() { let a = [1, 2]; let i = 2; print("reached"); print(a[i]); }', "reached\n", error="範囲の外"),
    case("array-boundary", [2, 9, 10], "fn main() { let a = [7, 9]; let i = 1; print(a[0], a[i]); }", "7 9\n"),
    case("list-write-keeps-ref", [1, 2, 11], "fn main() { let i64[] mut a = [7, 9]; let r = ref a[0]; a[0] = 4; print(*r); }", "4\n"),
    case("break-continue-cleanup", [1, 11], "fn main() { let mut n = 0; for i in range(5) { let a = own i; if i == 1 { continue; } if i == 3 { break; } n += *a; } print(n); }", "2\n"),
    case("return-cleanup", [1, 11], "fn own i64 f() { let scratch = own 1; let result = own 9; return result; } fn main() { let a = f(); print(*a); }", "9\n"),
    case("question-mark-cleanup", [10, 11], 'fn Result<i64, str> fail() { Err("stop") } fn Result<i64, str> f() { let a = own 9; let b = fail()?; Ok(*a + b) } fn main() { print(f()); }', "Err(stop)\n"),
    case("closure-own", [10, 11], "fn main() { let a = own 5; let f = () -> *a; print(f()); }", "5\n"),
    case("generic-own", [9, 10, 11], "struct Box<T> { T value; } fn T take<T>(Box<T> b) { b.value } fn main() { let p = take(Box { value: own 7 }); print(*p); }", error="フィールドにある own は取り出せません"),
    case("enum-own", [10, 11], "fn main() { let a = Some(own 7); match a { case Some(p) -> print(*p); case None -> print(0); } }", "7\n"),
    case("map-values", [10, 11], 'fn main() { let i64[str] m = ["a": 7]; print(m.len()); }', "1\n"),
    case("thread-join", [3, 11, 12], "fn i64 f() { 7 } fn main() { print(spawn(f).join()); }", "7\n"),
    case("thread-own-transfer", [3, 11, 12], "fn main() { let a = own 5; let t = spawn(() -> a); let b = t.join(); print(*b); }", "5\n"),
    case("channel-order", [3, 11, 12], "fn main() { let chan<i64> c = chan(); let t = spawn(() -> { c.send(7); c.close(); }); print(c.recv(), c.recv()); t.join(); }", "Some(7) None\n"),
    case("reject-immutable-write", [2, 10], "fn main() { let x = 1; x = 2; }", reject="mut ではない"),
    case("reject-readonly-ref", [2, 10], "fn main() { let mut a = own 1; let r = ref a; *r = 2; }", reject="読むだけの ref"),
    case("reject-ref-send", [2, 3, 10], "fn main() { let a = own 1; let r = ref a; let t = spawn(() -> *r); t.join(); }", reject="ref"),
    case("reject-missing-type", [10], "fn f(x) {} fn main() {}", reject="型が必要"),
]

CASES.extend([
    case("generic-enum-array-expanded-limit", [9,10], "enum Buffer<T> { Data(T[131072] cells); } fn accept(Buffer<i64[2]> b) {} fn main() {}", reject="配列が大きすぎます"),
    case("generic-array-expanded-limit", [9,10], "struct Buffer<T> { T[131072] cells; } fn accept(Buffer<i64[2]> b) {} fn main() {}", reject="配列が大きすぎます"),
    case("generic-array-expanded-boundary", [9,10], "struct Buffer<T> { T[131072] cells; } fn accept(Buffer<i64> b) {} fn main() {}"),
])

# Every invalidating list operation must reach the final read (not a type error).
for name, operation in [
    ("push", "a.push(8);"), ("pop", "a.pop();"), ("insert", "a.insert(0, 8);"),
    ("remove", "a.remove(0);"), ("drain", "a.drain();"), ("reverse", "a.reverse();"),
    ("sort", "a.sort();"), ("sort-by", "a.sort_by((x, y) -> x > y);"),
    ("sort-unstable", "a.sort_unstable();"),
    ("sort-unstable-by", "a.sort_unstable_by((x, y) -> x < y);"),
]:
    CASES.append(case("list-invalidates-" + name, [1, 2, 11],
                      'fn main() { let i64[] mut a = [3, 1, 2]; let r = ref a[0]; '
                      + operation + ' print("reached"); print(*r); }',
                      "reached\n", error="dangling reference"))

# Additional goal-level cases, executed independently of the twelve-category runs.
GOAL_CASES = {
    "semantics": [case("goal-evaluation-order", [10], "fn i64 mark(i64 n) { print(n); n } fn main() { print(mark(3) + mark(4)); }", "3\n4\n7\n")],
    "memory": [case("goal-repeated-owner-cleanup", [11], "fn main() { let mut n = 0; for i in range(100) { let a = own i; n += *a; } print(n); }", "4950\n")],
    "types": [case("goal-reject-return-type", [10], 'fn i64 f() { "bad" } fn main() {}', reject="型")],
    "ownership": [case("goal-move-chain", [11], "fn main() { let a = own 23; let r = ref a; let b = a; let c = b; print(*r, *c); }", "23 23\n")],
    "checks": [case("goal-check-after-owner-drop", [2], 'fn main() { let a = own 23; let r = ref a; drop(a); print("check"); print(*r); }', "check\n", error="dangling reference")],
}
