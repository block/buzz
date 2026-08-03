#!/usr/bin/env bash
# buzz-watch.sh [session-name|-] <channel-uuid> [poll-seconds]
#
# Emits one line per NEW message from a peer in the channel. Designed to be the
# `command` of Claude Code's Monitor tool with persistent: true — each stdout
# line becomes one notification, so this must be quiet unless something
# genuinely new arrived.
#
# Pass "-" as the session name (what buzz-connect.sh prints) and the watcher
# resolves this session's identity itself, so the command stays correct after a
# /rename. It loads the identity file too; nothing is sourced by hand.
#
# Three details this encodes; do not "simplify" them away:
#   1. `buzz messages get --since <ts>` is INCLUSIVE. A timestamp watermark
#      alone re-emits the newest message on every single poll. We dedupe on
#      event id instead and only use --since to bound the query.
#   2. The seen-set is primed from existing history on startup, so arming the
#      watcher does not replay the backlog as a burst of notifications.
#   3. A session must never react to its own messages — filter on own pubkey.
set -uo pipefail

HERE="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
. "$HERE/lib.sh"

NAME="${1:?usage: buzz-watch.sh [session-name|-] <channel-uuid> [poll-seconds]}"
CH="${2:?usage: buzz-watch.sh [session-name|-] <channel-uuid> [poll-seconds]}"
SLEEP="${3:-5}"

require_buzz

if [ "$NAME" = "-" ]; then
  IDENT=$("$HERE/buzz-session.sh" resolve) || exit 1
  NAME=$(printf '%s' "$IDENT" | cut -f1)
  ENV_FILE=$(printf '%s' "$IDENT" | cut -f4)
else
  ENV_FILE=$(identity_file "$NAME")
fi
[ -f "$ENV_FILE" ] || die \
"no identity '$NAME' in $SESSION_DIR — run $HERE/buzz-connect.sh first"

load_identity "$ENV_FILE" || die "could not load $ENV_FILE"
export RUST_LOG="${RUST_LOG:-error}"   # keep tracing off stdout

# Liveness marker so buzz-connect.sh --status can tell "watcher not armed" from
# "watcher armed and the channel is quiet" — two states that look identical.
MARKER=$(watch_marker "$NAME")
mkdir -p "$SESSION_DIR"
printf '%s\n%s\n' "$$" "$CH" > "$MARKER"

SEEN=$(mktemp -t buzz-watch-seen)
# TERM and INT too: Monitor stops a watcher by signalling it, and a marker left
# behind would make buzz-connect.sh --status claim a watcher that is gone.
trap 'rm -f "$SEEN" "$MARKER"; exit 0' EXIT INT TERM

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
