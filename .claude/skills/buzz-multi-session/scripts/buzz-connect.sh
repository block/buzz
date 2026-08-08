#!/usr/bin/env bash
# buzz-connect.sh — this session's whole relationship with the coordination
# channel: joining it, checking it, and ending it.
#
#   buzz-connect.sh [connect]              connect (the default)
#   buzz-connect.sh join <name>            open or enter a room for one piece of work
#   buzz-connect.sh status [--all]         am I connected, is the watcher alive
#   buzz-connect.sh leave                  stop participating in the current channel
#   buzz-connect.sh disconnect             stop participating entirely
#
# The verbs live here rather than in a dispatcher because every one of them needs
# the same first three steps — resolve this session's name, load its identity,
# resolve the room it is in — and those steps are this script. A dispatcher would
# either re-implement them or immediately hand back here.
#
# Every flag still works, and the verbs are additions rather than replacements:
# `--status` is `status`, `--channel <name>` is `join <name>`.
#
#   buzz-connect.sh [--channel <uuid-or-name>] [--invite <link>] [--status]
#                   [--quiet-hello] [--all] [--leave-channel] [--retire]
#
# CONNECT (the default) does everything a session needs, in one step and
# idempotently:
#
#   1. resolves this session's name (the /rename title, see buzz-session-name.sh)
#   2. mints or adopts its Buzz identity, following a /rename rather than
#      minting a second keypair
#   3. loads that identity itself — nothing is ever sourced by hand
#   4. enrols on the relay from an invite code if one is configured
#   5. publishes the display name so the session is findable in Buzz
#   6. finds or creates the coordination channel
#   7. announces HELLO
#   8. prints the exact Monitor command to arm, or reports the live watcher
#
# JOIN <name> is connect with a room named: it joins that channel or creates it,
# admits this session, and pins the room so a bare `buzz-msg.sh send` posts
# there. The UUID is cached per name, so a machine can hold several rooms at
# once. If the channel already exists and its owner's key is in ~/.buzz/sessions,
# this session is admitted automatically and told so (BUZZ_AUTO_ADMIT=0 turns
# that off).
#
# LEAVE and DISCONNECT do only the two unambiguous things: post DONE so peers
# know this session is gone rather than slow, and print the TaskStop that stops
# the watcher. Everything that cannot be undone by re-running connect is an
# explicit opt-in:
#
#   --leave-channel   give up channel membership (`buzz channels leave`). On a
#                     private channel the owner must re-admit you afterwards.
#   --retire          archive this session's identity (NIP-IA kind:9035). For a
#                     throwaway worktree, never for anything resumable.
#
# STATUS --all lists every identity on this machine, whether the relay still
# counts it as a member, and whether anything is listening for it. It prunes
# nothing.
#
# The only step that cannot be automated is authorising a new pubkey on a closed
# relay with no invite code available. That produces one clearly worded ask.
#
# Configuration, all optional, from the environment or ~/.buzz/config
# (KEY=value, parsed not sourced, chmod 600 — it holds a bearer token):
#   BUZZ_RELAY_URL           relay base URL          [http://localhost:3000]
#   BUZZ_INVITE_CODE         invite code every session self-enrols with
#   BUZZ_COORD_CHANNEL_NAME  default channel name    [agent-coordination]
#   BUZZ_AUTO_ADMIT          0 disables admitting with a local owner key  [1]
#
# Anything a relay minted is cached per relay, because a channel UUID and an
# invite code are both meaningless on a different one — and a UUID is
# structurally valid everywhere, so the wrong relay is silent rather than an
# error. These keys are written, not set by hand:
#   BUZZ_COORD_CHANNEL__<RELAY>   default channel's UUID on that relay
#   BUZZ_CHANNEL_<NAME>__<RELAY>  a dedicated channel's UUID on that relay
#   BUZZ_INVITE_CODE__<RELAY>     the code that worked on that relay
# The unscoped BUZZ_COORD_CHANNEL, BUZZ_CHANNEL_<NAME> and BUZZ_INVITE_CODE are
# still read, so an existing config keeps working, and are adopted into the
# scoped form on first use.
set -uo pipefail

