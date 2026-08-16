#!/usr/bin/env python3
"""Validate a pull request body against this fork's conventions.

WHY THIS IS A SCRIPT AND NOT INLINE YAML

It used to be a Python heredoc inside `.github/workflows/launchpad-pr-check.yml`.
Two consequences, both real:

  1. NOTHING COULD TEST IT. Logic embedded in YAML is not importable, so the only
     way to check a change was to open a PR and watch. The suite written for
     issue #125 had to copy the regexes by hand, which meant it would have passed
     unchanged if the real check were reverted or replaced with `sys.exit(0)`.
  2. THE MARKDOWN PARSING WAS WRONG, AND QUIETLY SO. See below.

WHY GITHUB ANSWERS THE CLOSING QUESTION, NOT A REGEX

The check's purpose is "will the board update when this merges?". A regex over the
body cannot answer that, because GitHub ignores references written inside code and
CommonMark has more code forms than anyone enumerates on the first attempt.
Measured against the previous fix on 2026-08-12, all four of these smuggled a
keyword past it while GitHub created no link:

    an unterminated ``` fence          (the template mandates pasted output, so
                                        a missing closing fence is the single most
                                        likely authoring slip in this repo)
    a ``double-backtick`` span         (no run-length matching)
    a ````quad fence```` with an
      inner shorter run                (non-greedy match closes early)
    a 4-space indented block           (not a delimiter the regex knew about)

So the authoritative answer comes from GitHub's own `closingIssuesReferences`,
passed in by the workflow. There is no markdown to misparse.

`Refs #n` still needs a text search, because it is a convention of ours that
GitHub knows nothing about. A false positive there is cheap: `Refs` makes no claim
that the board will move, so the worst case is a PR that named its issue in an
unusual place. The fence stripping below is therefore best-effort by design, and
its one hard case — the unterminated fence — is handled.

ABSENCE IS NOT EVIDENCE

`CLOSING_REFS` unset means the query did not run or failed. That is NOT the same as
"GitHub found no closing reference", and it must never read as either a pass or a
silent failure: the check degrades to a text search and says so on stdout, so a
reader can tell a verified result from a guessed one.

Usage:
    BODY=... LABELS='["by:agent"]' CLOSING_REFS='[116]' python3 pr_body_check.py

Exit: 0 = the body is acceptable, 1 = it is not.
"""

from __future__ import annotations

import json
import os
import re
import sys

CLOSING_RE = re.compile(r"\b(Closes|Fixes|Resolves)\s+#\d+", re.I)
REFS_RE = re.compile(r"\bRefs\s+#\d+", re.I)

ISSUE_TYPES = frozenset({"PRD", "Task", "Enhancement", "Bug", "ADR"})
PROVENANCE_FIELDS = ("Harness / provider", "Model", "Initiating human")
EMPTY_NOT_VERIFIED = frozenset({"nothing", "none", "n/a", ""})


def strip_comments(body: str) -> str:
    """Remove HTML comments so unfilled template placeholders never count."""
    return re.sub(r"<!--.*?-->", "", body, flags=re.S)


BLOCKQUOTE = re.compile(r"^(?: {0,3}>)+ ?")
FENCE_OPEN = re.compile(r"^ {0,3}(`{3,}|~{3,})")
# A closing fence carries nothing but its run and trailing whitespace. An opener
# may carry an info string (```python); a closer may not, so they need different
# patterns — accepting junk after a closing run ends the block early and starts a
# spurious new one, which both leaks code as prose and swallows real prose as code.
FENCE_CLOSE = re.compile(r"^ {0,3}(`{3,}|~{3,})[ \t]*$")


