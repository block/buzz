"""The pre-flight's fetch layer and CLI — launchpad-26/buzz#116.

This module is where the talking to ``gh`` happens, and the only place in the
tree that spawns a process. It exists as a separate module from the CLI script so
that controls can ``import`` it: ``pr-preflight.py`` cannot be imported under
that name, and a fetch layer no control can drive is a fetch layer no control can
make fail.

**The runner is injected.** Every call goes through a ``runner(argv) ->
RunResult`` callable that defaults to :func:`gh_runner`. Controls pass a fake that
answers from recorded fixtures, so the whole surface — including each way a fetch
can fail — is exercisable with no network. Hardcoding ``subprocess.run(["gh",
...])` at each call site would make that impossible without a rewrite.

Nine reads, five of them required
================================

============  ======================================================  =========
name          call                                                    kind
============  ======================================================  =========
pr            ``GET /repos/{o}/{r}/pulls/{n}``                         required
meta          ``gh pr view {n} --json title,body,labels``              required
compare       ``GET /repos/{o}/{r}/compare/{base.sha}...{head.sha}``   required
checks        GraphQL ``statusCheckRollup`` with ``isRequired``        required
tree          ``GET /repos/{o}/{r}/git/trees/{head}?recursive=1``      required
branch_rules  ``GET /repos/{o}/{r}/rules/branches/{base}``             skip-only
org_rulesets  ``GET /orgs/{o}/rulesets``                              skip-only
closing_refs  GraphQL ``closingIssuesReferences``                      skip-only
review_decis  ``gh pr view {n} --json reviewDecision``                  skip-only
============  ======================================================  =========

``review_decision`` is the only gate signal this token can read: §6's ruleset is
invisible without ``admin:org``, and ``reviewDecision`` confirms review is
*required* without exposing the count.

``compare`` is taken **by SHA**, ``base.sha...head.sha``, never by base branch
name: a name resolves to the tip today, and for a merged PR that tip already
contains the head, so the API answers ``200 OK`` with zero files — a real PR
rendered as one that changed nothing.

Exit codes
==========

===  =====================================================================
0    a record was emitted on stdout. It may carry SKIP-only entries
1    usage error — a bad argument, nothing was fetched
2    a REQUIRED input could not be read. Nothing is printed on stdout
===  =====================================================================

Exit 2 prints the skip register on **stderr** and nothing on stdout, so a caller
that pipes stdout into a reviewer never receives a record with holes in it.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from dataclasses import dataclass
from typing import Callable, Mapping

from preflight_core import (
    ABSENT,
    FORBIDDEN,
    MALFORMED,
    REQUIRED_INPUTS,
    SKIP_ONLY_INPUTS,
    SKIP_REASONS,
    UNREACHABLE,
    Read,
    RecordError,
    build_record,
)

DEFAULT_REPO = "launchpad-26/buzz"

#: The one binary this tool is allowed to spawn.
BINARY = "gh"

_HTTP_STATUS = re.compile(r"\(HTTP (\d{3})\)")

#: GraphQL says a thing does not exist in prose, with no HTTP status attached:
#: `gh pr view 999999` fails with "GraphQL: Could not resolve to a PullRequest
#: with the number of 999999." Without this, a definitive "it is not there" is
#: classified as unreachable — the exact confusion between "we could not ask" and
#: "we asked and it is absent" that the taxonomy exists to prevent.
_GRAPHQL_ABSENT = re.compile(
    r"could not resolve to a|no pull requests found|could not find any pull request",
    re.I,
)

#: GraphQL error `type` -> the reason it means. Checked in this order, because a
#: response can carry several errors and the most specific failure should win.
#: Mapping everything but FORBIDDEN onto `malformed` would report a definite
#: "this does not exist" as a response nobody could parse.
GRAPHQL_ERROR_REASONS = {
    "FORBIDDEN": FORBIDDEN,
    "NOT_FOUND": ABSENT,
    "RATE_LIMITED": UNREACHABLE,
    "SERVICE_UNAVAILABLE": UNREACHABLE,
}


@dataclass(frozen=True)
class RunResult:
    returncode: int
    stdout: str
    stderr: str


def gh_runner(argv: list[str]) -> RunResult:
    """Run ``gh``. The only process spawn in the pre-flight.

    Refuses any other binary rather than trusting its caller: this function is
    the seam where an injected runner could otherwise be talked into spawning
    something else.
    """
    if not argv or argv[0] != BINARY:
        raise ValueError(f"the pre-flight may only spawn {BINARY!r}, not {argv[:1]!r}")
    try:
        proc = subprocess.run(argv, capture_output=True, timeout=60, check=False)
    except subprocess.TimeoutExpired:
        return RunResult(124, "", f"{BINARY} timed out after 60s")
    except OSError as exc:
        # Every OSError, not only FileNotFoundError. An uncaught one exits 1 with
        # a traceback, and 1 is this tool's "usage error" code — so a broken gh
        # install would tell a caller its arguments were bad. Three reproduced
        # triggers, none of them a missing binary:
        #   EACCES  a gh on PATH that is not executable
        #   EACCES  a *directory* named gh on PATH
        #   ENOEXEC a gh that is executable but not a runnable binary — a
        #           truncated download or the wrong architecture, which is the
        #           plausible one in practice
        # A non-executable gh with a working gh later on PATH is harmless:
        # CPython keeps walking PATH and only reports the saved EACCES if no
        # entry succeeds.
        return RunResult(127, "", f"cannot run {BINARY}: {type(exc).__name__}: {exc}")
    return RunResult(
        proc.returncode,
        proc.stdout.decode("utf-8", "replace"),
        proc.stderr.decode("utf-8", "replace"),
    )


Runner = Callable[[list[str]], RunResult]


def _classify_failure(name: str, result: RunResult) -> tuple[str, str]:
    """Map a failed ``gh`` call onto one enumerated reason and a detail."""
    status = _HTTP_STATUS.search(result.stderr or "")
    detail = (result.stderr or "").strip().splitlines()
    detail = detail[-1] if detail else f"{BINARY} exited {result.returncode}"

    if result.returncode in (124, 127):
        return UNREACHABLE, detail
    if status:
        code = int(status.group(1))
        if code in (401, 403):
            return FORBIDDEN, detail
        if code == 404:
            # A 404 on org rulesets hides access; it does not report absence.
            # launchpad-26 is an Organization, verified, so "not found" here can
            # only mean this token cannot see it — reporting that as absent would
            # let a scope failure read as "this org configures no rulesets".
            if name == "org_rulesets":
                return FORBIDDEN, f"{detail} (404 for an organization that exists: no admin:org)"
            return ABSENT, detail
        if 500 <= code <= 599:
            return UNREACHABLE, detail
        return MALFORMED, detail
    if _GRAPHQL_ABSENT.search(result.stderr or ""):
        return ABSENT, detail
    return UNREACHABLE, detail


def _read(name: str, endpoint: str, argv: list[str], runner: Runner) -> Read:
    """Perform one read and classify its outcome. Never raises on failure."""
    try:
        result = runner(argv)
    except FileNotFoundError as exc:  # a runner that cannot start the binary
        return Read(name, skip=UNREACHABLE, detail=str(exc), endpoint=endpoint)

    if result.returncode != 0:
        reason, detail = _classify_failure(name, result)
        return Read(name, skip=reason, detail=detail, endpoint=endpoint)

    try:
        data = json.loads(result.stdout)
    except json.JSONDecodeError as exc:
        return Read(name, skip=MALFORMED, detail=f"not JSON: {exc}", endpoint=endpoint)

    # An errors array arriving with a SUCCESSFUL exit code — a failure wearing a
    # success's clothes.
    #
    # `gh` never does this: it exits non-zero whenever the response carries
    # errors, including the partial case where some aliases resolved and others
    # did not, so through the real binary this branch is unreachable and a
    # nonexistent PR is classified `absent` from gh's prose instead. Verified
    # against gh 2.93.0.
    #
    # It is kept for the INJECTED runner, which is the documented extension point
    # (see INTERFACE.md on adopting #120's fetch.fetch_all): a runner that speaks
    # to GitHub directly rather than through gh returns HTTP 200 with an errors
    # body and a zero status, and without this the errors array would be handed
    # on as if it were data. Both halves — the guard and the type mapping — are
    # driven by controls through a fake runner, because nothing else can drive
    # them.
    if isinstance(data, Mapping) and data.get("errors"):
        types = {e.get("type") for e in data["errors"] if isinstance(e, Mapping)}
        messages = "; ".join(
            str(e.get("message", "")) for e in data["errors"] if isinstance(e, Mapping)
        )
        reason = MALFORMED
        for error_type, mapped in GRAPHQL_ERROR_REASONS.items():
            if error_type in types:
                reason = mapped
                break
        return Read(name, skip=reason, detail=f"graphql errors: {messages}", endpoint=endpoint)

    return Read(name, data=data, endpoint=endpoint)


#: GitHub's own answer to what a PR closes. The only reliable one: a pattern over
#: the body reports a reference written inside code, misses second and later
#: references, and says a PR closes an issue when its base is not the default
#: branch and merging it will close nothing.
CLOSING_QUERY = """
query($owner:String!,$repo:String!,$pr:Int!){
  repository(owner:$owner,name:$repo){
    pullRequest(number:$pr){closingIssuesReferences(first:50){nodes{number}}}}}
