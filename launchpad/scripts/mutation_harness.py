#!/usr/bin/env python3
"""Neuter each check function in turn and require the suite to go red.

    python3 launchpad/scripts/mutation_harness.py

A control that has never been observed failing has not been shown to test
anything. "The suite passes" is the claim this harness exists to refuse: it
replaces one function's body with a constant, runs the whole suite, and expects
RED. A mutant that SURVIVES is a control gap, and the harness exits non-zero
naming it.

Each target names the constant it is neutered to, rather than the harness
inferring one, so the mutation is legible: you can read what the function was
reduced to and judge whether killing it is a fair test.

The files are restored from the originals held in memory, and the harness verifies
the restoration byte-for-byte before it exits — a harness that leaves a mutant in
the tree is worse than no harness at all.
"""

from __future__ import annotations

import ast
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO_ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
SUITE = [
    sys.executable, "-m", "unittest", "discover",
    "-s", "launchpad/scripts", "-t", "launchpad/scripts",
]

#: (module, qualified function name, the constant its body becomes).
#: Every function whose logic a control claims to check is here. The constants are
#: chosen to be the plausible wrong answer — the value a careless implementation
#: would return — not gibberish that anything would catch.
TARGETS: list[tuple[str, str, str]] = [
    # preflight_core — the record
    ("preflight_core.py", "build_pr", "return None"),
    ("preflight_core.py", "build_closing_issue", "return {'present': True, 'keyword': None, 'issue_numbers': [], 'source': None, 'text_disagrees': False, 'text_issue_numbers': []}"),
    ("preflight_core.py", "build_diff", "return {'merge_base_sha': 'x', 'head_sha': 'x', 'files': []}"),
    ("preflight_core.py", "build_checks", "return []"),
    ("preflight_core.py", "build_required_gate", "return {'configured': False, 'source_endpoint': 'assumed'}"),
    ("preflight_core.py", "build_nearest_rules", "return {}"),
    ("preflight_core.py", "build_record", "return dict.fromkeys(RECORD_FIELDS)"),
    # preflight_core — the rules that decide what an absence means
    ("preflight_core.py", "_nearest", "return None"),
    ("preflight_core.py", "_normalise_check", "return {'name': None, 'workflow': None, 'status': None, 'conclusion': None, 'required': None, 'details_url': None}"),
    ("preflight_core.py", "is_fatal", "return False"),
    ("preflight_core.py", "Read.__post_init__", "return None"),
    ("preflight_core.py", "Skips.add", "return None"),
    ("preflight_core.py", "_get", "return None"),
    # preflight_fetch — the fetch layer and the exit contract
    ("preflight_fetch.py", "_classify_failure", "return (UNREACHABLE, 'stubbed')"),
    ("preflight_fetch.py", "_read", "return Read(name, data={}, endpoint=endpoint)"),
    ("preflight_fetch.py", "fetch_all", "return {}"),
    ("preflight_fetch.py", "main", "return 0"),
    ("preflight_fetch.py", "gh_runner", "return RunResult(0, '{}', '')"),
    ("preflight_fetch.py", "_Parser.error", "raise SystemExit(2)"),
    ("preflight_fetch.py", "build_parser", "return argparse.ArgumentParser()"),
]


def find_function(tree: ast.Module, qualified: str) -> ast.FunctionDef:
    """Locate a top-level function, or one method inside one class."""
    if "." in qualified:
        class_name, _, method = qualified.partition(".")
        for node in tree.body:
            if isinstance(node, ast.ClassDef) and node.name == class_name:
                for child in node.body:
                    if isinstance(child, ast.FunctionDef) and child.name == method:
                        return child
        raise LookupError(qualified)
    for node in ast.walk(tree):
        if isinstance(node, ast.FunctionDef) and node.name == qualified.split(".")[-1]:
            return node
    raise LookupError(qualified)


def neuter(source: str, qualified: str, constant: str) -> str:
    """Replace one function's body with ``constant``, keeping its signature."""
    node = find_function(ast.parse(source), qualified)
    lines = source.splitlines(keepends=True)
    first_body = node.body[0]
    # The body starts at the first statement; the signature above it is untouched,
    # so the call sites and the decorators still typecheck and still import.
    start = first_body.lineno - 1
    end = node.end_lineno
    indent = " " * first_body.col_offset
    mutant = f"{indent}{constant}\n"
    return "".join(lines[:start]) + mutant + "".join(lines[end:])


def suite_is_red() -> tuple[bool, str]:
    env = {**os.environ, "PYTHONDONTWRITEBYTECODE": "1"}
    proc = subprocess.run(SUITE, cwd=REPO_ROOT, capture_output=True, text=True, env=env)
    tail = (proc.stderr or proc.stdout).strip().splitlines()
    return proc.returncode != 0, tail[-1] if tail else "(no output)"


def main() -> int:
    originals = {
        name: open(os.path.join(HERE, name), encoding="utf-8").read()
        for name in {module for module, _, _ in TARGETS}
    }

    red, summary = suite_is_red()
    if red:
        print("the suite is already failing; fix that before mutating anything", file=sys.stderr)
        return 1
    print(f"baseline: suite GREEN — {summary}\n")

    survivors: list[str] = []
    try:
        for module, qualified, constant in TARGETS:
            path = os.path.join(HERE, module)
            with open(path, "w", encoding="utf-8") as handle:
                handle.write(neuter(originals[module], qualified, constant))
            went_red, summary = suite_is_red()
            with open(path, "w", encoding="utf-8") as handle:
                handle.write(originals[module])

            verdict = "RED  (control works)" if went_red else "GREEN — SURVIVED"
            print(f"  {module}::{qualified:<26} -> {constant[:44]:<46} {verdict}")
            print(f"      {summary}")
            if not went_red:
                survivors.append(f"{module}::{qualified}")
    finally:
        for name, text in originals.items():
            with open(os.path.join(HERE, name), "w", encoding="utf-8") as handle:
                handle.write(text)
        for name, text in originals.items():
            restored = open(os.path.join(HERE, name), encoding="utf-8").read()
            if restored != text:  # pragma: no cover - a restore that did not restore
                print(f"RESTORE FAILED for {name}", file=sys.stderr)
                return 1

    red, summary = suite_is_red()
    print(f"\nrestored: suite GREEN — {summary}" if not red else f"\nrestored but RED: {summary}")
    if red:
        return 1

    print(f"\n{len(TARGETS) - len(survivors)}/{len(TARGETS)} mutants killed")
    if survivors:
        print("SURVIVED — these functions can be replaced by a constant and every control still passes:")
        for name in survivors:
            print(f"  {name}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
