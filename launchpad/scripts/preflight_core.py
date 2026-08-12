"""Deterministic pre-flight for the PR review agent — launchpad-26/buzz#116.

Pure functions over already-fetched data. **No subprocess, no network, and no
model call happens in this module** — every input arrives as a :class:`Read` that
somebody else performed, which is what makes the whole surface testable from
recorded fixtures. ``preflight_fetch`` does the talking to ``gh``.

The record
==========

Seven top-level fields, enumerated here so that a later change has a fixed list
to be checked against rather than an open-ended "every field". **Adding a field
to the record means adding it to this list in the same commit.**

==================  ========================================================
``pr``              number, title, body, labels, base_ref, head_sha
``closing_issue``   present, keyword, issue_numbers, source, text_disagrees,
                    text_issue_numbers
``diff``            merge_base_sha, head_sha, files[path, added, removed, status]
``checks``          [name, workflow, status, conclusion, required, details_url]
``required_gate``   configured, source_endpoint
``nearest_rules``   per changed path, the resolved AGENTS.md and CLAUDE.md
``skips``           [field, reason, detail, endpoint]
==================  ========================================================

``checks`` is a **list, never a name-keyed map.** PR 86 carries three checks all
named ``check``; a map would silently drop two of them and the record would
under-report the gate.

The SKIP taxonomy
=================

One enumerated reason per unreadable input. The distinctions exist because
collapsing them is how "we could not look" gets published as "there is nothing
there".

===============  =========================================================
``absent``       the API said 404 — the thing is not there
``forbidden``    403, or a 404 that hides access rather than reporting
                 absence (``GET /orgs/{org}/rulesets`` without ``admin:org``
                 answers 404 for an organization that demonstrably exists)
``malformed``    not JSON, or JSON whose shape the reader cannot use
``empty``        read successfully, and genuinely carries nothing
``truncated``    the API knowingly returned part of the answer — HTTP 200
                 with ``truncated: true``, which is a failed read wearing a
                 success's clothes
``unreachable``  the call was never completed: no ``gh`` binary, a timeout, a
                 transport error. Distinct from ``absent`` because "we never
                 asked" must not be reported as "it does not exist"
===============  =========================================================

The exit contract
=================

A field is one of two kinds, and the kinds decide the exit code. Without this
split the "exit non-zero when a required input cannot be read" rule is
meaningless, and "SKIP everything, always exit 0" would satisfy every other
criterion this script has.

**REQUIRED** — a failure to read exits non-zero. Five inputs:

``pr``
    the pull request itself: does it exist, what did it fork from
``meta``
    its title, body and labels
``compare``
    the merge-base comparison the diff is read from
``checks``
    the check list
``tree``
    the head-tree listing ``nearest_rules`` resolves against

**SKIP-ONLY** — absence is a legitimate reportable state and the exit stays 0:

``org_rulesets``
    org-level ruleset visibility, unreadable without ``admin:org``
``branch_rules``
    repo-level rules for the base branch. Readable and empty is a *fact*: it
    means no gate is configured, reported as ``configured: false`` naming the
    endpoint that said so, never as a silent zero
``closing_refs``
    GitHub's own answer to what the PR closes. Unreadable yields
    ``closing_issue.present: None`` — unknown, never False, because "we could not
    ask" and "it closes nothing" differ on whether the board updates on merge
*a path with no ancestor rules file at all*
    a real answer, not a failure

**``empty`` is never fatal, whichever kind the field belongs to.** An empty
answer was read. A head commit with no check runs yet and a check list that could
not be fetched are different facts, and only the second is a failure.

``truncated`` is why those two kinds do not blur. The trees API does not fail
loudly when it cannot return a whole tree: it answers HTTP 200 with
``truncated: true`` and a partial list past roughly 100,000 entries or 7 MB. A
path whose rules file sits beyond that boundary then looks exactly like the
SKIP-ONLY "no ancestor file" case while the real cause is an incomplete REQUIRED
read. So a truncated tree is a **tree-listing failure** and exits non-zero. This
fork is nowhere near the limit — 4337 entries at the time of writing — but this
is a CLI that takes any PR number, so the guard is required, not hypothetical.

That classification is a judgement, not a fact, and a reviewer may move any line
of it.

Containment
===========

Author-controlled text (title, body, per-file paths) is carried through as
**data** and never interpreted: it lands in labelled JSON string fields, which
CONTAINMENT.md § Contract for later stages permits for a stage that builds no
prompt. That permission is contingent on this module making no model call, so if
that ever changes, containment moves in with it. See ``INTERFACE.md``.
"""

