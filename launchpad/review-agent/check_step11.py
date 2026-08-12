"""Step 11 control: the CI workflow.

The trigger and permission assertions are the point. The suite under test lives in the
repository, so a pull request can modify the code this job runs. On `pull_request` that
runs with the fork's permissions and no secrets; `pull_request_target` would hand
attacker-modifiable code the base repository's token. Nothing else in this plan would
have caught that substitution — an independent review of revision 2 found the criterion
silent on it.
"""

from __future__ import annotations

import sys
from pathlib import Path

WORKFLOW = Path(__file__).resolve().parents[2] / ".github" / "workflows" / "launchpad-review-agent-controls.yml"

failures: list[str] = []


def check(ok: bool, label: str) -> None:
    print(f"{'PASS' if ok else 'FAIL'}  {label}")
    if not ok:
        failures.append(label)


check(WORKFLOW.exists(), f"workflow exists at {WORKFLOW.name}")
if not WORKFLOW.exists():
    sys.exit(1)

check(WORKFLOW.name.startswith("launchpad-"), "named launchpad-*.yml per AGENTS.md §3")

try:
    import yaml

    doc = yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))
    parsed = True
except ImportError:
    doc, parsed = None, False
    print("SKIP  YAML parse — PyYAML not installed (this is not a pass)")
    failures.append("YAML parse skipped")

if parsed:
    # `on` parses as the boolean True in YAML 1.1 unless quoted.
    triggers = doc.get("on", doc.get(True, {}))
    check("pull_request" in triggers, f"triggers on pull_request (got {list(triggers)})")
    check(
        "pull_request_target" not in triggers,
        "does NOT trigger on pull_request_target",
    )

    perms = doc.get("permissions", {})
    check(perms.get("contents") == "read", f"declares contents: read (got {perms.get('contents')})")
    writes = [scope for scope, level in perms.items() if level == "write"]
    check(not writes, f"declares no write scope (found: {writes})")

    jobs = doc.get("jobs", {})
    steps = [s for job in jobs.values() for s in job.get("steps", [])]
    runs = " ".join(s.get("run", "") for s in steps)
    check("run_controls.py" in runs, "invokes the controls runner")

# The runner must actually invoke the step-7 suite, or CI would run everything but the
# matrix the issue is about.
runner = (Path(__file__).parent / "run_controls.py").read_text(encoding="utf-8")
check("suite.py" in runner, "the runner invokes the step-7 suite (suite.py)")

# Raw text of the workflow, for assertions parsing would hide.
raw = WORKFLOW.read_text(encoding="utf-8")
check(
    "pull_request_target" in raw,
    "the workflow explains why pull_request_target is excluded",
)

print(f"\n{len(failures)} failure(s)")
sys.exit(1 if failures else 0)
