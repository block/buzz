"""Controls for steps 1 and 10 — the two steps whose deliverable is CONTAINMENT.md.

Both were built and neither had a control. A review-final found the consequence: the
contract stated the detector's recall in two places, one of them stale by seven cases,
and told #117 it inherited a gap that had just been closed. Nothing read that document,
so nothing noticed.

The lesson is narrower than "test the docs". A normative document that other issues are
told to build against is an interface, and an interface with no control drifts from the
code the moment either is edited. So this asserts the two things a reader depends on:
that the required sections exist, and that the numbers in them match what the code
actually measures — including that the superseded numbers are **gone**, not merely
outnumbered by correct ones.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

from contain import ENTRY_POINTS
from corpus import matrix
from detect import detect

CONTRACT = (Path(__file__).parent / "CONTAINMENT.md").read_text(encoding="utf-8")
failures: list[str] = []


def check(ok: bool, label: str) -> None:
    print(f"{'PASS' if ok else 'FAIL'}  {label}")
    if not ok:
        failures.append(label)


# --- step 1: the spec's required sections ----------------------------------
print("step 1 — spec sections")
for heading in (
    "## Envelope structure",
    "## Delimiter collision",
    "## Consumer preamble",
    "## Severity contract",
    "## Degenerate input",
):
    check(any(line == heading for line in CONTRACT.split("\n")), f"heading present: {heading}")

degenerate = CONTRACT.split("## Degenerate input", 1)[-1]
for state in ("absent", "empty", "oversized", "unparseable"):
    row = [ln for ln in degenerate.split("\n") if ln.startswith(f"| `{state}`")]
    check(bool(row) and row[0].count("|") >= 4, f"degenerate state `{state}` has a disposition")

# --- step 10: the contract for later stages --------------------------------
print("\nstep 10 — contract for later stages")
for issue in (116, 117, 118, 119):
    check(f"issues/{issue}" in CONTRACT, f"#{issue} is named")

# Every stage must name a function it calls. A prohibition is not an entry point: a
# stage with only a "must never" has no followable instruction, and #118 had exactly
# that until a review-final caught it.
table = [ln for ln in CONTRACT.split("\n") if ln.startswith("| [#")]
check(len(table) == 4, f"the contract table has one row per stage (got {len(table)})")
for row in table:
    issue = re.search(r"issues/(\d+)", row)
    called = re.findall(r"`([a-z_]+\.[a-z_]+)\(", row)
    check(bool(called), f"#{issue.group(1) if issue else '?'} names a function it must call")

for label in ENTRY_POINTS:
    check(f"`{label}`" in CONTRACT, f"entry point `{label}` appears in the contract")

# --- the numbers must match what the code measures -------------------------
print("\nrecall figures agree with the code")
caught = sum(1 for c in matrix() if detect(c.payload.text, c.entry_point))
missed = len(matrix()) - caught

check(f"{caught} of 35" in CONTRACT, f"the contract states the real caught count ({caught} of 35)")
check(f"{missed} of 35" in CONTRACT, f"the contract states the real missed count ({missed} of 35)")

# Presence alone is what let a stale figure survive beside a correct one. Any OTHER
# "N of 35" in the document is a number nobody updated.
stated = {int(n) for n in re.findall(r"(\d+) of 35", CONTRACT)}
check(
    stated <= {caught, missed},
    f"no superseded recall figure remains (found {sorted(stated)}, expected {sorted({caught, missed})})",
)
check(
    "21 of 35" not in CONTRACT and "14 cases" not in CONTRACT,
    "the pre-suppression figures are gone from the contract, not merely outvoted",
)

# --- the confusable script list must match the map, in BOTH directions -----
# The document named Latin as a covered script while _CONFUSABLES held no Latin key,
# so a delimiter in Latin small capitals passed unflagged under a contract that said it
# was covered. Naming a script the map lacks is the dangerous direction; omitting one it
# has is merely inaccurate, and both are checked because only checking one is how the
# first version stayed wrong.
print("\nconfusable script list agrees with the map")
import unicodedata  # noqa: E402

from contain import _CONFUSABLES  # noqa: E402

present = {unicodedata.name(k).split()[0] for k in _CONFUSABLES}
paragraph = CONTRACT.split("**How confusables are recognised", 1)[-1].split("\n\n")[0]
paragraph += CONTRACT.split("**How confusables are recognised", 1)[-1].split("\n\n")[1]

for script in ("LATIN", "GREEK", "CYRILLIC", "CHEROKEE", "LISU", "COPTIC", "ARMENIAN"):
    named = script.capitalize() in paragraph
    held = script in present
    check(named == held, f"{script.capitalize()}: named in the contract={named}, in the map={held}")

check("Canadian" in paragraph, "Canadian Aboriginal Syllabics is named (it is in the map)")
check(
    "not UTS #39" in CONTRACT,
    "the contract still says the skeleton is bounded, not a general confusables table",
)

print(f"\n{len(failures)} failure(s)")
sys.exit(1 if failures else 0)