from __future__ import annotations

import re
from dataclasses import dataclass, field
from typing import Any, Iterable, Mapping

# --------------------------------------------------------------------------- #
# The taxonomy, as values. Written once so the docstring, --help and the
# controls all read the same list rather than three copies that drift.
# --------------------------------------------------------------------------- #

ABSENT = "absent"
FORBIDDEN = "forbidden"
MALFORMED = "malformed"
EMPTY = "empty"
TRUNCATED = "truncated"
UNREACHABLE = "unreachable"

SKIP_REASONS: dict[str, str] = {
    ABSENT: "the API said 404 — the thing is not there",
    FORBIDDEN: "403, or a 404 that hides access rather than reporting absence",
    MALFORMED: "not JSON, or JSON whose shape the reader cannot use",
    EMPTY: "read successfully, and genuinely carries nothing",
    TRUNCATED: "the API knowingly returned part of the answer (HTTP 200 + truncated: true)",
    UNREACHABLE: "the call was never completed — no gh binary, a timeout, a transport error",
}

#: Inputs whose failure is fatal. See § The exit contract.
REQUIRED_INPUTS = ("pr", "meta", "compare", "checks", "tree")

#: Inputs whose absence is a reportable state, not a failure.
SKIP_ONLY_INPUTS = ("branch_rules", "org_rulesets", "closing_refs")

#: The record's top-level fields, in order. STEP 2 of the plan fixes this list.
RECORD_FIELDS = (
    "pr",
    "closing_issue",
    "diff",
    "checks",
    "required_gate",
    "nearest_rules",
    "skips",
)

#: Filenames that count as rules files. Compared as whole basenames, never with
#: endswith: `VISION_REMOTE_AGENTS.md` ends with "AGENTS.md" and is not one.
RULES_FILENAMES = ("AGENTS.md", "CLAUDE.md")

#: The launchpad PR-body check's own pattern and comment-stripping order, so the
#: pre-flight and the check cannot disagree about what a closing keyword is.
CLOSING_KEYWORD = re.compile(r"\b(Closes|Fixes|Resolves)\s+#(\d+)", re.I)
HTML_COMMENT = re.compile(r"<!--.*?-->", re.S)


@dataclass(frozen=True)
class Read:
    """One attempt to read one input.

    ``skip`` is None exactly when the read succeeded. A ``Read`` with data *and*
    a skip reason is a contradiction and :func:`build_record` refuses it.
    """

    name: str
    data: Any = None
    skip: str | None = None
    detail: str = ""
    endpoint: str = ""

    def __post_init__(self) -> None:
        if self.skip is not None and self.skip not in SKIP_REASONS:
            raise ValueError(f"unenumerated skip reason: {self.skip!r}")

    @property
    def ok(self) -> bool:
        return self.skip is None


@dataclass
class Skips:
    """The register of everything that could not be read, and why."""

    entries: list[dict[str, str]] = field(default_factory=list)

    def add(self, field_name: str, read: Read) -> None:
        self.entries.append(
            {
                "field": field_name,
                "reason": read.skip or MALFORMED,
                "detail": read.detail or SKIP_REASONS.get(read.skip or "", ""),
                "endpoint": read.endpoint,
            }
        )

    def reasons(self) -> set[str]:
        return {e["reason"] for e in self.entries}


class RecordError(Exception):
    """A REQUIRED input could not be read. The caller must exit non-zero."""

    def __init__(self, skips: list[dict[str, str]]) -> None:
        super().__init__("; ".join(f"{s['field']}: {s['reason']}" for s in skips))
        self.skips = skips


def _get(mapping: Any, *path: str, default: Any = None) -> Any:
    """Walk nested mappings without raising on a shape that is not there."""
    current = mapping
    for key in path:
        if not isinstance(current, Mapping) or key not in current:
            return default
        current = current[key]
    return current if current is not None else default


# --------------------------------------------------------------------------- #
# Section builders. Each takes the reads it needs and either returns a value or
# records a skip and returns None. None means "not read"; it never means "empty".
# --------------------------------------------------------------------------- #


