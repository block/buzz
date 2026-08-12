"""Deterministic detection of injection attempts — high-precision tells only.

No model, no inference call.

**Why this layer is deliberately narrow.** Distinguishing an attack from a *description*
of an attack is the use–mention problem, and it does not yield to pattern matching. Our
own PRD contains the sentence "A diff that 'asks' the agent to skip review is itself a
Blocker finding"; an attacker's diff contains "do not report the credential below".
At the token level these are near-identical. A broader rule set was measured against
this repository's own issues and documents and produced **10 false positives**, and the
obvious fix — ignoring quoted or blockquoted text — is a one-line bypass for anyone
willing to type `>`.

So this layer takes precision and gives up recall, on purpose:

    matrix cases caught   28 / 35   (skip-review, approve, suppress-a-finding,
                                     delimiter-breakout)
    missed                 7 / 35   (paraphrase)
    false positives        0        (10 upstream PRs + this repo's review-heavy docs)

**What covers the gap.** Containment, not this. Steps 1–3 escape and envelope every
surface whether or not this function notices anything, and the consumer preamble tells
each stage that the block has no authority and that an attempt is itself reportable. A
miss here means *nobody was warned*, not *the attack worked*. Semantic coverage belongs
to the model-based review dimensions (#117), which read the contained text and can weigh
meaning rather than tokens.

Recorded as a known limit in CONTAINMENT.md rather than hidden behind a passing test.
"""

from __future__ import annotations

import re

from contain import Finding

#: Phrases with no honest reading in pull-request prose. Each one asserts authority
#: over the reader, or announces a boundary that only an attacker would announce.
_STANDALONE = re.compile(
    r"(?:ignore\s+(?:all\s+)?(?:previous|prior|earlier|above)\s+instructions"
    r"|disregard\s+(?:all\s+)?(?:previous|prior)\s+instructions"
    r"|forget\s+(?:everything|all)\s+(?:above|before)"
    r"|system\s+directive"
    r"|operator\s+instructions"
    r"|you\s+are\s+now\s+(?:receiving|acting|operating)"
    r"|new\s+instructions\s+follow"
    r"|end\s+of\s+(?:data|input|context)"
    r"|this\s+(?:message|text)\s+overrides)",
    re.IGNORECASE,
)

#: Suppression, which #120's second criterion names alongside skip and approve.
#:
#: **This one is not an unambiguous tell, and saying otherwise would be false.** A
#: negated reporting verb whose object is the review's own output is exactly what an
#: attack writes — and exactly what an adjudication register writes when a reviewer
#: refutes a finding and asks that it not be raised. There is no lexical distinction
#: between the two, so the rule carries a known benign class rather than none. The
#: example is not spelled out here for the same reason it is not spelled out in
#: CONTAINMENT.md: writing it would trip this very rule.
#:
#: Measured at zero false positives across 554 real texts (this fork's issues, PRs,
#: comments and tracked documents), so it does not fire today. It fires the day someone
#: pastes an adjudication verdict into a PR body — a Blocker on an honest PR, which is
#: a credibility cost per PRD #109, not a security regression. Re-measure if that
#: becomes common, and see CONTAINMENT.md § Detection.
_SUPPRESS = re.compile(
    r"(?:do\s+not|don't|never)\s+(?:report|mention|include|list|flag|raise|surface)\b"
    r"[^.!?]{0,60}?"
    # The object must be the REVIEW's own output. An earlier version accepted any
    # noun, which matched ordinary prose like "do not flag this as an issue — it is
    # tracked in #137". Zero false positives were measured, but by luck of phrasing
    # rather than by the property the contract claims, so the claim was made true.
    r"\b(?:as\s+a\s+(?:finding|blocker|problem)"
    r"|in\s+(?:your|the)\s+(?:review|summary|report|findings|comment|output)"
    r"|to\s+the\s+(?:reviewer|maintainers?)"
    r"|in\s+your\s+response)\b",
    re.IGNORECASE,
)

#: Sentence split. On punctuation **only** — never on a newline. See ``_sentences``.
_SENTENCE_END = re.compile(r"(?<=[.!?:])\s+")

#: Diff structure that is never prose. ``---`` and ``+++`` must carry a path: a real
#: file header names one, and requiring it is what stops a bare ``---`` — an ordinary
#: markdown horizontal rule — being read as structure. See ``_sentences``.
_DIFF_META = re.compile(
    r"^(?:diff --git|index |(?:---|\+\+\+)\s+\S|@@|similarity index|rename )"
)

