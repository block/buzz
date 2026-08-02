#!/usr/bin/env bash
# buzz-watch.sh <session-name> <channel-uuid> [poll-seconds]
#
# Emits one line per NEW message from a peer in the channel. Designed to be the
# `command` of Claude Code's Monitor tool with persistent: true — each stdout
# line becomes one notification, so this must be quiet unless something
# genuinely new arrived.
#
# Three details this encodes; do not "simplify" them away:
#   1. `buzz messages get --since <ts>` is INCLUSIVE. A timestamp watermark
#      alone re-emits the newest message on every single poll. We dedupe on
#      event id instead and only use --since to bound the query.
#   2. The seen-set is primed from existing history on startup, so arming the
#      watcher does not replay the backlog as a burst of notifications.
#   3. A session must never react to its own messages — filter on own pubkey.
set -uo pipefail

NAME="${1:?usage: buzz-watch.sh <session-name> <channel-uuid> [poll-seconds]}"
CH="${2:?usage: buzz-watch.sh <session-name> <channel-uuid> [poll-seconds]}"
SLEEP="${3:-5}"

SESSION_DIR="${BUZZ_SESSION_DIR:-$HOME/.buzz/sessions}"
ENV_FILE="$SESSION_DIR/$NAME.env"
[ -f "$ENV_FILE" ] || { echo "no identity '$NAME' in $SESSION_DIR" >&2; exit 1; }

resolve_bin() {
  local name="$1" var="$2" found
  found="${!var:-}"
  if [ -n "$found" ]; then printf '%s' "$found"; return 0; fi
  if found=$(command -v "$name" 2>/dev/null); then printf '%s' "$found"; return 0; fi
  local root
  if root=$(git rev-parse --show-toplevel 2>/dev/null); then
    for cand in "$root/target/release/$name" "$root/target/debug/$name"; do
      [ -x "$cand" ] && { printf '%s' "$cand"; return 0; }
    done
  fi
  return 1
}
BUZZ=$(resolve_bin buzz BUZZ_BIN) || { echo "buzz not found on PATH (set BUZZ_BIN)" >&2; exit 1; }

# shellcheck source=/dev/null
set -a; . "$ENV_FILE"; set +a
export RUST_LOG="${RUST_LOG:-error}"   # keep tracing off stdout

SEEN=$(mktemp -t buzz-watch-seen)
trap 'rm -f "$SEEN"' EXIT

# --- prime: everything already in the channel counts as seen -----------------
"$BUZZ" messages get --channel "$CH" --limit 200 2>/dev/null \
  | python3 -c '
import json, sys
try:
    for m in json.load(sys.stdin):
        if m.get("id"):
            print(m["id"])
except Exception:
    pass
' > "$SEEN" 2>/dev/null || true

FILTER=$(cat <<'PY'
import json, os, sys
me = os.environ["ME"]
path = os.environ["SEEN"]
with open(path) as fh:
    seen = set(fh.read().split())
try:
    msgs = json.load(sys.stdin)
except Exception:
    sys.exit(0)
fresh = []
for m in sorted(msgs, key=lambda x: x.get("created_at", 0)):
    eid = m.get("id")
    if not eid or eid in seen:
        continue
    seen.add(eid)
    fresh.append(eid)
    if m.get("pubkey") == me:        # never react to our own messages
        continue
    if m.get("kind") not in (9, 1):  # chat kinds only
        continue
    who = m.get("pubkey", "")[:8]
    body = " ".join(m.get("content", "").split())[:400]
    print("[buzz] %s: %s" % (who, body), flush=True)
if fresh:
    with open(path, "a") as fh:
        fh.write("\n".join(fresh) + "\n")
PY
)

# Bound each query to a short trailing window; correctness comes from the
# id dedupe above, not from this watermark.
WINDOW="${BUZZ_WATCH_WINDOW:-300}"

while true; do
  SINCE=$(( $(date +%s) - WINDOW ))
  OUT=$("$BUZZ" messages get --channel "$CH" --since "$SINCE" --limit 100 2>/dev/null) || OUT=""
  if [ -n "$OUT" ]; then
    printf '%s' "$OUT" | ME="$BUZZ_PUBKEY" SEEN="$SEEN" python3 -c "$FILTER"
  fi
  sleep "$SLEEP"
done