HERE="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
. "$HERE/lib.sh"

USAGE="usage: $0 [connect|join <name>|status|leave|disconnect] [options]
  connect                       (default) join the current room and arm a watcher
  join <name>                   open or enter a room for one piece of work
  status [--all]                am I connected, is the watcher alive
  leave [--leave-channel]       stop participating in the current channel
  disconnect [--leave-channel] [--retire]
                                stop participating entirely
options: --channel <uuid-or-name> --invite <link-or-code> --status --quiet-hello"

# Verbs are an addition, not a replacement: --status is still status, and
# --channel <name> is still join <name>. A bare run is still connect.
VERB=connect
case "${1:-}" in
  connect|status|leave|disconnect) VERB="$1"; shift ;;
  join)
    VERB="join"; shift
    case "${1:-}" in
      ''|-*) die "usage: $0 join <channel-name-or-uuid>" ;;
    esac
    CHANNEL_ARG="$1"; shift ;;
esac

STATUS_ONLY=0
SAY_HELLO=1
CHANNEL_ARG="${CHANNEL_ARG:-}"
INVITE_ARG=""
SHOW_ALL=0
LEAVE_CHANNEL=0
RETIRE=0
while [ $# -gt 0 ]; do
  case "$1" in
    --channel) CHANNEL_ARG="${2:-}"; shift ;;
    --invite) INVITE_ARG="${2:-}"; shift ;;
    --status) [ "$VERB" = connect ] && VERB=status ;;
    --quiet-hello) SAY_HELLO=0 ;;
    --all) SHOW_ALL=1 ;;
    --leave-channel) LEAVE_CHANNEL=1 ;;
    --retire) RETIRE=1 ;;
    -h|--help) sed -n '2,66p' "$0"; exit 0 ;;
    *) die "$USAGE" ;;
  esac
  shift
done

# Refuse a flag that does not belong to the verb rather than ignoring it. A
# --retire that silently did nothing would be the worst possible outcome here,
# and so would one that fired on a verb the caller did not think was destructive.
case "$VERB" in
  status)
    [ "$RETIRE" = 0 ] && [ "$LEAVE_CHANNEL" = 0 ] \
      || die "--retire and --leave-channel are not status flags; see 'disconnect'" ;;
  leave)
    [ "$RETIRE" = 0 ] \
      || die "--retire is not a 'leave' flag. Leaving a room does not retire the
identity that was in it. If this session is finished for good:
  $0 disconnect --retire" ;;
  connect|join)
    [ "$RETIRE" = 0 ] && [ "$LEAVE_CHANNEL" = 0 ] \
      || die "--retire and --leave-channel are teardown flags; see 'leave' and 'disconnect'" ;;
esac
[ "$VERB" = status ] || [ "$SHOW_ALL" = 0 ] || die "--all is a 'status' flag"
if [ "$VERB" = status ]; then STATUS_ONLY=1; SAY_HELLO=0; fi

check_config_perms
require_buzz