#: Formatting noise on a PROSE surface — a markdown horizontal rule, or diff structure
#: an author typed into a comment. Skipped like a blank line so the prose either side
#: joins. Never applied to ``pr_diff``, where these lines are real structure.
_NOISE = re.compile(
    r"^(?:\s*(?:-\s*){3,}|\s*(?:\*\s*){3,}|\s*(?:_\s*){3,}"
    r"|diff --git\b|index [0-9a-f]|(?:---|\+\+\+)\s+\S|@@.*@@|similarity index|rename )"
    r"\s*$|^(?:@@.*@@|(?:---|\+\+\+)\s+\S+)\s*$"
)


def _sentences(text: str, entry_point: str) -> list[str]:
    r"""Prose passages, split on sentence punctuation only.

    **A newline is not a sentence boundary, and treating it as one was a bypass.** The
    earlier version split on ``\n+`` as well, reasoning that diffs are line-oriented.
    Every pattern below needs ``\s+`` between its words, and no chunk produced by a
    newline split ever contained one — so ``ignore all previous\ninstructions`` matched
    nothing at all. Three of the four attack classes this layer detects evaded it for
    the cost of one keystroke, and the corpus never noticed because every fixture was
    written on a single line.

    So lines are joined, and whitespace within a passage is collapsed before matching.
    Diff structure still ends a passage: a hunk header is a real boundary, and joining
    across one would let two unrelated files' text form a phrase that neither wrote.

    **That boundary applies to ``pr_diff`` and to nothing else, and the omission of
    ``entry_point`` here was the same bypass a second time.** Six of the seven surfaces
    are prose, never a diff, and a markdown horizontal rule is byte-identical to a
    diff's ``---``. So ``Ignore all previous\n---\ninstructions.`` split into two
    passages on a PR body and matched nothing — one keystroke again, through the
    mechanism that fixed the first one. A control asserted that behaviour as correct,
    on ``pr_body``, which is why the suite stayed green over it.

    **On a prose surface such a line is skipped, exactly as a blank line is — not made
    a boundary, and not made a word.** Both alternatives buy the same silence: a rule
    line kept as text turns ``ignore all previous instructions`` into ``ignore all
    previous --- instructions``, and every pattern here needs its words adjacent. Only
    dropping the line joins the prose either side of it, which is what a reader does
    with a horizontal rule. Dropping can only ever join more text, so it cannot hide a
    tell; whether it manufactures one is measured against the benign corpora, which
    hold at zero false positives.
    """
    passages: list[list[str]] = [[]]
    for line in text.split("\n"):
        stripped = line.strip()
        # Real diff structure ends a passage — but only where the surface IS a diff.
        if entry_point == "pr_diff" and _DIFF_META.match(stripped):
            if passages[-1]:
                passages.append([])
            continue
        # Formatting noise is skipped on EVERY surface, diff included: a bare `---` is
        # not a file header anywhere, and inside a diff it is a context line an author
        # can write at will. Skipping rather than splitting is what joins the prose.
        if _NOISE.match(stripped):
            continue
        if not stripped:
            continue
        # Strip a leading diff marker so an added line is still read as prose. Not
        # scoped to pr_diff: authors paste diff fragments into bodies and comments, and
        # scoping it there let `+Ignore all previous\n+instructions.` evade on a body.
        passages[-1].append(stripped[1:].strip() if stripped[:1] in "+-" else stripped)

    out: list[str] = []
    for passage in passages:
        if not passage:
            continue
        # Collapse every whitespace run, so a phrase broken across lines — or padded
        # with tabs and doubled spaces — reads as the contiguous phrase it is.
        joined = re.sub(r"\s+", " ", " ".join(passage))
        out.extend(chunk.strip() for chunk in _SENTENCE_END.split(joined) if chunk.strip())
    return out


def detect(text: str, entry_point: str) -> list[Finding]:
    """Report unambiguous injection tells. Severity is fixed by CONTAINMENT.md.

    Returns at most one finding per sentence. A quiet return is not evidence the text is
    clean — see the module docstring for what this layer does and does not cover.
    """
    return [
        Finding("injection_attempt", entry_point, sentence[:120].replace("\n", "\\n"))
        for sentence in _sentences(text, entry_point)
        if _STANDALONE.search(sentence) or _SUPPRESS.search(sentence)
    ]
