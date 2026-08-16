"""The no-model property, asserted by AST — launchpad-26/buzz#116.

The issue requires that the pre-flight make no model call. This module proves it
by parsing the source and comparing every imported module name against an
allowlist **written here**, so any new import fails by default rather than only
the handful someone thought to name in a pattern.

Why this is not a grep
======================

A grep reports every occurrence and leaves a human to sort the legitimate hits
from the real ones. That is not a check that can fail. An allowlist inverts the
burden: the check goes red on an import nobody anticipated.

Three ways an import evades a naive scan, all closed here
=========================================================

**Traverse with ``ast.walk``, not ``tree.body``.** ``tree.body`` sees only
top-level statements, so an ``import requests`` inside a function body is
invisible to it. :meth:`Traversal.test_a_top_level_scan_misses_a_nested_import`
demonstrates the difference on a sample rather than asserting it.

**Collect ``Call`` nodes too.** ``importlib.import_module("openai")`` is a Call,
never an Import — an import-name scan reports it as absent. One line, and the
no-model rule is broken outright while every import-based check stays green.

**``__import__("openai")`` needs no import statement at all**, so only the
call-node rule can see it.

This module deliberately **does not import** the modules it checks. It reads
their source and parses it. An import-hygiene check that imports its subject
cannot report on a subject whose new import is not installed: it dies at import
time, and a suite that is red because a module could not load is not evidence
that this check works. Keeping it source-only is what lets
``mutation_harness.py`` inject ``import openai`` and show *this* control turning
red.
"""

from __future__ import annotations

import ast
import os
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))

#: Written here, not inferred. Anything else fails the check by default.
ALLOWLIST: dict[str, frozenset[str]] = {
    # The record. No I/O of any kind: no subprocess, no filesystem, no network.
    "preflight_core.py": frozenset({"__future__", "re", "dataclasses", "typing"}),
    # The fetch layer. subprocess is here and nowhere else, and it may only spawn gh.
    "preflight_fetch.py": frozenset(
        {
            "__future__",
            "argparse",
            "json",
            "re",
            "subprocess",
            "sys",
            "dataclasses",
            "typing",
            "preflight_core",
        }
    ),
    # The entry point — the file the issue calls "the script", and the first one a
    # reader opens. It was missing from this list for three commits: an inference
    # call placed here left every control green, on precisely the file anyone
    # checking the no-model claim would look at.
    "pr-preflight.py": frozenset({"__future__", "os", "sys", "preflight_fetch"}),
    # ADR-0005's boundary check. Reads tracked files for upstream-namespace
    # leftovers; no subprocess, no network.
    "adr_boundary_check.py": frozenset({"re", "sys", "pathlib"}),
}


#: Files in this directory that belong to another task, named one by one with the
#: reason. Everything else is ours and must be on the allowlist.
NOT_OURS = {
    "pr_body_check.py": "the PR-body check from #126; its imports are that task's business",
    "mutation_harness.py": "the evidence generator — it rewrites source by design, and "
    "its own imports are not part of the no-model claim",
}


def preflight_sources() -> set[str]:
    """Every file this stage owns, by EXCLUSION rather than by name shape.

    The allowlist above is deliberately written by hand — that is what makes an
    unanticipated import fail — but *which files it must cover* cannot also be
    hand-written, or a new module simply never gets checked. That is how
    ``pr-preflight.py`` went unguarded.

    An earlier version of this function matched ``name.startswith("preflight")``,
    which is a hand-written list wearing a filename prefix: a module this stage
    owns called ``gate_reader.py`` would not be discovered, would not be on the
    allowlist, and the control asserting full coverage would stay green while its
    imports went unchecked — the same failure, one level up. So the rule is
    inverted. A new file is ours until someone says otherwise **here**, with a
    reason, and an unlisted new module fails closed whatever it is called.
    """
    return {
        name
        for name in os.listdir(HERE)
        if name.endswith(".py") and not name.startswith("test_") and name not in NOT_OURS
    }

#: Calls that load code by name, bypassing every import statement.
FORBIDDEN_CALLS = frozenset({"importlib.import_module", "__import__", "eval", "exec"})