def build_pr(pr: Read, meta: Read, skips: Skips) -> dict[str, Any] | None:
    if not pr.ok:
        skips.add("pr", pr)
        return None
    if not meta.ok:
        skips.add("pr", meta)
        return None

    number = _get(pr.data, "number")
    base_ref = _get(pr.data, "base", "ref")
    head_sha = _get(pr.data, "head", "sha")
    if number is None or not base_ref or not head_sha:
        skips.add("pr", Read("pr", skip=MALFORMED, detail="no number, base.ref or head.sha", endpoint=pr.endpoint))
        return None

    labels = _get(meta.data, "labels", default=[])
    if not isinstance(labels, list):
        skips.add("pr", Read("meta", skip=MALFORMED, detail="labels is not a list", endpoint=meta.endpoint))
        return None

    title = _get(meta.data, "title")
    if title is None:
        skips.add("pr", Read("meta", skip=MALFORMED, detail="no title field", endpoint=meta.endpoint))
        return None

    return {
        "number": number,
        "title": title,
        # An absent body is "" from GitHub and None from a hand-built payload.
        # Both mean the author wrote nothing; neither is a failed read.
        "body": _get(meta.data, "body", default="") or "",
        "labels": [lab.get("name") if isinstance(lab, Mapping) else lab for lab in labels],
        "base_ref": base_ref,
        "head_sha": head_sha,
    }


def build_closing_issue(meta: Read, closing_refs: Read, skips: Skips) -> dict[str, Any] | None:
    """Whether merging this PR would close an issue — asked of GitHub, not of a regex.

    **GitHub decides.** ``closingIssuesReferences`` is the only reliable answer,
    and a pattern over the body disagrees with it in both directions on real pull
    requests in this repository:

    - PR 86's body writes one ``Closes #79``; GitHub reports #79 **and** #91, so a
      first-match regex under-reports what the merge will do.
    - PR 92's body writes a visible ``Closes #n`` and GitHub reports **nothing**,
      because the PR's base was not the default branch — merging it would close
      no issue at all.
    - A reference written inside code is ignored by GitHub and matched by a
      regex. CommonMark has more code forms than a pattern enumerates; four of
      them defeated the launchpad PR-body check (launchpad-26/buzz#125).

    The body is still read, for two things a reference list cannot give: **which
    keyword** the author wrote, and whether the text and GitHub **disagree**. A
    disagreement is reportable signal — it is exactly the shape of #125 — so it
    is recorded rather than resolved silently.

    HTML comments are stripped before the text is scanned, in the PR-body check's
    order, so an unfilled ``<!-- Fixes #1234 -->`` placeholder is not read as a
    reference by the text half either.

    An unreadable ``closingIssuesReferences`` yields ``present: None`` — unknown.
    Never False. "We could not ask" and "it closes nothing" are different facts,
    and only one of them means the board will not update.
    """
    if not meta.ok:
        skips.add("closing_issue", meta)
        return None
    body = _get(meta.data, "body", default="")
    if body is None:
        body = ""
    if not isinstance(body, str):
        skips.add(
            "closing_issue",
            Read("meta", skip=MALFORMED, detail="body is not a string", endpoint=meta.endpoint),
        )
        return None

    visible = HTML_COMMENT.sub("", body)
    matches = list(CLOSING_KEYWORD.finditer(visible))
    keyword = matches[0].group(1) if matches else None
    text_numbers = sorted({int(m.group(2)) for m in matches})

    section: dict[str, Any] = {
        "present": None,
        "keyword": keyword,
        "issue_numbers": None,
        "source": None,
        "text_disagrees": None,
        "text_issue_numbers": text_numbers,
    }

    if not closing_refs.ok:
        skips.add("closing_issue.closing_refs", closing_refs)
        section["source"] = "unresolved: GitHub's own answer could not be read"
        return section

    nodes = _get(
        closing_refs.data,
        "data", "repository", "pullRequest", "closingIssuesReferences", "nodes",
    )
    if not isinstance(nodes, list):
        skips.add(
            "closing_issue.closing_refs",
            Read(
                "closing_refs",
                skip=MALFORMED,
                detail="closingIssuesReferences.nodes is not a list",
                endpoint=closing_refs.endpoint,
            ),
        )
        section["source"] = "unresolved: GitHub's own answer could not be read"
        return section

    numbers = sorted({n["number"] for n in nodes if isinstance(n, Mapping) and "number" in n})
    section.update(
        {
            "present": bool(numbers),
            "issue_numbers": numbers,
            "source": "graphql:closingIssuesReferences",
            "text_disagrees": set(numbers) != set(text_numbers),
        }
    )
    return section


