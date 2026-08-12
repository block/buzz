"""Controls for the pre-flight — launchpad-26/buzz#116.

Run from the repository root:

    python3 -m unittest discover -s launchpad/scripts -t launchpad/scripts

No control here touches the network. Fixtures under ``testdata/`` were recorded
from the live API by ``testdata/record.sh``; see ``testdata/README.md``.
"""

from __future__ import annotations

import json
import os
import unittest
from collections import Counter

TESTDATA = os.path.join(os.path.dirname(os.path.abspath(__file__)), "testdata")


def fixture(name: str):
    with open(os.path.join(TESTDATA, name), encoding="utf-8") as handle:
        return json.load(handle)


def rollup_contexts(recorded: dict) -> list[dict]:
    """The check contexts inside a recorded GraphQL statusCheckRollup response."""
    commit = recorded["data"]["repository"]["pullRequest"]["commits"]["nodes"][0]["commit"]
    return commit["statusCheckRollup"]["contexts"]["nodes"]


class FixtureIntegrity(unittest.TestCase):
    """STEP 1's done-when, as controls rather than as a command someone ran once.

    These are the properties the later controls lean on. If a re-recording makes
    one false, the fixture set has drifted out from under the suite and these
    fail first, where the cause is legible.
    """

    def test_every_fixture_parses_as_json(self):
        names = sorted(n for n in os.listdir(TESTDATA) if n.endswith(".json"))
        self.assertGreaterEqual(len(names), 17, "fixtures are missing; re-run record.sh")
        for name in names:
            with self.subTest(fixture=name):
                fixture(name)

    def test_check_names_collide_so_a_map_would_drop_entries(self):
        """The reason the record carries checks as a list, not a name-keyed map."""
        names = [c.get("name") or c.get("context") for c in rollup_contexts(fixture("pr86-checks.json"))]
        self.assertGreater(len(names), 1)
        duplicated = {n: c for n, c in Counter(names).items() if c > 1}
        self.assertIn("check", duplicated, "PR 86's colliding 'check' entries are the point of this fixture")
        self.assertGreaterEqual(duplicated["check"], 2)
        self.assertLess(
            len(set(names)),
            len(names),
            "a name-keyed map would silently drop the collisions",
        )

    def test_divergent_fixture_base_tip_is_not_the_merge_base(self):
        """Without this, no control can tell a three-dot diff from a two-dot one."""
        base_tip = fixture("upstream-divergent-pr.json")["base"]["sha"]
        merge_base = fixture("upstream-divergent-compare.json")["merge_base_commit"]["sha"]
        self.assertNotEqual(
            base_tip,
            merge_base,
            "the divergent fixture has converged — re-run record.sh to find a live one",
        )

    def test_rules_file_fixtures_cover_both_directions(self):
        added = [
            f["filename"]
            for f in fixture("pr14-compare.json")["files"]
            if f["status"] == "added" and f["filename"].rsplit("/", 1)[-1] in ("AGENTS.md", "CLAUDE.md")
        ]
        removed = [
            f["filename"]
            for f in fixture("prdelete-compare.json")["files"]
            if f["status"] == "removed" and f["filename"].rsplit("/", 1)[-1] in ("AGENTS.md", "CLAUDE.md")
        ]
        self.assertEqual(added, ["launchpad/AGENTS.md"])
        self.assertEqual(removed, ["launchpad/AGENTS.md"])

    def test_deleted_rules_file_is_absent_from_the_head_tree(self):
        """A resolver reading the local worktree passes the add case and fails this."""
        paths = {e["path"] for e in fixture("prdelete-tree.json")["tree"]}
        self.assertNotIn("launchpad/AGENTS.md", paths)
        self.assertIn("AGENTS.md", paths, "the root file it must fall back to")

    def test_truncated_tree_fixture_reports_success(self):
        """The trap: a partial tree arrives as HTTP 200 with truncated: true."""
        recorded = fixture("tree-truncated.json")
        self.assertTrue(recorded["truncated"])

    def test_unreadable_fixtures_are_not_empty_successes(self):
        self.assertEqual(fixture("pr-notfound.json")["message"], "Not Found")
        self.assertEqual(fixture("rules-branches-launchpad.json"), [])
        self.assertIn("message", fixture("orgs-rulesets-forbidden.json"))

    def test_markdown_tree_fixture_keeps_a_lookalike_rules_path(self):
        """`endswith("AGENTS.md")` is the wrong test, and this fixture proves it."""
        paths = {e["path"] for e in fixture("pr86-tree.json")["tree"]}
        self.assertIn("VISION_REMOTE_AGENTS.md", paths)
        self.assertIn("AGENTS.md", paths)


if __name__ == "__main__":
    unittest.main()
