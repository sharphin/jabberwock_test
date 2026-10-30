"""Compiler-attributed practical probes. Formal proof is deliberately separate."""
import sys
import local_safety as safety
from safety_platforms import executable, missing
from compiler_contracts import CASES, GOAL_CASES
import provisional_detectors as detectors

COMPILERS = ("jwc", "rust")

def catalog(target, scale, work, compilers=COMPILERS):
    rust = executable(target, "jabberwock")
    jwc = executable(target, "jwc-local")
    preparations = [safety.probe("cargo-build", [], ["cargo", "build", "--release"], timeout=1200),
                    safety.probe("cargo-debug-build", [], ["cargo", "build"], timeout=1200)]
    if "jwc" in compilers:
        p = safety.probe("jwc-build", [], [str(rust), "build", "--debug-native", "-o", str(jwc), "jwc/main.jw"], clang=True)
        p["dependencies"] = ["cargo-build"]
        preparations.append(p)
    if target == "windows":
        diagnostic = work / "windows-asan-diagnostic.json"
        preflight = safety.probe("windows-asan-preflight", [],
                            [sys.executable, "scripts/check_windows_asan.py", "--output", str(diagnostic)], clang=True, timeout=420)
        preflight["case_report"] = str(diagnostic)
        preparations.append(preflight)
    result = []
    for subject in compilers:
        compiler = jwc if subject == "jwc" else rust
        deps = ["jwc-build"] if subject == "jwc" else ["cargo-build", "cargo-debug-build"]
        env = {"JW_SAFETY_COMPILER": subject, "JW_SAFETY_STRICT": "1"}
        if subject == "jwc":
            env["JW_SAFETY_JWC"] = str(jwc)
        def add(name, items, command, *, nested=False, count=None, extra=None, timeout=3600):
            name = subject + "-" + name
            output = work / (name + ".json")
            p = safety.probe(name, items, command + (["--output", str(output)] if nested else []),
                             env={**env, **(extra or {})}, clang=True, timeout=timeout)
            p.update(compiler=subject, dependencies=deps[:])
            if nested:
                p.update(case_report=str(output), expected_subject=subject, expected_cases=count)
            result.append(p)
            return p
        for goal, cases in GOAL_CASES.items():
            p = add("goal-" + goal, [], [sys.executable, "scripts/check_compiler_contracts.py", "--subject", subject,
                    "--compiler", str(compiler), "--profile", "release", "--goal", goal], nested=True)
            p["goal"] = goal
            p["required_case_ids"] = [c["id"] + ":" + variant + suffix for c in cases
                                      for variant in ("elided", "checked")
                                      for suffix in (["-reject"] if c["reject"] else ["-O1", "-O2"])]
        for mode, items in [("contracts", [1,2,3,4,5,9,10,11,12]), ("address", [2,5,11]),
                            ("allocation", [8,11]), ("thread", [3]), ("memory", [6]), ("undefined", [4])]:
            if target == "windows" and mode in ("thread", "memory", "undefined"):
                detector = {"thread": "inspector", "memory": "drmemory", "undefined": "ubsan"}[mode]
                if detectors.executable(detector):
                    p = add(mode, items, [sys.executable, "scripts/check_external_detector.py", "--subject", subject,
                            "--compiler", str(compiler), "--detector", detector], nested=True, timeout=14400)
                    p["required_case_ids"] = detectors.expected_ids(detector)
                else:
                    variable, command = detectors.TOOLS[detector]
                    p = missing(subject + "-" + mode, items, "Provisional " + detector + " unavailable; install/configure " + variable + " (" + command + ")")
                    p.update(compiler=subject, dependencies=[])
                    result.append(p)
                p["provisional_detector"] = {"tool": detector, "replacement": "dedicated Jabberwock detector planned"}
                continue
            p = add(mode, items, [sys.executable, "scripts/check_compiler_contracts.py", "--subject", subject,
                                  "--compiler", str(compiler), "--profile", "release", "--mode", mode], nested=True)
            if mode != "allocation":
                cases = [c for c in CASES if mode != "thread" or 3 in c["items"]]
                p["required_case_ids"] = [c["id"] + ":" + variant + suffix for c in cases
                                          for variant in ("elided", "checked")
                                          for suffix in (["-reject"] if c["reject"] else ["-O1", "-O2"])]
            if mode in ("address", "thread", "memory"):
                negative = add(mode + "-negative", items,
                    ["cargo", "test", "--test", "sanitizers", "generated_llvm_is_instrumented", "--", "--exact", "--nocapture"],
                    extra={"JW_CFLAGS": "-fsanitize=" + mode})
                negative["required_test_names"] = ["generated_llvm_is_instrumented"]
        if detectors.executable("llvm"):
            p = add("llvm-verifier", [4], [sys.executable, "scripts/check_external_detector.py", "--subject", subject,
                    "--compiler", str(compiler), "--detector", "llvm"], nested=True)
            p["required_case_ids"] = detectors.expected_ids("llvm")
        else:
            p = missing(subject + "-llvm-verifier", [4], "LLVM opt unavailable; configure JW_OPT")
            p.update(compiler=subject, dependencies=[])
            result.append(p)
        p["provisional_detector"] = {"tool": "LLVM verifier", "replacement": "dedicated Jabberwock detector planned"}
        if target == "linux":
            p = add("undefined-negative", [4], ["cargo", "test", "--test", "sanitizers",
                    "c_runtime_instrumentation_detects_races_and_undefined_behavior", "--", "--exact", "--nocapture"],
                    extra={"JW_CFLAGS": "-fsanitize=undefined -fno-sanitize-recover=undefined"})
            p["required_test_names"] = ["c_runtime_instrumentation_detects_races_and_undefined_behavior"]
            p["evidence_scope"] = "shared C runtime detector, not generated LLVM verification"
        p = add("runtime-retirement", [2,11], ["cargo", "test", "--test", "retirement",
                "native_runtime_retires_without_reusing_identity_or_wrapping_flags", "--", "--exact", "--nocapture"])
        p["required_test_names"] = ["native_runtime_retires_without_reusing_identity_or_wrapping_flags"]
        p["evidence_scope"] = "shared runtime; compiler-generated guards checked separately"
        for level in ("O1", "O2"):
            fuzz = {"JW_FUZZ": str(100 * scale), "JW_FUZZ_SEED": "3000", "JW_CFLAGS": "-fsanitize=address",
                    "JW_FUZZ_DUMP": str(work / (subject + "-fuzz-inputs")), "JW_NATIVE": "o2" if level == "O2" else "1"}
            if subject == "jwc":
                fuzz["JW_FUZZ_JWC"] = str(jwc)
            p = add("fuzz-" + level, [1,2,5,10], ["cargo", "test", "--test", "fuzz", "--", "--nocapture"], extra=fuzz)
            p["required_test_names"] = ["interp_and_executables_agree_on_generated_programs", "typed_attacks_reach_their_checks", "arithmetic_edges_agree_with_checked_execution"]
        edges = [sys.executable, "scripts/retest_safety_edges.py", "--subject", subject, "--compiler", str(rust), "--jwc", str(jwc)]
        add("array-limits", [9], edges + ["--items", "9"], nested=True, count=8)
        if target == "linux":
            add("resources", [8], edges + ["--items", "8"], nested=True, count=1 if subject == "jwc" else 2)
        add("effects", [10,11,12], [sys.executable, "scripts/retest_practical_effects.py", "--subject", subject,
            "--debug-compiler", str(executable(target, "jabberwock", "debug")), "--release-compiler", str(rust),
            "--jwc", str(jwc)], nested=True, count=4 if subject == "jwc" else 12)
        # 10 の合意した基準: spec.md の節ごとの正しい例・拒否される例（tests/spec_coverage.json）
        add("spec-coverage", [10], [sys.executable, "scripts/check_spec_coverage.py", "--subject", subject,
            "--compiler", str(compiler), "--rust", str(rust)], nested=True)
        if subject == "jwc":
            # 01 の合意した基準: 消した検査を Lean の検査器で確かめ直す（検査器は jwc の IR を読む）
            p = add("elision-certificate", [1], [sys.executable, "scripts/elision_check.py", "--jwc", str(jwc), "--build",
                    "--out", str(work / "elision-check")], nested=True)
            p["lake"] = True
            add("bootstrap", [7,12], [sys.executable, "scripts/check_jwc_bootstrap.py", "--jwc", str(jwc)], nested=True, count=7, timeout=10800)
            p = add("retirement", [2,7,11], ["cargo", "test", "--test", "jwc", "jwc_generated_checks_reject_a_matching_retired_generation", "--", "--exact", "--nocapture"])
            p["required_test_names"] = ["jwc_generated_checks_reject_a_matching_retired_generation"]
        else:
            p = add("retirement", [2,11], ["cargo", "test", "--test", "retirement",
                    "rust_generated_checks_reject_a_retired_marker_even_when_generations_match", "--", "--exact", "--nocapture"])
            p["required_test_names"] = ["rust_generated_checks_reject_a_retired_marker_even_when_generations_match"]
            # Rust's role in self-hosting is building the bootstrap compiler, not jwc's execution result.
            add("bootstrap-source", [7], ["cargo", "test", "--test", "jwc", "--", "--nocapture"])
            add("source-regression", [10,11,12], ["cargo", "test", "--no-fail-fast"])
    for p in preparations + result:
        p["target_os"] = target
        p.setdefault("dependencies", [])
        if target == "linux" and any(p["name"].endswith("-" + suffix) for suffix in ("thread", "memory", "undefined", "thread-negative", "memory-negative", "undefined-negative")):
            p["provisional_detector"] = {"tool": "LLVM sanitizer", "replacement": "dedicated Jabberwock detector planned"}
        if target == "windows" and p.get("compiler") and ("address" in p["name"] or "-fuzz-" in p["name"]):
            p["dependencies"].append("windows-asan-preflight")
            p["runtime_environment_from"] = "windows-asan-preflight"
    return preparations, result
