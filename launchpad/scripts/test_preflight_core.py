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


if __name__ == "__main__":
    unittest.main()
