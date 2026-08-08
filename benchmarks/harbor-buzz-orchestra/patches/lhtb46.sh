#!/usr/bin/env bash
# Run one LHTB condition over ALL 46 tasks, with continue_until_timeout honored.
#
# Usage: lhtb46.sh <manifest-stem> <jobs-subdir> [n-concurrent] [task ...]
#   with no task list, runs all 46 tasks in ~/LHTB/tasks.
#
# Differs from lhtb.sh (the unpatched baseline runner) in exactly two ways, both
# deliberate and both breaking comparability with LHTB's published leaderboard:
#
#   1. All 46 tasks run, not 25. The other 21 shipped as allow_internet = false
#      so the agent could not reach OpenAI from inside the task container. They
#      have been flipped to true. Several of them (sokoban, 2048, sudoku-recovery,
#      rush_hour, snake_maze, generals-bot-arena) are puzzle games whose whole
#      design assumes the agent CANNOT look up a solver, so their scores are now
#      inflated by an unknown amount. Ours-only numbers.
#
#      One of the 21, `sudoku-recovery`, cannot run here at all and is a
#      known loss on BOTH cells (so the pairing is unaffected): it sets
#      `[agent] user = "agent"` to run the agent unprivileged as its anti-cheat
#      keystone, and the orchestra installs its stack into /opt/buzz, which that
#      user cannot create -- RuntimeLaunchError before the agent starts. It is
#      the only task in the set with a user override, and "fix" would mean
#      giving the agent root, which is precisely what the task is designed to
#      deny. Exclude it and report the loss; 45 tasks remain.
#
#      A surgical alternative was available and not taken: harbor's
#      --allow-environment-host promotes a no-network baseline to an allowlist
#      of exactly the hosts named (see merge_extra_allowlists in
#      harbor/trial/network_policy.py), which would have let the agent reach its
#      model and nothing else. Revisit if the game tasks look anomalous.
#
#   2. continue_until_timeout is honored, via patches/apply_continue_until_timeout.py.
#      Stock harbor ignores the flag, so 23 of the 25 tasks in the baseline run
#      went single-shot: agents self-reported DONE at a 3-8 minute mean against
#      60+ minute budgets and scored 0 completions.
#
# Timeout multiplier stays 3.0 and n stays 4, matching the unpatched baseline so
# patched-vs-unpatched differs in one variable. That makes this run LONG: with
# completions near zero the agents rarely exit early, so budget on roughly
# 345 agent-hours per cell (~86h wall at n=4).
set -euo pipefail

MANIFEST="${1:?usage: lhtb46.sh <manifest-stem> <jobs-subdir> [n-concurrent] [task ...]}"
JOBS="${2:?usage: lhtb46.sh <manifest-stem> <jobs-subdir> [n-concurrent] [task ...]}"
NCONC="${3:-4}"
shift 3 2>/dev/null || shift $#

cd "$HOME/buzz/benchmarks/harbor-buzz-orchestra"

# Refuse to burn days of model spend on a harbor that silently ignores the flag.
if ! testbed/.venv/bin/python -c \
  "from harbor.models.task.config import AgentConfig; \
   raise SystemExit(0 if 'continue_until_timeout' in AgentConfig.model_fields else 1)"
then
  echo "FATAL: harbor is not patched for continue_until_timeout." >&2
  echo "       run: testbed/.venv/bin/python patches/apply_continue_until_timeout.py" >&2
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
  NTASKS="$#"
else
  NTASKS="$(find "$HOME/LHTB/tasks" -mindepth 1 -maxdepth 1 -type d | wc -l)"
fi

echo "host      $(hostname)  ($BUZZ_BENCHMARK_DOCKER_HOST)"
echo "commit    $(git -C "$HOME/buzz" log --oneline -1)"
echo "manifest  manifests/${MANIFEST}.yaml"
echo "dataset   ~/LHTB/tasks  (${NTASKS} tasks, all allow_internet=true)"
echo "harbor    patched: continue_until_timeout honored"
echo "route     api.openai.com (direct)"
echo "jobs      jobs/${JOBS}   n=${NCONC}   attempts=1   clock=3x"
echo "started   $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo

exec uv run --project testbed --no-sync scripts/benchmark.py \
  --path "$HOME/LHTB/tasks" \
  "${INC[@]}" \
  --manifest "manifests/${MANIFEST}.yaml" \
  --endpoint-config testbed/endpoints/openai-live.json \
  --jobs-dir "jobs/${JOBS}" \
  --attempts 1 \
  --n-concurrent "$NCONC" \
  --timeout-multiplier 3.0
