#!/usr/bin/env python3
"""Kernel-check the actual gen_has_room AST for all uint64 inputs.

This accepts one function contract under an explicit trusted-frontend premise.
It does not accept a whole-runtime/compiler obligation. No test cases run.
The expected proposition is constructed here, never supplied by a report.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

from formal_report import ALLOWED_AXIOMS, proof_sources_identity, source_identity
from runtime_preprocessing_evidence import capture
from sync_runtime import render

ROOT = Path(__file__).resolve().parents[1]
CONTRACT = "jwc:runtime.gen_has_room:uint64-capacity"
ACCEPTED = "CONTRACT_ACCEPTED_ASSUMING_FRONTEND"
THEOREM = "JabberwockExtracted.source_refines_capacity_spec"
MAX_WORD = (1 << 64) - 1
MAX_COUNTER = (1 << 62) - 1


def contract_id(compiler: str) -> str:
    if compiler not in ("jwc", "rust"):
        raise ValueError("unknown compiler for runtime source contract")
    return compiler + ":runtime.gen_has_room:uint64-capacity"


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def regular(path: Path) -> bytes:
    if any(p.is_symlink() for p in (path, *path.parents)) or not path.is_file():
        raise ValueError("missing or symlinked input: " + str(path))
    return path.read_bytes()


def command(argv: list[str], cwd: Path, timeout: int) -> str:
    run = subprocess.run(argv, cwd=cwd, stdin=subprocess.DEVNULL, capture_output=True,
                         text=True, encoding="utf-8", errors="strict", timeout=timeout)
    if run.returncode:
        raise ValueError("command failed: " + (run.stderr or run.stdout)[-2000:])
    return run.stdout


def source_snapshot(compiler: str = "jwc") -> dict:
    contract_id(compiler)
    paths = ("runtime/jw_rt.c", "scripts/runtime_source_contract.py",
             "scripts/runtime_preprocessing_evidence.py", "scripts/sync_runtime.py",
             "scripts/formal_report.py", "scripts/source_contract_acceptance.py")
    if compiler == "jwc":
        paths += ("jwc/runtime.jw",)
    contents = {p: regular(ROOT / p) for p in paths}
    # Normalize line endings as Python's text reader used by sync_runtime does.
    runtime = contents["runtime/jw_rt.c"].decode("utf-8").replace("\r\n", "\n")
    if compiler == "jwc":
        embedded = contents["jwc/runtime.jw"].decode("utf-8").replace("\r\n", "\n")
        if render(runtime) != embedded:
            raise ValueError("jwc embedded runtime differs from audited C runtime")
    return {"files": {p: sha(b) for p, b in contents.items()},
            "compiler": compiler, "compiler_source": source_identity(compiler),
            "proofs": proof_sources_identity()}


def copy_proof_sources(snapshot: dict, destination: Path) -> None:
    """Build our Lean imports from captured source, not a shared .olean cache."""
    for relative, expected in snapshot["proofs"]["files"].items():
        path = Path(relative)
        if path.is_absolute() or ".." in path.parts or path.parts[0] != "formal":
            raise ValueError("proof source outside formal project")
        data = regular(ROOT / path)
        if sha(data) != expected:
            raise ValueError("proof source changed while making isolated project")
        target = destination / Path(*path.parts[1:])
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)


def children(node: dict, count: int) -> list[dict]:
    inner = node.get("inner", [])
    if (not isinstance(inner, list) or len(inner) != count
            or any(not isinstance(x, dict) for x in inner)):
        raise ValueError("unsupported AST arity: " + str(node.get("kind")))
    return inner


def ctype(node: dict) -> str:
    typ = node.get("type", {})
    return typ.get("desugaredQualType", typ.get("qualType", ""))


def uint64_types(macros: str) -> set[str]:
    definitions = dict(re.findall(r"(?m)^#define (\w+) (.+)$", macros))
    if definitions.get("__CHAR_BIT__") != "8":
        raise ValueError("only eight-bit-byte C targets supported")
    if definitions.get("__SIZEOF_INT__") != "4":
        raise ValueError("contract requires a 32-bit C int")
    result = set()
    for name, macro in (("unsigned int", "__SIZEOF_INT__"),
                        ("unsigned long", "__SIZEOF_LONG__"),
                        ("unsigned long long", "__SIZEOF_LONG_LONG__")):
        if definitions.get(macro) == "8":
            result.add(name)
    if not result:
        raise ValueError("no supported unsigned 64-bit C type")
    return result


class Extractor:
    """Closed, typed fragment. Calls, pointers, effects and unknown nodes fail."""
    def __init__(self, word_types: set[str], params: dict[str, str]):
        self.word_types = word_types
        self.params = params
        self.nodes = 0

    def visit(self, node: dict) -> None:
        self.nodes += 1
        if self.nodes > 512:
            raise ValueError("expression exceeds contract extraction limit")

    def literal(self, node: dict) -> int:
        if node.get("kind") != "IntegerLiteral":
            raise ValueError("expected nonnegative integer literal")
        children(node, 0)
        value = node.get("value")
        if not isinstance(value, str) or not re.fullmatch(r"[0-9]+", value):
            raise ValueError("invalid integer literal")
        n = int(value)
        if n > MAX_WORD:
            raise ValueError("literal exceeds uint64")
        # No negative or over-wide source integer may be silently reinterpreted.
        if ctype(node) not in self.word_types | {"int", "unsigned int"}:
            raise ValueError("unsupported literal type")
        if ctype(node) == "int" and n > (1 << 31) - 1:
            raise ValueError("literal exceeds supported C int range")
        if ctype(node) == "unsigned int" and n > (1 << 32) - 1:
            raise ValueError("literal exceeds supported C unsigned int range")
        return n

    def word(self, node: dict):
        self.visit(node)
        if ctype(node) not in self.word_types:
            raise ValueError("word operation is not unsigned 64-bit")
        kind = node.get("kind")
        if kind == "ParenExpr":
            return self.word(children(node, 1)[0])
        if kind in ("ImplicitCastExpr", "CStyleCastExpr"):
            child = children(node, 1)[0]
            cast = node.get("castKind")
            if cast == "LValueToRValue" and child.get("kind") == "DeclRefExpr":
                return self.word(child)
            if cast in ("IntegralCast", "NoOp"):
                if ctype(child) in self.word_types:
                    return self.word(child)
                return ("literal", self.literal(child))
            raise ValueError("unsupported C cast")
        if kind == "IntegerLiteral":
            return ("literal", self.literal(node))
        if kind == "DeclRefExpr":
            children(node, 0)
            ref = node.get("referencedDecl", {})
            identity = ref.get("id")
            if ref.get("kind") != "ParmVarDecl" or identity not in self.params:
                raise ValueError("expression references a nonparameter")
            return (self.params[identity],)
        if kind == "BinaryOperator":
            left, right = children(node, 2)
            op = node.get("opcode")
            if op in ("&", "-"):
                return ("band" if op == "&" else "sub", self.word(left), self.word(right))
            if op == "<<":
                count = self.literal(right)
                if count >= 64:
                    raise ValueError("C shift count outside width")
                return ("shl", self.word(left), count)
        raise ValueError("unsupported word AST: " + str(kind))

    def predicate(self, node: dict):
        self.visit(node)
        if ctype(node) != "int":
            raise ValueError("C predicate must produce int")
        if node.get("kind") == "ParenExpr":
            return self.predicate(children(node, 1)[0])
        if node.get("kind") != "BinaryOperator":
            raise ValueError("unsupported predicate AST")
        left, right = children(node, 2)
        op = node.get("opcode")
        if op == "&&":
            return ("and", self.predicate(left), self.predicate(right))
        if op in ("!=", "<="):
            return ("ne" if op == "!=" else "ule", self.word(left), self.word(right))
        raise ValueError("unsupported predicate operator")


def extract(function: dict, word_types: set[str]):
    if (function.get("kind") != "FunctionDecl" or function.get("name") != "gen_has_room"
            or function.get("storageClass") != "static"
            or function.get("type", {}).get("qualType") != "int (uint64_t, uint64_t)"):
        raise ValueError("unexpected source function signature")
    location = function.get("loc", {})
    if location.get("file") != str(ROOT / "runtime/jw_rt.c"):
        raise ValueError("guard must be defined directly in the audited runtime source")
    gen, step, body = children(function, 3)
    for param, name in ((gen, "gen"), (step, "step")):
        if (param.get("kind") != "ParmVarDecl" or param.get("name") != name
                or ctype(param) not in word_types or not isinstance(param.get("id"), str)):
            raise ValueError("unexpected source parameter")
        children(param, 0)
    if gen["id"] == step["id"] or body.get("kind") != "CompoundStmt":
        raise ValueError("invalid function body")
    statement = children(body, 1)[0]
    if statement.get("kind") != "ReturnStmt":
        raise ValueError("contract requires exactly one return statement")
    return Extractor(word_types, {gen["id"]: "gen", step["id"]: "step"}).predicate(
        children(statement, 1)[0])


def lean_word(tree) -> str:
    op = tree[0]
    if op in ("gen", "step"):
        return "." + op
    if op == "literal":
        return "(.literal (BitVec.ofNat 64 " + str(tree[1]) + "))"
    if op == "shl":
        return "(.shl " + lean_word(tree[1]) + " " + str(tree[2]) + " (by decide))"
    return "(." + op + " " + lean_word(tree[1]) + " " + lean_word(tree[2]) + ")"


def lean_predicate(tree) -> str:
    emit = lean_predicate if tree[0] == "and" else lean_word
    return "(." + tree[0] + " " + emit(tree[1]) + " " + emit(tree[2]) + ")"


def proof_text(tree, limit: int) -> str:
    # Names, operators and numerals come only from the closed typed extractor.
    return """import JabberwockFormal.UnsignedGuardSource
