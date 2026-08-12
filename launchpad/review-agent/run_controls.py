"""Run every containment control. One entry point, so CI has one thing to invoke.

A control whose input is missing reports SKIP with a reason and **never** PASS, and a
SKIP is surfaced in the summary rather than folded into the pass count. Absence of
evidence is not evidence — the same rule the contained surfaces themselves obey.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).parent

#: (script, needs_network). The step-7 suite is invoked here; CI calls this runner.
CONTROLS = [
    ("check_contract.py", False),
    ("check_step2.py", False),
    ("check_step3.py", True),
    ("check_step45.py", False),
    ("check_step6.py", True),
    ("suite.py", False),
    ("check_step8.py", False),
    ("check_step9.py", False),
    ("check_step11.py", False),
    ("check_invariants.py", False),
    ("check_mutations.py", False),
]


def network_available() -> tuple[bool, str]:
    if shutil.which("gh") is None:
        return False, "gh is not installed"
    probe = subprocess.run(
        ["gh", "api", "rate_limit"], capture_output=True, text=True, timeout=30
    )
    if probe.returncode != 0:
        return False, "gh is installed but unauthenticated or offline"
    return True, ""


def main() -> int:
    online, reason = network_available()
    passed, failed, skipped = [], [], []

    for script, needs_network in CONTROLS:
        if needs_network and not online:
            skipped.append((script, reason))
            print(f"SKIP  {script} — {reason}")
            continue
        proc = subprocess.run([sys.executable, str(HERE / script)], capture_output=True, text=True)
        if proc.returncode == 0:
            passed.append(script)
            print(f"PASS  {script}")
        else:
            failed.append(script)
            print(f"FAIL  {script}")
            print(proc.stdout[-2000:])
            print(proc.stderr[-1000:])

    print(f"\n{len(passed)} passed, {len(failed)} failed, {len(skipped)} skipped")
    for script, why in skipped:
        print(f"  SKIPPED {script}: {why} — this is not a pass")

    if failed:
        return 1
    if skipped:
        # A skipped control has not run. Do not report a clean gate.
        print("\nIncomplete: network-dependent controls did not run.")
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
