"""Step 8 control: the attempt must be visible in the published review.

The clause that carries the weight is the delimiter-breakout one. For plain-English
payloads escape(payload) == payload, so "quotes the payload in post-escape form" is
trivially satisfied by a renderer that echoed the raw source field and never saw the
contained form. The breakout payload is the only one where the two representations
diverge, so it is the only case that can tell a contained render from a bypassed one.
"""

from __future__ import annotations

import re
import sys

from contain import ENTRY_POINTS, ESC_TOKEN, TOKEN, escape, make_nonce, render, unescape
from corpus import matrix
from detect import detect
from fetch import Surface
from contain import Finding
from review import COVERAGE_NOTE, render_review

failures: list[str] = []


def check(ok: bool, label: str) -> None:
    print(f"{'PASS' if ok else 'FAIL'}  {label}")
    if not ok:
        failures.append(label)


def review_for(case) -> str:
    surfaces = {
        ep: Surface(ep, "ok", text=case.payload.text) if ep == case.entry_point
        else Surface(ep, "empty")
        for ep in ENTRY_POINTS
    }
    nonce = make_nonce(f"step8-{case.id}")
    _, findings, all_readable = render(surfaces, nonce)
    return render_review(findings, {ep: s.state for ep, s in surfaces.items()})


def records_in(body: str) -> list[tuple[str, str, str]]:
    """(severity, kind, fenced evidence) per rendered finding record."""
    pattern = re.compile(r"### (\w+) — (\w+)\n\nEntry point: `(\w+)`\n\n```\n(.*?)\n```", re.DOTALL)
    return [(m.group(1), m.group(2), m.group(4)) for m in pattern.finditer(body)]


# The pr_diff cases the deterministic layer actually detects — three of five.
diff_cases = [c for c in matrix() if c.entry_point == "pr_diff"]
detected = [c for c in diff_cases if detect(c.payload.text, c.entry_point)]
undetected = [c for c in diff_cases if not detect(c.payload.text, c.entry_point)]

check(len(diff_cases) == 5, f"five payloads routed through pr_diff (got {len(diff_cases)})")
check(
    len(detected) == 3,
    f"three are detected deterministically, matching step 6's measured recall "
    f"(got {len(detected)}: {[c.payload.id for c in detected]})",
)

def normalise(text: str) -> str:
    """detect()/_excerpt() render newlines as a literal backslash-n."""
    return text.replace("\n", "\\n")


for case in detected:
    body = review_for(case)
    recs = records_in(body)
    payload = normalise(case.payload.text)

    # ONE record must carry both properties: Blocker severity, and evidence that is
    # genuinely a slice of this payload once unescaped. Two substrings floating
    # anywhere in the output would not do — that was the defect in revision 2.
    matching = [
        (sev, kind, ev)
        for sev, kind, ev in recs
        if sev == "Blocker" and unescape(ev) and unescape(ev) in payload
    ]
    check(
        len(matching) >= 1,
        f"{case.payload.id}: one record is Blocker AND quotes this payload post-escape",
    )
    check(
        all(unescape(ev) in payload for _, _, ev in recs),
        f"{case.payload.id}: every quoted record round-trips to real payload text",
    )

# The discriminating case: escaped and raw must genuinely differ, or the whole
# post-escape clause is a no-op satisfied by echoing the source field.
breakout = next(c for c in detected if c.payload.id == "delimiter-breakout")
body = review_for(breakout)
recs = records_in(body)

check(TOKEN in breakout.payload.text, "delimiter-breakout: the raw payload contains the token")
check(
    TOKEN not in body,
    "delimiter-breakout: the raw token never appears in the review body",
)
check(
    any(ESC_TOKEN in ev for _, _, ev in recs),
    f"delimiter-breakout: a record quotes the escaped token {ESC_TOKEN!r}",
)
check(
    any(escape(unescape(ev)) != unescape(ev) for _, _, ev in recs),
    "delimiter-breakout: escaping genuinely transforms this evidence (the check has teeth)",
)

# A review over an undetected payload must never read as clean.
for case in undetected:
    body = review_for(case)
    check(
        COVERAGE_NOTE in body,
        f"{case.payload.id}: review states the coverage limit rather than reading clean",
    )
    check(
        "not evidence that" in body,
        f"{case.payload.id}: review explicitly denies that silence means clean",
    )

# A payload carrying its own code fence must not break the review's fence. Attacker
# backticks closing the block early would spill every following section — including the
# coverage note — into an unterminated code block.
# Backticks followed by whitespace ONLY — a valid CommonMark closing fence. With
# trailing text it is merely content and closes nothing, which is why an earlier
# version of this payload let the fixed-fence mutant live. detect() truncates
# evidence to 120 chars, so a sentence opening with ``` and a long run of spaces
# produces exactly this shape from real input.
fence_evidence = "```" + " " * 130
fenced = render_review([Finding("injection_attempt", "pr_body", fence_evidence)], {"pr_body": "ok"})


def fence_state(markdown: str) -> tuple[bool, list[str]]:
    """Walk the document as CommonMark does. Returns (ends_closed, lines_outside_fences).

    Counting backticks is not enough — an earlier version of this check used
    `count("```") % 2 == 0 or "````" in body`, and that `or` let a mutant through that
    replaced only the OPENING fence. A fence opens on a run of >= 3 backticks and closes
    on the next line whose run is at least as long, so the parse has to be simulated.
    """
    open_len = 0
    outside: list[str] = []
    for line in markdown.split("\n"):
        stripped = line.strip()
        run = len(stripped) - len(stripped.lstrip("`"))
        is_fence = run >= 3 and set(stripped[run:].strip()) <= {""} | set()
        if open_len == 0:
            if is_fence and not stripped[run:].strip():
                open_len = run
            else:
                outside.append(line)
        elif is_fence and run >= open_len and not stripped[run:].strip():
            open_len = 0
    return open_len == 0, outside


closed, outside = fence_state(fenced)
check(closed, "backtick evidence leaves every fence closed (parsed, not counted)")
check(
    any(COVERAGE_NOTE.split(".")[0] in line for line in outside),
    "the coverage note renders OUTSIDE any code fence, so a reader actually sees it",
)
check(COVERAGE_NOTE in fenced, "the coverage note survives backtick evidence")

print(f"\n{len(failures)} failure(s)")
sys.exit(1 if failures else 0)