def build_diff(compare: Read, pr_section: Mapping[str, Any] | None, skips: Skips) -> dict[str, Any] | None:
    """Per-file counts against the **merge base**, not the base branch tip.

    ``baseRefOid`` is the current tip of the base branch, not the commit the head
    branch forked from. Diffing against it attributes every commit landed on the
    base since the fork to this PR's author, in reverse. The merge base is
    REST-only: ``GET /repos/{o}/{r}/compare/{base}...{head}`` ->
    ``.merge_base_commit.sha``. There is no ``mergeBaseCommit`` field on the
    GraphQL ``PullRequest`` type.
    """
    if not compare.ok:
        skips.add("diff", compare)
        return None

    merge_base = _get(compare.data, "merge_base_commit", "sha")
    if not merge_base:
        skips.add(
            "diff",
            Read("compare", skip=MALFORMED, detail="no merge_base_commit.sha", endpoint=compare.endpoint),
        )
        return None

    files = _get(compare.data, "files")
    if not isinstance(files, list):
        skips.add(
            "diff",
            Read("compare", skip=MALFORMED, detail="files is not a list", endpoint=compare.endpoint),
        )
        return None

    entries = []
    for item in files:
        path = _get(item, "filename")
        if not path:
            skips.add(
                "diff",
                Read("compare", skip=MALFORMED, detail="a file entry has no filename", endpoint=compare.endpoint),
            )
            return None
        entries.append(
            {
                "path": path,
                "added": _get(item, "additions", default=0),
                "removed": _get(item, "deletions", default=0),
                "status": _get(item, "status", default="unknown"),
            }
        )

    return {
        "merge_base_sha": merge_base,
        # The head sha the comparison was actually taken at, so the record is
        # pinned to the commit pair it read rather than to whatever HEAD is now.
        "head_sha": _get(pr_section or {}, "head_sha"),
        "files": entries,
    }


def _normalise_check(node: Mapping[str, Any]) -> dict[str, Any]:
    """One rollup context, flattened.

    Two shapes arrive. A ``CheckRun`` has a name, a status and a conclusion. A
    ``StatusContext`` — the older commit-status API — has a context and a single
    state and no workflow at all; its state is mapped onto ``conclusion`` because
    that is what it means, and ``status`` stays None rather than being invented.
    """
    if node.get("__typename") == "StatusContext":
        return {
            "name": node.get("context"),
            "workflow": None,
            "status": None,
            "conclusion": node.get("state"),
            "required": node.get("isRequired"),
            "details_url": node.get("targetUrl"),
        }
    return {
        "name": node.get("name"),
        "workflow": _get(node, "checkSuite", "workflowRun", "workflow", "name"),
        "status": node.get("status"),
        "conclusion": node.get("conclusion"),
        "required": node.get("isRequired"),
        "details_url": node.get("detailsUrl"),
    }


def build_checks(checks: Read, skips: Skips) -> list[dict[str, Any]] | None:
    if not checks.ok:
        skips.add("checks", checks)
        return None

    nodes = _get(
        checks.data,
        "data", "repository", "pullRequest", "commits", "nodes",
    )
    if not isinstance(nodes, list) or not nodes:
        skips.add(
            "checks",
            Read("checks", skip=MALFORMED, detail="no commit nodes in the rollup", endpoint=checks.endpoint),
        )
        return None

    rollup = _get(nodes[0], "commit", "statusCheckRollup")
    if rollup is None:
        # GitHub returns null here when the head commit has no checks at all.
        # That is readable and empty, not malformed.
        skips.add(
            "checks",
            Read("checks", skip=EMPTY, detail="the head commit has no check rollup", endpoint=checks.endpoint),
        )
        return None

    contexts = _get(rollup, "contexts", "nodes")
    if not isinstance(contexts, list):
        skips.add(
            "checks",
            Read("checks", skip=MALFORMED, detail="contexts.nodes is not a list", endpoint=checks.endpoint),
        )
        return None
    if not contexts:
        skips.add(
            "checks",
            Read("checks", skip=EMPTY, detail="the rollup lists no contexts", endpoint=checks.endpoint),
        )
        return None

    # A list, in the order the API gave them. Names collide; position does not.
    return [_normalise_check(node) for node in contexts]


