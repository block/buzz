#!/usr/bin/env python3
"""Controls for adr_boundary_check.

These import the module the check actually runs and drive `check_documents`,
which takes a `read_file` callable so both halves — list consistency and content
assertions — run without a checkout. Nothing here re-implements a regex:
replacing check_documents with `return []` fails every negative case below.

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

# Content that satisfies the assertions: no upstream image reference, and a
# README that points at the wrapper.
GOOD_CONTENT = {
    "deploy/compose/compose.yml": "image: ${BUZZ_IMAGE:?set it}",
    "deploy/compose/.env.example": "BUZZ_IMAGE=ghcr.io/launchpad-26/buzz:sha-abc",
    ".github/workflows/docker.yml": "branches: [launchpad]",
    "Dockerfile": 'LABEL org.opencontainers.image.source="https://github.com/launchpad-26/buzz"',
    "deploy/compose/README.md": "Use the wrapper at launchpad/deploy instead.",
}


def reader(overrides=None, missing=()):
    content = dict(GOOD_CONTENT)
    content.update(overrides or {})
    def read(rel):
        return None if rel in missing else content.get(rel, "")
    return read


def adr(paths=None, count_word="Five", tbd=False, back_link=True):
    paths = FIVE if paths is None else paths
    rows = "\n".join(f"| `{p}` | what it carries | why not an override |" for p in paths)
    return f"""---
status: Proposed
issue: launchpad-26/buzz#{'TBD' if tbd else '149'}
---

# ADR-0005

**{count_word} files are sanctioned to carry Launchpad values.** A third
deliberate exception to {'[`../AGENTS.md` §3](../AGENTS.md)' if back_link else '§3'}.

| File | What it carries | Why not an override |
|---|---|---|
{rows}
"""


def agents(paths=None, marker="Deployment image provenance", link=True, extra_bullets=0):
    paths = FIVE if paths is None else paths
    named = ", ".join(f"`{p}`" for p in paths)
    target = (
        "[ADR-0005](decisions/ADR-0005-launchpad-deployment-boundary.md)"
        if link
        else "an ADR"
    )
    others = "".join(f"\n- Some other exception {i}" for i in range(extra_bullets))
    return f"""## 3. Layout

Deliberate exceptions:

- `.github/ISSUE_TEMPLATE/`{others}

- **{marker}** — five named files ({named}); see {target}.

