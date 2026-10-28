"""Independent Windows-12 and Linux-12 probe catalogs; results never cross OSes."""
from pathlib import Path
import sys

import local_safety as safety

ROOT = Path(__file__).resolve().parents[1]
TARGETS = ("windows", "linux")
WINDOWS_ASAN = "-fsanitize=address -fno-omit-frame-pointer"


def host_os():
    if sys.platform == "win32":
        return "windows"
    if sys.platform.startswith("linux"):
        return "linux"
    return None


def executable(target, name, profile="release"):
    return ROOT / "target" / profile / (name + (".exe" if target == "windows" else ""))


def missing(name, items, reason):
    result = safety.probe(name, items, [])
    result["unavailable_reason"] = reason
    return result


def detailed(name, items, command, output, expected_cases, **kwargs):
    result = safety.probe(name, items, [*command, "--output", str(output)], **kwargs)
    result["case_report"] = str(output)
    result["expected_cases"] = expected_cases
    return result


def targeted(target, output):
    compiler = executable(target, "jabberwock")
    jwc = executable(target, "jwc-local")
    edges = [sys.executable, "scripts/retest_safety_edges.py",
             "--compiler", str(compiler), "--jwc", str(jwc)]
    result = [detailed("array-limits", [9], [*edges, "--items", "9"],
                       output / "array-limits.json", 16, clang=True, jwc=True)]
    if target == "linux":
        result.append(detailed("resource-exhaustion", [8], [*edges, "--items", "8"],
                               output / "resource-exhaustion.json", 3,
                               linux=True, clang=True, jwc=True))
    else:
        result.append(missing("windows-resource-exhaustion", [8],
                              "Windows memory-limit/failure-injection probe is not implemented"))
    result.append(detailed("effects-10-11-12", [10, 11, 12],
                           [sys.executable, "scripts/retest_practical_effects.py",
                            "--debug-compiler", str(executable(target, "jabberwock", "debug")),
                            "--release-compiler", str(compiler), "--jwc", str(jwc)],
                           output / "practical-effects.json", 16, clang=True, jwc=True))
    return result


def catalog(target, scale, output):
    if target not in TARGETS:
        raise ValueError("unsupported safety platform: " + str(target))
    old = safety.catalog(scale, output)
    if target == "linux":
        result = [p for p in old if p["name"] not in ("edges-8-9", "effects-10-11-12")]
    else:
        common = {"cargo-build", "cargo-test", "native-differential", "retirement",
                  "formal-model", "jwc-build", "release-test"}
        result = [p for p in old if p["name"] in common]
        result.extend([
            safety.probe("windows-native-o2", [1, 2, 4, 5, 7, 10, 11, 12],
                         ["cargo", "test", "--no-fail-fast"], env={"JW_NATIVE": "o2"}, clang=True),
            safety.probe("windows-threads", [3, 11, 12],
                         ["cargo", "test", "--test", "threads", "--test", "native",
                          "--test", "system", "--test", "retirement", "--no-fail-fast"],
                         env={"JW_NATIVE": "1"}, clang=True),
            safety.probe("windows-robust", [2, 4, 11],
                         ["cargo", "test", "--test", "robust", "--", "--nocapture"],
                         env={"JW_ROBUST": str(150 * scale), "JW_ROBUST_SEED": "1000",
                              "JW_ROBUST_NATIVE": "1", "JW_ROBUST_DUMP": str(output / "robust-inputs")},
                         clang=True),
            safety.probe("windows-sanitizer-negative-asan", [2, 5],
                         ["cargo", "test", "--test", "sanitizers", "--", "--nocapture"],
                         env={"JW_CFLAGS": WINDOWS_ASAN}, clang=True),
            safety.probe("windows-native-all-asan", [2, 5, 7, 11, 12],
                         ["cargo", "test", "--no-fail-fast"],
                         env={"JW_NATIVE": "1", "JW_CFLAGS": WINDOWS_ASAN}, clang=True),
            missing("windows-race-detector", [3],
                    "Windows race-detector probe is not implemented; thread stress alone is insufficient"),
            missing("windows-undefined-behavior-detector", [4],
                    "Windows C-runtime undefined-behavior detector and negative probe are not implemented"),
            missing("windows-uninitialized-memory-detector", [6],
                    "Windows uninitialized-memory detector probe is not implemented"),
        ])
        for jwc in (False, True):
            for o2 in (False, True):
                label = ("jwc" if jwc else "rust") + ("-o2" if o2 else "-o1")
                environment = {"JW_FUZZ": str((100 if o2 else 200) * scale),
                               "JW_FUZZ_SEED": str((3000 if jwc else 2000) + (500 if o2 else 0)),
                               "JW_FUZZ_DUMP": str(output / "fuzz-inputs"),
                               "JW_CFLAGS": WINDOWS_ASAN}
                if o2:
                    environment["JW_NATIVE"] = "o2"
                if jwc:
                    environment["JW_FUZZ_JWC"] = str(executable(target, "jwc-local"))
                result.append(safety.probe("windows-fuzz-asan-" + label, [1, 2, 5] + ([7] if jwc else []),
                                           ["cargo", "test", "--test", "fuzz", "--", "--nocapture"],
                                           env=environment, clang=True, jwc=jwc))
    # Explicit paths make even a dry-run of the other OS's plan unambiguous.
    for p in result:
        if p["name"] == "jwc-build":
            p["cmd"] = [str(executable(target, "jabberwock")), "build", "-o",
                        str(executable(target, "jwc-local")), "jwc/main.jw"]
        if "JW_FUZZ_JWC" in p["env"]:
            p["env"]["JW_FUZZ_JWC"] = str(executable(target, "jwc-local"))
    result.extend(targeted(target, output))
    for p in result:
        p["target_os"] = target
        if p["jwc"]:
            p["jwc_executable"] = str(executable(target, "jwc-local"))
    return result
