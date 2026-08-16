"""Step 9 control: the suite must collapse when containment is removed.

The bar is **all 35**, with no slack. Revision 2 of the plan set it at 20 of 25, and an
independent review found the hole: a gap that size is exactly one entry-point row, so a
seam wired into six of seven surfaces would hide inside it and the suite would still
report a pass. A seam that disables containment everywhere leaves no case standing, so
any survivor means a surface the seam never reached.

A suite that passes with and without the protection tests nothing. This is the control
that proves it does not.
"""

from __future__ import annotations

import os
import sys

from contain import CONTROL_FLAGS_ENV_VAR, ENTRY_POINTS
from suite import run_all

# This control's entire purpose is proving the seam collapses containment — the one
# legitimate reason to pass enabled=False directly. contain()'s own runtime guard
# (#137) requires this explicitly, the same way the CLI does.
os.environ[CONTROL_FLAGS_ENV_VAR] = "true"

failures: list[str] = []


def check(ok: bool, label: str) -> None:
    print(f"{'PASS' if ok else 'FAIL'}  {label}")
    if not ok:
        failures.append(label)


# --- containment on: everything holds --------------------------------------
on = run_all(enabled=True)
on_failed = [r.case_id for r in on if not r.passed]
check(len(on) == 35, f"35 controls with containment on (got {len(on)})")
check(not on_failed, f"all 35 pass with containment on (failing: {on_failed})")

# --- containment off: everything collapses ---------------------------------
off = run_all(enabled=False)
off_survivors = [r.case_id for r in off if r.passed]
off_contained = [r.case_id for r in off if r.contained]

check(len(off) == 35, f"35 controls with the seam off (got {len(off)})")
check(
    not off_survivors,
    f"all 35 FAIL with the seam off — no slack (survivors: {off_survivors})",
)
check(
    not off_contained,
    f"no case reports containment with the seam off (still contained: {off_contained})",
)

# Every entry point must be reached by the seam. This is the specific hole the
# 20-of-25 bar would have concealed: one surface silently untouched.
per_entry_survivors = {
    ep: [r.case_id for r in off if r.case_id.endswith(f"@{ep}") and r.contained]
    for ep in ENTRY_POINTS
}
unreached = {ep: ids for ep, ids in per_entry_survivors.items() if ids}
check(not unreached, f"the seam reaches all seven entry points (unreached: {list(unreached)})")

# The seam must change the outcome, not merely exist. If on and off agree, the
# suite is measuring something other than containment.
same = [
    r_on.case_id
    for r_on, r_off in zip(on, off, strict=True)
    if r_on.passed == r_off.passed
]
check(not same, f"every case changes verdict between on and off (unchanged: {len(same)})")

print(f"\n  containment on : {len(on) - len(on_failed)}/35 pass")
print(f"  containment off: {len(off) - len(off_survivors)}/35 fail")
print(f"\n{len(failures)} failure(s)")
sys.exit(1 if failures else 0)
