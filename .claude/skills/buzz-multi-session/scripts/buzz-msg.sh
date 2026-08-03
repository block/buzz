#!/usr/bin/env bash
# buzz-msg.sh — post to, or catch up on, the coordination channel.
#
#   buzz-msg.sh send "CLAIM crates/buzz-auth/**"
#   buzz-msg.sh send -            (long content on stdin: diffs, stack traces)
#   buzz-msg.sh read [limit]      (default 50, oldest first)
#
# Both load this session's identity themselves. Nothing is sourced by hand and
# no channel UUID has to be pasted. The channel is, in order: --channel (a UUID
# or a name), BUZZ_COORD_CHANNEL in the environment, the room this session last
# connected to with buzz-connect.sh, then the cached UUID for the channel name,
# then a lookup by name. Run buzz-connect.sh first — this does not create.
#
# The session-pinned room is what makes dedicated channels work: after
# `buzz-connect.sh --channel pp-refactor`, a bare `buzz-msg.sh send` posts to
# pp-refactor and not to the machine's default channel.
set -uo pipefail

HERE="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
. "$HERE/lib.sh"

CHANNEL_ARG=""
cmd="${1:-}"
[ $# -gt 0 ] && shift
ARGS=""
while [ $# -gt 0 ]; do
  case "$1" in
    --channel) CHANNEL_ARG="${2:-}"; shift ;;
    *) ARGS="${ARGS:+$ARGS }$1" ;;
  esac
  shift
done

require_buzz
IDENT=$("$HERE/buzz-session.sh" resolve) || exit 1
SESSION_NAME=$(printf '%s' "$IDENT" | cut -f1)
PUBKEY=$(printf '%s' "$IDENT" | cut -f3)
IDFILE=$(printf '%s' "$IDENT" | cut -f4)
load_identity "$IDFILE" || die "could not load identity $IDFILE"
export RUST_LOG="${RUST_LOG:-error}"
RELAY="${BUZZ_RELAY_URL:-http://localhost:3000}"

resolve_channel "$CHANNEL_ARG" 0 || die \
"no coordination channel found (looked for '${CHANNEL_NAME:-?}').
  Fix: run $HERE/buzz-connect.sh — it finds or creates the channel."

case "$cmd" in
  send)
    [ -n "$ARGS" ] || die "usage: $0 send <text|->"
    if [ "$ARGS" = "-" ]; then
      buzz_run messages send --channel "$CHANNEL" --content -
    else
      buzz_run messages send --channel "$CHANNEL" --content "$ARGS"
    fi
    rc=$?
    if [ "$rc" != 0 ]; then
      note "send failed: ${BUZZ_ERR:-(no detail)}"
      case "$BUZZ_ERR" in
        *relay_membership_required*) diagnose_relay "$rc" "$BUZZ_ERR" "$PUBKEY" "$RELAY" ;;
        *) diagnose_channel "$CHANNEL" "${CHANNEL_NAME:-$CHANNEL}" "$PUBKEY" "" ;;
      esac
      exit "$rc"
    fi
    echo "sent to ${CHANNEL_NAME:-$CHANNEL} as $SESSION_NAME"
    ;;

  read)
    LIMIT="${ARGS:-50}"
    case "$LIMIT" in ''|*[!0-9]*) LIMIT=50 ;; esac
    if ! buzz_run messages get --channel "$CHANNEL" --limit "$LIMIT"; then
      rc=$?
      diagnose_channel "$CHANNEL" "${CHANNEL_NAME:-$CHANNEL}" "$PUBKEY" ""
      exit "$rc"
    fi
    LOG=$(printf '%s' "$BUZZ_OUT" | ME="$PUBKEY" python3 -c '
import json, os, sys
me = os.environ["ME"]
try:
    rows = json.load(sys.stdin)
except Exception:
    rows = []
rows = [r for r in rows if isinstance(r, dict)]
rows.sort(key=lambda r: r.get("created_at", 0))
for r in rows:
    who = r.get("pubkey", "")
    tag = "you     " if who == me else (who[:8] or "?")
    body = " ".join((r.get("content") or "").split())
    print("%s  %s" % (tag, body))
')
    # An empty channel is the classic symptom of the channel-membership gate,
    # so say so rather than printing nothing and letting the agent guess.
    if [ -n "$LOG" ]; then
      printf '%s\n' "$LOG"
    else
      echo "(no messages in ${CHANNEL_NAME:-$CHANNEL})"
      note ""
      note "  If peers say they have posted, this session is probably not a"
      note "  channel member — relay membership is a separate gate."
      note "  Check with: $HERE/buzz-connect.sh --status"
    fi
    ;;

  *) die "usage: $0 {send <text|->|read [limit]} [--channel <uuid-or-name>]" ;;
esac
