"""Controls for the pre-flight — launchpad-26/buzz#116.

Run from the repository root:

    python3 -m unittest discover -s launchpad/scripts -t launchpad/scripts

No control here touches the network. Fixtures under ``testdata/`` were recorded
from the live API by ``testdata/record.sh``; see ``testdata/README.md``.
"""

from __future__ import annotations

import json
import os
import sys
import unittest
from collections import Counter

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import preflight_core as core  # noqa: E402  (after the sys.path insert, deliberately)

TESTDATA = os.path.join(os.path.dirname(os.path.abspath(__file__)), "testdata")


def fixture(name: str):
    with open(os.path.join(TESTDATA, name), encoding="utf-8") as handle:
        return json.load(handle)


#: The endpoint each read would have come from, so a skip can name its source.
ENDPOINTS = {
    "pr": "GET /repos/{o}/{r}/pulls/{n}",
    "meta": "gh pr view {n} --json title,body,labels",
    "compare": "GET /repos/{o}/{r}/compare/{base}...{head}",
    "checks": "graphql:statusCheckRollup",
    "tree": "GET /repos/{o}/{r}/git/trees/{head}?recursive=1",
    "branch_rules": "GET /repos/{o}/{r}/rules/branches/{base}",
    "org_rulesets": "GET /orgs/{o}/rulesets",
}

#: PR 86, everything readable. The baseline every degradation is measured against.
PR86_FIXTURES = {
    "pr": "pr86-pr.json",
    "meta": "pr86-meta.json",
    "compare": "pr86-compare.json",
    "checks": "pr86-checks.json",
    "tree": "pr86-tree.json",
    "branch_rules": "rules-branches-launchpad.json",
}


def reads(**overrides: core.Read) -> dict[str, core.Read]:
    """PR 86's reads, with any of them replaced.

    Org rulesets default to the forbidden state, because that is the state this
    token is actually in — a control that assumed it readable would be testing a
    world we do not run in.
    """
    built = {
        name: core.Read(name, data=fixture(path), endpoint=ENDPOINTS[name])
        for name, path in PR86_FIXTURES.items()
    }
    built["org_rulesets"] = core.Read(
        "org_rulesets",
        skip=core.FORBIDDEN,
        detail="404 for an organization that exists — this token lacks admin:org",
        endpoint=ENDPOINTS["org_rulesets"],
    )
    built.update(overrides)
    return built


def unreadable(name: str, reason: str) -> core.Read:
    return core.Read(name, skip=reason, detail=f"forced {reason}", endpoint=ENDPOINTS[name])


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

    def test_pr86_compare_is_not_an_empty_success(self):
        """Recorded by branch name once, which answered 200 OK with zero files."""
        recorded = fixture("pr86-compare.json")
        self.assertGreater(len(recorded["files"]), 0, "re-record: compare by base.sha, not branch name")
        self.assertNotEqual(
            fixture("pr86-pr.json")["base"]["sha"],
            recorded["merge_base_commit"]["sha"],
            "PR 86's base tip and merge base have reconverged; the two-dot trap needs the upstream fixture",
        )

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


class RecordShape(unittest.TestCase):
    """STEP 2 — the record's fields are a fixed list, and checks stay a list."""

    def test_top_level_keys_are_exactly_the_seven_enumerated_fields(self):
        record = core.build_record(reads())
        self.assertEqual(tuple(record), core.RECORD_FIELDS)
        self.assertEqual(
            core.RECORD_FIELDS,
            ("pr", "closing_issue", "diff", "checks", "required_gate", "nearest_rules", "skips"),
            "the enumerated list changed — the module docstring must change with it",
        )

    def test_every_recorded_check_survives_into_the_record(self):
        """No de-duplication, no name-keying: 47 in, 47 out, three named `check`."""
        recorded = rollup_contexts(fixture("pr86-checks.json"))
        record = core.build_record(reads())
        self.assertEqual(len(record["checks"]), len(recorded))
        self.assertIsInstance(record["checks"], list)
        names = [c["name"] for c in record["checks"]]
        self.assertEqual(names.count("check"), 3)
        self.assertEqual(
            len({(c["name"], c["details_url"]) for c in record["checks"]}),
            len(recorded),
            "checks must stay distinguishable once names collide",
        )

    def test_each_check_carries_the_six_enumerated_keys(self):
        for check in core.build_record(reads())["checks"]:
            self.assertEqual(
                sorted(check),
                ["conclusion", "details_url", "name", "required", "status", "workflow"],
            )

    def test_pr_section_carries_the_six_enumerated_keys(self):
        pr = core.build_record(reads())["pr"]
        self.assertEqual(
            sorted(pr), ["base_ref", "body", "head_sha", "labels", "number", "title"]
        )
        self.assertEqual(pr["number"], 86)
        self.assertEqual(pr["base_ref"], "launchpad")

    def test_status_context_shape_does_not_invent_a_status(self):
        """The old commit-status API has one state and no workflow. Say so."""
        node = {"__typename": "StatusContext", "context": "ci/legacy", "state": "SUCCESS",
                "targetUrl": "https://example.invalid/1", "isRequired": True}
        self.assertEqual(
            core._normalise_check(node),
            {"name": "ci/legacy", "workflow": None, "status": None,
             "conclusion": "SUCCESS", "required": True,
             "details_url": "https://example.invalid/1"},
        )

    def test_an_unenumerated_skip_reason_is_refused(self):
        with self.assertRaises(ValueError):
            core.Read("pr", skip="probably-fine")


if __name__ == "__main__":
    unittest.main()
