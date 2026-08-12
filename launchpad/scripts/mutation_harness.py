#!/usr/bin/env python3
"""Break the pre-flight on purpose, and require its controls to notice.

    python3 launchpad/scripts/mutation_harness.py

A control that has never been observed failing has not been shown to test
anything. "The suite passes" is the claim this harness exists to refuse. Three
phases, and it exits non-zero if any of them finds a blind spot.

**Mutation.** Each function in TARGETS has its body replaced by a constant and
the whole suite must go RED. A mutant that SURVIVES is a control gap, named in
the output. Each target states the constant it becomes, rather than the harness
inferring one, so the mutation is legible: you can read what the function was
reduced to and judge whether killing it is a fair test. The constants are the
plausible wrong answer a careless implementation would return, not gibberish that
anything would catch.

TARGETS is not every function in the tree — it is every function whose logic a
control claims to check. A reviewer found three of those missing (``Read.ok``,
``RecordError.__init__``, ``_epilogue``); they are in the list now.

**Branch mutation.** Whole-function neutering cannot see a MISSING BRANCH: it
fails everything indiscriminately, which proves the function is called and
nothing about any decision inside it. A reviewer proved the gap by deleting the
org-rulesets 404-to-forbidden case, the 5xx case and the in-band GraphQL errors
block — each left the suite green while both enclosing functions were in TARGETS
and both "killed". This phase disables one branch at a time instead.

**Injection.** Each of a list of forbidden imports is added to a module — at the
top level and inside function bodies — and the no-model check must REFUSE it.
``importlib.import_module`` and ``__import__`` are in that list because they
create no Import node at all, so an import-name scan reports them as absent while
the no-model rule is broken outright.

The files are restored from the originals held in memory, and the harness verifies
the restoration byte-for-byte before it exits — a harness that leaves a mutant in
the tree is worse than no harness at all. It also refuses to start if the suite is
already red, so a survivor can never be a pre-existing failure misread as a
control gap.
"""

from __future__ import annotations

import ast
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO_ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
#: This stage's own controls, named explicitly rather than discovered.
#:
#: `launchpad/scripts/` is a SHARED directory: #126 merged pr_body_check.py and
#: test_pr_body_check.py into it, so plain discovery there runs 156 tests, not the
#: pre-flight's own. That matters in both directions. A regression in a module
#: this branch does not own aborts the harness with "the suite is already
#: failing" and reports it as the pre-flight's; and if one of those foreign
#: controls is red, every pre-flight mutant registers as "killed" while proving
#: nothing at all about the pre-flight.
OWN_TESTS = ["test_preflight_core.py", "test_no_model.py"]

SUITE = [
    sys.executable, "-m", "unittest",
    *(f"{name[:-3]}" for name in OWN_TESTS),
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
    ("preflight_core.py", "Read.ok", "return True"),
    # A weak mutant: it dies on an AttributeError rather than on an assertion.
    # Kept because a reviewer asked why it was absent, and the honest answer is
    # that it belongs in the list with its weakness named.
    ("preflight_core.py", "RecordError.__init__", "self.skips = []"),
    ("preflight_core.py", "_review_gate", "return {'review_required': None, 'review_decision': None, 'review_source_endpoint': 'stubbed'}"),
    # preflight_fetch — the fetch layer and the exit contract
    ("preflight_fetch.py", "_classify_failure", "return (UNREACHABLE, 'stubbed')"),
    ("preflight_fetch.py", "_read", "return Read(name, data={}, endpoint=endpoint)"),
    ("preflight_fetch.py", "fetch_all", "return {}"),
    ("preflight_fetch.py", "main", "return 0"),
    ("preflight_fetch.py", "gh_runner", "return RunResult(0, '{}', '')"),
    ("preflight_fetch.py", "_Parser.error", "raise SystemExit(2)"),
    ("preflight_fetch.py", "build_parser", "return argparse.ArgumentParser()"),
    ("preflight_fetch.py", "_epilogue", "return ''"),
]


