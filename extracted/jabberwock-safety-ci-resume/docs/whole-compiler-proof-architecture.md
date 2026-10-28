# Whole-compiler proof architecture

This is a proof engineering specification, **not** a proof of the implementation.

## Objective

For each of jwc and the Rust bootstrap compiler, establish G1–G5 for
every supported source program and each supported backend/runtime configuration.
A theorem about a tiny model, a green unit test, and a source SHA-256 are all
insufficient to conclude this.

## Refinement chain

Each edge must be specified with a *typed semantic relation* and proved for the
actual code path. No edge can be replaced by a string/metadata assertion:

1. **Source/lexer/parser:** characterize input tokens, parsed AST, diagnostics,
   import resolution, malformed input rejection, and parser termination.
2. **Typing/ownership:** prove type preservation and progress, effects,
   aliasing, borrow/move/drop, and implementation correspondence of the checker.
3. **Typed AST to raw IR:** show lowering preserves observable behavior or
   specified safe errors; include return, exceptions, loops, closures, arrays,
   sums, concurrency, FFI, and all supported constructs.
4. **IR analyses and optimizations:** certify all mutations, constant folding,
   range proofs, lifetime/check elision, integer overflow and rewrites.
   A proof must establish a simulation relation with all required side effects.
5. **LLVM emission:** prove emitted target IR is well-defined and refines IR
   semantics, including atomics, alignment, lifetime, pointer provenance,
   poison/undef, data layout, and trap/error behavior.
6. **Linker/runtime/OS:** prove or explicitly assume a trusted contract for
   C runtime entry points, memory allocation/free, thread synchronization,
   FFI, unsupported targets, and toolchain/linker preservation.
7. **Whole compiler:** compose verified edges, with source-to-artifact
   correspondence, runtime semantics and an explicit trusted computing base.

For each source file, `scripts/whole_compiler_proof.py` discovers the file and
creates goal/phase/source-specific proof obligations. Missing, duplicated,
changed, or untracked files cannot be silently accepted.

**This inventory is a starting point, not a substitute for the required
semantic relation.** File-level coverage alone is also insufficient: every
implemented branch, transform and supported configuration must be included.
All newly introduced features require new refinement obligations and proofs.

## Operational interfaces

Run in the repository root on Windows:

```powershell
py scripts/whole_compiler_proof.py
py scripts/verify_all_local.py
```

The full runner invokes the proof inventory as a required stage and keeps
`target/verification-complete/whole-proof-inventory.json`, alongside proof,
safety, benchmark, and regression reports. The inventory exits nonzero while
any compiler-wide relation remains unproved. The full runner likewise
remains nonzero until all 34 acceptance criteria are genuinely established.

## Proof-completion prerequisites

The remaining development work is substantive:

- Define operational semantics for **all** supported source/typed IR/LLVM/runtime
  constructs rather than only a reference-gate fragment.
- Extract verified algorithmic code from Lean or establish mechanically
  checked source-to-model refinement for both real implementations.
- Prove the implementation-specific transitions and pass composition.
- Model target/toolchain assumptions and test them independently.
- Check all theorem dependencies/axioms at the kernel and reject untrusted
  claims; review the trusted computing base explicitly.

Until these exist, output is **NOT_PROVEN**. No number of passing concrete
tests or manifest entries can be promoted into a universal theorem.


## 34件受入監査への証明義務の接続

`acceptance_report.py --inventory target/whole-proof-inventory.json` によって、
34件の正式受入判定に実装ソースごとの未証明リンクを必須入力として渡す。
新規ソース、証明義務の単件欠落、フェーズの偽装、ソースハッシュ変更、
`PROVEN` の自己申告は正規インベントリとの差分として拒否する。

このリストは**証明義務の網羅を補助する索引**であり、関数・分岐・FFI境界の意味論上の網羅性や
カーネルによる実装対応証明を保証するものではない。