def build_required_gate(
    branch_rules: Read,
    org_rulesets: Read,
    checks_section: Iterable[Mapping[str, Any]] | None,
    skips: Skips,
) -> dict[str, Any]:
    """Whether any check is required, and which endpoint said so.

    An empty required set is reported as ``configured: false`` **naming the
    endpoint that answered**, never as a silent zero — a zero is
    indistinguishable from a scope failure, and this repository is currently in
    exactly the state where that matters: three endpoints report no protection on
    ``launchpad`` while ``launchpad/AGENTS.md`` §6 says the branch is protected.
    Reporting the absence is this script's job; resolving the contradiction is a
    cohort decision.

    **What ``configured: false`` does and does not mean.** It means nothing
    *visible to this token* requires a check. Two blind spots survive, and both
    are why the org-ruleset skip must be published alongside the false rather
    than instead of it:

    - An org-level ruleset is unreadable without ``admin:org``. Its effect leaks
      through per-context ``isRequired``, which is why that is consulted too.
    - A required context that **never ran** is absent from the rollup entirely,
      so ``isRequired`` cannot report it. A ruleset requiring a check that never
      started is therefore invisible from here.

    So ``configured: false`` is a statement about what could be seen. The skip
    entry beside it is what stops that being read as a statement about what is.
    """
    if not org_rulesets.ok:
        # SKIP-ONLY. Never "no org rulesets exist" — this token cannot tell.
        skips.add("required_gate.org_rulesets", org_rulesets)

    required_by_check = [
        c.get("name") for c in (checks_section or []) if c.get("required") is True
    ]

    if not branch_rules.ok:
        skips.add("required_gate", branch_rules)
        return {
            "configured": None if not required_by_check else True,
            "source_endpoint": branch_rules.endpoint if not required_by_check else "graphql:isRequired",
        }

    if not isinstance(branch_rules.data, list):
        # A shape the reader cannot use is NOT an empty rules list. Coercing it to
        # [] here would publish `configured: false` — a definite answer — from a
        # response nobody could read.
        skips.add(
            "required_gate",
            Read(
                "branch_rules",
                skip=MALFORMED,
                detail=f"the rules probe returned {type(branch_rules.data).__name__}, not a list",
                endpoint=branch_rules.endpoint,
            ),
        )
        return {
            "configured": True if required_by_check else None,
            "source_endpoint": "graphql:isRequired" if required_by_check else branch_rules.endpoint,
        }

    rules = branch_rules.data
    rule_types = {_get(r, "type") for r in rules if isinstance(r, Mapping)}
    by_rules = "required_status_checks" in rule_types

    if by_rules:
        return {"configured": True, "source_endpoint": branch_rules.endpoint}
    if required_by_check:
        # The rules probe saw nothing but GraphQL marks contexts required, which
        # is what an org-level ruleset looks like from underneath.
        return {"configured": True, "source_endpoint": "graphql:isRequired"}
    return {"configured": False, "source_endpoint": branch_rules.endpoint}


def _nearest(path: str, filename: str, tree_paths: frozenset[str]) -> str | None:
    """The nearest ``filename`` at or above ``path``'s directory, or None."""
    parts = path.split("/")[:-1]
    while True:
        candidate = "/".join([*parts, filename])
        if candidate in tree_paths:
            return candidate
        if not parts:
            return None
        parts.pop()


def build_nearest_rules(
    tree: Read,
    diff_section: Mapping[str, Any] | None,
    skips: Skips,
) -> dict[str, dict[str, str | None]] | None:
    """For each changed path, the nearest AGENTS.md **and** the nearest CLAUDE.md.

    Both, not first-wins. ``launchpad/`` holds only an AGENTS.md while the root
    holds both, so first-wins would hide the root CLAUDE.md from every path under
    ``launchpad/``.

    Resolved against the **PR's head tree**, never the local worktree — the
    answer is wrong for any PR that is not checked out, and it is wrong in the
    direction that matters when the PR is the one adding or deleting the rules
    file.
    """
    if not tree.ok:
        skips.add("nearest_rules", tree)
        return None
    if diff_section is None:
        skips.add(
            "nearest_rules",
            Read("compare", skip=MALFORMED, detail="no diff to resolve paths for", endpoint=tree.endpoint),
        )
        return None

    if _get(tree.data, "truncated") is True:
        # HTTP 200 with a partial answer. Reporting "no rules file" from a tree
        # we only half read is the exact confusion this guard exists to prevent.
        skips.add(
            "nearest_rules",
            Read(
                "tree",
                skip=TRUNCATED,
                detail="the head tree came back truncated, so an absent rules file cannot be distinguished from an unread one",
                endpoint=tree.endpoint,
            ),
        )
        return None

    entries = _get(tree.data, "tree")
    if not isinstance(entries, list):
        skips.add(
            "nearest_rules",
            Read("tree", skip=MALFORMED, detail="tree is not a list", endpoint=tree.endpoint),
        )
        return None
    if not entries:
        # A head tree listing nothing cannot be reconciled with a diff that
        # changed files. Resolving every path to "no rules file" against it would
        # be an empty read publishing itself as an answer.
        skips.add(
            "nearest_rules",
            Read("tree", skip=EMPTY, detail="the head tree listed no entries at all", endpoint=tree.endpoint),
        )
        return None

    tree_paths = frozenset(
        p for p in (_get(e, "path") for e in entries) if isinstance(p, str)
    )

    resolved: dict[str, dict[str, str | None]] = {}
    for entry in diff_section["files"]:
        path = entry["path"]
        found = {name: _nearest(path, name, tree_paths) for name in RULES_FILENAMES}
        resolved[path] = found
        if not any(found.values()):
            # SKIP-ONLY: a path with no ancestor rules file is a real answer.
            skips.add(
                f"nearest_rules[{path}]",
                Read("tree", skip=EMPTY, detail="no ancestor AGENTS.md or CLAUDE.md", endpoint=tree.endpoint),
            )
    return resolved


