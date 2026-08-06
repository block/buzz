#!/usr/bin/env bash
# Reproduce the desktop release's multi-package build shape, then prove the
# Kubernetes provider reports an unreachable cluster in-band rather than
# panicking when rustls has multiple compiled crypto providers available.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
PROVIDER_BINARY="$TARGET_DIR/release/buzz-backend-kubernetes"

CARGO="${CARGO:-cargo}"
"$CARGO" build --release \
  -p buzz-acp \
  -p buzz-agent \
  -p buzz-dev-mcp \
  -p git-credential-nostr \
  -p buzz-cli \
  -p buzz-backend-kubernetes

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
cat >"$TMP/kubeconfig" <<'YAML'
apiVersion: v1
kind: Config
clusters:
  - name: unreachable
    cluster:
      server: http://127.0.0.1:9
contexts:
  - name: unreachable
    context:
      cluster: unreachable
      user: none
current-context: unreachable
users:
  - name: none
    user: {}
YAML

cat >"$TMP/request.json" <<'JSON'
{
  "op": "deploy",
  "agent": {
    "relay_url": "wss://relay.example",
    "private_key_nsec": "nsec1vl029mgpspedva04g90vltkh6fvh240zqtv9k0t9af8935ke9laqsnlfe5",
    "auth_tag": "owner",
    "launch": {"command": "goose", "args": [], "env": {}, "policy_env": {}}
  },
  "provider_config": {
    "context": "unreachable",
    "namespace": "never-created",
    "image": "example.invalid/sprig@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "inactivity_seconds": 60
  }
}
JSON

set +e
KUBECONFIG="$TMP/kubeconfig" \
  "$PROVIDER_BINARY" \
  <"$TMP/request.json" >"$TMP/stdout" 2>"$TMP/stderr"
status=$?
set -e

[[ "$status" == 0 ]] || {
  echo "provider exited $status instead of returning an in-band failure" >&2
  cat "$TMP/stderr" >&2
  exit 1
}
[[ "$(wc -l <"$TMP/stdout" | tr -d ' ')" == 1 ]]
jq -e '.ok == false and (.error | type == "string" and length > 0)' "$TMP/stdout" >/dev/null
if rg -i 'panic|crypto provider|no process-level' "$TMP/stdout" "$TMP/stderr"; then
  echo "provider emitted a panic/crypto-provider failure" >&2
  exit 1
fi
printf 'PASS: release-shaped provider returned one in-band unreachable-cluster failure\n'
