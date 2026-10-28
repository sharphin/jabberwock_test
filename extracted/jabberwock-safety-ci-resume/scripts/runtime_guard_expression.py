#!/usr/bin/env python3
"""Independently inspect the narrow C generation-capacity expression.

This is an exact expression-language check, NOT a verified C frontend,
whole-runtime refinement proof, or compiler semantic certification.
"""
from __future__ import annotations

import re
from pathlib import Path

# A deliberately small expression grammar: identifiers, parentheses, &, -,
# !=, <=, and &&. Reject any syntax outside that fragment.
TOKEN = re.compile(r"\s*(GEN_RETIRED|GEN_LIMIT|GEN_COUNTER_MASK|gen|step|&&|!=|<=|[()&-])")
EXPECTED = (
    "&&",
    ("&&", ("!=", "gen", "GEN_RETIRED"),
     ("<=", "step", "GEN_LIMIT")),
    ("<=", ("&", "gen", "GEN_COUNTER_MASK"),
     ("-", "GEN_LIMIT", "step")),
)
PRECEDENCE = {"&&": 1, "!=": 2, "<=": 2, "&": 3, "-": 4}
NAMES = {"gen", "step", "GEN_RETIRED", "GEN_LIMIT", "GEN_COUNTER_MASK"}


def expression_tokens(expression: str) -> list[str]:
    tokens = []
    offset = 0
    while offset < len(expression):
        if not expression[offset:].strip():
            break
        match = TOKEN.match(expression, offset)
        if not match or match.end() == offset:
            raise ValueError("unsupported C expression syntax")
        tokens.append(match.group(1))
        offset = match.end()
    return tokens


def parse_expression(tokens: list[str]):
    position = 0

    def primary():
        nonlocal position
        if position >= len(tokens):
            raise ValueError("incomplete C expression")
        token = tokens[position]
        position += 1
        if token == "(":
            expression = binary(1)
            if position >= len(tokens) or tokens[position] != ")":
                raise ValueError("unclosed C grouping")
            position += 1
            return expression
        if token not in NAMES:
            raise ValueError("unknown identifier in generation guard")
        return token

    def binary(minimum: int):
        nonlocal position
        left = primary()
        while position < len(tokens):
            operator = tokens[position]
            priority = PRECEDENCE.get(operator, 0)
            if priority < minimum:
                break
            position += 1
            right = binary(priority + 1)
            left = (operator, left, right)
        return left

    result = binary(1)
    if position != len(tokens):
        raise ValueError("trailing or unsupported C guard tokens")
    return result



# Whitelisted Lean AST surface for the already kernel-audited BitVec guard.
# The matching is exact modulo whitespace. An arbitrary change to the Lean
# executable guard must not be accepted merely because old theorem names remain.
LEAN_GUARD = (
    "decide (gen ≠ retiredWord) && step.ule limit && "
    "(gen &&& counterMask).ule (limit - step)"
)
LEAN_SPEC = (
    "gen ≠ retiredWord ∧ step ≤ limit ∧ "
    "(gen &&& counterMask) ≤ limit - step"
)


def lean_definition(source: str, declaration: str, arguments: str, result: str) -> str:
    # Exclude the next top-level declaration: do not search for a familiar
    # snippet elsewhere in the file or inside a later theorem.
    pattern = (
        r"(?m)^def\s+" + re.escape(declaration) +
        r"\s*" + arguments + r"\s*:\s*" + result +
        r"\s*:=\s*(.*?)(?=^(?:def|theorem|abbrev|end)\b|\Z)"
    )
    matches = re.findall(pattern, source, re.DOTALL | re.MULTILINE)
    if len(matches) != 1:
        raise ValueError("missing or ambiguous Lean declaration: " + declaration)
    # Ignore Lean comments only if absent, to keep this subset auditable.
    body = matches[0].strip()
    if "--" in body or "/-" in body:
        raise ValueError("comments within reviewed Lean guard are unsupported")
    return " ".join(body.split())


