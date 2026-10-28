# Formal verification tool: completion contract

## Scope and controlling distinction
The current engineering deliverable is the **tool that checks, runs, audits and reports formal verification**, not a theorem establishing correctness of the entire Jabberwock compiler.

These independent states MUST NOT be conflated:
1. **Tool implementation**: required commands, integrations, evidence validation, diagnostics and report generation have been implemented.
2. **Tool verification**: tool code has been built and independently tested on a target environment.
3. **Target proof acceptance**: the jwc and Rust compilers satisfy the 34 previously defined empirical and mathematical obligations.

A missing compiler-wide proof, a failed target proof, or lack of Windows execution evidence does NOT by itself make the tool implementation incomplete. Conversely, an implemented tool does NOT imply its own tests passed or that the target is proven correct. Preserve fail-closed behavior and `NOT_PROVEN` for proof results.

## Implementation completion inventory
Use a finite, explicit list of **tool capabilities** as the denominator. Each item is IMPLEMENTED, PARTIAL or MISSING on the basis of actual code paths, never the count of unproved source constructs or IR instructions. `UNKNOWN` is not counted as implemented. If no code audit has established an item, mark it UNKNOWN. Progress equals number of IMPLEMENTED items divided by total capabilities, and must be labelled an implementation estimate. Keep verification separately as NOT_RUN, PASS or FAIL.

Initial capability groups for review:
- Formal proof engine invocation and kernel/axiom audit
- Independent proof-obligation discovery and provenance checking
- Real compiler-to-model correspondence checking (report unsupported edges honestly)
- Practical safety probe orchestration on supported platforms
- Negative controls, missing-evidence rejection and tamper detection
- End-to-end runner with stage isolation, exit codes and sealed logs
- Machine-readable, explainable results for each obligation
- Evidence packaging and source/environment identity
- Repeatable Windows toolchain setup and independent local operation
- Regression tests for the tool and a single documented command entrypoint

Do not mark items IMPLEMENTED purely from the existence of a named script. An implementation of a correspondence **checker** can be complete even when the compiler-to-model correspondence does not hold; it must faithfully report NOT_PROVEN. If a claimed checker only matches strings while its contract calls for semantic checking, mark that capability PARTIAL.

## Execution discipline for the development agent
- First inspect the implementation, identify actual missing tool capabilities and fix them, in bounded batches.
- Do not grow the tool backlog merely because the tool discovers a new unproved compiler feature. That is a target-result finding, not automatically a tooling defect.
- Do not implement every source syntax, IR opcode or runtime path merely to improve an artificial proof-success percentage.
- Do not report 34/34 as tool progress, and do not turn NOT_PROVEN into PASS.
- Do not make tool implementation completion depend on tests executed by the user. Preserve independent readiness and tested-status reports.
- Keep existing acceptance audit and local all-tests command unchanged in meaning.
- In every update report: implemented capabilities / total, concrete missing tool functions, tool test status, and target proof status separately.

This contract changes **development scope and reporting**, not the mathematical proof requirements documented in `docs/acceptance-34.md`.
