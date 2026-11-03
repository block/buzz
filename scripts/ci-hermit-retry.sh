#!/usr/bin/env bash
# =============================================================================
# ci-hermit-retry.sh — fetch a Hermit tool with a per-attempt deadline
# =============================================================================
# CI jobs start with an empty Hermit package cache, so the first run of a bin/
# stub downloads its package. That download has no deadline: a stalled Node
# download hung Detect Changed Paths until the job's cap cancelled every gate.
# Hermit tries a download three times, a second apart, so a short run of
# GitHub release-asset 504s fails the job outright.
#
# Run a cheap command for each tool through this before the job's first real
# use (`scripts/ci-hermit-retry.sh ./bin/node --version`). Each attempt runs in
# its own process group, which is stopped at the deadline (TERM, then KILL)
# and retried after a growing backoff.
#
# A stopped attempt leaves nothing Hermit reuses. Hermit downloads to a
# temporary file and unpacks to a temporary directory, renaming each into place
# only once complete, and its state lock is an flock that the kernel drops with
# the process. The leftover temporaries only cost disk.
#
# With the defaults one invocation is bounded at under 6 minutes. Size job
# timeouts to fit that plus the job's real work. A job with a tight cap can
# fetch several tools in one invocation through `sh -c`, as Detect Changed
# Paths does.
#
# Runs under macOS's /bin/bash 3.2 and does not use timeout(1), which macOS
# runners lack.
#
# Usage:
#   scripts/ci-hermit-retry.sh <command> [args...]
#
# Env:
#   CI_HERMIT_ATTEMPTS            Attempts before failing (default: 3).
#   CI_HERMIT_ATTEMPT_SECONDS     Deadline per attempt (default: 90). Healthy
#                                 fetches finish well inside a minute.
#   CI_HERMIT_KILL_AFTER_SECONDS  Wait after TERM before KILL (default: 10).
#   CI_HERMIT_BACKOFF_SECONDS     Sleep after failed attempt n is n times this
#                                 (default: 15).
# =============================================================================
set -euo pipefail

attempts="${CI_HERMIT_ATTEMPTS:-3}"
deadline="${CI_HERMIT_ATTEMPT_SECONDS:-90}"
kill_after="${CI_HERMIT_KILL_AFTER_SECONDS:-10}"
backoff="${CI_HERMIT_BACKOFF_SECONDS:-15}"

if [ "$#" -eq 0 ]; then
  echo "usage: $0 <command> [args...]" >&2
  exit 2
fi

for attempt in $(seq 1 "$attempts"); do
  # Job control puts each background job in its own process group, so the
  # watchdog's signals reach everything the attempt started.
  set -m
  "$@" &
  pid=$!
  (
    sleep "$deadline"
    kill -TERM -- "-$pid" 2>/dev/null || exit 0
    sleep "$kill_after"
    kill -KILL -- "-$pid" 2>/dev/null || true
  ) >/dev/null 2>&1 &
  watchdog=$!
  set +m
  started=$SECONDS
  rc=0
  # Bash reports a job killed by a signal on wait's stderr; the warning below
  # says the same thing.
  wait "$pid" 2>/dev/null || rc=$?
  kill -- "-$watchdog" 2>/dev/null || true
  wait "$watchdog" 2>/dev/null || true
  [ "$rc" -eq 0 ] && exit 0
  # The leader can exit on TERM while a child that ignores it lives on.
  kill -KILL -- "-$pid" 2>/dev/null || true
  if [ $((SECONDS - started)) -ge "$deadline" ]; then
    echo "::warning::attempt $attempt/$attempts timed out after ${deadline}s: $*"
  else
    echo "::warning::attempt $attempt/$attempts failed (exit $rc): $*"
  fi
  if [ "$attempt" -lt "$attempts" ]; then
    sleep $((attempt * backoff))
  fi
done
echo "::error::all $attempts attempts failed: $*"
exit 1