#: (module, label, snippet, replacement) — ONE BRANCH, disabled.
#:
#: This phase exists because whole-function mutation cannot see a missing branch.
#: A reviewer proved it: deleting the org_rulesets 404-to-forbidden case, the 5xx
#: case, and the whole in-band GraphQL errors block each left the suite green,
#: while `_read` and `_classify_failure` were both in TARGETS and both "killed".
#: Neutering a function to a constant fails everything indiscriminately, which
#: proves the function is called — not that any decision inside it is checked.
#:
#: A snippet that no longer matches its source is a HARD ERROR, never a skip: a
#: branch mutation that silently fails to apply reports itself as a killed mutant.
BRANCH_MUTATIONS: list[tuple[str, str, str, str]] = [
    (
        "preflight_fetch.py",
        "a 404 on org rulesets stops being forbidden",
        '            if name == "org_rulesets":\n'
        '                return FORBIDDEN, f"{detail} (404 for an organization that exists: no admin:org)"\n',
        "",
    ),
    (
        "preflight_fetch.py",
        "a 5xx stops being unreachable",
        "        if 500 <= code <= 599:\n            return UNREACHABLE, detail\n",
        "",
    ),
    (
        "preflight_fetch.py",
        "401/403 stop being forbidden",
        "        if code in (401, 403):\n            return FORBIDDEN, detail\n",
        "",
    ),
    (
        "preflight_fetch.py",
        "GraphQL prose absence stops being absent",
        '    if _GRAPHQL_ABSENT.search(result.stderr or ""):\n        return ABSENT, detail\n',
        "",
    ),
    (
        "preflight_fetch.py",
        "an in-band errors array is handed on as data",
        '    if isinstance(data, Mapping) and data.get("errors"):',
        "    if False:",
    ),
    (
        "preflight_fetch.py",
        "the errors type mapping collapses to malformed",
        "        for error_type, mapped in GRAPHQL_ERROR_REASONS.items():\n"
        "            if error_type in types:\n"
        "                reason = mapped\n"
        "                break\n",
        "",
    ),
    (
        "preflight_fetch.py",
        "a broken gh install stops being caught",
        "    except OSError as exc:",
        "    except NotImplementedError as exc:",
    ),
    (
        "preflight_core.py",
        "empty becomes fatal again",
        '    if skip_entry["reason"] == EMPTY:\n        return False\n',
        "",
    ),
    (
        "preflight_core.py",
        "a truncated tree stops being a failure",
        '    if _get(tree.data, "truncated") is True:',
        "    if False:",
    ),
    (
        "preflight_core.py",
        "a tree entry's type stops being checked",
        '        if isinstance(path, str) and kind == "blob"',
        "        if isinstance(path, str)",
    ),
    (
        "preflight_core.py",
        "an empty head tree stops being a failure",
        "    if not entries:",
        "    if False:",
    ),
    (
        "preflight_core.py",
        "an unpinned diff stops being a failure",
        "    if not head_sha:",
        "    if False:",
    ),
    (
        "preflight_core.py",
        "an unreadable review decision becomes 'not required'",
        "    if not review_decision.ok:",
        "    if False:",
    ),
    (
        "preflight_core.py",
        "a null review decision stops being distinguishable from a required one",
        '        "review_required": bool(decision),',
        '        "review_required": True,',
    ),
    (
        "preflight_core.py",
        "a malformed rules probe is coerced to empty again",
        "    if not isinstance(branch_rules.data, list):",
        "    if False:",
    ),
]


#: (module, where, line) — an import the no-model check must refuse. `where` is
#: "module" for top-level or a function name for inside a body, because a
#: top-level-only scan passes the second while failing the first.
#:
#: Only test_no_model is run for these, and that is deliberate: httpx, openai and
#: anthropic are not installed here, so injecting them makes every module that
#: IMPORTS preflight_core die at import time. A suite that is red because a module
#: could not load is not evidence that the check works. test_no_model parses the
#: source without importing it, so when it goes red, the check is what went red.
INJECTIONS: list[tuple[str, str, str]] = [
    ("preflight_core.py", "module", "import urllib.request"),
    ("preflight_core.py", "module", "import requests"),
    ("preflight_core.py", "module", "import httpx"),
    ("preflight_core.py", "module", "import openai"),
    ("preflight_core.py", "module", "import anthropic"),
    ("preflight_core.py", "build_diff", "import requests"),
    ("preflight_core.py", "build_diff", "import openai"),
    ("preflight_core.py", "_nearest", "import httpx"),
    # No Import node exists for either of these. Only the call-node rule sees them.
    ("preflight_core.py", "module", 'importlib.import_module("requests")'),
    ("preflight_core.py", "build_diff", 'importlib.import_module("openai")'),
    ("preflight_core.py", "module", '__import__("anthropic")'),
    ("preflight_fetch.py", "module", "import httpx"),
    ("preflight_fetch.py", "gh_runner", "import openai"),
    # The entry point. It was absent from the allowlist for three commits, so an
    # inference call here passed every control; these two are what would have
    # failed.
    ("pr-preflight.py", "module", "import openai"),
    ("pr-preflight.py", "module", 'importlib.import_module("anthropic")'),
]

