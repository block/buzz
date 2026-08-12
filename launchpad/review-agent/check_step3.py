"""Step 3 control: the CLI over all seven surfaces.

Three criteria, per the plan:
  (a) a captured payload produces seven blocks, and each sentinel lands in its own
      block and no other — a label-and-count check would pass a CLI that copied one
      field into all seven, and every later step's matrix rests on that routing;
  (b) a live fetch against a PR that exists produces the same seven labels;
  (c) each degenerate state is forced in turn and gets its own disposition, with the
      unreadable three exiting non-zero and none rendering as a clean empty block.
"""

from __future__ import annotations

import json
import re
import subprocess
import sys

from contain import ENTRY_POINTS

HERE = __file__.rsplit("/", 1)[0]
PAYLOAD = f"{HERE}/fixtures/captured-pr.json"
SENTINELS = {
    "pr_title": "SENTINEL-TITLE-0a1b",
    "pr_body": "SENTINEL-BODY-2c3d",
    "pr_diff": "SENTINEL-DIFF-4e5f",
    "pr_issue_comments": "SENTINEL-ISSUECOMMENT-6a7b",
    "pr_review_comments": "SENTINEL-REVIEWCOMMENT-8c9d",
    "pr_review_bodies": "SENTINEL-REVIEWBODY-aeaf",
    "linked_issue": "SENTINEL-LINKEDISSUE-c0d1",
}

failures: list[str] = []


def run(args: list[str]) -> tuple[int, str]:
    proc = subprocess.run(
        [sys.executable, f"{HERE}/contain.py", *args], capture_output=True, text=True
    )
    return proc.returncode, proc.stdout


def check(ok: bool, label: str) -> None:
    print(f"{'PASS' if ok else 'FAIL'}  {label}")
    if not ok:
        failures.append(label)


def blocks_of(document: str) -> dict[str, str]:
    """Split the rendered document into {entry_point: block body}."""
    pattern = re.compile(
        r"<<<BUZZ-UNTRUSTED:(\w+):([0-9a-f]{32})\n(.*?)\nBUZZ-UNTRUSTED:\1:\2>>>",
        re.DOTALL,
    )
    return {m.group(1): m.group(3) for m in pattern.finditer(document)}


# --- (a) captured payload, offline, deterministic ---------------------------
code, out = run(["--payload", PAYLOAD, "--seed", "step3", "--json"])
data = json.loads(out)
found = blocks_of(data["document"])

check(code == 0, "(a) captured payload exits 0")
check(set(found) == set(ENTRY_POINTS), f"(a) seven blocks, one per label (got {len(found)})")

for entry_point, sentinel in SENTINELS.items():
    in_own = sentinel in found.get(entry_point, "")
    elsewhere = [ep for ep, body in found.items() if ep != entry_point and sentinel in body]
    check(in_own and not elsewhere, f"(a) {sentinel} is in {entry_point} and nowhere else")

# Determinism: the same seed must produce byte-identical output.
_, out2 = run(["--payload", PAYLOAD, "--seed", "step3", "--json"])
check(out == out2, "(a) --seed makes output byte-deterministic")

# --- (b) live fetch against a PR that exists --------------------------------
code_live, out_live = run(["--pr", "92", "--seed", "step3", "--json"])
try:
    live = json.loads(out_live)
    live_labels = set(blocks_of(live["document"])) | {
        ep for ep, st in live["states"].items() if st in ("absent", "oversized", "unparseable")
    }
    check(live_labels == set(ENTRY_POINTS), "(b) --pr 92 accounts for all seven labels")
    check(code_live in (0, 2), f"(b) --pr 92 exits 0 or 2, got {code_live}")
except (json.JSONDecodeError, KeyError):
    check(False, "(b) --pr 92 produced parseable output")

# --- (c) each degenerate state, forced --------------------------------------
for state in ("absent", "empty", "oversized", "unparseable"):
    code_d, out_d = run(
        ["--payload", PAYLOAD, "--seed", "step3", "--degrade", f"pr_diff={state}", "--json"]
    )
    data_d = json.loads(out_d)
    doc = data_d["document"]
    rendered = blocks_of(doc)

    if state == "empty":
        # Fetched fine, genuinely nothing there: a block, explicitly labelled, exit 0.
        check(code_d == 0, "(c) empty exits 0")
        check("pr_diff" in rendered, "(c) empty renders a block")
        check(
            "(pr_diff fetched successfully and is empty)" in doc,
            "(c) empty is explicitly marked as fetched-and-empty",
        )
    else:
        check(code_d != 0, f"(c) {state} exits non-zero (got {code_d})")
        check("pr_diff" not in rendered, f"(c) {state} renders no block at all")
        check(
            f"SKIP pr_diff: {state}" in doc, f"(c) {state} renders an explicit SKIP with its state"
        )
        reason_line = next((ln for ln in doc.splitlines() if ln.startswith("SKIP pr_diff")), "")
        check(
            len(reason_line.split("—", 1)[-1].strip()) > 0,
            f"(c) {state} SKIP carries a reason",
        )

# The four dispositions must be distinguishable from one another.
renders = {}
for state in ("absent", "empty", "oversized", "unparseable"):
    _, out_d = run(
        ["--payload", PAYLOAD, "--seed", "step3", "--degrade", f"pr_diff={state}", "--json"]
    )
    doc = json.loads(out_d)["document"]
    renders[state] = next(
        (ln for ln in doc.splitlines() if "pr_diff" in ln and ("SKIP" in ln or "empty" in ln)), ""
    )
check(len(set(renders.values())) == 4, "(c) all four dispositions render distinctly")

print(f"\n{len(failures)} failure(s)")
sys.exit(1 if failures else 0)
