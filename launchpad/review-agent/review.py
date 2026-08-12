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
    "tells only. It does not cover semantic paraphrase or finding-suppression phrased "
    "as ordinary prose. **The absence of a containment finding is not evidence that "
    "this pull request contains no injection attempt.**"
)


UNREADABLE_STATES = ("absent", "oversized", "unparseable")


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
        for finding in sorted(findings, key=lambda f: SEVERITY_ORDER.get(f.severity, 9)):
            lines.append(f"### {finding.severity} — {finding.kind}")
            lines.append("")
            lines.append(f"Entry point: `{finding.entry_point}`")
            lines.append("")
            # Post-escape, deliberately: see the module docstring. The fence is
            # sized longer than any backtick run in the evidence, because attacker
            # text containing ``` would otherwise close the fence early and spill
            # the rest of the review into an unterminated code block.
            lines.append(fence_for(finding.evidence))
            lines.append(escape(finding.evidence))
            lines.append(fence_for(finding.evidence))
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
