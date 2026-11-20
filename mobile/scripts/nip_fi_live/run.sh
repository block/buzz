#!/usr/bin/env bash
# Runs the mobile app's hosted-relay auth against a real buzz-relay that
# enforces NIP-FI, then judges the relay's access log.
#
#   mobile/scripts/nip_fi_live/run.sh [workdir]
#
# Needs a built relay (`cargo build -p buzz-relay`), Postgres, Redis and an
# S3-compatible store reachable at the NIP_FI_* addresses in fixture.py (the
# defaults match scripts/start-isolated-test-relay.sh), `uv`, `psql`, and the
# Hermit toolchain (`. bin/activate-hermit`). It resets the database it is
# pointed at, and only if its name contains `nip_fi` and it is on this machine
# (the default is `buzz_nip_fi`, created on first use). The work directory
# must be outside the repository. Exit status 0 means the oracle passed.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../../.." && pwd -P)"
work="${1:-$(mktemp -d "${TMPDIR:-/tmp}/nip-fi-live.XXXXXX")}"

# The absolute, physical path of the work directory, resolved through its
# nearest existing ancestor so nothing is created yet.
absolute_path() {
  local path="$1" tail=""
  while [[ ! -d "$path" ]]; do
    tail="/$(basename "$path")$tail"
    path="$(dirname "$path")"
  done
  echo "$(cd "$path" && pwd -P)$tail"
}
work="$(absolute_path "$work")"

# Refuse a work directory inside the repository before creating anything. The
# fixture creates the directory itself and refuses one that is not new or empty.
case "$work/" in
  "$repo"/*)
    echo "refusing work directory $work: it is inside the repository and holds a test key" >&2
    exit 2
    ;;
esac

fixture_out="$(mktemp "${TMPDIR:-/tmp}/nip-fi-fixture.XXXXXX")"
fixture_pid=""
cleanup() {
  if [[ -n "$fixture_pid" ]] && kill -0 "$fixture_pid" 2>/dev/null; then
    kill -TERM "$fixture_pid" 2>/dev/null || true
    wait "$fixture_pid" 2>/dev/null || true
  fi
  # Keep the fixture's output with the run, but only in a directory the
  # fixture accepted (it leaves its marker there), never in a refused one.
  if [[ -f "$work/.nip-fi-live-workdir" && -f "$fixture_out" ]]; then
    mv -f "$fixture_out" "$work/fixture.out" 2>/dev/null || true
  fi
  rm -f "$fixture_out"
}
trap cleanup EXIT

uv run --quiet --with cryptography python "$here/fixture.py" --workdir "$work" \
  >"$fixture_out" 2>&1 &
fixture_pid=$!

for _ in $(seq 1 240); do
  [[ -s "$work/fixture.json" ]] && break
  if ! kill -0 "$fixture_pid" 2>/dev/null; then
    echo "fixture exited before it was ready:" >&2
    cat "$fixture_out" >&2
    exit 2
  fi
  sleep 0.5
done
[[ -s "$work/fixture.json" ]] || { echo "fixture was not ready in time" >&2; exit 2; }

# The hosted build settings, as a public Builderlab build would compile them.
# The names resolve to the fixture's TLS front inside the test; see
# test/nip_fi_live/nip_fi_live_fixture.dart.
test_status=0
(
  cd "$repo/mobile"
  BUZZ_NIP_FI_FIXTURE="$work/fixture.json" flutter test \
    --tags nip-fi-live --run-skipped \
    --dart-define=BUZZ_HOSTED_RELAY_AUTH=true \
    --dart-define=BUZZ_DISTRIBUTION=public \
    --dart-define=BUZZ_BUILDERLAB_URL=https://account.nipfi.localhost \
    --dart-define=BUZZ_BUILDERLAB_RELAYS=nipfi.localhost \
    test/nip_fi_live
) || test_status=$?

# Stop the fixture first: it records whether the relay was still alive.
kill -TERM "$fixture_pid" 2>/dev/null || true
wait "$fixture_pid" 2>/dev/null || true
fixture_pid=""

oracle_status=0
python3 "$here/oracle.py" "$work" || oracle_status=$?

echo
echo "flutter test exit: $test_status; oracle exit: $oracle_status; work dir: $work"
[[ $test_status -eq 0 && $oracle_status -eq 0 ]]