# --------------------------------------------------------------------------- #
# The record
# --------------------------------------------------------------------------- #


def build_record(reads: Mapping[str, Read]) -> dict[str, Any]:
    """Assemble the record from already-fetched reads.

    Raises :class:`RecordError` when a REQUIRED input could not be read, so that
    the caller exits non-zero instead of publishing a record with holes in it.
    """
    missing = set(REQUIRED_INPUTS) - set(reads)
    if missing:
        raise KeyError(f"no read supplied for required inputs: {sorted(missing)}")

    skips = Skips()
    pr_section = build_pr(reads["pr"], reads["meta"], skips)
    closing = build_closing_issue(
        reads["meta"],
        reads.get("closing_refs", Read("closing_refs", skip=UNREACHABLE, detail="not attempted")),
        skips,
    )
    diff_section = build_diff(reads["compare"], pr_section, skips)
    checks_section = build_checks(reads["checks"], skips)
    gate = build_required_gate(
        reads.get("branch_rules", Read("branch_rules", skip=UNREACHABLE, detail="not attempted")),
        reads.get("org_rulesets", Read("org_rulesets", skip=UNREACHABLE, detail="not attempted")),
        checks_section,
        skips,
    )
    rules = build_nearest_rules(reads["tree"], diff_section, skips)

    record = {
        "pr": pr_section,
        "closing_issue": closing,
        "diff": diff_section,
        "checks": checks_section,
        "required_gate": gate,
        "nearest_rules": rules,
        "skips": skips.entries,
    }
    assert tuple(record) == RECORD_FIELDS, "the record grew a field without the docstring"

    fatal = [entry for entry in skips.entries if is_fatal(entry)]
    if fatal:
        raise RecordError(fatal)
    return record


#: Reasons that mean the input was NOT read. `empty` is deliberately absent: it
#: means the input was read and carries nothing, which is an answer.
FATAL_REASONS = (ABSENT, FORBIDDEN, MALFORMED, TRUNCATED, UNREACHABLE)

#: Record fields whose unreadability is fatal — the five REQUIRED inputs, seen
#: from the record's side rather than the fetch layer's.
FATAL_FIELDS = ("pr", "closing_issue", "diff", "checks", "nearest_rules")

#: Named explicitly rather than matched by prefix, so a new field cannot become
#: non-fatal by accident of what it is called.
NEVER_FATAL_FIELDS = (
    "required_gate",
    "required_gate.org_rulesets",
    "closing_issue.closing_refs",
)


def is_fatal(skip_entry: Mapping[str, str]) -> bool:
    """Whether one skip entry means the process must exit non-zero.

    Two rules, and the first outranks the second:

    1. **``empty`` is never fatal, for any field.** An empty answer was *read*.
       A pull request whose head commit has no check runs yet, and a pull request
       whose check list could not be fetched, are different facts — and treating
       the first as a failure is the same conflation, pointed the other way, as
       treating the second as "no checks are required".
    2. Otherwise a field is fatal when it is one of the five REQUIRED inputs.
    """
    if skip_entry["reason"] == EMPTY:
        return False
    field_name = skip_entry["field"]
    if field_name in NEVER_FATAL_FIELDS:
        return False
    if field_name.startswith("nearest_rules["):
        return False
    return field_name in FATAL_FIELDS
