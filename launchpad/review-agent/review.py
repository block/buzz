"""Render the review body. Does not post it — #119 owns publication.

Two properties this renderer exists to hold:

**The attempt is visible.** #120 requires a control asserting the injection attempt
appears in the published review, so a swallowed attack is a visible failure rather than
a silent one. Every containment finding is rendered, none are collapsed into a count.

**The attempt is neutralised where it is quoted.** A review body is read by a human and,
in later phases, by other agents. Quoting attacker text verbatim would move the payload
from a contained block into a fresh position with fresh authority. Evidence is therefore
rendered in its post-escape form — the same transform the envelope applies.
"""

from __future__ import annotations

import re

from contain import Finding, escape

def fence_for(evidence: str) -> str:
    """A code fence guaranteed longer than the longest backtick run in ``evidence``.

    CommonMark closes a fenced block on the first line of >= as many backticks as
    opened it. Attacker text containing ``` would therefore break out of a fixed
    three-backtick fence and corrupt every following section of the review.
    """
    longest = max((len(m) for m in re.findall(r"`+", evidence)), default=0)
    return "`" * max(3, longest + 1)


SEVERITY_ORDER = {"Blocker": 0, "High": 1, "Medium": 2, "Low": 3}

COVERAGE_NOTE = (
    "Automated containment covers the delimiter boundary and unambiguous injection "
    "tells only. It does not cover injection phrased as ordinary, unremarkable "
    "prose. **The absence of a containment finding is not evidence that "
    "this pull request contains no injection attempt.**"
)


UNREADABLE_STATES = ("absent", "oversized", "unparseable")

#: GitHub's PR review/comment body limit is 65536 bytes. Budgeting the findings
#: section alone, well under that ceiling, leaves room for COVERAGE_NOTE, the
#: incomplete banner and the empty-surfaces line that follow it, plus this section's
#: own truncation notice.
MAX_FINDINGS_BYTES = 48 * 1024


def render_review(findings: list[Finding], states: dict[str, str]) -> str:
    """The review body, as markdown.

    ``unreadable`` is derived from ``states``, never passed in. It was a keyword
    argument with no producer anywhere on the branch, so the "this review is
    incomplete" banner could never render — a caller cannot forget an argument that
    does not exist.
    """
    unreadable = [ep for ep, st in states.items() if st in UNREADABLE_STATES]
    lines = ["## Containment", ""]

    if findings:
        lines.append(
            f"{len(findings)} containment finding(s). Pull-request text attempted to "
            "act on the review itself; it was contained and is reported here."
        )
        lines.append("")
        # A PR author can pad enough distinct sentences to make EVERY finding real
        # and still render past GitHub's own body limit -- amplification, not
        # evasion, and the previous unbounded loop let it suppress the whole
        # review, Blockers included.
        #
        # Sorting by severity does NOT protect a specific finding here, and this
        # must not be described as though it does: every Finding constructed in
        # this codebase (contain.py, detect.py) uses the dataclass default and is
        # "Blocker", so there is no severity spread to sort by in production --
        # SEVERITY_ORDER.get(f.severity, 9) puts every real finding in the same
        # bucket, and Python's stable sort then keeps them in insertion order.
        # What this budget actually guarantees, independent of severity: at least
        # one finding always renders even if it alone exceeds the budget, so a
        # review with real findings never renders as though it had none; and any
        # truncation is disclosed with an explicit count, never silent. If a
        # future producer ever assigns differentiated severities, the sort then
        # starts doing real prioritisation work for free -- it costs nothing to
        # keep, but claims nothing it cannot back today.
        ordered = sorted(findings, key=lambda f: SEVERITY_ORDER.get(f.severity, 9))
        rendered_bytes = 0
        omitted = 0
        for index, finding in enumerate(ordered):
            block = "\n".join(
                [
                    f"### {finding.severity} — {finding.kind}",
                    "",
                    f"Entry point: `{finding.entry_point}`",
                    "",
                    # Post-escape, deliberately: see the module docstring. The
                    # fence is sized longer than any backtick run in the
                    # evidence, because attacker text containing ``` would
                    # otherwise close the fence early and spill the rest of the
                    # review into an unterminated code block.
                    fence_for(finding.evidence),
                    escape(finding.evidence),
                    fence_for(finding.evidence),
                    "",
                ]
            )
            block_bytes = len(block.encode("utf-8"))
            if rendered_bytes and rendered_bytes + block_bytes > MAX_FINDINGS_BYTES:
                omitted = len(ordered) - index
                break
            lines.append(block)
            rendered_bytes += block_bytes
        if omitted:
            lines.append(
                f"**{omitted} further finding(s) omitted.** Rendering every finding "
                f"would exceed a {MAX_FINDINGS_BYTES}-byte budget. A pull request "
                "producing enough findings to hit this budget is itself worth "
                "escalating."
            )
            lines.append("")
    else:
        lines.append("No containment findings.")
        lines.append("")

    if unreadable:
        lines.append(
            f"**Incomplete.** {len(unreadable)} surface(s) could not be read "
            f"({', '.join(sorted(unreadable))}), so this review does not cover them."
        )
        lines.append("")

    empty = sorted(ep for ep, st in states.items() if st == "empty")
    if empty:
        lines.append(f"Fetched and empty: {', '.join(f'`{ep}`' for ep in empty)}.")
        lines.append("")

    lines.append(COVERAGE_NOTE)
    return "\n".join(lines)