def source_of(name: str) -> str:
    with open(os.path.join(HERE, name), encoding="utf-8") as handle:
        return handle.read()


def dotted_name(node: ast.AST) -> str:
    """`a.b.c` for an Attribute/Name chain, so a call can be named in full."""
    parts: list[str] = []
    current = node
    while isinstance(current, ast.Attribute):
        parts.append(current.attr)
        current = current.value
    if isinstance(current, ast.Name):
        parts.append(current.id)
    return ".".join(reversed(parts))


def imported_modules(source: str) -> set[str]:
    """Every module name imported anywhere in ``source``, nesting included."""
    found: set[str] = set()
    for node in ast.walk(ast.parse(source)):  # walk, never tree.body
        if isinstance(node, ast.Import):
            for alias in node.names:
                found.add(alias.name.split(".")[0])
        elif isinstance(node, ast.ImportFrom):
            if node.level:  # a relative import names no external module
                continue
            if node.module:
                found.add(node.module.split(".")[0])
    return found


def loader_calls(source: str) -> set[str]:
    """Every call that loads code by name rather than by import statement."""
    found: set[str] = set()
    for node in ast.walk(ast.parse(source)):
        if isinstance(node, ast.Call):
            name = dotted_name(node.func)
            if name in FORBIDDEN_CALLS or name.split(".")[-1] in {"import_module"}:
                found.add(name)
    return found


class NoModelCall(unittest.TestCase):
    def test_every_file_this_stage_owns_is_on_the_allowlist(self):
        """A new pre-flight module must fail this, not silently skip the check.

        The entry point was absent from ALLOWLIST for three commits while the
        suite reported the no-model property as proved. Enumerating the directory
        is what makes a missing entry a failure instead of a blind spot.
        """
        found = preflight_sources()
        self.assertIn("pr-preflight.py", found, "the entry point must be discovered")
        self.assertEqual(
            found,
            set(ALLOWLIST),
            f"unchecked: {sorted(found - set(ALLOWLIST))}; "
            f"listed but absent from disk: {sorted(set(ALLOWLIST) - found)}",
        )

    def test_the_scope_excludes_another_task_s_script(self):
        """#126's pr_body_check.py shares this directory and is not ours to police."""
        self.assertTrue(
            os.path.exists(os.path.join(HERE, "pr_body_check.py")),
            "this control is only meaningful while that file is really here",
        )
        self.assertNotIn("pr_body_check.py", preflight_sources())
        for name, reason in NOT_OURS.items():
            with self.subTest(excluded=name):
                self.assertTrue(reason, "an exclusion without a stated reason is a hole")

    def test_a_new_module_fails_closed_whatever_it_is_called(self):
        """The prefix filter this replaced would have missed `gate_reader.py`.

        Discovery is by exclusion now, so a file nobody anticipated is ours by
        default and its absence from ALLOWLIST is a failure, not a silence.
        """
        invented = os.path.join(HERE, "gate_reader.py")
        self.assertFalse(os.path.exists(invented), "pick a name that is not real")
        try:
            with open(invented, "w", encoding="utf-8") as handle:
                handle.write("import openai\n")
            self.assertIn("gate_reader.py", preflight_sources(), "a new module must be discovered")
            self.assertNotIn("gate_reader.py", ALLOWLIST, "and it is not on the allowlist")
            with self.assertRaises(AssertionError):
                NoModelCall("test_every_file_this_stage_owns_is_on_the_allowlist").test_every_file_this_stage_owns_is_on_the_allowlist()
        finally:
            os.remove(invented)
        self.assertNotIn("gate_reader.py", preflight_sources())

    def test_every_import_is_on_the_allowlist(self):
        for name, allowed in ALLOWLIST.items():
            with self.subTest(module=name):
                found = imported_modules(source_of(name))
                self.assertLessEqual(
                    found,
                    allowed,
                    f"{name} imports {sorted(found - allowed)}, which is not on the allowlist. "
                    "An import this check did not anticipate fails it by default — that is the point. "
                    "If the import is legitimate, add it here and say why in the commit.",
                )

    def test_the_entry_point_only_wires_the_cli_up(self):
        """It may touch sys.path; it may not fetch, decide, or call a model."""
        source = source_of("pr-preflight.py")
        found = imported_modules(source)
        self.assertEqual(found, ALLOWLIST["pr-preflight.py"] & found)
        for banned in ("subprocess", "socket", "http", "urllib", "requests", "httpx", "openai"):
            self.assertNotIn(banned, found)
        self.assertEqual(loader_calls(source), set())

    def test_the_record_module_performs_no_io_at_all(self):
        """No subprocess, no socket, no filesystem: the record is pure."""
        found = imported_modules(source_of("preflight_core.py"))
        for banned in ("subprocess", "socket", "os", "pathlib", "http", "urllib", "requests", "httpx"):
            self.assertNotIn(banned, found)

    def test_no_module_loads_code_by_name(self):
        for name in ALLOWLIST:
            with self.subTest(module=name):
                self.assertEqual(
                    loader_calls(source_of(name)),
                    set(),
                    "importlib.import_module or __import__ bypasses every import-name scan",
                )

    def test_no_model_sdk_is_reachable_from_either_module(self):
        for name in ALLOWLIST:
            with self.subTest(module=name):
                found = imported_modules(source_of(name))
                for sdk in ("openai", "anthropic", "google", "cohere", "mistralai", "litellm", "ollama"):
                    self.assertNotIn(sdk, found)

    def test_only_the_fetch_layer_may_spawn_anything(self):
        """One spawn site, inside gh_runner, and it refuses any binary but gh.

        Counted as call nodes rather than as text. A string count over the source
        reads this module's own prose about ``subprocess.run`` as a second spawn
        site — which is the grep-shaped failure this whole file argues against,
        and it happened here first.
        """
        self.assertIn("subprocess", ALLOWLIST["preflight_fetch.py"])
        self.assertNotIn("subprocess", ALLOWLIST["preflight_core.py"])

        source = source_of("preflight_fetch.py")
        tree = ast.parse(source)
        spawns = [
            node
            for node in ast.walk(tree)
            if isinstance(node, ast.Call) and dotted_name(node.func) == "subprocess.run"
        ]
        self.assertEqual(len(spawns), 1, "exactly one place in the tree starts a process")

        runner = next(
            node
            for node in ast.walk(tree)
            if isinstance(node, ast.FunctionDef) and node.name == "gh_runner"
        )
        self.assertTrue(
            runner.lineno <= spawns[0].lineno <= runner.end_lineno,
            "the spawn must be inside gh_runner, which is the seam controls replace",
        )
        self.assertIn('BINARY = "gh"', source)
        self.assertIn("argv[0] != BINARY", source, "the spawn site must refuse any other binary")


