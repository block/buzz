#!/usr/bin/env python3
"""Controls for adr_boundary_check.

These import the module the check actually runs and drive `check_documents`, which
takes an `exists` callable so the comparison can be exercised without a checkout.
Nothing here re-implements a regex: replacing check_documents with `return []`
fails every negative case below.

Run:  python3 -m unittest discover -s launchpad/scripts
"""

from __future__ import annotations

import unittest

import adr_boundary_check as m

FIVE = [
    "deploy/compose/compose.yml",
    "deploy/compose/.env.example",
    "deploy/compose/README.md",
    "Dockerfile",
    ".github/workflows/docker.yml",
]


def adr(paths=None, count_word="Five", tbd=False, back_link=True):
    paths = FIVE if paths is None else paths
    rows = "\n".join(f"| `{p}` | what it carries | why not an override |" for p in paths)
    return f"""---
status: Proposed
issue: launchpad-26/buzz#{'TBD' if tbd else '149'}
---

# ADR-0005

**{count_word} files are sanctioned to carry Launchpad values.** This is a third
deliberate exception to {'[`../AGENTS.md` §3](../AGENTS.md)' if back_link else '§3'}.

| File | What it carries | Why not an override |
|---|---|---|
{rows}
"""


def agents(paths=None, heading="Three deliberate exceptions", link=True):
    paths = FIVE if paths is None else paths
    named = ", ".join(f"`{p}`" for p in paths)
    target = (
        "[ADR-0005](decisions/ADR-0005-launchpad-deployment-boundary.md)"
        if link
        else "an ADR"
    )
    return f"""## 3. Layout

{heading}:

- `.github/ISSUE_TEMPLATE/`
- `.github/PULL_REQUEST_TEMPLATE.md`
- Deployment image provenance — five named files ({named}); see {target}.
"""


def all_exist(_rel):
    return True


class AdrBoundaryCheck(unittest.TestCase):
    def test_consistent_documents_produce_no_failures(self):
        self.assertEqual(m.check_documents(adr(), agents(), all_exist), [])

    def test_agents_missing_a_sanctioned_file_is_reported(self):
        short = [p for p in FIVE if p != "Dockerfile"]
        failures = m.check_documents(adr(), agents(short), all_exist)
        self.assertTrue(any("does not name" in f and "Dockerfile" in f for f in failures))

    def test_prose_count_disagreeing_with_table_is_reported(self):
        failures = m.check_documents(adr(count_word="Six"), agents(), all_exist)
        self.assertTrue(any("prose says 6" in f for f in failures))

    def test_a_sixth_file_added_to_the_table_trips_the_prose_count(self):
        six = FIVE + ["justfile"]
        failures = m.check_documents(adr(six), agents(six), all_exist)
        self.assertTrue(any("table lists 6" in f for f in failures))

    def test_missing_file_on_disk_is_reported(self):
        failures = m.check_documents(
            adr(), agents(), lambda rel: rel != "Dockerfile"
        )
        self.assertTrue(any("does not exist" in f and "Dockerfile" in f for f in failures))

    def test_tbd_placeholder_is_reported(self):
        failures = m.check_documents(adr(tbd=True), agents(), all_exist)
        self.assertTrue(any("#TBD" in f for f in failures))

    def test_stale_two_exceptions_wording_is_reported(self):
        failures = m.check_documents(
            adr(), agents(heading="Two deliberate exceptions"), all_exist
        )
        self.assertTrue(any("Three deliberate exceptions" in f for f in failures))

    def test_missing_links_are_reported_in_both_directions(self):
        no_fwd = m.check_documents(adr(), agents(link=False), all_exist)
        self.assertTrue(any("does not link to the ADR" in f for f in no_fwd))
        no_back = m.check_documents(adr(back_link=False), agents(), all_exist)
        self.assertTrue(any("does not link back" in f for f in no_back))

    def test_fenced_example_rows_are_not_mistaken_for_the_table(self):
        """A fenced block showing a sample row must not be parsed as sanctioned."""
        decoy = adr() + "\n```\n| `not/a/real/file.yml` | example | example |\n```\n"
        self.assertEqual(m.parse_adr_files(decoy), FIVE)

    def test_duplicate_row_is_reported(self):
        dupes = FIVE + ["Dockerfile"]
        failures = m.check_documents(adr(dupes), agents(), all_exist)
        self.assertTrue(any("lists a file twice" in f for f in failures))


if __name__ == "__main__":
    unittest.main()