"""

CHECKS_QUERY = """
query($owner:String!,$repo:String!,$pr:Int!){
  repository(owner:$owner,name:$repo){
    pullRequest(number:$pr){
      commits(last:1){nodes{commit{oid statusCheckRollup{state contexts(first:100){totalCount nodes{
        __typename
        ... on CheckRun{name status conclusion detailsUrl isRequired(pullRequestNumber:$pr) checkSuite{workflowRun{workflow{name}}}}
        ... on StatusContext{context state targetUrl isRequired(pullRequestNumber:$pr)}
      }}}}}}
    }
  }
}
"""


def fetch_all(number: int, repo: str, runner: Runner) -> dict[str, Read]:
    """Perform all nine reads. A failure in one never aborts the others."""
    owner, _, name = repo.partition("/")
    reads: dict[str, Read] = {}

    reads["pr"] = _read(
        "pr",
        f"GET /repos/{repo}/pulls/{number}",
        [BINARY, "api", f"repos/{repo}/pulls/{number}"],
        runner,
    )
    reads["meta"] = _read(
        "meta",
        f"gh pr view {number} --json title,body,labels",
        [BINARY, "pr", "view", str(number), "--repo", repo, "--json", "title,body,labels"],
        runner,
    )
    reads["checks"] = _read(
        "checks",
        "graphql:statusCheckRollup",
        [
            BINARY, "api", "graphql",
            "-f", f"owner={owner}",
            "-f", f"repo={name}",
            "-F", f"pr={number}",
            "-f", f"query={CHECKS_QUERY}",
        ],
        runner,
    )

    # The one gate signal readable without admin:org. launchpad/AGENTS.md §6: the
    # ruleset requiring two approving reviews is invisible to this token, but a
    # live PR's reviewDecision confirms review IS required — and it differs by
    # base, so it cannot be assumed from the repository alone.
    reads["review_decision"] = _read(
        "review_decision",
        f"gh pr view {number} --json reviewDecision",
        [BINARY, "pr", "view", str(number), "--repo", repo, "--json", "reviewDecision"],
        runner,
    )
    reads["closing_refs"] = _read(
        "closing_refs",
        "graphql:closingIssuesReferences",
        [
            BINARY, "api", "graphql",
            "-f", f"owner={owner}",
            "-f", f"repo={name}",
            "-F", f"pr={number}",
            "-f", f"query={CLOSING_QUERY}",
        ],
        runner,
    )

    # compare and tree need the PR's own base and head SHAs. Without them the
    # calls cannot be made at all, and "not attempted" is a different fact from
    # "answered 404" — inventing a base ref to fetch anyway would be worse.
    base_sha = head_sha = base_ref = None
    if reads["pr"].ok and isinstance(reads["pr"].data, Mapping):
        base_sha = (reads["pr"].data.get("base") or {}).get("sha")
        base_ref = (reads["pr"].data.get("base") or {}).get("ref")
        head_sha = (reads["pr"].data.get("head") or {}).get("sha")

    not_attempted = "not attempted: the PR read did not yield base.sha and head.sha"
    if base_sha and head_sha:
        reads["compare"] = _read(
            "compare",
            f"GET /repos/{repo}/compare/{base_sha}...{head_sha}",
            [BINARY, "api", f"repos/{repo}/compare/{base_sha}...{head_sha}"],
            runner,
        )
        reads["tree"] = _read(
            "tree",
            f"GET /repos/{repo}/git/trees/{head_sha}?recursive=1",
            [BINARY, "api", f"repos/{repo}/git/trees/{head_sha}?recursive=1"],
            runner,
        )
    else:
        reads["compare"] = Read("compare", skip=UNREACHABLE, detail=not_attempted)
        reads["tree"] = Read("tree", skip=UNREACHABLE, detail=not_attempted)

    if base_ref:
        reads["branch_rules"] = _read(
            "branch_rules",
            f"GET /repos/{repo}/rules/branches/{base_ref}",
            [BINARY, "api", f"repos/{repo}/rules/branches/{base_ref}"],
            runner,
        )
    else:
        reads["branch_rules"] = Read("branch_rules", skip=UNREACHABLE, detail=not_attempted)

    reads["org_rulesets"] = _read(
        "org_rulesets",
        f"GET /orgs/{owner}/rulesets",
        [BINARY, "api", f"orgs/{owner}/rulesets"],
        runner,
    )
    return reads


def _epilogue() -> str:
    """The taxonomy and the exit contract, in --help as well as the docstring."""
    reasons = "\n".join(f"  {name:<13} {why}" for name, why in SKIP_REASONS.items())
    return (
        "SKIP reasons, one per unreadable input:\n"
        f"{reasons}\n\n"
        "Required inputs — a failure to read any of these exits 2, and no record\n"
        "is printed:\n"
        f"  {', '.join(REQUIRED_INPUTS)}\n\n"
        "Skip-only inputs — absence is a reportable state and the exit stays 0:\n"
        f"  {', '.join(SKIP_ONLY_INPUTS)}\n"
        "  a changed path with no ancestor AGENTS.md or CLAUDE.md\n\n"
        "A truncated head tree is a required-input FAILURE, not an empty result:\n"
        "the trees API answers HTTP 200 with truncated: true and a partial list,\n"
        "so a path whose rules file sits past the boundary is indistinguishable\n"
        "from a path that has none.\n\n"
        "Exit codes: 0 record emitted, 1 usage error, 2 a required input was unreadable.\n"
    )


class _Parser(argparse.ArgumentParser):
    """argparse exits 2 on a usage error, which is this tool's "unreadable
    required input" code. Two very different failures sharing an exit code makes
    the contract untestable, so usage errors are moved to 1 and 2 keeps its
    meaning."""

    def error(self, message: str):  # noqa: D102 - argparse's own contract
        self.print_usage(sys.stderr)
        sys.stderr.write(f"{self.prog}: error: {message}\n")
        raise SystemExit(1)


def build_parser() -> argparse.ArgumentParser:
    parser = _Parser(
        prog="pr-preflight.py",
        description=(
            "Emit the deterministic facts a PR review needs, as JSON on stdout. "
            "Makes no model call and reaches no conclusion about the code: "
            "author-controlled text is carried through as labelled data."
        ),
        epilog=_epilogue(),
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("number", type=int, help="the pull request number")
    parser.add_argument("--repo", default=DEFAULT_REPO, help=f"owner/name (default {DEFAULT_REPO})")
    parser.add_argument("--indent", type=int, default=2, help="JSON indent; 0 for one line")
    return parser


def main(argv: list[str] | None = None, runner: Runner | None = None) -> int:
    """Fetch, build, print. Returns the process exit code."""
    parser = build_parser()
    args = parser.parse_args(argv)
    if args.number <= 0:
        parser.error("PR number must be positive")

    reads = fetch_all(args.number, args.repo, runner or gh_runner)
    try:
        record = build_record(reads)
    except RecordError as failure:
        json.dump(
            {"error": "a required input could not be read", "skips": failure.skips},
            sys.stderr,
            indent=2,
        )
        sys.stderr.write("\n")
        return 2

    json.dump(record, sys.stdout, indent=args.indent or None)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":  # pragma: no cover - exercised via pr-preflight.py
    sys.exit(main())