NO_MODEL_ONLY = [sys.executable, "-m", "unittest", "test_no_model"]


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


def inject(source: str, where: str, line: str) -> str:
    """Insert ``line`` at module level, or as the first statement of a function."""
    tree = ast.parse(source)
    lines = source.splitlines(keepends=True)
    if where == "module":
        last_import = max(
            (node.end_lineno for node in tree.body if isinstance(node, (ast.Import, ast.ImportFrom))),
            default=1,
        )
        return "".join(lines[:last_import]) + line + "\n" + "".join(lines[last_import:])
    node = find_function(tree, where)
    first = node.body[0]
    at = first.lineno - 1
    indent = " " * first.col_offset
    return "".join(lines[:at]) + f"{indent}{line}\n" + "".join(lines[at:])


def run(command: list[str]) -> tuple[bool, str]:
    """Run a test command. Returns (red, last line of output)."""
    env = {**os.environ, "PYTHONDONTWRITEBYTECODE": "1"}
    proc = subprocess.run(command, cwd=HERE, capture_output=True, text=True, env=env)
    tail = (proc.stderr or proc.stdout).strip().splitlines()
    return proc.returncode != 0, tail[-1] if tail else "(no output)"


def suite_is_red() -> tuple[bool, str]:
    return run(SUITE)


def no_model_is_red() -> tuple[bool, str]:
    return run([*NO_MODEL_ONLY])


def main() -> int:
    originals = {
        name: open(os.path.join(HERE, name), encoding="utf-8").read()
        for name in (
            {module for module, _, _ in TARGETS}
            | {module for module, _, _ in INJECTIONS}
            | {module for module, _, _, _ in BRANCH_MUTATIONS}
        )
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

    print("\n--- single branches, disabled one at a time ---\n")
    lived: list[str] = []
    try:
        for module, label, snippet, replacement in BRANCH_MUTATIONS:
            source = originals[module]
            if snippet not in source:
                print(f"  {module}::{label}: SNIPPET NOT FOUND — the harness is stale", file=sys.stderr)
                return 1
            path = os.path.join(HERE, module)
            with open(path, "w", encoding="utf-8") as handle:
                handle.write(source.replace(snippet, replacement, 1))
            went_red, summary = suite_is_red()
            with open(path, "w", encoding="utf-8") as handle:
                handle.write(source)

            verdict = "RED  (branch is checked)" if went_red else "GREEN — BRANCH UNCHECKED"
            print(f"  {module:<20} {label:<52} {verdict}")
            print(f"      {summary}")
            if not went_red:
                lived.append(f"{module}: {label}")
    finally:
        for name, text in originals.items():
            with open(os.path.join(HERE, name), "w", encoding="utf-8") as handle:
                handle.write(text)

    print(f"\n{len(BRANCH_MUTATIONS) - len(lived)}/{len(BRANCH_MUTATIONS)} branches checked")
    if lived:
        print("UNCHECKED — these branches can be deleted and every control still passes:")
        for name in lived:
            print(f"  {name}")
        return 1

    print("\n--- injected imports: the no-model check must refuse each one ---\n")
    accepted: list[str] = []
    try:
        for module, where, line in INJECTIONS:
            path = os.path.join(HERE, module)
            with open(path, "w", encoding="utf-8") as handle:
                handle.write(inject(originals[module], where, line))
            went_red, summary = no_model_is_red()
            with open(path, "w", encoding="utf-8") as handle:
                handle.write(originals[module])

            verdict = "REFUSED" if went_red else "ACCEPTED — CHECK IS BLIND"
            print(f"  {module}::{where:<12} {line:<40} {verdict}")
            print(f"      {summary}")
            if not went_red:
                accepted.append(f"{module}::{where} {line}")
    finally:
        for name, text in originals.items():
            with open(os.path.join(HERE, name), "w", encoding="utf-8") as handle:
                handle.write(text)

    red, summary = suite_is_red()
    print(f"\nrestored again: suite GREEN — {summary}" if not red else f"\nrestored but RED: {summary}")
    if red:
        return 1

    print(f"\n{len(INJECTIONS) - len(accepted)}/{len(INJECTIONS)} injected imports refused")
    if accepted:
        print("ACCEPTED — the no-model check does not see these:")
        for name in accepted:
            print(f"  {name}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
