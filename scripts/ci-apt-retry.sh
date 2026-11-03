#!/usr/bin/env bash
# =============================================================================
# ci-apt-retry.sh — run an apt-backed CI command with a per-attempt deadline
# =============================================================================
# Ubuntu mirrors on hosted runners are sometimes degraded: most requests hit
# apt's Acquire timeouts and apt keeps re-queuing them, so one apt step can run
# past the job timeout without failing. This kills each attempt after a
# deadline, stops any apt-get/dpkg the attempt left behind, and retries.
# Downloaded .debs stay in /var/cache/apt/archives, so a retry resumes where
# the last attempt stopped. It retries the same mirror, so
# against a mirror that stays degraded this bounds each invocation (about 25m
# with the defaults) rather than recovering it. Size job timeouts to fit the
# bound of every invocation in the job plus the job's real work.
#
# Ephemeral hosted runners only: between attempts it stops every apt-get/dpkg
# on the machine, not just the ones it started. Don't use it on a self-hosted
# runner or a developer machine.
#
# It also writes the apt retry/timeout options to apt.conf.d, so commands that
# call apt-get themselves (e.g. `playwright install-deps`) pick them up too.
#
# Usage:
#   scripts/ci-apt-retry.sh <command> [args...]
#
# Env:
#   CI_APT_ATTEMPTS         Attempts before failing (default: 3).
#   CI_APT_ATTEMPT_SECONDS  Deadline per attempt (default: 420). Healthy apt
#                           steps finish in under 4 minutes; degraded ones run
#                           past 13.
#   CI_APT_REPAIR_SECONDS   Deadline for the dpkg repair between attempts
#                           (default: 60).
# =============================================================================
set -euo pipefail

attempts="${CI_APT_ATTEMPTS:-3}"
deadline="${CI_APT_ATTEMPT_SECONDS:-420}"
repair_deadline="${CI_APT_REPAIR_SECONDS:-60}"

if [ "$#" -eq 0 ]; then
  echo "usage: $0 <command> [args...]" >&2
  exit 2
fi

printf '%s\n' \
  'Acquire::Retries "3";' \
  'Acquire::http::Timeout "30";' \
  'Acquire::https::Timeout "30";' \
  'DPkg::Lock::Timeout "120";' |
  sudo tee /etc/apt/apt.conf.d/80-ci-timeouts >/dev/null

# Intentionally machine-wide: any other apt-get/dpkg would hold the same lock.
reap_package_managers() {
  sudo pkill -TERM -x 'apt-get|dpkg' || true
  for _ in $(seq 1 15); do
    pgrep -x 'apt-get|dpkg' >/dev/null || return 0
    sleep 1
  done
  sudo pkill -KILL -x 'apt-get|dpkg' || true
}

for attempt in $(seq 1 "$attempts"); do
  rc=0
  timeout --kill-after=15s "$deadline" "$@" || rc=$?
  [ "$rc" -eq 0 ] && exit 0
  if [ "$rc" -eq 124 ] || [ "$rc" -eq 137 ]; then
    echo "::warning::attempt $attempt/$attempts timed out after ${deadline}s: $*"
  else
    echo "::warning::attempt $attempt/$attempts failed (exit $rc): $*"
  fi
  # timeout(1) signals only its own process group, but sudo runs its command
  # in a new one. When apt-get is not sudo's direct command (playwright
  # install-deps runs `sudo sh -c "apt-get update && apt-get install ..."`),
  # the forwarded signal stops sh and apt-get survives, holding the dpkg lock
  # so every retry is a lock wait. Stop any survivor before retrying, and
  # before giving up so no apt-get outlives the step.
  reap_package_managers
  [ "$attempt" -eq "$attempts" ] && break
  # A killed apt/dpkg run can leave dpkg half-configured; repair before
  # retrying, under its own deadline so a stuck repair can't eat the job.
  timeout --kill-after=15s "$repair_deadline" sudo dpkg --configure -a || true
done
echo "::error::all $attempts attempts failed: $*"
exit 1
