"""Step 6 control: the deterministic detector.

Precision first. The bar that matters is zero false positives on **both** benign
corpora — the upstream records, and this repository's own review-heavy text, which is
what the agent will actually be pointed at. The original criterion measured only the
first, passed, and was wrong; see the plan's revision 5 note.

Recall is measured and reported, not asserted at 35/35, because this layer deliberately
does not cover semantic paraphrase. A control that demanded 35/35 here could only be
satisfied by a detector that had memorised the corpus.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

from contain import CONTROL_FLAGS_ENV_VAR
from corpus import load_benign, matrix
from detect import detect

HERE = Path(__file__).parent
failures: list[str] = []


def check(ok: bool, label: str) -> None:
    print(f"{'PASS' if ok else 'FAIL'}  {label}")
    if not ok:
        failures.append(label)


def gh_body(number: int) -> str:
    proc = subprocess.run(
        ["gh", "api", f"repos/launchpad-26/buzz/issues/{number}"],
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        return ""
    return json.loads(proc.stdout).get("body") or ""


# --- precision: both benign corpora ----------------------------------------
records, held_out, held_out_in_scope = load_benign()
upstream_fp = [
    (r["source"], key)
    for r in records
    for key in ("pr_body", "pr_diff")
    if detect(r[key], key)
]
check(not upstream_fp, f"zero false positives on ten upstream records (got {upstream_fp})")

repo_texts: list[tuple[str, str]] = []
for number in (109, 110, 116, 117, 118, 119, 120):
    body = gh_body(number)
    if body:
        repo_texts.append((f"issue#{number}", body))
for path in (HERE / "CONTAINMENT.md", HERE.parent / "plans"):
    if path.is_dir():
        repo_texts += [(p.name, p.read_text(encoding="utf-8")) for p in path.glob("*.md")]
    else:
        repo_texts.append((path.name, path.read_text(encoding="utf-8")))

check(len(repo_texts) >= 8, f"review-heavy corpus assembled ({len(repo_texts)} documents)")
repo_fp = [(name, f.evidence[:70]) for name, text in repo_texts for f in detect(text, "pr_body")]
check(not repo_fp, f"zero false positives on this repo's review-heavy text (got {repo_fp})")

# --- recall: measured and stated, not asserted at 35/35 ---------------------
cases = matrix()
caught = [c for c in cases if detect(c.payload.text, c.entry_point)]
missed = [c for c in cases if not detect(c.payload.text, c.entry_point)]
caught_kinds = sorted({c.payload.id for c in caught})
missed_kinds = sorted({c.payload.id for c in missed})

print(f"\n  recall: {len(caught)}/35 caught {caught_kinds}")
print(f"          {len(missed)}/35 missed {missed_kinds} — model layer's responsibility (#117)\n")

check(len(caught) > 0, "the detector catches something")

# The held-out payload was loaded and never passed to detect(). Its whole purpose is to
# prove the detector was not tuned to payloads.json, and that claim was untested.
in_scope = detect(held_out_in_scope, "pr_body")
check(
    bool(in_scope),
    "IN-SCOPE held-out payload is detected — the detector generalises, not memorises",
)
check(all(f.severity == "Blocker" for f in in_scope), "in-scope held-out findings are Blocker")
# The out-of-scope one must be MISSED. Asserting the miss pins the documented gap: if a
# future edit widened the detector this would fail loudly rather than drifting silently.
check(
    not detect(held_out, "pr_body"),
    "OUT-OF-SCOPE held-out payload is missed, exactly as CONTAINMENT.md documents",
)
corpus_texts = [p.text.strip() for p in __import__("corpus").load_payloads()]
check(
    held_out.strip() not in corpus_texts and held_out_in_scope.strip() not in corpus_texts,
    "both held-out payloads are genuinely absent from the attack corpus",
)
check(
    all(f.severity == "Blocker" for c in caught for f in detect(c.payload.text, c.entry_point)),
    "every finding carries severity Blocker",
)
check(
    all(
        f.entry_point == c.entry_point
        for c in caught
        for f in detect(c.payload.text, c.entry_point)
    ),
    "every finding names its own entry point",
)

# The measured counts must be stated in the contract, not left implied.
contract = (HERE / "CONTAINMENT.md").read_text(encoding="utf-8")
check("## Detection, and what it does not cover" in contract, "CONTAINMENT.md documents the limit")
check(
    f"{len(caught)} of 35" in contract and f"{len(missed)} of 35" in contract,
    f"CONTAINMENT.md states the real counts ({len(caught)} caught / {len(missed)} missed)",
)

# --- wiring: findings reach the CLI output ---------------------------------
payload = Path(tempfile.mkdtemp()) / "attack-pr.json"
attack = {ep: "" for ep in ("pr_title", "pr_diff", "pr_issue_comments", "pr_review_comments", "pr_review_bodies", "linked_issue")}
attack["pr_body"] = next(c for c in caught).payload.text
payload.write_text(json.dumps(attack), encoding="utf-8")
proc = subprocess.run(
    [sys.executable, str(HERE / "contain.py"), "--payload", str(payload), "--seed", "s6", "--json"],
    capture_output=True,
    text=True,
    env={**os.environ, CONTROL_FLAGS_ENV_VAR: "true"},
)
data = json.loads(proc.stdout)
kinds = {f["kind"] for f in data["containment_findings"]}
check("injection_attempt" in kinds, f"contain.py --json carries injection_attempt (got {kinds})")
check(
    all(f["severity"] == "Blocker" for f in data["containment_findings"]),
    "every finding in CLI output is Blocker",
)
payload.unlink()

print(f"\n{len(failures)} failure(s)")
sys.exit(1 if failures else 0)