Unrelated prose that happens to mention Dockerfile and deploy/compose/compose.yml.
"""


class ListConsistency(unittest.TestCase):
    def test_consistent_documents_produce_no_failures(self):
        self.assertEqual(m.check_documents(adr(), agents(), reader()), [])

    def test_agents_entry_missing_a_file_is_reported(self):
        short = [p for p in FIVE if p != "Dockerfile"]
        failures = m.check_documents(adr(), agents(short), reader())
        self.assertTrue(any("does not name" in f and "Dockerfile" in f for f in failures))

    def test_a_path_only_in_unrelated_prose_does_not_satisfy_the_check(self):
        """The defect this replaced: `path in whole_document` passed on any mention.

        The template's trailing line mentions Dockerfile and compose.yml outside
        the exception entry. Dropping them from the entry must still fail.
        """
        entry_without = [p for p in FIVE if p not in
                         ("Dockerfile", "deploy/compose/compose.yml")]
        failures = m.check_documents(adr(), agents(entry_without), reader())
        self.assertTrue(any("does not name" in f for f in failures))

    def test_missing_exception_entry_is_reported(self):
        failures = m.check_documents(adr(), agents(marker="Something else"), reader())
        self.assertTrue(any("no 'Deployment image provenance'" in f for f in failures))

    def test_a_fourth_unrelated_exception_does_not_break_the_check(self):
        """An earlier version hardcoded 'Three deliberate exceptions'.

        Adding a legitimate fourth exception would have turned an
        intended-required check red for the whole repository.
        """
        self.assertEqual(
            m.check_documents(adr(), agents(extra_bullets=2), reader()), []
        )

    def test_prose_count_disagreeing_with_table_is_reported(self):
        failures = m.check_documents(adr(count_word="Six"), agents(), reader())
        self.assertTrue(any("prose says 6" in f for f in failures))

    def test_tbd_placeholder_is_reported(self):
        failures = m.check_documents(adr(tbd=True), agents(), reader())
        self.assertTrue(any("#TBD" in f for f in failures))

    def test_missing_links_are_reported_in_both_directions(self):
        no_fwd = m.check_documents(adr(), agents(link=False), reader())
        self.assertTrue(any("does not link to the ADR" in f for f in no_fwd))
        no_back = m.check_documents(adr(back_link=False), agents(), reader())
        self.assertTrue(any("does not link back" in f for f in no_back))

    def test_fenced_example_rows_are_not_mistaken_for_the_table(self):
        decoy = adr() + "\n```\n| `not/a/real/file.yml` | example | example |\n```\n"
        self.assertEqual(m.parse_adr_files(decoy), FIVE)

    def test_duplicate_row_is_reported(self):
        failures = m.check_documents(adr(FIVE + ["Dockerfile"]), agents(), reader())
        self.assertTrue(any("lists a file twice" in f for f in failures))


class ContentAssertions(unittest.TestCase):
    """The half that was missing, and let a green check mean nothing."""

    def test_upstream_image_default_in_compose_is_reported(self):
        failures = m.check_documents(
            adr(), agents(),
            reader({"deploy/compose/compose.yml":
                    "image: ${BUZZ_IMAGE:-ghcr.io/block/buzz:main}"}),
        )
        self.assertTrue(any("compose.yml" in f and "clean checkout" in f
                            for f in failures))

    def test_upstream_labels_in_dockerfile_are_reported(self):
        failures = m.check_documents(
            adr(), agents(),
            reader({"Dockerfile": 'source="https://github.com/block/buzz"'}),
        )
        self.assertTrue(any("Dockerfile" in f and "upstream" in f for f in failures))

    def test_upstream_namespace_in_workflow_is_reported(self):
        failures = m.check_documents(
            adr(), agents(),
            reader({".github/workflows/docker.yml": "IMAGE_NAME: ghcr.io/block/buzz"}),
        )
        self.assertTrue(any("docker.yml" in f for f in failures))

    def test_readme_not_pointing_at_the_wrapper_is_reported(self):
        failures = m.check_documents(
            adr(), agents(), reader({"deploy/compose/README.md": "Run run.sh."}),
        )
        self.assertTrue(any("README.md" in f and "wrapper" in f for f in failures))

    def test_existing_but_unchanged_file_no_longer_passes(self):
        """The Blocker: Path.exists() on files that exist upstream anyway.

        Every file present, every one still carrying upstream values -> five
        failures, where the previous version reported none.
        """
        upstream = {
            "deploy/compose/compose.yml": "image: ${BUZZ_IMAGE:-ghcr.io/block/buzz:main}",
            "deploy/compose/.env.example": "BUZZ_IMAGE=ghcr.io/block/buzz:main",
            ".github/workflows/docker.yml": "IMAGE_NAME: ghcr.io/block/buzz",
            "Dockerfile": 'source="https://github.com/block/buzz"',
            "deploy/compose/README.md": "Run deploy/compose/run.sh.",
        }
        failures = m.check_documents(adr(), agents(), reader(upstream))
        self.assertEqual(len(failures), 5, failures)

    def test_missing_file_is_reported(self):
        failures = m.check_documents(
            adr(), agents(), reader(missing={"Dockerfile"}),
        )
        self.assertTrue(any("does not exist" in f and "Dockerfile" in f
                            for f in failures))

    def test_a_sixth_file_with_no_content_assertion_fails_rather_than_skips(self):
        """Widening the list must not sanction a file on the strength of existing."""
        six = FIVE + ["justfile"]
        failures = m.check_documents(adr(six, count_word="Six"), agents(six), reader())
        self.assertTrue(any("justfile" in f and "asserts nothing" in f
                            for f in failures))


if __name__ == "__main__":
    unittest.main()
