#!/usr/bin/env bash
# buzz-watch.sh [session-name|-] <channel-uuid> [poll-seconds]
#
# The WAKE path, and nothing else. Designed to be the `command` of Claude Code's
# Monitor tool with persistent: true — each stdout line becomes one notification,
# so this must be quiet unless something genuinely new arrived.
#
# It does not talk to the relay. `buzz-stream.sh` does that, as a daemon outside
# Monitor, appending one line per new peer message to
# ~/.buzz/stream/<identity>.<channel>.log. All this does is make sure that
# receiver is running and then `tail -f` the log from a stored line offset.
#
# That split is the guardrail. Monitor-hosted watchers have been observed dying
# with exit 144 where the identical command under nohup stayed healthy on the
# same channel, and Monitor reaps a task's output before anyone can read it, so
# three deaths produced no diagnosis. Rather than explain it, this removes the
# consequence: if this process dies, messages keep landing in the log, and
# re-arming replays every one of them from the offset. What a Monitor death now
# costs is the wake, not the messages.
#
# Interface is unchanged: same three arguments, same one-line-per-message output,
# and the same `-` for "resolve this session's identity yourself", so a Monitor
# command recorded before the split still works after it.
#
# Three details this encodes; do not "simplify" them away:
#   1. The offset is persisted per line delivered, not at exit. This process is
#      killed rather than asked to stop, and an offset lost on death means the
#      next arm either replays the whole log or skips what it missed.
#   2. `tail -n +K` counts lines, not bytes. Byte offsets and multi-byte content
#      drift apart; line counts do not.
#   3. The receiver is (re)started here as well as at connect. Whichever runs
#      later is the one that repairs it, and re-arming a watcher is exactly when
#      a session is asking to be able to hear again.
set -uo pipefail

# See buzz-stream.sh: bash ignores SIGURG by default, so it cannot by itself
# explain exit 144 — this is inherited-across-exec insurance, not a fix.
trap '' URG

HERE="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
. "$HERE/lib.sh"

NAME="${1:?usage: buzz-watch.sh [session-name|-] <channel-uuid> [poll-seconds]}"
CH="${2:?usage: buzz-watch.sh [session-name|-] <channel-uuid> [poll-seconds]}"
SLEEP="${3:-5}"

if [ "$NAME" = "-" ]; then
  IDENT=$("$HERE/buzz-session.sh" resolve) || exit 1
  NAME=$(printf '%s' "$IDENT" | cut -f1)
fi
[ -f "$(identity_file "$NAME")" ] || die \
"no identity '$NAME' in $SESSION_DIR — run $HERE/buzz-connect.sh first"

LOG=$(stream_log "$NAME" "$CH")
POS=$(stream_pos "$NAME" "$CH")

# --- the receiver has to exist before there is anything to tail ---------------
if ! ensure_receiver "$NAME" "$CH" "$SLEEP"; then
  die "could not start the receiver for '$NAME' on ${CH:0:8}.
  Its own log is the only place this is explained:
    $(stream_err "$NAME" "$CH")
  Nothing is listening and nothing is being fetched. Fix that before relying on
  this channel — until then peers cannot reach this session at all."
fi

# Liveness marker so buzz-connect.sh status can tell "watcher not armed" from
# "watcher armed and the channel is quiet" — two states that look identical.
MARKER=$(watch_marker "$NAME")
mkdir -p "$SESSION_DIR"
printf '%s\n%s\n' "$$" "$CH" > "$MARKER"

TAIL_PID=""
TAILPIDF="$(stream_base "$NAME" "$CH").tailpid"
cleanup() {
  trap - EXIT INT TERM
  [ -n "$TAIL_PID" ] && kill -TERM "$TAIL_PID" 2>/dev/null
  rm -f "$MARKER" "$TAILPIDF"
  exit 0
}
trap cleanup EXIT INT TERM

# A tail left behind by a watcher that was killed rather than stopped holds the
# log open and would double every notification once a new watcher arms. Nothing
# runs in a SIGKILLed process, so this is where that is cleaned up: on the way
# in, by whoever arms next.
if [ -f "$TAILPIDF" ]; then
  OLD=$(cat "$TAILPIDF" 2>/dev/null)
  case "$OLD" in
    ''|*[!0-9]*) ;;
    *) if kill -0 "$OLD" 2>/dev/null; then
         note "stopping a tail left by a previous watcher (pid $OLD)"
         kill -TERM "$OLD" 2>/dev/null
       fi ;;
  esac
  rm -f "$TAILPIDF"
fi

# --- where to resume from -----------------------------------------------------
# No offset file means this session has never armed a watcher on this channel, so
# start at the end: the log may hold everything since connect, and arming for the
# first time must not replay it as a burst.
#
# An offset that exists is the promise this whole design makes. Everything the
# receiver appended while no Monitor was alive is delivered now, in order, once.
if [ -f "$POS" ]; then
  START=$(cat "$POS" 2>/dev/null)
  case "$START" in ''|*[!0-9]*) START=0 ;; esac
else
  START=$(wc -l < "$LOG" 2>/dev/null | tr -d ' ')
  case "$START" in ''|*[!0-9]*) START=0 ;; esac
  printf '%s\n' "$START" > "$POS"
fi

# A log that was truncated or replaced under a stored offset would silently skip
# everything up to it. Restart from the beginning of the new log instead.
HAVE=$(wc -l < "$LOG" 2>/dev/null | tr -d ' ')
case "$HAVE" in ''|*[!0-9]*) HAVE=0 ;; esac
if [ "$START" -gt "$HAVE" ]; then
  note "stored offset $START is past the end of the log ($HAVE lines) — it was"
  note "rotated or replaced. Resuming from the start of the current log."
  START=0
  printf '%s\n' "$START" > "$POS"
fi

# `tail -n +K` starts AT line K, so the first undelivered line is START+1. -F
# rather than -f so a rotated log is picked up instead of tailing a stale inode.
#
# The tail feeds a FIFO and the delivery loop runs in THIS shell rather than in a
# pipeline subshell. That is not a style choice, and reverting it silently breaks
# the guarantee this whole design exists for:
#
#   - A pipeline subshell survives its parent. When the watcher was SIGKILLed —
#     which is what an unexplained exit 144 looks like from the outside — the
#     orphaned subshell went on reading the log and advancing the offset with
#     nobody receiving the lines. Re-arming then resumed past messages that had
#     never been delivered: silent loss, caused by the very code meant to
#     prevent it. Observed, not theorised.
#   - A loop in this shell dies exactly when this shell dies, so the offset can
#     never run ahead of what was delivered.
#   - `read` is a builtin, so bash services a trapped signal while it is blocked.
#     A foreground external command would defer the trap forever and leave the
#     marker claiming a watcher that is gone.
FIFO=$(mktemp -u -t buzz-watch-fifo)
mkfifo "$FIFO" || die "could not create $FIFO"
tail -n "+$(( START + 1 ))" -F "$LOG" 2>/dev/null > "$FIFO" &
TAIL_PID=$!
printf '%s\n' "$TAIL_PID" > "$TAILPIDF"
exec 3< "$FIFO"
rm -f "$FIFO"     # unlinked; both ends stay open until this process exits

DELIVERED="$START"
while IFS= read -r line <&3; do
  # Advance the offset only after the line is actually out. If the consumer has
  # gone, stop rather than mark undelivered messages as delivered — the next arm
  # is then a replay, which is recoverable, instead of a gap, which is not.
  printf '%s\n' "$line" || break
  DELIVERED=$(( DELIVERED + 1 ))
  printf '%s\n' "$DELIVERED" > "$POS"
done