def inspect_lean(lean: Path) -> dict:
    if lean.is_symlink() or not lean.is_file():
        raise ValueError("Lean BitVec source missing or symlinked")
    contents = lean.read_text(encoding="utf-8")
    parameters = r"\(\s*gen\s+step\s+limit\s*:\s*Word\s*\)"
    actual_guard = lean_definition(contents, "guard", parameters, "Bool")
    actual_spec = lean_definition(contents, "guardSpec", parameters, "Prop")
    if actual_guard != LEAN_GUARD or actual_spec != LEAN_SPEC:
        raise ValueError("Lean executable guard or specification changed from reviewed expression")
    return {"status": "LEAN_EXPRESSION_ALIGNED_MODEL_ONLY",
            "scope": "exact BitVec guard and guardSpec expression bodies",
            "implementation_refinement": "NOT_PROVEN"}

# An executable reference semantics for the *restricted* C expression tree.
# This is not a substitute for a validated C frontend or a kernel proof.
UINT64_MASK = (1 << 64) - 1
COUNTER_MASK = (1 << 62) - 1


def eval_unsigned_guard(ast, gen: int, step: int, limit: int) -> bool:
    """Interpret reviewed operators with uint64_t arithmetic and C && semantics."""
    if any(type(word) is not int or not 0 <= word <= UINT64_MASK
           for word in (gen, step, limit)):
        raise ValueError("generation words must be unsigned 64-bit integers")
    symbols = {
        "gen": gen, "step": step, "GEN_RETIRED": UINT64_MASK,
        "GEN_COUNTER_MASK": COUNTER_MASK, "GEN_LIMIT": limit
    }

    def evaluate(node):
        if isinstance(node, str):
            return symbols[node]
        operator, lhs, rhs = node
        if operator == "&&":
            # C short circuits the subtraction when step > limit.
            return int(bool(evaluate(lhs)) and bool(evaluate(rhs)))
        left, right = evaluate(lhs), evaluate(rhs)
        if operator == "!=":
            return int(left != right)
        if operator == "<=":
            return int(left <= right)
        if operator == "&":
            return left & right
        if operator == "-":
            return (left - right) & UINT64_MASK
        raise ValueError("unsupported C guard operator")

    return bool(evaluate(ast))


# Symbolic compiler for the reviewed unsigned expression subset. Unlike
# example-based evaluation, this preserves the operator tree for all inputs.
# This establishes agreement of *defined subset semantics*, not C compiler
# correctness or source-to-Lean refinement.
def symbolic_unsigned(node):
    symbols = {
        "gen": ("word64", "gen"),
        "step": ("word64", "step"),
        "GEN_RETIRED": ("word64", "retiredWord"),
        "GEN_COUNTER_MASK": ("word64", "counterMask"),
        "GEN_LIMIT": ("word64", "limit"),
    }
    if isinstance(node, str):
        if node not in symbols:
            raise ValueError("unknown generation symbol")
        return symbols[node]
    if not isinstance(node, tuple) or len(node) != 3:
        raise ValueError("invalid generation expression")
    op, lhs, rhs = node
    left = symbolic_unsigned(lhs)
    right = symbolic_unsigned(rhs)
    if op == "&&":
        if left[0] != "bool" or right[0] != "bool":
            raise ValueError("C logical AND requires boolean operands in reviewed subset")
        return ("bool", "and", left, right)
    if op in ("!=", "<="):
        if left[0] != "word64" or right[0] != "word64":
            raise ValueError("unsigned comparison type mismatch")
        return ("bool", "ne" if op == "!=" else "ule", left, right)
    if op in ("&", "-"):
        if left[0] != "word64" or right[0] != "word64":
            raise ValueError("unsigned arithmetic type mismatch")
        return ("word64", "band" if op == "&" else "sub64", left, right)
    raise ValueError("unsupported operator")


# Trusted specification of the restricted Lean guard's BitVec operations,
# checked against the actual Lean definition body by inspect_lean().
LEAN_SYMBOLIC_GUARD = (
    "bool", "and",
    ("bool", "and",
     ("bool", "ne", ("word64", "gen"), ("word64", "retiredWord")),
     ("bool", "ule", ("word64", "step"), ("word64", "limit"))),
    ("bool", "ule",
     ("word64", "band", ("word64", "gen"), ("word64", "counterMask")),
     ("word64", "sub64", ("word64", "limit"), ("word64", "step"))),
)


