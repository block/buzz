#!/usr/bin/env bash
# =============================================================================
# ci-mesh-lifecycle-smoke.sh — relay-driven mesh lifecycle smoke
# =============================================================================
# Provisions a membership-gated buzz-relay and runs the full relay-driven
# mesh lifecycle harness (crates/buzz-relay/examples/mesh_relay_lifecycle_smoke.rs):
# membership → signed discovery notes → relay-derived allowlist → join →
# inference over QUIC → stranger denied.
#
# Mirrors the shape of mesh-llm's own CI smoke scripts (single runner, tiny
# CPU model, real multi-process mesh), but with the Buzz relay as the control
# plane instead of a hand-carried invite token.
#
# Usage:
#   ./scripts/ci-mesh-lifecycle-smoke.sh [--profile <cargo-profile>] [--no-build]
#
# Env:
#   MESH_SMOKE_MODEL   Override the served model ref (default: SmolLM2-135M).
# =============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

CARGO_PROFILE="${CARGO_PROFILE:-ci}"
SKIP_BUILD=false
while [[ $# -gt 0 ]]; do
  case "$1" in
    --profile) CARGO_PROFILE="$2"; shift 2 ;;
    --no-build) SKIP_BUILD=true; shift ;;
    *) echo "Unknown option: $1" >&2; exit 1 ;;
  esac
done

BLUE='\033[0;34m'
GREEN='\033[0;32m'
RED='\033[0;31m'
NC='\033[0m'
log() { echo -e "${BLUE}[mesh-lifecycle]${NC} $*"; }
ok()  { echo -e "${GREEN}[mesh-lifecycle]${NC} $*"; }
err() { echo -e "${RED}[mesh-lifecycle]${NC} $*" >&2; }

# ── Build ─────────────────────────────────────────────────────────────────────

if [[ "${SKIP_BUILD}" == "true" ]]; then
  log "Skipping build (--no-build)"
else
  log "Building relay, admin CLI, and lifecycle harness (profile: ${CARGO_PROFILE})..."
  cargo build --profile "${CARGO_PROFILE}" -p buzz-relay -p buzz-admin -p git-credential-nostr
  cargo build --profile "${CARGO_PROFILE}" -p buzz-relay --example mesh_relay_lifecycle_smoke
fi

ADMIN_BIN="target/${CARGO_PROFILE}/buzz-admin"
HARNESS_BIN="target/${CARGO_PROFILE}/examples/mesh_relay_lifecycle_smoke"
for bin in "${ADMIN_BIN}" "${HARNESS_BIN}" "target/${CARGO_PROFILE}/buzz-relay"; do
  if [[ ! -x "${bin}" ]]; then
    err "Missing binary: ${bin}"
    exit 1
  fi
done

# ── Relay identity + membership gating ───────────────────────────────────────
# The relay owner and signing key are throwaway CI identities. RELAY_OWNER
# never participates in the mesh; it only satisfies the NIP-43 requirement
# that a membership-gated relay has an administrable owner.

log "Generating relay owner + signing identities..."
read_keys() { "${ADMIN_BIN}" generate-key 2>/dev/null; }
OWNER_OUT="$(read_keys)"
RELAY_OWNER_PUBKEY="$(echo "${OWNER_OUT}" | awk '/Public key:/ {print $3}')"
SIGNER_OUT="$(read_keys)"
BUZZ_RELAY_PRIVATE_KEY="$(echo "${SIGNER_OUT}" | awk '/Secret key:/ {print $3}')"
if [[ -z "${RELAY_OWNER_PUBKEY}" || -z "${BUZZ_RELAY_PRIVATE_KEY}" ]]; then
  err "Failed to generate relay identities via buzz-admin generate-key"
  exit 1
fi
export BUZZ_REQUIRE_RELAY_MEMBERSHIP=true
export RELAY_OWNER_PUBKEY
export BUZZ_RELAY_PRIVATE_KEY

# ── Start the membership-gated relay ─────────────────────────────────────────
# A stale relay on :3000 would pass the readiness poll while silently running
# WITHOUT membership gating — the stranger-denied assertion would then fail
# (or worse, an open relay would mask a real gating regression). Fail fast.
if lsof -nP -iTCP:3000 -sTCP:LISTEN >/dev/null 2>&1; then
  err "Port 3000 is already in use — stop the existing relay first (its config would not be membership-gated)"
  exit 1
fi

log "Starting membership-gated relay..."
CARGO_PROFILE="${CARGO_PROFILE}" ./scripts/start-relay-for-tests.sh --no-build

cleanup() {
  log "Stopping relay..."
  if [[ -f /tmp/buzz-relay.pid ]]; then
    kill "$(cat /tmp/buzz-relay.pid)" 2>/dev/null || true
  fi
  # The harness kills its own children; sweep any stragglers from a hard fail.
  pkill -f mesh_relay_lifecycle_smoke 2>/dev/null || true
}
trap cleanup EXIT

# ── Run the lifecycle harness ────────────────────────────────────────────────

log "Running relay-driven mesh lifecycle smoke..."
RELAY_URL=ws://localhost:3000 \
DATABASE_URL=postgres://buzz:buzz_dev@localhost:5432/buzz \
REDIS_URL=redis://localhost:6379 \
BUZZ_ADMIN_BIN="${ADMIN_BIN}" \
  "${HARNESS_BIN}"

ok "Relay-driven mesh lifecycle smoke passed"