def _strip_fences(text: str) -> str:
    """Drop fenced blocks, honouring CommonMark's run-length rule.

    A regex cannot do this. A closing fence must be *at least as long* as the one
    that opened it, and `re` has no "backreference, or longer" — so a non-greedy
    ```` ```.*?``` ```` closes early on any shorter run inside a longer fence, which
    is how a ````quad fence```` wrapping a ``` example leaked a keyword into prose.
    An unterminated fence runs to the end of the document, so it consumes the rest.

    A fence must also close with the character it opened with: ``` is not closed by
    ~~~, however long the run.

    Blockquote markers are stripped before matching, so a fence quoted with `> `
    is recognised. Without that, quoting someone else's fenced output left its
    contents standing as prose.

    A FENCE OPENED INSIDE A QUOTE CLOSES WITH THE QUOTE. CommonMark ends a block
    quote lazily — a line with no `>` marker (a blank line included, since a bare
    `>` strips to empty) closes it, the same way a blank line ends one at the top
    level. A fence keeps no memory of the container it opened in, so one opened
    with `> ` and never explicitly closed used to stay open to the end of the
    document once the quote ended, silently consuming every line after it —
    including a `Refs #<n>` GitHub would render as ordinary prose (#145).
    """
    out: list[str] = []
    fence: str | None = None
    fence_in_quote = False
    for line in text.splitlines(keepends=True):
        if fence is not None and fence_in_quote and not BLOCKQUOTE.match(line):
            # The quote that contained this fence just ended. The fence closes
            # with it, so this line is evaluated fresh below — as prose, or as
            # the start of a new fence — not as content of the old one.
            fence, fence_in_quote = None, False

        probe = BLOCKQUOTE.sub("", line)
        if fence is None:
            m = FENCE_OPEN.match(probe)
            if m:
                fence = m.group(1)
                fence_in_quote = bool(BLOCKQUOTE.match(line))
                continue
            out.append(line)
        else:
            closer = FENCE_CLOSE.match(probe)
            if closer and closer.group(1)[0] == fence[0] and len(closer.group(1)) >= len(fence):
                fence, fence_in_quote = None, False
    return "".join(out)


def strip_code(text: str) -> str:
    """Best-effort removal of code so a text search sees prose only.

    Fences go first: run the inline-span pattern first and it eats a fence's
    delimiters piecemeal, leaving the fence body behind as prose.

    DELIBERATELY NOT HANDLED: four-space indented code blocks. Stripping every
    line that starts with four spaces looked right and was worse than the bug it
    was meant to help — CommonMark ties the four-space rule to the *container's*
    content column, so ordinary markdown that GitHub renders as prose was being
    deleted:

        - part of a larger plan:
            - Refs #116 covers the follow-up

    That reference vanished and the check rejected a compliant PR for having no
    reference at all. A false block is worse than the false pass this file exists
    to fix: the false pass merely failed to catch something, while this actively
    obstructed an author who had done nothing wrong. Since the closing question is
    now answered by GitHub, the only cost of not stripping indented code is a
    `Refs` written inside an indented block being counted, which claims nothing
    about the board.
    """
    out = _strip_fences(text)
    # Run-length matched spans: the backreference forces the closing run to equal
    # the opening one, so ``x`` is one span rather than two empty ones around x.
    return re.sub(r"(`+)([^\n]*?)\1", "", out)


def section(visible: str, name: str) -> str | None:
    """Text under a '### name' heading, up to the next heading."""
    m = re.search(
        rf"^#+\s*{re.escape(name)}\s*$(.*?)(?=^#+\s|\Z)",
        visible,
        flags=re.M | re.S,
    )
    return m.group(1).strip() if m else None


def parse_closing_refs(raw: str | None) -> list[int] | None:
    """GitHub's answer, or None when we do not have one.

    None is returned for an unset, blank or unparseable value. It means "unknown",
    never "none found" — the caller degrades and says so rather than deciding.
    """
    if raw is None or not raw.strip():
        return None
    try:
        parsed = json.loads(raw)
    except json.JSONDecodeError:
        return None
    if not isinstance(parsed, list):
        return None
    # `bool` subclasses `int`, so a JSON `true` would otherwise survive as 1 and be
    # printed as "#True". Not reachable through the workflow's jq expression today,
    # which yields plain integers, but a hand-set CLOSING_REFS could hit it.
    return [n for n in parsed if isinstance(n, int) and not isinstance(n, bool)]