# --- 0. an invite handed in by the user --------------------------------------
# What a human has is whatever "Invite to community → Copy link" put on their
# clipboard: a whole URL. Asking them to strip the code out of it by hand is the
# kind of step that makes this developer-only, so take either form, and persist
# it so this is the last time anyone is asked.
if [ -n "$INVITE_ARG" ]; then
  code=${INVITE_ARG##*/invite/}     # URL → code; a bare code is unchanged
  code=${code%%[?#]*}               # drop any query string or fragment
  code=$(printf '%s' "$code" | tr -d '[:space:]')
  case "$code" in
    ""|*[!A-Za-z0-9._~-]*)
      die "that does not look like an invite code or link.
  Expected the whole link from Buzz Desktop → Invite to community → Copy link,
  e.g. https://relay.example/invite/v2.abc123 — or just the code after /invite/." ;;
  esac
  # Saved against this relay AND unscoped. The scoped key is what a later relay
  # change reads, so switching relays cannot retry a code the new one never
  # minted; the unscoped one keeps older copies of these scripts working.
  config_set "$(invite_cache_key)" "$code"
  config_set BUZZ_INVITE_CODE "$code"
  echo "invite   : saved to $CONFIG_FILE for $(setting BUZZ_RELAY_URL '') —"
  echo "           every future session on this relay enrols itself"
fi

# --- 1-3. identity -----------------------------------------------------------
IDENT=$("$HERE/buzz-session.sh" resolve) || exit 1
SESSION_NAME=$(printf '%s' "$IDENT" | cut -f1)
SESSION_DISPLAY=$(printf '%s' "$IDENT" | cut -f2)
PUBKEY=$(printf '%s' "$IDENT" | cut -f3)
IDFILE=$(printf '%s' "$IDENT" | cut -f4)
load_identity "$IDFILE" || die "could not load identity $IDFILE"
RELAY="${BUZZ_RELAY_URL:-http://localhost:3000}"
export RUST_LOG="${RUST_LOG:-error}"

# The published name says what kind of member this is, not just which one. In a
# channel listing a bare "spec-kit-arch-governance-init" is indistinguishable
# from a human; "Claude Code (spec-kit-arch-governance-init)" tells a reader at
# a glance that it is an agent session and which terminal to go find. Override
# the prefix with BUZZ_PROFILE_PREFIX, or set it empty for the bare name.
PROFILE_PREFIX=$(setting BUZZ_PROFILE_PREFIX "Claude Code")
if [ -n "$PROFILE_PREFIX" ]; then
  SESSION_DISPLAY="$PROFILE_PREFIX ($SESSION_DISPLAY)"
fi

printf 'session  : %s\nidentity : %s\npubkey   : %s\nrelay    : %s\n' \
  "$SESSION_DISPLAY" "$SESSION_NAME" "$PUBKEY" "$RELAY"

# --- teardown ----------------------------------------------------------------
# leave and disconnect stop here. They deliberately skip the relay probe, the
# profile publish and the auto-admit: a session that is going away should not
# enrol itself or get itself readmitted on the way out, and the two things that
# always have to happen — DONE, and stopping the watcher — must still happen when
# the relay is unreachable.
if [ "$VERB" = leave ] || [ "$VERB" = disconnect ]; then
  if ! resolve_channel "$CHANNEL_ARG" 0; then
    note "note: no channel to leave (looked for '${CHANNEL_NAME:-?}')."
    note "      Stopping the watcher and clearing local state anyway."
    CHANNEL=""
  else
    echo "channel  : ${CHANNEL_NAME:-<uuid>} ($CHANNEL)"
  fi
  teardown "$VERB" "$LEAVE_CHANNEL" "$RETIRE"
  exit $?
fi

# --- 4. relay membership -----------------------------------------------------
ensure_relay_membership "$PUBKEY" "$RELAY" || exit $?

# --- 5. profile --------------------------------------------------------------
publish_profile "$SESSION_NAME" "$SESSION_DISPLAY"

# --- 6. channel --------------------------------------------------------------
# status reports; it does not act. It must not create a channel and — the case
# that actually bites — it must not re-admit a session that has just left one,
# which would make `leave` look like it silently failed.
CREATE=1
[ "$STATUS_ONLY" = 1 ] && CREATE=0
if ! resolve_channel "$CHANNEL_ARG" "$CREATE"; then
  if [ "$STATUS_ONLY" = 1 ]; then
    echo "channel  : none — this session is not in a room. Join one with"
    echo "           '$(basename "$0")' for the default channel, or"
    echo "           '$(basename "$0") join <name>' for a room of its own."
    [ "$SHOW_ALL" = 1 ] && roster_report
    exit 2
  fi
  note "could not find or create channel '${CHANNEL_NAME:-?}': ${BUZZ_ERR:-(no detail)}"
  exit 2
fi
if [ "$CHANNEL_CREATED" = 1 ]; then
  echo "channel  : created '$CHANNEL_NAME' ($CHANNEL)"
  echo "           recorded as $CHANNEL_KEY in $CONFIG_FILE, so other"
  echo "           sessions on this machine join it rather than creating their own."
else
  echo "channel  : ${CHANNEL_NAME:-<uuid>} ($CHANNEL)"
fi
# Which relay this channel belongs to is invisible in a UUID, and getting it
# wrong is silent, so status says it out loud.
if [ "$STATUS_ONLY" = 1 ]; then
  echo "           on $RELAY"
  [ -n "${CHANNEL_KEY:-}" ] && echo "           cached as $CHANNEL_KEY"
fi

# --- 6b. channel membership --------------------------------------------------
# The second gate. Relay membership does not imply channel membership, and the
# symptom of missing it is an empty channel with no error at all.
#
# A non-member gets [] from `channels members`, not a 403, so "empty list" and
# "not allowed to look" are the same response. Treat both as not-a-member and
# let join_channel work out whether it can be fixed here.
if [ "$CHANNEL_CREATED" = 0 ]; then
  MEMBER=""
  OWNER=""
  if buzz_run channels members --channel "$CHANNEL"; then
    CHECK=$(printf '%s' "$BUZZ_OUT" | ME="$PUBKEY" python3 -c '
import json, os, sys
me = os.environ["ME"]
try:
    rows = json.load(sys.stdin)
except Exception:
    rows = []
member, owner = "", ""
for row in rows if isinstance(rows, list) else []:
    if not isinstance(row, dict):
        continue
    if row.get("pubkey") == me:
        member = "1"
    if row.get("role") == "owner" and not owner:
        owner = row.get("pubkey") or ""
sys.stdout.write("%s\t%s" % (member, owner))
')
    MEMBER=$(printf '%s' "$CHECK" | cut -f1)
    OWNER=$(printf '%s' "$CHECK" | cut -f2)
  fi
  if [ -z "$MEMBER" ]; then
    if [ "$STATUS_ONLY" = 1 ]; then
      echo "channel  : NOT a member of '${CHANNEL_NAME:-$CHANNEL}'. Peers' messages"
      echo "           cannot reach this session and its sends will be refused."
      echo "           Rejoin with: $(basename "$0") join ${CHANNEL_NAME:-$CHANNEL}"
      [ "$SHOW_ALL" = 1 ] && roster_report
      exit 4
    fi
    if ! join_channel "$CHANNEL" "${CHANNEL_NAME:-$CHANNEL}" "$PUBKEY"; then
      diagnose_channel "$CHANNEL" "${CHANNEL_NAME:-$CHANNEL}" "$PUBKEY" "$OWNER"
      exit 4
    fi
  fi
fi

# Pin the room to this session, so buzz-msg.sh posts where this session
# actually is. A session that opened a dedicated channel stays in it until it
# is pointed somewhere else with --channel; the machine-wide default is not
# allowed to drag it back.
meta_set "$SESSION_NAME" BUZZ_SESSION_CHANNEL "$CHANNEL"
meta_set "$SESSION_NAME" BUZZ_SESSION_CHANNEL_NAME "${CHANNEL_NAME:-}"
# The relay the pin belongs to. Without it, a relay change leaves the session
# posting a valid-looking UUID into a channel that does not exist there.
meta_set "$SESSION_NAME" BUZZ_SESSION_CHANNEL_RELAY "$(relay_tag)"

# --- 7. HELLO ----------------------------------------------------------------
if [ "$SAY_HELLO" = 1 ]; then
  # --abbrev-ref prints "HEAD" *and* fails on an unborn branch; check the value.
  BRANCH=$(git rev-parse --abbrev-ref HEAD 2>/dev/null)
  case "$BRANCH" in ''|HEAD) BRANCH=$(git branch --show-current 2>/dev/null) ;; esac
  [ -n "$BRANCH" ] || BRANCH="(no branch)"
  WHERE=$(basename "$(git rev-parse --show-toplevel 2>/dev/null || pwd)")
  if buzz_run messages send --channel "$CHANNEL" \
      --content "HELLO $SESSION_DISPLAY branch=$BRANCH dir=$WHERE"; then
    echo "hello    : announced"
  else
    note "warning: HELLO failed: ${BUZZ_ERR:-(no detail)}"
  fi
fi

# --- 8. receiver --------------------------------------------------------------
# Reception is started here, before anything is armed, and it is a separate
# concern from waking. It runs outside Monitor and keeps fetching messages into
# a log whatever happens to the Monitor task; see lib.sh for why.
WATCH_CMD="$HERE/buzz-watch.sh - $CHANNEL 5"
STREAM_LOG=$(stream_log "$SESSION_NAME" "$CHANNEL")
RECV_BAD=0
case "$(receiver_state "$SESSION_NAME" "$CHANNEL")" in
  live*) ;;
  *) ensure_receiver "$SESSION_NAME" "$CHANNEL" 5 || RECV_BAD=1 ;;