def inspect_generation_macros(source: Path) -> dict:
    """Audit both preprocessing branches; do not guess which build branch ran."""
    if source.is_symlink() or not source.is_file():
        raise ValueError("runtime generation source unavailable")
    text = source.read_text(encoding="utf-8")
    # Reject conflicting redefinitions, not just the absence of one good match.
    declarations = {}
    for macro in ("GEN_RETIRED", "GEN_COUNTER_MASK", "GEN_LIMIT"):
        declarations[macro] = re.findall(
            r"(?m)^\s*#\s*define\s+" + macro + r"\b([^\n]*)", text
        )
    if len(declarations["GEN_RETIRED"]) != 1:
        raise ValueError("ambiguous retired sentinel macro")
    if declarations["GEN_RETIRED"][0].strip() != "UINT64_MAX":
        raise ValueError("retired sentinel differs from 64-bit maximum")
    if len(declarations["GEN_COUNTER_MASK"]) != 1:
        raise ValueError("ambiguous counter mask macro")
    if re.sub(r"\s+", "", declarations["GEN_COUNTER_MASK"][0]) != "((UINT64_C(1)<<62)-1)":
        raise ValueError("counter mask differs from low 62 bits")
    if len(declarations["GEN_LIMIT"]) != 2:
        raise ValueError("generation limit branch definitions missing or ambiguous")
    # Inspect the full active preprocessor block rather than finding
    # disconnected good-looking directives elsewhere in the source.
    block = re.search(
        r"(?m)^#ifdef JW_TEST_GENERATION_LIMIT\s*\n"
        r"(.*?)^#else\s*\n(.*?)^#endif\s*$",
        text, flags=re.DOTALL | re.MULTILINE
    )
    if block is None or len(re.findall(r"(?m)^#ifdef JW_TEST_GENERATION_LIMIT\b", text)) != 1:
        raise ValueError("generation limit preprocessor branches ambiguous")
    testing, production = block.groups()
    if len(re.findall(r"(?m)^#define GEN_LIMIT\s+\(\(uint64_t\)JW_TEST_GENERATION_LIMIT\)\s*$", testing)) != 1:
        raise ValueError("noncanonical overridden generation limit")
    if len(re.findall(r"(?m)^#define GEN_LIMIT\s+GEN_COUNTER_MASK\s*$", production)) != 1:
        raise ValueError("noncanonical production generation limit")
    if (re.search(r"\b_Static_assert\s*\(\s*JW_TEST_GENERATION_LIMIT\s*>=\s*3\s*&&"
                  r"\s*JW_TEST_GENERATION_LIMIT\s*<=\s*GEN_COUNTER_MASK\s*&&"
                  r"\s*\(JW_TEST_GENERATION_LIMIT\s*&\s*3\)\s*==\s*3\s*,", testing) is None):
        raise ValueError("generation override static restriction missing")
    if re.search(r"(?m)^\s*#\s*(?:define|undef|if|ifdef|ifndef|elif|else|endif)\b", testing.replace(
        "#define GEN_LIMIT ((uint64_t)JW_TEST_GENERATION_LIMIT)", ""
    )):
        raise ValueError("unreviewed directive in generation limit override branch")
    if re.search(r"(?m)^\s*#\s*(?:define|undef|if|ifdef|ifndef|elif|else|endif)\b", production.replace(
        "#define GEN_LIMIT GEN_COUNTER_MASK", ""
    )):
        raise ValueError("unreviewed directive in production generation limit branch")
    return {
        "status": "MACRO_BRANCHES_REVIEWED_BUILD_CONFIGURATION_UNKNOWN",
        "production_limit": COUNTER_MASK,
        "override_constraints": "3 <= limit <= 2^62 - 1 and (limit & 3) == 3",
        "active_branch": "UNDETERMINED",
        "implementation_refinement": "NOT_PROVEN",
    }



def inspect_native_build_path(native_source: Path) -> dict:
    """Identify how the shipped runtime is passed to clang, without guessing flags."""
    if native_source.is_symlink() or not native_source.is_file():
        raise ValueError("native compiler source unavailable")
    text = native_source.read_text(encoding="utf-8")
    reviewed = (
        'pub const RUNTIME_C: &str = include_str!("../runtime/jw_rt.c");',
        'std::env::var("JW_CFLAGS")',
        '.args(extra_flags())',
        'fs::write(&rt, RUNTIME_C)',
        'let mut args = vec![opt, path_str(&llp)?, path_str(&rt)?',
        'run_clang(&lto).or_else(|_| run_clang(&args))',
    )
    if any((text.count(fragment) < 1 if fragment == "fs::write(&rt, RUNTIME_C)" else text.count(fragment) != 1) for fragment in reviewed):
        raise ValueError("native compilation path changed from reviewed source")
    # The exact arguments of a particular invocation (especially JW_CFLAGS)
    # are not available from the source of the build function.
    return {
        "status": "BUILD_SOURCE_PATH_MATCHED_FLAGS_UNVERIFIED",
        "runtime_source": "runtime/jw_rt.c",
        "compiler": "JW_CLANG or clang",
        "injected_environment": "JW_CFLAGS",
        "active_preprocessor_branch": "UNDETERMINED",
        "implementation_refinement": "NOT_PROVEN",
        "limitations": [
            "No actual clang invocation or preprocessed runtime was authenticated.",
            "JW_CFLAGS may define JW_TEST_GENERATION_LIMIT or other macros.",
            "A source-level build-path check cannot establish actual compiler flags.",
        ],
    }


