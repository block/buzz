#!/usr/bin/env bash
# Cheap smoke of one LHTB cell: real manifest, real route, real patched harbor,
# but a deliberately tiny --timeout-multiplier so the agent budget is ~9 min
# instead of 3h.
#
# Usage: lhtb-smoke.sh <manifest-stem> <jobs-subdir> [task ...]
#   defaults to great-expectations-audit, the cheapest continue_until_timeout
#   task and the one both baseline cells scored at exactly 0.3636.
#
# SMOKE_MULT (default 0.15) sets the clock. 0.15 is ~9 min of agent budget and
# is enough for a default-effort cell -- and for a high-effort *team*, which
# turned two phases in that window. It is NOT enough for a high-effort solo:
# tb-solo-sol-high timed out inside phase 1 at 540s, so the trial never reached
# the verifier and the gate read `phases=0 rewards=[None]` on a cell that was
# working fine. Raise it to 0.5 (~30 min) for anything pinned to high effort.
# The gate is not part of the measurement, so the two cells of a pair may smoke
# at different multipliers; only the full runs have to match.
#
# The point is not the score -- at 0.15x nothing scores well. The point is that
# the plumbing holds: the seats speak, the phase loop fires more than once, the
# relay forwarder survives the inter-phase teardown, and receipts land per
# phase rather than being overwritten by the last one.
set -euo pipefail

MANIFEST="${1:?usage: lhtb-smoke.sh <manifest-stem> <jobs-subdir> [task ...]}"
JOBS="${2:?usage: lhtb-smoke.sh <manifest-stem> <jobs-subdir> [task ...]}"
MULT="${SMOKE_MULT:-0.15}"
shift 2

cd "$HOME/buzz/benchmarks/harbor-buzz-orchestra"

if ! testbed/.venv/bin/python -c \
  "from harbor.models.task.config import AgentConfig; \
   raise SystemExit(0 if 'continue_until_timeout' in AgentConfig.model_fields else 1)"
then
  echo "FATAL: harbor is not patched for continue_until_timeout." >&2
  exit 1
fi

export PATH="$HOME/buzz/target/debug:$PATH"
export BUZZ_BENCHMARK_DOCKER_HOST="$(hostname -I | awk '{print $1}')"
export AWS_DEFAULT_REGION=us-east-1
export OPENAI_API_KEY="$(aws secretsmanager get-secret-value \
  --secret-id buzz-oss/staging/benchmark/openai-api-key \
  --query SecretString --output text)"

INC=()
if [ "$#" -gt 0 ]; then
  for t in "$@"; do INC+=(--include-task "$t"); done
else
  INC=(--include-task great-expectations-audit)
fi

echo "host      $(hostname)  ($BUZZ_BENCHMARK_DOCKER_HOST)"
echo "commit    $(git -C "$HOME/buzz" log --oneline -1)"
echo "manifest  manifests/${MANIFEST}.yaml"
echo "harbor    patched: continue_until_timeout honored"
echo "jobs      jobs/${JOBS}   n=1   attempts=1   clock=${MULT}x (SMOKE)"
echo "started   $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo

exec uv run --project testbed --no-sync scripts/benchmark.py \
  --path "$HOME/LHTB/tasks" \
  "${INC[@]}" \
  --manifest "manifests/${MANIFEST}.yaml" \
  --endpoint-config testbed/endpoints/openai-live.json \
  --jobs-dir "jobs/${JOBS}" \
  --attempts 1 \
  --n-concurrent 1 \
  --timeout-multiplier "$MULT"
