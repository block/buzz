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
# pointed at. Exit status 0 means the oracle passed.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
work="${1:-$(mktemp -d "${TMPDIR:-/tmp}/nip-fi-live.XXXXXX")}"
mkdir -p "$work"
work="$(cd "$work" && pwd)"

fixture_pid=""
cleanup() {
  if [[ -n "$fixture_pid" ]] && kill -0 "$fixture_pid" 2>/dev/null; then
    kill -TERM "$fixture_pid" 2>/dev/null || true
    wait "$fixture_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT

rm -f "$work"/fixture.json "$work"/fixture-result.json "$work"/markers.jsonl "$work"/relay.log
uv run --quiet --with cryptography python "$here/fixture.py" --workdir "$work" \
  >"$work/fixture.out" 2>&1 &
fixture_pid=$!

for _ in $(seq 1 240); do
  [[ -s "$work/fixture.json" ]] && break
  if ! kill -0 "$fixture_pid" 2>/dev/null; then
    echo "fixture exited before it was ready:" >&2
    cat "$work/fixture.out" >&2
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
