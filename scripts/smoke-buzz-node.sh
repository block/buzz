#!/usr/bin/env bash
set -euo pipefail

# Start buzz-node against an already-running local relay and verify its
# process health and relay readiness contract.
relay_url="${1:-ws://localhost:3000}"
health_addr="${BUZZ_NODE_SMOKE_HEALTH_ADDR:-127.0.0.1:18081}"
health_url="http://${health_addr}"
data_dir="$(mktemp -d "${TMPDIR:-/tmp}/buzz-node-smoke.XXXXXX")"
node_pid=""

cleanup() {
  if [[ -n "$node_pid" ]]; then
    kill "$node_pid" 2>/dev/null || true
    wait "$node_pid" 2>/dev/null || true
  fi
  rm -rf "$data_dir"
}
trap cleanup EXIT

BUZZ_RELAY_URL="$relay_url" \
BUZZ_NODE_DATA_DIR="$data_dir" \
BUZZ_NODE_HEALTH_ADDR="$health_addr" \
cargo run -p buzz-node --quiet -- run >"$data_dir/node.log" 2>&1 &
node_pid=$!

for _ in $(seq 1 60); do
  if curl --silent --fail --max-time 1 "$health_url/health" >/dev/null \
    && curl --silent --fail --max-time 1 "$health_url/ready" >/dev/null; then
    echo "buzz-node connected to $relay_url"
    exit 0
  fi
  if ! kill -0 "$node_pid" 2>/dev/null; then
    cat "$data_dir/node.log" >&2
    exit 1
  fi
  sleep 0.5
done

cat "$data_dir/node.log" >&2
echo "buzz-node did not become relay-ready" >&2
exit 1
