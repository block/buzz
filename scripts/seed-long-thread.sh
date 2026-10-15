#!/usr/bin/env bash
# Seed one long thread on the local dev relay, for reproducing and eyeballing
# long-thread loading in the apps.
#
#   just relay                  # stock local relay on ws://localhost:3000
#   just seed-long-thread 187   # or: scripts/seed-long-thread.sh [replies]
#
# Targets SEED_RELAY_URL (default: .env RELAY_URL, else ws://localhost:3000)
# and refuses non-loopback relays. Ambient BUZZ_RELAY_URL/BUZZ_PRIVATE_KEY are
# ignored so an agent or shell session can never seed its real relay. Posts
# with SEED_PRIVATE_KEY (default: a fresh throwaway key) into a new open stream
# channel, or into SEED_CHANNEL if that key may post there.
#
# Every write (channel, root, replies, reactions) counts against the relay's
# 60 human messages/min, so writes are paced SEED_DELAY seconds apart (default
# 1.05): the default run takes ~3-4 minutes on a stock relay. Set SEED_DELAY=0
# when BUZZ_RATE_LIMIT_HUMAN_MESSAGES_PER_MIN is raised on the relay.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

if [[ -f ".env" ]]; then
  set -o allexport
  # shellcheck disable=SC1091
  source .env
  set +o allexport
fi

REPLIES="${1:-187}"
REACTIONS="${SEED_REACTIONS:-20}"
export BUZZ_RELAY_URL="${SEED_RELAY_URL:-${RELAY_URL:-ws://localhost:3000}}"
export BUZZ_PRIVATE_KEY="${SEED_PRIVATE_KEY:-$(openssl rand -hex 32)}"
unset BUZZ_AUTH_TAG
if [[ ! "$BUZZ_RELAY_URL" =~ ^wss?://(localhost|127\.0\.0\.1|\[::1\])(:[0-9]+)?/?$ ]]; then
  echo "refusing non-local relay: $BUZZ_RELAY_URL" >&2
  exit 1
fi
BUZZ="${BUZZ_CLI:-cargo run -q -p buzz-cli --}"
DELAY="${SEED_DELAY:-1.05}"
write() { $BUZZ "$@"; sleep "$DELAY"; }

channel="${SEED_CHANNEL:-}"
if [[ -z "$channel" ]]; then
  channel="$(write channels create --name "long-thread-$$" --type stream --visibility open | jq -er .channel_id)"
fi
root="$(write messages send --channel "$channel" --content "long thread root ($REPLIES replies)" | jq -er .event_id)"

ids=()
for i in $(seq 1 "$REPLIES"); do
  ids+=("$(write messages send --channel "$channel" --reply-to "$root" --content "reply $i" | jq -er .event_id)")
done
if (( REPLIES > 0 )); then
  for i in $(seq 1 "$REACTIONS"); do
    write reactions add --event "${ids[$(( (i * 7) % REPLIES ))]}" --emoji "👍" >/dev/null
  done
fi

echo "channel=$channel"
echo "root=$root"
echo "replies=$REPLIES reactions=$(( REPLIES > 0 ? REACTIONS : 0 ))"
