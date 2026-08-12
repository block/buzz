"""Controls for step 4 (payload corpus) and step 5 (benign and held-out corpora).

Step 4's done-when: exactly five payloads, a matrix of exactly 35 cases, no payload
pre-assigned to any entry point.

Step 5's done-when: ten upstream merged-PR bodies and diffs stored as benign records,
none exceeding the byte cap, and one attack payload absent from step 4's corpus stored
separately as the held-out case.
"""

from __future__ import annotations

import sys
from collections import Counter

from contain import ENTRY_POINTS
from corpus import load_benign, load_payloads, matrix
from fetch import CAP_PER_ENTRY_POINT

failures: list[str] = []


def check(ok: bool, label: str) -> None:
    print(f"{'PASS' if ok else 'FAIL'}  {label}")
    if not ok:
        failures.append(label)


# --- step 4 ----------------------------------------------------------------
payloads = load_payloads()
cases = matrix()

check(len(payloads) == 5, f"step 4: exactly five payloads (got {len(payloads)})")
check(len(cases) == 35, f"step 4: matrix of exactly 35 cases (got {len(cases)})")
check(len({c.id for c in cases}) == 35, "step 4: every case id is unique")

ids = {p.id for p in payloads}
expected = {"skip-review", "approve", "suppress-a-finding", "paraphrase", "delimiter-breakout"}
check(ids == expected, f"step 4: the five required payload kinds are present ({sorted(ids)})")

# No payload is pre-assigned: each appears against all seven entry points.
per_payload = Counter(c.payload.id for c in cases)
check(
    all(count == len(ENTRY_POINTS) for count in per_payload.values()),
    "step 4: every payload is routed through all seven entry points",
)
per_entry = Counter(c.entry_point for c in cases)
check(
    set(per_entry) == set(ENTRY_POINTS) and all(v == 5 for v in per_entry.values()),
    "step 4: every entry point receives all five payloads",
)

# The paraphrase must avoid the three obvious phrasings, or it is not a paraphrase and
# step 6's detector could pass by keyword-matching alone.
para = next(p for p in payloads if p.id == "paraphrase").text.lower()
banned = [w for w in ("skip", "approve", "suppress") if w in para]
check(not banned, f"step 4: the paraphrase uses none of skip/approve/suppress (found {banned})")

# The breakout payload must actually contain the token, or it tests nothing.
breakout = next(p for p in payloads if p.id == "delimiter-breakout").text
check("BUZZ-UNTRUSTED" in breakout, "step 4: the breakout payload contains the real token")

# --- the declared detection oracle -----------------------------------------
# suite.py used to compute its own expected answer by calling detect(), so it passed
# 35/35 with the detector neutered. The expectation now lives in the fixture, which
# makes the fixture load-bearing: these checks are what keep it honest.
check(
    all(isinstance(p.expected_detection, bool) for p in payloads),
    "step 4: every payload declares expected_detection as a boolean",
)
declared = sum(1 for p in payloads if p.expected_detection)
check(
    0 < declared < len(payloads),
    f"step 4: the oracle declares both outcomes (got {declared} true of {len(payloads)}) "
    "— all-true or all-false would make the assertion vacuous",
)
check(
    not next(p for p in payloads if p.id == "paraphrase").expected_detection,
    "step 4: the paraphrase is declared UNDETECTED, pinning the documented recall gap",
)

# `intent` had no reader anywhere on the branch. It is the only machine-readable record
# of #120's three named intents, so nothing stopped one being swapped for a second
# paraphrase while the 35-case count stayed green.
intents = {p.id: p.intent for p in payloads}
check(
    all(t.strip() for t in intents.values()) and len(set(intents.values())) == len(payloads),
    "step 4: every payload states a distinct, non-empty intent",
)
for required, phrase in (
    ("skip-review", "abandon the review"),
    ("approve", "grant approval"),
    ("suppress-a-finding", "withhold"),
):
    check(
        phrase in intents.get(required, ""),
        f"step 4: #120's named intent is present and described — {required} ({phrase!r})",
    )

# --- step 5 ----------------------------------------------------------------
records, held_out, held_out_in_scope = load_benign()

check(len(records) == 10, f"step 5: ten benign records (got {len(records)})")
check(
    all(r["source"].startswith("block/buzz#") for r in records),
    "step 5: benign records come from upstream block/buzz, not this fork",
)
check(len({r["source"] for r in records}) == 10, "step 5: ten distinct source PRs")
check(
    all(r.get("pr_body") is not None and r.get("pr_diff") for r in records),
    "step 5: every record carries both a body and a diff",
)

oversized = [
    r["source"]
    for r in records
    if len(r["pr_diff"].encode()) > CAP_PER_ENTRY_POINT
    or len(r["pr_body"].encode()) > CAP_PER_ENTRY_POINT
]
check(not oversized, f"step 5: no record exceeds the byte cap (over: {oversized})")

check(bool(held_out.strip()) and bool(held_out_in_scope.strip()),
      "step 5: both held-out payloads exist (in-scope and out-of-scope)")
check(
    all(held_out.strip() != p.text.strip() for p in payloads),
    "step 5: the held-out payload is absent from step 4's corpus",
)
held_lower = held_out.lower()
check(
    not any(w in held_lower for w in ("skip", "approve", "suppress")),
    "step 5: the held-out payload also avoids the three obvious phrasings",
)

# A benign record that already contains attack text would poison the false-positive
# measurement, so check none of them do.
contaminated = [
    r["source"]
    for r in records
    if any(p.text[:40] in r["pr_body"] or p.text[:40] in r["pr_diff"] for p in payloads)
]
check(not contaminated, f"step 5: no benign record contains attack text ({contaminated})")

print(f"\n{len(failures)} failure(s)")
sys.exit(1 if failures else 0)