esac
RSTATE=$(receiver_state "$SESSION_NAME" "$CHANNEL")
case "$RSTATE" in
  live*)
    echo "receiver : $(printf '%s' "$RSTATE" | awk '{print "live pid " $2 ", heartbeat " $3 "s ago"}')"
    echo "           queueing to $STREAM_LOG" ;;
  *)
    RECV_BAD=1
    cat >&2 <<EOF
receiver : $RSTATE — NOT RECEIVING. This is the serious one: nothing is
           fetching messages for this session, so peers' messages are not
           being missed by the watcher, they are not arriving at all.
           Why, if it said anything: $(stream_err "$SESSION_NAME" "$CHANNEL")
EOF
    ;;
esac

# --- 9. watcher ---------------------------------------------------------------
if PID=$(watcher_pid "$SESSION_NAME"); then
  echo "watcher  : armed (pid $PID)"
  [ "$SHOW_ALL" = 1 ] && roster_report
  [ "$RECV_BAD" = 0 ] || exit 6
  exit 0
fi

QUEUED=0
if [ -f "$STREAM_LOG" ] && [ -f "$(stream_pos "$SESSION_NAME" "$CHANNEL")" ]; then
  HAVE=$(wc -l < "$STREAM_LOG" 2>/dev/null | tr -d ' ')
  DONE=$(cat "$(stream_pos "$SESSION_NAME" "$CHANNEL")" 2>/dev/null)
  case "$HAVE" in ''|*[!0-9]*) HAVE=0 ;; esac
  case "$DONE" in ''|*[!0-9]*) DONE=0 ;; esac
  [ "$HAVE" -gt "$DONE" ] && QUEUED=$(( HAVE - DONE ))