class Traversal(unittest.TestCase):
    """The scan's own blind spots, demonstrated rather than asserted."""

    NESTED = (
        "import json\n"
        "def fetch():\n"
        "    import requests\n"
        "    return requests\n"
    )
    BY_NAME = 'value = importlib.import_module("openai")\n'
    DUNDER = 'value = __import__("openai")\n'

    def test_a_top_level_scan_misses_a_nested_import(self):
        top_level = {
            alias.name
            for node in ast.parse(self.NESTED).body
            if isinstance(node, ast.Import)
            for alias in node.names
        }
        self.assertEqual(top_level, {"json"}, "tree.body sees only the top level")
        self.assertEqual(imported_modules(self.NESTED), {"json", "requests"}, "ast.walk sees both")

    def test_an_import_name_scan_misses_a_call(self):
        self.assertEqual(imported_modules(self.BY_NAME), set(), "no Import node exists to find")
        self.assertEqual(loader_calls(self.BY_NAME), {"importlib.import_module"})

    def test_dunder_import_needs_no_import_statement_at_all(self):
        self.assertEqual(imported_modules(self.DUNDER), set())
        self.assertEqual(loader_calls(self.DUNDER), {"__import__"})

    def test_a_relative_import_is_not_mistaken_for_a_module(self):
        self.assertEqual(imported_modules("from . import sibling\n"), set())


if __name__ == "__main__":
    unittest.main()
