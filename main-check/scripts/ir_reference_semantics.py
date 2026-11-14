#!/usr/bin/env python3
"""Executable reference semantics for the core IR ownership operations.

This is a bounded *semantic oracle*, not a claim that every Jabberwock IR
instruction is modeled. It deliberately fails on unsupported operations.
Each pointer carries an allocation ID and generation; retirement invalidates
aliases even when an allocator reuses the same address.
"""
from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any


class IRTrap(Exception):
    pass


@dataclass(frozen=True)
class Ref:
    allocation: int
    generation: int


@dataclass
class Cell:
    generation: int = 0
    value: Any = None
    live: bool = True


@dataclass
class Machine:
    locals: dict[int, Any] = field(default_factory=dict)
    heap: dict[int, Cell] = field(default_factory=dict)
    next_allocation: int = 1
    outputs: list[Any] = field(default_factory=list)

    def allocate(self, value: Any) -> Ref:
        identity = self.next_allocation
        self.next_allocation += 1
        self.heap[identity] = Cell(value=value)
        return Ref(identity, 0)

    def dereference(self, ref: Ref, *, checked: bool = True) -> Any:
        if not isinstance(ref, Ref):
            raise IRTrap("dereference of non-reference")
        cell = self.heap.get(ref.allocation)
        if cell is None or not cell.live:
            raise IRTrap("dangling reference")
        # The oracle never trusts elision from its input; a caller must supply
        # independently certified liveness before removing this check.
        if cell.generation != ref.generation:
            raise IRTrap("stale generation")
        return cell.value

    def retire(self, ref: Ref) -> None:
        cell = self.heap.get(ref.allocation)
        if cell is None or not cell.live or cell.generation != ref.generation:
            raise IRTrap("double retirement or invalid owner")
        cell.live = False
        cell.generation += 1
        cell.value = None

    def _local(self, number: int) -> Any:
        if number not in self.locals:
            raise IRTrap("use of moved or uninitialized local")
        return self.locals[number]

    def read(self, operand: dict) -> Any:
        kind = operand.get("kind")
        if kind == "const":
            return operand["value"]
        if kind == "copy":
            return self._local(operand["local"])
        if kind in ("move", "read"):
            number = operand["local"]
            value = self._local(number)
            del self.locals[number]
            return value
        if kind == "deref":
            return self.dereference(self.read(operand["ref"]))
        raise IRTrap("unsupported operand: " + str(kind))

    def evaluate(self, value: dict) -> Any:
        """Evaluate a bounded set of IR rvalues in explicit left-to-right order."""
        kind = value.get("kind")
        if kind in ("const", "copy", "move", "read", "deref"):
            return self.read(value)
        if kind == "bin":
            left = self.evaluate(value["left"])
            right = self.evaluate(value["right"])
            op = value["op"]
            if op in ("add", "sub", "mul", "div", "mod"):
                if type(left) is not int or type(right) is not int:
                    raise IRTrap("arithmetic on non-integers")
                if op in ("div", "mod") and right == 0:
                    raise IRTrap("division by zero")
                if op == "add":
                    return left + right
                if op == "sub":
                    return left - right
                if op == "mul":
                    return left * right
                if op == "div":
                    return (abs(left) // abs(right)) * (-1 if (left < 0) != (right < 0) else 1)
                quotient = (abs(left) // abs(right)) * (-1 if (left < 0) != (right < 0) else 1)
                return left - quotient * right
            if op in ("eq", "ne", "lt", "le", "gt", "ge"):
                if type(left) is not type(right):
                    raise IRTrap("comparison of different types")
                if op == "eq":
                    return left == right
                if op == "ne":
                    return left != right
                if op == "lt":
                    return left < right
                if op == "le":
                    return left <= right
                if op == "gt":
                    return left > right
                return left >= right
            if op in ("and", "or") and type(left) is bool and type(right) is bool:
                return left and right if op == "and" else left or right
            raise IRTrap("unsupported binary operator: " + str(op))
        if kind == "un":
            operand = self.evaluate(value["operand"])
            if value["op"] == "neg" and type(operand) is int:
                return -operand
            if value["op"] == "not" and type(operand) is bool:
                return not operand
            raise IRTrap("unsupported unary operation")
        if kind == "array":
            return [self.evaluate(item) for item in value["items"]]
        if kind == "struct":
            names = value["fields"]
            if not isinstance(names, dict):
                raise IRTrap("malformed struct fields")
            return {name: self.evaluate(item) for name, item in names.items()}
        if kind == "index":
            container = self.evaluate(value["base"])
            index = self.evaluate(value["index"])
            if type(index) is not int or not isinstance(container, (list, tuple)):
                raise IRTrap("invalid indexed access")
            if index < 0 or index >= len(container):
                raise IRTrap("out-of-bounds access")
            return container[index]
        raise IRTrap("unsupported rvalue: " + str(kind))

    def execute(self, instruction: dict) -> Any:
        kind = instruction.get("kind")
        if kind == "assign":
            value = self.evaluate(instruction["value"])
            number = instruction["local"]
            if number in self.locals and isinstance(self.locals[number], Ref):
                self.retire(self.locals[number])
            self.locals[number] = value
        elif kind == "own":
            self.locals[instruction["local"]] = self.allocate(
                self.read(instruction["value"]))
        elif kind == "borrow":
            value = self._local(instruction["owner"])
            if not isinstance(value, Ref):
                raise IRTrap("borrow without an owner")
            self.locals[instruction["local"]] = value
        elif kind == "drop":
            value = self.locals.pop(instruction["local"], None)
            if isinstance(value, Ref) and instruction.get("owns", False):
                self.retire(value)
        elif kind == "discard":
            value = self.read(instruction["value"])
            if instruction.get("owns", False) and isinstance(value, Ref):
                self.retire(value)
        elif kind == "show":
            self.outputs.append(self.read(instruction["value"]))
        else:
            raise IRTrap("unsupported instruction: " + str(kind))

    def run(self, instructions: list[dict]) -> dict:
        for index, instruction in enumerate(instructions):
            try:
                self.execute(instruction)
            except (IRTrap, KeyError, TypeError) as exc:
                return {"status": "TRAP", "at": index, "reason": str(exc),
                        "outputs": self.outputs}
        return {"status": "OK", "outputs": self.outputs}


    def run_cfg(self, blocks: list[dict], *, entry: int = 0,
                max_steps: int = 100000) -> dict:
        """Run CFG with explicit fail-closed block transitions and a fuel bound."""
        if not isinstance(blocks, list) or not blocks:
            raise ValueError("missing IR basic blocks")
        if not isinstance(max_steps, int) or max_steps <= 0:
            raise ValueError("invalid step budget")
        pc = entry
        steps = 0
        while True:
            if type(pc) is not int or not (0 <= pc < len(blocks)):
                return {"status": "TRAP", "reason": "invalid block target", "outputs": self.outputs}
            block = blocks[pc]
            if not isinstance(block, dict) or not isinstance(block.get("insts"), list):
                return {"status": "TRAP", "reason": "malformed block", "outputs": self.outputs}
            try:
                for instruction in block["insts"]:
                    if steps >= max_steps:
                        raise IRTrap("execution fuel exhausted")
                    steps += 1
                    self.execute(instruction)
                if steps >= max_steps:
                    raise IRTrap("execution fuel exhausted")
                steps += 1
                term = block["term"]
                kind = term["kind"]
                if kind == "goto":
                    pc = term["target"]
                elif kind == "branch":
                    condition = self.read(term["condition"])
                    if type(condition) is not bool:
                        raise IRTrap("branch condition is not Boolean")
                    pc = term["yes"] if condition else term["no"]
                elif kind == "switch":
                    value = self.read(term["value"])
                    pc = term["cases"].get(str(value), term["default"])
                elif kind == "step":
                    counter = term["counter"]
                    previous = self._local(counter)
                    delta = self.read(term["amount"])
                    if type(previous) is not int or type(delta) is not int:
                        raise IRTrap("non-integer step")
                    total = previous + delta
                    if term["min"] <= total <= term["max"]:
                        self.locals[counter] = total
                        pc = term["next"]
                    else:
                        pc = term["done"]
                elif kind == "return":
                    value = self.read(term["value"])
                    return {"status": "RETURN", "value": value, "outputs": self.outputs}
                elif kind == "fail":
                    raise IRTrap("IR fail: " + str(term.get("message", "")))
                elif kind == "unreachable":
                    raise IRTrap("reached unreachable block")
                else:
                    raise IRTrap("unsupported terminator: " + str(kind))
            except (IRTrap, KeyError, TypeError, ValueError, IndexError) as exc:
                return {"status": "TRAP", "block": pc, "reason": str(exc), "outputs": self.outputs}



def interpret(document: dict) -> dict:
    if document.get("schema_version") != 1 or not isinstance(document.get("instructions"), list):
        raise ValueError("invalid reference IR program")
    return Machine().run(document["instructions"])