fi

if [ "$STATUS_ONLY" = 1 ]; then
  # Exit non-zero so "connected but deaf" is a checkable state, not prose.
  echo "watcher  : NOT ARMED — nothing will wake this session."
  if [ "$QUEUED" != 0 ]; then
    echo "           $QUEUED message(s) already queued; arming delivers them."
  fi
  echo "           Arm it with: Monitor(command: \"$WATCH_CMD\", persistent: true)"
  [ "$SHOW_ALL" = 1 ] && roster_report
  [ "$RECV_BAD" = 0 ] || exit 6
  exit 1
fi

cat <<EOF

watcher  : NOT ARMED. Arm it now — until you do, peers can see you but you
           cannot see them, which looks exactly like an agent ignoring them.

Monitor(
  command: "$WATCH_CMD",
  description: "buzz coordination: ${CHANNEL_NAME:-$CHANNEL}",
  persistent: true
)
EOF

[ "$QUEUED" = 0 ] || cat <<EOF

           $QUEUED message(s) arrived while nothing was armed and are waiting in
           the log. Arming replays them from the stored offset — in order, once.
EOF

cat <<EOF

Keep the task id that call returns — 'buzz-connect.sh leave' and 'disconnect'
print the TaskStop that needs it, and a watcher nobody can stop outlives the
session.

If that Monitor ever dies, messages are NOT lost: the receiver is a separate
process and keeps queueing them. Re-arm with exactly the call above and every
message since is delivered. Check with 'buzz-connect.sh status', which exits
non-zero when the watcher is unarmed and 6 when the receiver itself is down.

Post and catch up with (they load this session's identity themselves):
  $HERE/buzz-msg.sh send "STATUS ..."
  $HERE/buzz-msg.sh read 50
EOF
[ "$RECV_BAD" = 0 ] || exit 6
