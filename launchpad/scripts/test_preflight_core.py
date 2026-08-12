"""Controls for the pre-flight — launchpad-26/buzz#116.

Run from the repository root:

    python3 -m unittest discover -s launchpad/scripts -t launchpad/scripts

No control here touches the network. Fixtures under ``testdata/`` were recorded
from the live API by ``testdata/record.sh``; see ``testdata/README.md``.
"""

from __future__ import annotations

import contextlib
import io
import json
import os
import sys
import unittest
from collections import Counter

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import preflight_core as core  # noqa: E402  (after the sys.path insert, deliberately)
import preflight_fetch as fetch  # noqa: E402

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
    "closing_refs": "graphql:closingIssuesReferences",
}

#: PR 86, everything readable. The baseline every degradation is measured against.
PR86_FIXTURES = {
    "pr": "pr86-pr.json",
    "meta": "pr86-meta.json",
    "compare": "pr86-compare.json",
    "checks": "pr86-checks.json",
    "tree": "pr86-tree.json",
    "branch_rules": "rules-branches-launchpad.json",
    "closing_refs": "pr86-closing-refs.json",
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


# --------------------------------------------------------------------------- #
# The fake runner. Every control that drives the CLI goes through this, so no
# control needs the network and each individual call can be made to fail.
# --------------------------------------------------------------------------- #


class FakeGh:
    """Answer ``gh`` calls from recorded fixtures, and remember every call made.

    ``fail`` forces one read to a given (returncode, stdout, stderr) so a control
    can break exactly one call and leave the other six working — which is the
    only way to show that *this* input is the one whose failure is fatal.
    """

    def __init__(self, fail: dict[str, fetch.RunResult] | None = None, payloads: dict[str, object] | None = None):
        self.fail = fail or {}
        self.payloads = payloads or {}
        self.calls: list[list[str]] = []
        self.binaries: list[str] = []

    #: fixture per read for a fully-readable PR 86 run
    FIXTURES = {
        "pr": "pr86-pr.json",
        "meta": "pr86-meta.json",
        "checks": "pr86-checks.json",
        "compare": "pr86-compare.json",
        "tree": "pr86-tree.json",
        "branch_rules": "rules-branches-launchpad.json",
        "closing_refs": "pr86-closing-refs.json",
    }

    @staticmethod
    def classify(argv: list[str]) -> str:
        if argv[1:3] == ["pr", "view"]:
            return "meta"
        target = argv[2] if len(argv) > 2 else ""
        if target == "graphql":
            query = " ".join(argv)
            if "closingIssuesReferences" in query:
                return "closing_refs"
            if "statusCheckRollup" in query:
                return "checks"
            raise AssertionError(f"unrecognised graphql query: {query[:120]}")
        if "/compare/" in target:
            return "compare"
        if "/git/trees/" in target:
            return "tree"
        if "/rules/branches/" in target:
            return "branch_rules"
        if target.startswith("orgs/"):
            return "org_rulesets"
        if "/pulls/" in target:
            return "pr"
        raise AssertionError(f"the fake runner does not know this call: {argv}")

    def __call__(self, argv: list[str]) -> fetch.RunResult:
        self.calls.append(argv)
        self.binaries.append(argv[0])
        if argv[0] != "gh":
            # Not raising: a control asserts on `binaries`, and a runner that
            # threw here would hide the argv from the assertion.
            return fetch.RunResult(127, "", f"refusing to spawn {argv[0]!r}")
        name = self.classify(argv)
        if name in self.fail:
            return self.fail[name]
        if name in self.payloads:
            return fetch.RunResult(0, json.dumps(self.payloads[name]), "")
        if name == "org_rulesets":
            # The state this token is really in: a 404 that hides access.
            return fetch.RunResult(1, "", "gh: Not Found (HTTP 404)")
        return fetch.RunResult(0, json.dumps(fixture(self.FIXTURES[name])), "")


def run_cli(argv: list[str], runner) -> tuple[int, str, str]:
    """Drive the CLI in-process and capture its streams and exit code."""
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        code = fetch.main(argv, runner=runner)
    return code, out.getvalue(), err.getvalue()


NOT_FOUND = fetch.RunResult(1, "", "gh: Not Found (HTTP 404)")
FORBIDDEN_RESULT = fetch.RunResult(1, "", "gh: Resource not accessible (HTTP 403)")
GARBAGE = fetch.RunResult(0, "<html>502 upstream</html>", "")


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


class ClosingIssue(unittest.TestCase):
    """STEP 4 — GitHub decides what a PR closes; the text supplies the keyword.

    Every control here is driven by a recorded response, and each of the three
    closing-refs fixtures exists because the text and GitHub disagree in a
    different direction.
    """

    def closing(self, meta: str, refs: str | None) -> dict:
        overrides = {"meta": core.Read("meta", data=fixture(meta), endpoint=ENDPOINTS["meta"])}
        if refs is not None:
            overrides["closing_refs"] = core.Read(
                "closing_refs", data=fixture(refs), endpoint="graphql:closingIssuesReferences"
            )
        return core.build_record(reads(**overrides))["closing_issue"]

    def test_a_visible_keyword_is_reported_with_the_keyword_used(self):
        section = self.closing("pr86-meta.json", "pr86-closing-refs.json")
        self.assertTrue(section["present"])
        self.assertEqual(section["keyword"], "Closes")
        self.assertEqual(section["source"], "graphql:closingIssuesReferences")

    def test_every_reference_is_collected_not_only_the_first(self):
        """PR 86 closes two issues, and the plan's `re.search` would report one.

        This is the control that pins the implementation off first-match: it
        reproduces what a single search returns and requires the record to hold
        more than that.
        """
        section = self.closing("pr86-meta.json", "pr86-closing-refs.json")
        self.assertEqual(section["issue_numbers"], [79, 91], "GitHub's answer")
        self.assertEqual(section["text_issue_numbers"], [79, 91], "and every keyword in the body")

        body = core.HTML_COMMENT.sub("", fixture("pr86-meta.json")["body"])
        first_only = [int(core.CLOSING_KEYWORD.search(body).group(2))]
        self.assertEqual(first_only, [79])
        self.assertNotEqual(
            section["text_issue_numbers"],
            first_only,
            "a first-match regex reports one of the two issues this PR closes",
        )
        self.assertFalse(section["text_disagrees"], "on PR 86 the body and GitHub do agree")

    def test_a_keyword_that_closes_nothing_is_not_reported_as_present(self):
        """PR 92's base was not the default branch, so merging closes no issue."""
        section = self.closing("pr92-meta.json", "pr92-closing-refs.json")
        self.assertFalse(section["present"])
        self.assertEqual(section["issue_numbers"], [])
        self.assertTrue(section["text_issue_numbers"], "the body really does carry a keyword")
        self.assertTrue(section["text_disagrees"])

    def test_a_keyword_only_inside_an_html_comment_is_not_present(self):
        """An unfilled <!-- Fixes #1234 --> placeholder closes nothing."""
        section = self.closing("upstream5695-meta.json", "upstream5695-closing-refs.json")
        self.assertFalse(section["present"])
        self.assertEqual(section["issue_numbers"], [])
        self.assertEqual(
            section["text_issue_numbers"], [], "comments are stripped before the text is scanned"
        )
        self.assertFalse(section["text_disagrees"], "both halves agree it closes nothing")
        self.assertIsNone(section["keyword"])

    def test_the_commented_out_keyword_is_really_in_the_fixture(self):
        """Otherwise the control above passes for the wrong reason."""
        body = fixture("upstream5695-meta.json")["body"]
        self.assertRegex(body, r"(?is)<!--[^>]*fixes\s+#\d+")
        self.assertNotRegex(
            core.HTML_COMMENT.sub("", body),
            r"(?i)\b(closes|fixes|resolves)\s+#\d+",
            "outside its comments this body has no keyword at all",
        )

    def test_an_unreadable_github_answer_is_unknown_and_never_false(self):
        """"We could not ask" must not read as "it closes nothing"."""
        section = core.build_record(
            reads(closing_refs=unreadable("closing_refs", core.FORBIDDEN))
        )["closing_issue"]
        self.assertIsNone(section["present"], "unknown, not False")
        self.assertIsNone(section["issue_numbers"])
        self.assertIn("unresolved", section["source"])
        self.assertEqual(section["keyword"], "Closes", "the text half still reports what it saw")

    def test_an_unreadable_github_answer_records_a_skip_but_exits_zero(self):
        code, out, err = run_cli(["86"], FakeGh(fail={"closing_refs": FORBIDDEN_RESULT}))
        self.assertEqual(code, 0, err)
        record = json.loads(out)
        skips = {s["field"]: s["reason"] for s in record["skips"]}
        self.assertEqual(skips.get("closing_issue.closing_refs"), core.FORBIDDEN)
        self.assertIsNone(record["closing_issue"]["present"])

    def test_a_malformed_github_answer_is_unknown_too(self):
        section = core.build_record(
            reads(closing_refs=core.Read("closing_refs", data={"data": {"repository": None}}))
        )["closing_issue"]
        self.assertIsNone(section["present"])
        self.assertIn("unresolved", section["source"])


class MergeBaseDiff(unittest.TestCase):
    """STEP 5 — the diff is against the merge base, not the base branch tip.

    ``baseRefOid`` is the tip of the base branch *now*, not the commit the head
    forked from. Diffing against it attributes every commit landed on the base
    since the fork to this PR's author, in reverse.
    """

    def test_the_recorded_paths_are_the_prs_own_files(self):
        record = core.build_record(reads())
        self.assertEqual(
            sorted(f["path"] for f in record["diff"]["files"]),
            sorted(f["filename"] for f in fixture("pr86-compare.json")["files"]),
        )
        self.assertEqual(
            sorted(f["path"] for f in record["diff"]["files"]),
            [
                "launchpad/ARCHITECTURE.md",
                "launchpad/ENVIRONMENTS.md",
                "launchpad/README.md",
                "launchpad/REQUIREMENTS.md",
                "launchpad/SECURITY-POSTURE.md",
                "launchpad/VISION.md",
            ],
            "identical to `gh pr diff 86 --repo launchpad-26/buzz --name-only | sort`",
        )

    def test_each_file_carries_the_four_enumerated_keys(self):
        for entry in core.build_record(reads())["diff"]["files"]:
            self.assertEqual(sorted(entry), ["added", "path", "removed", "status"])

    def test_the_recorded_base_is_the_merge_base_and_not_the_base_tip(self):
        """On PR 86, whose base tip has moved 6 commits past its fork point."""
        record = core.build_record(reads())
        recorded = fixture("pr86-compare.json")
        self.assertEqual(record["diff"]["merge_base_sha"], recorded["merge_base_commit"]["sha"])
        self.assertNotEqual(
            record["diff"]["merge_base_sha"],
            fixture("pr86-pr.json")["base"]["sha"],
            "the base tip is not the merge base",
        )

    def test_a_divergent_pr_reports_its_fork_point_not_its_base_tip(self):
        """The fixture that exists because a two-dot implementation passes without it."""
        pr = fixture("upstream-divergent-pr.json")
        compare = fixture("upstream-divergent-compare.json")
        skips = core.Skips()
        diff = core.build_diff(
            core.Read("compare", data=compare, endpoint=ENDPOINTS["compare"]),
            {"head_sha": pr["head"]["sha"]},
            skips,
        )
        self.assertEqual(diff["merge_base_sha"], compare["merge_base_commit"]["sha"])
        self.assertNotEqual(
            diff["merge_base_sha"],
            pr["base"]["sha"],
            "a two-dot implementation records the base tip here and this is where it fails",
        )
        self.assertEqual(skips.entries, [])

    def test_the_head_sha_pins_the_commit_pair_the_record_read(self):
        record = core.build_record(reads())
        self.assertEqual(record["diff"]["head_sha"], fixture("pr86-pr.json")["head"]["sha"])
        self.assertEqual(record["diff"]["head_sha"], record["pr"]["head_sha"])

    def test_a_compare_without_a_merge_base_is_a_skip_not_an_empty_diff(self):
        broken = {k: v for k, v in fixture("pr86-compare.json").items() if k != "merge_base_commit"}
        skips = core.Skips()
        diff = core.build_diff(core.Read("compare", data=broken), {"head_sha": "abc"}, skips)
        self.assertIsNone(diff)
        self.assertEqual(skips.entries[0]["reason"], core.MALFORMED)


class RequiredGate(unittest.TestCase):
    """STEP 6 — an empty required set is reported, with the endpoint that answered."""

    def test_every_check_on_pr_86_reports_required_false(self):
        record = core.build_record(reads())
        self.assertEqual(len(record["checks"]), len(rollup_contexts(fixture("pr86-checks.json"))))
        self.assertEqual({c["required"] for c in record["checks"]}, {False})

    def test_no_gate_is_reported_as_false_naming_the_endpoint_that_said_so(self):
        gate = core.build_record(reads())["required_gate"]
        self.assertIs(gate["configured"], False)
        self.assertIn("/rules/branches/", gate["source_endpoint"])
        self.assertEqual(sorted(gate), ["configured", "source_endpoint"])

    def test_false_is_never_published_without_the_org_ruleset_skip_beside_it(self):
        """The token cannot see org rulesets, so false means "nothing visible"."""
        record = core.build_record(reads())
        self.assertIs(record["required_gate"]["configured"], False)
        skips = {s["field"]: s["reason"] for s in record["skips"]}
        self.assertEqual(skips.get("required_gate.org_rulesets"), core.FORBIDDEN)

    def test_org_rulesets_are_skipped_not_asserted_absent(self):
        entry = next(
            s for s in core.build_record(reads())["skips"]
            if s["field"] == "required_gate.org_rulesets"
        )
        self.assertIn("admin:org", entry["detail"])
        self.assertEqual(entry["endpoint"], ENDPOINTS["org_rulesets"])

    def test_a_repo_rule_requiring_checks_is_reported_from_that_endpoint(self):
        rules = [{"type": "required_status_checks", "ruleset_id": 1}]
        gate = core.build_record(
            reads(branch_rules=core.Read("branch_rules", data=rules, endpoint=ENDPOINTS["branch_rules"]))
        )["required_gate"]
        self.assertIs(gate["configured"], True)
        self.assertIn("/rules/branches/", gate["source_endpoint"])

    def test_a_required_context_with_no_readable_rule_is_credited_to_graphql(self):
        """What an org-level ruleset looks like from underneath it."""
        recorded = fixture("pr86-checks.json")
        nodes = rollup_contexts(recorded)
        nodes[0] = {**nodes[0], "isRequired": True}
        gate = core.build_record(
            reads(checks=core.Read("checks", data=recorded, endpoint=ENDPOINTS["checks"]))
        )["required_gate"]
        self.assertIs(gate["configured"], True)
        self.assertEqual(gate["source_endpoint"], "graphql:isRequired")

    def test_an_unreadable_rules_probe_is_unknown_not_false(self):
        gate = core.build_record(
            reads(branch_rules=unreadable("branch_rules", core.FORBIDDEN))
        )["required_gate"]
        self.assertIsNone(gate["configured"], "unknown, because nothing answered")

    def test_an_unreadable_rules_probe_still_exits_zero(self):
        code, out, err = run_cli(["86"], FakeGh(fail={"branch_rules": FORBIDDEN_RESULT}))
        self.assertEqual(code, 0, err)
        self.assertIsNone(json.loads(out)["required_gate"]["configured"])


class NearestRules(unittest.TestCase):
    """STEP 7 — both rules files, resolved against the PR's head tree.

    ``resolve`` drives the pure resolver with a real recorded tree and a list of
    changed paths, because the paths a control needs to exercise are not all in
    one PR's diff. The tree — the thing whose shape could be wrong — is always
    recorded.
    """

    def resolve(self, paths: list[str], tree_name: str = "pr86-tree.json", tree=None):
        skips = core.Skips()
        resolved = core.build_nearest_rules(
            core.Read("tree", data=tree if tree is not None else fixture(tree_name), endpoint=ENDPOINTS["tree"]),
            {"files": [{"path": p} for p in paths]},
            skips,
        )
        return resolved, skips

    def test_a_launchpad_path_gets_launchpad_agents_and_the_root_claude(self):
        """First-wins would hide the root CLAUDE.md from every launchpad/ path."""
        resolved, skips = self.resolve(["launchpad/AGENTS.md"])
        self.assertEqual(
            resolved["launchpad/AGENTS.md"],
            {"AGENTS.md": "launchpad/AGENTS.md", "CLAUDE.md": "CLAUDE.md"},
        )
        self.assertEqual(skips.entries, [])

    def test_a_desktop_source_path_gets_both_root_files(self):
        resolved, _ = self.resolve(["desktop/src/main.tsx"])
        self.assertEqual(
            resolved["desktop/src/main.tsx"],
            {"AGENTS.md": "AGENTS.md", "CLAUDE.md": "CLAUDE.md"},
        )

    def test_the_nearest_wins_over_an_ancestor(self):
        """desktop/src/features/agents/ has its own AGENTS.md in the recorded tree."""
        resolved, _ = self.resolve(["desktop/src/features/agents/AgentCard.tsx"])
        self.assertEqual(
            resolved["desktop/src/features/agents/AgentCard.tsx"]["AGENTS.md"],
            "desktop/src/features/agents/AGENTS.md",
        )

    def test_a_lookalike_filename_is_not_a_rules_file(self):
        """VISION_REMOTE_AGENTS.md ends with AGENTS.md and is not one."""
        self.assertIn("VISION_REMOTE_AGENTS.md", {e["path"] for e in fixture("pr86-tree.json")["tree"]})
        resolved, _ = self.resolve(["README.md"])
        self.assertEqual(resolved["README.md"]["AGENTS.md"], "AGENTS.md")

    def test_a_deleted_rules_file_falls_back_to_the_root_one(self):
        """The head tree, not the worktree: launchpad/AGENTS.md exists locally."""
        local = os.path.join(os.path.dirname(TESTDATA), "..", "AGENTS.md")
        self.assertTrue(
            os.path.exists(os.path.abspath(local)),
            "this control is only meaningful while the file really is on disk here",
        )
        resolved, skips = self.resolve(["launchpad/README.md"], "prdelete-tree.json")
        self.assertEqual(
            resolved["launchpad/README.md"],
            {"AGENTS.md": "AGENTS.md", "CLAUDE.md": "CLAUDE.md"},
            "a resolver reading the local checkout answers launchpad/AGENTS.md here",
        )
        self.assertEqual(skips.entries, [])

    def test_an_added_rules_file_is_picked_up_from_the_head_tree(self):
        """PR 14 adds launchpad/AGENTS.md; before it, launchpad/ had none."""
        added = [
            f["filename"] for f in fixture("pr14-compare.json")["files"] if f["status"] == "added"
        ]
        self.assertIn("launchpad/AGENTS.md", added)
        resolved, _ = self.resolve(["launchpad/README.md"], "pr14-tree.json")
        self.assertEqual(resolved["launchpad/README.md"]["AGENTS.md"], "launchpad/AGENTS.md")

    def test_resolution_never_reads_the_filesystem(self):
        """A tree naming a file that exists nowhere on disk still resolves."""
        invented = {
            "sha": "0" * 40,
            "truncated": False,
            "tree": [{"path": "made/up/dir/AGENTS.md", "type": "blob"}],
        }
        resolved, _ = self.resolve(["made/up/dir/thing.py"], tree=invented)
        self.assertEqual(resolved["made/up/dir/thing.py"]["AGENTS.md"], "made/up/dir/AGENTS.md")
        self.assertIsNone(resolved["made/up/dir/thing.py"]["CLAUDE.md"])

    def test_a_path_with_no_ancestor_rules_file_is_skip_only(self):
        """A real answer, not a failure — so the run still exits 0."""
        recorded = fixture("pr86-tree.json")
        # The input, not the response shape: the same recorded tree with its two
        # root rules files taken out, which is the only way to reach a path that
        # has no ancestor at all in a repository that has them at the root.
        stripped = {
            **recorded,
            "tree": [e for e in recorded["tree"] if e["path"] not in ("AGENTS.md", "CLAUDE.md")],
        }
        resolved, skips = self.resolve(["desktop/src/main.tsx"], tree=stripped)
        self.assertEqual(resolved["desktop/src/main.tsx"], {"AGENTS.md": None, "CLAUDE.md": None})
        entry = skips.entries[0]
        self.assertEqual(entry["field"], "nearest_rules[desktop/src/main.tsx]")
        self.assertEqual(entry["reason"], core.EMPTY)
        self.assertFalse(core.is_fatal(entry), "having no rules file is a fact, not a failure")

    def test_every_changed_path_in_a_real_run_is_resolved(self):
        record = core.build_record(reads())
        self.assertEqual(
            sorted(record["nearest_rules"]),
            sorted(f["path"] for f in record["diff"]["files"]),
        )


class CliShell(unittest.TestCase):
    """STEP 3 — the CLI prints a record, and refuses to print a broken one."""

    def test_a_readable_pr_prints_the_record_and_exits_zero(self):
        code, out, err = run_cli(["86"], FakeGh())
        self.assertEqual(code, 0, err)
        record = json.loads(out)
        self.assertEqual(tuple(record), core.RECORD_FIELDS)
        self.assertEqual(record["pr"]["number"], 86)
        self.assertEqual(len(record["checks"]), len(rollup_contexts(fixture("pr86-checks.json"))))

    def test_an_absent_pr_exits_non_zero_and_prints_no_record(self):
        code, out, err = run_cli(["999999"], FakeGh(fail={"pr": NOT_FOUND}))
        self.assertNotEqual(code, 0)
        self.assertEqual(out, "", "stdout must stay empty so a caller never pipes a holed record")
        self.assertEqual(json.loads(err)["skips"][0]["reason"], core.ABSENT)

    def test_the_runner_is_injected_so_no_control_touches_the_network(self):
        """The default runner is the real gh; every control replaces it."""
        fake = FakeGh()
        run_cli(["86"], fake)
        self.assertGreater(len(fake.calls), 0)
        self.assertEqual(set(fake.binaries), {"gh"})

    def test_dependent_calls_are_not_attempted_when_the_pr_read_fails(self):
        """No base.sha means compare and tree cannot be called at all."""
        fake = FakeGh(fail={"pr": NOT_FOUND})
        run_cli(["999999"], fake)
        made = {FakeGh.classify(argv) for argv in fake.calls}
        self.assertNotIn("compare", made)
        self.assertNotIn("tree", made)

    def test_compare_is_taken_by_sha_not_by_branch_name(self):
        """By name it answers 200 with zero files once the base tip moves past the head."""
        fake = FakeGh()
        run_cli(["86"], fake)
        compare = next(a for a in fake.calls if "/compare/" in a[2])
        base, _, head = compare[2].partition("...")
        pr = fixture("pr86-pr.json")
        self.assertTrue(base.endswith(pr["base"]["sha"]), compare[2])
        self.assertEqual(head, pr["head"]["sha"])
        self.assertNotIn("launchpad...", compare[2])

    def test_graphql_prose_absence_is_absent_not_unreachable(self):
        """`gh pr view 999999` reports absence with no HTTP status attached."""
        graphql_404 = fetch.RunResult(
            1, "", "GraphQL: Could not resolve to a PullRequest with the number of 999999."
        )
        code, out, err = run_cli(["999999"], FakeGh(fail={"meta": graphql_404}))
        self.assertNotEqual(code, 0)
        reasons = {s["reason"] for s in json.loads(err)["skips"]}
        self.assertIn(core.ABSENT, reasons)
        self.assertNotIn(core.UNREACHABLE, reasons)

    def test_help_names_the_taxonomy_and_the_exit_contract(self):
        text = fetch.build_parser().format_help()
        for reason in core.SKIP_REASONS:
            self.assertIn(reason, text)
        for name in core.REQUIRED_INPUTS:
            self.assertIn(name, text)
        for name in core.SKIP_ONLY_INPUTS:
            self.assertIn(name, text)
        self.assertIn("exits 2", text.lower(), "--help must say what a required-input failure does")
        self.assertIn("exit codes", text.lower())
        self.assertIn("truncated: true", text.lower(), "--help must name the partial-tree trap")


class ExitContract(unittest.TestCase):
    """STEP 8 — which unreadable input is fatal, proved one call at a time.

    The nonexistent-PR case STEP 3 covers is the easy one. These break exactly
    one call each and leave the other seven working, which is the only way to
    show that *this* input is the one whose failure is fatal — and, for the
    skip-only ones, that it is not.
    """

    def test_each_required_input_failing_alone_exits_non_zero(self):
        for name in core.REQUIRED_INPUTS:
            with self.subTest(required=name):
                code, out, err = run_cli(["86"], FakeGh(fail={name: NOT_FOUND}))
                self.assertNotEqual(code, 0, f"{name} failed and the run still exited 0")
                self.assertEqual(out, "", f"{name} failed and a record was printed anyway")
                self.assertIn("skips", json.loads(err))

    def test_each_skip_only_input_failing_alone_exits_zero(self):
        for name in core.SKIP_ONLY_INPUTS:
            with self.subTest(skip_only=name):
                code, out, err = run_cli(["86"], FakeGh(fail={name: FORBIDDEN_RESULT}))
                self.assertEqual(code, 0, f"{name} is skip-only and must not be fatal: {err}")
                record = json.loads(out)
                self.assertTrue(record["skips"], "the skip must be published, not swallowed")

    def test_a_truncated_head_tree_exits_non_zero(self):
        """HTTP 200 with a partial list, which must not read as "no rules file"."""
        code, out, err = run_cli(["86"], FakeGh(payloads={"tree": fixture("tree-truncated.json")}))
        self.assertNotEqual(code, 0, "a half-read tree cannot answer the nearest-rules question")
        self.assertEqual(out, "")
        entry = next(s for s in json.loads(err)["skips"] if s["field"] == "nearest_rules")
        self.assertEqual(entry["reason"], core.TRUNCATED)

    def test_a_truncated_tree_is_not_reported_as_an_empty_result(self):
        skips = core.Skips()
        resolved = core.build_nearest_rules(
            core.Read("tree", data=fixture("tree-truncated.json"), endpoint=ENDPOINTS["tree"]),
            {"files": [{"path": "kernel/fork.c"}]},
            skips,
        )
        self.assertIsNone(resolved, "None means not read; {} would mean nothing found")
        self.assertTrue(core.is_fatal(skips.entries[0]))

    def test_every_enumerated_reason_is_reachable_from_a_control(self):
        gh_missing = fetch.RunResult(127, "", "gh is not installed")
        empty_rollup = {
            "data": {"repository": {"pullRequest": {"commits": {"nodes": [{"commit": {"statusCheckRollup": None}}]}}}}
        }
        cases = {
            core.ABSENT: FakeGh(fail={"tree": NOT_FOUND}),
            core.FORBIDDEN: FakeGh(fail={"compare": FORBIDDEN_RESULT}),
            core.MALFORMED: FakeGh(fail={"compare": GARBAGE}),
            core.UNREACHABLE: FakeGh(fail={"pr": gh_missing}),
            core.TRUNCATED: FakeGh(payloads={"tree": fixture("tree-truncated.json")}),
            core.EMPTY: FakeGh(payloads={"checks": empty_rollup}),
        }
        self.assertEqual(set(cases), set(core.SKIP_REASONS), "a reason with no control is a reason nothing tests")
        for reason, fake in cases.items():
            with self.subTest(reason=reason):
                _, out, err = run_cli(["86"], fake)
                published = json.loads(out or err)
                self.assertIn(reason, {s["reason"] for s in published["skips"]})

    def test_an_empty_but_readable_check_list_is_not_a_failure(self):
        """A head commit whose checks have not started is readable, and empty."""
        empty_rollup = {
            "data": {"repository": {"pullRequest": {"commits": {"nodes": [{"commit": {"statusCheckRollup": None}}]}}}}
        }
        code, out, err = run_cli(["86"], FakeGh(payloads={"checks": empty_rollup}))
        self.assertEqual(code, 0, f"empty is an answer, not a failed read: {err}")
        record = json.loads(out)
        self.assertIsNone(record["checks"], "None, never [] — nothing was read into it")
        entry = next(s for s in record["skips"] if s["field"] == "checks")
        self.assertEqual(entry["reason"], core.EMPTY)

    def test_a_required_field_that_was_not_read_is_none_and_never_a_value(self):
        for name in ("compare", "checks", "tree"):
            with self.subTest(required=name):
                skips = core.Skips()
                if name == "compare":
                    value = core.build_diff(unreadable("compare", core.ABSENT), {"head_sha": "x"}, skips)
                elif name == "checks":
                    value = core.build_checks(unreadable("checks", core.ABSENT), skips)
                else:
                    value = core.build_nearest_rules(
                        unreadable("tree", core.ABSENT), {"files": []}, skips
                    )
                self.assertIsNone(value, "an unread field must not come back as [] or {}")
                self.assertTrue(skips.entries)

    def test_the_module_docstring_lists_the_taxonomy_and_the_split(self):
        doc = core.__doc__
        for reason in core.SKIP_REASONS:
            self.assertIn(reason, doc)
        for name in core.REQUIRED_INPUTS:
            self.assertIn(name, doc)
        for name in core.SKIP_ONLY_INPUTS:
            self.assertIn(name, doc)
        self.assertIn("truncated: true", doc)
        self.assertIn("exit", doc.lower())

    def test_a_usage_error_is_a_different_exit_code_from_an_unreadable_input(self):
        """Both would be 2 if argparse's default stood, and the contract says 1."""
        for bad in (["0"], ["-4"], ["not-a-number"]):
            with self.subTest(argv=bad):
                fake = FakeGh()
                with contextlib.redirect_stderr(io.StringIO()):
                    with self.assertRaises(SystemExit) as caught:
                        fetch.main(bad, runner=fake)
                self.assertEqual(caught.exception.code, 1, "usage error is 1, not argparse's 2")
                self.assertEqual(fake.calls, [], "nothing should be fetched for a bad argument")

        unreadable_code, _, _ = run_cli(["86"], FakeGh(fail={"pr": NOT_FOUND}))
        self.assertEqual(unreadable_code, 2)
        self.assertNotEqual(unreadable_code, 1)


#: The read each record field is built from, and what an EMPTY answer from that
#: read looks like on the wire. Seven fields are enumerated in preflight_core;
#: `skips` is the register the other six report into and has no read of its own,
#: so it gets its own controls below rather than a row here.
FIELD_SOURCES = {
    "pr": ("pr", {}),
    "closing_issue": (
        "closing_refs",
        {"data": {"repository": {"pullRequest": {"closingIssuesReferences": {"nodes": []}}}}},
    ),
    "diff": ("compare", {}),
    "checks": ("checks", {"data": {"repository": {"pullRequest": {"commits": {"nodes": []}}}}}),
    "required_gate": ("branch_rules", []),
    "nearest_rules": ("tree", {"sha": "0" * 40, "url": "x", "truncated": False, "tree": []}),
}

#: Two fields where an EMPTY answer is a real answer rather than a skip, each for
#: a stated reason. Pretending all six behave alike would be a tidier table and a
#: less honest one.
EMPTY_IS_AN_ANSWER = {
    "required_gate": "an empty rules list means no gate is configured, which is the fact to report",
    "closing_issue": "GitHub answering with no references means the PR closes nothing",
}


class AbsenceIsNeverAValue(unittest.TestCase):
    """STEP 9 — the mechanical proof, per record field, that a hole stays a hole.

    Six fields, three degradations each: empty, malformed, and erroring. No grep
    could do this. A grep reports every occurrence and leaves a human to sort the
    legitimate hits from the real ones, which is not a check that can fail.
    """

    def build(self, read_name: str, variant: str, payload=None):
        """Return (record, skips). A required input's failure builds NO record —
        that is the strongest form of "the field never became a value" — so the
        skip register comes off the exception instead."""
        if variant == "empty":
            degraded = core.Read(read_name, data=payload, endpoint=ENDPOINTS[read_name])
        elif variant == "malformed":
            degraded = core.Read(read_name, data="<html>502</html>", endpoint=ENDPOINTS[read_name])
        else:
            degraded = unreadable(read_name, core.ABSENT)
        try:
            record = core.build_record(reads(**{read_name: degraded}))
        except core.RecordError as failure:
            return None, failure.skips
        return record, record["skips"]

    def assert_not_a_value(self, field: str, record, skips: list[dict]):
        reported = {s["field"] for s in skips}
        self.assertTrue(
            any(f == field or f.startswith(f"{field}.") or f.startswith(f"{field}[") for f in reported),
            f"{field} was degraded and no skip names it",
        )
        for entry in skips:
            if entry["field"].startswith(field):
                self.assertIn(entry["reason"], core.SKIP_REASONS, "the reason must be enumerated")
                self.assertTrue(entry["detail"], "a skip with no detail is a shrug")
        if field in EMPTY_IS_AN_ANSWER:
            return  # asserted separately, per EMPTY_IS_AN_ANSWER
        if record is None:
            return  # no record was emitted at all, so no field became anything
        value = record[field]
        self.assertIsNone(value, f"{field} came back as {value!r} instead of a skip")
        self.assertNotEqual(value, [], "[] would read as 'nothing there' rather than 'not read'")
        self.assertNotEqual(value, {}, "{} would read as 'nothing there' rather than 'not read'")

    def test_an_erroring_response_never_becomes_a_value(self):
        for field, (read_name, _) in FIELD_SOURCES.items():
            with self.subTest(field=field, variant="erroring"):
                record, skips = self.build(read_name, "erroring")
                if record is not None and field == "required_gate":
                    self.assertIsNone(record[field]["configured"], "unknown, not false")
                if record is not None and field == "closing_issue":
                    self.assertIsNone(record[field]["present"], "unknown, not false")
                self.assert_not_a_value(field, record, skips)

    def test_a_malformed_response_never_becomes_a_value(self):
        for field, (read_name, _) in FIELD_SOURCES.items():
            with self.subTest(field=field, variant="malformed"):
                record, skips = self.build(read_name, "malformed")
                if record is not None and field == "required_gate":
                    self.assertIsNone(record[field]["configured"], "malformed is not an empty rules list")
                if record is not None and field == "closing_issue":
                    self.assertIsNone(record[field]["present"])
                self.assert_not_a_value(field, record, skips)

    def test_an_empty_response_is_either_a_skip_or_a_stated_answer(self):
        for field, (read_name, empty_payload) in FIELD_SOURCES.items():
            with self.subTest(field=field, variant="empty"):
                record, skips = self.build(read_name, "empty", empty_payload)
                if field == "required_gate":
                    self.assertIs(record[field]["configured"], False, EMPTY_IS_AN_ANSWER[field])
                    self.assertIn("/rules/branches/", record[field]["source_endpoint"])
                elif field == "closing_issue":
                    self.assertIs(record[field]["present"], False, EMPTY_IS_AN_ANSWER[field])
                    self.assertEqual(record[field]["issue_numbers"], [])
                else:
                    self.assert_not_a_value(field, record, skips)

    def test_the_two_empty_is_an_answer_exceptions_are_the_only_ones(self):
        self.assertEqual(set(EMPTY_IS_AN_ANSWER), {"required_gate", "closing_issue"})
        self.assertEqual(
            set(FIELD_SOURCES) | {"skips"},
            set(core.RECORD_FIELDS),
            "every record field is either degraded above or is the skip register itself",
        )

    def test_the_skip_register_reports_every_degradation(self):
        """`skips` is the seventh field: it has no read, it has this instead."""
        for field, (read_name, _) in FIELD_SOURCES.items():
            with self.subTest(field=field):
                _, skips = self.build(read_name, "malformed")
                self.assertTrue(skips, f"{field} was degraded and the register is empty")
                for entry in skips:
                    self.assertEqual(sorted(entry), ["detail", "endpoint", "field", "reason"])

    def test_a_clean_run_still_publishes_the_skip_it_has(self):
        """PR 86 is the readable case and still cannot see org rulesets."""
        record = core.build_record(reads())
        self.assertEqual(
            [s["field"] for s in record["skips"]], ["required_gate.org_rulesets"]
        )


class NoNetwork(unittest.TestCase):
    """STEP 9 — the suite cannot reach GitHub even if a control tried to."""

    def test_no_control_can_spawn_a_process(self):
        """Break subprocess entirely, then run the whole CLI through the fake."""
        def explode(*args, **kwargs):
            raise AssertionError("a control tried to spawn a real process")

        original = fetch.subprocess.run
        fetch.subprocess.run = explode
        try:
            code, out, _ = run_cli(["86"], FakeGh())
        finally:
            fetch.subprocess.run = original
        self.assertEqual(code, 0)
        self.assertEqual(json.loads(out)["pr"]["number"], 86)

    def test_the_real_runner_refuses_any_binary_but_gh(self):
        with self.assertRaises(ValueError):
            fetch.gh_runner(["curl", "https://example.invalid"])
        with self.assertRaises(ValueError):
            fetch.gh_runner([])

    def test_gh_is_the_only_binary_the_runner_is_asked_for(self):
        fake = FakeGh()
        run_cli(["86"], fake)
        self.assertEqual(set(fake.binaries), {"gh"})
        self.assertEqual(len(fake.calls), 8, "eight reads, and no call made twice")


if __name__ == "__main__":
    unittest.main()
