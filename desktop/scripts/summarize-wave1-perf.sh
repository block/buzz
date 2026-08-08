#!/usr/bin/env bash
# Summarize WAVE1_PERF success logs into median/p95 aggregates.
set -euo pipefail
FILE="${1:?success log}"
python3 - <<'PY' "$FILE"
import sys, statistics
from pathlib import Path
path = Path(sys.argv[1])
rows = []
for line in path.read_text().splitlines():
    if not line.startswith("WAVE1_PERF"):
        continue
    parts = dict(p.split("=", 1) for p in line.split()[1:])
    rows.append({k: float(v) for k, v in parts.items() if k != "repeat"})
print(f"n={len(rows)} file={path}")
if not rows:
    raise SystemExit(1)

def agg(key):
    vals = [r[key] for r in rows]
    return (
        statistics.median(vals),
        statistics.quantiles(vals, n=20)[18] if len(vals) >= 2 else vals[0],
        min(vals),
        max(vals),
        sum(vals) / len(vals),
    )

for key in [
    "quiet_median",
    "quiet_p95",
    "quiet_over50",
    "quiet_longtask",
    "busy_median",
    "busy_p95",
    "busy_over50",
    "busy_longtask",
]:
    med, p95, mn, mx, mean = agg(key)
    print(f"{key}: median={med:.1f} p95={p95:.1f} min={mn:.1f} max={mx:.1f} mean={mean:.1f}")

busy_minus_quiet = [r["busy_median"] - r["quiet_median"] for r in rows]
print(
    "busy_minus_quiet_median: "
    f"median={statistics.median(busy_minus_quiet):.1f} "
    f"mean={sum(busy_minus_quiet)/len(busy_minus_quiet):.1f}"
)
PY