def inspect_symbolic_alignment(c_source: Path, lean_source: Path) -> dict:
    # Both source inspections are mandatory; the evaluator alone must not
    # discharge this source-bound obligation.
    c_result = inspect(c_source)
    lean_result = inspect_lean(lean_source)
    if (c_result["status"] != "EXPRESSION_ALIGNED_MODEL_ONLY"
            or lean_result["status"] != "LEAN_EXPRESSION_ALIGNED_MODEL_ONLY"
            or symbolic_unsigned(EXPECTED) != LEAN_SYMBOLIC_GUARD):
        raise ValueError("restricted C and Lean symbolic operator trees differ")
    return {
        "status": "SYMBOLIC_SUBSET_ALIGNED_NOT_PROVEN",
        "operators": ["short_circuit_and", "unsigned_ne", "unsigned_le",
                      "unsigned_and", "unsigned_sub_mod_2_64"],
        "scope": "reviewed gen_has_room guard and Lean BitVec guard expressions",
        "implementation_refinement": "NOT_PROVEN",
        "assumptions": [
            "uint64_t has exactly 64 value bits",
            "GEN_RETIRED, GEN_COUNTER_MASK and GEN_LIMIT expand to reviewed values",
            "C frontend and preprocessing correctly implement the reviewed expression",
            "Lean BitVec operators have the expected 64-bit interpretations",
        ],
    }


def bitvec_guard_spec(gen: int, step: int, limit: int) -> bool:
    """Direct 64-bit interpretation of the reviewed Lean guardSpec formula."""
    if any(type(word) is not int or not 0 <= word <= UINT64_MASK
           for word in (gen, step, limit)):
        raise ValueError("generation words must be unsigned 64-bit integers")
    return (gen != UINT64_MASK and step <= limit
            and (gen & COUNTER_MASK) <= ((limit - step) & UINT64_MASK))


def inspect(source: Path) -> dict:
    if source.is_symlink() or not source.is_file():
        raise ValueError("runtime guard source unavailable or symlinked")
    text = source.read_text(encoding="utf-8")
    # Match one complete function body; no fallback to an arbitrary occurrence
    # of the expected expression elsewhere in the source.
    functions = re.findall(
        r"static\s+int\s+gen_has_room\s*\(\s*uint64_t\s+gen\s*,\s*"
        r"uint64_t\s+step\s*\)\s*\{([^{}]*)\}", text,
        flags=re.DOTALL,
    )
    if len(functions) != 1:
        raise ValueError("missing or ambiguous generation guard function")
    body = functions[0].strip()
    returns = re.fullmatch(r"return\s+([^;]+)\s*;", body, flags=re.DOTALL)
    if not returns:
        raise ValueError("generation guard contains unsupported C statements")
    parsed = parse_expression(expression_tokens(returns.group(1)))
    if parsed != EXPECTED:
        raise ValueError("generation guard differs from the reviewed BitVec guard expression")
    # The two interpreters remain separate representations; expression AST
    # alignment is necessary, not sufficient, for universal semantic proof.
    return {
        "status": "EXPRESSION_ALIGNED_MODEL_ONLY",
        "source": source.as_posix(),
        "scope": "gen_has_room expression grammar and operator grouping",
        "implementation_refinement": "NOT_PROVEN",
        "limitations": [
            "C preprocessing, integer conversion, C compiler semantics and macro expansion are not verified.",
            "Alignment with the Lean BitVec guard is expression-level only.",
            "Pointer lifetimes, atomics, allocation, threading and external behavior remain unproved.",
        ],
    }
