#!/usr/bin/env python3
"""Decide whether an LHTB smoke run earned its full run.

Usage: gatecheck.py <jobs-dir> <required-seat> [<required-seat> ...]

Prints one KEY=VALUE line per signal and exits 0 only if every gate passes.
Four gates, each of which has silently ruined a run before:

  seats      every required seat id appears in receipts. A goosetown team whose
             role name fails to resolve still reports every send as a success;
             the trial burns its whole budget and reads as a bad score rather
             than a broken manifest.
  phases     continue_until_timeout fired more than once. If harbor is running
             unpatched the trial is single-shot and the whole point of the run
             is gone -- and the applier's own --check cannot tell a stale patch
             from a current one.
  receipts   metadata.receipts_phase_files is present. This is the first run
             with the per-phase receipts fix; without it the bill is whatever
             the last phase happened to cost, which under-reported a team cell
             by 87% on the previous LHTB wave.
  scored     the verifier produced a reward (may be 0.0; None means the trial
             never reached scoring).
"""

import json
import pathlib
import re
import sys

jobs = pathlib.Path(sys.argv[1])
required = set(sys.argv[2:])


def role(agent_id: str) -> str:
    """Roster id behind a receipt's agent_id.

    Receipts carry the *instance* id -- `lead-1`, `scout-1`, `worker-1` -- while
    the manifest roster and this script's arguments carry the role, because a
    seat with `count: 2` produces `worker-1` and `worker-2`. Comparing the two
    directly fails the gate on a cell where every seat spoke perfectly.
    """
    return re.sub(r"-\d+$", "", agent_id)


seats: set[str] = set()
for f in jobs.glob("*/*/agent/buzz/receipts.jsonl"):
    for line in f.read_text().splitlines():
        if line.strip():
            seats.add(role(json.loads(line).get("agent_id", "?")))

results = sorted(jobs.glob("*/*/result.json"))
phases, phased_receipts, rewards = [], [], []
for r in results:
    d = json.loads(r.read_text())
    md = ((d.get("agent_result") or {}).get("metadata")) or {}
    phases.append(int(md.get("continue_until_timeout_phases") or 0))
    phased_receipts.append(md.get("receipts_phase_files") is not None)
    vr = (d.get("verifier_result") or {}).get("rewards") or {}
    rewards.append(vr.get("reward"))

ok_seats = required <= seats
ok_phases = bool(phases) and max(phases) > 1
ok_receipts = bool(phased_receipts) and all(phased_receipts)
ok_scored = bool(rewards) and all(r is not None for r in rewards)

print(f"trials={len(results)}")
print(f"seats={','.join(sorted(seats)) or '(none)'}  required={','.join(sorted(required))}")
print(f"phases={phases}")
print(f"receipts_phase_files_present={phased_receipts}")
print(f"rewards={rewards}")
print(f"GATE seats={ok_seats} phases={ok_phases} receipts={ok_receipts} scored={ok_scored}")

sys.exit(0 if (ok_seats and ok_phases and ok_receipts and ok_scored) else 1)