def check_reference(prose: str, closing_refs: list[int] | None) -> tuple[list[str], str]:
    """Does this PR name an issue? Returns (errors, one line about how we know)."""
    refs = bool(REFS_RE.search(prose))

    if closing_refs is None:
        # Degraded: no authoritative answer, so fall back to the text search that
        # this script exists to stop relying on. Say so out loud, and say WHICH form
        # matched — one identical note for both was untestable and told a reader
        # less than it appeared to.
        if CLOSING_RE.search(prose):
            return [], (
                "reference: a closing keyword was found by text search only — "
                "GitHub's answer was unavailable, so 'the board updates on merge' "
                "is NOT verified"
            )
        if refs:
            return [], (
                "reference: 'Refs' found by text search; GitHub's answer was "
                "unavailable, but nothing was expected to close anyway"
            )
        return [
            "No issue reference found. Use 'Closes #<n>' when this PR completes the "
            "issue, or 'Refs #<n>' when it does not."
        ], "reference: absent by text search; GitHub's answer was unavailable"

    if closing_refs:
        joined = ", ".join(f"#{n}" for n in closing_refs)
        return [], f"reference: GitHub will close {joined} on merge"
    if refs:
        return [], "reference: 'Refs' present; nothing closes on merge, as intended"

    hint = ""
    if CLOSING_RE.search(prose):
        hint = (
            " A closing keyword appears in the body but GitHub created no link from "
            "it, which happens when it sits inside a code span or fenced block — "
            "write it as plain text."
        )
    return [
        "No issue reference GitHub recognises. Use 'Closes #<n>' when this PR "
        "completes the issue so the board updates on merge, or 'Refs #<n>' when it "
        "does not." + hint
    ], "reference: none — GitHub reports no closing link and no 'Refs' was found"


def check(body: str, labels: list[str], closing_refs: list[int] | None) -> tuple[list[str], list[str]]:
    """Return (errors, notes). Empty errors means the body is acceptable."""
    visible = strip_comments(body)
    prose = strip_code(visible)
    is_agent = "by:agent" in labels
    errors: list[str] = []
    notes: list[str] = []

    if not visible.strip():
        errors.append("PR body is empty. Use the PR template.")

    ref_errors, ref_note = check_reference(prose, closing_refs)
    errors.extend(ref_errors)
    notes.append(ref_note)

    itype = section(visible, "Issue type")
    if not itype:
        errors.append("Missing '### Issue type' section.")
    elif not any(v.lower() in itype.lower() for v in ISSUE_TYPES):
        errors.append(f"Issue type must be one of {sorted(ISSUE_TYPES)}. Found: {itype!r}")

    if is_agent:
        for field in PROVENANCE_FIELDS:
            m = re.search(rf"\|\s*{re.escape(field)}\s*\|(.*?)\|", visible)
            if not m or not m.group(1).strip():
                errors.append(f"by:agent PR missing provenance value for '{field}'.")

        nv = section(visible, "Not verified")
        if not nv:
            errors.append("by:agent PR missing '### Not verified' section.")
        elif nv.lower().strip(" .") in EMPTY_NOT_VERIFIED:
            errors.append(
                "'Not verified' must name something specific. "
                "There is always something that was not checked."
            )

        # Deliberately reads `visible`, not `prose`: this rule REQUIRES a fence to
        # be present, so stripping code first would make it unsatisfiable.
        if "```" not in visible:
            errors.append("by:agent PR must paste raw command output in a fenced code block.")

    return errors, notes


def main() -> int:
    body = os.environ.get("BODY") or ""
    try:
        labels = json.loads(os.environ.get("LABELS") or "[]")
    except json.JSONDecodeError:
        labels = []
    closing_refs = parse_closing_refs(os.environ.get("CLOSING_REFS"))

    errors, notes = check(body, labels, closing_refs)
    for note in notes:
        print(f"  {note}")

    if errors:
        print("\nPR body check failed:\n")
        for e in errors:
            print(f"  - {e}")
        print("\nHuman PRs: .github/PULL_REQUEST_TEMPLATE.md")
        print("Agent PRs: launchpad/AGENT_PR_TEMPLATE.md")
        return 1

    kind = "agent" if "by:agent" in labels else "human"
    print(f"\nPR body check passed. ({kind})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