set_option autoImplicit false
set_option maxRecDepth 4096
set_option warningAsError true
namespace JabberwockExtracted
open JabberwockFormal.UnsignedGuardSource
def source : Predicate := """ + lean_predicate(tree) + """
theorem source_refines_capacity_spec (gen step : Word) :
    eval gen step source = true ↔
      JabberwockFormal.RuntimeGenerationBitVec.guardSpec gen step (BitVec.ofNat 64 """ + str(limit) + """) := by
  have mask : ((BitVec.ofNat 64 1) <<< (62 : Nat)) - BitVec.ofNat 64 1 = BitVec.ofNat 64 4611686018427387903 := by decide
  have maskDef : JabberwockFormal.RuntimeGenerationBitVec.counterMask = BitVec.ofNat 64 4611686018427387903 := by decide
  have retiredDef : JabberwockFormal.RuntimeGenerationBitVec.retiredWord = BitVec.ofNat 64 18446744073709551615 := by decide
  simp only [source, eval, evalWord, mask, JabberwockFormal.RuntimeGenerationBitVec.guardSpec,
    maskDef, retiredDef, BitVec.ule_eq_decide_le, Bool.and_eq_true, decide_eq_true_eq, and_assoc]
end JabberwockExtracted
#print axioms JabberwockExtracted.source_refines_capacity_spec
"""


def check(timeout: int = 600, *, compiler: str = "jwc") -> dict:
    if timeout < 1:
        raise ValueError("timeout must be positive")
    before = source_snapshot(compiler)
    # Only local configuration chooses executables. Never execute a path/argv
    # supplied by an uploaded report when replaying its claimed proof.
    clang = shutil.which(os.environ.get("JW_CLANG", "clang"))
    lake = shutil.which("lake")
    if not clang or not lake:
        raise ValueError("configured clang and Lean 4.24.0 lake are required")
    clang_path, lake_path = Path(clang).resolve(), Path(lake).resolve()
    tool_hashes = {"clang": sha(regular(clang_path)), "lake_launcher": sha(regular(lake_path))}
    clang_version = command([clang, "--version"], ROOT, timeout)
    # This contract fixes the default production preprocessing configuration.
    # It does not authenticate an arbitrary user's compiled binary or flags.
    preprocessing = capture(clang, [], timeout)
    if preprocessing["active_generation_limit_branch"] != "production":
        raise ValueError("contract requires production generation limit")
    macros = command([clang, "-dM", "-E", "-x", "c", str(ROOT / "runtime/jw_rt.c")], ROOT, timeout)
    raw_ast = command([clang, "-fsyntax-only", "-Xclang", "-ast-dump=json",
                       "-Xclang", "-ast-dump-filter=gen_has_room",
                       str(ROOT / "runtime/jw_rt.c")], ROOT, timeout)
    parsed = json.loads(raw_ast)
    if not isinstance(parsed, dict):
        raise ValueError("expected one function AST")
    tree = extract(parsed, uint64_types(macros))
    generated = proof_text(tree, MAX_COUNTER)
    with tempfile.TemporaryDirectory(prefix="jw-source-contract-") as temporary:
        project = Path(temporary) / "formal"
        copy_proof_sources(before, project)
        lean_version = command([lake, "env", "lean", "--version"], project, timeout)
        if "version 4.24.0," not in lean_version:
            raise ValueError("Lean 4.24.0 required")
        prefix = command([lake, "env", "lean", "--print-prefix"], project, timeout).strip()
        if not Path(prefix).is_absolute():
            raise ValueError("Lean installation prefix must be absolute")
        lean_path = (Path(prefix) / "bin" / ("lean.exe" if os.name == "nt" else "lean")).resolve()
        tool_hashes["lean"] = sha(regular(lean_path))
        command([lake, "build", "JabberwockFormal.UnsignedGuardSource"], project, timeout)
        file = project / "SourceContract.lean"
        file.write_text(generated, encoding="utf-8")
        output = command([lake, "env", "lean", str(file)], project, timeout)
    match = re.search(re.escape("'" + THEOREM + "'") +
                      r" (?:does not depend on any axioms|depends on axioms:\s*\[([^\]]*)\])", output)
    if match is None:
        raise ValueError("missing kernel dependency audit for extracted-source theorem")
    axioms = sorted(filter(None, (s.strip() for s in (match[1] or "").split(","))))
    if set(axioms) - ALLOWED_AXIOMS:
        raise ValueError("unapproved dependency in extracted-source theorem")
    if source_snapshot(compiler) != before:
        raise ValueError("source, extractor or proof inputs changed during verification")
    after_tools = {"clang": sha(regular(clang_path)), "lake_launcher": sha(regular(lake_path)),
                   "lean": sha(regular(lean_path))}
    if after_tools != tool_hashes:
        raise ValueError("tool executable changed during verification")
    return {"schema_version": 1, "kind": "runtime-source-contract", "compiler": compiler,
            "contract_id": contract_id(compiler), "status": ACCEPTED, "whole_implementation": "NOT_PROVEN",
            "source_snapshot": before, "tool_hashes": tool_hashes,
            "clang_version": clang_version.strip(), "lean_version": lean_version.strip(),
            "preprocessor_configuration": "production-default-flags",
            "proof_build": "FRESH_SOURCE_PROJECT",
            "typed_expression": tree, "generated_proof": generated,
            "generated_proof_sha256": sha(generated.encode()), "theorem": THEOREM,
            "kernel_axioms": axioms,
            "quantification": "all gen, step : uint64_t; GEN_LIMIT = 2^62-1",
            "embedding_check": ("jwc/runtime.jw bytes match sync_runtime.render; emission not proved"
                                if compiler == "jwc" else "C function checked; Rust embedding/emission not proved"),
            "trust_premises": ["Clang preprocessing, parsing and target type layout",
                               "closed typed AST extractor and C expression value semantics",
                               "Lean 4.24.0 kernel and approved dependencies"],
            "excluded": ["arbitrary C or compiler functions", "allocator and pointer lifetime",
                         "atomic memory ordering and concurrency", "LLVM optimization and linking",
                         "binary provenance", "nondefault build flags", "whole-compiler refinement"]}


def replay(report: dict, timeout: int = 600, *, compiler: str = "jwc") -> dict:
    """Re-prove locally; a CONTRACT_ACCEPTED string or digest is not evidence."""
    if (not isinstance(report, dict) or report.get("schema_version") != 1
            or report.get("kind") != "runtime-source-contract"
            or report.get("compiler") != compiler or report.get("contract_id") != contract_id(compiler)
            or report.get("status") != ACCEPTED
            or report.get("whole_implementation") != "NOT_PROVEN"):
        raise ValueError("wrong source contract, compiler, scope or status")
    actual = check(timeout, compiler=compiler)
    # JSON normalizes tuples; compare the full record after that normalization.
    actual = json.loads(json.dumps(actual))
    if report != actual:
        raise ValueError("source contract differs from independently replayed proof")
    return actual


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--timeout", type=int, default=600)
    parser.add_argument("--compiler", choices=("jwc", "rust"), default="jwc")
    args = parser.parse_args(argv)
    if args.timeout < 1:
        parser.error("timeout must be positive")
    try:
        result = check(args.timeout, compiler=args.compiler)
    except (OSError, ValueError, KeyError, TypeError, RecursionError, subprocess.TimeoutExpired) as error:
        result = {"schema_version": 1, "kind": "runtime-source-contract", "compiler": args.compiler,
                  "contract_id": contract_id(args.compiler), "status": "INVALID_EVIDENCE", "reason": str(error),
                  "whole_implementation": "NOT_PROVEN"}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(result["status"], "->", args.output, flush=True)
    return 0 if result["status"] == ACCEPTED else 2


if __name__ == "__main__":
    sys.exit(main())
